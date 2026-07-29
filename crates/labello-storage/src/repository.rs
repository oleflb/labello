use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use labello_domain::{
    Actor, DatasetConfig, DatasetMetadata, DatasetSnapshot, EventLogEntry, EventPayload, ImageId,
    ImageRecord, ImageState, ImagesIndex, SCHEMA_VERSION, SnapshotFileEntry, labello_schema_bundle,
    now, rebuild_state,
};
use parking_lot::Mutex;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex as AsyncMutex,
};

use crate::{
    error::{PathIo, PathJson, PathTomlEncode, StorageError, StorageResult},
    fsjson::{read_json, write_json_atomic},
    fstoml::{read_toml, write_toml_atomic},
    paths,
    stats::StatsCache,
};

#[derive(Clone, Debug)]
pub struct DatasetRepository {
    root: Arc<PathBuf>,
    locks: Arc<Mutex<BTreeMap<ImageId, Arc<AsyncMutex<()>>>>>,
    pub(crate) assignment_cursors: Arc<Mutex<BTreeMap<String, usize>>>,
    pub(crate) stats_cache: Arc<StatsCache>,
}

impl DatasetRepository {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: Arc::new(root.into()),
            locks: Arc::new(Mutex::new(BTreeMap::new())),
            assignment_cursors: Arc::new(Mutex::new(BTreeMap::new())),
            stats_cache: Arc::new(StatsCache::default()),
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
            for image in index.images_by_hash.values() {
                let events = self.load_events(&image.image_id).await?;
                let events_text = events
                    .iter()
                    .map(serde_json::to_string)
                    .collect::<Result<Vec<_>, _>>()
                    .with_json_path(self.events_path(&image.image_id))?
                    .join("\n");
                let events_text = if events_text.is_empty() {
                    String::new()
                } else {
                    format!("{events_text}\n")
                };
                let relative = format!(
                    "{}/{}/{}",
                    paths::ANNOTATIONS_DIR,
                    image.image_id,
                    paths::EVENTS_FILE
                );
                write_snapshot_bytes(&temporary, &relative, events_text.as_bytes(), &mut files)
                    .await?;

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
        if result.is_err() {
            let _ = tokio::fs::remove_dir_all(&temporary).await;
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
                snapshots.push(read_json(&manifest_path).await?);
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
            read_json(&snapshot.join(paths::SNAPSHOT_MANIFEST_FILE)).await?;
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
        let config: DatasetConfig = read_toml(&self.dataset_path()).await?;
        labello_domain::validate_schema_version(config.schema_version)?;
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
        let config: DatasetConfig = read_toml(&self.dataset_path()).await?;
        labello_domain::validate_schema_version(config.schema_version)?;
        Ok(config.into_metadata(BTreeMap::new()))
    }

    pub async fn save_dataset(&self, metadata: &DatasetMetadata) -> StorageResult<()> {
        labello_domain::validate_schema_version(metadata.schema_version)?;
        write_toml_atomic(
            &self.dataset_path(),
            &DatasetConfig::from_metadata(metadata),
        )
        .await?;
        self.stats_cache.invalidate();
        Ok(())
    }

    pub async fn load_images_index(&self) -> StorageResult<ImagesIndex> {
        if !tokio::fs::try_exists(self.images_index_path())
            .await
            .with_path(self.images_index_path())?
        {
            return Ok(ImagesIndex::default());
        }
        let index: ImagesIndex = read_json(&self.images_index_path()).await?;
        labello_domain::validate_schema_version(index.schema_version)?;
        Ok(index)
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
        labello_domain::validate_schema_version(index.schema_version)?;
        let mut index = index.clone();
        index.image_count = index.images_by_hash.len();
        write_json_atomic(&self.images_index_path(), &index).await?;
        self.stats_cache.invalidate();
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
            labello_domain::validate_schema_version(event.schema_version)?;
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
        Ok(state)
    }

    pub async fn load_image_state(&self, image_id: &ImageId) -> StorageResult<ImageState> {
        let path = self.state_path(image_id);
        let cached = if tokio::fs::try_exists(&path).await.with_path(&path)? {
            let state: ImageState = read_json(&path).await?;
            labello_domain::validate_schema_version(state.schema_version)?;
            Some(state)
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
        if cached.is_some() || !events.is_empty() {
            write_json_atomic(&path, &state).await?;
        }
        Ok(state)
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
        payloads: Vec<EventPayload>,
    ) -> StorageResult<(Vec<EventLogEntry>, ImageState)> {
        let mut next_state = self.load_image_state(image_id).await?;
        let mut events = Vec::with_capacity(payloads.len());
        for payload in payloads {
            let event = EventLogEntry::new(
                next_state.current_sequence + 1,
                image_id.clone(),
                actor.user_id.clone(),
                actor.role.clone(),
                now(),
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
mod tests {
    use labello_domain::{
        Actor, DatasetId, DatasetMetadata, DatasetRole, EventPayload, ImageId, TaskId, TaskState,
        UserId, now,
    };

    use super::*;

    #[tokio::test]
    async fn appends_events_and_rebuilds_state() {
        let temp = tempfile::tempdir().unwrap();
        let repo = DatasetRepository::new(temp.path());
        repo.initialize(DatasetMetadata::new(
            DatasetId::from("ds"),
            "Dataset",
            now(),
        ))
        .await
        .unwrap();
        let image_id = ImageId::from("img_test");
        let actor = Actor {
            user_id: UserId::from("user_1"),
            role: DatasetRole::Annotator,
        };
        let task_state = TaskState::new(TaskId::from("bounding_box:person"), now());

        let event = repo
            .append_payload(
                &image_id,
                &actor,
                EventPayload::TaskStateChanged { task_state },
            )
            .await
            .unwrap();
        assert_eq!(event.event_sequence, 1);

        let events = repo.load_events(&image_id).await.unwrap();
        assert_eq!(events.len(), 1);
        let rebuilt = repo.rebuild_image_state(&image_id).await.unwrap();
        assert_eq!(rebuilt.current_sequence, 1);
    }

    #[tokio::test]
    async fn loading_missing_image_state_does_not_create_state_file() {
        let temp = tempfile::tempdir().unwrap();
        let repo = DatasetRepository::new(temp.path());
        repo.initialize(DatasetMetadata::new(
            DatasetId::from("ds"),
            "Dataset",
            now(),
        ))
        .await
        .unwrap();
        let image_id = ImageId::from("img_empty");

        let state = repo.load_image_state(&image_id).await.unwrap();

        assert_eq!(state.current_sequence, 0);
        assert!(
            !tokio::fs::try_exists(repo.state_path(&image_id))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn rejects_reinitializing_an_existing_dataset() {
        let temp = tempfile::tempdir().unwrap();
        let repo = DatasetRepository::new(temp.path());
        repo.initialize(DatasetMetadata::new(
            DatasetId::from("ds"),
            "Original",
            now(),
        ))
        .await
        .unwrap();

        let error = repo
            .initialize(DatasetMetadata::new(
                DatasetId::from("ds"),
                "Replacement",
                now(),
            ))
            .await
            .unwrap_err();

        assert!(matches!(error, StorageError::AlreadyExists(_)));
        assert_eq!(repo.load_dataset_config().await.unwrap().name, "Original");
    }

    #[tokio::test]
    async fn validates_event_against_cloned_state_before_append() {
        let temp = tempfile::tempdir().unwrap();
        let repo = DatasetRepository::new(temp.path());
        repo.initialize(DatasetMetadata::new(
            DatasetId::from("ds"),
            "Dataset",
            now(),
        ))
        .await
        .unwrap();
        let image_id = ImageId::from("img_test");
        let actor = Actor {
            user_id: UserId::from("user_1"),
            role: DatasetRole::Annotator,
        };

        let error = repo
            .append_payload(
                &image_id,
                &actor,
                EventPayload::AnnotationDeleted {
                    annotation_id: labello_domain::AnnotationId::from("ann_missing"),
                    version: 1,
                    reason: None,
                },
            )
            .await
            .unwrap_err();

        assert!(matches!(error, StorageError::Domain(_)));
        assert!(repo.load_events(&image_id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn validates_entire_resequenced_batch_before_append() {
        let temp = tempfile::tempdir().unwrap();
        let repo = DatasetRepository::new(temp.path());
        repo.initialize(DatasetMetadata::new(
            DatasetId::from("ds"),
            "Dataset",
            now(),
        ))
        .await
        .unwrap();
        let image_id = ImageId::from("img_test");
        let user_id = UserId::from("user_1");
        let mut state = repo.load_image_state(&image_id).await.unwrap();
        let events = vec![
            EventLogEntry::new(
                1,
                image_id.clone(),
                user_id.clone(),
                DatasetRole::Annotator,
                now(),
                EventPayload::TaskStateChanged {
                    task_state: TaskState::new(TaskId::from("task_1"), now()),
                },
            ),
            EventLogEntry::new(
                2,
                image_id.clone(),
                user_id,
                DatasetRole::Annotator,
                now(),
                EventPayload::AnnotationDeleted {
                    annotation_id: labello_domain::AnnotationId::from("ann_missing"),
                    version: 1,
                    reason: None,
                },
            ),
        ];

        let error = repo
            .append_resequenced_events(&image_id, &mut state, &events)
            .await
            .unwrap_err();

        assert!(matches!(error, StorageError::Domain(_)));
        assert!(repo.load_events(&image_id).await.unwrap().is_empty());
        assert_eq!(state.current_sequence, 0);
    }

    #[tokio::test]
    async fn load_recovers_state_left_behind_a_complete_event_batch() {
        let temp = tempfile::tempdir().unwrap();
        let repo = DatasetRepository::new(temp.path());
        repo.initialize(DatasetMetadata::new(
            DatasetId::from("ds"),
            "Dataset",
            now(),
        ))
        .await
        .unwrap();
        let image_id = ImageId::from("img_recovery");
        let actor = Actor {
            user_id: UserId::from("user_1"),
            role: DatasetRole::Annotator,
        };
        repo.append_payload(
            &image_id,
            &actor,
            EventPayload::TaskStateChanged {
                task_state: TaskState::new(TaskId::from("task_1"), now()),
            },
        )
        .await
        .unwrap();
        let stale = repo.load_image_state(&image_id).await.unwrap();

        let lock = repo.image_lock(&image_id);
        let _guard = lock.lock().await;
        repo.append_payloads_unlocked(
            &image_id,
            &actor,
            vec![
                EventPayload::TaskStateChanged {
                    task_state: TaskState::new(TaskId::from("task_2"), now()),
                },
                EventPayload::TaskStateChanged {
                    task_state: TaskState::new(TaskId::from("task_3"), now()),
                },
            ],
        )
        .await
        .unwrap();
        write_json_atomic(&repo.state_path(&image_id), &stale)
            .await
            .unwrap();
        drop(_guard);

        let recovered = repo.load_image_state(&image_id).await.unwrap();
        assert_eq!(recovered.current_sequence, 3);
        assert_eq!(recovered.task_states.len(), 3);
        assert_eq!(
            read_json::<ImageState>(&repo.state_path(&image_id))
                .await
                .unwrap(),
            recovered
        );
    }

    #[test]
    fn rejects_image_path_traversal() {
        let repo = DatasetRepository::new("/tmp/labello-dataset");
        assert!(repo.image_path("images/frame.png").is_ok());
        assert!(repo.image_path("../secret.png").is_err());
        assert!(repo.image_path("/etc/passwd").is_err());
    }

    #[test]
    fn extracts_image_count_hint_from_index_prefix() {
        assert_eq!(
            extract_image_count_hint(r#"{"schemaVersion":1,"imageCount":42,"imagesByHash":{}}"#),
            Some(42)
        );
        assert_eq!(extract_image_count_hint(r#"{"imagesByHash":{}}"#), None);
    }
}
