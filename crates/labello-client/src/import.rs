use std::fmt;

use labello_domain::{
    AnnotationId, Assignment, AssignmentId, ClassId, DatasetId, ImageState, ImportId,
    KeypointState, MigrationConfirmation, MigrationCursor, MigrationExclusionReason, MigrationHash,
    MigrationPass, MigrationPassId, ObjectGroupId, ReviewDecision, SkeletonGeometry, SkeletonSpec,
    TaskDefinition, TaskId, Timestamp, UserId,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportProfile {
    UltralyticsYoloDetectV1,
    UltralyticsYoloPoseV1,
    CocoInstancesGtV1,
    CocoKeypointsGtV1,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportTransport {
    BrowserFolder,
    ServerDirectory,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportCapabilities {
    pub available: bool,
    #[serde(default)]
    pub unavailable_reason: Option<String>,
    #[serde(default)]
    pub profiles: Vec<ImportProfileCapability>,
    #[serde(default)]
    pub transports: Vec<ImportTransportCapability>,
    #[serde(default)]
    pub server_roots: Vec<ServerImportRoot>,
    #[serde(default)]
    pub limits: ImportLimits,
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub parser_version: String,
    #[serde(default)]
    pub tool_version: String,
    #[serde(default)]
    pub manual_box_guide_migration: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportProfileCapability {
    pub profile: ImportProfile,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub profile_version: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportTransportCapability {
    pub transport: ImportTransport,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub resumable: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerImportRoot {
    pub root_id: String,
    pub display_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportLimits {
    pub max_browser_files: u64,
    pub max_browser_bytes: u64,
    pub max_server_files: u64,
    pub max_source_bytes: u64,
    pub max_selected_images: u64,
    pub max_single_file_bytes: u64,
    pub upload_chunk_bytes: u64,
    pub max_selected_categories: u32,
    pub max_generated_tasks: u32,
    pub max_annotations: u64,
    pub max_annotations_per_image: u32,
    pub max_keypoints_per_skeleton: u32,
    pub max_diagnostic_page_size: u32,
}

impl Default for ImportLimits {
    fn default() -> Self {
        Self {
            max_browser_files: 25_000,
            max_browser_bytes: 20 * 1024 * 1024 * 1024,
            max_server_files: 50_000,
            max_source_bytes: 100 * 1024 * 1024 * 1024,
            max_selected_images: 10_000,
            max_single_file_bytes: 4 * 1024 * 1024 * 1024,
            upload_chunk_bytes: 8 * 1024 * 1024,
            max_selected_categories: 100,
            max_generated_tasks: 200,
            max_annotations: 1_000_000,
            max_annotations_per_image: 10_000,
            max_keypoints_per_skeleton: 512,
            max_diagnostic_page_size: 100,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "transport",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ImportSourceSelection {
    BrowserFolder,
    ServerDirectory {
        import_root_id: String,
        relative_path: String,
    },
}

impl fmt::Debug for ImportSourceSelection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BrowserFolder => formatter.write_str("BrowserFolder"),
            Self::ServerDirectory { import_root_id, .. } => formatter
                .debug_struct("ServerDirectory")
                .field("import_root_id", import_root_id)
                .field("relative_path", &"<redacted>")
                .finish(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportAttestations {
    pub ground_truth: bool,
    pub exhaustive: bool,
    pub coverage_scope: Vec<String>,
    pub provenance: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateImportRequest {
    pub destination_dataset_id: DatasetId,
    pub destination_name: String,
    pub profile: ImportProfile,
    pub source: ImportSourceSelection,
    pub attestations: ImportAttestations,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportLifecycle {
    Registering,
    Uploading,
    Sealed,
    Preflighting,
    AwaitingDecision,
    Building,
    Verifying,
    Committing,
    Succeeded,
    Failed,
    Cancelled,
    Expired,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportProgressPhase {
    Registration,
    Upload,
    Sealing,
    Preflight,
    Build,
    Verification,
    Commit,
    Cleanup,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportProgress {
    pub phase: ImportProgressPhase,
    #[serde(default)]
    pub registered_files: u64,
    #[serde(default)]
    pub uploaded_files: u64,
    #[serde(default)]
    pub total_files: u64,
    #[serde(default)]
    pub accepted_bytes: u64,
    #[serde(default)]
    pub total_bytes: u64,
    #[serde(default)]
    pub processed_images: u64,
    #[serde(default)]
    pub total_images: u64,
    #[serde(default)]
    pub processed_objects: u64,
    #[serde(default)]
    pub total_objects: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportFailure {
    pub code: String,
    pub phase: ImportProgressPhase,
    pub safe_summary: String,
    #[serde(default)]
    pub retryable: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportJob {
    pub import_id: ImportId,
    pub owner_user_id: UserId,
    pub destination_dataset_id: DatasetId,
    pub destination_name: String,
    pub profile: ImportProfile,
    pub transport: ImportTransport,
    pub lifecycle: ImportLifecycle,
    #[serde(default)]
    pub progress: ImportProgress,
    #[serde(default)]
    pub failure: Option<ImportFailure>,
    #[serde(default)]
    pub source_fingerprint: Option<String>,
    #[serde(default)]
    pub plan_hash: Option<String>,
    #[serde(default)]
    pub preflight_report: Option<ImportPreflightReport>,
    #[serde(default)]
    pub can_cancel: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    #[serde(default)]
    pub expires_at: Option<Timestamp>,
    #[serde(default)]
    pub recovery: Option<ImportRecoveryState>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRecoveryState {
    pub attestations: ImportAttestations,
    #[serde(default)]
    pub server_root_id: Option<String>,
    #[serde(default)]
    pub source: Option<ImportSourceConfiguration>,
    #[serde(default)]
    pub registered_files: Vec<RegisteredImportFile>,
    #[serde(default)]
    pub accepted_plan: Option<ImportPlan>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportFileRegistration {
    pub client_file_id: String,
    pub relative_path: String,
    pub byte_size: u64,
    #[serde(default)]
    pub blake3: Option<String>,
}

impl fmt::Debug for ImportFileRegistration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportFileRegistration")
            .field("client_file_id", &self.client_file_id)
            .field("relative_path", &"<redacted>")
            .field("byte_size", &self.byte_size)
            .field("blake3", &self.blake3.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegisterImportFilesRequest {
    pub files: Vec<ImportFileRegistration>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredImportFile {
    pub client_file_id: String,
    pub file_id: String,
    pub byte_size: u64,
    #[serde(default)]
    pub accepted_bytes: u64,
    #[serde(default)]
    pub complete: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterImportFilesResult {
    #[serde(default)]
    pub files: Vec<RegisteredImportFile>,
    #[serde(default)]
    pub registered_files: u64,
    #[serde(default)]
    pub registered_bytes: u64,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ImportChunkUpload {
    pub offset: u64,
    pub length: u64,
    pub digest: String,
    pub bytes: Vec<u8>,
}

impl fmt::Debug for ImportChunkUpload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportChunkUpload")
            .field("offset", &self.offset)
            .field("length", &self.length)
            .field("digest", &"<redacted>")
            .field(
                "bytes",
                &format_args!("<redacted: {} bytes>", self.bytes.len()),
            )
            .finish()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportChunkResult {
    pub file_id: String,
    #[serde(default)]
    pub accepted_offset: u64,
    #[serde(default)]
    pub complete: bool,
    #[serde(default)]
    pub file_blake3: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportDescriptorKind {
    YoloDataset,
    CocoInstances,
    CocoKeypoints,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowseServerImportRootRequest {
    #[serde(default)]
    pub relative_path: String,
    #[serde(default)]
    pub offset: u32,
}

impl fmt::Debug for BrowseServerImportRootRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BrowseServerImportRootRequest")
            .field("relative_path", &"<redacted>")
            .field("offset", &self.offset)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportSourceBrowseMode {
    Descriptors,
    Images,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowseImportSourceRequest {
    #[serde(default)]
    pub relative_path: String,
    #[serde(default)]
    pub offset: u32,
    pub mode: ImportSourceBrowseMode,
}

impl fmt::Debug for BrowseImportSourceRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BrowseImportSourceRequest")
            .field("relative_path", &"<redacted>")
            .field("offset", &self.offset)
            .field("mode", &self.mode)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportBrowseEntryKind {
    Directory,
    File,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportBrowseEntry {
    pub name: String,
    pub relative_path: String,
    pub kind: ImportBrowseEntryKind,
    #[serde(default)]
    pub file_id: Option<String>,
}

impl fmt::Debug for ImportBrowseEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportBrowseEntry")
            .field("name", &"<redacted>")
            .field("relative_path", &"<redacted>")
            .field("kind", &self.kind)
            .field("file_id", &self.file_id.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportBrowsePage {
    #[serde(default)]
    pub relative_path: String,
    #[serde(default)]
    pub entries: Vec<ImportBrowseEntry>,
    #[serde(default)]
    pub next_offset: Option<u32>,
}

impl fmt::Debug for ImportBrowsePage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportBrowsePage")
            .field("relative_path", &"<redacted>")
            .field("entry_count", &self.entries.len())
            .field("next_offset", &self.next_offset)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InspectYoloDescriptorRequest {
    pub descriptor_file_id: String,
}

impl fmt::Debug for InspectYoloDescriptorRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InspectYoloDescriptorRequest")
            .field("descriptor_file_id", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YoloDescriptorInspection {
    #[serde(default)]
    pub splits: Vec<YoloSplitInspection>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YoloSplitInspection {
    pub name: String,
    pub usable: bool,
    #[serde(default)]
    pub issue: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportDescriptorSelection {
    pub descriptor_file_id: String,
    pub kind: ImportDescriptorKind,
    pub release: String,
    pub split: String,
    #[serde(default)]
    pub image_root_file_id: Option<String>,
    #[serde(default)]
    pub pairing_group: Option<String>,
}

impl fmt::Debug for ImportDescriptorSelection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportDescriptorSelection")
            .field("descriptor_file_id", &"<redacted>")
            .field("kind", &self.kind)
            .field("release", &self.release)
            .field("split", &self.split)
            .field(
                "image_root_file_id",
                &self.image_root_file_id.as_ref().map(|_| "<redacted>"),
            )
            .field("pairing_group", &self.pairing_group)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportSourceConfiguration {
    pub source_namespace: String,
    pub descriptors: Vec<ImportDescriptorSelection>,
    pub selected_splits: Vec<String>,
    #[serde(default)]
    pub selected_category_keys: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SealImportRequest {
    pub source: ImportSourceConfiguration,
    pub attestations: ImportAttestations,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SealImportResult {
    pub import_id: ImportId,
    pub source_fingerprint: String,
    #[serde(default)]
    pub files: u64,
    #[serde(default)]
    pub bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartImportPreflightRequest {
    #[serde(default)]
    pub restart: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSourceCounts {
    #[serde(default)]
    pub files: u64,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub descriptors: u64,
    #[serde(default)]
    pub splits: u64,
    #[serde(default)]
    pub images: u64,
    #[serde(default)]
    pub categories: u64,
    #[serde(default)]
    pub objects: u64,
    #[serde(default)]
    pub keypoints: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportGeometryCounts {
    #[serde(default)]
    pub direct: u64,
    #[serde(default)]
    pub clipped: u64,
    #[serde(default)]
    pub skipped: u64,
    #[serde(default)]
    pub template_derived: u64,
    #[serde(default)]
    pub envelope_derived: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportCoverageCounts {
    #[serde(default)]
    pub complete: u64,
    #[serde(default)]
    pub verified_empty: u64,
    #[serde(default)]
    pub incomplete: u64,
    #[serde(default)]
    pub excluded: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportCoverageByGeometry {
    #[serde(default)]
    pub bounding_boxes: ImportCoverageCounts,
    #[serde(default)]
    pub skeletons: ImportCoverageCounts,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOutputEstimate {
    #[serde(default)]
    pub classes: u64,
    #[serde(default)]
    pub tasks: u64,
    #[serde(default)]
    pub annotations: u64,
    #[serde(default)]
    pub events: u64,
    #[serde(default)]
    pub output_bytes: u64,
    #[serde(default)]
    pub temporary_bytes: u64,
    #[serde(default)]
    pub required_free_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportDiagnosticSeverity {
    Error,
    WarningRequiresAck,
    Warning,
    Info,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDiagnosticImpact {
    #[serde(default)]
    pub blocks_commit: bool,
    #[serde(default)]
    pub requires_acknowledgement: bool,
    #[serde(default)]
    pub changes_coverage: bool,
    #[serde(default)]
    pub discards_metadata: bool,
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDiagnosticSourceReference {
    #[serde(default)]
    pub relative_path: Option<String>,
    #[serde(default)]
    pub source_image_id: Option<String>,
    #[serde(default)]
    pub category_id: Option<String>,
    #[serde(default)]
    pub annotation_id: Option<String>,
    #[serde(default)]
    pub line: Option<u64>,
}

impl fmt::Debug for ImportDiagnosticSourceReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportDiagnosticSourceReference")
            .field("authorized_source_reference", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDiagnosticExample {
    #[serde(default)]
    pub source: Option<ImportDiagnosticSourceReference>,
    #[serde(default)]
    pub safe_summary: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDiagnosticSummary {
    pub code: String,
    pub severity: ImportDiagnosticSeverity,
    pub source_profile: ImportProfile,
    #[serde(default)]
    pub count: u64,
    #[serde(default)]
    pub safe_summary: String,
    #[serde(default)]
    pub impact: ImportDiagnosticImpact,
    #[serde(default)]
    pub examples: Vec<ImportDiagnosticExample>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreflightReport {
    #[serde(default)]
    pub source_fingerprint: String,
    #[serde(default)]
    pub plan_hash: Option<String>,
    #[serde(default)]
    pub source: ImportSourceCounts,
    #[serde(default)]
    pub geometry: ImportGeometryCounts,
    #[serde(default)]
    pub coverage: ImportCoverageCounts,
    #[serde(default)]
    pub coverage_by_geometry: ImportCoverageByGeometry,
    #[serde(default)]
    pub output: ImportOutputEstimate,
    #[serde(default)]
    pub diagnostics: Vec<ImportDiagnosticSummary>,
    #[serde(default)]
    pub blocking_diagnostics: u64,
    #[serde(default)]
    pub required_acknowledgements: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportDiagnosticsQuery {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default = "default_diagnostic_page_size")]
    pub limit: u32,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub severity: Option<ImportDiagnosticSeverity>,
}

impl Default for ImportDiagnosticsQuery {
    fn default() -> Self {
        Self {
            cursor: None,
            limit: default_diagnostic_page_size(),
            code: None,
            severity: None,
        }
    }
}

fn default_diagnostic_page_size() -> u32 {
    100
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDiagnostic {
    pub diagnostic_id: String,
    pub code: String,
    pub severity: ImportDiagnosticSeverity,
    pub source_profile: ImportProfile,
    #[serde(default)]
    pub safe_summary: String,
    #[serde(default)]
    pub impact: ImportDiagnosticImpact,
    #[serde(default)]
    pub source: Option<ImportDiagnosticSourceReference>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDiagnosticsPage {
    #[serde(default)]
    pub diagnostics: Vec<ImportDiagnostic>,
    #[serde(default)]
    pub next_cursor: Option<String>,
    #[serde(default)]
    pub total: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportGeometryKind {
    BoundingBox,
    Skeleton,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportGeometryPolicy {
    Direct,
    KeypointEnvelopeV1,
    ManualBoxGuideV1,
    BoxRelativeTemplateV1,
    Omit,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportCategoryMappingRequest {
    pub source_category_key: String,
    pub source_category_id: String,
    pub class_id: ClassId,
    pub class_name: String,
    pub color: String,
    pub selected: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportGeometryMappingRequest {
    pub source_category_key: String,
    pub source_geometry: ImportGeometryKind,
    pub target_geometry: ImportGeometryKind,
    pub policy: ImportGeometryPolicy,
    #[serde(default)]
    pub parameters: Vec<ImportMappingParameter>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum ImportMappingParameter {
    Scalar {
        name: String,
        value: f64,
    },
    Boolean {
        name: String,
        value: bool,
    },
    Point {
        name: String,
        x: f64,
        y: f64,
        state: KeypointState,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportWorkflowIntent {
    AuthoritativeGroundTruth,
    RequireApproval,
    SeedFutureAnnotation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportTaskMappingRequest {
    pub source_category_key: String,
    pub task: TaskDefinition,
    pub workflow_intent: ImportWorkflowIntent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportSkeletonMappingRequest {
    pub source_category_key: String,
    pub target_task_id: TaskId,
    pub skeleton: SkeletonSpec,
    pub source_keypoint_names: Vec<String>,
    pub names_confirmed: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum YoloMissingLabelPolicy {
    #[default]
    Block,
    Incomplete,
    MissingIsBackground,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum YoloDuplicateRowPolicy {
    #[default]
    Block,
    Deduplicate,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CocoCrowdPolicy {
    #[default]
    Block,
    Incomplete,
    ExcludeImageTask,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CocoStructurePolicy {
    #[default]
    Canonical,
    BboxCompatibility,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeometryBoundsPolicy {
    #[default]
    Reject,
    Clip,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrossSplitDuplicatePolicy {
    #[default]
    Block,
    MergeMemberships,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingKeypointNamesPolicy {
    #[default]
    Block,
    GenerateIndexed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportCompatibilityPolicies {
    #[serde(default)]
    pub yolo_missing_labels: YoloMissingLabelPolicy,
    #[serde(default)]
    pub yolo_duplicate_rows: YoloDuplicateRowPolicy,
    #[serde(default)]
    pub coco_crowds: CocoCrowdPolicy,
    #[serde(default)]
    pub coco_structure: CocoStructurePolicy,
    #[serde(default)]
    pub geometry_bounds: GeometryBoundsPolicy,
    #[serde(default)]
    pub cross_split_duplicates: CrossSplitDuplicatePolicy,
    #[serde(default)]
    pub missing_keypoint_names: MissingKeypointNamesPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportAcknowledgementRequest {
    pub diagnostic_code: String,
    pub policy: String,
    pub affected_count: u64,
    pub acknowledged: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateImportPlanRequest {
    pub category_mappings: Vec<ImportCategoryMappingRequest>,
    pub geometry_mappings: Vec<ImportGeometryMappingRequest>,
    pub task_mappings: Vec<ImportTaskMappingRequest>,
    pub skeleton_mappings: Vec<ImportSkeletonMappingRequest>,
    #[serde(default)]
    pub compatibility: ImportCompatibilityPolicies,
    #[serde(default)]
    pub acknowledgements: Vec<ImportAcknowledgementRequest>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPlan {
    pub import_id: ImportId,
    pub source_fingerprint: String,
    pub plan_hash: String,
    #[serde(default)]
    pub commit_ready: bool,
    #[serde(default)]
    pub blocking_diagnostic_codes: Vec<String>,
    #[serde(default)]
    pub required_acknowledgement_codes: Vec<String>,
    #[serde(default)]
    pub report: ImportPreflightReport,
    #[serde(default)]
    pub source_categories: Vec<ImportSourceCategory>,
    #[serde(default)]
    pub accepted_request: Option<UpdateImportPlanRequest>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSourceCategory {
    pub source_category_key: String,
    pub source_category_id: String,
    pub source_name: String,
    #[serde(default)]
    pub source_supercategory: Option<String>,
    pub source_namespace: String,
    #[serde(default)]
    pub direct_geometry: Vec<ImportGeometryKind>,
    #[serde(default)]
    pub keypoint_schema: Option<SkeletonSpec>,
    pub generated_category_mapping: ImportCategoryMappingRequest,
    #[serde(default)]
    pub generated_task_mappings: Vec<ImportTaskMappingRequest>,
    pub current_category_mapping: ImportCategoryMappingRequest,
    #[serde(default)]
    pub current_geometry_mappings: Vec<ImportGeometryMappingRequest>,
    #[serde(default)]
    pub current_task_mappings: Vec<ImportTaskMappingRequest>,
    #[serde(default)]
    pub current_skeleton_mappings: Vec<ImportSkeletonMappingRequest>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommitImportRequest {
    pub plan_hash: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitImportResult {
    pub import_id: ImportId,
    pub dataset_id: DatasetId,
    pub plan_hash: String,
    #[serde(default)]
    pub recovered: bool,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelImportRequest {
    #[serde(default)]
    pub reason: Option<String>,
}

impl fmt::Debug for CancelImportRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancelImportRequest")
            .field("reason", &self.reason.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelImportResult {
    pub import_id: ImportId,
    pub lifecycle: ImportLifecycle,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MigrationTargetExpectation {
    pub object_group_id: ObjectGroupId,
    pub expected_guide_annotation_version: u32,
    pub expected_guide_deleted: bool,
    pub expected_disposition_version: u32,
    #[serde(default)]
    pub expected_skeleton_version: Option<u32>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveMigrationSkeletonRequest {
    pub assignment_id: AssignmentId,
    #[serde(default)]
    pub pass_id: Option<MigrationPassId>,
    pub target: MigrationTargetExpectation,
    pub skeleton: SkeletonGeometry,
}

impl fmt::Debug for SaveMigrationSkeletonRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SaveMigrationSkeletonRequest")
            .field("assignment_id", &self.assignment_id)
            .field("pass_id", &self.pass_id)
            .field("target", &self.target)
            .field("skeleton", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AddMigrationSkeletonRequest {
    pub assignment_id: AssignmentId,
    #[serde(default)]
    pub pass_id: Option<MigrationPassId>,
    pub task_id: TaskId,
    pub skeleton: SkeletonGeometry,
}

impl fmt::Debug for AddMigrationSkeletonRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AddMigrationSkeletonRequest")
            .field("assignment_id", &self.assignment_id)
            .field("pass_id", &self.pass_id)
            .field("task_id", &self.task_id)
            .field("skeleton", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditMigrationSkeletonRequest {
    pub assignment_id: AssignmentId,
    #[serde(default)]
    pub pass_id: Option<MigrationPassId>,
    pub task_id: TaskId,
    pub annotation_id: AnnotationId,
    pub expected_version: u32,
    pub skeleton: SkeletonGeometry,
}

impl fmt::Debug for EditMigrationSkeletonRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EditMigrationSkeletonRequest")
            .field("assignment_id", &self.assignment_id)
            .field("pass_id", &self.pass_id)
            .field("task_id", &self.task_id)
            .field("annotation_id", &self.annotation_id)
            .field("expected_version", &self.expected_version)
            .field("skeleton", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteMigrationSkeletonRequest {
    pub assignment_id: AssignmentId,
    #[serde(default)]
    pub pass_id: Option<MigrationPassId>,
    pub task_id: TaskId,
    pub annotation_id: AnnotationId,
    pub expected_version: u32,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExcludeMigrationTargetRequest {
    pub assignment_id: AssignmentId,
    #[serde(default)]
    pub pass_id: Option<MigrationPassId>,
    pub target: MigrationTargetExpectation,
    pub reason: MigrationExclusionReason,
    #[serde(default)]
    pub note: Option<String>,
}

impl fmt::Debug for ExcludeMigrationTargetRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExcludeMigrationTargetRequest")
            .field("assignment_id", &self.assignment_id)
            .field("pass_id", &self.pass_id)
            .field("target", &self.target)
            .field("reason", &self.reason)
            .field("note", &self.note.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReopenMigrationTargetRequest {
    pub assignment_id: AssignmentId,
    #[serde(default)]
    pub pass_id: Option<MigrationPassId>,
    pub target: MigrationTargetExpectation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RevisitMigrationTargetRequest {
    pub assignment_id: AssignmentId,
    #[serde(default)]
    pub pass_id: Option<MigrationPassId>,
    pub target: MigrationTargetExpectation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartMigrationPassRequest {
    pub assignment_id: AssignmentId,
    pub task_id: TaskId,
    pub expected_target_set_hash: MigrationHash,
    pub expected_state_hash: MigrationHash,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeepMigrationTargetRequest {
    pub assignment_id: AssignmentId,
    pub pass_id: MigrationPassId,
    pub target: MigrationTargetExpectation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfirmMigrationRequest {
    pub assignment_id: AssignmentId,
    pub task_id: TaskId,
    pub target_set_hash: MigrationHash,
    pub state_hash: MigrationHash,
    pub confirmation_hash: MigrationHash,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewMigrationRequest {
    pub assignment_id: AssignmentId,
    pub task_id: TaskId,
    pub target: MigrationReviewTarget,
    pub decision: ReviewDecision,
    #[serde(default)]
    pub comment: Option<String>,
}

impl fmt::Debug for ReviewMigrationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReviewMigrationRequest")
            .field("assignment_id", &self.assignment_id)
            .field("task_id", &self.task_id)
            .field("target", &self.target)
            .field("decision", &self.decision)
            .field("comment", &self.comment.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "targetType",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum MigrationReviewTarget {
    Disposition {
        object_group_id: ObjectGroupId,
        disposition_version: u32,
    },
    Confirmation {
        confirmation_hash: MigrationHash,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualMigrationProgress {
    #[serde(default)]
    pub expected: u64,
    #[serde(default)]
    pub annotated: u64,
    #[serde(default)]
    pub excluded: u64,
    #[serde(default)]
    pub pending: u64,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualMigrationCommandResult {
    pub image_state: ImageState,
    #[serde(default)]
    pub cursor: Option<MigrationCursor>,
    #[serde(default)]
    pub progress: ManualMigrationProgress,
    #[serde(default)]
    pub active_pass: Option<MigrationPass>,
    #[serde(default)]
    pub confirmation: Option<MigrationConfirmation>,
    #[serde(default)]
    pub assignment: Option<Assignment>,
    #[serde(default)]
    pub annotation_id: Option<AnnotationId>,
}

impl fmt::Debug for ManualMigrationCommandResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ManualMigrationCommandResult")
            .field("image_state", &"<redacted>")
            .field("cursor", &self.cursor)
            .field("progress", &self.progress)
            .field(
                "active_pass",
                &self.active_pass.as_ref().map(|_| "<redacted>"),
            )
            .field(
                "confirmation",
                &self.confirmation.as_ref().map(|_| "<redacted>"),
            )
            .field("assignment", &self.assignment)
            .field("annotation_id", &self.annotation_id)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_profiles_and_fields_use_the_contract_casing() {
        let request = CreateImportRequest {
            destination_dataset_id: DatasetId::from("animals"),
            destination_name: "Animals".to_string(),
            profile: ImportProfile::CocoKeypointsGtV1,
            source: ImportSourceSelection::BrowserFolder,
            attestations: ImportAttestations {
                ground_truth: true,
                exhaustive: false,
                coverage_scope: vec!["person".to_string()],
                provenance: "curated release".to_string(),
            },
        };

        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::json!({
                "destinationDatasetId": "animals",
                "destinationName": "Animals",
                "profile": "coco_keypoints_gt_v1",
                "source": { "transport": "browser_folder" },
                "attestations": {
                    "groundTruth": true,
                    "exhaustive": false,
                    "coverageScope": ["person"],
                    "provenance": "curated release"
                }
            })
        );
    }

    #[test]
    fn import_requests_reject_unknown_fields() {
        let error = serde_json::from_value::<CommitImportRequest>(serde_json::json!({
            "planHash": "plan",
            "force": true
        }))
        .unwrap_err();
        assert!(error.to_string().contains("unknown field"));

        let error = serde_json::from_value::<InspectYoloDescriptorRequest>(serde_json::json!({
            "descriptorFileId": "file-1",
            "path": "private.yaml"
        }))
        .unwrap_err();
        assert!(error.to_string().contains("unknown field"));

        let error = serde_json::from_value::<BrowseImportSourceRequest>(serde_json::json!({
            "relativePath": "release",
            "offset": 0,
            "mode": "descriptors",
            "recursive": true
        }))
        .unwrap_err();
        assert!(error.to_string().contains("unknown field"));

        let error = serde_json::from_value::<CreateImportRequest>(serde_json::json!({
            "destinationDatasetId": "animals",
            "destinationName": "Animals",
            "profile": "coco_instances_gt_v1",
            "source": { "transport": "server_directory", "importRootId": "root", "relativePath": "source", "followLinks": true },
            "attestations": { "groundTruth": true, "exhaustive": true, "coverageScope": [], "provenance": "release" }
        }))
        .unwrap_err();
        assert!(error.to_string().contains("unknown field"));

        let error = serde_json::from_value::<RevisitMigrationTargetRequest>(serde_json::json!({
            "assignmentId": "asg_1",
            "passId": null,
            "target": {
                "objectGroupId": "group_1",
                "expectedGuideAnnotationVersion": 1,
                "expectedGuideDeleted": false,
                "expectedDispositionVersion": 2,
                "expectedSkeletonVersion": 1
            },
            "jump": true
        }))
        .unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn import_responses_ignore_new_fields_and_unknown_enum_values() {
        let capabilities: ImportCapabilities = serde_json::from_value(serde_json::json!({
            "available": true,
            "profiles": [{
                "profile": "future_profile_v2",
                "enabled": true,
                "futureOption": true
            }],
            "futureCapability": { "enabled": true }
        }))
        .unwrap();

        assert_eq!(capabilities.profiles[0].profile, ImportProfile::Unknown);
        assert!(capabilities.profiles[0].enabled);
        assert_eq!(capabilities.limits, ImportLimits::default());
    }

    #[test]
    fn defaults_are_strict_and_pagination_is_bounded() {
        let policies: ImportCompatibilityPolicies =
            serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(policies, ImportCompatibilityPolicies::default());
        assert_eq!(policies.geometry_bounds, GeometryBoundsPolicy::Reject);
        assert_eq!(policies.coco_crowds, CocoCrowdPolicy::Block);

        let query: ImportDiagnosticsQuery = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(query, ImportDiagnosticsQuery::default());
        assert_eq!(query.limit, 100);
    }

    #[test]
    fn sensitive_import_debug_output_is_redacted() {
        let upload = ImportChunkUpload {
            offset: 0,
            length: 4,
            digest: "blake3=secret-digest".to_string(),
            bytes: b"data".to_vec(),
        };
        let registration = ImportFileRegistration {
            client_file_id: "browser-1".to_string(),
            relative_path: "private/person/image.jpg".to_string(),
            byte_size: 4,
            blake3: Some("secret-file-digest".to_string()),
        };
        let exclusion = ExcludeMigrationTargetRequest {
            assignment_id: AssignmentId::from("asg_1"),
            pass_id: None,
            target: MigrationTargetExpectation {
                object_group_id: ObjectGroupId::from("group_1"),
                expected_guide_annotation_version: 1,
                expected_guide_deleted: false,
                expected_disposition_version: 1,
                expected_skeleton_version: None,
            },
            reason: MigrationExclusionReason::Other,
            note: Some("private review note".to_string()),
        };
        let edit = EditMigrationSkeletonRequest {
            assignment_id: AssignmentId::from("asg_1"),
            pass_id: None,
            task_id: TaskId::from("skeleton:person"),
            annotation_id: AnnotationId::from("ann_discovered"),
            expected_version: 1,
            skeleton: SkeletonGeometry {
                keypoints: vec![labello_domain::KeypointAnnotation {
                    name: "private-keypoint".to_string(),
                    state: KeypointState::Visible,
                    point: Some(labello_domain::NormalizedPoint { x: 0.25, y: 0.75 }),
                }],
            },
        };
        let inspection = InspectYoloDescriptorRequest {
            descriptor_file_id: "private/dataset.yaml".to_string(),
        };
        let browse = BrowseImportSourceRequest {
            relative_path: "private/release".to_string(),
            offset: 0,
            mode: ImportSourceBrowseMode::Descriptors,
        };
        let browse_page = ImportBrowsePage {
            relative_path: "private/release".to_string(),
            entries: vec![ImportBrowseEntry {
                name: "dataset.yaml".to_string(),
                relative_path: "private/release/dataset.yaml".to_string(),
                kind: ImportBrowseEntryKind::File,
                file_id: Some("secret-file-id".to_string()),
            }],
            next_offset: None,
        };

        let output = format!(
            "{upload:?} {registration:?} {exclusion:?} {edit:?} {inspection:?} {browse:?} {browse_page:?}"
        );
        for secret in [
            "secret-digest",
            "data",
            "private/person/image.jpg",
            "secret-file-digest",
            "private review note",
            "private-keypoint",
            "private/dataset.yaml",
            "private/release",
            "dataset.yaml",
            "secret-file-id",
        ] {
            assert!(!output.contains(secret), "Debug output leaked {secret}");
        }
    }
}
