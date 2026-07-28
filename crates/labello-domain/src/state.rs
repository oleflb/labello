use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    AdjudicationRecord, AnnotationId, AnnotationOrigin, AnnotationVersion, Assignment,
    AssignmentKind, AssignmentStatus, DomainError, DomainResult, EventLogEntry, EventPayload,
    HumanRevisionKind, ImageId, ImportCoverage, ImportId, MigrationConfirmation,
    MigrationDependencyKind, MigrationDependencyMarker, MigrationDisposition,
    MigrationDispositionStatus, MigrationHashContext, MigrationHashStateTarget, MigrationPass,
    MigrationPassId, MigrationTargetSetInitialization, ObjectGroupId, ReviewDecision, ReviewRecord,
    ReviewTarget, ReviewerCorrectionRecord, RevisionSource, SCHEMA_VERSION, TaskId, TaskOutcome,
    TaskState, TaskStatus, migration_confirmation_hash, migration_state_hash,
    migration_target_set_hash,
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
    #[serde(default)]
    pub import_ids: BTreeSet<ImportId>,
    #[serde(default)]
    pub import_coverage: BTreeMap<TaskId, ImportCoverage>,
    #[serde(default)]
    pub included_import_tasks: BTreeSet<TaskId>,
    #[serde(default)]
    pub migration_target_sets: BTreeMap<TaskId, MigrationTargetSetInitialization>,
    #[serde(default)]
    pub migration_dispositions: BTreeMap<TaskId, BTreeMap<ObjectGroupId, MigrationDisposition>>,
    #[serde(default)]
    pub migration_dependencies:
        BTreeMap<TaskId, BTreeMap<ObjectGroupId, MigrationDependencyMarker>>,
    #[serde(default)]
    pub migration_passes: BTreeMap<MigrationPassId, MigrationPass>,
    #[serde(default)]
    pub migration_confirmations: BTreeMap<TaskId, MigrationConfirmation>,
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
            import_ids: BTreeSet::new(),
            import_coverage: BTreeMap::new(),
            included_import_tasks: BTreeSet::new(),
            migration_target_sets: BTreeMap::new(),
            migration_dispositions: BTreeMap::new(),
            migration_dependencies: BTreeMap::new(),
            migration_passes: BTreeMap::new(),
            migration_confirmations: BTreeMap::new(),
        }
    }
}

mod annotation_replay;
mod migration_replay;
mod query;
mod replay;

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
        AnnotationGeometry, AnnotationOrigin, AnnotationType, BoundingBox, DatasetRole,
        HumanRevisionKind, RevisionSource, UserId, now,
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
            object_group_id: None,
            origin: AnnotationOrigin::native(),
            task_id: TaskId::from("bounding_box:person"),
            class_id: crate::ClassId::from("person"),
            annotation_type: AnnotationType::BoundingBox,
            revision_source: RevisionSource::Human {
                action: HumanRevisionKind::Authored,
            },
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
            object_group_id: None,
            origin: AnnotationOrigin::native(),
            task_id: task_id.clone(),
            class_id: crate::ClassId::from("person"),
            annotation_type: AnnotationType::BoundingBox,
            revision_source: RevisionSource::Human {
                action: HumanRevisionKind::Authored,
            },
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
            revision_source: RevisionSource::ReviewerCorrection {
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
