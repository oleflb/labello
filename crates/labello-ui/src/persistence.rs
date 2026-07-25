use std::{collections::VecDeque, future::Future, pin::Pin, rc::Rc};

use labello_domain::{
    AnnotationGeometry, AnnotationId, AnnotationVersion, Assignment, AssignmentId, AssignmentKind,
    AssignmentStatus, DatasetId, DatasetMetadata, ImageId, ImageState, TaskId, Timestamp, UserId,
};
use serde::{Deserialize, Serialize};
use web_time::{Duration, Instant};

const PREFERENCE_VERSION: u32 = 2;
const DRAFT_VERSION: u32 = 2;
const LOCAL_PREFIX: &str = "labello:workspace:v2";
#[cfg(target_arch = "wasm32")]
const DATABASE_NAME: &str = "labello-workspace-v2";
#[cfg(target_arch = "wasm32")]
const DRAFT_STORE: &str = "drafts";
pub(crate) const DRAFT_TTL_SECONDS: i64 = 7 * 24 * 60 * 60;
const MAX_DRAFT_BYTES: usize = 8 * 1024 * 1024;
const MAX_ADMIN_DRAFT_BYTES: usize = 256 * 1024;
const STORAGE_RETRY_BASE: Duration = Duration::from_millis(100);
const STORAGE_RETRY_MAX: Duration = Duration::from_secs(5);

pub(crate) type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + 'a>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StorageIdentity {
    pub server: String,
    pub user_id: UserId,
}

impl StorageIdentity {
    pub(crate) fn new(api_base_url: &str, user_id: UserId) -> Result<Self, String> {
        Ok(Self {
            server: normalize_server_identity(api_base_url)?,
            user_id,
        })
    }

    fn prefix(&self) -> String {
        format!(
            "{LOCAL_PREFIX}:{}:{}",
            key_segment(&self.server),
            key_segment(self.user_id.as_str())
        )
    }

    fn owns_key(&self, key: &str) -> bool {
        key.strip_prefix(&self.prefix())
            .is_some_and(|suffix| suffix.starts_with(':'))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredView {
    Annotate,
    Review,
    Adjudicate,
    Admin,
    Stats,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredCanvasTransform {
    pub zoom: f32,
    pub pan_x: f32,
    pub pan_y: f32,
}

impl StoredCanvasTransform {
    pub(crate) fn clamped(self) -> Self {
        Self {
            zoom: finite_or(self.zoom, 1.0).clamp(1.0, 12.0),
            pan_x: finite_or(self.pan_x, 0.0).clamp(-100_000.0, 100_000.0),
            pan_y: finite_or(self.pan_y, 0.0).clamp(-100_000.0, 100_000.0),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspacePreference {
    pub version: u32,
    pub dataset_id: DatasetId,
    pub view: StoredView,
    pub task_id: Option<TaskId>,
    pub assignment_id: Option<AssignmentId>,
    pub assignment_image_id: Option<ImageId>,
    pub assignment_kind: Option<AssignmentKind>,
    pub drawer: Option<String>,
    pub show_settings: bool,
    pub show_tutorial: bool,
    pub selected_annotation: Option<AnnotationId>,
    pub canvas: StoredCanvasTransform,
}

pub(crate) fn load_workspace_preference(
    identity: &StorageIdentity,
) -> Result<Option<WorkspacePreference>, String> {
    let Some(value) = local_get(&format!("{}:location", identity.prefix()))? else {
        return Ok(None);
    };
    let preference: WorkspacePreference = serde_json::from_str(&value)
        .map_err(|error| format!("browser workspace preference is corrupt: {error}"))?;
    if preference.version != PREFERENCE_VERSION {
        return Ok(None);
    }
    Ok(Some(preference))
}

pub(crate) fn save_workspace_preference(
    identity: &StorageIdentity,
    preference: &WorkspacePreference,
) -> Result<(), String> {
    let mut preference = preference.clone();
    preference.version = PREFERENCE_VERSION;
    preference.canvas = preference.canvas.clamped();
    let value = serde_json::to_string(&preference)
        .map_err(|error| format!("could not encode browser workspace preference: {error}"))?;
    local_set(&format!("{}:location", identity.prefix()), &value)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DraftKind {
    Annotation,
    ReviewerCorrection,
    AdminConfig,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnnotationDraft {
    pub annotations: Vec<AnnotationVersion>,
    pub accepted_prelabels: Vec<String>,
    pub selected_annotation: Option<AnnotationId>,
    pub active_skeleton: Option<AnnotationId>,
    pub skeleton_keypoint_index: usize,
    pub next_keypoint_hidden: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredCorrectionDraft {
    pub correction_id: labello_domain::CorrectionId,
    pub annotation_id: AnnotationId,
    pub expected_version: u32,
    pub original_geometry: AnnotationGeometry,
    pub edited_geometry: AnnotationGeometry,
    pub reason: String,
    pub geometry_history: Vec<AnnotationGeometry>,
    pub selected_keypoint: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReviewDraft {
    pub target_annotation: Option<AnnotationId>,
    pub correction: Option<StoredCorrectionDraft>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "payloadType", content = "payload", rename_all = "snake_case")]
pub(crate) enum WorkDraftPayload {
    Annotation(AnnotationDraft),
    Review(ReviewDraft),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkDraft {
    pub version: u32,
    pub key: String,
    pub server: String,
    pub user_id: UserId,
    pub dataset_id: DatasetId,
    pub assignment_id: AssignmentId,
    pub image_id: ImageId,
    pub task_id: TaskId,
    pub assignment_kind: AssignmentKind,
    pub kind: DraftKind,
    pub lease_expires_at: Option<Timestamp>,
    pub base_event_sequence: u64,
    #[serde(default)]
    pub edit_generation: u64,
    pub updated_at: Timestamp,
    pub payload: WorkDraftPayload,
}

impl WorkDraft {
    pub(crate) fn new(
        identity: &StorageIdentity,
        dataset_id: DatasetId,
        assignment: &Assignment,
        base_event_sequence: u64,
        edit_generation: u64,
        payload: WorkDraftPayload,
    ) -> Self {
        let kind = match payload {
            WorkDraftPayload::Annotation(_) => DraftKind::Annotation,
            WorkDraftPayload::Review(_) => DraftKind::ReviewerCorrection,
        };
        let key = work_draft_key(identity, &dataset_id, assignment);
        Self {
            version: DRAFT_VERSION,
            key,
            server: identity.server.clone(),
            user_id: identity.user_id.clone(),
            dataset_id,
            assignment_id: assignment.assignment_id.clone(),
            image_id: assignment.image_id.clone(),
            task_id: assignment.task_id.clone(),
            assignment_kind: assignment.kind.clone(),
            kind,
            lease_expires_at: assignment.expires_at,
            base_event_sequence,
            edit_generation,
            updated_at: labello_domain::now(),
            payload,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AdminDraft {
    pub version: u32,
    pub key: String,
    pub server: String,
    pub user_id: UserId,
    pub dataset_id: DatasetId,
    pub kind: DraftKind,
    pub updated_at: Timestamp,
    pub baseline: DatasetMetadata,
    pub config: DatasetMetadata,
}

impl AdminDraft {
    pub(crate) fn new(
        identity: &StorageIdentity,
        dataset_id: DatasetId,
        baseline: &DatasetMetadata,
        config: &DatasetMetadata,
    ) -> Self {
        Self {
            version: DRAFT_VERSION,
            key: admin_draft_key(identity, &dataset_id),
            server: identity.server.clone(),
            user_id: identity.user_id.clone(),
            dataset_id,
            kind: DraftKind::AdminConfig,
            updated_at: labello_domain::now(),
            baseline: admin_config_snapshot(baseline),
            config: admin_config_snapshot(config),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "recordType", content = "record", rename_all = "snake_case")]
pub(crate) enum DraftRecord {
    Work(Box<WorkDraft>),
    Admin(Box<AdminDraft>),
}

impl DraftRecord {
    pub(crate) fn key(&self) -> &str {
        match self {
            Self::Work(draft) => &draft.key,
            Self::Admin(draft) => &draft.key,
        }
    }

    fn updated_at(&self) -> Timestamp {
        match self {
            Self::Work(draft) => draft.updated_at,
            Self::Admin(draft) => draft.updated_at,
        }
    }

    fn validate_size(&self) -> Result<String, String> {
        let encoded = serde_json::to_string(self)
            .map_err(|error| format!("could not encode browser draft: {error}"))?;
        let maximum = match self {
            Self::Work(_) => MAX_DRAFT_BYTES,
            Self::Admin(_) => MAX_ADMIN_DRAFT_BYTES,
        };
        if encoded.len() > maximum {
            return Err(format!(
                "browser draft is {} bytes; the safe limit is {maximum} bytes",
                encoded.len()
            ));
        }
        Ok(encoded)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DraftValidation {
    Valid,
    Expired(String),
    Conflict(String),
}

pub(crate) fn validate_work_draft(
    draft: &WorkDraft,
    identity: &StorageIdentity,
    dataset_id: &DatasetId,
    assignment: &Assignment,
    state: &ImageState,
    now: Timestamp,
) -> DraftValidation {
    if draft.version != DRAFT_VERSION
        || draft.server != identity.server
        || draft.user_id != identity.user_id
        || &draft.dataset_id != dataset_id
    {
        return DraftValidation::Conflict(
            "The saved draft belongs to a different server, user, or dataset.".to_string(),
        );
    }
    if draft.lease_expires_at.is_some_and(|expires| expires <= now) {
        return DraftValidation::Expired(
            "The saved draft's assignment lease has expired; it was not applied.".to_string(),
        );
    }
    if draft.assignment_id != assignment.assignment_id
        || draft.image_id != assignment.image_id
        || draft.task_id != assignment.task_id
        || draft.assignment_kind != assignment.kind
        || assignment.assigned_to != identity.user_id
    {
        return DraftValidation::Conflict(
            "The saved draft belongs to a different assignment.".to_string(),
        );
    }
    if assignment.status != AssignmentStatus::Active
        || assignment.expires_at.is_some_and(|expires| expires <= now)
    {
        return DraftValidation::Expired(
            "The saved draft's assignment lease has expired; it was not applied.".to_string(),
        );
    }
    if state.image_id != draft.image_id || state.current_sequence != draft.base_event_sequence {
        return DraftValidation::Conflict(format!(
            "Server events changed from sequence {} to {}; the draft was not applied.",
            draft.base_event_sequence, state.current_sequence
        ));
    }
    DraftValidation::Valid
}

pub(crate) fn validate_admin_draft(
    draft: &AdminDraft,
    identity: &StorageIdentity,
    dataset_id: &DatasetId,
    server_config: &DatasetMetadata,
) -> DraftValidation {
    if draft.version != DRAFT_VERSION
        || draft.server != identity.server
        || draft.user_id != identity.user_id
        || &draft.dataset_id != dataset_id
    {
        return DraftValidation::Conflict(
            "The admin draft belongs to a different workspace.".to_string(),
        );
    }
    if draft.baseline != admin_config_snapshot(server_config) {
        return DraftValidation::Conflict(
            "The server configuration changed after this admin draft was created.".to_string(),
        );
    }
    DraftValidation::Valid
}

fn admin_config_snapshot(metadata: &DatasetMetadata) -> DatasetMetadata {
    let mut snapshot = metadata.clone();
    snapshot.images.clear();
    snapshot
}

pub(crate) fn work_draft_key(
    identity: &StorageIdentity,
    dataset_id: &DatasetId,
    assignment: &Assignment,
) -> String {
    work_draft_key_parts(
        identity,
        dataset_id,
        &assignment.assignment_id,
        &assignment.image_id,
        &assignment.task_id,
        &assignment.kind,
    )
}

fn work_draft_key_parts(
    identity: &StorageIdentity,
    dataset_id: &DatasetId,
    assignment_id: &AssignmentId,
    image_id: &ImageId,
    task_id: &TaskId,
    kind: &AssignmentKind,
) -> String {
    format!(
        "{}:draft:{}:{}:{}:{}:{}",
        identity.prefix(),
        key_segment(dataset_id.as_str()),
        key_segment(assignment_id.as_str()),
        key_segment(image_id.as_str()),
        key_segment(task_id.as_str()),
        assignment_kind_segment(kind),
    )
}

pub(crate) fn admin_draft_key(identity: &StorageIdentity, dataset_id: &DatasetId) -> String {
    format!(
        "{}:admin:{}",
        identity.prefix(),
        key_segment(dataset_id.as_str())
    )
}

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

#[cfg(any(not(target_arch = "wasm32"), test))]
#[derive(Default)]
pub(crate) struct MemoryDraftStore {
    records: std::cell::RefCell<std::collections::BTreeMap<String, DraftRecord>>,
    failure: std::cell::RefCell<Option<(String, Option<usize>)>>,
}

#[cfg(any(not(target_arch = "wasm32"), test))]
impl MemoryDraftStore {
    #[cfg(test)]
    pub(crate) fn fail_with(&self, error: impl Into<String>) {
        *self.failure.borrow_mut() = Some((error.into(), None));
    }

    #[cfg(test)]
    pub(crate) fn fail_next(&self, count: usize, error: impl Into<String>) {
        *self.failure.borrow_mut() = Some((error.into(), Some(count)));
    }

    fn check_failure(&self) -> Result<(), String> {
        let mut failure = self.failure.borrow_mut();
        let Some((error, remaining)) = failure.as_mut() else {
            return Ok(());
        };
        let error = error.clone();
        if let Some(remaining) = remaining {
            *remaining = remaining.saturating_sub(1);
            if *remaining == 0 {
                *failure = None;
            }
        }
        Err(error)
    }
}

#[cfg(any(not(target_arch = "wasm32"), test))]
impl DraftStore for MemoryDraftStore {
    fn get<'a>(&'a self, key: &'a str) -> StoreFuture<'a, Option<DraftRecord>> {
        Box::pin(async move {
            self.check_failure()?;
            Ok(self.records.borrow().get(key).cloned())
        })
    }

    fn put<'a>(&'a self, record: DraftRecord) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            self.check_failure()?;
            record.validate_size()?;
            self.records
                .borrow_mut()
                .insert(record.key().to_string(), record);
            Ok(())
        })
    }

    fn delete<'a>(&'a self, key: &'a str) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            self.check_failure()?;
            self.records.borrow_mut().remove(key);
            Ok(())
        })
    }

    fn garbage_collect<'a>(&'a self, now: Timestamp) -> StoreFuture<'a, usize> {
        Box::pin(async move {
            self.check_failure()?;
            let cutoff = now - chrono::Duration::seconds(DRAFT_TTL_SECONDS);
            let before = self.records.borrow().len();
            self.records
                .borrow_mut()
                .retain(|_, record| record.updated_at() >= cutoff);
            Ok(before - self.records.borrow().len())
        })
    }
}

pub(crate) fn normalize_server_identity(value: &str) -> Result<String, String> {
    let mut url = url::Url::parse(value.trim())
        .map_err(|error| format!("API URL cannot identify browser storage: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("API URL must use http or https and include a host".to_string());
    }
    url.set_fragment(None);
    url.set_query(None);
    let path = url.path().trim_end_matches('/').to_string();
    url.set_path(if path.is_empty() { "/" } else { &path });
    let mut normalized = url.to_string();
    if normalized.ends_with('/') {
        normalized.pop();
    }
    Ok(normalized)
}

impl From<&crate::app::CorrectionDraft> for StoredCorrectionDraft {
    fn from(draft: &crate::app::CorrectionDraft) -> Self {
        Self {
            correction_id: draft.correction_id.clone(),
            annotation_id: draft.annotation_id.clone(),
            expected_version: draft.expected_version,
            original_geometry: draft.original_geometry.clone(),
            edited_geometry: draft.edited_geometry.clone(),
            reason: draft.reason.clone(),
            geometry_history: draft.geometry_history.clone(),
            selected_keypoint: draft.selected_keypoint,
        }
    }
}

impl From<StoredCorrectionDraft> for crate::app::CorrectionDraft {
    fn from(draft: StoredCorrectionDraft) -> Self {
        Self {
            correction_id: draft.correction_id,
            annotation_id: draft.annotation_id,
            expected_version: draft.expected_version,
            original_geometry: draft.original_geometry,
            edited_geometry: draft.edited_geometry,
            reason: draft.reason,
            geometry_history: draft.geometry_history,
            selected_keypoint: draft.selected_keypoint,
        }
    }
}

impl crate::app::LabelloApp {
    pub(crate) fn initialize_browser_workspace(&mut self) {
        self.runtime.storage_error = None;
        let identity =
            match StorageIdentity::new(&self.config.api_base_url, self.config.user_id.clone()) {
                Ok(identity) => identity,
                Err(error) => {
                    self.runtime.error = Some(error);
                    return;
                }
            };
        let preference = match load_workspace_preference(&identity) {
            Ok(preference) => preference,
            Err(error) => {
                self.storage_failure(error);
                None
            }
        };
        self.runtime.persistence.identity = Some(identity);
        self.runtime.persistence.preference_encoded = preference
            .as_ref()
            .and_then(|preference| serde_json::to_string(preference).ok());
        self.runtime.persistence.preference_desired_encoded =
            self.runtime.persistence.preference_encoded.clone();
        self.runtime.persistence.preference_retry.reset();
        self.runtime.persistence.preference = preference;
        self.runtime.persistence.restoration_attempted = false;
        self.runtime.persistence.expected_assignment = None;
        self.runtime.persistence.recovery = None;
        self.runtime.persistence.last_work_draft = None;
        self.runtime.persistence.desired_work_draft = None;
        self.runtime.persistence.work_ready = None;
        self.runtime.persistence.last_admin_config = None;
        self.runtime.persistence.desired_admin_config = None;
        self.runtime.persistence.admin_delete_desired = false;
        self.runtime.persistence.commands.clear();
        self.queue_persistence(PersistenceCommand::GarbageCollect(labello_domain::now()));
    }

    pub(crate) fn isolate_browser_workspace(&mut self) {
        self.runtime.persistence.identity = None;
        self.runtime.persistence.preference = None;
        self.runtime.persistence.preference_encoded = None;
        self.runtime.persistence.preference_desired_encoded = None;
        self.runtime.persistence.preference_retry.reset();
        self.runtime.persistence.restoration_attempted = false;
        self.runtime.persistence.expected_assignment = None;
        self.runtime.persistence.recovery = None;
        self.runtime.persistence.last_work_draft = None;
        self.runtime.persistence.desired_work_draft = None;
        self.runtime.persistence.work_ready = None;
        self.runtime.persistence.last_admin_config = None;
        self.runtime.persistence.desired_admin_config = None;
        self.runtime.persistence.admin_delete_desired = false;
        self.runtime.persistence.commands.clear();
    }

    pub(crate) fn reopen_previous_workspace(&mut self) {
        if self.runtime.persistence.restoration_attempted {
            return;
        }
        self.runtime.persistence.restoration_attempted = true;
        if self.loading.dataset || self.datasets.requested_view.is_some() {
            return;
        }
        let Some(preference) = self.runtime.persistence.preference.clone() else {
            return;
        };
        let Some(summary) = self
            .datasets
            .summaries
            .iter()
            .find(|summary| summary.dataset_id == preference.dataset_id)
        else {
            self.runtime.notice =
                Some("The previous dataset is no longer available to this account.".to_string());
            return;
        };
        let view = app_view(preference.view);
        let authorized = match view {
            crate::app::AppView::Annotate => summary
                .roles
                .contains(&labello_domain::DatasetRole::Annotator),
            crate::app::AppView::Review => summary
                .roles
                .contains(&labello_domain::DatasetRole::Reviewer),
            crate::app::AppView::Adjudicate => summary
                .roles
                .contains(&labello_domain::DatasetRole::Adjudicator),
            crate::app::AppView::Admin => summary
                .roles
                .contains(&labello_domain::DatasetRole::DataAdmin),
            crate::app::AppView::Stats => !summary.roles.is_empty(),
            crate::app::AppView::Setup => false,
        };
        if !authorized {
            self.runtime.notice = Some(
                "The previous view is no longer authorized; choose an available dataset view."
                    .to_string(),
            );
            return;
        }
        self.runtime.persistence.expected_assignment = preference.assignment_id.clone();
        self.open_dataset(preference.dataset_id, view);
    }

    pub(crate) fn persist_workspace_preference(&mut self) {
        if self.auth.account.is_none()
            || self.datasets.metadata.is_none()
            || self.view == crate::app::AppView::Setup
            || !self.can_open_view(self.view)
        {
            return;
        }
        let Some(identity) = self.runtime.persistence.identity.clone() else {
            return;
        };
        let preference = WorkspacePreference {
            version: PREFERENCE_VERSION,
            dataset_id: self.config.dataset_id.clone(),
            view: stored_view(self.view),
            task_id: self.selected_task_id.clone(),
            assignment_id: self
                .assignment
                .as_ref()
                .map(|assignment| assignment.assignment_id.clone()),
            assignment_image_id: self
                .assignment
                .as_ref()
                .map(|assignment| assignment.image_id.clone()),
            assignment_kind: self
                .assignment
                .as_ref()
                .map(|assignment| assignment.kind.clone()),
            drawer: self.drawer.map(|drawer| match drawer {
                crate::app::Drawer::Workflow => "workflow".to_string(),
                crate::app::Drawer::Inspector => "inspector".to_string(),
            }),
            show_settings: self.show_settings,
            show_tutorial: self.show_tutorial,
            selected_annotation: self.selected_annotation.clone(),
            canvas: self.canvas.stored_transform(),
        };
        let encoded = match serde_json::to_string(&preference) {
            Ok(encoded) => encoded,
            Err(error) => {
                self.runtime.error = Some(format!(
                    "could not encode browser workspace preference: {error}"
                ));
                return;
            }
        };
        if self.runtime.persistence.preference_encoded.as_deref() == Some(&encoded) {
            return;
        }
        if self
            .runtime
            .persistence
            .preference_desired_encoded
            .as_deref()
            != Some(&encoded)
        {
            self.runtime.persistence.preference_desired_encoded = Some(encoded.clone());
            self.runtime.persistence.preference_retry.reset();
        }
        let now = Instant::now();
        if let Some(remaining) = self.runtime.persistence.preference_retry.remaining(now) {
            if let Some(ctx) = self.runtime.repaint_ctx.as_ref() {
                ctx.request_repaint_after(remaining);
            }
            return;
        }
        match save_workspace_preference(&identity, &preference) {
            Ok(()) => {
                self.runtime.persistence.preference = Some(preference);
                self.runtime.persistence.preference_encoded = Some(encoded);
                self.runtime.persistence.preference_retry.reset();
            }
            Err(error) => {
                self.storage_failure(error);
                let delay = self
                    .runtime
                    .persistence
                    .preference_retry
                    .failed(Instant::now());
                if let Some(ctx) = self.runtime.repaint_ctx.as_ref() {
                    ctx.request_repaint_after(delay);
                }
            }
        }
    }

    pub(crate) fn apply_assignment_preferences(&mut self) {
        let Some(assignment) = self.assignment.as_ref() else {
            return;
        };
        let Some(preference) = self
            .runtime
            .persistence
            .preference
            .as_ref()
            .filter(|preference| {
                preference.dataset_id == self.config.dataset_id
                    && preference.assignment_id.as_ref() == Some(&assignment.assignment_id)
                    && preference.assignment_kind.as_ref() == Some(&assignment.kind)
            })
            .cloned()
        else {
            return;
        };
        self.selected_annotation = preference.selected_annotation.filter(|annotation_id| {
            self.annotations
                .iter()
                .any(|annotation| &annotation.annotation_id == annotation_id && !annotation.deleted)
        });
        self.canvas.restore_transform(preference.canvas);
        self.drawer = match preference.drawer.as_deref() {
            Some("workflow") => Some(crate::app::Drawer::Workflow),
            Some("inspector") => Some(crate::app::Drawer::Inspector),
            _ => None,
        };
        self.show_settings = preference.show_settings;
        self.show_tutorial = preference.show_tutorial;
    }

    pub(crate) fn request_work_draft_load(&mut self) {
        let (Some(identity), Some(assignment)) = (
            self.runtime.persistence.identity.as_ref(),
            self.assignment.as_ref(),
        ) else {
            return;
        };
        let key = work_draft_key(identity, &self.config.dataset_id, assignment);
        self.runtime.persistence.work_ready = None;
        self.queue_persistence(PersistenceCommand::Load(key));
    }

    pub(crate) fn request_previous_draft_status(&mut self) {
        let (Some(identity), Some(preference)) = (
            self.runtime.persistence.identity.as_ref(),
            self.runtime.persistence.preference.as_ref(),
        ) else {
            return;
        };
        let (Some(assignment_id), Some(image_id), Some(task_id), Some(kind)) = (
            preference.assignment_id.as_ref(),
            preference.assignment_image_id.as_ref(),
            preference.task_id.as_ref(),
            preference.assignment_kind.as_ref(),
        ) else {
            return;
        };
        let key = work_draft_key_parts(
            identity,
            &preference.dataset_id,
            assignment_id,
            image_id,
            task_id,
            kind,
        );
        self.queue_persistence(PersistenceCommand::Load(key));
    }

    pub(crate) fn request_admin_draft_load(&mut self) {
        let Some(identity) = self.runtime.persistence.identity.as_ref() else {
            return;
        };
        let key = admin_draft_key(identity, &self.config.dataset_id);
        self.queue_persistence(PersistenceCommand::Load(key));
    }

    pub(crate) fn queue_current_drafts(&mut self) {
        let (Some(identity), Some(assignment), Some(state)) = (
            self.runtime.persistence.identity.clone(),
            self.assignment.clone(),
            self.current_state.as_ref(),
        ) else {
            self.queue_admin_draft();
            return;
        };
        if assignment
            .expires_at
            .is_some_and(|expires_at| expires_at <= labello_domain::now())
        {
            if self.runtime.persistence.work_ready.take().is_some() {
                self.clear_current_work_draft(&assignment);
                self.runtime.notice = Some(
                    "The local assignment lease expired; its browser draft was discarded without changing server state."
                        .to_string(),
                );
            }
            return;
        }
        if self.runtime.persistence.work_ready.as_ref() != Some(&assignment.assignment_id)
            || matches!(
                self.runtime.persistence.recovery,
                Some(DraftRecovery::Work(_, _))
            )
        {
            self.queue_admin_draft();
            return;
        }
        if self.canvas.is_dragging() {
            self.queue_admin_draft();
            return;
        }
        let payload = match self.view {
            crate::app::AppView::Annotate
                if matches!(
                    self.save_status,
                    crate::app::SaveStatus::Dirty
                        | crate::app::SaveStatus::Saving
                        | crate::app::SaveStatus::Retry
                ) =>
            {
                WorkDraftPayload::Annotation(AnnotationDraft {
                    annotations: self.annotations.clone(),
                    accepted_prelabels: self.accepted_prelabels.clone(),
                    selected_annotation: self.selected_annotation.clone(),
                    active_skeleton: self.active_skeleton.clone(),
                    skeleton_keypoint_index: self.skeleton_keypoint_index,
                    next_keypoint_hidden: self.next_keypoint_hidden,
                })
            }
            crate::app::AppView::Review => WorkDraftPayload::Review(ReviewDraft {
                target_annotation: self.selected_annotation.clone(),
                correction: self
                    .correction_draft
                    .as_ref()
                    .map(StoredCorrectionDraft::from),
            }),
            _ => {
                self.queue_admin_draft();
                return;
            }
        };
        let draft = WorkDraft::new(
            &identity,
            self.config.dataset_id.clone(),
            &assignment,
            state.current_sequence,
            self.edit_generation,
            payload,
        );
        let already_persisted = self
            .runtime
            .persistence
            .last_work_draft
            .as_ref()
            .is_some_and(|saved| same_work_draft(saved, &draft));
        let already_desired = self
            .runtime
            .persistence
            .desired_work_draft
            .as_ref()
            .is_some_and(|desired| same_work_draft(desired, &draft));
        if !already_persisted && !already_desired {
            self.runtime.persistence.desired_work_draft = Some(draft.clone());
            self.queue_persistence(PersistenceCommand::Save(Box::new(DraftRecord::Work(
                Box::new(draft),
            ))));
        }
        self.queue_admin_draft();
    }

    fn queue_admin_draft(&mut self) {
        if self.view != crate::app::AppView::Admin || self.loading.admin {
            return;
        }
        let (Some(identity), Some(baseline), Some(config)) = (
            self.runtime.persistence.identity.clone(),
            self.datasets.admin_baseline.as_ref(),
            self.datasets.admin_config.as_ref(),
        ) else {
            return;
        };
        if config == baseline {
            if !self.runtime.persistence.admin_delete_desired
                && (self.runtime.persistence.last_admin_config.is_some()
                    || self.runtime.persistence.desired_admin_config.is_some())
            {
                self.clear_admin_draft();
            }
            return;
        }
        let draft = AdminDraft::new(&identity, self.config.dataset_id.clone(), baseline, config);
        if self.runtime.persistence.last_admin_config.as_ref() == Some(&draft.config)
            || self.runtime.persistence.desired_admin_config.as_ref() == Some(&draft.config)
        {
            return;
        }
        self.runtime.persistence.admin_delete_desired = false;
        self.runtime.persistence.desired_admin_config = Some(draft.config.clone());
        self.queue_persistence(PersistenceCommand::Save(Box::new(DraftRecord::Admin(
            Box::new(draft),
        ))));
    }

    pub(crate) fn clear_current_work_draft(&mut self, assignment: &Assignment) {
        let Some(identity) = self.runtime.persistence.identity.as_ref() else {
            return;
        };
        let key = work_draft_key(identity, &self.config.dataset_id, assignment);
        self.runtime.persistence.desired_work_draft = None;
        self.queue_persistence(PersistenceCommand::Delete(key));
    }

    pub(crate) fn clear_admin_draft(&mut self) {
        let Some(identity) = self.runtime.persistence.identity.as_ref() else {
            return;
        };
        let key = admin_draft_key(identity, &self.config.dataset_id);
        self.runtime.persistence.desired_admin_config = None;
        self.runtime.persistence.admin_delete_desired = true;
        self.queue_persistence(PersistenceCommand::Delete(key));
    }

    pub(crate) fn recover_browser_draft(&mut self) {
        let Some(recovery) = self.runtime.persistence.recovery.clone() else {
            return;
        };
        match recovery {
            DraftRecovery::Work(draft, DraftValidation::Valid) => {
                match draft.payload {
                    WorkDraftPayload::Annotation(draft) => {
                        self.annotations = draft.annotations;
                        self.accepted_prelabels = draft.accepted_prelabels;
                        self.selected_annotation = draft.selected_annotation;
                        self.active_skeleton = draft.active_skeleton;
                        self.skeleton_keypoint_index = draft.skeleton_keypoint_index;
                        self.next_keypoint_hidden = draft.next_keypoint_hidden;
                        self.recompute_modified_annotations();
                        self.edit_generation = self.edit_generation.wrapping_add(1);
                        self.save_status = crate::app::SaveStatus::Dirty;
                        self.last_edit_at = Some(Instant::now());
                    }
                    WorkDraftPayload::Review(draft) => {
                        if let Some(target) = draft.target_annotation
                            && self.annotations.iter().any(|annotation| {
                                annotation.annotation_id == target && !annotation.deleted
                            })
                        {
                            self.selected_annotation = Some(target);
                        }
                        self.correction_draft = draft.correction.map(Into::into);
                    }
                }
                self.runtime.notice = Some("Recovered the validated browser draft.".to_string());
                self.runtime.persistence.recovery = None;
            }
            DraftRecovery::Admin(draft, DraftValidation::Valid) => {
                let recovered = draft.config;
                let mut config = self
                    .datasets
                    .admin_baseline
                    .clone()
                    .unwrap_or_else(|| recovered.clone());
                self.runtime.persistence.last_admin_config = Some(recovered.clone());
                config.name = recovered.name;
                config.image_roots = recovered.image_roots;
                config.label_classes = recovered.label_classes;
                config.tasks = recovered.tasks;
                config.imbalance = recovered.imbalance;
                config.prelabel_configs = recovered.prelabel_configs;
                self.datasets.admin_config = Some(config.clone());
                self.runtime.notice = Some("Recovered the validated admin draft.".to_string());
                self.runtime.persistence.recovery = None;
            }
            _ => {}
        }
    }

    pub(crate) fn discard_browser_draft(&mut self) {
        let Some(recovery) = self.runtime.persistence.recovery.take() else {
            return;
        };
        let key = match recovery {
            DraftRecovery::Work(draft, _) => draft.key,
            DraftRecovery::Admin(draft, _) => draft.key,
        };
        self.queue_persistence(PersistenceCommand::Delete(key));
        self.runtime.notice = Some("Browser draft discarded.".to_string());
    }

    pub(crate) fn rebase_work_draft_after_save(&mut self, saved_generation: u64) {
        if self
            .runtime
            .persistence
            .last_work_draft
            .as_ref()
            .is_some_and(|draft| draft.edit_generation <= saved_generation)
        {
            self.runtime.persistence.last_work_draft = None;
        }
        if self
            .runtime
            .persistence
            .desired_work_draft
            .as_ref()
            .is_some_and(|draft| draft.edit_generation <= saved_generation)
        {
            self.runtime.persistence.desired_work_draft = None;
        }
        self.runtime.persistence.commands.retain(|queued| {
            !matches!(
                &queued.command,
                PersistenceCommand::Save(record)
                    if matches!(record.as_ref(), DraftRecord::Work(draft) if draft.edit_generation <= saved_generation)
            )
        });
    }

    pub(crate) fn reset_work_draft_tracking(&mut self) {
        self.runtime.persistence.last_work_draft = None;
        self.runtime.persistence.desired_work_draft = None;
    }

    pub(crate) fn start_next_persistence_command(&mut self) {
        if self.runtime.persistence.active {
            return;
        }
        let now = Instant::now();
        let Some(index) = self
            .runtime
            .persistence
            .commands
            .iter()
            .position(|command| command.ready_at <= now)
        else {
            if let Some(delay) = self
                .runtime
                .persistence
                .commands
                .iter()
                .filter_map(|command| command.ready_at.checked_duration_since(now))
                .min()
                && let Some(ctx) = self.runtime.repaint_ctx.as_ref()
            {
                ctx.request_repaint_after(delay);
            }
            return;
        };
        let command = self
            .runtime
            .persistence
            .commands
            .remove(index)
            .expect("ready persistence command exists");
        self.runtime.persistence.active = true;
        let store = self.runtime.persistence.store.clone();
        let request = crate::app::RequestIdentity {
            auth_epoch: self.auth_epoch,
            workspace_epoch: self.workspace_epoch,
            request_id: 0,
            dataset_id: Some(self.config.dataset_id.clone()),
        };
        self.spawn_message(request, async move {
            crate::app::UiMessage::PersistenceFinished(Box::new(
                execute_persistence_command(store, command).await,
            ))
        });
    }

    pub(crate) fn handle_persistence_completion(&mut self, completion: PersistenceCompletion) {
        self.runtime.persistence.active = false;
        let command = completion.command();
        if self.runtime.persistence.identity.as_ref() != Some(&command.identity)
            || command
                .key()
                .is_some_and(|key| !command.identity.owns_key(key))
        {
            return;
        }
        match completion {
            PersistenceCompletion::Loaded { command, result } => {
                let key = command.key().expect("load command has a key").to_string();
                match *result {
                    Ok(Some(DraftRecord::Work(draft))) => {
                        if let (Some(identity), Some(assignment)) = (
                            self.runtime.persistence.identity.as_ref(),
                            self.assignment.as_ref(),
                        ) && key == work_draft_key(identity, &self.config.dataset_id, assignment)
                        {
                            self.runtime.persistence.work_ready =
                                Some(assignment.assignment_id.clone());
                        }
                        let validation =
                            match (self.assignment.as_ref(), self.current_state.as_ref()) {
                                (Some(assignment), Some(state)) if draft.key == key => {
                                    validate_work_draft(
                                        &draft,
                                        self.runtime
                                            .persistence
                                            .identity
                                            .as_ref()
                                            .expect("identity exists"),
                                        &self.config.dataset_id,
                                        assignment,
                                        state,
                                        labello_domain::now(),
                                    )
                                }
                                _ => DraftValidation::Conflict(
                                    "The assignment changed before its draft finished loading."
                                        .to_string(),
                                ),
                            };
                        if matches!(validation, DraftValidation::Expired(_)) {
                            self.queue_persistence(PersistenceCommand::Delete(draft.key.clone()));
                        }
                        self.runtime.persistence.recovery =
                            Some(DraftRecovery::Work(draft, validation));
                        if matches!(
                            self.runtime.persistence.recovery,
                            Some(DraftRecovery::Work(_, DraftValidation::Valid))
                        ) {
                            self.recover_browser_draft();
                        }
                    }
                    Ok(Some(DraftRecord::Admin(draft))) => {
                        let validation = match self.datasets.admin_baseline.as_ref() {
                            Some(baseline) => validate_admin_draft(
                                &draft,
                                self.runtime
                                    .persistence
                                    .identity
                                    .as_ref()
                                    .expect("identity exists"),
                                &self.config.dataset_id,
                                baseline,
                            ),
                            None => DraftValidation::Conflict(
                                "The admin dataset changed before its draft finished loading."
                                    .to_string(),
                            ),
                        };
                        self.runtime.persistence.recovery =
                            Some(DraftRecovery::Admin(draft, validation));
                    }
                    Ok(None) => {
                        if let (Some(identity), Some(assignment)) = (
                            self.runtime.persistence.identity.as_ref(),
                            self.assignment.as_ref(),
                        ) && key == work_draft_key(identity, &self.config.dataset_id, assignment)
                        {
                            self.runtime.persistence.work_ready =
                                Some(assignment.assignment_id.clone());
                        }
                    }
                    Err(error) => self.storage_failure(error),
                }
            }
            PersistenceCompletion::Saved { command, result } => match result {
                Ok(()) => match &command.command {
                    PersistenceCommand::Save(record) => match record.as_ref() {
                        DraftRecord::Work(draft) => {
                            self.runtime.persistence.last_work_draft = Some((**draft).clone());
                        }
                        DraftRecord::Admin(draft) => {
                            self.runtime.persistence.last_admin_config = Some(draft.config.clone());
                        }
                    },
                    _ => unreachable!("saved completion has a save command"),
                },
                Err(error) => {
                    let key = command.key().expect("save command has a key").to_string();
                    self.storage_failure(format!("{key}: {error}"));
                    self.retry_persistence(command);
                }
            },
            PersistenceCompletion::Deleted { command, result } => match result {
                Ok(()) => {
                    let key = command.key().expect("delete command has a key");
                    if self
                        .runtime
                        .persistence
                        .last_work_draft
                        .as_ref()
                        .is_some_and(|draft| draft.key == key)
                    {
                        self.runtime.persistence.last_work_draft = None;
                    }
                    if self.runtime.persistence.last_admin_config.is_some()
                        && self
                            .runtime
                            .persistence
                            .identity
                            .as_ref()
                            .is_some_and(|identity| {
                                key == admin_draft_key(identity, &self.config.dataset_id)
                            })
                    {
                        self.runtime.persistence.last_admin_config = None;
                        self.runtime.persistence.admin_delete_desired = false;
                    }
                }
                Err(error) => {
                    let key = command.key().expect("delete command has a key").to_string();
                    self.storage_failure(format!("{key}: {error}"));
                    self.retry_persistence(command);
                }
            },
            PersistenceCompletion::GarbageCollected { result, .. } => {
                if let Err(error) = result {
                    self.storage_failure(error);
                }
            }
        }
    }

    fn queue_persistence(&mut self, command: PersistenceCommand) {
        let Some(identity) = self.runtime.persistence.identity.clone() else {
            return;
        };
        self.enqueue_persistence(QueuedPersistenceCommand {
            identity,
            command,
            attempt: 0,
            ready_at: Instant::now(),
        });
    }

    fn retry_persistence(&mut self, mut command: QueuedPersistenceCommand) {
        let Some(key) = command.key().map(str::to_string) else {
            return;
        };
        if self.runtime.persistence.commands.iter().any(|queued| {
            queued.key() == Some(&key)
                && matches!(
                    queued.command,
                    PersistenceCommand::Save(_) | PersistenceCommand::Delete(_)
                )
        }) {
            return;
        }
        command.attempt = command.attempt.saturating_add(1);
        let delay = retry_delay(command.attempt.saturating_sub(1));
        command.ready_at = Instant::now() + delay;
        self.runtime.persistence.commands.push_back(command);
        if let Some(ctx) = self.runtime.repaint_ctx.as_ref() {
            ctx.request_repaint_after(delay);
        }
    }

    fn enqueue_persistence(&mut self, command: QueuedPersistenceCommand) {
        let key = match &command {
            QueuedPersistenceCommand {
                command: PersistenceCommand::Load(key),
                ..
            }
            | QueuedPersistenceCommand {
                command: PersistenceCommand::Delete(key),
                ..
            } => Some(key.as_str()),
            QueuedPersistenceCommand {
                command: PersistenceCommand::Save(record),
                ..
            } => Some(record.key()),
            QueuedPersistenceCommand {
                command: PersistenceCommand::GarbageCollect(_),
                ..
            } => None,
        };
        if let Some(key) = key {
            self.runtime
                .persistence
                .commands
                .retain(|queued| match queued {
                    QueuedPersistenceCommand {
                        command: PersistenceCommand::Load(queued_key),
                        ..
                    } => {
                        !matches!(command.command, PersistenceCommand::Load(_)) || queued_key != key
                    }
                    QueuedPersistenceCommand {
                        command: PersistenceCommand::Save(record),
                        ..
                    } => {
                        !matches!(
                            command.command,
                            PersistenceCommand::Save(_) | PersistenceCommand::Delete(_)
                        ) || record.key() != key
                    }
                    QueuedPersistenceCommand {
                        command: PersistenceCommand::Delete(queued_key),
                        ..
                    } => {
                        !matches!(
                            command.command,
                            PersistenceCommand::Save(_) | PersistenceCommand::Delete(_)
                        ) || queued_key != key
                    }
                    QueuedPersistenceCommand {
                        command: PersistenceCommand::GarbageCollect(_),
                        ..
                    } => true,
                });
        }
        self.runtime.persistence.commands.push_back(command);
    }

    fn storage_failure(&mut self, error: String) {
        tracing::warn!(
            event = "browser_storage.failed",
            "browser persistence operation failed"
        );
        self.runtime.storage_error = Some(format!("Browser storage failed: {error}"));
    }
}

fn stored_view(view: crate::app::AppView) -> StoredView {
    match view {
        crate::app::AppView::Annotate => StoredView::Annotate,
        crate::app::AppView::Review => StoredView::Review,
        crate::app::AppView::Adjudicate => StoredView::Adjudicate,
        crate::app::AppView::Admin => StoredView::Admin,
        crate::app::AppView::Stats | crate::app::AppView::Setup => StoredView::Stats,
    }
}

fn app_view(view: StoredView) -> crate::app::AppView {
    match view {
        StoredView::Annotate => crate::app::AppView::Annotate,
        StoredView::Review => crate::app::AppView::Review,
        StoredView::Adjudicate => crate::app::AppView::Adjudicate,
        StoredView::Admin => crate::app::AppView::Admin,
        StoredView::Stats => crate::app::AppView::Stats,
    }
}

fn assignment_kind_segment(kind: &AssignmentKind) -> &'static str {
    match kind {
        AssignmentKind::Annotation => "annotation",
        AssignmentKind::Review => "review",
        AssignmentKind::Adjudication => "adjudication",
    }
}

fn key_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

fn same_work_draft(left: &WorkDraft, right: &WorkDraft) -> bool {
    left.key == right.key
        && left.base_event_sequence == right.base_event_sequence
        && left.edit_generation == right.edit_generation
        && left.payload == right.payload
}

#[cfg(target_arch = "wasm32")]
struct IndexedDbDraftStore;

#[cfg(target_arch = "wasm32")]
impl DraftStore for IndexedDbDraftStore {
    fn get<'a>(&'a self, key: &'a str) -> StoreFuture<'a, Option<DraftRecord>> {
        Box::pin(async move {
            let database = open_database().await?;
            let transaction = database
                .transaction_with_str_and_mode(DRAFT_STORE, web_sys::IdbTransactionMode::Readonly)
                .map_err(js_error)?;
            let transaction_done = watch_transaction(&transaction);
            let request = transaction
                .object_store(DRAFT_STORE)
                .map_err(js_error)?
                .get(&wasm_bindgen::JsValue::from_str(key))
                .map_err(js_error)?;
            let value = await_request(request).await?;
            transaction_done.await.map_err(js_error)?;
            if value.is_undefined() {
                return Ok(None);
            }
            let encoded = value
                .as_string()
                .ok_or_else(|| "IndexedDB draft is not encoded text".to_string())?;
            serde_json::from_str(&encoded)
                .map(Some)
                .map_err(|error| format!("IndexedDB draft is corrupt: {error}"))
        })
    }

    fn put<'a>(&'a self, record: DraftRecord) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let encoded = record.validate_size()?;
            let key = record.key().to_string();
            let database = open_database().await?;
            let transaction = database
                .transaction_with_str_and_mode(DRAFT_STORE, web_sys::IdbTransactionMode::Readwrite)
                .map_err(js_error)?;
            let transaction_done = watch_transaction(&transaction);
            let request = transaction
                .object_store(DRAFT_STORE)
                .map_err(js_error)?
                .put_with_key(
                    &wasm_bindgen::JsValue::from_str(&encoded),
                    &wasm_bindgen::JsValue::from_str(&key),
                )
                .map_err(js_error)?;
            await_request(request).await?;
            transaction_done.await.map(|_| ()).map_err(js_error)
        })
    }

    fn delete<'a>(&'a self, key: &'a str) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let database = open_database().await?;
            let transaction = database
                .transaction_with_str_and_mode(DRAFT_STORE, web_sys::IdbTransactionMode::Readwrite)
                .map_err(js_error)?;
            let transaction_done = watch_transaction(&transaction);
            let request = transaction
                .object_store(DRAFT_STORE)
                .map_err(js_error)?
                .delete(&wasm_bindgen::JsValue::from_str(key))
                .map_err(js_error)?;
            await_request(request).await?;
            transaction_done.await.map(|_| ()).map_err(js_error)
        })
    }

    fn garbage_collect<'a>(&'a self, now: Timestamp) -> StoreFuture<'a, usize> {
        Box::pin(async move {
            let database = open_database().await?;
            let read = database
                .transaction_with_str_and_mode(DRAFT_STORE, web_sys::IdbTransactionMode::Readonly)
                .map_err(js_error)?;
            let read_done = watch_transaction(&read);
            let values = await_request(
                read.object_store(DRAFT_STORE)
                    .map_err(js_error)?
                    .get_all()
                    .map_err(js_error)?,
            )
            .await?;
            read_done.await.map_err(js_error)?;
            let cutoff = now - chrono::Duration::seconds(DRAFT_TTL_SECONDS);
            let values = js_sys::Array::from(&values);
            let mut keys = Vec::new();
            for value in values.iter() {
                let Some(encoded) = value.as_string() else {
                    continue;
                };
                if let Ok(record) = serde_json::from_str::<DraftRecord>(&encoded)
                    && record.updated_at() < cutoff
                {
                    keys.push(record.key().to_string());
                }
            }
            for key in &keys {
                let transaction = database
                    .transaction_with_str_and_mode(
                        DRAFT_STORE,
                        web_sys::IdbTransactionMode::Readwrite,
                    )
                    .map_err(js_error)?;
                let transaction_done = watch_transaction(&transaction);
                await_request(
                    transaction
                        .object_store(DRAFT_STORE)
                        .map_err(js_error)?
                        .delete(&wasm_bindgen::JsValue::from_str(key))
                        .map_err(js_error)?,
                )
                .await?;
                transaction_done.await.map_err(js_error)?;
            }
            Ok(keys.len())
        })
    }
}

#[cfg(target_arch = "wasm32")]
async fn open_database() -> Result<web_sys::IdbDatabase, String> {
    use wasm_bindgen::JsCast;

    let factory = web_sys::window()
        .ok_or_else(|| "missing browser window".to_string())?
        .indexed_db()
        .map_err(js_error)?
        .ok_or_else(|| "IndexedDB is unavailable in this browser context".to_string())?;
    let request = factory.open_with_u32(DATABASE_NAME, 1).map_err(js_error)?;
    let upgrade_request = request.clone();
    let upgrade =
        wasm_bindgen::closure::Closure::<dyn FnMut(_)>::new(move |_event: web_sys::Event| {
            if let Ok(database) = upgrade_request.result()
                && let Ok(database) = database.dyn_into::<web_sys::IdbDatabase>()
                && !database.object_store_names().contains(DRAFT_STORE)
            {
                let _ = database.create_object_store(DRAFT_STORE);
            }
        });
    request.set_onupgradeneeded(Some(upgrade.as_ref().unchecked_ref()));
    let value = await_request(request.clone().unchecked_into()).await;
    request.set_onupgradeneeded(None);
    drop(upgrade);
    value?
        .dyn_into::<web_sys::IdbDatabase>()
        .map_err(|_| "IndexedDB open returned an invalid database".to_string())
}

#[cfg(target_arch = "wasm32")]
async fn await_request(request: web_sys::IdbRequest) -> Result<wasm_bindgen::JsValue, String> {
    use wasm_bindgen::JsCast;

    let watched = request.clone();
    let promise = js_sys::Promise::new(&mut move |resolve, reject| {
        let request = watched.clone();
        let resolve = resolve.clone();
        let reject = reject.clone();
        let callback =
            wasm_bindgen::closure::Closure::once_into_js(move |_event: web_sys::Event| {
                request.set_onsuccess(None);
                request.set_onerror(None);
                match request.error() {
                    Ok(Some(error)) => {
                        let _ = reject.call1(
                            &wasm_bindgen::JsValue::UNDEFINED,
                            &wasm_bindgen::JsValue::from_str(&error.message()),
                        );
                    }
                    _ => match request.result() {
                        Ok(value) => {
                            let _ = resolve.call1(&wasm_bindgen::JsValue::UNDEFINED, &value);
                        }
                        Err(error) => {
                            let _ = reject.call1(&wasm_bindgen::JsValue::UNDEFINED, &error);
                        }
                    },
                }
            });
        let function = callback.unchecked_ref::<js_sys::Function>();
        watched.set_onsuccess(Some(function));
        watched.set_onerror(Some(function));
    });
    wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(js_error)
}

#[cfg(target_arch = "wasm32")]
fn watch_transaction(transaction: &web_sys::IdbTransaction) -> wasm_bindgen_futures::JsFuture {
    use wasm_bindgen::JsCast;

    let watched = transaction.clone();
    let promise = js_sys::Promise::new(&mut move |resolve, reject| {
        let transaction = watched.clone();
        let resolve = resolve.clone();
        let reject = reject.clone();
        let callback =
            wasm_bindgen::closure::Closure::once_into_js(move |event: web_sys::Event| {
                transaction.set_oncomplete(None);
                transaction.set_onabort(None);
                transaction.set_onerror(None);
                if event.type_() == "complete" {
                    let _ = resolve.call0(&wasm_bindgen::JsValue::UNDEFINED);
                    return;
                }
                let error = transaction
                    .error()
                    .map(|error| error.message())
                    .unwrap_or_else(|| format!("IndexedDB transaction {}", event.type_()));
                let _ = reject.call1(
                    &wasm_bindgen::JsValue::UNDEFINED,
                    &wasm_bindgen::JsValue::from_str(&error),
                );
            });
        let function = callback.unchecked_ref::<js_sys::Function>();
        watched.set_oncomplete(Some(function));
        watched.set_onabort(Some(function));
        watched.set_onerror(Some(function));
    });
    wasm_bindgen_futures::JsFuture::from(promise)
}

#[cfg(target_arch = "wasm32")]
fn js_error(error: wasm_bindgen::JsValue) -> String {
    error
        .as_string()
        .unwrap_or_else(|| format!("browser storage error: {error:?}"))
}

#[cfg(target_arch = "wasm32")]
fn browser_storage() -> Result<web_sys::Storage, String> {
    web_sys::window()
        .ok_or_else(|| "missing browser window".to_string())?
        .local_storage()
        .map_err(js_error)?
        .ok_or_else(|| "localStorage is unavailable in this browser context".to_string())
}

#[cfg(target_arch = "wasm32")]
fn local_set(key: &str, value: &str) -> Result<(), String> {
    browser_storage()?.set_item(key, value).map_err(|error| {
        format!(
            "could not save browser workspace preference: {}",
            js_error(error)
        )
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn local_set(_key: &str, _value: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn local_get(key: &str) -> Result<Option<String>, String> {
    browser_storage()?.get_item(key).map_err(|error| {
        format!(
            "could not load browser workspace preference: {}",
            js_error(error)
        )
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn local_get(_key: &str) -> Result<Option<String>, String> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use labello_domain::{ClassId, LabelClass};

    fn assignment() -> Assignment {
        let now = labello_domain::now();
        Assignment {
            assignment_id: AssignmentId::from("assignment-a"),
            image_id: ImageId::from("image-a"),
            task_id: TaskId::from("task-a"),
            assigned_to: UserId::from("user-a"),
            kind: AssignmentKind::Annotation,
            status: AssignmentStatus::Active,
            expires_at: Some(now + chrono::Duration::minutes(30)),
            created_at: now,
            updated_at: now,
        }
    }

    fn identity() -> StorageIdentity {
        StorageIdentity::new("HTTPS://Example.COM:443/api/", UserId::from("user-a")).unwrap()
    }

    fn work_draft() -> WorkDraft {
        WorkDraft::new(
            &identity(),
            DatasetId::from("data-a"),
            &assignment(),
            7,
            3,
            WorkDraftPayload::Annotation(AnnotationDraft {
                annotations: Vec::new(),
                accepted_prelabels: Vec::new(),
                selected_annotation: None,
                active_skeleton: None,
                skeleton_keypoint_index: 0,
                next_keypoint_hidden: false,
            }),
        )
    }

    #[test]
    fn normalizes_server_and_namespaces_every_identity_dimension() {
        assert_eq!(identity().server, "https://example.com/api");
        let first = assignment();
        let mut second = first.clone();
        second.assignment_id = AssignmentId::from("assignment-b");
        assert_ne!(
            work_draft_key(&identity(), &DatasetId::from("data-a"), &first),
            work_draft_key(&identity(), &DatasetId::from("data-a"), &second)
        );
        assert_ne!(
            work_draft_key(&identity(), &DatasetId::from("data-a"), &first),
            work_draft_key(&identity(), &DatasetId::from("data-b"), &first)
        );
        let other_user =
            StorageIdentity::new("https://example.com/api", UserId::from("user-b")).unwrap();
        assert_ne!(
            work_draft_key(&identity(), &DatasetId::from("data-a"), &first),
            work_draft_key(&other_user, &DatasetId::from("data-a"), &first)
        );
    }

    #[test]
    fn canvas_preferences_clamp_non_finite_and_extreme_values() {
        assert_eq!(
            StoredCanvasTransform {
                zoom: f32::NAN,
                pan_x: f32::INFINITY,
                pan_y: -200_000.0,
            }
            .clamped(),
            StoredCanvasTransform {
                zoom: 1.0,
                pan_x: 0.0,
                pan_y: -100_000.0,
            }
        );
    }

    #[test]
    fn validates_exact_assignment_sequence_and_expiration() {
        let draft = work_draft();
        let assignment = assignment();
        let mut state = ImageState::new(assignment.image_id.clone());
        state.current_sequence = 7;
        assert_eq!(
            validate_work_draft(
                &draft,
                &identity(),
                &DatasetId::from("data-a"),
                &assignment,
                &state,
                labello_domain::now(),
            ),
            DraftValidation::Valid
        );
        state.current_sequence = 8;
        assert!(matches!(
            validate_work_draft(
                &draft,
                &identity(),
                &DatasetId::from("data-a"),
                &assignment,
                &state,
                labello_domain::now(),
            ),
            DraftValidation::Conflict(_)
        ));
        assert!(matches!(
            validate_work_draft(
                &draft,
                &identity(),
                &DatasetId::from("data-a"),
                &assignment,
                &state,
                assignment.expires_at.unwrap() + chrono::Duration::seconds(1),
            ),
            DraftValidation::Expired(_)
        ));
    }

    #[test]
    fn memory_store_is_async_bounded_isolated_and_garbage_collected() {
        let store = MemoryDraftStore::default();
        let mut draft = work_draft();
        poll(store.put(DraftRecord::Work(Box::new(draft.clone())))).unwrap();
        assert_eq!(
            poll(store.get(&draft.key)).unwrap(),
            Some(DraftRecord::Work(Box::new(draft.clone())))
        );
        draft.updated_at = labello_domain::now() - chrono::Duration::seconds(DRAFT_TTL_SECONDS + 1);
        poll(store.put(DraftRecord::Work(Box::new(draft.clone())))).unwrap();
        assert_eq!(
            poll(store.garbage_collect(labello_domain::now())).unwrap(),
            1
        );
        assert_eq!(poll(store.get(&draft.key)).unwrap(), None);

        let huge = AdminDraft::new(
            &identity(),
            DatasetId::from("data-a"),
            &metadata("baseline"),
            &metadata(&"x".repeat(MAX_ADMIN_DRAFT_BYTES)),
        );
        assert!(poll(store.put(DraftRecord::Admin(Box::new(huge)))).is_err());
    }

    #[test]
    fn memory_store_surfaces_failures() {
        let store = MemoryDraftStore::default();
        store.fail_with("quota denied");
        assert_eq!(
            poll(store.get("key")).unwrap_err(),
            "quota denied".to_string()
        );
    }

    #[test]
    fn failed_put_retries_the_unchanged_record_and_advances_marker_only_on_success() {
        let store = Rc::new(MemoryDraftStore::default());
        store.fail_next(1, "quota denied");
        let mut app = crate::app::LabelloApp::default();
        app.runtime.persistence.identity = Some(identity());
        app.runtime.persistence.store = store.clone();
        let record = DraftRecord::Work(Box::new(work_draft()));
        let expected = match &record {
            DraftRecord::Work(draft) => (**draft).clone(),
            DraftRecord::Admin(_) => unreachable!(),
        };
        app.runtime.persistence.desired_work_draft = match &record {
            DraftRecord::Work(draft) => Some((**draft).clone()),
            DraftRecord::Admin(_) => None,
        };
        app.queue_persistence(PersistenceCommand::Save(Box::new(record.clone())));

        let command = app.runtime.persistence.commands.pop_front().unwrap();
        let completion = poll(execute_persistence_command(store.clone(), command));
        app.handle_persistence_completion(completion);

        assert!(app.runtime.storage_error.is_some());
        assert!(app.runtime.persistence.last_work_draft.is_none());
        let retry = app.runtime.persistence.commands.front().unwrap();
        assert_eq!(retry.attempt, 1);
        assert!(matches!(
            &retry.command,
            PersistenceCommand::Save(queued) if queued.as_ref() == &record
        ));

        let mut retry = app.runtime.persistence.commands.pop_front().unwrap();
        retry.ready_at = Instant::now();
        let completion = poll(execute_persistence_command(store.clone(), retry));
        app.handle_persistence_completion(completion);
        assert_eq!(app.runtime.persistence.last_work_draft, Some(expected));
        assert_eq!(poll(store.get(record.key())).unwrap(), Some(record));
        assert_eq!(retry_delay(u8::MAX), STORAGE_RETRY_MAX);
    }

    #[test]
    fn edit_during_save_keeps_the_new_generation_queued_and_rebases_only_the_saved_one() {
        let store = Rc::new(MemoryDraftStore::default());
        let mut app = crate::app::LabelloApp::default();
        app.runtime.persistence.identity = Some(identity());
        app.runtime.persistence.store = store.clone();
        let saved = work_draft();
        let mut newer = saved.clone();
        newer.edit_generation += 1;
        if let WorkDraftPayload::Annotation(payload) = &mut newer.payload {
            payload.accepted_prelabels.push("later-edit".to_string());
        }

        app.runtime.persistence.desired_work_draft = Some(saved.clone());
        app.queue_persistence(PersistenceCommand::Save(Box::new(DraftRecord::Work(
            Box::new(saved.clone()),
        ))));
        let in_flight = app.runtime.persistence.commands.pop_front().unwrap();
        app.runtime.persistence.desired_work_draft = Some(newer.clone());
        app.queue_persistence(PersistenceCommand::Save(Box::new(DraftRecord::Work(
            Box::new(newer.clone()),
        ))));

        let completion = poll(execute_persistence_command(store.clone(), in_flight));
        app.handle_persistence_completion(completion);
        assert_eq!(app.runtime.persistence.last_work_draft, Some(saved.clone()));
        app.rebase_work_draft_after_save(saved.edit_generation);
        assert!(app.runtime.persistence.last_work_draft.is_none());
        assert_eq!(
            app.runtime.persistence.desired_work_draft,
            Some(newer.clone())
        );
        assert!(app.runtime.persistence.commands.iter().any(|queued| matches!(
            &queued.command,
            PersistenceCommand::Save(record)
                if matches!(record.as_ref(), DraftRecord::Work(draft) if draft.as_ref() == &newer)
        )));

        let mut rebased = newer;
        rebased.base_event_sequence += 1;
        app.runtime.persistence.desired_work_draft = Some(rebased.clone());
        app.queue_persistence(PersistenceCommand::Save(Box::new(DraftRecord::Work(
            Box::new(rebased.clone()),
        ))));
        let restarted = app.runtime.persistence.commands.pop_front().unwrap();
        let completion = poll(execute_persistence_command(store.clone(), restarted));
        app.handle_persistence_completion(completion);
        assert_eq!(
            poll(store.get(&rebased.key)).unwrap(),
            Some(DraftRecord::Work(Box::new(rebased)))
        );
    }

    #[test]
    fn completion_identity_requires_the_exact_namespace_not_a_prefix_collision() {
        let current = StorageIdentity::new("https://example.test", UserId::from("user")).unwrap();
        let colliding =
            StorageIdentity::new("https://example.test", UserId::from("user2")).unwrap();
        let mut assignment = assignment();
        assignment.assigned_to = colliding.user_id.clone();
        let key = work_draft_key(&colliding, &DatasetId::from("data-a"), &assignment);
        assert!(key.starts_with(&current.prefix()));
        assert!(!current.owns_key(&key));
        assert!(colliding.owns_key(&key));

        let mut app = crate::app::LabelloApp::default();
        app.runtime.persistence.identity = Some(current);
        app.handle_persistence_completion(PersistenceCompletion::Saved {
            command: QueuedPersistenceCommand {
                identity: colliding,
                command: PersistenceCommand::Save(Box::new(DraftRecord::Work(Box::new(
                    work_draft(),
                )))),
                attempt: 0,
                ready_at: Instant::now(),
            },
            result: Ok(()),
        });
        assert!(app.runtime.persistence.last_work_draft.is_none());
    }

    #[test]
    fn admin_drafts_exclude_the_image_index() {
        let baseline = metadata("baseline");
        let mut config = baseline.clone();
        config.images.insert(
            ImageId::from("image-a"),
            labello_domain::ImageRecord {
                image_id: ImageId::from("image-a"),
                blake3: "hash".to_string(),
                canonical_path: "x".repeat(MAX_ADMIN_DRAFT_BYTES),
                known_paths: Vec::new(),
                duplicate_paths: Vec::new(),
                source_memberships: None,
                file_name: "image.png".to_string(),
                byte_size: 1,
                width: 1,
                height: 1,
                media_type: "image/png".to_string(),
            },
        );
        let draft = AdminDraft::new(&identity(), DatasetId::from("data-a"), &baseline, &config);
        assert!(draft.config.images.is_empty());
        assert!(DraftRecord::Admin(Box::new(draft)).validate_size().is_ok());
    }

    fn metadata(name: &str) -> DatasetMetadata {
        let mut metadata =
            DatasetMetadata::new(DatasetId::from("data-a"), name, labello_domain::now());
        metadata.image_roots.clear();
        metadata.label_classes = vec![LabelClass {
            class_id: ClassId::from("class"),
            name: "Class".to_string(),
            color: "#ffffff".to_string(),
            description: None,
        }];
        metadata
    }

    fn poll<T>(future: impl Future<Output = T>) -> T {
        use std::task::{Context, Poll, Waker};
        let mut future = Box::pin(future);
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("memory store future was unexpectedly pending"),
        }
    }
}
