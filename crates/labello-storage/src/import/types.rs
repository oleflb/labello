use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use labello_domain::{
    ClassId, DatasetId, ImportCoverageTotals, ImportDescriptorKind, ImportGeometryMapping,
    ImportId, TaskDefinition, Timestamp, UserId,
};
use serde::{Deserialize, Serialize};

pub const PROFILE_YOLO_DETECT: &str = "ultralytics_yolo_detect_v1";
pub const PROFILE_YOLO_POSE: &str = "ultralytics_yolo_pose_v1";
pub const PROFILE_COCO_INSTANCES: &str = "coco_instances_gt_v1";
pub const PROFILE_COCO_KEYPOINTS: &str = "coco_keypoints_gt_v1";
pub const IMPORT_PARSER_VERSION: &str = "labello-storage-import-v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportConfig {
    pub enabled: bool,
    pub import_roots: Vec<ImportRoot>,
    pub allowed_profiles: Vec<ImportProfile>,
    pub limits: ImportLimits,
    pub retain_raw_source: bool,
    pub failed_retention: Duration,
    pub successful_metadata_retention: Duration,
}

impl Default for ImportConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            import_roots: Vec::new(),
            allowed_profiles: ImportProfile::ALL.to_vec(),
            limits: ImportLimits::default(),
            retain_raw_source: false,
            failed_retention: Duration::from_secs(24 * 60 * 60),
            successful_metadata_retention: Duration::from_secs(30 * 24 * 60 * 60),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRoot {
    pub root_id: String,
    pub path: PathBuf,
    /// Empty means access is controlled entirely by the caller's bootstrap-admin check.
    pub allowed_owners: Vec<UserId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportLimits {
    pub concurrent_build_jobs: usize,
    pub concurrent_browser_upload_jobs: usize,
    pub active_reservations_per_owner: usize,
    pub browser_source_files: usize,
    pub browser_source_bytes: u64,
    pub server_source_files: usize,
    pub total_source_bytes: u64,
    pub selected_images: usize,
    pub single_source_file_bytes: u64,
    pub descriptor_bytes: u64,
    pub upload_chunk_bytes: usize,
    pub source_path_bytes: usize,
    pub source_path_depth: usize,
    pub source_component_bytes: usize,
    pub selected_categories: usize,
    pub selected_tasks: usize,
    pub coverage_entries: usize,
    pub annotations_total: usize,
    pub annotations_per_image: usize,
    pub generated_file_bytes_per_image: u64,
    pub keypoints_per_skeleton: usize,
    pub yolo_line_bytes: usize,
    pub yolo_columns: usize,
    pub structured_data_nesting: usize,
    pub decoded_image_pixels: u64,
    pub decoded_image_bytes: u64,
    pub staged_bytes: u64,
    pub diagnostic_examples_per_code: usize,
}

impl Default for ImportLimits {
    fn default() -> Self {
        Self {
            concurrent_build_jobs: 1,
            concurrent_browser_upload_jobs: 2,
            active_reservations_per_owner: 2,
            browser_source_files: 25_000,
            browser_source_bytes: 20 * 1024 * 1024 * 1024,
            server_source_files: 50_000,
            total_source_bytes: 100 * 1024 * 1024 * 1024,
            selected_images: 10_000,
            single_source_file_bytes: 4 * 1024 * 1024 * 1024,
            descriptor_bytes: 16 * 1024 * 1024,
            upload_chunk_bytes: 8 * 1024 * 1024,
            source_path_bytes: 1024,
            source_path_depth: 32,
            source_component_bytes: 255,
            selected_categories: 100,
            selected_tasks: 200,
            coverage_entries: 2_000_000,
            annotations_total: 1_000_000,
            annotations_per_image: 10_000,
            generated_file_bytes_per_image: 64 * 1024 * 1024,
            keypoints_per_skeleton: 512,
            yolo_line_bytes: 1024 * 1024,
            yolo_columns: 4096,
            structured_data_nesting: 64,
            decoded_image_pixels: 50_000_000,
            decoded_image_bytes: 512 * 1024 * 1024,
            staged_bytes: 250 * 1024 * 1024 * 1024,
            diagnostic_examples_per_code: 100,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportProfile {
    UltralyticsYoloDetectV1,
    UltralyticsYoloPoseV1,
    CocoInstancesGtV1,
    CocoKeypointsGtV1,
}

impl ImportProfile {
    pub const ALL: [Self; 4] = [
        Self::UltralyticsYoloDetectV1,
        Self::UltralyticsYoloPoseV1,
        Self::CocoInstancesGtV1,
        Self::CocoKeypointsGtV1,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::UltralyticsYoloDetectV1 => PROFILE_YOLO_DETECT,
            Self::UltralyticsYoloPoseV1 => PROFILE_YOLO_POSE,
            Self::CocoInstancesGtV1 => PROFILE_COCO_INSTANCES,
            Self::CocoKeypointsGtV1 => PROFILE_COCO_KEYPOINTS,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportCapabilities {
    pub available: bool,
    pub unavailable_reason: Option<String>,
    pub profiles: Vec<ImportProfile>,
    pub browser_upload: bool,
    pub server_directory_roots: Vec<String>,
    pub limits: ImportLimits,
    pub schema_version: u32,
    pub parser_version: String,
    pub atomic_publication: bool,
    pub secure_server_open: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportTransport {
    Browser,
    ServerDirectory,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportJobPhase {
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
}

impl ImportJobPhase {
    pub fn terminal(&self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Expired
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportJob {
    pub schema_version: u32,
    pub import_id: ImportId,
    pub owner_user_id: UserId,
    pub destination_dataset_id: DatasetId,
    pub destination_name: String,
    pub profile: ImportProfile,
    pub transport: ImportTransport,
    pub phase: ImportJobPhase,
    pub source_fingerprint: Option<String>,
    pub plan_hash: Option<String>,
    #[serde(default)]
    pub preflight_generation: Option<String>,
    pub accepted_files: usize,
    pub accepted_bytes: u64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub failure_code: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateImportRequest {
    pub destination_dataset_id: DatasetId,
    pub destination_name: String,
    pub profile: ImportProfile,
    pub transport: ImportTransport,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserFileRegistration {
    pub relative_path: String,
    pub byte_size: u64,
    pub blake3: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredFile {
    pub file_id: String,
    pub relative_path: String,
    pub byte_size: u64,
    pub blake3: String,
    pub accepted_bytes: u64,
    pub complete: bool,
    #[serde(default)]
    pub accepted_chunks: BTreeMap<u64, AcceptedChunk>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptedChunk {
    pub length: usize,
    pub blake3: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerDirectorySelection {
    pub root_id: String,
    pub relative_directory: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CocoDescriptorSelection {
    pub kind: ImportDescriptorKind,
    pub descriptor_path: String,
    pub image_root: String,
    pub split: String,
    pub source_namespace: String,
    pub release: String,
    pub pairing_group: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportIntent {
    AuthoritativeGroundTruth,
    RequireApproval,
    SeedFutureAnnotation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum YoloMissingLabelPolicy {
    Block,
    MissingIsBackground,
    RetainIncomplete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateRowPolicy {
    Block,
    Deduplicate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CocoCrowdPolicy {
    Block,
    Incomplete,
    ExcludeImageTask,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeometryBoundsPolicy {
    Block,
    ClipDerived,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrossSplitDuplicatePolicy {
    Block,
    MultipleMemberships,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum YoloKeypointNamePolicy {
    RequireSourceNames,
    GenerateIndexed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompatibilityPolicies {
    pub yolo_missing_labels: YoloMissingLabelPolicy,
    pub yolo_duplicate_rows: DuplicateRowPolicy,
    pub coco_crowds: CocoCrowdPolicy,
    pub coco_bbox_only: bool,
    pub geometry_bounds: GeometryBoundsPolicy,
    pub cross_split_duplicates: CrossSplitDuplicatePolicy,
    pub yolo_keypoint_names: YoloKeypointNamePolicy,
}

impl Default for CompatibilityPolicies {
    fn default() -> Self {
        Self {
            yolo_missing_labels: YoloMissingLabelPolicy::Block,
            yolo_duplicate_rows: DuplicateRowPolicy::Block,
            coco_crowds: CocoCrowdPolicy::Block,
            coco_bbox_only: false,
            geometry_bounds: GeometryBoundsPolicy::Block,
            cross_split_duplicates: CrossSplitDuplicatePolicy::Block,
            yolo_keypoint_names: YoloKeypointNamePolicy::RequireSourceNames,
        }
    }
}

pub type TemplateKeypoint = labello_domain::ImportTemplateKeypoint;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "policy", rename_all = "snake_case")]
pub enum BoxToSkeletonPolicy {
    #[default]
    None,
    Template {
        keypoints: Vec<TemplateKeypoint>,
    },
    ManualBoxGuide {
        keypoint_names: Vec<String>,
        edges: Vec<(String, String)>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputPolicy {
    pub bounding_boxes: bool,
    pub skeletons: bool,
    pub box_to_skeleton: BoxToSkeletonPolicy,
}

impl OutputPolicy {
    pub fn defaults_for(profile: ImportProfile) -> Self {
        let pose = matches!(
            profile,
            ImportProfile::UltralyticsYoloPoseV1 | ImportProfile::CocoKeypointsGtV1
        );
        Self {
            bounding_boxes: true,
            skeletons: pose,
            box_to_skeleton: BoxToSkeletonPolicy::None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightRequest {
    pub descriptor_paths: Vec<String>,
    pub selected_splits: Vec<String>,
    pub coco_descriptors: Vec<CocoDescriptorSelection>,
    pub ground_truth_attested: bool,
    pub exhaustive_attested: bool,
    pub source_namespace: String,
    pub source_release: String,
    pub coverage_scope: Vec<String>,
    pub attestation_provenance: String,
    pub intent: ImportIntent,
    pub policies: CompatibilityPolicies,
    pub output: OutputPolicy,
    pub acknowledged_warning_codes: Vec<String>,
    #[serde(default)]
    pub category_mappings: Vec<ImportCategoryMapping>,
    #[serde(default)]
    pub task_mappings: Vec<ImportTaskMapping>,
    #[serde(default)]
    pub geometry_mappings: Vec<ImportGeometryMapping>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportCategoryMapping {
    pub source_category_key: String,
    pub source_category_id: String,
    pub class_id: ClassId,
    pub class_name: String,
    pub color: String,
    pub selected: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportTaskMapping {
    pub source_category_key: String,
    pub task: TaskDefinition,
    pub intent: ImportIntent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
    WarningRequiresAck,
    Warning,
    Info,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticExample {
    pub source_path: Option<String>,
    pub source_image_key: Option<String>,
    pub source_object_key: Option<String>,
    pub line: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDiagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub profile: ImportProfile,
    pub count: u64,
    pub summary: String,
    pub blocks_commit: bool,
    pub requires_acknowledgement: bool,
    pub changes_coverage: bool,
    pub examples: Vec<DiagnosticExample>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportTotals {
    pub source_files: usize,
    pub source_bytes: u64,
    pub descriptors: usize,
    pub images: usize,
    pub categories: usize,
    pub source_objects: usize,
    pub keypoints: usize,
    pub direct_boxes: usize,
    pub direct_skeletons: usize,
    pub derived_geometry: usize,
    #[serde(default)]
    pub clipped_geometry: usize,
    #[serde(default)]
    pub envelope_derived: usize,
    #[serde(default)]
    pub template_derived: usize,
    pub output_tasks: usize,
    pub output_annotations: usize,
    pub estimated_output_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPlan {
    pub schema_version: u32,
    pub import_id: ImportId,
    pub destination_dataset_id: DatasetId,
    pub source_fingerprint: String,
    pub plan_hash: String,
    pub request: PreflightRequest,
    pub totals: ImportTotals,
    pub coverage: ImportCoverageTotals,
    pub diagnostics: Vec<ImportDiagnostic>,
    pub source_categories: BTreeMap<String, ImportSourceCategory>,
    pub class_ids: BTreeMap<String, String>,
    pub task_ids: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSourceCategory {
    pub source_namespace: String,
    pub source_category_id: String,
    pub source_name: String,
    pub source_supercategory: Option<String>,
    #[serde(default)]
    pub direct_bounding_boxes: bool,
    #[serde(default)]
    pub direct_skeletons: bool,
    #[serde(default)]
    pub keypoint_names: Vec<String>,
    #[serde(default)]
    pub edges: Vec<(String, String)>,
    #[serde(default)]
    pub allow_hidden: bool,
}

impl ImportPlan {
    pub fn committable(&self) -> bool {
        !self.diagnostics.iter().any(|diagnostic| {
            diagnostic.blocks_commit
                || (diagnostic.requires_acknowledgement
                    && !self
                        .request
                        .acknowledged_warning_codes
                        .contains(&diagnostic.code))
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportCommitResult {
    pub import_id: ImportId,
    pub dataset_id: DatasetId,
    pub dataset_path: PathBuf,
    pub recovered: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryReport {
    pub recovered_successes: usize,
    pub resumed_to_awaiting_decision: usize,
    pub failed_incomplete_commits: usize,
    pub released_reservations: usize,
    pub expired_abandoned_jobs: usize,
}
