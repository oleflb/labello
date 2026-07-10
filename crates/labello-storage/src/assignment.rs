use labello_domain::{
    Actor, Assignment, AssignmentId, AssignmentKind, AssignmentStatus, DatasetRole, EventPayload,
    ImageId, ReviewDecision, ReviewRecord, ReviewTarget, ReviewWorkflow, TaskId, TaskState,
    TaskStatus, UserId, require_role,
};

use crate::{DatasetRepository, StorageError, StorageResult};

impl DatasetRepository {
    pub async fn assign_next_image(
        &self,
        user_id: &UserId,
        task_id: &TaskId,
        kind: AssignmentKind,
    ) -> StorageResult<Option<Assignment>> {
        let metadata = self.load_dataset().await?;
        let required_role = match kind {
            AssignmentKind::Annotation => DatasetRole::Annotator,
            AssignmentKind::Review => DatasetRole::Reviewer,
            AssignmentKind::Adjudication => DatasetRole::Adjudicator,
        };
        require_role(
            &metadata.role_assignments,
            &metadata.dataset_id,
            user_id,
            required_role.clone(),
        )?;
        let task = metadata
            .task(task_id)
            .ok_or_else(|| StorageError::Unauthorized(format!("task {task_id} does not exist")))?;
        if kind == AssignmentKind::Review && task.review.workflow == ReviewWorkflow::None {
            return Ok(None);
        }
        if metadata
            .imbalance
            .as_ref()
            .is_some_and(|config| config.enforce)
            && self.task_is_overrepresented(task_id).await?
        {
            return Ok(None);
        }

        for image_id in metadata.images.keys() {
            let state = self.load_image_state(image_id).await?;
            if kind == AssignmentKind::Review
                && (has_task_review_by_user(&state.reviews, task_id, user_id)
                    || task_approval_count(&state.reviews, task_id) >= task.review.required_reviews)
            {
                continue;
            }
            if let Some(assignment) =
                active_assignment_for_user(&state.assignments, task_id, user_id, &kind)
            {
                return Ok(Some(assignment.clone()));
            }
            if has_conflicting_assignment(&state.assignments, task_id, user_id, &kind) {
                continue;
            }
            let status = state
                .task_states
                .get(task_id)
                .map(|state| &state.status)
                .unwrap_or(&TaskStatus::Pending);
            if !status_matches_kind(status, &kind) {
                continue;
            }
            let now = labello_domain::now();
            let assignment = Assignment {
                assignment_id: AssignmentId::generate(),
                image_id: image_id.clone(),
                task_id: task.task_id.clone(),
                assigned_to: user_id.clone(),
                kind: kind.clone(),
                status: AssignmentStatus::Active,
                created_at: now,
                updated_at: now,
            };
            let actor = Actor {
                user_id: user_id.clone(),
                role: required_role,
            };
            self.append_payload(
                image_id,
                &actor,
                EventPayload::AssignmentUpdated {
                    assignment: assignment.clone(),
                },
            )
            .await?;
            if kind == AssignmentKind::Annotation {
                let task_state = TaskState {
                    task_id: task_id.clone(),
                    status: TaskStatus::InProgress,
                    assigned_to: Some(user_id.clone()),
                    completed_by: None,
                    completed_at: None,
                    updated_at: now,
                };
                self.append_payload(
                    image_id,
                    &actor,
                    EventPayload::TaskStateChanged { task_state },
                )
                .await?;
            }
            return Ok(Some(assignment));
        }
        Ok(None)
    }

    async fn task_is_overrepresented(&self, selected_task_id: &TaskId) -> StorageResult<bool> {
        let metadata = self.load_dataset().await?;
        let Some(config) = metadata.imbalance.as_ref() else {
            return Ok(false);
        };
        let stats = self.dataset_stats().await?;
        let selected = stats
            .per_task
            .get(selected_task_id)
            .map(|task| task.completed)
            .unwrap_or_default();
        let min_other = metadata
            .tasks
            .iter()
            .filter(|task| &task.task_id != selected_task_id)
            .map(|task| {
                stats
                    .per_task
                    .get(&task.task_id)
                    .map(|stats| stats.completed)
                    .unwrap_or_default()
            })
            .min()
            .unwrap_or(0);
        if min_other == 0 {
            Ok(selected > 0 && config.max_ratio <= 1.0)
        } else {
            Ok((selected as f32 / min_other as f32) > config.max_ratio)
        }
    }
}

fn active_assignment_for_user<'a>(
    assignments: &'a [Assignment],
    task_id: &TaskId,
    user_id: &UserId,
    kind: &AssignmentKind,
) -> Option<&'a Assignment> {
    assignments.iter().find(|assignment| {
        &assignment.task_id == task_id
            && &assignment.kind == kind
            && assignment.status == AssignmentStatus::Active
            && &assignment.assigned_to == user_id
    })
}

fn status_matches_kind(status: &TaskStatus, kind: &AssignmentKind) -> bool {
    match kind {
        AssignmentKind::Annotation => {
            matches!(status, TaskStatus::Pending | TaskStatus::NeedsCorrection)
        }
        AssignmentKind::Review => matches!(status, TaskStatus::Submitted),
        AssignmentKind::Adjudication => matches!(status, TaskStatus::AdjudicationRequired),
    }
}

fn has_conflicting_assignment(
    assignments: &[Assignment],
    task_id: &TaskId,
    user_id: &UserId,
    kind: &AssignmentKind,
) -> bool {
    if *kind == AssignmentKind::Review {
        return false;
    }
    assignments.iter().any(|assignment| {
        &assignment.task_id == task_id
            && &assignment.kind == kind
            && assignment.status == AssignmentStatus::Active
            && &assignment.assigned_to != user_id
    })
}

fn has_task_review_by_user(reviews: &[ReviewRecord], task_id: &TaskId, user_id: &UserId) -> bool {
    reviews.iter().any(|review| {
        review.reviewer_user_id == *user_id
            && matches!(
                &review.target,
                ReviewTarget::Task {
                    task_id: reviewed_task_id
                } if reviewed_task_id == task_id
            )
    })
}

fn task_approval_count(reviews: &[ReviewRecord], task_id: &TaskId) -> u32 {
    reviews
        .iter()
        .filter_map(|review| match (&review.target, &review.decision) {
            (ReviewTarget::Task { task_id: reviewed }, ReviewDecision::Approved)
                if reviewed == task_id =>
            {
                Some(&review.reviewer_user_id)
            }
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>()
        .len() as u32
}

#[allow(dead_code)]
fn _image_id_type_is_used(_: &ImageId) {}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use labello_domain::{
        AdjudicationDecision, AdjudicationId, AdjudicationRecord, AnnotationType, ClassId,
        DatasetId, DatasetMetadata, DatasetRoleAssignment, ImageRecord, ImagesIndex, LabelClass,
        ReviewConfig, ReviewDecision, ReviewId, ReviewRecord, ReviewTarget, ReviewWorkflow,
        SCHEMA_VERSION, TaskDefinition, TutorialContent, now,
    };

    use super::*;

    #[tokio::test]
    async fn retries_return_same_users_active_assignment() {
        let temp = tempfile::tempdir().unwrap();
        let repo = DatasetRepository::new(temp.path());
        let user_id = UserId::from("annotator");
        let task_id = TaskId::from("bounding_box:person");
        let class_id = ClassId::from("person");
        let mut metadata = DatasetMetadata::new(DatasetId::from("ds"), "Dataset", now());
        metadata.label_classes.push(LabelClass {
            class_id: class_id.clone(),
            name: "Person".to_string(),
            color: "#5eead4".to_string(),
            description: None,
        });
        metadata.tasks.push(TaskDefinition {
            task_id: task_id.clone(),
            name: "Person boxes".to_string(),
            annotation_type: AnnotationType::BoundingBox,
            class_ids: vec![class_id],
            instructions: TutorialContent {
                title: "Instructions".to_string(),
                example_text: "Draw boxes.".to_string(),
                example_images: Vec::new(),
            },
            skeleton: None,
            review: ReviewConfig::default(),
            prelabel_config_ids: Vec::new(),
            enabled: true,
        });
        metadata.role_assignments.push(DatasetRoleAssignment {
            dataset_id: metadata.dataset_id.clone(),
            user_id: user_id.clone(),
            roles: BTreeSet::from([DatasetRole::Annotator]),
            assigned_at: now(),
            assigned_by: None,
        });
        repo.initialize(metadata).await.unwrap();
        let image = ImageRecord {
            image_id: ImageId::from("img_1"),
            blake3: "hash".to_string(),
            canonical_path: "images/one.png".to_string(),
            known_paths: vec!["images/one.png".to_string()],
            duplicate_paths: Vec::new(),
            file_name: "one.png".to_string(),
            byte_size: 4,
            width: 2,
            height: 2,
            media_type: "image/png".to_string(),
        };
        let second_image = ImageRecord {
            image_id: ImageId::from("img_2"),
            blake3: "hash2".to_string(),
            canonical_path: "images/two.png".to_string(),
            known_paths: vec!["images/two.png".to_string()],
            duplicate_paths: Vec::new(),
            file_name: "two.png".to_string(),
            byte_size: 4,
            width: 2,
            height: 2,
            media_type: "image/png".to_string(),
        };
        repo.save_images_index(&ImagesIndex {
            schema_version: SCHEMA_VERSION,
            image_count: 2,
            images_by_hash: BTreeMap::from([
                ("hash".to_string(), image),
                ("hash2".to_string(), second_image),
            ]),
        })
        .await
        .unwrap();

        let first = repo
            .assign_next_image(&user_id, &task_id, AssignmentKind::Annotation)
            .await
            .unwrap()
            .unwrap();
        let retry = repo
            .assign_next_image(&user_id, &task_id, AssignmentKind::Annotation)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(retry.assignment_id, first.assignment_id);
        assert_eq!(retry.image_id, first.image_id);

        let timestamp = now();
        repo.append_payload(
            &first.image_id,
            &Actor {
                user_id: user_id.clone(),
                role: DatasetRole::Annotator,
            },
            EventPayload::TaskStateChanged {
                task_state: TaskState {
                    task_id: task_id.clone(),
                    status: TaskStatus::Submitted,
                    assigned_to: Some(user_id.clone()),
                    completed_by: Some(user_id.clone()),
                    completed_at: Some(timestamp),
                    updated_at: timestamp,
                },
            },
        )
        .await
        .unwrap();

        let next = repo
            .assign_next_image(&user_id, &task_id, AssignmentKind::Annotation)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(next.image_id, ImageId::from("img_2"));
        let first_state = repo.load_image_state(&first.image_id).await.unwrap();
        assert_eq!(
            first_state
                .assignments
                .iter()
                .find(|assignment| assignment.assignment_id == first.assignment_id)
                .unwrap()
                .status,
            AssignmentStatus::Completed
        );
    }

    #[tokio::test]
    async fn review_and_adjudication_complete_active_assignments() {
        let temp = tempfile::tempdir().unwrap();
        let repo = DatasetRepository::new(temp.path());
        repo.initialize(DatasetMetadata::new(
            DatasetId::from("ds"),
            "Dataset",
            now(),
        ))
        .await
        .unwrap();
        let image_id = ImageId::from("img_1");
        let user_id = UserId::from("worker");
        let task_id = TaskId::from("bounding_box:person");

        for (kind, role) in [
            (AssignmentKind::Review, DatasetRole::Reviewer),
            (AssignmentKind::Adjudication, DatasetRole::Adjudicator),
        ] {
            let actor = Actor {
                user_id: user_id.clone(),
                role,
            };
            let assignment = Assignment {
                assignment_id: AssignmentId::generate(),
                image_id: image_id.clone(),
                task_id: task_id.clone(),
                assigned_to: user_id.clone(),
                kind: kind.clone(),
                status: AssignmentStatus::Active,
                created_at: now(),
                updated_at: now(),
            };
            repo.append_payload(
                &image_id,
                &actor,
                EventPayload::AssignmentUpdated {
                    assignment: assignment.clone(),
                },
            )
            .await
            .unwrap();
            let payload = match kind {
                AssignmentKind::Review => EventPayload::ReviewRecorded {
                    review: ReviewRecord {
                        review_id: ReviewId::generate(),
                        target: ReviewTarget::Task {
                            task_id: task_id.clone(),
                        },
                        reviewer_user_id: user_id.clone(),
                        decision: ReviewDecision::Approved,
                        timestamp: now(),
                        comment: None,
                    },
                },
                AssignmentKind::Adjudication => EventPayload::AdjudicationRecorded {
                    adjudication: AdjudicationRecord {
                        adjudication_id: AdjudicationId::generate(),
                        task_id: task_id.clone(),
                        annotation_ids: Vec::new(),
                        adjudicator_user_id: user_id.clone(),
                        decision: AdjudicationDecision::AcceptAnnotation,
                        resolution: "accepted".to_string(),
                        timestamp: now(),
                    },
                },
                AssignmentKind::Annotation => unreachable!(),
            };
            repo.append_payload(&image_id, &actor, payload)
                .await
                .unwrap();

            let state = repo.load_image_state(&image_id).await.unwrap();
            assert_eq!(
                state
                    .assignments
                    .iter()
                    .find(|candidate| candidate.assignment_id == assignment.assignment_id)
                    .unwrap()
                    .status,
                AssignmentStatus::Completed
            );
        }
    }

    #[tokio::test]
    async fn review_assignment_skips_users_with_final_reviews_until_threshold() {
        let temp = tempfile::tempdir().unwrap();
        let repo = DatasetRepository::new(temp.path());
        let task_id = TaskId::from("bounding_box:person");
        let class_id = ClassId::from("person");
        let reviewers = [
            UserId::from("reviewer_1"),
            UserId::from("reviewer_2"),
            UserId::from("reviewer_3"),
        ];
        let mut metadata = DatasetMetadata::new(DatasetId::from("ds"), "Dataset", now());
        metadata.label_classes.push(LabelClass {
            class_id: class_id.clone(),
            name: "Person".to_string(),
            color: "#5eead4".to_string(),
            description: None,
        });
        metadata.tasks.push(TaskDefinition {
            task_id: task_id.clone(),
            name: "Person boxes".to_string(),
            annotation_type: AnnotationType::BoundingBox,
            class_ids: vec![class_id],
            instructions: TutorialContent {
                title: "Instructions".to_string(),
                example_text: "Draw boxes.".to_string(),
                example_images: Vec::new(),
            },
            skeleton: None,
            review: ReviewConfig {
                required_reviews: 2,
                workflow: ReviewWorkflow::Approval,
                allow_reviewer_corrections: false,
                agreement_threshold: None,
            },
            prelabel_config_ids: Vec::new(),
            enabled: true,
        });
        metadata
            .role_assignments
            .extend(reviewers.iter().map(|user_id| DatasetRoleAssignment {
                dataset_id: metadata.dataset_id.clone(),
                user_id: user_id.clone(),
                roles: BTreeSet::from([DatasetRole::Reviewer]),
                assigned_at: now(),
                assigned_by: None,
            }));
        repo.initialize(metadata).await.unwrap();
        let image_id = ImageId::from("img_1");
        repo.save_images_index(&ImagesIndex {
            schema_version: SCHEMA_VERSION,
            image_count: 1,
            images_by_hash: BTreeMap::from([(
                "hash".to_string(),
                ImageRecord {
                    image_id: image_id.clone(),
                    blake3: "hash".to_string(),
                    canonical_path: "images/one.png".to_string(),
                    known_paths: vec!["images/one.png".to_string()],
                    duplicate_paths: Vec::new(),
                    file_name: "one.png".to_string(),
                    byte_size: 4,
                    width: 2,
                    height: 2,
                    media_type: "image/png".to_string(),
                },
            )]),
        })
        .await
        .unwrap();
        let timestamp = now();
        repo.append_payload(
            &image_id,
            &Actor {
                user_id: UserId::from("annotator"),
                role: DatasetRole::Annotator,
            },
            EventPayload::TaskStateChanged {
                task_state: TaskState {
                    task_id: task_id.clone(),
                    status: TaskStatus::Submitted,
                    assigned_to: None,
                    completed_by: Some(UserId::from("annotator")),
                    completed_at: Some(timestamp),
                    updated_at: timestamp,
                },
            },
        )
        .await
        .unwrap();

        let first = repo
            .assign_next_image(&reviewers[0], &task_id, AssignmentKind::Review)
            .await
            .unwrap()
            .unwrap();
        let second = repo
            .assign_next_image(&reviewers[1], &task_id, AssignmentKind::Review)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.image_id, image_id);
        assert_eq!(second.image_id, image_id);

        let first_actor = Actor {
            user_id: reviewers[0].clone(),
            role: DatasetRole::Reviewer,
        };
        repo.append_payload(
            &image_id,
            &first_actor,
            EventPayload::ReviewRecorded {
                review: ReviewRecord {
                    review_id: ReviewId::generate(),
                    target: ReviewTarget::AnnotationVersion {
                        annotation_id: labello_domain::AnnotationId::from("ann_1"),
                        version: 1,
                    },
                    reviewer_user_id: reviewers[0].clone(),
                    decision: ReviewDecision::Approved,
                    timestamp: now(),
                    comment: None,
                },
            },
        )
        .await
        .unwrap();
        let object_review_retry = repo
            .assign_next_image(&reviewers[0], &task_id, AssignmentKind::Review)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(object_review_retry.assignment_id, first.assignment_id);

        for _ in 0..2 {
            repo.append_payload(
                &image_id,
                &first_actor,
                EventPayload::ReviewRecorded {
                    review: ReviewRecord {
                        review_id: ReviewId::generate(),
                        target: ReviewTarget::Task {
                            task_id: task_id.clone(),
                        },
                        reviewer_user_id: reviewers[0].clone(),
                        decision: ReviewDecision::Approved,
                        timestamp: now(),
                        comment: None,
                    },
                },
            )
            .await
            .unwrap();
        }
        assert!(
            repo.assign_next_image(&reviewers[0], &task_id, AssignmentKind::Review)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            repo.assign_next_image(&reviewers[1], &task_id, AssignmentKind::Review)
                .await
                .unwrap()
                .unwrap()
                .assignment_id,
            second.assignment_id
        );

        let second_actor = Actor {
            user_id: reviewers[1].clone(),
            role: DatasetRole::Reviewer,
        };
        repo.append_payload(
            &image_id,
            &second_actor,
            EventPayload::ReviewRecorded {
                review: ReviewRecord {
                    review_id: ReviewId::generate(),
                    target: ReviewTarget::Task {
                        task_id: task_id.clone(),
                    },
                    reviewer_user_id: reviewers[1].clone(),
                    decision: ReviewDecision::Approved,
                    timestamp: now(),
                    comment: None,
                },
            },
        )
        .await
        .unwrap();
        assert!(
            repo.assign_next_image(&reviewers[2], &task_id, AssignmentKind::Review)
                .await
                .unwrap()
                .is_none()
        );
    }
}
