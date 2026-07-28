use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use labello_domain::{
    ARTIFACT_MIGRATION_JOURNAL_VERSION, Actor, ArtifactMigrationFile, ArtifactMigrationJournal,
    ArtifactMigrationKind, ArtifactMigrationPhase, DatasetConfig, DatasetMetadata, DatasetSnapshot,
    EventLogEntry, EventPayload, ImageId, ImageRecord, ImageState, ImagesIndex, ImportId,
    ImportManifest, KeybindingSet, LEGACY_SCHEMA_VERSION, MigrationRecord, SCHEMA_VERSION,
    SnapshotFileEntry, SnapshotImportEntry, TaskId, UserId, labello_schema_bundle, now,
    rebuild_state,
};
use parking_lot::Mutex;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex as AsyncMutex,
};

use crate::{
    error::{PathIo, PathJson, PathTomlEncode, StorageError, StorageResult},
    fsjson::{
        create_dir_all_synced, read_current_json, read_json, read_schema_version,
        write_bytes_atomic, write_json_atomic,
    },
    fstoml::{read_current_toml, read_toml, write_toml_atomic},
    paths,
    stats::StatsCache,
};

const ASSIGNMENT_AVAILABILITY_CACHE_TTL: Duration = Duration::from_secs(30);

pub(crate) type AssignmentAvailabilityCacheKey = (UserId, String);

#[derive(Clone, Debug)]
struct CachedAssignmentAvailability {
    generation: u64,
    cached_at: Instant,
    tasks: BTreeMap<TaskId, bool>,
}

#[derive(Debug, Default)]
pub(crate) struct AssignmentAvailabilityCache {
    generation: AtomicU64,
    values: AsyncMutex<BTreeMap<AssignmentAvailabilityCacheKey, CachedAssignmentAvailability>>,
    refresh: AsyncMutex<()>,
    #[cfg(test)]
    scans: AtomicU64,
}

impl AssignmentAvailabilityCache {
    pub(crate) fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub(crate) async fn get(
        &self,
        key: &AssignmentAvailabilityCacheKey,
        generation: u64,
    ) -> Option<BTreeMap<TaskId, bool>> {
        self.values
            .lock()
            .await
            .get(key)
            .filter(|cached| {
                cached.generation == generation
                    && cached.cached_at.elapsed() < ASSIGNMENT_AVAILABILITY_CACHE_TTL
            })
            .map(|cached| cached.tasks.clone())
    }

    pub(crate) async fn lock_refresh(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.refresh.lock().await
    }

    pub(crate) async fn store(
        &self,
        key: AssignmentAvailabilityCacheKey,
        generation: u64,
        tasks: BTreeMap<TaskId, bool>,
    ) {
        self.values.lock().await.insert(
            key,
            CachedAssignmentAvailability {
                generation,
                cached_at: Instant::now(),
                tasks,
            },
        );
    }

    #[cfg(test)]
    pub(crate) fn record_scan(&self) {
        self.scans.fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) fn scan_count(&self) -> u64 {
        self.scans.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Debug)]
pub struct DatasetRepository {
    root: Arc<PathBuf>,
    locks: Arc<Mutex<BTreeMap<ImageId, Arc<AsyncMutex<()>>>>>,
    migration_lock: Arc<AsyncMutex<()>>,
    migration_complete: Arc<AtomicBool>,
    pub(crate) stats_cache: Arc<StatsCache>,
    pub(crate) assignment_availability_cache: Arc<AssignmentAvailabilityCache>,
    #[cfg(test)]
    migration_failure: Arc<Mutex<Option<ArtifactMigrationPhase>>>,
    #[cfg(test)]
    image_state_loads: Arc<AtomicU64>,
}

impl DatasetRepository {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: Arc::new(root.into()),
            locks: Arc::new(Mutex::new(BTreeMap::new())),
            migration_lock: Arc::new(AsyncMutex::new(())),
            migration_complete: Arc::new(AtomicBool::new(false)),
            stats_cache: Arc::new(StatsCache::default()),
            assignment_availability_cache: Arc::new(AssignmentAvailabilityCache::default()),
            #[cfg(test)]
            migration_failure: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            image_state_loads: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn dataset_path(&self) -> PathBuf {
        self.root.join(paths::DATASET_FILE)
    }

    pub fn schema_path(&self) -> PathBuf {
        self.root.join(paths::SCHEMA_FILE)
    }

    pub fn images_index_path(&self) -> PathBuf {
        self.root.join(paths::IMAGES_INDEX_FILE)
    }

    pub fn annotations_dir(&self, image_id: &ImageId) -> PathBuf {
        self.root
            .join(paths::ANNOTATIONS_DIR)
            .join(image_id.as_str())
    }

    pub fn state_path(&self, image_id: &ImageId) -> PathBuf {
        self.annotations_dir(image_id).join(paths::STATE_FILE)
    }

    pub fn events_path(&self, image_id: &ImageId) -> PathBuf {
        self.annotations_dir(image_id).join(paths::EVENTS_FILE)
    }

    pub fn snapshots_dir(&self) -> PathBuf {
        self.root.join(paths::SNAPSHOTS_DIR)
    }

    pub fn imports_dir(&self) -> PathBuf {
        self.root.join(paths::IMPORTS_DIR)
    }

    pub fn artifact_migration_journal_path(&self) -> PathBuf {
        self.root
            .join(paths::ARTIFACT_MIGRATION_DIR)
            .join(paths::ARTIFACT_MIGRATION_JOURNAL_FILE)
    }

    pub async fn create_snapshot(&self) -> StorageResult<DatasetSnapshot> {
        let config = self.load_dataset_config().await?;
        let index = self.load_images_index().await?;
        let snapshot_id = format!(
            "{}-{}",
            now().format("%Y%m%dT%H%M%S%.3fZ"),
            uuid::Uuid::new_v4().simple()
        );
        let snapshots_dir = self.snapshots_dir();
        let temporary = snapshots_dir.join(format!(".{snapshot_id}.tmp"));
        let destination = snapshots_dir.join(&snapshot_id);
        tokio::fs::create_dir_all(&temporary)
            .await
            .with_path(&temporary)?;

        let result = async {
            let mut files = Vec::new();
            self.snapshot_copy_file(paths::DATASET_FILE, &temporary, &mut files)
                .await?;
            self.snapshot_copy_file(paths::IMAGES_INDEX_FILE, &temporary, &mut files)
                .await?;
            if tokio::fs::try_exists(self.schema_path())
                .await
                .with_path(self.schema_path())?
            {
                self.snapshot_copy_file(paths::SCHEMA_FILE, &temporary, &mut files)
                    .await?;
            }
            let imports = self
                .snapshot_import_records(&config.dataset_id, &temporary, &mut files)
                .await?;
            for image in index.images_by_hash.values() {
                let events = self.load_events(&image.image_id).await?;
                let events_path = self.events_path(&image.image_id);
                let event_bytes = if tokio::fs::try_exists(&events_path)
                    .await
                    .with_path(&events_path)?
                {
                    tokio::fs::read(&events_path)
                        .await
                        .with_path(&events_path)?
                } else {
                    Vec::new()
                };
                let relative = format!(
                    "{}/{}/{}",
                    paths::ANNOTATIONS_DIR,
                    image.image_id,
                    paths::EVENTS_FILE
                );
                write_snapshot_bytes(&temporary, &relative, &event_bytes, &mut files).await?;

                let state = rebuild_state(image.image_id.clone(), &events)?;
                let state_bytes = serde_json::to_vec_pretty(&state)
                    .with_json_path(self.state_path(&image.image_id))?;
                let relative = format!(
                    "{}/{}/{}",
                    paths::ANNOTATIONS_DIR,
                    image.image_id,
                    paths::STATE_FILE
                );
                write_snapshot_bytes(&temporary, &relative, &state_bytes, &mut files).await?;
            }
            files.sort_by(|left, right| left.path.cmp(&right.path));
            let manifest = DatasetSnapshot {
                schema_version: SCHEMA_VERSION,
                snapshot_id: snapshot_id.clone(),
                dataset_id: config.dataset_id,
                created_at: now(),
                includes_image_bytes: false,
                total_bytes: files.iter().map(|file| file.byte_size).sum(),
                files,
                imports,
            };
            let manifest_bytes = serde_json::to_vec_pretty(&manifest)
                .with_json_path(temporary.join(paths::SNAPSHOT_MANIFEST_FILE))?;
            write_snapshot_bytes(
                &temporary,
                paths::SNAPSHOT_MANIFEST_FILE,
                &manifest_bytes,
                &mut Vec::new(),
            )
            .await?;
            tokio::fs::rename(&temporary, &destination)
                .await
                .with_path(&destination)?;
            Ok(manifest)
        }
        .await;
        if result.is_err()
            && let Err(error) = tokio::fs::remove_dir_all(&temporary).await
        {
            tracing::warn!(
                event = "snapshot.cleanup.failed",
                error_kind = %error.kind(),
                "could not remove incomplete snapshot"
            );
        }
        result
    }

    pub async fn list_snapshots(&self) -> StorageResult<Vec<DatasetSnapshot>> {
        let directory = self.snapshots_dir();
        if !tokio::fs::try_exists(&directory)
            .await
            .with_path(&directory)?
        {
            return Ok(Vec::new());
        }
        let mut entries = tokio::fs::read_dir(&directory)
            .await
            .with_path(&directory)?;
        let mut snapshots: Vec<DatasetSnapshot> = Vec::new();
        while let Some(entry) = entries.next_entry().await.with_path(&directory)? {
            if !entry.file_type().await.with_path(entry.path())?.is_dir()
                || entry.file_name().to_string_lossy().starts_with('.')
            {
                continue;
            }
            let manifest_path = entry.path().join(paths::SNAPSHOT_MANIFEST_FILE);
            if tokio::fs::try_exists(&manifest_path)
                .await
                .with_path(&manifest_path)?
            {
                snapshots.push(read_current_json(&manifest_path).await?);
            }
        }
        snapshots.sort_by_key(|snapshot| std::cmp::Reverse(snapshot.created_at));
        Ok(snapshots)
    }

    pub async fn snapshot_file(
        &self,
        snapshot_id: &str,
        relative_path: &str,
    ) -> StorageResult<Vec<u8>> {
        validate_snapshot_segment(snapshot_id)?;
        let relative = Path::new(relative_path);
        if relative.as_os_str().is_empty()
            || relative.is_absolute()
            || relative.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(StorageError::OutsideDatasetRoot(relative.to_path_buf()));
        }
        let snapshot = self.snapshots_dir().join(snapshot_id);
        let manifest: DatasetSnapshot =
            read_current_json(&snapshot.join(paths::SNAPSHOT_MANIFEST_FILE)).await?;
        if relative_path != paths::SNAPSHOT_MANIFEST_FILE
            && !manifest.files.iter().any(|file| file.path == relative_path)
        {
            return Err(StorageError::NotFound(snapshot.join(relative)));
        }
        let path = snapshot.join(relative);
        tokio::fs::read(&path).await.with_path(path)
    }

    async fn snapshot_copy_file(
        &self,
        relative_path: &str,
        destination: &Path,
        files: &mut Vec<SnapshotFileEntry>,
    ) -> StorageResult<()> {
        let source = self.root.join(relative_path);
        let bytes = tokio::fs::read(&source).await.with_path(&source)?;
        write_snapshot_bytes(destination, relative_path, &bytes, files).await
    }

    async fn snapshot_import_records(
        &self,
        dataset_id: &labello_domain::DatasetId,
        destination: &Path,
        files: &mut Vec<SnapshotFileEntry>,
    ) -> StorageResult<Vec<SnapshotImportEntry>> {
        let records = self.import_records(dataset_id).await?;
        let mut imports = Vec::with_capacity(records.len());
        for (manifest, manifest_path, source_objects_path) in records {
            let relative_root = format!("{}/{}", paths::IMPORTS_DIR, manifest.import_id);
            let manifest_relative = format!("{relative_root}/{}", paths::IMPORT_MANIFEST_FILE);
            let source_objects_relative =
                format!("{relative_root}/{}", paths::IMPORT_SOURCE_OBJECTS_FILE);
            let manifest_bytes = tokio::fs::read(&manifest_path)
                .await
                .with_path(&manifest_path)?;
            write_snapshot_bytes(destination, &manifest_relative, &manifest_bytes, files).await?;
            let source_objects_bytes = tokio::fs::read(&source_objects_path)
                .await
                .with_path(&source_objects_path)?;
            write_snapshot_bytes(
                destination,
                &source_objects_relative,
                &source_objects_bytes,
                files,
            )
            .await?;
            imports.push(SnapshotImportEntry {
                import_id: manifest.import_id,
                manifest_path: manifest_relative,
                source_objects_path: source_objects_relative,
            });
        }
        Ok(imports)
    }

    pub async fn load_import_manifests(&self) -> StorageResult<Vec<ImportManifest>> {
        let dataset_id = self.load_dataset_config().await?.dataset_id;
        Ok(self
            .import_records(&dataset_id)
            .await?
            .into_iter()
            .map(|(manifest, _, _)| manifest)
            .collect())
    }

    async fn import_records(
        &self,
        dataset_id: &labello_domain::DatasetId,
    ) -> StorageResult<Vec<(ImportManifest, PathBuf, PathBuf)>> {
        let directory = self.imports_dir();
        if !tokio::fs::try_exists(&directory)
            .await
            .with_path(&directory)?
        {
            return Ok(Vec::new());
        }
        let mut reader = tokio::fs::read_dir(&directory)
            .await
            .with_path(&directory)?;
        let mut import_directories = Vec::new();
        while let Some(entry) = reader.next_entry().await.with_path(&directory)? {
            if entry.file_type().await.with_path(entry.path())?.is_dir()
                && !entry.file_name().to_string_lossy().starts_with('.')
            {
                import_directories.push(entry.path());
            }
        }
        import_directories.sort();

        let mut records = Vec::with_capacity(import_directories.len());
        for import_directory in import_directories {
            let directory_name = import_directory
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| StorageError::OutsideDatasetRoot(import_directory.clone()))?;
            let directory_import_id = ImportId::from(directory_name);
            directory_import_id
                .validate_path_segment()
                .map_err(|_| StorageError::OutsideDatasetRoot(import_directory.clone()))?;
            let manifest_path = import_directory.join(paths::IMPORT_MANIFEST_FILE);
            let source_objects_path = import_directory.join(paths::IMPORT_SOURCE_OBJECTS_FILE);
            let manifest: ImportManifest = read_json(&manifest_path).await?;
            labello_domain::validate_schema_version(manifest.schema_version)?;
            if manifest.import_id != directory_import_id || manifest.dataset_id != *dataset_id {
                return Err(StorageError::Domain(
                    labello_domain::DomainError::InvalidImport(
                        "import manifest identity does not match its repository path".to_string(),
                    ),
                ));
            }
            if !tokio::fs::try_exists(&source_objects_path)
                .await
                .with_path(&source_objects_path)?
            {
                return Err(StorageError::NotFound(source_objects_path));
            }
            records.push((manifest, manifest_path, source_objects_path));
        }
        Ok(records)
    }

    pub fn image_path(&self, relative_path: &str) -> StorageResult<PathBuf> {
        let relative = Path::new(relative_path);
        if relative.is_absolute()
            || relative.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(StorageError::OutsideDatasetRoot(relative.to_path_buf()));
        }
        Ok(self.root.join(relative))
    }

    pub async fn initialize(&self, mut metadata: DatasetMetadata) -> StorageResult<()> {
        self.ensure_layout().await?;
        metadata.schema_version = SCHEMA_VERSION;
        metadata.updated_at = now();
        self.create_dataset(&metadata).await?;
        self.save_images_index(&ImagesIndex::default()).await?;
        write_json_atomic(&self.schema_path(), &labello_schema_bundle()).await?;
        Ok(())
    }

    async fn create_dataset(&self, metadata: &DatasetMetadata) -> StorageResult<()> {
        labello_domain::validate_schema_version(metadata.schema_version)?;
        let path = self.dataset_path();
        let text = toml::to_string_pretty(&DatasetConfig::from_metadata(metadata))
            .with_toml_encode_path(&path)?;
        let mut file = match tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .await
        {
            Ok(file) => file,
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(StorageError::AlreadyExists(path));
            }
            Err(source) => return Err(StorageError::Io { path, source }),
        };
        file.write_all(text.as_bytes()).await.with_path(&path)?;
        if !text.ends_with('\n') {
            file.write_all(b"\n").await.with_path(&path)?;
        }
        file.sync_all().await.with_path(&path)?;
        Ok(())
    }

    pub async fn ensure_layout(&self) -> StorageResult<()> {
        tokio::fs::create_dir_all(&*self.root)
            .await
            .with_path(&*self.root)?;
        tokio::fs::create_dir_all(self.root.join(paths::IMAGES_DIR))
            .await
            .with_path(self.root.join(paths::IMAGES_DIR))?;
        tokio::fs::create_dir_all(self.root.join(paths::ANNOTATIONS_DIR))
            .await
            .with_path(self.root.join(paths::ANNOTATIONS_DIR))?;
        tokio::fs::create_dir_all(self.root.join(paths::USERS_DIR))
            .await
            .with_path(self.root.join(paths::USERS_DIR))?;
        Ok(())
    }

    pub async fn load_dataset(&self) -> StorageResult<DatasetMetadata> {
        self.ensure_artifact_migration().await?;
        let config: DatasetConfig = read_current_toml(&self.dataset_path()).await?;
        let images = self
            .load_images_index()
            .await?
            .images_by_hash
            .into_values()
            .map(|record| (record.image_id.clone(), record))
            .collect();
        Ok(config.into_metadata(images))
    }

    pub async fn load_dataset_config(&self) -> StorageResult<DatasetMetadata> {
        self.ensure_artifact_migration().await?;
        let config: DatasetConfig = read_current_toml(&self.dataset_path()).await?;
        Ok(config.into_metadata(BTreeMap::new()))
    }

    pub async fn save_dataset(&self, metadata: &DatasetMetadata) -> StorageResult<()> {
        self.ensure_artifact_migration().await?;
        labello_domain::validate_schema_version(metadata.schema_version)?;
        write_toml_atomic(
            &self.dataset_path(),
            &DatasetConfig::from_metadata(metadata),
        )
        .await?;
        self.stats_cache.invalidate();
        self.assignment_availability_cache.invalidate();
        Ok(())
    }

    pub async fn load_images_index(&self) -> StorageResult<ImagesIndex> {
        self.ensure_artifact_migration().await?;
        if !tokio::fs::try_exists(self.images_index_path())
            .await
            .with_path(self.images_index_path())?
        {
            return Ok(ImagesIndex::default());
        }
        read_current_json(&self.images_index_path()).await
    }

    pub async fn image_count(&self) -> StorageResult<usize> {
        if !tokio::fs::try_exists(self.images_index_path())
            .await
            .with_path(self.images_index_path())?
        {
            return Ok(0);
        }
        if let Some(count) = self.read_image_count_hint().await? {
            return Ok(count);
        }
        Ok(self.load_images_index().await?.images_by_hash.len())
    }

    pub async fn load_image_record(&self, image_id: &ImageId) -> StorageResult<ImageRecord> {
        self.load_images_index()
            .await?
            .images_by_hash
            .into_values()
            .find(|record| &record.image_id == image_id)
            .ok_or_else(|| StorageError::NotFound(self.images_index_path()))
    }

    pub async fn save_images_index(&self, index: &ImagesIndex) -> StorageResult<()> {
        self.ensure_artifact_migration().await?;
        labello_domain::validate_schema_version(index.schema_version)?;
        let mut index = index.clone();
        index.image_count = index.images_by_hash.len();
        write_json_atomic(&self.images_index_path(), &index).await?;
        self.stats_cache.invalidate();
        self.assignment_availability_cache.invalidate();
        Ok(())
    }

    async fn read_image_count_hint(&self) -> StorageResult<Option<usize>> {
        let path = self.images_index_path();
        let mut file = tokio::fs::File::open(&path).await.with_path(&path)?;
        let mut buffer = vec![0; 4096];
        let read = file.read(&mut buffer).await.with_path(&path)?;
        let prefix = String::from_utf8_lossy(&buffer[..read]);
        Ok(extract_image_count_hint(&prefix))
    }

    pub async fn load_events(&self, image_id: &ImageId) -> StorageResult<Vec<EventLogEntry>> {
        let path = self.events_path(image_id);
        if !tokio::fs::try_exists(&path).await.with_path(&path)? {
            return Ok(Vec::new());
        }
        let text = tokio::fs::read_to_string(&path).await.with_path(&path)?;
        let events = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).with_json_path(&path))
            .collect::<StorageResult<Vec<EventLogEntry>>>()?;
        for event in &events {
            labello_domain::validate_supported_schema_version(event.schema_version)?;
        }
        Ok(events)
    }

    pub async fn rebuild_image_state(&self, image_id: &ImageId) -> StorageResult<ImageState> {
        let events = self.load_events(image_id).await?;
        let state = rebuild_state(image_id.clone(), &events)?;
        write_json_atomic(&self.state_path(image_id), &state).await?;
        if events.iter().any(stats_relevant_event) {
            self.stats_cache.invalidate();
        }
        self.assignment_availability_cache.invalidate();
        Ok(state)
    }

    pub async fn load_image_state(&self, image_id: &ImageId) -> StorageResult<ImageState> {
        #[cfg(test)]
        self.image_state_loads.fetch_add(1, Ordering::Relaxed);
        self.ensure_artifact_migration().await?;
        let path = self.state_path(image_id);
        let cache_exists = tokio::fs::try_exists(&path).await.with_path(&path)?;
        let cached = if cache_exists {
            let schema_version = read_schema_version(&path).await?;
            labello_domain::validate_supported_schema_version(schema_version)?;
            if schema_version == SCHEMA_VERSION {
                Some(read_current_json::<ImageState>(&path).await?)
            } else {
                None
            }
        } else {
            None
        };
        let events = self.load_events(image_id).await?;
        let event_sequence = events
            .last()
            .map(|event| event.event_sequence)
            .unwrap_or_default();
        if let Some(state) = cached.as_ref()
            && state.image_id == *image_id
            && state.current_sequence == event_sequence
        {
            return Ok(state.clone());
        }
        let state = rebuild_state(image_id.clone(), &events)?;
        if cache_exists || !events.is_empty() {
            tracing::warn!(
                event = "image_state.cache.rebuilt",
                image_id = %image_id,
                cached = cached.is_some(),
                event_sequence,
                "image state cache rebuilt from events"
            );
            write_json_atomic(&path, &state).await?;
        }
        Ok(state)
    }

    #[cfg(test)]
    pub(crate) fn reset_image_state_load_count(&self) {
        self.image_state_loads.store(0, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) fn image_state_load_count(&self) -> u64 {
        self.image_state_loads.load(Ordering::Relaxed)
    }

    pub async fn append_payload(
        &self,
        image_id: &ImageId,
        actor: &Actor,
        payload: EventPayload,
    ) -> StorageResult<EventLogEntry> {
        let lock = self.image_lock(image_id);
        let _guard = lock.lock().await;
        self.append_payloads_unlocked(image_id, actor, vec![payload])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| StorageError::Unauthorized("no payload was appended".to_string()))
    }

    pub(crate) async fn append_payloads_unlocked(
        &self,
        image_id: &ImageId,
        actor: &Actor,
        payloads: Vec<EventPayload>,
    ) -> StorageResult<Vec<EventLogEntry>> {
        let (events, _) = self
            .append_payloads_with_state_unlocked(image_id, actor, payloads)
            .await?;
        Ok(events)
    }

    pub(crate) async fn append_payloads_with_state_unlocked(
        &self,
        image_id: &ImageId,
        actor: &Actor,
        mut payloads: Vec<EventPayload>,
    ) -> StorageResult<(Vec<EventLogEntry>, ImageState)> {
        let mut next_state = self.load_image_state(image_id).await?;
        let timestamp = now();
        crate::assignment::append_guide_invalidation_payloads(
            &next_state,
            &mut payloads,
            timestamp,
        );
        let mut events = Vec::with_capacity(payloads.len());
        for payload in payloads {
            let event = EventLogEntry::new(
                next_state.current_sequence + 1,
                image_id.clone(),
                actor.user_id.clone(),
                actor.role.clone(),
                timestamp,
                payload,
            );
            next_state.apply_event(&event)?;
            events.push(event);
        }
        self.append_events_atomic(image_id, &events).await?;
        write_json_atomic(&self.state_path(image_id), &next_state).await?;
        if events.iter().any(stats_relevant_event) {
            self.stats_cache.invalidate();
        }
        self.assignment_availability_cache.invalidate();
        Ok((events, next_state))
    }

    pub(crate) async fn append_resequenced_events(
        &self,
        image_id: &ImageId,
        state: &mut ImageState,
        events: &[EventLogEntry],
    ) -> StorageResult<usize> {
        let mut next_state = state.clone();
        let mut resequenced = Vec::with_capacity(events.len());
        for original in events {
            let mut event = original.clone();
            event.event_sequence = next_state.current_sequence + 1;
            event.image_id = image_id.clone();
            next_state.apply_event(&event)?;
            resequenced.push(event);
        }
        self.append_events_atomic(image_id, &resequenced).await?;
        *state = next_state;
        write_json_atomic(&self.state_path(image_id), state).await?;
        if resequenced.iter().any(stats_relevant_event) {
            self.stats_cache.invalidate();
        }
        self.assignment_availability_cache.invalidate();
        Ok(events.len())
    }

    pub(crate) fn image_lock(&self, image_id: &ImageId) -> Arc<AsyncMutex<()>> {
        self.locks
            .lock()
            .entry(image_id.clone())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    async fn append_events_atomic(
        &self,
        image_id: &ImageId,
        events: &[EventLogEntry],
    ) -> StorageResult<()> {
        if events.is_empty() {
            return Ok(());
        }
        let path = self.events_path(image_id);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.with_path(parent)?;
        }
        let temporary = path.with_extension(format!("tmp-{}", uuid::Uuid::now_v7().simple()));
        let existing = if tokio::fs::try_exists(&path).await.with_path(&path)? {
            tokio::fs::read(&path).await.with_path(&path)?
        } else {
            Vec::new()
        };
        let mut file = tokio::fs::File::create(&temporary)
            .await
            .with_path(&temporary)?;
        file.write_all(&existing).await.with_path(&temporary)?;
        if !existing.is_empty() && !existing.ends_with(b"\n") {
            file.write_all(b"\n").await.with_path(&temporary)?;
        }
        for event in events {
            let line = serde_json::to_string(event).with_json_path(&path)?;
            file.write_all(line.as_bytes())
                .await
                .with_path(&temporary)?;
            file.write_all(b"\n").await.with_path(&temporary)?;
        }
        file.sync_all().await.with_path(&temporary)?;
        drop(file);
        tokio::fs::rename(&temporary, &path)
            .await
            .with_path(&path)?;
        if let Some(parent) = path.parent() {
            tokio::fs::File::open(parent)
                .await
                .with_path(parent)?
                .sync_all()
                .await
                .with_path(parent)?;
        }
        Ok(())
    }

    pub(crate) async fn ensure_artifact_migration(&self) -> StorageResult<()> {
        if self.migration_complete.load(Ordering::Acquire) {
            return Ok(());
        }
        let _guard = self.migration_lock.lock().await;
        if self.migration_complete.load(Ordering::Acquire) {
            return Ok(());
        }
        let journal_path = self.artifact_migration_journal_path();
        let mut journal = if tokio::fs::try_exists(&journal_path)
            .await
            .with_path(&journal_path)?
        {
            let journal: ArtifactMigrationJournal = read_json(&journal_path).await?;
            validate_artifact_migration_journal(&journal)?;
            if journal.phase == ArtifactMigrationPhase::Completed {
                self.migration_complete.store(true, Ordering::Release);
                return Ok(());
            }
            journal
        } else {
            if !tokio::fs::try_exists(self.dataset_path())
                .await
                .with_path(self.dataset_path())?
            {
                return Ok(());
            }
            let config: DatasetConfig = read_toml(&self.dataset_path()).await?;
            labello_domain::validate_supported_schema_version(config.schema_version)?;
            if config.schema_version == SCHEMA_VERSION && !self.has_legacy_artifacts().await? {
                self.migration_complete.store(true, Ordering::Release);
                return Ok(());
            }
            self.prepare_artifact_migration(config).await?
        };

        if journal.phase < ArtifactMigrationPhase::DatasetConfigPublished {
            self.publish_migration_kind(&journal, ArtifactMigrationKind::DatasetConfig)
                .await?;
            self.record_migration_phase(
                &mut journal,
                ArtifactMigrationPhase::DatasetConfigPublished,
            )
            .await?;
        }
        if journal.phase < ArtifactMigrationPhase::ImagesIndexPublished {
            self.publish_migration_kind(&journal, ArtifactMigrationKind::ImagesIndex)
                .await?;
            self.record_migration_phase(&mut journal, ArtifactMigrationPhase::ImagesIndexPublished)
                .await?;
        }
        if journal.phase < ArtifactMigrationPhase::SchemaPublished {
            self.publish_migration_kind(&journal, ArtifactMigrationKind::Schema)
                .await?;
            self.record_migration_phase(&mut journal, ArtifactMigrationPhase::SchemaPublished)
                .await?;
        }
        if journal.phase < ArtifactMigrationPhase::KeybindingsPublished {
            self.publish_migration_kind(&journal, ArtifactMigrationKind::Keybindings)
                .await?;
            self.record_migration_phase(&mut journal, ArtifactMigrationPhase::KeybindingsPublished)
                .await?;
        }
        if journal.phase < ArtifactMigrationPhase::StatesRebuilt {
            self.rebuild_migrated_state_caches(&journal).await?;
            self.record_migration_phase(&mut journal, ArtifactMigrationPhase::StatesRebuilt)
                .await?;
        }
        if journal.phase < ArtifactMigrationPhase::Completed {
            self.record_migration_phase(&mut journal, ArtifactMigrationPhase::Completed)
                .await?;
        }
        self.stats_cache.invalidate();
        self.assignment_availability_cache.invalidate();
        self.migration_complete.store(true, Ordering::Release);
        Ok(())
    }

    async fn prepare_artifact_migration(
        &self,
        mut config: DatasetConfig,
    ) -> StorageResult<ArtifactMigrationJournal> {
        labello_domain::validate_supported_schema_version(config.schema_version)?;
        let timestamp = now();
        config.schema_version = SCHEMA_VERSION;
        config.updated_at = timestamp;
        if !config.migration_history.iter().any(|record| {
            record.from_version == LEGACY_SCHEMA_VERSION && record.to_version == SCHEMA_VERSION
        }) {
            config.migration_history.push(MigrationRecord {
                from_version: LEGACY_SCHEMA_VERSION,
                to_version: SCHEMA_VERSION,
                name: "schema-v2-to-v3-artifacts".to_string(),
                applied_at: timestamp,
            });
        }

        let mut index = if tokio::fs::try_exists(self.images_index_path())
            .await
            .with_path(self.images_index_path())?
        {
            let mut index: ImagesIndex = read_json(&self.images_index_path()).await?;
            labello_domain::validate_supported_schema_version(index.schema_version)?;
            index.schema_version = SCHEMA_VERSION;
            index
        } else {
            ImagesIndex::default()
        };
        index.image_count = index.images_by_hash.len();

        let generation = self.next_artifact_migration_generation().await?;
        let generation_dir = self.artifact_migration_generation_dir(generation);
        create_dir_all_synced(&generation_dir).await?;
        let mut files = Vec::new();

        let staged_config = generation_dir.join(paths::DATASET_FILE);
        write_toml_atomic(&staged_config, &config).await?;
        push_migration_file(
            &mut files,
            ArtifactMigrationKind::DatasetConfig,
            paths::DATASET_FILE,
            &staged_config,
        )
        .await?;

        let staged_index = generation_dir.join(paths::IMAGES_INDEX_FILE);
        write_json_atomic(&staged_index, &index).await?;
        push_migration_file(
            &mut files,
            ArtifactMigrationKind::ImagesIndex,
            paths::IMAGES_INDEX_FILE,
            &staged_index,
        )
        .await?;

        let staged_schema = generation_dir.join(paths::SCHEMA_FILE);
        write_json_atomic(&staged_schema, &labello_schema_bundle()).await?;
        push_migration_file(
            &mut files,
            ArtifactMigrationKind::Schema,
            paths::SCHEMA_FILE,
            &staged_schema,
        )
        .await?;

        for (relative_path, keybindings) in self.legacy_keybindings().await? {
            let staged = generation_dir.join(&relative_path);
            write_toml_atomic(&staged, &keybindings).await?;
            push_migration_file(
                &mut files,
                ArtifactMigrationKind::Keybindings,
                &relative_path,
                &staged,
            )
            .await?;
        }
        files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

        let mut journal = ArtifactMigrationJournal {
            journal_version: ARTIFACT_MIGRATION_JOURNAL_VERSION,
            from_version: LEGACY_SCHEMA_VERSION,
            to_version: SCHEMA_VERSION,
            generation,
            phase: ArtifactMigrationPhase::GenerationPrepared,
            files,
            phase_history: Vec::new(),
            started_at: timestamp,
            updated_at: timestamp,
        };
        journal.record_phase(ArtifactMigrationPhase::GenerationPrepared, timestamp);
        write_json_atomic(&self.artifact_migration_journal_path(), &journal).await?;
        self.maybe_fail_artifact_migration(ArtifactMigrationPhase::GenerationPrepared)?;
        Ok(journal)
    }

    async fn legacy_keybindings(&self) -> StorageResult<Vec<(String, KeybindingSet)>> {
        let users = self.root.join(paths::USERS_DIR);
        if !tokio::fs::try_exists(&users).await.with_path(&users)? {
            return Ok(Vec::new());
        }
        let mut reader = tokio::fs::read_dir(&users).await.with_path(&users)?;
        let mut paths_to_read = Vec::new();
        while let Some(entry) = reader.next_entry().await.with_path(&users)? {
            if !entry.file_type().await.with_path(entry.path())?.is_dir() {
                continue;
            }
            let path = entry.path().join(paths::KEYBINDINGS_FILE);
            if tokio::fs::try_exists(&path).await.with_path(&path)? {
                paths_to_read.push(path);
            }
        }
        paths_to_read.sort();
        let mut keybindings = Vec::with_capacity(paths_to_read.len());
        for path in paths_to_read {
            let mut value: KeybindingSet = read_toml(&path).await?;
            labello_domain::validate_supported_schema_version(value.schema_version)?;
            let directory_name = path
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                .ok_or_else(|| StorageError::OutsideDatasetRoot(path.clone()))?;
            let directory_user_id = UserId::from(directory_name);
            directory_user_id
                .validate_path_segment()
                .map_err(|_| StorageError::OutsideDatasetRoot(path.clone()))?;
            if value.user_id != directory_user_id {
                return Err(StorageError::OutsideDatasetRoot(path));
            }
            value.schema_version = SCHEMA_VERSION;
            value.normalize();
            value.validate()?;
            let relative = path
                .strip_prefix(&*self.root)
                .map_err(|_| StorageError::OutsideDatasetRoot(path.clone()))?
                .to_str()
                .ok_or_else(|| StorageError::OutsideDatasetRoot(path.clone()))?
                .replace(std::path::MAIN_SEPARATOR, "/");
            keybindings.push((relative, value));
        }
        Ok(keybindings)
    }

    async fn has_legacy_artifacts(&self) -> StorageResult<bool> {
        if tokio::fs::try_exists(self.images_index_path())
            .await
            .with_path(self.images_index_path())?
        {
            let version = read_schema_version(&self.images_index_path()).await?;
            labello_domain::validate_supported_schema_version(version)?;
            if version == LEGACY_SCHEMA_VERSION {
                return Ok(true);
            }
        }
        if tokio::fs::try_exists(self.schema_path())
            .await
            .with_path(self.schema_path())?
        {
            let version = read_schema_version(&self.schema_path()).await?;
            labello_domain::validate_supported_schema_version(version)?;
            if version == LEGACY_SCHEMA_VERSION {
                return Ok(true);
            }
        }

        let users = self.root.join(paths::USERS_DIR);
        if !tokio::fs::try_exists(&users).await.with_path(&users)? {
            return Ok(false);
        }
        let mut reader = tokio::fs::read_dir(&users).await.with_path(&users)?;
        while let Some(entry) = reader.next_entry().await.with_path(&users)? {
            if !entry.file_type().await.with_path(entry.path())?.is_dir() {
                continue;
            }
            let keybindings = entry.path().join(paths::KEYBINDINGS_FILE);
            if tokio::fs::try_exists(&keybindings)
                .await
                .with_path(&keybindings)?
            {
                let value: KeybindingSet = read_toml(&keybindings).await?;
                labello_domain::validate_supported_schema_version(value.schema_version)?;
                if value.schema_version == LEGACY_SCHEMA_VERSION {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    async fn next_artifact_migration_generation(&self) -> StorageResult<u64> {
        let mut generation = now().timestamp_micros().unsigned_abs();
        while tokio::fs::try_exists(self.artifact_migration_generation_dir(generation))
            .await
            .with_path(self.artifact_migration_generation_dir(generation))?
        {
            generation += 1;
        }
        Ok(generation)
    }

    fn artifact_migration_generation_dir(&self, generation: u64) -> PathBuf {
        self.root
            .join(paths::ARTIFACT_MIGRATION_DIR)
            .join(paths::ARTIFACT_MIGRATION_GENERATIONS_DIR)
            .join(format!("{generation:020}"))
    }

    async fn publish_migration_kind(
        &self,
        journal: &ArtifactMigrationJournal,
        kind: ArtifactMigrationKind,
    ) -> StorageResult<()> {
        let files = journal
            .files
            .iter()
            .filter(|file| file.kind == kind)
            .cloned()
            .collect::<Vec<_>>();
        for file in files {
            validate_migration_relative_path(&file.relative_path)?;
            let staged = self
                .artifact_migration_generation_dir(journal.generation)
                .join(&file.relative_path);
            let bytes = tokio::fs::read(&staged).await.with_path(&staged)?;
            if blake3::hash(&bytes).to_hex().as_str() != file.blake3 {
                return Err(StorageError::BackgroundTask(format!(
                    "artifact migration generation {} failed integrity verification",
                    journal.generation
                )));
            }
            write_bytes_atomic(&self.root.join(&file.relative_path), &bytes).await?;
        }
        Ok(())
    }

    async fn record_migration_phase(
        &self,
        journal: &mut ArtifactMigrationJournal,
        phase: ArtifactMigrationPhase,
    ) -> StorageResult<()> {
        journal.record_phase(phase, now());
        write_json_atomic(&self.artifact_migration_journal_path(), journal).await?;
        self.maybe_fail_artifact_migration(phase)
    }

    async fn rebuild_migrated_state_caches(
        &self,
        journal: &ArtifactMigrationJournal,
    ) -> StorageResult<()> {
        let index_file = journal
            .files
            .iter()
            .find(|file| file.kind == ArtifactMigrationKind::ImagesIndex)
            .ok_or_else(|| {
                StorageError::BackgroundTask(
                    "artifact migration generation has no image index".to_string(),
                )
            })?;
        let staged_index = self
            .artifact_migration_generation_dir(journal.generation)
            .join(&index_file.relative_path);
        let index: ImagesIndex = read_current_json(&staged_index).await?;
        let mut image_ids = index
            .images_by_hash
            .values()
            .map(|record| record.image_id.clone())
            .collect::<BTreeSet<_>>();

        let annotations = self.root.join(paths::ANNOTATIONS_DIR);
        if tokio::fs::try_exists(&annotations)
            .await
            .with_path(&annotations)?
        {
            let mut reader = tokio::fs::read_dir(&annotations)
                .await
                .with_path(&annotations)?;
            while let Some(entry) = reader.next_entry().await.with_path(&annotations)? {
                if entry.file_type().await.with_path(entry.path())?.is_dir()
                    && let Some(name) = entry.file_name().to_str()
                    && !name.starts_with('.')
                {
                    let image_id = ImageId::from(name);
                    image_id
                        .validate_path_segment()
                        .map_err(|_| StorageError::OutsideDatasetRoot(entry.path()))?;
                    image_ids.insert(image_id);
                }
            }
        }

        for image_id in image_ids {
            let events = self.load_events(&image_id).await?;
            let state = rebuild_state(image_id.clone(), &events)?;
            write_json_atomic(&self.state_path(&image_id), &state).await?;
        }
        Ok(())
    }

    #[cfg(test)]
    fn fail_artifact_migration_after(&self, phase: ArtifactMigrationPhase) {
        *self.migration_failure.lock() = Some(phase);
    }

    #[cfg(test)]
    fn maybe_fail_artifact_migration(&self, phase: ArtifactMigrationPhase) -> StorageResult<()> {
        let mut failure = self.migration_failure.lock();
        if *failure == Some(phase) {
            *failure = None;
            Err(StorageError::BackgroundTask(format!(
                "injected artifact migration crash after {phase:?}"
            )))
        } else {
            Ok(())
        }
    }

    #[cfg(not(test))]
    fn maybe_fail_artifact_migration(&self, _phase: ArtifactMigrationPhase) -> StorageResult<()> {
        Ok(())
    }
}

async fn push_migration_file(
    files: &mut Vec<ArtifactMigrationFile>,
    kind: ArtifactMigrationKind,
    relative_path: &str,
    staged_path: &Path,
) -> StorageResult<()> {
    validate_migration_relative_path(relative_path)?;
    let bytes = tokio::fs::read(staged_path).await.with_path(staged_path)?;
    files.push(ArtifactMigrationFile {
        kind,
        relative_path: relative_path.to_string(),
        blake3: blake3::hash(&bytes).to_hex().to_string(),
    });
    Ok(())
}

fn validate_artifact_migration_journal(journal: &ArtifactMigrationJournal) -> StorageResult<()> {
    if journal.journal_version != ARTIFACT_MIGRATION_JOURNAL_VERSION
        || journal.from_version != LEGACY_SCHEMA_VERSION
        || journal.to_version != SCHEMA_VERSION
    {
        return Err(StorageError::BackgroundTask(
            "unsupported artifact migration journal".to_string(),
        ));
    }
    let expected_phases = [
        ArtifactMigrationPhase::GenerationPrepared,
        ArtifactMigrationPhase::DatasetConfigPublished,
        ArtifactMigrationPhase::ImagesIndexPublished,
        ArtifactMigrationPhase::SchemaPublished,
        ArtifactMigrationPhase::KeybindingsPublished,
        ArtifactMigrationPhase::StatesRebuilt,
        ArtifactMigrationPhase::Completed,
    ];
    let current_phase = expected_phases
        .iter()
        .position(|phase| *phase == journal.phase)
        .expect("all artifact migration phases are listed");
    if journal.generation == 0
        || journal
            .phase_history
            .iter()
            .map(|record| record.phase)
            .ne(expected_phases[..=current_phase].iter().copied())
    {
        return Err(StorageError::BackgroundTask(
            "invalid artifact migration phase history".to_string(),
        ));
    }

    let mut unique = BTreeSet::new();
    let mut core_counts = [0_u8; 3];
    for file in &journal.files {
        validate_migration_file(file)?;
        if !unique.insert(&file.relative_path) || blake3::Hash::from_hex(&file.blake3).is_err() {
            return Err(StorageError::BackgroundTask(
                "invalid artifact migration file manifest".to_string(),
            ));
        }
        match file.kind {
            ArtifactMigrationKind::DatasetConfig => core_counts[0] += 1,
            ArtifactMigrationKind::ImagesIndex => core_counts[1] += 1,
            ArtifactMigrationKind::Schema => core_counts[2] += 1,
            ArtifactMigrationKind::Keybindings => {}
        }
    }
    if core_counts != [1, 1, 1] {
        return Err(StorageError::BackgroundTask(
            "artifact migration journal is missing a core artifact".to_string(),
        ));
    }
    Ok(())
}

fn validate_migration_file(file: &ArtifactMigrationFile) -> StorageResult<()> {
    validate_migration_relative_path(&file.relative_path)?;
    let valid = match file.kind {
        ArtifactMigrationKind::DatasetConfig => file.relative_path == paths::DATASET_FILE,
        ArtifactMigrationKind::ImagesIndex => file.relative_path == paths::IMAGES_INDEX_FILE,
        ArtifactMigrationKind::Schema => file.relative_path == paths::SCHEMA_FILE,
        ArtifactMigrationKind::Keybindings => {
            let components = Path::new(&file.relative_path)
                .components()
                .collect::<Vec<_>>();
            components.len() == 3
                && components[0].as_os_str() == paths::USERS_DIR
                && components[2].as_os_str() == paths::KEYBINDINGS_FILE
                && components[1]
                    .as_os_str()
                    .to_str()
                    .is_some_and(|value| UserId::from(value).validate_path_segment().is_ok())
        }
    };
    if valid {
        Ok(())
    } else {
        Err(StorageError::OutsideDatasetRoot(PathBuf::from(
            &file.relative_path,
        )))
    }
}

fn validate_migration_relative_path(relative_path: &str) -> StorageResult<()> {
    let path = Path::new(relative_path);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        Err(StorageError::OutsideDatasetRoot(path.to_path_buf()))
    } else {
        Ok(())
    }
}

fn stats_relevant_event(event: &EventLogEntry) -> bool {
    !matches!(&event.payload, EventPayload::AssignmentUpdated { .. })
}

async fn write_snapshot_bytes(
    root: &Path,
    relative_path: &str,
    bytes: &[u8],
    files: &mut Vec<SnapshotFileEntry>,
) -> StorageResult<()> {
    let path = root.join(relative_path);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.with_path(parent)?;
    }
    tokio::fs::write(&path, bytes).await.with_path(&path)?;
    files.push(SnapshotFileEntry {
        path: relative_path.to_string(),
        byte_size: bytes.len() as u64,
        blake3: blake3::hash(bytes).to_hex().to_string(),
    });
    Ok(())
}

fn validate_snapshot_segment(value: &str) -> StorageResult<()> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
    {
        Err(StorageError::OutsideDatasetRoot(PathBuf::from(value)))
    } else {
        Ok(())
    }
}

fn extract_image_count_hint(text: &str) -> Option<usize> {
    let key = "\"imageCount\"";
    let rest = text.get(text.find(key)? + key.len()..)?;
    let rest = rest.get(rest.find(':')? + 1..)?.trim_start();
    let digits = rest
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

#[cfg(test)]
mod tests;
