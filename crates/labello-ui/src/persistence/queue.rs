pub(crate) trait DraftStore {
    fn get<'a>(&'a self, key: &'a str) -> StoreFuture<'a, Option<DraftRecord>>;
    fn put<'a>(&'a self, record: DraftRecord) -> StoreFuture<'a, ()>;
    fn delete<'a>(&'a self, key: &'a str) -> StoreFuture<'a, ()>;
    fn garbage_collect<'a>(&'a self, now: Timestamp) -> StoreFuture<'a, usize>;
}

#[derive(Clone, Debug)]
pub(crate) enum PersistenceCommand {
    Load(String),
    Save(Box<DraftRecord>),
    Delete(String),
    GarbageCollect(Timestamp),
}

#[derive(Clone, Debug)]
pub(crate) struct QueuedPersistenceCommand {
    identity: StorageIdentity,
    command: PersistenceCommand,
    attempt: u8,
    ready_at: Instant,
}

impl QueuedPersistenceCommand {
    fn key(&self) -> Option<&str> {
        match &self.command {
            PersistenceCommand::Load(key) | PersistenceCommand::Delete(key) => Some(key),
            PersistenceCommand::Save(record) => Some(record.key()),
            PersistenceCommand::GarbageCollect(_) => None,
        }
    }
}

#[derive(Debug)]
pub(crate) enum PersistenceCompletion {
    Loaded {
        command: QueuedPersistenceCommand,
        result: Box<Result<Option<DraftRecord>, String>>,
    },
    Saved {
        command: QueuedPersistenceCommand,
        result: Result<(), String>,
    },
    Deleted {
        command: QueuedPersistenceCommand,
        result: Result<(), String>,
    },
    GarbageCollected {
        command: QueuedPersistenceCommand,
        result: Result<usize, String>,
    },
}

impl PersistenceCompletion {
    fn command(&self) -> &QueuedPersistenceCommand {
        match self {
            Self::Loaded { command, .. }
            | Self::Saved { command, .. }
            | Self::Deleted { command, .. }
            | Self::GarbageCollected { command, .. } => command,
        }
    }
}

async fn execute_persistence_command(
    store: Rc<dyn DraftStore>,
    command: QueuedPersistenceCommand,
) -> PersistenceCompletion {
    match &command.command {
        PersistenceCommand::Load(key) => {
            let result = store.get(key).await;
            PersistenceCompletion::Loaded {
                command,
                result: Box::new(result),
            }
        }
        PersistenceCommand::Save(record) => {
            let result = store.put((**record).clone()).await;
            PersistenceCompletion::Saved { command, result }
        }
        PersistenceCommand::Delete(key) => {
            let result = store.delete(key).await;
            PersistenceCompletion::Deleted { command, result }
        }
        PersistenceCommand::GarbageCollect(now) => {
            let result = store.garbage_collect(*now).await;
            PersistenceCompletion::GarbageCollected { command, result }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum DraftRecovery {
    Work(Box<WorkDraft>, DraftValidation),
    Admin(Box<AdminDraft>, DraftValidation),
}

pub(crate) struct PersistenceState {
    pub store: Rc<dyn DraftStore>,
    pub commands: VecDeque<QueuedPersistenceCommand>,
    pub active: bool,
    pub identity: Option<StorageIdentity>,
    pub preference: Option<WorkspacePreference>,
    pub preference_encoded: Option<String>,
    preference_desired_encoded: Option<String>,
    preference_retry: RetryState,
    pub restoration_attempted: bool,
    pub expected_assignment: Option<AssignmentId>,
    pub recovery: Option<DraftRecovery>,
    last_work_draft: Option<WorkDraft>,
    desired_work_draft: Option<WorkDraft>,
    pub work_ready: Option<AssignmentId>,
    pub last_admin_config: Option<DatasetMetadata>,
    desired_admin_config: Option<DatasetMetadata>,
    admin_delete_desired: bool,
}

#[derive(Clone, Debug, Default)]
struct RetryState {
    attempt: u8,
    ready_at: Option<Instant>,
}

impl RetryState {
    fn failed(&mut self, now: Instant) -> Duration {
        let delay = retry_delay(self.attempt);
        self.attempt = self.attempt.saturating_add(1);
        self.ready_at = Some(now + delay);
        delay
    }

    fn reset(&mut self) {
        *self = Self::default();
    }

    fn remaining(&self, now: Instant) -> Option<Duration> {
        self.ready_at
            .and_then(|ready_at| ready_at.checked_duration_since(now))
    }
}

impl Default for PersistenceState {
    fn default() -> Self {
        Self {
            store: browser_draft_store(),
            commands: VecDeque::new(),
            active: false,
            identity: None,
            preference: None,
            preference_encoded: None,
            preference_desired_encoded: None,
            preference_retry: RetryState::default(),
            restoration_attempted: false,
            expected_assignment: None,
            recovery: None,
            last_work_draft: None,
            desired_work_draft: None,
            work_ready: None,
            last_admin_config: None,
            desired_admin_config: None,
            admin_delete_desired: false,
        }
    }
}

fn retry_delay(attempt: u8) -> Duration {
    let multiplier = 1_u32
        .checked_shl(u32::from(attempt.min(16)))
        .unwrap_or(u32::MAX);
    STORAGE_RETRY_BASE
        .checked_mul(multiplier)
        .unwrap_or(STORAGE_RETRY_MAX)
        .min(STORAGE_RETRY_MAX)
}

pub(crate) fn browser_draft_store() -> Rc<dyn DraftStore> {
    #[cfg(target_arch = "wasm32")]
    {
        Rc::new(IndexedDbDraftStore)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Rc::new(MemoryDraftStore::default())
    }
}
