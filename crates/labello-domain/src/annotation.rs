use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    AnnotationGeometry, AnnotationId, AnnotationType, ClassId, CorrectionId, DomainError,
    DomainResult, ImageDimensions, PrelabelConfigId, TaskDefinition, TaskId, Timestamp, UserId,
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
    ReviewerCorrection {
        correction_id: CorrectionId,
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
            (AnnotationType::BoundingBox, AnnotationGeometry::BoundingBox(_)) => {
                self.geometry.validate()
            }
            (AnnotationType::Skeleton, AnnotationGeometry::Skeleton(skeleton)) => {
                let spec = task.skeleton.as_ref().ok_or_else(|| {
                    DomainError::InvalidGeometry(format!(
                        "skeleton task {} has no skeleton specification",
                        task.task_id
                    ))
                })?;
                if skeleton.keypoints.len() != spec.keypoints.len() {
                    return Err(DomainError::InvalidGeometry(format!(
                        "skeleton has {} keypoints; task {} requires {}",
                        skeleton.keypoints.len(),
                        task.task_id,
                        spec.keypoints.len()
                    )));
                }
                for (keypoint, expected) in skeleton.keypoints.iter().zip(&spec.keypoints) {
                    if keypoint.name != expected.name {
                        return Err(DomainError::InvalidGeometry(format!(
                            "skeleton keypoint {} must be {} in configured order",
                            keypoint.name, expected.name
                        )));
                    }
                    match keypoint.state {
                        crate::KeypointState::Hidden if !spec.allow_hidden => {
                            return Err(DomainError::InvalidGeometry(format!(
                                "hidden keypoint {} is not allowed",
                                keypoint.name
                            )));
                        }
                        crate::KeypointState::Absent if !spec.allow_absent || expected.required => {
                            return Err(DomainError::InvalidGeometry(format!(
                                "absent keypoint {} is not allowed",
                                keypoint.name
                            )));
                        }
                        _ => {}
                    }
                }
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
#[serde(rename_all = "snake_case")]
pub enum TaskOutcome {
    AnnotationCompleted,
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

#[cfg(test)]
mod tests {
    use crate::{
        KeypointAnnotation, KeypointSpec, KeypointState, NormalizedPoint, ReviewConfig,
        SkeletonGeometry, SkeletonSpec, TaskDefinition, TutorialContent, now,
    };

    use super::*;

    #[test]
    fn skeleton_geometry_must_match_configured_keypoints_and_visibility_policy() {
        let task = skeleton_task();
        let valid = skeleton_annotation(vec![
            keypoint("nose", KeypointState::Visible),
            keypoint("tail", KeypointState::Absent),
        ]);
        assert!(
            valid
                .validate_for_task(
                    &task,
                    ImageDimensions {
                        width: 10,
                        height: 10
                    }
                )
                .is_ok()
        );

        for keypoints in [
            vec![keypoint("nose", KeypointState::Visible)],
            vec![
                keypoint("tail", KeypointState::Visible),
                keypoint("nose", KeypointState::Visible),
            ],
            vec![
                keypoint("nose", KeypointState::Hidden),
                keypoint("tail", KeypointState::Absent),
            ],
            vec![
                keypoint("nose", KeypointState::Absent),
                keypoint("tail", KeypointState::Visible),
            ],
        ] {
            assert!(
                skeleton_annotation(keypoints)
                    .validate_for_task(
                        &task,
                        ImageDimensions {
                            width: 10,
                            height: 10
                        }
                    )
                    .is_err()
            );
        }
    }

    fn skeleton_task() -> TaskDefinition {
        TaskDefinition {
            task_id: TaskId::from("skeleton:person"),
            name: "Person skeleton".to_string(),
            annotation_type: AnnotationType::Skeleton,
            class_ids: vec![ClassId::from("person")],
            instructions: TutorialContent {
                title: "Skeleton".to_string(),
                example_text: "Place keypoints".to_string(),
                example_images: Vec::new(),
            },
            skeleton: Some(SkeletonSpec {
                keypoints: vec![
                    KeypointSpec {
                        name: "nose".to_string(),
                        required: true,
                    },
                    KeypointSpec {
                        name: "tail".to_string(),
                        required: false,
                    },
                ],
                edges: Vec::new(),
                allow_hidden: false,
                allow_absent: true,
            }),
            review: ReviewConfig::default(),
            prelabel_config_ids: Vec::new(),
            enabled: true,
        }
    }

    fn skeleton_annotation(keypoints: Vec<KeypointAnnotation>) -> AnnotationVersion {
        AnnotationVersion {
            annotation_id: AnnotationId::from("ann_1"),
            version: 1,
            task_id: TaskId::from("skeleton:person"),
            class_id: ClassId::from("person"),
            annotation_type: AnnotationType::Skeleton,
            source: AnnotationSource::Human,
            geometry: AnnotationGeometry::Skeleton(SkeletonGeometry { keypoints }),
            author_user_id: UserId::from("annotator"),
            created_at: now(),
            updated_at: now(),
            deleted: false,
        }
    }

    fn keypoint(name: &str, state: KeypointState) -> KeypointAnnotation {
        let point = (state != KeypointState::Absent).then_some(NormalizedPoint { x: 0.5, y: 0.5 });
        KeypointAnnotation {
            name: name.to_string(),
            state,
            point,
        }
    }
}
