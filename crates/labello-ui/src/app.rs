use std::{
    collections::{BTreeSet, VecDeque},
    ops::{Deref, DerefMut},
    rc::Rc,
    sync::mpsc,
};
#[cfg(not(target_arch = "wasm32"))]
use std::{future::Future, pin::Pin};

use eframe::egui::{self, TextureHandle};
use labello_client::{
    AuthOptions, CorrectionRequest, DatasetSummary, DatasetUser, ImageExplorerQuery, IngestJob,
    LabelloApi, SessionInfo, SnapshotFile,
};
use labello_domain::{
    AdjudicationRecord, AnnotationGeometry, AnnotationId, AnnotationSource, AnnotationType,
    Assignment, AssignmentId, AssignmentKind, BoundingBox, ClassId, DatasetId, DatasetMetadata,
    DatasetRole, DatasetSnapshot, DatasetStats, ImageExplorerPage, ImageId, ImageRecord,
    ImageState, KeybindingSet, KeypointAnnotation, KeypointState, LabelClass, NormalizedPoint,
    PrelabelConfigId, PrelabelSuggestion, ReviewRecord, SkeletonGeometry, TaskDefinition, TaskId,
    TaskStatus, TutorialContent, UserAccount, UserId,
};
use web_time::{Duration, Instant};

use crate::{
    canvas::CanvasState,
    queue::{ImageQueue, QueuedImage},
    theme,
};

pub const IMAGE_QUEUE_SIZE: usize = 2;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReviewPhase {
    Object,
    FullImage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestIdentity {
    pub auth_epoch: u64,
    pub workspace_epoch: u64,
    pub request_id: u64,
    pub dataset_id: Option<DatasetId>,
}

#[derive(Debug)]
pub(crate) enum UiMessage {
    AuthOptionsLoaded {
        request: RequestIdentity,
        result: Result<AuthOptions, String>,
    },
    SessionLoaded {
        request: RequestIdentity,
        result: Result<SessionInfo, String>,
    },
    LogoutFinished {
        request: RequestIdentity,
        result: Result<(), String>,
    },
    GithubLoginUrl {
        request: RequestIdentity,
        result: Result<String, String>,
    },
    DatasetList {
        request: RequestIdentity,
        result: Result<Vec<DatasetSummary>, String>,
    },
    DatasetCreated {
        request: RequestIdentity,
        result: Box<Result<DatasetMetadata, String>>,
    },
    DatasetLoaded {
        request: RequestIdentity,
        result: Box<Result<LoadedDataset, String>>,
    },
    AdminLoaded {
        request: RequestIdentity,
        result: Box<Result<LoadedAdmin, String>>,
    },
    AdminSaved {
        request: RequestIdentity,
        result: Box<Result<DatasetMetadata, String>>,
    },
    DatasetRolesSaved {
        request: RequestIdentity,
        result: Result<DatasetUser, String>,
    },
    ImagesLoaded {
        request: RequestIdentity,
        result: Result<ImageExplorerPage, String>,
    },
    SnapshotsLoaded {
        request: RequestIdentity,
        result: Result<Vec<DatasetSnapshot>, String>,
    },
    SnapshotCreated {
        request: RequestIdentity,
        result: Result<DatasetSnapshot, String>,
    },
    SnapshotDownloaded {
        request: RequestIdentity,
        result: Result<SnapshotFile, String>,
    },
    ImageLoaded {
        request: RequestIdentity,
        operation_id: u64,
        assignment: Option<Assignment>,
        result: Box<Result<Option<LoadedImage>, String>>,
    },
    PreviousAssignmentLoaded {
        request: RequestIdentity,
        operation_id: u64,
        assignment: Option<Assignment>,
        result: Box<Result<LoadedImage, String>>,
    },
    PrefetchLoaded {
        request: RequestIdentity,
        operation_id: u64,
        result: Box<Result<Option<LoadedImage>, String>>,
    },
    ReservationReleased {
        request: RequestIdentity,
        result: Result<(), String>,
    },
    SaveFinished {
        request: RequestIdentity,
        operation_id: u64,
        assignment_id: AssignmentId,
        edit_generation: u64,
        completed: bool,
        result: Box<Result<ImageState, String>>,
    },
    ReleaseFinished {
        request: RequestIdentity,
        operation_id: u64,
        assignment_id: AssignmentId,
        result: Result<(), String>,
    },
    ReviewFinished {
        request: RequestIdentity,
        operation_id: u64,
        assignment_id: AssignmentId,
        phase: ReviewPhase,
        decision: labello_domain::ReviewDecision,
        result: Box<Result<ImageState, String>>,
    },
    CorrectionFinished {
        request: RequestIdentity,
        operation_id: u64,
        assignment_id: AssignmentId,
        result: Result<(), String>,
    },
    AdjudicationFinished {
        request: RequestIdentity,
        operation_id: u64,
        assignment_id: AssignmentId,
        result: Result<(), String>,
    },
    PersistenceFinished(Box<crate::persistence::PersistenceCompletion>),
    IngestJobLoaded {
        request: RequestIdentity,
        result: Result<IngestJob, String>,
    },
    StatsLoaded {
        request: RequestIdentity,
        result: Result<DatasetStats, String>,
    },
    KeybindingsSaved {
        request: RequestIdentity,
        result: Result<KeybindingSet, String>,
    },
    #[allow(dead_code)]
    RequestFailed {
        request: RequestIdentity,
        error: String,
    },
    #[allow(dead_code)]
    FolderUploadProgress {
        request: RequestIdentity,
        progress: FolderUploadProgress,
    },
    #[allow(dead_code)]
    FolderUploadFinished {
        request: RequestIdentity,
        result: Result<String, String>,
    },
}

pub(crate) enum UiCommand {
    AuthOptions {
        request: RequestIdentity,
    },
    Session {
        request: RequestIdentity,
    },
    LocalAdminLogin {
        request: RequestIdentity,
    },
    Logout {
        request: RequestIdentity,
    },
    GithubLogin {
        request: RequestIdentity,
        return_to: Option<String>,
    },
    DatasetList {
        request: RequestIdentity,
    },
    CreateDataset {
        request: RequestIdentity,
        dataset_id: DatasetId,
        name: String,
        admin_user_id: UserId,
    },
    LoadDataset {
        request: RequestIdentity,
        dataset_id: DatasetId,
        user_id: UserId,
    },
    LoadAdmin {
        request: RequestIdentity,
        dataset_id: DatasetId,
    },
    SaveAdmin {
        request: RequestIdentity,
        metadata: DatasetMetadata,
    },
    SaveDatasetRoles {
        request: RequestIdentity,
        dataset_id: DatasetId,
        user_id: UserId,
        roles: Vec<DatasetRole>,
    },
    LoadImages {
        request: RequestIdentity,
        dataset_id: DatasetId,
        query: ImageExplorerQuery,
    },
    LoadSnapshots {
        request: RequestIdentity,
        dataset_id: DatasetId,
    },
    CreateSnapshot {
        request: RequestIdentity,
        dataset_id: DatasetId,
    },
    DownloadSnapshot {
        request: RequestIdentity,
        dataset_id: DatasetId,
        snapshot_id: String,
        path: String,
    },
    Ingest {
        request: RequestIdentity,
        dataset_id: DatasetId,
    },
    PollIngest {
        request: RequestIdentity,
        dataset_id: DatasetId,
        job_id: String,
    },
    Stats {
        request: RequestIdentity,
        dataset_id: DatasetId,
    },
    SaveKeybindings {
        request: RequestIdentity,
        dataset_id: DatasetId,
        keybindings: KeybindingSet,
    },
    ClaimAssignment {
        request: RequestIdentity,
        operation_id: u64,
        dataset_id: DatasetId,
        task_id: TaskId,
        prelabel_config_ids: Vec<PrelabelConfigId>,
        kind: AssignmentKind,
        reclaim_assignment_id: Option<AssignmentId>,
        excluded_image_ids: Vec<ImageId>,
    },
    PrefetchAssignment {
        request: RequestIdentity,
        operation_id: u64,
        dataset_id: DatasetId,
        task_id: TaskId,
        prelabel_config_ids: Vec<PrelabelConfigId>,
        excluded_image_ids: Vec<ImageId>,
    },
    ReleaseReservation {
        request: RequestIdentity,
        dataset_id: DatasetId,
        assignment: Assignment,
    },
    ReloadAssignment {
        request: RequestIdentity,
        operation_id: u64,
        dataset_id: DatasetId,
        assignment: Assignment,
        prelabel_config_ids: Vec<PrelabelConfigId>,
        fetch_prelabels: bool,
    },
    ReopenAssignment {
        request: RequestIdentity,
        operation_id: u64,
        dataset_id: DatasetId,
        assignment: Assignment,
        prelabel_config_ids: Vec<PrelabelConfigId>,
    },
    SaveAnnotations {
        request: RequestIdentity,
        operation_id: u64,
        edit_generation: u64,
        dataset_id: DatasetId,
        assignment: Assignment,
        annotations: Vec<labello_domain::AnnotationVersion>,
        persisted: BTreeSet<AnnotationId>,
        modified: BTreeSet<AnnotationId>,
        submit: bool,
    },
    ReleaseAssignment {
        request: RequestIdentity,
        operation_id: u64,
        dataset_id: DatasetId,
        assignment: Assignment,
    },
    Review {
        request: RequestIdentity,
        operation_id: u64,
        dataset_id: DatasetId,
        assignment: Assignment,
        review: ReviewRecord,
        phase: ReviewPhase,
    },
    Correction {
        request: RequestIdentity,
        operation_id: u64,
        dataset_id: DatasetId,
        assignment: Assignment,
        correction: CorrectionRequest,
    },
    Adjudication {
        request: RequestIdentity,
        operation_id: u64,
        dataset_id: DatasetId,
        assignment: Assignment,
        adjudication: AdjudicationRecord,
    },
}

impl UiCommand {
    pub(crate) fn request(&self) -> &RequestIdentity {
        match self {
            Self::AuthOptions { request }
            | Self::Session { request }
            | Self::LocalAdminLogin { request }
            | Self::Logout { request }
            | Self::GithubLogin { request, .. }
            | Self::DatasetList { request }
            | Self::CreateDataset { request, .. }
            | Self::LoadDataset { request, .. }
            | Self::LoadAdmin { request, .. }
            | Self::SaveAdmin { request, .. }
            | Self::SaveDatasetRoles { request, .. }
            | Self::LoadImages { request, .. }
            | Self::LoadSnapshots { request, .. }
            | Self::CreateSnapshot { request, .. }
            | Self::DownloadSnapshot { request, .. }
            | Self::Ingest { request, .. }
            | Self::PollIngest { request, .. }
            | Self::Stats { request, .. }
            | Self::SaveKeybindings { request, .. }
            | Self::ClaimAssignment { request, .. }
            | Self::PrefetchAssignment { request, .. }
            | Self::ReleaseReservation { request, .. }
            | Self::ReloadAssignment { request, .. }
            | Self::ReopenAssignment { request, .. }
            | Self::SaveAnnotations { request, .. }
            | Self::ReleaseAssignment { request, .. }
            | Self::Review { request, .. }
            | Self::Correction { request, .. }
            | Self::Adjudication { request, .. } => request,
        }
    }
}

impl UiMessage {
    pub(crate) fn request(&self) -> Option<&RequestIdentity> {
        match self {
            Self::AuthOptionsLoaded { request, .. }
            | Self::SessionLoaded { request, .. }
            | Self::LogoutFinished { request, .. }
            | Self::GithubLoginUrl { request, .. }
            | Self::DatasetList { request, .. }
            | Self::DatasetCreated { request, .. }
            | Self::DatasetLoaded { request, .. }
            | Self::AdminLoaded { request, .. }
            | Self::AdminSaved { request, .. }
            | Self::DatasetRolesSaved { request, .. }
            | Self::ImagesLoaded { request, .. }
            | Self::SnapshotsLoaded { request, .. }
            | Self::SnapshotCreated { request, .. }
            | Self::SnapshotDownloaded { request, .. }
            | Self::ImageLoaded { request, .. }
            | Self::PreviousAssignmentLoaded { request, .. }
            | Self::PrefetchLoaded { request, .. }
            | Self::ReservationReleased { request, .. }
            | Self::SaveFinished { request, .. }
            | Self::ReleaseFinished { request, .. }
            | Self::ReviewFinished { request, .. }
            | Self::CorrectionFinished { request, .. }
            | Self::AdjudicationFinished { request, .. }
            | Self::IngestJobLoaded { request, .. }
            | Self::StatsLoaded { request, .. }
            | Self::KeybindingsSaved { request, .. }
            | Self::RequestFailed { request, .. } => Some(request),
            Self::PersistenceFinished(_)
            | Self::FolderUploadProgress { .. }
            | Self::FolderUploadFinished { .. } => None,
        }
    }
}

#[derive(Debug)]
pub(crate) struct LoadedDataset {
    pub metadata: DatasetMetadata,
    pub keybindings: KeybindingSet,
}

#[derive(Debug)]
pub(crate) struct LoadedAdmin {
    pub metadata: DatasetMetadata,
    pub users: Vec<DatasetUser>,
}

#[derive(Clone, Debug)]
pub(crate) struct LoadedImage {
    pub assignment: Assignment,
    pub queued: QueuedImage,
    pub annotations: Vec<labello_domain::AnnotationVersion>,
    pub state: ImageState,
    pub color_image: Option<egui::ColorImage>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FolderUploadProgress {
    pub uploaded_files: u32,
    pub total_files: u32,
    pub current_batch: u32,
    pub message: String,
}

impl FolderUploadProgress {
    pub(crate) fn fraction(&self) -> f32 {
        if self.total_files == 0 {
            0.0
        } else {
            (self.uploaded_files as f32 / self.total_files as f32).clamp(0.0, 1.0)
        }
    }

    pub(crate) fn label(&self) -> String {
        if self.total_files == 0 {
            self.message.clone()
        } else {
            format!(
                "{} of {} files - {}",
                self.uploaded_files, self.total_files, self.message
            )
        }
    }
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
        }
    }
}

pub(crate) struct SetupState {
    pub api_base_url_draft: String,
    pub create_dataset_id: String,
    pub create_dataset_name: String,
    pub started: bool,
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
    pub(crate) show_settings: bool,
    pub(crate) shortcut_settings: ShortcutSettingsState,
    pub(crate) next_operation_id: u64,
    pub(crate) active_load_id: Option<u64>,
    pub(crate) active_prefetch_id: Option<u64>,
    pub(crate) active_operation_id: Option<u64>,
    pub(crate) one_shot_excluded_image_id: Option<ImageId>,
    pub(crate) next_demo_image_index: usize,
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
    pub(crate) auth: AuthState,
    pub(crate) datasets: DatasetState,
    pub(crate) admin_tools: AdminToolsState,
    pub(crate) work: WorkState,
    pub(crate) view: AppView,
    pub(crate) auth_epoch: u64,
    pub(crate) workspace_epoch: u64,
    pub(crate) theme_applied: bool,
}

impl Deref for LabelloApp {
    type Target = WorkState;

    fn deref(&self) -> &Self::Target {
        &self.work
    }
}

impl DerefMut for LabelloApp {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.work
    }
}

impl Default for LabelloApp {
    fn default() -> Self {
        Self::demo(AppConfig::default())
    }
}

impl LabelloApp {
    pub fn demo(config: AppConfig) -> Self {
        let classes = vec![LabelClass {
            class_id: ClassId::from("person"),
            name: "Person".to_string(),
            color: "#5eead4".to_string(),
            description: Some("Visible people in the image".to_string()),
        }];
        let tasks = vec![TaskDefinition {
            task_id: TaskId::from("bounding_box:person"),
            name: "Person bounding boxes".to_string(),
            annotation_type: AnnotationType::BoundingBox,
            class_ids: vec![ClassId::from("person")],
            instructions: TutorialContent {
                title: "Label every visible person".to_string(),
                example_text: "Draw tight boxes around each visible person. Include partially visible people, but skip reflections and posters.".to_string(),
                example_images: vec!["tutorial/person-box-example.png".to_string()],
            },
            skeleton: None,
            review: labello_domain::ReviewConfig::default(),
            prelabel_config_ids: vec![],
            enabled: true,
        }];
        let mut queue = ImageQueue::new(config.queue_size);
        for index in 2..=queue.queue_size() + 1 {
            queue.push_if_room(demo_image(index));
        }
        let current = Some(demo_image(1));
        let setup = SetupState {
            api_base_url_draft: config.api_base_url.clone(),
            create_dataset_id: config.dataset_id.to_string(),
            create_dataset_name: "Demo Dataset".to_string(),
            started: true,
        };
        let work = WorkState {
            classes,
            tasks,
            selected_task_id: Some(TaskId::from("bounding_box:person")),
            tool: Tool::BoundingBox,
            assignment: None,
            previous_annotation_assignment: None,
            current,
            current_state: None,
            current_texture: None,
            queue,
            annotations: Vec::new(),
            persisted_annotations: BTreeSet::new(),
            modified_annotations: BTreeSet::new(),
            accepted_prelabels: Vec::new(),
            selected_prelabel: None,
            selected_annotation: None,
            active_skeleton: None,
            skeleton_keypoint_index: 0,
            next_keypoint_hidden: false,
            keybindings: KeybindingSet::defaults_for(config.user_id.clone()),
            canvas: CanvasState::default(),
            save_status: SaveStatus::Idle,
            edit_generation: 0,
            last_edit_at: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            offline: false,
            review_index: 0,
            review_rejected: false,
            correction_draft: None,
            show_tutorial: false,
            pending_transition: None,
            drawer: None,
            show_settings: false,
            shortcut_settings: ShortcutSettingsState::default(),
            next_operation_id: 0,
            active_load_id: None,
            active_prefetch_id: None,
            active_operation_id: None,
            one_shot_excluded_image_id: None,
            next_demo_image_index: config.queue_size.clamp(1, IMAGE_QUEUE_SIZE) + 2,
        };
        Self {
            runtime: RuntimeState::new(),
            loading: LoadingState::default(),
            setup,
            auth: AuthState {
                account: None,
                can_create_datasets: false,
                options: AuthOptions {
                    github_oauth: false,
                    local_admin_login: false,
                },
                options_checked: true,
                checked: true,
                session_request_id: 0,
                active_session_request_id: None,
                local_admin_login_pending: false,
            },
            datasets: DatasetState::new(),
            admin_tools: AdminToolsState::default(),
            work,
            view: AppView::Annotate,
            auth_epoch: 0,
            workspace_epoch: 0,
            config,
            theme_applied: false,
        }
    }

    pub fn live_http(config: AppConfig) -> Self {
        let mut app = Self::demo(config);
        app.view = AppView::Setup;
        app.setup.started = false;
        app.setup.create_dataset_id.clear();
        app.setup.create_dataset_name.clear();
        app.auth.options_checked = false;
        app.auth.checked = false;
        app.current = None;
        app.queue.clear();
        app.rebuild_http_api();
        app
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_native_task_spawner(
        &mut self,
        spawner: impl Fn(Pin<Box<dyn Future<Output = ()> + 'static>>) + 'static,
    ) {
        self.runtime.native_task_spawner = Some(Rc::new(spawner));
    }

    pub(crate) fn selected_task(&self) -> Option<&TaskDefinition> {
        let selected = self.selected_task_id.as_ref()?;
        self.tasks
            .iter()
            .find(|task| task.task_id == *selected && valid_workflow(task))
    }

    pub(crate) fn selected_class_id(&self) -> Option<&ClassId> {
        self.selected_task()?.class_ids.first()
    }

    pub(crate) fn workflow_choices(&self) -> Vec<WorkflowChoice> {
        let mut choices = Vec::new();
        for task in &self.tasks {
            if !valid_workflow(task) {
                continue;
            }
            choices.push(WorkflowChoice {
                task_id: task.task_id.clone(),
                task_name: task.name.clone(),
                annotation_type: task.annotation_type.clone(),
            });
        }
        choices
    }

    pub(crate) fn selected_workflow(&self) -> Option<WorkflowChoice> {
        let task = self.selected_task()?;
        Some(WorkflowChoice {
            task_id: task.task_id.clone(),
            task_name: task.name.clone(),
            annotation_type: task.annotation_type.clone(),
        })
    }

    pub(crate) fn select_workflow(&mut self, task_id: &TaskId) -> bool {
        let Some(task) = self
            .tasks
            .iter()
            .find(|task| task.task_id == *task_id && valid_workflow(task))
        else {
            return false;
        };
        if self.selected_task_id.as_ref() == Some(task_id) {
            return false;
        }
        let annotation_type = task.annotation_type.clone();
        self.selected_task_id = Some(task_id.clone());
        self.tool = tool_for_annotation_type(&annotation_type);
        true
    }

    pub(crate) fn ensure_valid_task_selection(&mut self) -> bool {
        if self.selected_task().is_some() {
            return true;
        }
        let Some((task_id, annotation_type)) = self
            .tasks
            .iter()
            .find(|task| valid_workflow(task))
            .map(|task| (task.task_id.clone(), task.annotation_type.clone()))
        else {
            self.selected_task_id = None;
            return false;
        };
        self.selected_task_id = Some(task_id);
        self.tool = tool_for_annotation_type(&annotation_type);
        true
    }

    pub(crate) fn sync_work_config(&mut self, metadata: DatasetMetadata) {
        self.classes = metadata.label_classes.clone();
        self.tasks = metadata.tasks.clone();
        self.datasets.metadata = Some(metadata);
        if let Some(task_id) = self
            .runtime
            .persistence
            .preference
            .as_ref()
            .filter(|preference| preference.dataset_id == self.config.dataset_id)
            .and_then(|preference| preference.task_id.clone())
            && self
                .tasks
                .iter()
                .any(|task| task.task_id == task_id && valid_workflow(task))
        {
            self.selected_task_id = Some(task_id);
        }
        if self.ensure_valid_task_selection()
            && let Some(annotation_type) = self
                .selected_task()
                .map(|task| task.annotation_type.clone())
        {
            self.tool = tool_for_annotation_type(&annotation_type);
        }
    }

    pub(crate) fn annotation_matches_selected_workflow(
        &self,
        annotation: &labello_domain::AnnotationVersion,
    ) -> bool {
        let Some(task) = self.selected_task() else {
            return false;
        };
        let Some(class_id) = self.selected_class_id() else {
            return false;
        };
        annotation.task_id == task.task_id && &annotation.class_id == class_id
    }

    pub(crate) fn has_dataset_role(&self, role: DatasetRole) -> bool {
        self.datasets
            .summaries
            .iter()
            .find(|summary| summary.dataset_id == self.config.dataset_id)
            .is_some_and(|summary| summary.roles.contains(&role))
    }

    pub(crate) fn can_open_view(&self, view: AppView) -> bool {
        let role = match view {
            AppView::Annotate => Some(DatasetRole::Annotator),
            AppView::Review => Some(DatasetRole::Reviewer),
            AppView::Adjudicate => Some(DatasetRole::Adjudicator),
            AppView::Admin => Some(DatasetRole::DataAdmin),
            AppView::Setup | AppView::Stats => None,
        };
        role.is_none_or(|role| self.has_dataset_role(role))
    }

    pub(crate) fn assignment_kind(&self) -> Option<AssignmentKind> {
        match self.view {
            AppView::Annotate => Some(AssignmentKind::Annotation),
            AppView::Review => Some(AssignmentKind::Review),
            AppView::Adjudicate => Some(AssignmentKind::Adjudication),
            AppView::Setup | AppView::Admin | AppView::Stats => None,
        }
    }

    pub(crate) fn work_view(&self) -> bool {
        self.assignment_kind().is_some()
    }

    pub(crate) fn admin_changes_dirty(&self) -> bool {
        self.datasets.admin_config != self.datasets.admin_baseline
            || self.datasets.users != self.datasets.users_baseline
    }

    pub(crate) fn short_viewport(size: egui::Vec2) -> bool {
        size.y < 480.0
    }

    pub(crate) fn workspace_context_height(&self, layout: LayoutMode, viewport: egui::Vec2) -> f32 {
        if self.current.is_some()
            && !Self::short_viewport(viewport)
            && (layout == LayoutMode::Compact
                || (layout == LayoutMode::Medium && self.view != AppView::Annotate))
        {
            100.0
        } else {
            56.0
        }
    }

    pub(crate) fn workspace_actions_height(&self, layout: LayoutMode, viewport: egui::Vec2) -> f32 {
        if layout == LayoutMode::Compact
            && self.view == AppView::Review
            && self.correction_draft.is_none()
        {
            112.0
        } else if Self::short_viewport(viewport) || layout == LayoutMode::Compact {
            60.0
        } else {
            68.0
        }
    }

    pub(crate) fn class_name(&self, class_id: &ClassId) -> String {
        self.classes
            .iter()
            .find(|class| &class.class_id == class_id)
            .map(|class| class.name.clone())
            .unwrap_or_else(|| class_id.to_string())
    }

    pub(crate) fn request_transition(&mut self, transition: PendingTransition) {
        if self.loading.saving || self.loading.image || self.transition_is_current(&transition) {
            return;
        }
        if self.assignment.is_some() {
            self.pending_transition = Some(transition);
            return;
        }
        self.execute_transition(transition);
    }

    pub(crate) fn execute_transition(&mut self, transition: PendingTransition) {
        match transition {
            PendingTransition::NextAssignment => {
                if self.runtime.api.is_some() {
                    self.clear_current_image();
                    self.request_next_image();
                } else {
                    self.advance_current_image();
                }
            }
            PendingTransition::PreviousAssignment(assignment) => {
                self.previous_annotation_assignment = Some(assignment.clone());
                self.clear_current_image();
                self.request_reopen_assignment(assignment);
            }
            PendingTransition::Workflow(task_id) => {
                if self.select_workflow(&task_id) {
                    self.clear_previous_annotation_assignment();
                    self.begin_workspace_epoch();
                    self.clear_current_image();
                    self.request_next_image();
                }
            }
            PendingTransition::View(view) => {
                self.show_tutorial = false;
                self.drawer = None;
                self.begin_workspace_epoch();
                self.clear_previous_annotation_assignment();
                if view == AppView::Admin {
                    self.clear_current_image();
                    self.request_admin_dataset();
                    return;
                }
                self.clear_current_image();
                self.view = view;
                if matches!(
                    view,
                    AppView::Annotate | AppView::Review | AppView::Adjudicate
                ) {
                    self.request_next_image();
                } else if view == AppView::Stats {
                    self.request_stats();
                }
            }
        }
    }

    fn transition_is_current(&self, transition: &PendingTransition) -> bool {
        match transition {
            PendingTransition::NextAssignment => false,
            PendingTransition::PreviousAssignment(_) => false,
            PendingTransition::Workflow(task_id) => self.selected_task_id.as_ref() == Some(task_id),
            PendingTransition::View(view) => self.view == *view,
        }
    }

    pub(crate) fn submit_pending_transition(&mut self) {
        if self.view != AppView::Annotate || self.pending_transition.is_none() {
            return;
        }
        if let Some(issue) = self.submission_issue() {
            self.runtime.error = Some(issue);
            return;
        }
        self.request_save(true);
    }

    pub(crate) fn release_pending_transition(&mut self) {
        if self.pending_transition.is_some() {
            self.request_release();
        }
    }

    pub(crate) fn cancel_pending_transition(&mut self) {
        if !self.loading.saving {
            self.pending_transition = None;
        }
    }

    pub(crate) fn submit_and_advance(&mut self) {
        if self.view != AppView::Annotate
            || self.loading.saving
            || (self.assignment.is_none() && self.runtime.api.is_some())
        {
            return;
        }
        if let Some(issue) = self.submission_issue() {
            self.runtime.error = Some(issue);
            return;
        }
        if self.runtime.api.is_none() {
            self.execute_transition(PendingTransition::NextAssignment);
            return;
        }
        self.pending_transition = Some(PendingTransition::NextAssignment);
        self.request_save(true);
    }

    pub(crate) fn skip_assignment(&mut self) {
        if self.loading.saving || (self.assignment.is_none() && self.runtime.api.is_some()) {
            return;
        }
        if self.view == AppView::Annotate
            && self.runtime.api.is_some()
            && matches!(self.save_status, SaveStatus::Dirty | SaveStatus::Retry)
        {
            self.pending_transition = Some(PendingTransition::NextAssignment);
            return;
        }
        if self.runtime.api.is_none() {
            self.execute_transition(PendingTransition::NextAssignment);
            return;
        }
        self.pending_transition = Some(PendingTransition::NextAssignment);
        self.request_release();
    }

    pub(crate) fn return_to_previous_assignment(&mut self) {
        if self.view != AppView::Annotate
            || self.loading.saving
            || self.loading.image
            || self.pending_transition.is_some()
            || self.runtime.api.is_none()
        {
            return;
        }
        let Some(previous) = self.previous_annotation_assignment.clone() else {
            return;
        };
        if previous.status == labello_domain::AssignmentStatus::Active
            && previous
                .expires_at
                .is_some_and(|expires_at| expires_at <= labello_domain::now())
        {
            self.clear_previous_annotation_assignment();
            self.runtime.error = Some(
                "The previous assignment lease expired and can no longer be returned to."
                    .to_string(),
            );
            return;
        }
        if self.assignment.is_some()
            && matches!(self.save_status, SaveStatus::Dirty | SaveStatus::Retry)
        {
            self.pending_transition = Some(PendingTransition::PreviousAssignment(previous));
            return;
        }
        self.request_reopen_assignment(previous);
    }

    fn submission_issue(&self) -> Option<String> {
        let task = self.selected_task()?;
        let spec = task.skeleton.as_ref()?;
        if self.active_skeleton.is_some() {
            return Some(
                "Finish the active skeleton or mark its remaining optional keypoints absent before submitting."
                    .to_string(),
            );
        }
        for annotation in self.annotations.iter().filter(|annotation| {
            !annotation.deleted && self.annotation_matches_selected_workflow(annotation)
        }) {
            let AnnotationGeometry::Skeleton(skeleton) = &annotation.geometry else {
                continue;
            };
            if let Some(required) = spec.keypoints.iter().find(|required| {
                required.required
                    && skeleton.keypoints.iter().any(|keypoint| {
                        keypoint.name == required.name && keypoint.state == KeypointState::Absent
                    })
            }) {
                return Some(format!(
                    "Required keypoint '{}' is absent. Place it before submitting.",
                    required.name
                ));
            }
        }
        None
    }

    pub(crate) fn advance_current_image(&mut self) {
        self.assignment = None;
        self.current_texture = None;
        self.current_state = None;
        self.current = self.queue.pop_next();
        self.annotations.clear();
        self.persisted_annotations.clear();
        self.modified_annotations.clear();
        self.accepted_prelabels.clear();
        self.selected_prelabel = None;
        self.selected_annotation = None;
        if self.runtime.api.is_some() {
            self.request_next_image();
        } else {
            self.replenish_demo_queue();
        }
    }

    pub(crate) fn autosave(&mut self) {
        if self.view == AppView::Annotate
            && matches!(self.save_status, SaveStatus::Dirty | SaveStatus::Retry)
        {
            if self.runtime.api.is_some() {
                self.request_save(false);
                return;
            }
            self.save_status = if self.offline {
                SaveStatus::Saving
            } else {
                SaveStatus::Saved
            };
        }
    }

    pub(crate) fn can_correct_review_object(&self) -> bool {
        self.view == AppView::Review
            && self
                .assignment
                .as_ref()
                .is_some_and(|assignment| assignment.kind == AssignmentKind::Review)
            && self.selected_task().is_some_and(|task| {
                task.review.workflow == labello_domain::ReviewWorkflow::Approval
                    && task.review.allow_reviewer_corrections
            })
            && self.current_review_annotation().is_some_and(|annotation| {
                self.selected_annotation.as_ref() == Some(&annotation.annotation_id)
            })
    }

    pub(crate) fn current_review_annotation(&self) -> Option<&labello_domain::AnnotationVersion> {
        (self.view == AppView::Review).then_some(()).and_then(|()| {
            self.annotations
                .iter()
                .filter(|annotation| {
                    !annotation.deleted && self.annotation_matches_selected_workflow(annotation)
                })
                .nth(self.review_index)
        })
    }

    pub(crate) fn start_correction(&mut self) {
        if self.correction_draft.is_some() || !self.can_correct_review_object() {
            return;
        }
        let Some(annotation) = self.current_review_annotation().cloned() else {
            return;
        };
        let annotation_id = annotation.annotation_id.clone();
        self.correction_draft = Some(CorrectionDraft {
            correction_id: labello_domain::CorrectionId::generate(),
            annotation_id,
            expected_version: annotation.version,
            original_geometry: annotation.geometry.clone(),
            edited_geometry: annotation.geometry,
            reason: String::new(),
            geometry_history: Vec::new(),
            selected_keypoint: None,
        });
        self.runtime.error = None;
    }

    pub(crate) fn discard_correction(&mut self) {
        self.correction_draft = None;
    }

    pub(crate) fn undo_correction(&mut self) {
        let Some(draft) = self.correction_draft.as_mut() else {
            return;
        };
        if let Some(geometry) = draft.geometry_history.pop() {
            draft.edited_geometry = geometry;
        }
    }

    fn update_correction_geometry(&mut self, geometry: AnnotationGeometry) {
        let Some(draft) = self.correction_draft.as_mut() else {
            return;
        };
        if draft.edited_geometry == geometry {
            return;
        }
        draft.geometry_history.push(draft.edited_geometry.clone());
        draft.edited_geometry = geometry;
        self.runtime.error = None;
    }

    pub(crate) fn edit_correction_bbox(&mut self, edit: crate::canvas::BoundingBoxEdit) {
        let Some(draft) = self.correction_draft.as_ref() else {
            return;
        };
        if draft.annotation_id != edit.annotation_id
            || !matches!(&draft.edited_geometry, AnnotationGeometry::BoundingBox(_))
        {
            return;
        }
        self.update_correction_geometry(AnnotationGeometry::BoundingBox(edit.bounding_box));
    }

    pub(crate) fn select_correction_keypoint(&mut self, index: usize) {
        let Some(draft) = self.correction_draft.as_mut() else {
            return;
        };
        let AnnotationGeometry::Skeleton(skeleton) = &draft.edited_geometry else {
            return;
        };
        if index < skeleton.keypoints.len() {
            draft.selected_keypoint = Some(index);
        }
    }

    pub(crate) fn edit_correction_keypoint(&mut self, edit: crate::canvas::KeypointEdit) {
        let Some(draft) = self.correction_draft.as_ref() else {
            return;
        };
        if draft.annotation_id != edit.annotation_id {
            return;
        }
        let mut geometry = draft.edited_geometry.clone();
        let AnnotationGeometry::Skeleton(skeleton) = &mut geometry else {
            return;
        };
        let Some(keypoint) = skeleton.keypoints.get_mut(edit.keypoint_index) else {
            return;
        };
        keypoint.point = Some(edit.point);
        self.update_correction_geometry(geometry);
        self.select_correction_keypoint(edit.keypoint_index);
    }

    pub(crate) fn set_correction_keypoint_state(&mut self, state: KeypointState) {
        let Some(draft) = self.correction_draft.as_ref() else {
            return;
        };
        let Some(index) = draft.selected_keypoint else {
            return;
        };
        let mut geometry = draft.edited_geometry.clone();
        let AnnotationGeometry::Skeleton(skeleton) = &mut geometry else {
            return;
        };
        let Some(keypoint) = skeleton.keypoints.get_mut(index) else {
            return;
        };
        if keypoint.state == state {
            return;
        }
        if state == KeypointState::Absent {
            keypoint.point = None;
        } else if keypoint.point.is_none() {
            return;
        }
        keypoint.state = state;
        self.update_correction_geometry(geometry);
    }

    pub(crate) fn replenish_demo_queue(&mut self) {
        while self.queue.len() < self.queue.queue_size() {
            let image = demo_image(self.next_demo_image_index);
            self.queue.push_if_room(image);
            self.next_demo_image_index += 1;
        }
        self.queue.set_loading(false);
    }

    pub(crate) fn create_bbox(&mut self, bbox: BoundingBox) {
        let Some(task) = self.selected_task() else {
            return;
        };
        let Some(class_id) = self.selected_class_id() else {
            return;
        };
        let task_id = task.task_id.clone();
        let class_id = class_id.clone();
        let user_id = self.config.user_id.clone();
        let timestamp = labello_domain::now();
        let annotation_id = AnnotationId::generate();
        self.record_edit();
        self.annotations.push(labello_domain::AnnotationVersion {
            annotation_id: annotation_id.clone(),
            version: 1,
            task_id,
            class_id,
            annotation_type: AnnotationType::BoundingBox,
            source: AnnotationSource::Human,
            geometry: AnnotationGeometry::BoundingBox(bbox),
            author_user_id: user_id,
            created_at: timestamp,
            updated_at: timestamp,
            deleted: false,
        });
        self.selected_annotation = Some(annotation_id);
        self.mark_edited();
    }

    pub(crate) fn edit_bbox(&mut self, edit: crate::canvas::BoundingBoxEdit) {
        let annotation_id = edit.annotation_id;
        let persisted = self.persisted_annotations.contains(&annotation_id);
        let persisted_version = self
            .current_state
            .as_ref()
            .and_then(|state| state.current_annotation(&annotation_id))
            .map(|annotation| annotation.version);
        let Some(index) = self.annotations.iter().position(|annotation| {
            annotation.annotation_id == annotation_id && !annotation.deleted
        }) else {
            return;
        };
        let AnnotationGeometry::BoundingBox(current) = &self.annotations[index].geometry else {
            return;
        };
        if *current == edit.bounding_box {
            return;
        }
        self.record_edit();
        let annotation = &mut self.annotations[index];
        let AnnotationGeometry::BoundingBox(current) = &mut annotation.geometry else {
            return;
        };
        *current = edit.bounding_box;
        annotation.updated_at = labello_domain::now();
        if persisted {
            annotation.version = persisted_version.unwrap_or(annotation.version) + 1;
            self.modified_annotations.insert(annotation_id);
        }
        self.mark_edited();
    }

    pub(crate) fn place_keypoint(&mut self, point: NormalizedPoint) {
        let Some(task) = self.selected_task().cloned() else {
            return;
        };
        let Some(spec) = task.skeleton.clone() else {
            self.runtime.error = Some(
                "This skeleton workflow has no keypoint specification. Ask a data admin to configure it."
                    .to_string(),
            );
            return;
        };
        if let Some(active_id) = self.work.active_skeleton.clone() {
            self.record_edit();
            let keypoint_index = self.work.skeleton_keypoint_index;
            let hidden = self.work.next_keypoint_hidden;
            let Some(annotation) =
                self.work.annotations.iter_mut().find(|annotation| {
                    annotation.annotation_id == active_id && !annotation.deleted
                })
            else {
                self.work.active_skeleton = None;
                return;
            };
            let AnnotationGeometry::Skeleton(skeleton) = &mut annotation.geometry else {
                return;
            };
            if let Some(keypoint) = skeleton.keypoints.get_mut(keypoint_index) {
                keypoint.state = if hidden {
                    KeypointState::Hidden
                } else {
                    KeypointState::Visible
                };
                keypoint.point = Some(point);
                annotation.updated_at = labello_domain::now();
                let completed = keypoint_index + 1 >= skeleton.keypoints.len();
                self.work.skeleton_keypoint_index = keypoint_index + 1;
                self.work.next_keypoint_hidden = false;
                if completed {
                    self.work.active_skeleton = None;
                    self.work.skeleton_keypoint_index = 0;
                }
                self.mark_edited();
            }
            return;
        }

        let Some(class_id) = self.selected_class_id().cloned() else {
            return;
        };
        let timestamp = labello_domain::now();
        let author_user_id = self.config.user_id.clone();
        let keypoint_count = spec.keypoints.len();
        let mut keypoints = spec
            .keypoints
            .into_iter()
            .map(|keypoint| KeypointAnnotation {
                name: keypoint.name,
                state: KeypointState::Absent,
                point: None,
            })
            .collect::<Vec<_>>();
        let Some(first) = keypoints.first_mut() else {
            self.runtime.error =
                Some("Skeleton workflows require at least one keypoint".to_string());
            return;
        };
        first.state = if self.next_keypoint_hidden {
            KeypointState::Hidden
        } else {
            KeypointState::Visible
        };
        first.point = Some(point);
        let annotation_id = AnnotationId::generate();
        self.record_edit();
        self.work
            .annotations
            .push(labello_domain::AnnotationVersion {
                annotation_id: annotation_id.clone(),
                version: 1,
                task_id: task.task_id,
                class_id,
                annotation_type: AnnotationType::Skeleton,
                source: AnnotationSource::Human,
                geometry: AnnotationGeometry::Skeleton(SkeletonGeometry { keypoints }),
                author_user_id,
                created_at: timestamp,
                updated_at: timestamp,
                deleted: false,
            });
        self.selected_annotation = Some(annotation_id.clone());
        if keypoint_count > 1 {
            self.active_skeleton = Some(annotation_id);
            self.skeleton_keypoint_index = 1;
        }
        self.next_keypoint_hidden = false;
        self.mark_edited();
    }

    pub(crate) fn skip_keypoint(&mut self) {
        let Some((allow_absent, keypoint_count, required)) = self
            .selected_task()
            .and_then(|task| task.skeleton.as_ref())
            .map(|spec| {
                (
                    spec.allow_absent,
                    spec.keypoints.len(),
                    spec.keypoints
                        .get(self.skeleton_keypoint_index)
                        .is_some_and(|keypoint| keypoint.required),
                )
            })
        else {
            return;
        };
        if !allow_absent || self.active_skeleton.is_none() {
            return;
        }
        if required {
            self.runtime.error =
                Some("This keypoint is required and cannot be marked absent.".to_string());
            return;
        }
        self.record_edit();
        self.skeleton_keypoint_index += 1;
        self.next_keypoint_hidden = false;
        if self.skeleton_keypoint_index >= keypoint_count {
            self.active_skeleton = None;
            self.skeleton_keypoint_index = 0;
        }
        self.mark_edited();
    }

    pub(crate) fn accept_prelabel(&mut self, suggestion: &PrelabelSuggestion) {
        let Some(task) = self.selected_task() else {
            return;
        };
        let Some(class_id) = self.selected_class_id() else {
            return;
        };
        if suggestion.task_id != task.task_id || &suggestion.class_id != class_id {
            return;
        }
        if self
            .accepted_prelabels
            .iter()
            .any(|id| id == &suggestion.suggestion_id)
        {
            return;
        }
        let timestamp = labello_domain::now();
        let user_id = self.config.user_id.clone();
        let annotation_id = AnnotationId::generate();
        self.record_edit();
        self.annotations.push(labello_domain::AnnotationVersion {
            annotation_id: annotation_id.clone(),
            version: 1,
            task_id: suggestion.task_id.clone(),
            class_id: suggestion.class_id.clone(),
            annotation_type: match suggestion.geometry {
                AnnotationGeometry::BoundingBox(_) => AnnotationType::BoundingBox,
                AnnotationGeometry::Skeleton(_) => AnnotationType::Skeleton,
            },
            source: AnnotationSource::PrelabelSuggestion {
                config_id: suggestion.config_id.clone(),
                model_id: "browser-local-or-server".to_string(),
                confidence: suggestion.confidence,
            },
            geometry: suggestion.geometry.clone(),
            author_user_id: user_id,
            created_at: timestamp,
            updated_at: timestamp,
            deleted: false,
        });
        self.accepted_prelabels
            .push(suggestion.suggestion_id.clone());
        self.selected_annotation = Some(annotation_id);
        self.mark_edited();
    }

    pub(crate) fn delete_selected(&mut self) {
        if let Some(selected) = self.selected_annotation.clone() {
            let persisted = self.persisted_annotations.contains(&selected);
            let persisted_version = self
                .current_state
                .as_ref()
                .and_then(|state| state.current_annotation(&selected))
                .map(|annotation| annotation.version);
            if let Some(index) = self
                .annotations
                .iter()
                .position(|annotation| annotation.annotation_id == selected)
            {
                if self.annotations[index].deleted {
                    self.selected_annotation = None;
                    return;
                }
                self.record_edit();
                let annotation = &mut self.annotations[index];
                annotation.deleted = true;
                annotation.updated_at = labello_domain::now();
                if persisted {
                    if let Some(version) = persisted_version {
                        annotation.version = version;
                    }
                    self.modified_annotations.remove(&selected);
                }
                if self.active_skeleton.as_ref() == Some(&selected) {
                    self.active_skeleton = None;
                    self.skeleton_keypoint_index = 0;
                    self.next_keypoint_hidden = false;
                }
                self.selected_annotation = None;
                self.mark_edited();
            } else {
                self.selected_annotation = None;
            }
        }
    }

    fn snapshot(&self) -> EditSnapshot {
        let approx_bytes = serde_json::to_vec(&self.annotations)
            .map(|value| value.len())
            .unwrap_or_else(|_| {
                self.annotations.len() * std::mem::size_of::<labello_domain::AnnotationVersion>()
            })
            + self
                .accepted_prelabels
                .iter()
                .map(|value| value.len())
                .sum::<usize>()
            + 256;
        EditSnapshot {
            annotations: self.annotations.clone(),
            accepted_prelabels: self.accepted_prelabels.clone(),
            selected_annotation: self.selected_annotation.clone(),
            active_skeleton: self.active_skeleton.clone(),
            skeleton_keypoint_index: self.skeleton_keypoint_index,
            next_keypoint_hidden: self.next_keypoint_hidden,
            approx_bytes,
        }
    }

    fn record_edit(&mut self) {
        let snapshot = self.snapshot();
        push_history(&mut self.undo_stack, snapshot);
        self.redo_stack.clear();
    }

    fn restore_snapshot(&mut self, snapshot: EditSnapshot) {
        self.annotations = snapshot.annotations;
        if let Some(state) = self.current_state.as_ref() {
            let persisted_annotations = state.active_annotations().cloned().collect::<Vec<_>>();
            for persisted in persisted_annotations {
                if self
                    .annotations
                    .iter()
                    .all(|annotation| annotation.annotation_id != persisted.annotation_id)
                {
                    let mut deleted = persisted;
                    deleted.deleted = true;
                    deleted.updated_at = labello_domain::now();
                    self.annotations.push(deleted);
                }
            }
        }
        self.accepted_prelabels = snapshot.accepted_prelabels;
        self.selected_annotation = snapshot.selected_annotation;
        self.active_skeleton = snapshot.active_skeleton;
        self.skeleton_keypoint_index = snapshot.skeleton_keypoint_index;
        self.next_keypoint_hidden = snapshot.next_keypoint_hidden;
        self.recompute_modified_annotations();
        self.mark_edited();
    }

    pub(crate) fn undo(&mut self) {
        if let Some(snapshot) = self.undo_stack.pop() {
            let current = self.snapshot();
            push_history(&mut self.redo_stack, current);
            self.restore_snapshot(snapshot);
        }
    }

    pub(crate) fn redo(&mut self) {
        if let Some(snapshot) = self.redo_stack.pop() {
            let current = self.snapshot();
            push_history(&mut self.undo_stack, current);
            self.restore_snapshot(snapshot);
        }
    }

    pub(crate) fn recompute_modified_annotations(&mut self) {
        let persisted_annotations = self.persisted_annotations.clone();
        let current_state = self.current_state.clone();
        self.modified_annotations = self
            .annotations
            .iter()
            .filter(|annotation| {
                persisted_annotations.contains(&annotation.annotation_id)
                    && current_state
                        .as_ref()
                        .and_then(|state| state.current_annotation(&annotation.annotation_id))
                        != Some(annotation)
                    && !annotation.deleted
            })
            .map(|annotation| annotation.annotation_id.clone())
            .collect();
        for annotation in &mut self.annotations {
            if persisted_annotations.contains(&annotation.annotation_id)
                && let Some(persisted) = current_state
                    .as_ref()
                    .and_then(|state| state.current_annotation(&annotation.annotation_id))
            {
                annotation.version = if annotation.deleted {
                    persisted.version
                } else if annotation != persisted {
                    persisted.version + 1
                } else {
                    persisted.version
                };
            }
        }
    }

    fn mark_edited(&mut self) {
        self.edit_generation = self.edit_generation.wrapping_add(1);
        self.save_status = SaveStatus::Dirty;
        self.last_edit_at = Some(Instant::now());
    }

    pub(crate) fn autosave_if_due(&mut self) {
        if self.save_status == SaveStatus::Dirty
            && !self.loading.saving
            && self.pending_transition.is_none()
            && !self.canvas.is_dragging()
            && self
                .last_edit_at
                .is_some_and(|edited| edited.elapsed() >= Duration::from_millis(750))
        {
            self.autosave();
        }
    }

    pub(crate) fn open_shortcut_settings(&mut self) {
        if self.show_settings {
            return;
        }
        let mut draft = self.keybindings.clone();
        draft.normalize();
        self.shortcut_settings.baseline = Some(draft.clone());
        self.shortcut_settings.draft = Some(draft);
        self.shortcut_settings.error = None;
        self.shortcut_settings.recording = None;
        self.shortcut_settings.confirm_discard = false;
        self.drawer = None;
        self.show_tutorial = false;
        self.show_settings = true;
    }

    pub(crate) fn shortcut_text(
        &self,
        ctx: &egui::Context,
        action: labello_domain::UserAction,
    ) -> String {
        self.keybindings
            .bindings
            .get(&action)
            .and_then(keyboard_shortcut)
            .map(|shortcut| ctx.format_shortcut(&shortcut))
            .unwrap_or_default()
    }

    fn cycle_workflow(&mut self, direction: isize) {
        let choices = self.workflow_choices();
        if choices.len() < 2 {
            return;
        }
        let current = choices
            .iter()
            .position(|choice| Some(&choice.task_id) == self.selected_task_id.as_ref())
            .unwrap_or(0);
        let next = (current as isize + direction).rem_euclid(choices.len() as isize) as usize;
        self.request_transition(PendingTransition::Workflow(choices[next].task_id.clone()));
    }

    fn cycle_object(&mut self, direction: isize) {
        let objects = self
            .annotations
            .iter()
            .filter(|annotation| {
                !annotation.deleted && self.annotation_matches_selected_workflow(annotation)
            })
            .map(|annotation| annotation.annotation_id.clone())
            .collect::<Vec<_>>();
        if objects.is_empty() {
            self.selected_annotation = None;
            return;
        }
        let current = self
            .selected_annotation
            .as_ref()
            .and_then(|selected| objects.iter().position(|id| id == selected));
        let next = current.map_or_else(
            || if direction < 0 { objects.len() - 1 } else { 0 },
            |current| (current as isize + direction).rem_euclid(objects.len() as isize) as usize,
        );
        self.selected_annotation = Some(objects[next].clone());
    }

    fn cycle_prelabel(&mut self, direction: isize) {
        let prelabels = self.visible_prelabels();
        if prelabels.is_empty() {
            self.selected_prelabel = None;
            return;
        }
        let current = self.selected_prelabel.as_ref().and_then(|selected| {
            prelabels
                .iter()
                .position(|suggestion| &suggestion.suggestion_id == selected)
        });
        let next = current.map_or_else(
            || {
                if direction < 0 {
                    prelabels.len() - 1
                } else {
                    0
                }
            },
            |current| (current as isize + direction).rem_euclid(prelabels.len() as isize) as usize,
        );
        self.selected_prelabel = Some(prelabels[next].suggestion_id.clone());
    }

    fn active_prelabel(&self) -> Option<labello_domain::PrelabelSuggestion> {
        let prelabels = self.visible_prelabels();
        self.selected_prelabel
            .as_ref()
            .and_then(|selected| {
                prelabels
                    .iter()
                    .find(|suggestion| &suggestion.suggestion_id == selected)
            })
            .cloned()
            .or_else(|| prelabels.into_iter().next())
    }

    pub(crate) fn discard_prelabel(&mut self, suggestion_id: String) {
        if !self.accepted_prelabels.contains(&suggestion_id) {
            self.accepted_prelabels.push(suggestion_id);
        }
        self.selected_prelabel = self
            .visible_prelabels()
            .first()
            .map(|suggestion| suggestion.suggestion_id.clone());
    }

    pub(crate) fn trigger_user_action(&mut self, action: labello_domain::UserAction) {
        use labello_domain::UserAction;
        let ready = (self.assignment.is_some() || self.runtime.api.is_none())
            && !self.loading.saving
            && !self.loading.image
            && self.pending_transition.is_none()
            && !self.canvas.is_dragging();
        let previous_ready = self.view == AppView::Annotate
            && self.previous_annotation_assignment.is_some()
            && self.runtime.api.is_some()
            && !self.loading.saving
            && !self.loading.image
            && self.pending_transition.is_none()
            && !self.canvas.is_dragging();
        match action {
            UserAction::NextImage if self.view == AppView::Annotate && ready => {
                self.submit_and_advance()
            }
            UserAction::PreviousImage if previous_ready => self.return_to_previous_assignment(),
            UserAction::UndoEdit if self.view == AppView::Annotate && ready => self.undo(),
            UserAction::RedoEdit if self.view == AppView::Annotate && ready => self.redo(),
            UserAction::SaveAnnotations if self.view == AppView::Annotate && ready => {
                self.autosave()
            }
            UserAction::SkipAssignment if self.work_view() && ready => self.skip_assignment(),
            UserAction::DeleteAnnotation if self.view == AppView::Annotate && ready => {
                self.delete_selected()
            }
            UserAction::OpenTutorial => {
                self.drawer = None;
                self.show_tutorial = !self.show_tutorial;
            }
            UserAction::ToggleWorkflowPanel => {
                self.show_tutorial = false;
                self.drawer = (self.drawer != Some(Drawer::Workflow)).then_some(Drawer::Workflow)
            }
            UserAction::ToggleInspectorPanel => {
                self.show_tutorial = false;
                self.drawer = (self.drawer != Some(Drawer::Inspector)).then_some(Drawer::Inspector)
            }
            UserAction::OpenSettings => self.open_shortcut_settings(),
            UserAction::SelectPreviousWorkflow if self.view == AppView::Annotate && ready => {
                self.cycle_workflow(-1)
            }
            UserAction::SelectNextWorkflow if self.view == AppView::Annotate && ready => {
                self.cycle_workflow(1)
            }
            UserAction::SelectPreviousObject if self.view == AppView::Annotate && ready => {
                self.cycle_object(-1)
            }
            UserAction::SelectNextObject if self.view == AppView::Annotate && ready => {
                self.cycle_object(1)
            }
            UserAction::SelectPreviousPrelabel if self.view == AppView::Annotate && ready => {
                self.cycle_prelabel(-1)
            }
            UserAction::SelectNextPrelabel if self.view == AppView::Annotate && ready => {
                self.cycle_prelabel(1)
            }
            UserAction::AcceptPrelabel if self.view == AppView::Annotate && ready => {
                if let Some(suggestion) = self.active_prelabel() {
                    self.accept_prelabel(&suggestion);
                    self.selected_prelabel = self
                        .visible_prelabels()
                        .first()
                        .map(|suggestion| suggestion.suggestion_id.clone());
                }
            }
            UserAction::DiscardPrelabel if self.view == AppView::Annotate && ready => {
                if let Some(suggestion) = self.active_prelabel() {
                    self.discard_prelabel(suggestion.suggestion_id);
                }
            }
            UserAction::ToggleKeypointHidden if self.view == AppView::Annotate && ready => {
                if self
                    .selected_task()
                    .and_then(|task| task.skeleton.as_ref())
                    .is_some_and(|spec| spec.allow_hidden)
                {
                    self.next_keypoint_hidden = !self.next_keypoint_hidden;
                }
            }
            UserAction::MarkKeypointAbsent if self.view == AppView::Annotate && ready => {
                if self.active_skeleton.is_some()
                    && self
                        .selected_task()
                        .and_then(|task| task.skeleton.as_ref())
                        .is_some_and(|spec| spec.allow_absent)
                {
                    self.skip_keypoint();
                }
            }
            UserAction::RetryImageLoad
                if self.view == AppView::Annotate
                    && self.current.is_none()
                    && !self.loading.image =>
            {
                self.retry_assignment_load()
            }
            UserAction::TogglePanMode if self.work_view() && self.current.is_some() => {
                self.canvas.toggle_pan_mode()
            }
            UserAction::ZoomIn if self.work_view() && self.current.is_some() => {
                self.canvas.zoom_in()
            }
            UserAction::ZoomOut if self.work_view() && self.current.is_some() => {
                self.canvas.zoom_out()
            }
            UserAction::FitImage if self.work_view() && self.current.is_some() => {
                self.canvas.fit_view()
            }
            UserAction::AcceptReviewObject if self.view == AppView::Review => {
                if self.correction_draft.is_none() {
                    self.request_review(labello_domain::ReviewDecision::Approved);
                }
            }
            UserAction::RejectReviewObject if self.view == AppView::Review => {
                if self.correction_draft.is_none() {
                    self.request_review(labello_domain::ReviewDecision::Rejected);
                }
            }
            UserAction::SelectBoundingBoxTool
            | UserAction::SelectKeypointTool
            | UserAction::ToggleOfflineMode => {}
            _ => {}
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if !self.work_view()
            || ctx.text_edit_focused()
            || self.loading.saving
            || self.loading.image
            || self.pending_transition.is_some()
            || self.show_settings
            || self.drawer.is_some()
            || self.runtime.persistence.recovery.is_some()
            || egui::Popup::is_any_open(ctx)
        {
            return;
        }
        if self.canvas.pan_mode() && ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.canvas.exit_pan_mode();
        }
        if self.view == AppView::Review
            && self.correction_draft.is_some()
            && ctx.input_mut(|input| {
                input.consume_shortcut(&egui::KeyboardShortcut::new(
                    egui::Modifiers::COMMAND,
                    egui::Key::Z,
                )) || input.consume_shortcut(&egui::KeyboardShortcut::new(
                    egui::Modifiers::CTRL,
                    egui::Key::Z,
                ))
            })
        {
            self.undo_correction();
        }
        let mut bindings = self
            .keybindings
            .bindings
            .iter()
            .filter(|(action, _)| match action.context() {
                labello_domain::ActionContext::WorkWorkspace => self.work_view(),
                labello_domain::ActionContext::WorkImage => {
                    self.work_view() && self.current.is_some()
                }
                labello_domain::ActionContext::AnnotateWorkspace => self.view == AppView::Annotate,
                labello_domain::ActionContext::AnnotateImage => {
                    self.view == AppView::Annotate && self.current.is_some()
                }
                labello_domain::ActionContext::AnnotateNoImage => {
                    self.view == AppView::Annotate && self.current.is_none()
                }
                labello_domain::ActionContext::Review => self.view == AppView::Review,
                labello_domain::ActionContext::Legacy => false,
            })
            .map(|(action, chord)| (*action, chord.clone()))
            .collect::<Vec<_>>();
        bindings.sort_by_key(|(_, chord)| {
            std::cmp::Reverse(
                chord.shift as u8 + chord.alt as u8 + (chord.ctrl || chord.command) as u8,
            )
        });
        for (action, chord) in bindings {
            if consume_keyboard_shortcut(ctx, &chord) {
                self.trigger_user_action(action);
            }
        }
    }
}

fn push_history(stack: &mut Vec<EditSnapshot>, snapshot: EditSnapshot) {
    stack.push(snapshot);
    let mut bytes = stack.iter().map(|entry| entry.approx_bytes).sum::<usize>();
    while stack.len() > MAX_HISTORY_OPERATIONS || bytes > MAX_HISTORY_BYTES {
        let removed = stack.remove(0);
        bytes = bytes.saturating_sub(removed.approx_bytes);
    }
}

fn keyboard_shortcut(chord: &labello_domain::KeyChord) -> Option<egui::KeyboardShortcut> {
    let key = parse_key(&chord.key)?;
    let mut modifiers = egui::Modifiers::NONE;
    modifiers.command = chord.ctrl || chord.command;
    modifiers.shift = chord.shift;
    modifiers.alt = chord.alt;
    Some(egui::KeyboardShortcut::new(modifiers, key))
}

fn consume_keyboard_shortcut(ctx: &egui::Context, chord: &labello_domain::KeyChord) -> bool {
    let Some(shortcut) = keyboard_shortcut(chord) else {
        return false;
    };
    if ctx.input_mut(|input| input.consume_shortcut(&shortcut)) {
        return true;
    }
    if chord.ctrl || chord.command {
        let mut ctrl_shortcut = shortcut;
        ctrl_shortcut.modifiers.command = false;
        ctrl_shortcut.modifiers.ctrl = true;
        return ctx.input_mut(|input| input.consume_shortcut(&ctrl_shortcut));
    }
    false
}

pub(crate) fn parse_key(key: &str) -> Option<egui::Key> {
    egui::Key::from_name(key)
}

pub(crate) fn tool_for_annotation_type(annotation_type: &AnnotationType) -> Tool {
    match annotation_type {
        AnnotationType::BoundingBox => Tool::BoundingBox,
        AnnotationType::Skeleton => Tool::Keypoints,
    }
}

fn valid_workflow(task: &TaskDefinition) -> bool {
    task.enabled && task.class_ids.len() == 1
}

pub(crate) fn annotation_type_label(annotation_type: &AnnotationType) -> &'static str {
    match annotation_type {
        AnnotationType::BoundingBox => "bounding box",
        AnnotationType::Skeleton => "skeleton",
    }
}

impl eframe::App for LabelloApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if !self.theme_applied {
            self.theme_applied = theme::apply_fallback(ui.ctx());
            if !self.theme_applied {
                return;
            }
        }
        self.process_messages(ui.ctx());
        self.retry_prefetch_if_due(ui.ctx());
        if self.queue.remove_expired() {
            self.request_prefetch();
        }
        self.sync_review_selection();
        self.start_next_persistence_command();
        self.start_setup_load();
        self.refresh_stats_if_due();
        self.refresh_ingest_if_due();
        self.autosave_if_due();
        self.handle_shortcuts(ui.ctx());
        let viewport = ui.available_size();
        let layout = LayoutMode::for_width(ui.available_width());
        egui::Panel::top("app_bar")
            .exact_size(56.0)
            .frame(theme::top_bar_frame().inner_margin(egui::Margin::symmetric(14, 6)))
            .show(ui, |ui| self.app_bar(ui, layout));
        if self.work_view() {
            egui::Panel::top("workspace_context")
                .exact_size(self.workspace_context_height(layout, viewport))
                .frame(
                    theme::top_bar_frame()
                        .fill(theme::PANEL)
                        .inner_margin(egui::Margin::symmetric(14, 6)),
                )
                .show(ui, |ui| self.workspace_context_bar(ui, layout));
        }
        if self.work_view() && layout != LayoutMode::Wide {
            egui::Panel::bottom("compact_primary_actions")
                .exact_size(self.workspace_actions_height(layout, viewport))
                .frame(theme::top_bar_frame())
                .show(ui, |ui| {
                    if layout == LayoutMode::Compact {
                        self.compact_workspace_actions(ui);
                    } else {
                        ui.horizontal_wrapped(|ui| self.workspace_actions(ui, layout));
                    }
                });
        } else if self.view == AppView::Admin {
            let changes_dirty = self.admin_changes_dirty();
            egui::Panel::bottom("admin_save_status")
                .exact_size(if changes_dirty {
                    self.admin_status_height(layout)
                } else {
                    0.0
                })
                .frame(if changes_dirty {
                    theme::top_bar_frame()
                } else {
                    egui::Frame::NONE
                })
                .show(ui, |ui| {
                    if changes_dirty {
                        self.admin_status_bar(ui);
                    }
                });
        }
        if self.work_view() && layout == LayoutMode::Wide {
            egui::Panel::left("task_panel")
                .resizable(false)
                .exact_size(LayoutMode::TASK_PANEL_WIDTH)
                .frame(theme::side_frame())
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.task_panel(ui));
                });
            egui::Panel::right("review_panel")
                .resizable(false)
                .exact_size(LayoutMode::INSPECTOR_PANEL_WIDTH)
                .frame(theme::side_frame())
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.right_panel(ui, true));
                });
        } else if !self.work_view() && layout == LayoutMode::Wide {
            egui::Panel::left("desktop_navigation")
                .resizable(false)
                .exact_size(176.0)
                .frame(theme::side_frame())
                .show(ui, |ui| self.desktop_navigation(ui));
        }
        let central_frame = if self.work_view() {
            theme::central_frame().inner_margin(egui::Margin::symmetric(8, 8))
        } else {
            theme::central_frame()
        };
        egui::CentralPanel::default()
            .frame(central_frame)
            .show(ui, |ui| self.central(ui, layout));
        self.overlays(ui.ctx(), layout);
        self.queue_current_drafts();
        self.persist_workspace_preference();
        self.start_next_command();
        if self.save_status == SaveStatus::Dirty
            && let Some(edited) = self.last_edit_at
        {
            ui.ctx().request_repaint_after(
                std::time::Duration::from_millis(750).saturating_sub(edited.elapsed()),
            );
        }
        if self.loading.ingesting {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(500));
        }
        if self.view == AppView::Stats && !self.loading.stats {
            let until_refresh = self
                .datasets
                .last_stats_attempt
                .map(|attempt| std::time::Duration::from_secs(3).saturating_sub(attempt.elapsed()))
                .unwrap_or(std::time::Duration::from_secs(3));
            ui.ctx().request_repaint_after(until_refresh);
        }
    }
}

fn demo_image(index: usize) -> QueuedImage {
    let image = ImageRecord {
        image_id: ImageId::from(format!("img_demo_{index}")),
        blake3: format!("demo_hash_{index}"),
        canonical_path: format!("images/demo_{index}.jpg"),
        known_paths: vec![format!("images/demo_{index}.jpg")],
        duplicate_paths: vec![],
        file_name: format!("demo_{index}.jpg"),
        byte_size: 1024,
        width: 1280,
        height: 800,
        media_type: "image/jpeg".to_string(),
    };
    let prelabels = vec![PrelabelSuggestion {
        suggestion_id: format!("pre_demo_{index}"),
        config_id: labello_domain::PrelabelConfigId::from("demo-prelabel"),
        task_id: TaskId::from("bounding_box:person"),
        class_id: ClassId::from("person"),
        confidence: 0.82,
        geometry: AnnotationGeometry::BoundingBox(BoundingBox {
            x: 0.32,
            y: 0.22,
            width: 0.2,
            height: 0.46,
        }),
    }];
    QueuedImage { image, prelabels }
}

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
        assert!(app.undo_stack.len() <= MAX_HISTORY_OPERATIONS);

        app.undo_stack.clear();
        for _ in 0..20 {
            app.accepted_prelabels = vec!["x".repeat(2 * 1024 * 1024)];
            app.record_edit();
        }
        assert!(
            app.undo_stack
                .iter()
                .map(|snapshot| snapshot.approx_bytes)
                .sum::<usize>()
                <= MAX_HISTORY_BYTES
        );
        assert!(app.undo_stack.len() < 20);
    }
}
