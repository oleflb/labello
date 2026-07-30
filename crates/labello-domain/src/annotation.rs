use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    AnnotationGeometry, AnnotationId, AnnotationType, ClassId, CorrectionId, DomainError,
    DomainResult, ImageDimensions, ImportId, ImportedOrigin, ObjectGroupId, PrelabelConfigId,
    TaskDefinition, TaskId, Timestamp, UserId,
};

pub use crate::task::{TaskOutcome, TaskState, TaskStatus};

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
#[serde(
    tag = "origin",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AnnotationOrigin {
    Native { legacy_v2: bool },
    Imported { imported: ImportedOrigin },
}

impl AnnotationOrigin {
    pub fn native() -> Self {
        Self::Native { legacy_v2: false }
    }

    pub fn legacy_v2() -> Self {
        Self::Native { legacy_v2: true }
    }

    pub fn is_legacy_v2(&self) -> bool {
        matches!(self, Self::Native { legacy_v2: true })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HumanRevisionKind {
    Authored,
    Edited,
    AcceptedUnchanged,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum RevisionSource {
    Human {
        action: HumanRevisionKind,
    },
    Import {
        import_id: ImportId,
    },
    PrelabelSuggestion {
        config_id: PrelabelConfigId,
        model_id: String,
        confidence: f32,
    },
    ReviewerCorrection {
        correction_id: CorrectionId,
    },
}

impl From<AnnotationSource> for RevisionSource {
    fn from(source: AnnotationSource) -> Self {
        match source {
            AnnotationSource::Human => Self::Human {
                action: HumanRevisionKind::Authored,
            },
            AnnotationSource::PrelabelSuggestion {
                config_id,
                model_id,
                confidence,
            } => Self::PrelabelSuggestion {
                config_id,
                model_id,
                confidence,
            },
            AnnotationSource::ReviewerCorrection { correction_id } => {
                Self::ReviewerCorrection { correction_id }
            }
        }
    }
}

impl From<&RevisionSource> for AnnotationSource {
    fn from(source: &RevisionSource) -> Self {
        match source {
            RevisionSource::Human { .. } | RevisionSource::Import { .. } => Self::Human,
            RevisionSource::PrelabelSuggestion {
                config_id,
                model_id,
                confidence,
            } => Self::PrelabelSuggestion {
                config_id: config_id.clone(),
                model_id: model_id.clone(),
                confidence: *confidence,
            },
            RevisionSource::ReviewerCorrection { correction_id } => Self::ReviewerCorrection {
                correction_id: correction_id.clone(),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationVersion {
    pub annotation_id: AnnotationId,
    pub version: u32,
    pub object_group_id: Option<ObjectGroupId>,
    pub origin: AnnotationOrigin,
    pub task_id: TaskId,
    pub class_id: ClassId,
    #[serde(rename = "type")]
    pub annotation_type: AnnotationType,
    pub revision_source: RevisionSource,
    pub geometry: AnnotationGeometry,
    pub author_user_id: UserId,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub deleted: bool,
}

impl AnnotationVersion {
    pub fn native(
        annotation_id: AnnotationId,
        task_id: TaskId,
        class_id: ClassId,
        annotation_type: AnnotationType,
        geometry: AnnotationGeometry,
        author_user_id: UserId,
        timestamp: Timestamp,
    ) -> Self {
        Self {
            annotation_id,
            version: 1,
            object_group_id: None,
            origin: AnnotationOrigin::native(),
            task_id,
            class_id,
            annotation_type,
            revision_source: RevisionSource::Human {
                action: HumanRevisionKind::Authored,
            },
            geometry,
            author_user_id,
            created_at: timestamp,
            updated_at: timestamp,
            deleted: false,
        }
    }

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

    #[test]
    fn generic_validation_keeps_historical_all_absent_skeletons_replayable() {
        let mut task = skeleton_task();
        let spec = task.skeleton.as_mut().unwrap();
        spec.keypoints[0].required = false;
        let annotation = skeleton_annotation(vec![
            keypoint("nose", KeypointState::Absent),
            keypoint("tail", KeypointState::Absent),
        ]);

        annotation
            .validate_for_task(
                &task,
                ImageDimensions {
                    width: 10,
                    height: 10,
                },
            )
            .unwrap();
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
            manual_box_guide_migration: None,
            enabled: true,
        }
    }

    fn skeleton_annotation(keypoints: Vec<KeypointAnnotation>) -> AnnotationVersion {
        AnnotationVersion {
            annotation_id: AnnotationId::from("ann_1"),
            version: 1,
            object_group_id: None,
            origin: AnnotationOrigin::native(),
            task_id: TaskId::from("skeleton:person"),
            class_id: ClassId::from("person"),
            annotation_type: AnnotationType::Skeleton,
            revision_source: RevisionSource::Human {
                action: HumanRevisionKind::Authored,
            },
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
