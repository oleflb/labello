use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    AnnotationGeometry, AnnotationId, AnnotationType, ClassId, DomainError, DomainResult,
    ImageDimensions, PrelabelConfigId, TaskDefinition, TaskId, Timestamp, UserId,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum AnnotationSource {
    Human,
    PrelabelSuggestion {
        config_id: PrelabelConfigId,
        model_id: String,
        confidence: f32,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationVersion {
    pub annotation_id: AnnotationId,
    pub version: u32,
    pub task_id: TaskId,
    pub class_id: ClassId,
    #[serde(rename = "type")]
    pub annotation_type: AnnotationType,
    pub source: AnnotationSource,
    pub geometry: AnnotationGeometry,
    pub author_user_id: UserId,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub deleted: bool,
}

impl AnnotationVersion {
    pub fn validate_for_task(
        &self,
        task: &TaskDefinition,
        image_dimensions: ImageDimensions,
    ) -> DomainResult<()> {
        image_dimensions.validate()?;
        if self.annotation_type != task.annotation_type {
            return Err(DomainError::AnnotationTypeMismatch {
                task_id: task.task_id.to_string(),
                annotation_type: self.annotation_type.to_string(),
            });
        }
        if !task.allows_class(&self.class_id) {
            return Err(DomainError::ClassNotAllowed {
                task_id: task.task_id.to_string(),
                class_id: self.class_id.to_string(),
            });
        }
        match (&self.annotation_type, &self.geometry) {
            (AnnotationType::BoundingBox, AnnotationGeometry::BoundingBox(_))
            | (AnnotationType::Skeleton, AnnotationGeometry::Skeleton(_)) => {
                self.geometry.validate()
            }
            _ => Err(DomainError::AnnotationTypeMismatch {
                task_id: task.task_id.to_string(),
                annotation_type: self.annotation_type.to_string(),
            }),
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
#[serde(rename_all = "camelCase")]
pub struct TaskState {
    pub task_id: TaskId,
    pub status: TaskStatus,
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
            assigned_to: None,
            completed_by: None,
            completed_at: None,
            updated_at: timestamp,
        }
    }
}
