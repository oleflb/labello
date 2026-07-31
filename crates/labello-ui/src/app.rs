use std::{
    collections::{BTreeSet, VecDeque},
    rc::Rc,
    sync::mpsc,
};
#[cfg(not(target_arch = "wasm32"))]
use std::{future::Future, pin::Pin};

use eframe::egui::{self, TextureHandle};
use labello_client::{AuthOptions, DatasetSummary, DatasetUser, ImageExplorerQuery, LabelloApi};
use labello_domain::{
    AnnotationGeometry, AnnotationId, AnnotationOrigin, AnnotationType, Assignment, AssignmentKind,
    BoundingBox, ClassId, DatasetId, DatasetMetadata, DatasetRole, DatasetRoleAssignment,
    DatasetSnapshot, DatasetStats, HumanRevisionKind, ImageExplorerPage, ImageId, ImageRecord,
    ImageState, KeybindingSet, KeypointAnnotation, KeypointState, LabelClass, NormalizedPoint,
    PrelabelSuggestion, RevisionSource, SkeletonGeometry, TaskDefinition, TaskId, TaskStatus,
    Timestamp, TutorialContent, UserAccount, UserId,
};
use web_time::{Duration, Instant};

use crate::{
    canvas::CanvasState,
    import_flow::ImportFlowState,
    manual_migration::ManualMigrationState,
    queue::{ImageQueue, QueuedImage},
    theme,
};

pub(crate) use crate::live_protocol::{
    FolderUploadProgress, ImportActivity, ImportRequestIdentity, LoadedAdmin, LoadedDataset,
    LoadedImage, MigrationAction, RequestIdentity, ReviewPhase, UiCommand, UiMessage,
};

pub const IMAGE_QUEUE_SIZE: usize = 2;
pub(crate) const ASSIGNMENT_AVAILABILITY_CACHE_TTL: Duration = Duration::from_secs(30);
pub(crate) const ADJUDICATION_UNAVAILABLE_MESSAGE: &str =
    "Adjudication is unavailable until independent-agreement routing is implemented.";
const MAX_HISTORY_OPERATIONS: usize = 256;
const MAX_HISTORY_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub api_base_url: String,
    pub application_url: Option<String>,
    pub user_id: UserId,
    pub dataset_id: DatasetId,
    pub queue_size: usize,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            api_base_url: "http://127.0.0.1:8080".to_string(),
            application_url: None,
            user_id: UserId::from("demo_user"),
            dataset_id: DatasetId::from("demo"),
            queue_size: IMAGE_QUEUE_SIZE,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tool {
    BoundingBox,
    Keypoints,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SaveStatus {
    Idle,
    Dirty,
    Saved,
    Saving,
    Retry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppView {
    Setup,
    Annotate,
    Review,
    Adjudicate,
    Admin,
    Stats,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LayoutMode {
    Compact,
    Medium,
    Wide,
}

impl LayoutMode {
    pub(crate) const COMPACT_MAX_WIDTH: f32 = 600.0;
    pub(crate) const TASK_PANEL_WIDTH: f32 = 280.0;
    pub(crate) const INSPECTOR_PANEL_WIDTH: f32 = 315.0;
    pub(crate) const MIN_WIDE_CANVAS_WIDTH: f32 = 645.0;
    const WIDE_GUTTERS: f32 = 48.0;

    pub(crate) fn for_width(width: f32) -> Self {
        if width < Self::COMPACT_MAX_WIDTH {
            Self::Compact
        } else if width
            < Self::TASK_PANEL_WIDTH
                + Self::INSPECTOR_PANEL_WIDTH
                + Self::MIN_WIDE_CANVAS_WIDTH
                + Self::WIDE_GUTTERS
        {
            Self::Medium
        } else {
            Self::Wide
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Drawer {
    Workflow,
    Inspector,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PendingTransition {
    NextAssignment,
    PreviousAssignment(Assignment),
    Workflow(TaskId),
    View(AppView),
}

pub(crate) struct RuntimeState {
    pub api: Option<Rc<dyn LabelloApi>>,
    pub tx: mpsc::Sender<UiMessage>,
    pub rx: mpsc::Receiver<UiMessage>,
    pub commands: VecDeque<UiCommand>,
    pub active_requests: BTreeSet<u64>,
    pub repaint_ctx: Option<egui::Context>,
    pub error: Option<String>,
    pub storage_error: Option<String>,
    pub notice: Option<String>,
    pub persistence: crate::persistence::PersistenceState,
    pub import_chunk_uploader: Option<crate::import_flow::RawImportChunkUploader>,
    #[cfg(not(target_arch = "wasm32"))]
    pub native_task_spawner: Option<NativeTaskSpawner>,
}

#[cfg(not(target_arch = "wasm32"))]
type NativeTaskSpawner = Rc<dyn Fn(Pin<Box<dyn Future<Output = ()> + 'static>>) + 'static>;

impl RuntimeState {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            api: None,
            tx,
            rx,
            commands: VecDeque::new(),
            active_requests: BTreeSet::new(),
            repaint_ctx: None,
            error: None,
            storage_error: None,
            notice: None,
            persistence: Default::default(),
            import_chunk_uploader: None,
            #[cfg(not(target_arch = "wasm32"))]
            native_task_spawner: None,
        }
    }
}

#[derive(Default)]
pub(crate) struct LoadingState {
    pub session: bool,
    pub logout: bool,
    pub datasets: bool,
    pub dataset: bool,
    pub admin: bool,
    pub roles_user: Option<UserId>,
    pub image: bool,
    pub saving: bool,
    pub ingesting: bool,
    pub ingest_polling: bool,
    pub ingest_job_id: Option<String>,
    pub last_ingest_poll: Option<Instant>,
    pub uploading: bool,
    pub upload_progress: Option<FolderUploadProgress>,
    pub stats: bool,
    pub keybindings: bool,
    pub images: bool,
    pub snapshots: bool,
    pub creating_snapshot: bool,
    pub snapshot_file: Option<(String, String)>,
}

#[derive(Default)]
pub(crate) struct AssignmentAvailabilityState {
    pub(crate) dataset_id: Option<DatasetId>,
    pub(crate) kind: Option<AssignmentKind>,
    pub(crate) tasks: std::collections::BTreeMap<TaskId, bool>,
    pub(crate) resolved: bool,
    pub(crate) checked_at: Option<Timestamp>,
    pub(crate) loading: bool,
    pub(crate) load_after_resolution: bool,
    pub(crate) refresh_after_load: bool,
    pub(crate) error: Option<String>,
    pub(crate) last_attempt: Option<Instant>,
    pub(crate) cache: Vec<CachedAssignmentAvailability>,
}

#[derive(Clone)]
pub(crate) struct CachedAssignmentAvailability {
    pub(crate) dataset_id: DatasetId,
    pub(crate) kind: AssignmentKind,
    pub(crate) tasks: std::collections::BTreeMap<TaskId, bool>,
    pub(crate) checked_at: Timestamp,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AdminSection {
    #[default]
    Overview,
    People,
    Images,
    Schema,
    Automation,
    Backups,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SetupSection {
    #[default]
    Datasets,
    Connection,
    Create,
    Import,
}

pub(crate) struct AdminToolsState {
    pub dataset_id: Option<DatasetId>,
    pub section: AdminSection,
    pub load_error: Option<String>,
    pub upload_error: Option<String>,
    pub people_search: String,
    pub image_query: ImageExplorerQuery,
    pub image_search: String,
    pub image_task: Option<TaskId>,
    pub image_class: Option<ClassId>,
    pub image_status: Option<TaskStatus>,
    pub images: Option<ImageExplorerPage>,
    pub images_error: Option<String>,
    pub snapshots: Vec<DatasetSnapshot>,
    pub snapshots_loaded: bool,
    pub snapshots_error: Option<String>,
    pub snapshot_action_error: Option<String>,
    pub confirm_discard: bool,
    pub pending_role_saves: VecDeque<UserId>,
}

impl Default for AdminToolsState {
    fn default() -> Self {
        Self {
            dataset_id: None,
            section: AdminSection::default(),
            load_error: None,
            upload_error: None,
            people_search: String::new(),
            image_query: ImageExplorerQuery {
                page: 1,
                page_size: 25,
                ..Default::default()
            },
            image_search: String::new(),
            image_task: None,
            image_class: None,
            image_status: None,
            images: None,
            images_error: None,
            snapshots: Vec::new(),
            snapshots_loaded: false,
            snapshots_error: None,
            snapshot_action_error: None,
            confirm_discard: false,
            pending_role_saves: VecDeque::new(),
        }
    }
}

pub(crate) struct SetupState {
    pub api_base_url_draft: String,
    pub create_dataset_id: String,
    pub create_dataset_name: String,
    pub started: bool,
    pub section: SetupSection,
}

pub(crate) struct AuthState {
    pub account: Option<UserAccount>,
    pub can_create_datasets: bool,
    pub options: AuthOptions,
    pub options_checked: bool,
    pub checked: bool,
    pub session_request_id: u64,
    pub active_session_request_id: Option<u64>,
    pub local_admin_login_pending: bool,
}

pub(crate) struct DatasetState {
    pub summaries: Vec<DatasetSummary>,
    pub summaries_error: Option<String>,
    pub metadata: Option<DatasetMetadata>,
    pub admin_config: Option<DatasetMetadata>,
    pub admin_baseline: Option<DatasetMetadata>,
    pub stats: DatasetStats,
    pub stats_request_id: u64,
    pub active_stats_request: Option<(u64, DatasetId)>,
    pub last_stats_attempt: Option<Instant>,
    pub last_stats_completion: Option<Instant>,
    pub stats_error: Option<String>,
    pub requested_view: Option<AppView>,
    pub users: Vec<DatasetUser>,
    pub users_baseline: Vec<DatasetUser>,
}

impl DatasetState {
    fn new() -> Self {
        Self {
            summaries: Vec::new(),
            summaries_error: None,
            metadata: None,
            admin_config: None,
            admin_baseline: None,
            stats: DatasetStats::default(),
            stats_request_id: 0,
            active_stats_request: None,
            last_stats_attempt: None,
            last_stats_completion: None,
            stats_error: None,
            requested_view: None,
            users: Vec::new(),
            users_baseline: Vec::new(),
        }
    }
}

#[derive(Default)]
pub(crate) struct NavigationState {
    pub(crate) drawer_open: bool,
    pub(crate) restore_drawer_trigger_focus: bool,
}

pub struct WorkState {
    pub(crate) classes: Vec<LabelClass>,
    pub(crate) tasks: Vec<TaskDefinition>,
    pub(crate) selected_task_id: Option<TaskId>,
    pub(crate) tool: Tool,
    pub(crate) assignment: Option<Assignment>,
    pub(crate) previous_annotation_assignment: Option<Assignment>,
    pub(crate) current: Option<QueuedImage>,
    pub(crate) current_state: Option<ImageState>,
    pub(crate) current_texture: Option<TextureHandle>,
    pub(crate) queue: ImageQueue,
    pub(crate) annotations: Vec<labello_domain::AnnotationVersion>,
    pub(crate) persisted_annotations: BTreeSet<AnnotationId>,
    pub(crate) modified_annotations: BTreeSet<AnnotationId>,
    pub(crate) accepted_prelabels: Vec<String>,
    pub(crate) selected_prelabel: Option<String>,
    pub(crate) selected_annotation: Option<AnnotationId>,
    pub(crate) active_skeleton: Option<AnnotationId>,
    pub(crate) skeleton_keypoint_index: usize,
    pub(crate) next_keypoint_hidden: bool,
    pub(crate) keybindings: KeybindingSet,
    pub(crate) canvas: CanvasState,
    pub(crate) save_status: SaveStatus,
    pub(crate) edit_generation: u64,
    pub(crate) last_edit_at: Option<Instant>,
    pub(crate) undo_stack: Vec<EditSnapshot>,
    pub(crate) redo_stack: Vec<EditSnapshot>,
    pub(crate) offline: bool,
    pub(crate) review_index: usize,
    pub(crate) review_rejected: bool,
    pub(crate) correction_draft: Option<CorrectionDraft>,
    pub(crate) show_tutorial: bool,
    pub(crate) pending_transition: Option<PendingTransition>,
    pub(crate) drawer: Option<Drawer>,
    pub(crate) workflow_panel_collapsed: bool,
    pub(crate) inspector_panel_collapsed: bool,
    pub(crate) show_settings: bool,
    pub(crate) shortcut_settings: ShortcutSettingsState,
    pub(crate) next_operation_id: u64,
    pub(crate) active_load_id: Option<u64>,
    pub(crate) active_prefetch_id: Option<u64>,
    pub(crate) active_operation_id: Option<u64>,
    pub(crate) one_shot_excluded_image_id: Option<ImageId>,
    pub(crate) next_demo_image_index: usize,
    pub(crate) migration: ManualMigrationState,
    pub(crate) availability: AssignmentAvailabilityState,
}

#[derive(Default)]
pub(crate) struct ShortcutSettingsState {
    pub(crate) draft: Option<KeybindingSet>,
    pub(crate) baseline: Option<KeybindingSet>,
    pub(crate) error: Option<String>,
    pub(crate) search: String,
    pub(crate) recording: Option<labello_domain::UserAction>,
    pub(crate) confirm_discard: bool,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct CorrectionDraft {
    pub correction_id: labello_domain::CorrectionId,
    pub annotation_id: AnnotationId,
    pub expected_version: u32,
    pub original_geometry: AnnotationGeometry,
    pub edited_geometry: AnnotationGeometry,
    pub reason: String,
    pub geometry_history: Vec<AnnotationGeometry>,
    pub selected_keypoint: Option<usize>,
}

impl CorrectionDraft {
    pub(crate) fn geometry_changed(&self) -> bool {
        self.edited_geometry != self.original_geometry
    }
}

#[derive(Clone, Debug)]
pub(crate) struct EditSnapshot {
    annotations: Vec<labello_domain::AnnotationVersion>,
    accepted_prelabels: Vec<String>,
    selected_annotation: Option<AnnotationId>,
    active_skeleton: Option<AnnotationId>,
    skeleton_keypoint_index: usize,
    next_keypoint_hidden: bool,
    approx_bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkflowChoice {
    pub task_id: TaskId,
    pub task_name: String,
    pub annotation_type: AnnotationType,
}

impl WorkflowChoice {
    pub(crate) fn label(&self) -> String {
        self.task_name.clone()
    }
}

pub struct LabelloApp {
    pub(crate) config: AppConfig,
    pub(crate) runtime: RuntimeState,
    pub(crate) loading: LoadingState,
    pub(crate) setup: SetupState,
    pub(crate) import: ImportFlowState,
    pub(crate) auth: AuthState,
    pub(crate) datasets: DatasetState,
    pub(crate) admin: AdminToolsState,
    pub(crate) navigation: NavigationState,
    pub(crate) work: WorkState,
    pub(crate) view: AppView,
    pub(crate) auth_epoch: u64,
    pub(crate) workspace_epoch: u64,
    pub(crate) import_epoch: u64,
    pub(crate) theme_applied: bool,
}

impl Default for LabelloApp {
    fn default() -> Self {
        Self::demo(AppConfig::default())
    }
}

include!("app/construction.rs");
include!("app/selection.rs");
include!("app/transitions.rs");
include!("app/editing.rs");
include!("app/shortcuts.rs");
include!("app/support.rs");
include!("app/shell.rs");

#[cfg(test)]
mod history_tests {
    use super::*;

    #[test]
    fn undo_history_respects_operation_and_approximate_memory_budgets() {
        let mut app = LabelloApp::default();
        for index in 0..(MAX_HISTORY_OPERATIONS + 20) {
            app.create_bbox(BoundingBox {
                x: (index % 10) as f32 * 0.01,
                y: 0.1,
                width: 0.1,
                height: 0.1,
            });
        }
        assert!(app.work.undo_stack.len() <= MAX_HISTORY_OPERATIONS);

        app.work.undo_stack.clear();
        for _ in 0..20 {
            app.work.accepted_prelabels = vec!["x".repeat(2 * 1024 * 1024)];
            app.record_edit();
        }
        assert!(
            app.work
                .undo_stack
                .iter()
                .map(|snapshot| snapshot.approx_bytes)
                .sum::<usize>()
                <= MAX_HISTORY_BYTES
        );
        assert!(app.work.undo_stack.len() < 20);
    }
}
