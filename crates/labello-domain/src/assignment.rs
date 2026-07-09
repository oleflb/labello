use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{AssignmentId, ImageId, TaskId, Timestamp, UserId};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentKind {
    Annotation,
    Review,
    Adjudication,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentStatus {
    Active,
    Submitted,
    Completed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Assignment {
    pub assignment_id: AssignmentId,
    pub image_id: ImageId,
    pub task_id: TaskId,
    pub assigned_to: UserId,
    pub kind: AssignmentKind,
    pub status: AssignmentStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}
