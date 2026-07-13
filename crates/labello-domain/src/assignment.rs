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
    /// End of the current lease. Legacy records omit this field and are
    /// interpreted by storage relative to `updated_at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_legacy_assignment_without_expiration() {
        let assignment: Assignment = serde_json::from_value(serde_json::json!({
            "assignmentId": "0190f6f5-4e8a-7a42-8ac7-20e973cf6d2a",
            "imageId": "image_1",
            "taskId": "bounding_box:person",
            "assignedTo": "annotator",
            "kind": "annotation",
            "status": "active",
            "createdAt": "2026-01-01T00:00:00Z",
            "updatedAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();

        assert_eq!(assignment.expires_at, None);
        assert!(
            serde_json::to_value(assignment)
                .unwrap()
                .get("expiresAt")
                .is_none()
        );
    }
}
