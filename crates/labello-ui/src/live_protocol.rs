use std::collections::BTreeSet;

use eframe::egui;
use labello_client::{
    AuthOptions, CancelImportResult, CommitImportResult, CorrectionRequest, CreateImportRequest,
    DatasetSummary, DatasetUser, ImageExplorerQuery, ImportCapabilities, ImportJob, ImportPlan,
    IngestJob, SessionInfo, SnapshotFile, UpdateImportPlanRequest,
};
use labello_domain::{
    AdjudicationRecord, AnnotationId, Assignment, AssignmentId, AssignmentKind, DatasetId,
    DatasetMetadata, DatasetRole, DatasetSnapshot, DatasetStats, ImageExplorerPage, ImageId,
    ImageState, KeybindingSet, PrelabelConfigId, ReviewRecord, TaskId, UserId,
};

use crate::queue::QueuedImage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReviewPhase {
    Object,
    FullImage,
}

#[derive(Debug)]
pub(crate) enum MigrationAction {
    SaveSkeleton(labello_client::SaveMigrationSkeletonRequest),
    Exclude(labello_client::ExcludeMigrationTargetRequest),
    Reopen(labello_client::ReopenMigrationTargetRequest),
    StartPass(labello_client::StartMigrationPassRequest),
    Keep(labello_client::KeepMigrationTargetRequest),
    Confirm(labello_client::ConfirmMigrationRequest),
    Review(labello_client::ReviewMigrationRequest),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestIdentity {
    pub auth_epoch: u64,
    pub workspace_epoch: u64,
    pub request_id: u64,
    pub dataset_id: Option<DatasetId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImportRequestIdentity {
    pub auth_epoch: u64,
    pub import_epoch: u64,
    pub request_id: u64,
    pub import_id: Option<labello_domain::ImportId>,
}

#[derive(Debug)]
pub(crate) enum UiMessage {
    ImportCapabilitiesLoaded {
        request: ImportRequestIdentity,
        result: Result<ImportCapabilities, String>,
    },
    ImportJobLoaded {
        request: ImportRequestIdentity,
        result: Box<Result<ImportJob, String>>,
    },
    #[cfg(target_arch = "wasm32")]
    ImportBrowserFilesSelected {
        request: ImportRequestIdentity,
        result: Result<Vec<crate::import_flow::BrowserImportFile>, String>,
    },
    ImportFilesRegistered {
        request: ImportRequestIdentity,
        result: Result<labello_client::RegisterImportFilesResult, String>,
    },
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    ImportChunkUploaded {
        request: ImportRequestIdentity,
        file_id: String,
        result: Result<crate::import_flow::RawImportChunkResponse, String>,
    },
    ImportSealed {
        request: ImportRequestIdentity,
        result: Result<labello_client::SealImportResult, String>,
    },
    ImportPlanUpdated {
        request: ImportRequestIdentity,
        result: Box<Result<ImportPlan, String>>,
    },
    ImportDiagnosticsLoaded {
        request: ImportRequestIdentity,
        result: Result<labello_client::ImportDiagnosticsPage, String>,
    },
    ImportCommitted {
        request: ImportRequestIdentity,
        result: Result<CommitImportResult, String>,
    },
    ImportCancelled {
        request: ImportRequestIdentity,
        result: Result<CancelImportResult, String>,
    },
    MigrationFinished {
        request: RequestIdentity,
        result: Box<Result<labello_client::ManualMigrationCommandResult, String>>,
    },
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
    ImportCapabilities {
        request: ImportRequestIdentity,
    },
    CreateImport {
        request: ImportRequestIdentity,
        body: CreateImportRequest,
        idempotency_key: String,
    },
    GetImport {
        request: ImportRequestIdentity,
        import_id: labello_domain::ImportId,
    },
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    RegisterImportFiles {
        request: ImportRequestIdentity,
        import_id: labello_domain::ImportId,
        body: labello_client::RegisterImportFilesRequest,
        idempotency_key: String,
    },
    SealImport {
        request: ImportRequestIdentity,
        import_id: labello_domain::ImportId,
        body: labello_client::SealImportRequest,
        idempotency_key: String,
    },
    PreflightImport {
        request: ImportRequestIdentity,
        import_id: labello_domain::ImportId,
        body: labello_client::StartImportPreflightRequest,
        idempotency_key: String,
    },
    UpdateImportPlan {
        request: ImportRequestIdentity,
        import_id: labello_domain::ImportId,
        body: UpdateImportPlanRequest,
        idempotency_key: String,
    },
    ImportDiagnostics {
        request: ImportRequestIdentity,
        import_id: labello_domain::ImportId,
        query: labello_client::ImportDiagnosticsQuery,
    },
    CommitImport {
        request: ImportRequestIdentity,
        import_id: labello_domain::ImportId,
        body: labello_client::CommitImportRequest,
        idempotency_key: String,
    },
    CancelImport {
        request: ImportRequestIdentity,
        import_id: labello_domain::ImportId,
        idempotency_key: String,
    },
    Migration {
        request: RequestIdentity,
        dataset_id: DatasetId,
        image_id: ImageId,
        action: MigrationAction,
        idempotency_key: String,
    },
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
            Self::ImportCapabilities { .. }
            | Self::CreateImport { .. }
            | Self::GetImport { .. }
            | Self::RegisterImportFiles { .. }
            | Self::SealImport { .. }
            | Self::PreflightImport { .. }
            | Self::UpdateImportPlan { .. }
            | Self::ImportDiagnostics { .. }
            | Self::CommitImport { .. }
            | Self::CancelImport { .. } => panic!("import commands use import_request"),
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
            | Self::Adjudication { request, .. }
            | Self::Migration { request, .. } => request,
        }
    }

    pub(crate) fn import_request(&self) -> Option<&ImportRequestIdentity> {
        match self {
            Self::ImportCapabilities { request }
            | Self::CreateImport { request, .. }
            | Self::GetImport { request, .. }
            | Self::RegisterImportFiles { request, .. }
            | Self::SealImport { request, .. }
            | Self::PreflightImport { request, .. }
            | Self::UpdateImportPlan { request, .. }
            | Self::ImportDiagnostics { request, .. }
            | Self::CommitImport { request, .. }
            | Self::CancelImport { request, .. } => Some(request),
            _ => None,
        }
    }
}

impl UiMessage {
    pub(crate) fn request(&self) -> Option<&RequestIdentity> {
        match self {
            Self::ImportCapabilitiesLoaded { .. }
            | Self::ImportJobLoaded { .. }
            | Self::ImportFilesRegistered { .. }
            | Self::ImportChunkUploaded { .. }
            | Self::ImportSealed { .. }
            | Self::ImportPlanUpdated { .. }
            | Self::ImportDiagnosticsLoaded { .. }
            | Self::ImportCommitted { .. }
            | Self::ImportCancelled { .. } => None,
            #[cfg(target_arch = "wasm32")]
            Self::ImportBrowserFilesSelected { .. } => None,
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
            | Self::MigrationFinished { request, .. }
            | Self::RequestFailed { request, .. } => Some(request),
            Self::PersistenceFinished(_)
            | Self::FolderUploadProgress { .. }
            | Self::FolderUploadFinished { .. } => None,
        }
    }

    pub(crate) fn import_request(&self) -> Option<&ImportRequestIdentity> {
        match self {
            Self::ImportCapabilitiesLoaded { request, .. }
            | Self::ImportJobLoaded { request, .. }
            | Self::ImportFilesRegistered { request, .. }
            | Self::ImportChunkUploaded { request, .. }
            | Self::ImportSealed { request, .. }
            | Self::ImportPlanUpdated { request, .. }
            | Self::ImportDiagnosticsLoaded { request, .. }
            | Self::ImportCommitted { request, .. }
            | Self::ImportCancelled { request, .. } => Some(request),
            #[cfg(target_arch = "wasm32")]
            Self::ImportBrowserFilesSelected { request, .. } => Some(request),
            _ => None,
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
