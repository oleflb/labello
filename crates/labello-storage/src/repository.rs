use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use labello_domain::{
    Actor, DatasetMetadata, EventLogEntry, EventPayload, ImageId, ImageState, ImagesIndex,
    SCHEMA_VERSION, labello_schema_bundle, now, rebuild_state,
};
use parking_lot::Mutex;
use tokio::{io::AsyncWriteExt, sync::Mutex as AsyncMutex};

use crate::{
    error::{PathIo, PathJson, StorageError, StorageResult},
    fsjson::{read_json, write_json_atomic},
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
        self.save_dataset(&metadata).await?;
        self.save_images_index(&ImagesIndex::default()).await?;
        write_json_atomic(&self.schema_path(), &labello_schema_bundle()).await?;
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
        let metadata: DatasetMetadata = read_json(&self.dataset_path()).await?;
        labello_domain::validate_schema_version(metadata.schema_version)?;
        Ok(metadata)
    }

    pub async fn save_dataset(&self, metadata: &DatasetMetadata) -> StorageResult<()> {
        labello_domain::validate_schema_version(metadata.schema_version)?;
        write_json_atomic(&self.dataset_path(), metadata).await
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

    pub async fn save_images_index(&self, index: &ImagesIndex) -> StorageResult<()> {
        labello_domain::validate_schema_version(index.schema_version)?;
        write_json_atomic(&self.images_index_path(), index).await
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
            self.rebuild_image_state(image_id).await
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
        let mut state = self.load_image_state(image_id).await?;
        let event = EventLogEntry::new(
            state.current_sequence + 1,
            image_id.clone(),
            actor.user_id.clone(),
            actor.role.clone(),
            now(),
            payload,
        );
        self.append_event_line(&event).await?;
        state.apply_event(&event)?;
        write_json_atomic(&self.state_path(image_id), &state).await?;
        Ok(event)
    }

    pub(crate) async fn append_resequenced_events(
        &self,
        image_id: &ImageId,
        state: &mut ImageState,
        events: &[EventLogEntry],
    ) -> StorageResult<usize> {
        let mut merged = 0;
        for original in events {
            let mut event = original.clone();
            event.event_sequence = state.current_sequence + 1;
            event.image_id = image_id.clone();
            self.append_event_line(&event).await?;
            state.apply_event(&event)?;
            merged += 1;
        }
        write_json_atomic(&self.state_path(image_id), state).await?;
        Ok(merged)
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

    #[test]
    fn rejects_image_path_traversal() {
        let repo = DatasetRepository::new("/tmp/labello-dataset");
        assert!(repo.image_path("images/frame.png").is_ok());
        assert!(repo.image_path("../secret.png").is_err());
        assert!(repo.image_path("/etc/passwd").is_err());
    }
}
