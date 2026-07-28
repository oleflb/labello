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
