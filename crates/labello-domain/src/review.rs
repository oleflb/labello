use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    AdjudicationId, AnnotationId, AssignmentId, CorrectionId, ImageId, MigrationHash,
    ObjectGroupId, ReviewId, TaskId, Timestamp, UserId,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Approved,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "targetType", rename_all = "snake_case")]
pub enum ReviewTarget {
    AnnotationVersion {
        annotation_id: AnnotationId,
        version: u32,
    },
    MigrationDisposition {
        task_id: TaskId,
        object_group_id: ObjectGroupId,
        disposition_version: u32,
    },
    MigrationConfirmation {
        task_id: TaskId,
        confirmation_hash: MigrationHash,
    },
    Task {
        task_id: TaskId,
    },
    Image {
        image_id: ImageId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRecord {
    pub review_id: ReviewId,
    pub target: ReviewTarget,
    pub reviewer_user_id: UserId,
    pub decision: ReviewDecision,
    pub timestamp: Timestamp,
    pub comment: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewerCorrectionRecord {
    pub correction_id: CorrectionId,
    pub assignment_id: AssignmentId,
    pub annotation_id: AnnotationId,
    pub previous_version: u32,
    pub corrected_version: u32,
    pub task_id: TaskId,
    pub reviewer_user_id: UserId,
    pub timestamp: Timestamp,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AdjudicationDecision {
    AcceptAnnotation,
    RejectAnnotation,
    MergeAnnotations,
    NeedsCorrection,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdjudicationRecord {
    pub adjudication_id: AdjudicationId,
    pub task_id: TaskId,
    pub annotation_ids: Vec<AnnotationId>,
    pub adjudicator_user_id: UserId,
    pub decision: AdjudicationDecision,
    pub resolution: String,
    pub timestamp: Timestamp,
}

mod policy;

pub use policy::{current_task_reviews, has_task_review_by_user, task_approval_count};
