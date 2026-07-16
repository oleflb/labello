use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    AdjudicationRecord, AnnotationId, AnnotationVersion, Assignment, AssignmentKind,
    AssignmentStatus, DomainError, DomainResult, EventLogEntry, EventPayload, ImageId,
    ReviewDecision, ReviewRecord, ReviewTarget, ReviewerCorrectionRecord, SCHEMA_VERSION, TaskId,
    TaskOutcome, TaskState, TaskStatus,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImageState {
    pub schema_version: u32,
    pub image_id: ImageId,
    pub current_sequence: u64,
    pub annotations: BTreeMap<AnnotationId, Vec<AnnotationVersion>>,
    pub reviews: Vec<ReviewRecord>,
    pub reviewer_corrections: Vec<ReviewerCorrectionRecord>,
    pub adjudications: Vec<AdjudicationRecord>,
    pub task_states: BTreeMap<TaskId, TaskState>,
    pub assignments: Vec<Assignment>,
}

impl ImageState {
    pub fn new(image_id: ImageId) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            image_id,
            current_sequence: 0,
            annotations: BTreeMap::new(),
            reviews: Vec::new(),
            reviewer_corrections: Vec::new(),
            adjudications: Vec::new(),
            task_states: BTreeMap::new(),
            assignments: Vec::new(),
        }
    }

    pub fn apply_event(&mut self, event: &EventLogEntry) -> DomainResult<()> {
        if event.image_id != self.image_id {
            return Err(DomainError::ImageMismatch {
                expected: self.image_id.to_string(),
                found: event.image_id.to_string(),
            });
        }
        let expected = self.current_sequence + 1;
        if event.event_sequence != expected {
            return Err(DomainError::InvalidEventSequence {
                expected,
                found: event.event_sequence,
            });
        }
        event.validate_shape()?;
        match &event.payload {
            EventPayload::AnnotationVersionCreated {
                annotation,
                previous_version,
                ..
            } => {
                self.apply_annotation_version(annotation.clone(), *previous_version)?;
            }
            EventPayload::AnnotationDeleted {
                annotation_id,
                version,
                ..
            } => {
                let versions = self
                    .annotations
                    .get_mut(annotation_id)
                    .ok_or_else(|| DomainError::MissingAnnotation(annotation_id.to_string()))?;
                let current = versions
                    .last_mut()
                    .ok_or_else(|| DomainError::MissingAnnotation(annotation_id.to_string()))?;
                if current.version != *version || current.deleted {
                    return Err(DomainError::MissingAnnotationVersion {
                        annotation_id: annotation_id.to_string(),
                        version: *version,
                    });
                }
                current.deleted = true;
            }
            EventPayload::TaskStateChanged { task_state } => {
                self.task_states
                    .insert(task_state.task_id.clone(), task_state.clone());
            }
            EventPayload::ReviewRecorded { review } => self.reviews.push(review.clone()),
            EventPayload::ReviewerCorrectionRecorded {
                correction,
                annotation,
                review,
                task_state,
                assignments,
            } => {
                self.apply_reviewer_correction(
                    correction,
                    annotation,
                    review,
                    task_state,
                    assignments,
                )?;
            }
            EventPayload::AdjudicationRecorded { adjudication } => {
                self.adjudications.push(adjudication.clone());
            }
            EventPayload::AssignmentUpdated { assignment } => {
                if let Some(existing) = self
                    .assignments
                    .iter_mut()
                    .find(|candidate| candidate.assignment_id == assignment.assignment_id)
                {
                    *existing = assignment.clone();
                } else {
                    self.assignments.push(assignment.clone());
                }
            }
        }
        self.current_sequence = event.event_sequence;
        Ok(())
    }

    pub fn current_annotation(&self, annotation_id: &AnnotationId) -> Option<&AnnotationVersion> {
        self.annotations
            .get(annotation_id)
            .and_then(|versions| versions.last())
    }

    pub fn active_annotations(&self) -> impl Iterator<Item = &AnnotationVersion> {
        self.annotations
            .values()
            .filter_map(|versions| versions.last())
            .filter(|v| !v.deleted)
    }

    fn apply_annotation_version(
        &mut self,
        annotation: AnnotationVersion,
        previous_version: Option<u32>,
    ) -> DomainResult<()> {
        let versions = self
            .annotations
            .entry(annotation.annotation_id.clone())
            .or_default();
        if let Some(previous_version) = previous_version {
            let Some(previous) = versions.last() else {
                return Err(DomainError::MissingAnnotation(
                    annotation.annotation_id.to_string(),
                ));
            };
            if previous.version != previous_version || annotation.version != previous_version + 1 {
                return Err(DomainError::MissingAnnotationVersion {
                    annotation_id: annotation.annotation_id.to_string(),
                    version: previous_version,
                });
            }
        } else if annotation.version != 1 || !versions.is_empty() {
            return Err(DomainError::InvalidGeometry(format!(
                "annotation {} version chain is invalid",
                annotation.annotation_id
            )));
        }
        versions.push(annotation);
        Ok(())
    }

    fn apply_reviewer_correction(
        &mut self,
        correction: &ReviewerCorrectionRecord,
        annotation: &AnnotationVersion,
        review: &ReviewRecord,
        task_state: &TaskState,
        assignments: &[Assignment],
    ) -> DomainResult<()> {
        let valid_review = review.decision == ReviewDecision::Rejected
            && review.reviewer_user_id == correction.reviewer_user_id
            && matches!(
                &review.target,
                ReviewTarget::AnnotationVersion {
                    annotation_id,
                    version,
                } if annotation_id == &correction.annotation_id
                    && *version == correction.previous_version
            );
        let valid_annotation = annotation.annotation_id == correction.annotation_id
            && annotation.task_id == correction.task_id
            && annotation.version == correction.corrected_version
            && correction.corrected_version == correction.previous_version + 1
            && annotation.author_user_id == correction.reviewer_user_id
            && matches!(
                &annotation.source,
                crate::AnnotationSource::ReviewerCorrection { correction_id }
                    if correction_id == &correction.correction_id
            );
        let valid_task_state = task_state.task_id == correction.task_id
            && task_state.status == TaskStatus::Completed
            && task_state.outcome == Some(TaskOutcome::ReviewerCorrected)
            && task_state.completed_by.as_ref() == Some(&correction.reviewer_user_id);
        let valid_assignments = assignments.iter().any(|assignment| {
            assignment.assignment_id == correction.assignment_id
                && assignment.assigned_to == correction.reviewer_user_id
                && assignment.status == AssignmentStatus::Completed
        }) && assignments.iter().all(|assignment| {
            assignment.image_id == self.image_id
                && assignment.task_id == correction.task_id
                && assignment.kind == AssignmentKind::Review
                && matches!(
                    assignment.status,
                    AssignmentStatus::Completed | AssignmentStatus::Cancelled
                )
        });
        if !valid_review || !valid_annotation || !valid_task_state || !valid_assignments {
            return Err(DomainError::InvalidReviewerCorrection(
                correction.correction_id.to_string(),
            ));
        }
        if self
            .reviewer_corrections
            .iter()
            .any(|candidate| candidate.correction_id == correction.correction_id)
        {
            return Err(DomainError::DuplicateReviewerCorrection(
                correction.correction_id.to_string(),
            ));
        }

        self.apply_annotation_version(annotation.clone(), Some(correction.previous_version))?;
        self.reviews.push(review.clone());
        self.reviewer_corrections.push(correction.clone());
        self.task_states
            .insert(task_state.task_id.clone(), task_state.clone());
        for assignment in assignments {
            if let Some(existing) = self
                .assignments
                .iter_mut()
                .find(|candidate| candidate.assignment_id == assignment.assignment_id)
            {
                *existing = assignment.clone();
            } else {
                self.assignments.push(assignment.clone());
            }
        }
        Ok(())
    }
}

pub fn rebuild_state(image_id: ImageId, events: &[EventLogEntry]) -> DomainResult<ImageState> {
    let mut state = ImageState::new(image_id);
    for event in events {
        state.apply_event(event)?;
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use crate::{
        AnnotationGeometry, AnnotationSource, AnnotationType, BoundingBox, DatasetRole, UserId, now,
    };

    use super::*;

    #[test]
    fn replays_annotation_versions_at_event_boundaries() {
        let image_id = ImageId::from("img_test");
        let user_id = UserId::from("user_1");
        let annotation_id = AnnotationId::from("ann_1");
        let first = AnnotationVersion {
            annotation_id: annotation_id.clone(),
            version: 1,
            task_id: TaskId::from("bounding_box:person"),
            class_id: crate::ClassId::from("person"),
            annotation_type: AnnotationType::BoundingBox,
            source: AnnotationSource::Human,
            geometry: AnnotationGeometry::BoundingBox(BoundingBox {
                x: 0.1,
                y: 0.1,
                width: 0.2,
                height: 0.2,
            }),
            author_user_id: user_id.clone(),
            created_at: now(),
            updated_at: now(),
            deleted: false,
        };
        let mut second = first.clone();
        second.version = 2;
        second.geometry = AnnotationGeometry::BoundingBox(BoundingBox {
            x: 0.2,
            y: 0.1,
            width: 0.2,
            height: 0.2,
        });
        let events = vec![
            EventLogEntry::new(
                1,
                image_id.clone(),
                user_id.clone(),
                DatasetRole::Annotator,
                now(),
                EventPayload::AnnotationVersionCreated {
                    annotation: first,
                    previous_version: None,
                    reason: None,
                },
            ),
            EventLogEntry::new(
                2,
                image_id.clone(),
                user_id,
                DatasetRole::Annotator,
                now(),
                EventPayload::AnnotationVersionCreated {
                    annotation: second,
                    previous_version: Some(1),
                    reason: Some("move".to_string()),
                },
            ),
        ];
        let state_after_first = rebuild_state(image_id.clone(), &events[..1]).unwrap();
        assert_eq!(
            state_after_first
                .current_annotation(&annotation_id)
                .unwrap()
                .version,
            1
        );
        let mut state_after_second = rebuild_state(image_id, &events).unwrap();
        assert_eq!(
            state_after_second
                .current_annotation(&annotation_id)
                .unwrap()
                .version,
            2
        );

        let stale_delete = EventLogEntry::new(
            3,
            state_after_second.image_id.clone(),
            UserId::from("user_1"),
            DatasetRole::Annotator,
            now(),
            EventPayload::AnnotationDeleted {
                annotation_id,
                version: 1,
                reason: None,
            },
        );
        assert!(state_after_second.apply_event(&stale_delete).is_err());
    }

    #[test]
    fn replays_reviewer_correction_as_one_terminal_rejection() {
        let image_id = ImageId::from("img_correction");
        let task_id = TaskId::from("bounding_box:person");
        let annotation_id = AnnotationId::from("ann_1");
        let annotator = UserId::from("annotator");
        let reviewer = UserId::from("reviewer");
        let correction_id = crate::CorrectionId::from("cor_1");
        let assignment_id = crate::AssignmentId::from("asg_1");
        let timestamp = now();
        let first = AnnotationVersion {
            annotation_id: annotation_id.clone(),
            version: 1,
            task_id: task_id.clone(),
            class_id: crate::ClassId::from("person"),
            annotation_type: AnnotationType::BoundingBox,
            source: AnnotationSource::Human,
            geometry: AnnotationGeometry::BoundingBox(BoundingBox {
                x: 0.1,
                y: 0.1,
                width: 0.2,
                height: 0.2,
            }),
            author_user_id: annotator.clone(),
            created_at: timestamp,
            updated_at: timestamp,
            deleted: false,
        };
        let corrected = AnnotationVersion {
            version: 2,
            source: AnnotationSource::ReviewerCorrection {
                correction_id: correction_id.clone(),
            },
            geometry: AnnotationGeometry::BoundingBox(BoundingBox {
                x: 0.2,
                y: 0.2,
                width: 0.3,
                height: 0.3,
            }),
            author_user_id: reviewer.clone(),
            updated_at: timestamp,
            ..first.clone()
        };
        let assignment = Assignment {
            assignment_id: assignment_id.clone(),
            image_id: image_id.clone(),
            task_id: task_id.clone(),
            assigned_to: reviewer.clone(),
            kind: AssignmentKind::Review,
            status: AssignmentStatus::Completed,
            expires_at: Some(timestamp + std::time::Duration::from_secs(60)),
            created_at: timestamp,
            updated_at: timestamp,
        };
        let correction = ReviewerCorrectionRecord {
            correction_id: correction_id.clone(),
            assignment_id,
            annotation_id: annotation_id.clone(),
            previous_version: 1,
            corrected_version: 2,
            task_id: task_id.clone(),
            reviewer_user_id: reviewer.clone(),
            timestamp,
            reason: Some("box was too small".to_string()),
        };
        let events = vec![
            EventLogEntry::new(
                1,
                image_id.clone(),
                annotator,
                DatasetRole::Annotator,
                timestamp,
                EventPayload::AnnotationVersionCreated {
                    annotation: first,
                    previous_version: None,
                    reason: None,
                },
            ),
            EventLogEntry::new(
                2,
                image_id.clone(),
                reviewer.clone(),
                DatasetRole::Reviewer,
                timestamp,
                EventPayload::ReviewerCorrectionRecorded {
                    correction,
                    annotation: Box::new(corrected),
                    review: ReviewRecord {
                        review_id: crate::ReviewId::from("rev_1"),
                        target: ReviewTarget::AnnotationVersion {
                            annotation_id: annotation_id.clone(),
                            version: 1,
                        },
                        reviewer_user_id: reviewer.clone(),
                        decision: ReviewDecision::Rejected,
                        timestamp,
                        comment: Some("box was too small".to_string()),
                    },
                    task_state: TaskState {
                        task_id: task_id.clone(),
                        status: TaskStatus::Completed,
                        outcome: Some(TaskOutcome::ReviewerCorrected),
                        assigned_to: None,
                        completed_by: Some(reviewer),
                        completed_at: Some(timestamp),
                        updated_at: timestamp,
                    },
                    assignments: vec![assignment],
                },
            ),
        ];

        let state = rebuild_state(image_id, &events).unwrap();

        assert_eq!(state.current_annotation(&annotation_id).unwrap().version, 2);
        assert_eq!(state.reviews[0].decision, ReviewDecision::Rejected);
        assert_eq!(state.reviewer_corrections.len(), 1);
        assert_eq!(
            state.task_states[&task_id].outcome,
            Some(TaskOutcome::ReviewerCorrected)
        );
        assert_eq!(state.assignments[0].status, AssignmentStatus::Completed);
    }
}
