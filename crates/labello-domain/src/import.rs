use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    AnnotationId, ClassId, DatasetId, ImageId, ImportId, KeypointState, MigrationCardinality,
    MigrationSequence, ObjectGroupId, SkeletonSpec, TaskDefinition, TaskId, Timestamp, UserId,
};

mod provenance;

pub use provenance::*;

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ImportDescriptorKind {
    YoloDataset,
    CocoInstances,
    CocoKeypoints,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportDescriptor {
    pub kind: ImportDescriptorKind,
    pub descriptor_path: String,
    pub image_root: Option<String>,
    pub source_namespace: String,
    pub release: String,
    pub split: String,
    pub pairing_group: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportAttestations {
    pub ground_truth: bool,
    pub exhaustive: bool,
    pub coverage_scope: Vec<String>,
    pub provenance: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportYoloMissingLabelPolicy {
    #[default]
    Block,
    MissingIsBackground,
    RetainIncomplete,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportDuplicateRowPolicy {
    #[default]
    Block,
    Deduplicate,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportCocoCrowdPolicy {
    #[default]
    Block,
    Incomplete,
    ExcludeImageTask,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportGeometryBoundsPolicy {
    #[default]
    Block,
    ClipDerived,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportCrossSplitDuplicatePolicy {
    #[default]
    Block,
    MultipleMemberships,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportKeypointNamePolicy {
    #[default]
    RequireSourceNames,
    GenerateIndexed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportCompatibilityPolicies {
    pub yolo_missing_labels: ImportYoloMissingLabelPolicy,
    pub yolo_duplicate_rows: ImportDuplicateRowPolicy,
    pub coco_crowds: ImportCocoCrowdPolicy,
    pub coco_bbox_only: bool,
    pub geometry_bounds: ImportGeometryBoundsPolicy,
    pub cross_split_duplicates: ImportCrossSplitDuplicatePolicy,
    pub yolo_keypoint_names: ImportKeypointNamePolicy,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportTemplateKeypoint {
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub state: KeypointState,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ImportGeometryKind {
    BoundingBox,
    Skeleton,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "policy",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ImportGeometryPolicy {
    Direct,
    KeypointEnvelopeV1 {
        padding_ratio: f64,
        minimum_pixels: u32,
        include_hidden: bool,
    },
    ManualBoxGuideV1,
    BoxRelativeTemplateV1 {
        keypoints: Vec<ImportTemplateKeypoint>,
    },
    Omit,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportGeometryMapping {
    pub source_category_key: String,
    pub source_geometry: ImportGeometryKind,
    pub target_geometry: ImportGeometryKind,
    pub policy: ImportGeometryPolicy,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "policy", rename_all = "snake_case")]
pub enum ImportBoxToSkeletonPolicy {
    #[default]
    None,
    Template {
        keypoints: Vec<ImportTemplateKeypoint>,
    },
    ManualBoxGuide {
        keypoint_names: Vec<String>,
        edges: Vec<(String, String)>,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportTransformPolicies {
    pub bounding_boxes: bool,
    pub skeletons: bool,
    pub box_to_skeleton: ImportBoxToSkeletonPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportSourceFile {
    pub relative_path: String,
    pub byte_size: u64,
    pub blake3: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportCategoryMapping {
    pub source_namespace: String,
    pub source_category_key: String,
    pub source_category_id: String,
    pub source_name: String,
    pub source_supercategory: Option<String>,
    pub class_id: ClassId,
    pub class_name: String,
    pub color: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportTaskIntent {
    AuthoritativeGroundTruth,
    RequireApproval,
    SeedFutureAnnotation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportTaskMapping {
    pub source_category_key: String,
    pub task: TaskDefinition,
    pub intent: ImportTaskIntent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportSkeletonMapping {
    pub source_category_key: String,
    pub target_task_id: TaskId,
    pub source_keypoint_names: Vec<String>,
    pub skeleton: SkeletonSpec,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportManualMigrationMapping {
    pub source_category_key: String,
    pub guide_task_id: TaskId,
    pub target_task_id: TaskId,
    pub cardinality: MigrationCardinality,
    pub allow_exclusion: bool,
    pub sequence: MigrationSequence,
    pub expected_targets: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportManifest {
    pub schema_version: u32,
    pub import_id: ImportId,
    pub dataset_id: DatasetId,
    pub source_profile: SourceProfile,
    pub source_fingerprint: String,
    pub plan_hash: String,
    pub parser_version: String,
    pub tool_version: String,
    pub descriptors: Vec<ImportDescriptor>,
    pub source_files: Vec<ImportSourceFile>,
    pub attestations: ImportAttestations,
    pub compatibility_policies: ImportCompatibilityPolicies,
    pub transform_policies: ImportTransformPolicies,
    pub acknowledged_warning_codes: Vec<String>,
    pub category_mappings: Vec<ImportCategoryMapping>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub geometry_mappings: Vec<ImportGeometryMapping>,
    pub task_mappings: Vec<ImportTaskMapping>,
    pub skeleton_mappings: Vec<ImportSkeletonMapping>,
    pub manual_migration_mappings: Vec<ImportManualMigrationMapping>,
    pub source_memberships: BTreeMap<ImageId, Vec<SourceMembership>>,
    pub coverage_totals: ImportCoverageTotals,
    pub migration_totals: ImportMigrationTotals,
    pub output_totals: ImportOutputTotals,
    pub output_integrity: BTreeMap<String, String>,
    pub created_by: UserId,
    pub created_at: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportedObjectMapping {
    pub source_object_key: String,
    pub object_group_id: Option<ObjectGroupId>,
    pub annotation_ids: Vec<AnnotationId>,
}
