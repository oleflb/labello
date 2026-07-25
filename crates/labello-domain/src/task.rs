use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    ClassId, DomainError, DomainResult, ManualBoxGuideMigration, MigrationCardinality,
    MigrationSequence, PrelabelConfigId, TaskId, Timestamp, UserId,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationType {
    BoundingBox,
    Skeleton,
}

impl std::fmt::Display for AnnotationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::BoundingBox => "bounding_box",
            Self::Skeleton => "skeleton",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LabelClass {
    pub class_id: ClassId,
    pub name: String,
    pub color: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TutorialContent {
    pub title: String,
    pub example_text: String,
    pub example_images: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct KeypointSpec {
    pub name: String,
    pub required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkeletonEdge {
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkeletonSpec {
    pub keypoints: Vec<KeypointSpec>,
    pub edges: Vec<SkeletonEdge>,
    pub allow_hidden: bool,
    pub allow_absent: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewConfig {
    pub required_reviews: u32,
    pub workflow: ReviewWorkflow,
    pub allow_reviewer_corrections: bool,
    pub agreement_threshold: Option<AgreementThreshold>,
}

impl Default for ReviewConfig {
    fn default() -> Self {
        Self {
            required_reviews: 1,
            workflow: ReviewWorkflow::Approval,
            allow_reviewer_corrections: false,
            agreement_threshold: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewWorkflow {
    None,
    Approval,
    IndependentAgreement,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgreementThreshold {
    pub metric: AgreementMetric,
    pub threshold: f32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgreementMetric {
    Iou,
    KeypointMeanDistance,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TaskDefinition {
    pub task_id: TaskId,
    pub name: String,
    pub annotation_type: AnnotationType,
    pub class_ids: Vec<ClassId>,
    pub instructions: TutorialContent,
    pub skeleton: Option<SkeletonSpec>,
    pub review: ReviewConfig,
    pub prelabel_config_ids: Vec<PrelabelConfigId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_box_guide_migration: Option<ManualBoxGuideMigration>,
    pub enabled: bool,
}

impl TaskDefinition {
    pub fn allows_class(&self, class_id: &ClassId) -> bool {
        self.class_ids.iter().any(|candidate| candidate == class_id)
    }

    pub fn validate_manual_migration(&self, guide: &TaskDefinition) -> DomainResult<()> {
        let Some(config) = &self.manual_box_guide_migration else {
            return Ok(());
        };
        let valid = self.annotation_type == AnnotationType::Skeleton
            && guide.annotation_type == AnnotationType::BoundingBox
            && config.guide_task_id == guide.task_id
            && self.task_id != guide.task_id
            && self.class_ids.len() == 1
            && self.class_ids == guide.class_ids
            && config.cardinality == MigrationCardinality::ExactlyOne
            && config.sequence == MigrationSequence::ImportedSpatialOrderV1
            && config.allow_exclusion
            && matches!(
                self.review.workflow,
                ReviewWorkflow::None | ReviewWorkflow::Approval
            )
            && !self.review.allow_reviewer_corrections;
        if valid {
            Ok(())
        } else {
            Err(DomainError::InvalidMigration(format!(
                "task {} has an invalid manual box-guide migration configuration",
                self.task_id
            )))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Submitted,
    Completed,
    NeedsCorrection,
    AdjudicationRequired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutcome {
    AnnotationCompleted,
    ImportedGroundTruth,
    Approved,
    ReviewerCorrected,
    Adjudicated,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TaskState {
    pub task_id: TaskId,
    pub status: TaskStatus,
    pub outcome: Option<TaskOutcome>,
    pub assigned_to: Option<UserId>,
    pub completed_by: Option<UserId>,
    pub completed_at: Option<Timestamp>,
    pub updated_at: Timestamp,
}

impl TaskState {
    pub fn new(task_id: TaskId, timestamp: Timestamp) -> Self {
        Self {
            task_id,
            status: TaskStatus::Pending,
            outcome: None,
            assigned_to: None,
            completed_by: None,
            completed_at: None,
            updated_at: timestamp,
        }
    }
}
