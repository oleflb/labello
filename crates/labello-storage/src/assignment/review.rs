use super::*;

impl DatasetRepository {
    pub async fn current_task_reviews(
        &self,
        image_id: &ImageId,
        task_id: &TaskId,
    ) -> StorageResult<Vec<ReviewRecord>> {
        let events = self.load_events(image_id).await?;
        Ok(current_task_reviews_from_events(&events, task_id))
    }

    pub async fn record_review_for_assignment(
        &self,
        user_id: &UserId,
        assignment_context: AssignmentContext<'_>,
        review: ReviewRecord,
    ) -> StorageResult<labello_domain::ImageState> {
        let AssignmentContext {
            assignment_id,
            image_id,
            task_id,
            kind,
        } = assignment_context;
        if kind != AssignmentKind::Review {
            return Err(StorageError::InvalidAssignment(
                "reviews require a review assignment".to_string(),
            ));
        }
        if review.reviewer_user_id != *user_id {
            return Err(StorageError::Unauthorized(
                "cannot record reviews for another user".to_string(),
            ));
        }
        let metadata = self.load_dataset().await?;
        require_role(
            &metadata.role_assignments,
            &metadata.dataset_id,
            user_id,
            DatasetRole::Reviewer,
        )?;
        ensure_assignment_target_exists(&metadata, image_id, task_id)?;
        let task = metadata.task(task_id).ok_or_else(|| {
            StorageError::InvalidAssignment(format!("task {task_id} does not exist"))
        })?;
        if task.manual_box_guide_migration.is_some() {
            return Err(StorageError::InvalidAssignment(
                "manual migration reviews require the migration review workflow".to_string(),
            ));
        }
        if task.review.workflow != ReviewWorkflow::Approval {
            return Err(StorageError::InvalidAssignment(format!(
                "approval reviews are not enabled for task {task_id}"
            )));
        }
        let lock = self.image_lock(image_id);
        let _guard = lock.lock().await;
        let state = self.load_image_state(image_id).await?;
        let now = labello_domain::now();
        let mut assignment = exact_active_assignment(
            &state.assignments,
            assignment_id,
            image_id,
            task_id,
            user_id,
            &AssignmentKind::Review,
            now,
        )?
        .clone();
        if state
            .task_states
            .get(task_id)
            .map(|task_state| &task_state.status)
            != Some(&TaskStatus::Submitted)
        {
            return Err(StorageError::AssignmentConflict(format!(
                "task {task_id} is no longer eligible for review"
            )));
        }

        let complete = match &review.target {
            ReviewTarget::Image {
                image_id: reviewed_image_id,
            } => {
                if reviewed_image_id != image_id {
                    return Err(StorageError::InvalidAssignment(
                        "review target image does not match assignment image".to_string(),
                    ));
                }
                false
            }
            ReviewTarget::Task {
                task_id: reviewed_task_id,
            } => {
                if reviewed_task_id != task_id {
                    return Err(StorageError::InvalidAssignment(
                        "review target task does not match assignment task".to_string(),
                    ));
                }
                true
            }
            ReviewTarget::AnnotationVersion {
                annotation_id,
                version,
            } => {
                let annotation = state
                    .annotations
                    .get(annotation_id)
                    .and_then(|versions| {
                        versions
                            .iter()
                            .find(|candidate| candidate.version == *version)
                    })
                    .ok_or_else(|| {
                        StorageError::InvalidAssignment(format!(
                            "annotation {annotation_id} version {version} does not exist"
                        ))
                    })?;
                if annotation.task_id != *task_id {
                    return Err(StorageError::InvalidAssignment(
                        "review target task does not match assignment task".to_string(),
                    ));
                }
                false
            }
            ReviewTarget::MigrationDisposition {
                task_id: reviewed_task_id,
                object_group_id,
                disposition_version,
            } => {
                let disposition = state
                    .migration_dispositions
                    .get(reviewed_task_id)
                    .and_then(|dispositions| dispositions.get(object_group_id));
                if reviewed_task_id != task_id
                    || disposition.map(|value| value.disposition_version)
                        != Some(*disposition_version)
                {
                    return Err(StorageError::InvalidAssignment(
                        "migration disposition review target is stale or belongs to another task"
                            .to_string(),
                    ));
                }
                false
            }
            ReviewTarget::MigrationConfirmation {
                task_id: reviewed_task_id,
                confirmation_hash,
            } => {
                let confirmation = state.migration_confirmations.get(reviewed_task_id);
                if reviewed_task_id != task_id
                    || confirmation.map(|value| &value.confirmation_hash) != Some(confirmation_hash)
                {
                    return Err(StorageError::InvalidAssignment(
                        "migration confirmation review target is stale or belongs to another task"
                            .to_string(),
                    ));
                }
                false
            }
        };

        let mut payloads = vec![EventPayload::ReviewRecorded {
            review: review.clone(),
        }];
        if complete {
            let events = self.load_events(image_id).await?;
            let current_reviews = current_task_reviews_from_events(&events, task_id);
            if has_task_review_by_user(&current_reviews, task_id, user_id) {
                return Err(StorageError::AssignmentConflict(format!(
                    "user {user_id} already reviewed task {task_id} in this round"
                )));
            }
            let approval_count = task_approval_count(&current_reviews, task_id)
                + u32::from(review.decision == ReviewDecision::Approved);
            let status = match review.decision {
                ReviewDecision::Approved if approval_count >= task.review.required_reviews => {
                    TaskStatus::Completed
                }
                ReviewDecision::Approved => TaskStatus::Submitted,
                ReviewDecision::Rejected => TaskStatus::NeedsCorrection,
            };
            if status != TaskStatus::Submitted {
                payloads.push(EventPayload::TaskStateChanged {
                    task_state: TaskState {
                        task_id: task_id.clone(),
                        outcome: (review.decision == ReviewDecision::Approved)
                            .then_some(TaskOutcome::Approved),
                        status,
                        assigned_to: None,
                        completed_by: Some(user_id.clone()),
                        completed_at: Some(now),
                        updated_at: now,
                    },
                });
            }
            assignment.status = AssignmentStatus::Completed;
            assignment.updated_at = now;
        } else {
            renew_assignment(&mut assignment, now);
        }
        payloads.push(EventPayload::AssignmentUpdated { assignment });
        let (_, state) = self
            .append_payloads_with_state_unlocked(
                image_id,
                &Actor {
                    user_id: user_id.clone(),
                    role: DatasetRole::Reviewer,
                },
                payloads,
            )
            .await?;
        Ok(state)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn correct_review_annotation(
        &self,
        user_id: &UserId,
        assignment_context: AssignmentContext<'_>,
        correction_id: &CorrectionId,
        annotation_id: &AnnotationId,
        expected_version: u32,
        geometry: AnnotationGeometry,
        reason: Option<String>,
    ) -> StorageResult<EventLogEntry> {
        let AssignmentContext {
            assignment_id,
            image_id,
            task_id,
            kind,
        } = assignment_context;
        if kind != AssignmentKind::Review {
            return Err(StorageError::InvalidCorrection(
                "a reviewer correction requires a review assignment".to_string(),
            ));
        }
        let metadata = self.load_dataset().await?;
        require_role(
            &metadata.role_assignments,
            &metadata.dataset_id,
            user_id,
            DatasetRole::Reviewer,
        )?;
        ensure_assignment_target_exists(&metadata, image_id, task_id)?;
        let task = metadata.task(task_id).ok_or_else(|| {
            StorageError::InvalidCorrection(format!("task {task_id} does not exist"))
        })?;
        let image = metadata.images.get(image_id).ok_or_else(|| {
            StorageError::InvalidCorrection(format!("image {image_id} does not exist"))
        })?;

        let lock = self.image_lock(image_id);
        let _guard = lock.lock().await;
        let state = self.load_image_state(image_id).await?;

        if let Some(existing) = self.load_events(image_id).await?.into_iter().find(|event| {
            matches!(
                &event.payload,
                EventPayload::ReviewerCorrectionRecorded { correction, .. }
                    if &correction.correction_id == correction_id
            )
        }) {
            let matches_request = matches!(
                &existing.payload,
                EventPayload::ReviewerCorrectionRecorded {
                    correction,
                    annotation,
                    ..
                } if correction.assignment_id == *assignment_id
                    && correction.annotation_id == *annotation_id
                    && correction.previous_version == expected_version
                    && correction.task_id == *task_id
                    && correction.reviewer_user_id == *user_id
                    && correction.reason == reason
                    && annotation.geometry == geometry
            );
            return if matches_request {
                Ok(existing)
            } else {
                Err(StorageError::AssignmentConflict(format!(
                    "correction {correction_id} was already used for a different request"
                )))
            };
        }

        if task.review.workflow != ReviewWorkflow::Approval {
            return Err(StorageError::InvalidCorrection(
                "reviewer corrections require the approval workflow".to_string(),
            ));
        }
        if !task.review.allow_reviewer_corrections {
            return Err(StorageError::InvalidCorrection(
                "reviewer corrections are disabled for this task".to_string(),
            ));
        }

        let now = labello_domain::now();
        let current_assignment = exact_active_assignment(
            &state.assignments,
            assignment_id,
            image_id,
            task_id,
            user_id,
            &AssignmentKind::Review,
            now,
        )?;
        if state
            .task_states
            .get(task_id)
            .map(|task_state| &task_state.status)
            != Some(&TaskStatus::Submitted)
        {
            return Err(StorageError::AssignmentConflict(format!(
                "task {task_id} is no longer eligible for review correction"
            )));
        }
        let current = state.current_annotation(annotation_id).ok_or_else(|| {
            StorageError::InvalidCorrection(format!("annotation {annotation_id} does not exist"))
        })?;
        if current.task_id != *task_id {
            return Err(StorageError::InvalidCorrection(
                "annotation task does not match the review assignment".to_string(),
            ));
        }
        if current.deleted {
            return Err(StorageError::InvalidCorrection(format!(
                "annotation {annotation_id} is not active"
            )));
        }
        if current.version != expected_version {
            return Err(StorageError::AssignmentConflict(format!(
                "annotation {annotation_id} is at version {}, expected {expected_version}",
                current.version
            )));
        }
        if current.geometry == geometry {
            return Err(StorageError::InvalidCorrection(
                "corrected geometry must differ from the active annotation".to_string(),
            ));
        }
        if !matches!(
            (&current.annotation_type, &geometry),
            (
                labello_domain::AnnotationType::BoundingBox,
                AnnotationGeometry::BoundingBox(_)
            ) | (
                labello_domain::AnnotationType::Skeleton,
                AnnotationGeometry::Skeleton(_)
            )
        ) {
            return Err(StorageError::InvalidCorrection(
                "corrected geometry type does not match the active annotation".to_string(),
            ));
        }

        let annotation = labello_domain::AnnotationVersion {
            annotation_id: current.annotation_id.clone(),
            version: current.version + 1,
            object_group_id: current.object_group_id.clone(),
            origin: current.origin.clone(),
            task_id: current.task_id.clone(),
            class_id: current.class_id.clone(),
            annotation_type: current.annotation_type.clone(),
            revision_source: RevisionSource::ReviewerCorrection {
                correction_id: correction_id.clone(),
            },
            geometry,
            author_user_id: user_id.clone(),
            created_at: current.created_at,
            updated_at: now,
            deleted: false,
        };
        annotation
            .validate_for_task(task, image.dimensions())
            .map_err(|error| StorageError::InvalidCorrection(error.to_string()))?;

        let correction = ReviewerCorrectionRecord {
            correction_id: correction_id.clone(),
            assignment_id: assignment_id.clone(),
            annotation_id: annotation_id.clone(),
            previous_version: expected_version,
            corrected_version: annotation.version,
            task_id: task_id.clone(),
            reviewer_user_id: user_id.clone(),
            timestamp: now,
            reason: reason.clone(),
        };
        let review = ReviewRecord {
            review_id: ReviewId::generate(),
            target: ReviewTarget::AnnotationVersion {
                annotation_id: annotation_id.clone(),
                version: expected_version,
            },
            reviewer_user_id: user_id.clone(),
            decision: ReviewDecision::Rejected,
            timestamp: now,
            comment: reason,
        };
        let task_state = TaskState {
            task_id: task_id.clone(),
            status: TaskStatus::Completed,
            outcome: Some(TaskOutcome::ReviewerCorrected),
            assigned_to: None,
            completed_by: Some(user_id.clone()),
            completed_at: Some(now),
            updated_at: now,
        };
        let mut assignments = state
            .assignments
            .iter()
            .filter(|assignment| {
                assignment.task_id == *task_id
                    && assignment.kind == AssignmentKind::Review
                    && assignment.status == AssignmentStatus::Active
            })
            .cloned()
            .collect::<Vec<_>>();
        for assignment in &mut assignments {
            assignment.status = if assignment.assignment_id == current_assignment.assignment_id {
                AssignmentStatus::Completed
            } else {
                AssignmentStatus::Cancelled
            };
            assignment.updated_at = now;
        }

        let payload = EventPayload::ReviewerCorrectionRecorded {
            correction,
            annotation: Box::new(annotation),
            review,
            task_state,
            assignments,
        };
        Ok(self
            .append_payloads_unlocked(
                image_id,
                &Actor {
                    user_id: user_id.clone(),
                    role: DatasetRole::Reviewer,
                },
                vec![payload],
            )
            .await?
            .into_iter()
            .next()
            .expect("one reviewer correction event was appended"))
    }
}

pub(super) fn current_task_reviews_from_events(
    events: &[EventLogEntry],
    task_id: &TaskId,
) -> Vec<ReviewRecord> {
    let Some(round_start) = events.iter().rposition(|event| {
        matches!(
            &event.payload,
            EventPayload::TaskStateChanged { task_state }
                if task_state.task_id == *task_id
                    && task_state.status == TaskStatus::Submitted
        )
    }) else {
        return Vec::new();
    };
    events
        .iter()
        .skip(round_start + 1)
        .filter_map(|event| match &event.payload {
            EventPayload::ReviewRecorded { review }
                if matches!(
                    &review.target,
                    ReviewTarget::Task { task_id: reviewed } if reviewed == task_id
                ) =>
            {
                Some(review.clone())
            }
            _ => None,
        })
        .collect()
}

pub(super) fn has_task_review_by_user(
    reviews: &[ReviewRecord],
    task_id: &TaskId,
    user_id: &UserId,
) -> bool {
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

pub(super) fn task_approval_count(reviews: &[ReviewRecord], task_id: &TaskId) -> u32 {
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
