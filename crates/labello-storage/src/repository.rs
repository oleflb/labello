use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use labello_domain::{
    Actor, DatasetConfig, DatasetMetadata, EventLogEntry, EventPayload, ImageId, ImageRecord,
    ImageState, ImagesIndex, SCHEMA_VERSION, labello_schema_bundle, now, rebuild_state,
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
};

#[derive(Clone, Debug)]
pub struct DatasetRepository {
    root: Arc<PathBuf>,
    locks: Arc<Mutex<BTreeMap<ImageId, Arc<AsyncMutex<()>>>>>,
}

impl DatasetRepository {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: Arc::new(root.into()),
            locks: Arc::new(Mutex::new(BTreeMap::new())),
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
        .await
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
        write_json_atomic(&self.images_index_path(), &index).await
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
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).with_json_path(&path))
            .collect()
    }

    pub async fn rebuild_image_state(&self, image_id: &ImageId) -> StorageResult<ImageState> {
        let events = self.load_events(image_id).await?;
        let state = rebuild_state(image_id.clone(), &events)?;
        write_json_atomic(&self.state_path(image_id), &state).await?;
        Ok(state)
    }

    pub async fn load_image_state(&self, image_id: &ImageId) -> StorageResult<ImageState> {
        let path = self.state_path(image_id);
        if tokio::fs::try_exists(&path).await.with_path(&path)? {
            let state: ImageState = read_json(&path).await?;
            labello_domain::validate_schema_version(state.schema_version)?;
            Ok(state)
        } else {
            rebuild_state(image_id.clone(), &self.load_events(image_id).await?)
                .map_err(StorageError::from)
        }
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
        for event in &events {
            self.append_event_line(event).await?;
        }
        write_json_atomic(&self.state_path(image_id), &next_state).await?;
        Ok(events)
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
        for event in &resequenced {
            self.append_event_line(event).await?;
        }
        *state = next_state;
        write_json_atomic(&self.state_path(image_id), state).await?;
        Ok(events.len())
    }

    pub(crate) fn image_lock(&self, image_id: &ImageId) -> Arc<AsyncMutex<()>> {
        self.locks
            .lock()
            .entry(image_id.clone())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    async fn append_event_line(&self, event: &EventLogEntry) -> StorageResult<()> {
        let path = self.events_path(&event.image_id);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.with_path(parent)?;
        }
        let line = serde_json::to_string(event).with_json_path(&path)?;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .with_path(&path)?;
        file.write_all(line.as_bytes()).await.with_path(&path)?;
        file.write_all(b"\n").await.with_path(&path)?;
        file.flush().await.with_path(&path)?;
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
