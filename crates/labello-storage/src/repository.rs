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
    ARTIFACT_MIGRATION_JOURNAL_VERSION, ArtifactMigrationFile, ArtifactMigrationJournal,
    ArtifactMigrationKind, ArtifactMigrationPhase, DatasetConfig, DatasetMetadata, DatasetSnapshot,
    EventLogEntry, EventPayload, ImageId, ImageRecord, ImageState, ImagesIndex, ImportId,
    ImportManifest, KeybindingSet, LEGACY_SCHEMA_VERSION, MigrationRecord, SCHEMA_VERSION,
    SnapshotFileEntry, SnapshotImportEntry, TaskId, UserId, labello_schema_bundle, now,
    rebuild_state,
};
use parking_lot::Mutex;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Mutex as AsyncMutex, RwLock as AsyncRwLock},
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

mod artifact_migration;
mod cache;
mod config;
mod events;
mod layout;
mod locks;
mod snapshots;

use cache::AssignmentAvailabilityCache;
#[cfg(test)]
use config::extract_image_count_hint;
pub(crate) use events::stats_relevant_event;

#[derive(Clone, Debug)]
pub struct DatasetRepository {
    root: Arc<PathBuf>,
    locks: Arc<Mutex<BTreeMap<ImageId, Arc<AsyncMutex<()>>>>>,
    migration_lock: Arc<AsyncMutex<()>>,
    migration_complete: Arc<AtomicBool>,
    images_index_cache: Arc<AsyncRwLock<Option<Arc<ImagesIndex>>>>,
    pub(crate) stats_cache: Arc<StatsCache>,
    pub(crate) assignment_availability_cache: Arc<AssignmentAvailabilityCache>,
    #[cfg(test)]
    migration_failure: Arc<Mutex<Option<ArtifactMigrationPhase>>>,
    #[cfg(test)]
    image_state_loads: Arc<AtomicU64>,
    #[cfg(test)]
    images_index_loads: Arc<AtomicU64>,
}

impl DatasetRepository {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: Arc::new(root.into()),
            locks: Arc::new(Mutex::new(BTreeMap::new())),
            migration_lock: Arc::new(AsyncMutex::new(())),
            migration_complete: Arc::new(AtomicBool::new(false)),
            images_index_cache: Arc::new(AsyncRwLock::new(None)),
            stats_cache: Arc::new(StatsCache::default()),
            assignment_availability_cache: Arc::new(AssignmentAvailabilityCache::default()),
            #[cfg(test)]
            migration_failure: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            image_state_loads: Arc::new(AtomicU64::new(0)),
            #[cfg(test)]
            images_index_loads: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[cfg(test)]
mod tests;
