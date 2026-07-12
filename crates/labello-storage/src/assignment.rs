use labello_domain::{
    Actor, Assignment, AssignmentId, AssignmentKind, AssignmentStatus, DatasetRole, EventLogEntry,
    EventPayload, ImageId, ReviewDecision, ReviewRecord, ReviewTarget, ReviewWorkflow, TaskId,
    TaskState, TaskStatus, UserId, require_role,
};

use crate::{DatasetRepository, StorageError, StorageResult};

pub struct AssignmentContext<'a> {
    pub assignment_id: &'a AssignmentId,
    pub image_id: &'a ImageId,
    pub task_id: &'a TaskId,
    pub kind: AssignmentKind,
}

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
        if !task.enabled {
            return Ok(None);
        }
        if task.class_ids.len() != 1 {
            return Err(StorageError::InvalidAssignment(format!(
                "enabled task {task_id} must have exactly one class"
            )));
        }
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
            let lock = self.image_lock(image_id);
            let _guard = lock.lock().await;
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
            let mut payloads = vec![EventPayload::AssignmentUpdated {
                assignment: assignment.clone(),
            }];
            if kind == AssignmentKind::Annotation && status == &TaskStatus::Pending {
                let task_state = TaskState {
                    task_id: task_id.clone(),
                    status: TaskStatus::InProgress,
                    assigned_to: Some(user_id.clone()),
                    completed_by: None,
                    completed_at: None,
                    updated_at: now,
                };
                payloads.push(EventPayload::TaskStateChanged { task_state });
            }
            self.append_payloads_unlocked(image_id, &actor, payloads)
                .await?;
            return Ok(Some(assignment));
        }
        Ok(None)
    }

    pub async fn release_assignment(
        &self,
        user_id: &UserId,
        assignment_id: &AssignmentId,
        image_id: &ImageId,
        task_id: &TaskId,
        kind: AssignmentKind,
    ) -> StorageResult<Assignment> {
        let role = role_for_kind(&kind);
        let metadata = self.load_dataset().await?;
        require_role(
            &metadata.role_assignments,
            &metadata.dataset_id,
            user_id,
            role.clone(),
        )?;
        ensure_assignment_target_exists(&metadata, image_id, task_id)?;

        let lock = self.image_lock(image_id);
        let _guard = lock.lock().await;
        let state = self.load_image_state(image_id).await?;
        let mut assignment = exact_active_assignment(
            &state.assignments,
            assignment_id,
            image_id,
            task_id,
            user_id,
            &kind,
        )?
        .clone();
        let now = labello_domain::now();
        assignment.status = AssignmentStatus::Cancelled;
        assignment.updated_at = now;
        let mut payloads = vec![EventPayload::AssignmentUpdated {
            assignment: assignment.clone(),
        }];
        if kind == AssignmentKind::Annotation
            && state
                .task_states
                .get(task_id)
                .is_some_and(|task_state| task_state.status == TaskStatus::InProgress)
        {
            payloads.push(EventPayload::TaskStateChanged {
                task_state: TaskState {
                    task_id: task_id.clone(),
                    status: TaskStatus::Pending,
                    assigned_to: None,
                    completed_by: None,
                    completed_at: None,
                    updated_at: now,
                },
            });
        }
        self.append_payloads_unlocked(
            image_id,
            &Actor {
                user_id: user_id.clone(),
                role,
            },
            payloads,
        )
        .await?;
        Ok(assignment)
    }

    pub async fn complete_assignment(
        &self,
        user_id: &UserId,
        assignment_id: &AssignmentId,
        image_id: &ImageId,
        task_id: &TaskId,
        kind: AssignmentKind,
    ) -> StorageResult<Assignment> {
        if kind != AssignmentKind::Annotation {
            return Err(StorageError::InvalidAssignment(
                "review and adjudication assignments complete with their records".to_string(),
            ));
        }
        let now = labello_domain::now();
        let task_state = TaskState {
            task_id: task_id.clone(),
            status: TaskStatus::Submitted,
            assigned_to: Some(user_id.clone()),
            completed_by: Some(user_id.clone()),
            completed_at: Some(now),
            updated_at: now,
        };
        let (_, assignment) = self
            .apply_to_assignment(
                user_id,
                AssignmentContext {
                    assignment_id,
                    image_id,
                    task_id,
                    kind,
                },
                vec![EventPayload::TaskStateChanged { task_state }],
                true,
            )
            .await?;
        Ok(assignment)
    }

    pub async fn append_for_assignment(
        &self,
        user_id: &UserId,
        assignment: AssignmentContext<'_>,
        payloads: Vec<EventPayload>,
        complete: bool,
    ) -> StorageResult<(Vec<EventLogEntry>, Assignment)> {
        self.apply_to_assignment(user_id, assignment, payloads, complete)
            .await
    }

    async fn apply_to_assignment(
        &self,
        user_id: &UserId,
        assignment_context: AssignmentContext<'_>,
        mut payloads: Vec<EventPayload>,
        complete: bool,
    ) -> StorageResult<(Vec<EventLogEntry>, Assignment)> {
        let AssignmentContext {
            assignment_id,
            image_id,
            task_id,
            kind,
        } = assignment_context;
        let role = role_for_kind(&kind);
        let metadata = self.load_dataset().await?;
        require_role(
            &metadata.role_assignments,
            &metadata.dataset_id,
            user_id,
            role.clone(),
        )?;
        ensure_assignment_target_exists(&metadata, image_id, task_id)?;

        let lock = self.image_lock(image_id);
        let _guard = lock.lock().await;
        let state = self.load_image_state(image_id).await?;
        let mut assignment = exact_active_assignment(
            &state.assignments,
            assignment_id,
            image_id,
            task_id,
            user_id,
            &kind,
        )?
        .clone();
        let task_status = state
            .task_states
            .get(task_id)
            .map(|task_state| &task_state.status)
            .unwrap_or(&TaskStatus::Pending);
        let valid_task_status = match kind {
            AssignmentKind::Annotation => {
                matches!(
                    task_status,
                    TaskStatus::InProgress | TaskStatus::NeedsCorrection
                )
            }
            AssignmentKind::Review => task_status == &TaskStatus::Submitted,
            AssignmentKind::Adjudication => task_status == &TaskStatus::AdjudicationRequired,
        };
        if !valid_task_status {
            return Err(StorageError::AssignmentConflict(format!(
                "assignment {assignment_id} is not valid for task status {task_status:?}"
            )));
        }
        if complete {
            assignment.status = AssignmentStatus::Completed;
            assignment.updated_at = labello_domain::now();
            payloads.push(EventPayload::AssignmentUpdated {
                assignment: assignment.clone(),
            });
        }
        let events = self
            .append_payloads_unlocked(
                image_id,
                &Actor {
                    user_id: user_id.clone(),
                    role,
                },
                payloads,
            )
            .await?;
        Ok((events, assignment))
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

fn role_for_kind(kind: &AssignmentKind) -> DatasetRole {
    match kind {
        AssignmentKind::Annotation => DatasetRole::Annotator,
        AssignmentKind::Review => DatasetRole::Reviewer,
        AssignmentKind::Adjudication => DatasetRole::Adjudicator,
    }
}

fn ensure_assignment_target_exists(
    metadata: &labello_domain::DatasetMetadata,
    image_id: &ImageId,
    task_id: &TaskId,
) -> StorageResult<()> {
    if !metadata.images.contains_key(image_id) {
        return Err(StorageError::InvalidAssignment(format!(
            "image {image_id} does not belong to dataset {}",
            metadata.dataset_id
        )));
    }
    if metadata.task(task_id).is_none() {
        return Err(StorageError::InvalidAssignment(format!(
            "task {task_id} does not belong to dataset {}",
            metadata.dataset_id
        )));
    }
    Ok(())
}

fn exact_active_assignment<'a>(
    assignments: &'a [Assignment],
    assignment_id: &AssignmentId,
    image_id: &ImageId,
    task_id: &TaskId,
    user_id: &UserId,
    kind: &AssignmentKind,
) -> StorageResult<&'a Assignment> {
    let assignment = assignments
        .iter()
        .find(|assignment| &assignment.assignment_id == assignment_id)
        .ok_or_else(|| {
            StorageError::InvalidAssignment(format!("assignment {assignment_id} does not exist"))
        })?;
    if &assignment.assigned_to != user_id {
        return Err(StorageError::Unauthorized(format!(
            "assignment {assignment_id} belongs to another user"
        )));
    }
    if &assignment.image_id != image_id
        || &assignment.task_id != task_id
        || &assignment.kind != kind
    {
        return Err(StorageError::InvalidAssignment(format!(
            "assignment {assignment_id} does not match the requested image, task, and kind"
        )));
    }
    if assignment.status != AssignmentStatus::Active {
        return Err(StorageError::AssignmentConflict(format!(
            "assignment {assignment_id} is not active"
        )));
    }
    Ok(assignment)
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

        repo.complete_assignment(
            &user_id,
            &first.assignment_id,
            &first.image_id,
            &task_id,
            AssignmentKind::Annotation,
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
    async fn records_do_not_infer_assignment_completion() {
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
                AssignmentStatus::Active
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

        repo.append_for_assignment(
            &reviewers[0],
            AssignmentContext {
                assignment_id: &first.assignment_id,
                image_id: &image_id,
                task_id: &task_id,
                kind: AssignmentKind::Review,
            },
            vec![EventPayload::ReviewRecorded {
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
            }],
            true,
        )
        .await
        .unwrap();
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

        repo.append_for_assignment(
            &reviewers[1],
            AssignmentContext {
                assignment_id: &second.assignment_id,
                image_id: &image_id,
                task_id: &task_id,
                kind: AssignmentKind::Review,
            },
            vec![EventPayload::ReviewRecorded {
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
            }],
            true,
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

    #[tokio::test]
    async fn release_cancels_assignment_and_makes_image_reclaimable() {
        let (_temp, repo, task_id, users) = annotation_repo(1, &["worker"]).await;
        let first = repo
            .assign_next_image(&users[0], &task_id, AssignmentKind::Annotation)
            .await
            .unwrap()
            .unwrap();

        let released = repo
            .release_assignment(
                &users[0],
                &first.assignment_id,
                &first.image_id,
                &task_id,
                AssignmentKind::Annotation,
            )
            .await
            .unwrap();
        assert_eq!(released.status, AssignmentStatus::Cancelled);
        let state = repo.load_image_state(&first.image_id).await.unwrap();
        assert_eq!(state.task_states[&task_id].status, TaskStatus::Pending);

        let reclaimed = repo
            .assign_next_image(&users[0], &task_id, AssignmentKind::Annotation)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reclaimed.image_id, first.image_id);
        assert_ne!(reclaimed.assignment_id, first.assignment_id);
    }

    #[tokio::test]
    async fn release_preserves_needs_correction() {
        let (_temp, repo, task_id, users) = annotation_repo(1, &["worker"]).await;
        let image_id = ImageId::from("img_0");
        let timestamp = now();
        repo.append_payload(
            &image_id,
            &Actor {
                user_id: users[0].clone(),
                role: DatasetRole::Annotator,
            },
            EventPayload::TaskStateChanged {
                task_state: TaskState {
                    task_id: task_id.clone(),
                    status: TaskStatus::NeedsCorrection,
                    assigned_to: None,
                    completed_by: None,
                    completed_at: None,
                    updated_at: timestamp,
                },
            },
        )
        .await
        .unwrap();
        let assignment = repo
            .assign_next_image(&users[0], &task_id, AssignmentKind::Annotation)
            .await
            .unwrap()
            .unwrap();

        repo.release_assignment(
            &users[0],
            &assignment.assignment_id,
            &image_id,
            &task_id,
            AssignmentKind::Annotation,
        )
        .await
        .unwrap();

        assert_eq!(
            repo.load_image_state(&image_id).await.unwrap().task_states[&task_id].status,
            TaskStatus::NeedsCorrection
        );
    }

    #[tokio::test]
    async fn exact_assignment_rejects_the_wrong_user() {
        let (_temp, repo, task_id, users) = annotation_repo(1, &["owner", "other"]).await;
        let assignment = repo
            .assign_next_image(&users[0], &task_id, AssignmentKind::Annotation)
            .await
            .unwrap()
            .unwrap();

        let error = repo
            .complete_assignment(
                &users[1],
                &assignment.assignment_id,
                &assignment.image_id,
                &task_id,
                AssignmentKind::Annotation,
            )
            .await
            .unwrap_err();
        assert!(matches!(error, StorageError::Unauthorized(_)));
        assert_eq!(
            repo.load_image_state(&assignment.image_id)
                .await
                .unwrap()
                .assignments[0]
                .status,
            AssignmentStatus::Active
        );
    }

    #[tokio::test]
    async fn concurrent_claims_cannot_take_the_same_annotation_work() {
        let (_temp, repo, task_id, users) = annotation_repo(1, &["first", "second"]).await;
        let first_repo = repo.clone();
        let second_repo = repo.clone();
        let first_task = task_id.clone();
        let second_task = task_id.clone();
        let first_user = users[0].clone();
        let second_user = users[1].clone();

        let (first, second) = tokio::join!(
            async move {
                first_repo
                    .assign_next_image(&first_user, &first_task, AssignmentKind::Annotation)
                    .await
                    .unwrap()
            },
            async move {
                second_repo
                    .assign_next_image(&second_user, &second_task, AssignmentKind::Annotation)
                    .await
                    .unwrap()
            }
        );

        assert_eq!(
            usize::from(first.is_some()) + usize::from(second.is_some()),
            1
        );
    }

    #[tokio::test]
    async fn claim_rejects_enabled_tasks_without_exactly_one_class() {
        let (_temp, repo, task_id, users) = annotation_repo(1, &["worker"]).await;
        let mut metadata = repo.load_dataset_config().await.unwrap();
        metadata.tasks[0].class_ids.push(ClassId::from("second"));
        repo.save_dataset(&metadata).await.unwrap();

        let error = repo
            .assign_next_image(&users[0], &task_id, AssignmentKind::Annotation)
            .await
            .unwrap_err();
        assert!(matches!(error, StorageError::InvalidAssignment(_)));
    }

    async fn annotation_repo(
        image_count: usize,
        user_names: &[&str],
    ) -> (tempfile::TempDir, DatasetRepository, TaskId, Vec<UserId>) {
        let temp = tempfile::tempdir().unwrap();
        let repo = DatasetRepository::new(temp.path());
        let task_id = TaskId::from("bounding_box:person");
        let class_id = ClassId::from("person");
        let users = user_names
            .iter()
            .map(|user| UserId::from(*user))
            .collect::<Vec<_>>();
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
        metadata
            .role_assignments
            .extend(users.iter().map(|user_id| DatasetRoleAssignment {
                dataset_id: metadata.dataset_id.clone(),
                user_id: user_id.clone(),
                roles: BTreeSet::from([DatasetRole::Annotator]),
                assigned_at: now(),
                assigned_by: None,
            }));
        repo.initialize(metadata).await.unwrap();
        repo.save_images_index(&ImagesIndex {
            schema_version: SCHEMA_VERSION,
            image_count,
            images_by_hash: (0..image_count)
                .map(|index| {
                    let image_id = ImageId::from(format!("img_{index}"));
                    (
                        format!("hash_{index}"),
                        ImageRecord {
                            image_id,
                            blake3: format!("hash_{index}"),
                            canonical_path: format!("images/{index}.png"),
                            known_paths: vec![format!("images/{index}.png")],
                            duplicate_paths: Vec::new(),
                            file_name: format!("{index}.png"),
                            byte_size: 4,
                            width: 2,
                            height: 2,
                            media_type: "image/png".to_string(),
                        },
                    )
                })
                .collect(),
        })
        .await
        .unwrap();
        (temp, repo, task_id, users)
    }
}
