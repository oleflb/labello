use labello_domain::{
    Actor, AnnotationGeometry, AnnotationId, Assignment, AssignmentId, AssignmentKind,
    AssignmentStatus, CorrectionId, DatasetMetadata, DatasetRole, EventLogEntry, EventPayload,
    ImageId, ReviewDecision, ReviewId, ReviewRecord, ReviewTarget, ReviewWorkflow,
    ReviewerCorrectionRecord, RevisionSource, TaskDefinition, TaskId, TaskOutcome, TaskState,
    TaskStatus, UserId, require_role,
};

use crate::{DatasetRepository, StorageError, StorageResult};

mod claim;
mod migration;
mod review;

pub(crate) use migration::append_guide_invalidation_payloads;
pub use migration::*;

/// Assignments are renewed by claim retries and successful assignment-backed
/// writes, so a separate heartbeat endpoint is not required.
pub const DEFAULT_ASSIGNMENT_LEASE_DURATION: std::time::Duration =
    std::time::Duration::from_secs(30 * 60);

pub struct AssignmentContext<'a> {
    pub assignment_id: &'a AssignmentId,
    pub image_id: &'a ImageId,
    pub task_id: &'a TaskId,
    pub kind: AssignmentKind,
}

impl DatasetRepository {
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
        let now = labello_domain::now();
        let mut assignment = exact_active_assignment(
            &state.assignments,
            assignment_id,
            image_id,
            task_id,
            user_id,
            &kind,
            now,
        )?
        .clone();
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
                    outcome: None,
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

    pub async fn reopen_annotation_assignment(
        &self,
        user_id: &UserId,
        assignment_id: &AssignmentId,
        image_id: &ImageId,
        task_id: &TaskId,
        kind: AssignmentKind,
    ) -> StorageResult<Assignment> {
        if kind != AssignmentKind::Annotation {
            return Err(StorageError::InvalidAssignment(
                "only annotation assignments can be reopened".to_string(),
            ));
        }
        let metadata = self.load_dataset().await?;
        require_role(
            &metadata.role_assignments,
            &metadata.dataset_id,
            user_id,
            DatasetRole::Annotator,
        )?;
        ensure_assignment_target_exists(&metadata, image_id, task_id)?;
        let task = metadata.task(task_id).ok_or_else(|| {
            StorageError::InvalidAssignment(format!("task {task_id} does not exist"))
        })?;
        if !task.enabled {
            return Err(StorageError::InvalidAssignment(format!(
                "task {task_id} is disabled"
            )));
        }

        let lock = self.image_lock(image_id);
        let _guard = lock.lock().await;
        let state = self.load_image_state(image_id).await?;
        let target_index = state
            .assignments
            .iter()
            .position(|assignment| assignment.assignment_id == *assignment_id)
            .ok_or_else(|| {
                StorageError::InvalidAssignment(format!(
                    "assignment {assignment_id} does not exist"
                ))
            })?;
        let target = &state.assignments[target_index];
        if target.assigned_to != *user_id {
            return Err(StorageError::Unauthorized(format!(
                "assignment {assignment_id} belongs to another user"
            )));
        }
        if target.image_id != *image_id
            || target.task_id != *task_id
            || target.kind != AssignmentKind::Annotation
        {
            return Err(StorageError::InvalidAssignment(format!(
                "assignment {assignment_id} does not match the requested image, task, and kind"
            )));
        }

        let now = labello_domain::now();
        let later_assignments = state
            .assignments
            .iter()
            .skip(target_index + 1)
            .filter(|assignment| assignment.task_id == *task_id)
            .collect::<Vec<_>>();
        let blocking_later_assignments = later_assignments
            .iter()
            .copied()
            .filter(|assignment| {
                task.manual_box_guide_migration.is_none()
                    || assignment.kind == AssignmentKind::Annotation
            })
            .collect::<Vec<_>>();
        if let [successor] = blocking_later_assignments.as_slice()
            && successor.kind == AssignmentKind::Annotation
            && successor.assigned_to == *user_id
            && successor.status == AssignmentStatus::Active
            && !assignment_is_expired(successor, now)
            && state.task_states.get(task_id).is_some_and(|task_state| {
                matches!(
                    task_state.status,
                    TaskStatus::InProgress | TaskStatus::NeedsCorrection
                )
            })
        {
            let mut successor = (*successor).clone();
            renew_assignment(&mut successor, now);
            self.append_payloads_unlocked(
                image_id,
                &Actor {
                    user_id: user_id.clone(),
                    role: DatasetRole::Annotator,
                },
                vec![EventPayload::AssignmentUpdated {
                    assignment: successor.clone(),
                }],
            )
            .await?;
            return Ok(successor);
        }
        if !blocking_later_assignments.is_empty() {
            return Err(StorageError::AssignmentConflict(format!(
                "assignment {assignment_id} is no longer the latest attempt for task {task_id}"
            )));
        }

        let current_status = state
            .task_states
            .get(task_id)
            .map(|task_state| task_state.status.clone())
            .unwrap_or(TaskStatus::Pending);
        let reopened_status = match target.status {
            AssignmentStatus::Cancelled => match current_status {
                TaskStatus::Pending => TaskStatus::InProgress,
                TaskStatus::NeedsCorrection => TaskStatus::NeedsCorrection,
                status => {
                    return Err(StorageError::AssignmentConflict(format!(
                        "cancelled assignment {assignment_id} cannot be reopened from task status {status:?}"
                    )));
                }
            },
            AssignmentStatus::Completed | AssignmentStatus::Submitted => {
                if !matches!(
                    current_status,
                    TaskStatus::Submitted | TaskStatus::Completed
                ) {
                    return Err(StorageError::AssignmentConflict(format!(
                        "completed assignment {assignment_id} cannot be reopened from task status {current_status:?}"
                    )));
                }
                if task.manual_box_guide_migration.is_some() {
                    TaskStatus::NeedsCorrection
                } else {
                    let events = self.load_events(image_id).await?;
                    annotation_status_before_completion(&events, assignment_id, task_id)
                        .ok_or_else(|| {
                            StorageError::AssignmentConflict(format!(
                                "assignment {assignment_id} has no replayable annotation state"
                            ))
                        })?
                }
            }
            AssignmentStatus::Active => {
                return Err(StorageError::AssignmentConflict(format!(
                    "assignment {assignment_id} is still active"
                )));
            }
        };

        let assignment = Assignment {
            assignment_id: AssignmentId::generate(),
            image_id: image_id.clone(),
            task_id: task_id.clone(),
            assigned_to: user_id.clone(),
            kind: AssignmentKind::Annotation,
            status: AssignmentStatus::Active,
            expires_at: Some(lease_expiration(now)),
            created_at: now,
            updated_at: now,
        };
        let assigned_to = (reopened_status == TaskStatus::InProgress).then(|| user_id.clone());
        let mut payloads = vec![
            EventPayload::AssignmentUpdated {
                assignment: assignment.clone(),
            },
            EventPayload::TaskStateChanged {
                task_state: TaskState {
                    task_id: task_id.clone(),
                    status: reopened_status,
                    outcome: None,
                    assigned_to,
                    completed_by: None,
                    completed_at: None,
                    updated_at: now,
                },
            },
        ];
        if task.manual_box_guide_migration.is_some() {
            for review_assignment in later_assignments.into_iter().filter(|assignment| {
                assignment.kind == AssignmentKind::Review
                    && assignment.status == AssignmentStatus::Active
            }) {
                let mut review_assignment = review_assignment.clone();
                review_assignment.status = AssignmentStatus::Cancelled;
                review_assignment.updated_at = now;
                payloads.push(EventPayload::AssignmentUpdated {
                    assignment: review_assignment,
                });
            }
        }
        self.append_payloads_unlocked(
            image_id,
            &Actor {
                user_id: user_id.clone(),
                role: DatasetRole::Annotator,
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
        let metadata = self.load_dataset().await?;
        let task = metadata.task(task_id).ok_or_else(|| {
            StorageError::InvalidAssignment(format!("task {task_id} does not exist"))
        })?;
        let status = match task.review.workflow {
            ReviewWorkflow::None => TaskStatus::Completed,
            ReviewWorkflow::Approval => TaskStatus::Submitted,
            ReviewWorkflow::IndependentAgreement => {
                return Err(StorageError::InvalidAssignment(format!(
                    "independent agreement workflow is not implemented for task {task_id}"
                )));
            }
        };
        let now = labello_domain::now();
        let task_state = TaskState {
            task_id: task_id.clone(),
            outcome: (status == TaskStatus::Completed).then_some(TaskOutcome::AnnotationCompleted),
            status,
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

    pub async fn apply_annotation_batch(
        &self,
        user_id: &UserId,
        assignment_context: AssignmentContext<'_>,
        payloads: Vec<EventPayload>,
        complete: bool,
    ) -> StorageResult<labello_domain::ImageState> {
        let AssignmentContext {
            assignment_id,
            image_id,
            task_id,
            kind,
        } = assignment_context;
        if kind != AssignmentKind::Annotation {
            return Err(StorageError::InvalidAssignment(
                "annotation batches require an annotation assignment".to_string(),
            ));
        }
        let metadata = self.load_dataset().await?;
        require_role(
            &metadata.role_assignments,
            &metadata.dataset_id,
            user_id,
            DatasetRole::Annotator,
        )?;
        ensure_assignment_target_exists(&metadata, image_id, task_id)?;
        let task = metadata.task(task_id).ok_or_else(|| {
            StorageError::InvalidAssignment(format!("task {task_id} does not exist"))
        })?;
        if task.manual_box_guide_migration.is_some() {
            return Err(StorageError::InvalidAssignment(
                "manual migration annotations require the migration command workflow".to_string(),
            ));
        }
        let image = metadata.images.get(image_id).ok_or_else(|| {
            StorageError::InvalidAssignment(format!("image {image_id} does not exist"))
        })?;
        let completion_status = match task.review.workflow {
            ReviewWorkflow::None => TaskStatus::Completed,
            ReviewWorkflow::Approval => TaskStatus::Submitted,
            ReviewWorkflow::IndependentAgreement => {
                return Err(StorageError::InvalidAssignment(format!(
                    "independent agreement workflow is not implemented for task {task_id}"
                )));
            }
        };

        let lock = self.image_lock(image_id);
        let _guard = lock.lock().await;
        let state = self.load_image_state(image_id).await?;
        let actor = Actor {
            user_id: user_id.clone(),
            role: DatasetRole::Annotator,
        };
        let mut payloads =
            validate_annotation_batch(&state, task, image.dimensions(), &actor, payloads)?;
        if let Some(existing) = state
            .assignments
            .iter()
            .find(|candidate| candidate.assignment_id == *assignment_id)
            && existing.status == AssignmentStatus::Completed
        {
            if existing.assigned_to != *user_id {
                return Err(StorageError::Unauthorized(format!(
                    "assignment {assignment_id} belongs to another user"
                )));
            }
            if existing.image_id != *image_id
                || existing.task_id != *task_id
                || existing.kind != AssignmentKind::Annotation
            {
                return Err(StorageError::AssignmentConflict(format!(
                    "assignment {assignment_id} does not match the requested work"
                )));
            }
            if complete && payloads.is_empty() {
                return Ok(state);
            }
        }

        let now = labello_domain::now();
        let mut assignment = exact_active_assignment(
            &state.assignments,
            assignment_id,
            image_id,
            task_id,
            user_id,
            &AssignmentKind::Annotation,
            now,
        )?
        .clone();
        let task_status = state
            .task_states
            .get(task_id)
            .map(|task_state| &task_state.status)
            .unwrap_or(&TaskStatus::Pending);
        if !matches!(
            task_status,
            TaskStatus::InProgress | TaskStatus::NeedsCorrection
        ) {
            return Err(StorageError::AssignmentConflict(format!(
                "assignment {assignment_id} is not valid for task status {task_status:?}"
            )));
        }
        if payloads.is_empty() && !complete {
            return Ok(state);
        }

        if complete {
            payloads.push(EventPayload::TaskStateChanged {
                task_state: TaskState {
                    task_id: task_id.clone(),
                    outcome: (completion_status == TaskStatus::Completed)
                        .then_some(TaskOutcome::AnnotationCompleted),
                    status: completion_status,
                    assigned_to: Some(user_id.clone()),
                    completed_by: Some(user_id.clone()),
                    completed_at: Some(now),
                    updated_at: now,
                },
            });
            assignment.status = AssignmentStatus::Completed;
            assignment.updated_at = now;
        } else {
            renew_assignment(&mut assignment, now);
        }
        payloads.push(EventPayload::AssignmentUpdated { assignment });
        self.append_payloads_unlocked(image_id, &actor, payloads)
            .await?;
        self.load_image_state(image_id).await
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
        if metadata
            .task(task_id)
            .is_some_and(|task| task.manual_box_guide_migration.is_some())
        {
            return Err(StorageError::InvalidAssignment(
                "manual migration writes require the migration command workflow".to_string(),
            ));
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
            &kind,
            now,
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
            assignment.updated_at = now;
        } else {
            renew_assignment(&mut assignment, now);
        }
        payloads.push(EventPayload::AssignmentUpdated {
            assignment: assignment.clone(),
        });
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
}

fn annotation_status_before_completion(
    events: &[EventLogEntry],
    assignment_id: &AssignmentId,
    task_id: &TaskId,
) -> Option<TaskStatus> {
    let completion = events.iter().rposition(|event| {
        matches!(
            &event.payload,
            EventPayload::AssignmentUpdated { assignment }
                if assignment.assignment_id == *assignment_id
                    && matches!(
                        assignment.status,
                        AssignmentStatus::Completed | AssignmentStatus::Submitted
                    )
        )
    })?;
    events[..completion]
        .iter()
        .rev()
        .find_map(|event| match &event.payload {
            EventPayload::TaskStateChanged { task_state }
                if task_state.task_id == *task_id
                    && matches!(
                        task_state.status,
                        TaskStatus::InProgress | TaskStatus::NeedsCorrection
                    ) =>
            {
                Some(task_state.status.clone())
            }
            _ => None,
        })
}

fn validate_annotation_batch(
    state: &labello_domain::ImageState,
    task: &labello_domain::TaskDefinition,
    image_dimensions: labello_domain::ImageDimensions,
    actor: &Actor,
    payloads: Vec<EventPayload>,
) -> StorageResult<Vec<EventPayload>> {
    let mut next_state = state.clone();
    let mut validated = Vec::with_capacity(payloads.len());
    for payload in payloads {
        match &payload {
            EventPayload::AnnotationVersionCreated { annotation, .. } => {
                if annotation.task_id != task.task_id {
                    return Err(StorageError::InvalidAssignment(
                        "annotation task does not match assignment task".to_string(),
                    ));
                }
                annotation
                    .validate_for_task(task, image_dimensions)
                    .map_err(|error| StorageError::InvalidAssignment(error.to_string()))?;
                if next_state.current_annotation(&annotation.annotation_id) == Some(annotation) {
                    continue;
                }
            }
            EventPayload::AnnotationDeleted {
                annotation_id,
                version,
                ..
            } => {
                let current = next_state
                    .current_annotation(annotation_id)
                    .ok_or_else(|| {
                        StorageError::InvalidAssignment(format!(
                            "unknown annotation {annotation_id}"
                        ))
                    })?;
                if current.task_id != task.task_id {
                    return Err(StorageError::InvalidAssignment(
                        "annotation task does not match assignment task".to_string(),
                    ));
                }
                if current.deleted || current.version != *version {
                    return Err(StorageError::InvalidAssignment(format!(
                        "annotation {annotation_id} deletion version {version} is not the current active version {}",
                        current.version
                    )));
                }
            }
            _ => {
                return Err(StorageError::InvalidAssignment(
                    "annotation batches only accept annotation mutations".to_string(),
                ));
            }
        }
        let event = EventLogEntry::new(
            next_state.current_sequence + 1,
            next_state.image_id.clone(),
            actor.user_id.clone(),
            actor.role.clone(),
            labello_domain::now(),
            payload.clone(),
        );
        next_state.apply_event(&event)?;
        validated.push(payload);
    }
    Ok(validated)
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
    now: labello_domain::Timestamp,
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
    if assignment_is_expired(assignment, now) {
        return Err(StorageError::AssignmentConflict(format!(
            "assignment {assignment_id} lease has expired"
        )));
    }
    Ok(assignment)
}

fn lease_expiration(now: labello_domain::Timestamp) -> labello_domain::Timestamp {
    now + DEFAULT_ASSIGNMENT_LEASE_DURATION
}

fn assignment_is_expired(assignment: &Assignment, now: labello_domain::Timestamp) -> bool {
    assignment
        .expires_at
        .unwrap_or_else(|| lease_expiration(assignment.updated_at))
        <= now
}

fn renew_assignment(assignment: &mut Assignment, now: labello_domain::Timestamp) {
    assignment.updated_at = now;
    assignment.expires_at = Some(lease_expiration(now));
}

#[allow(dead_code)]
fn _image_id_type_is_used(_: &ImageId) {}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use labello_domain::{
        AdjudicationDecision, AdjudicationId, AdjudicationRecord, AnnotationGeometry,
        AnnotationOrigin, AnnotationType, AnnotationVersion, BoundingBox, ClassId, DatasetId,
        DatasetMetadata, DatasetRoleAssignment, HumanRevisionKind, ImageRecord, ImagesIndex,
        ImportCoverage, ImportId, ImportTaskInitialization, KeypointAnnotation, KeypointSpec,
        KeypointState, LabelClass, NormalizedPoint, ReviewConfig, ReviewDecision, ReviewId,
        ReviewRecord, ReviewTarget, ReviewWorkflow, SCHEMA_VERSION, SkeletonGeometry, SkeletonSpec,
        TaskDefinition, TutorialContent, now,
    };

    use super::review::{current_task_reviews_from_events, task_approval_count};
    use super::*;

    #[test]
    fn current_reviews_begin_after_the_latest_submission() {
        let image_id = ImageId::from("img_1");
        let task_id = TaskId::from("bounding_box:person");
        let reviewer = UserId::from("reviewer");
        let timestamp = now();
        let task_state = |status| EventPayload::TaskStateChanged {
            task_state: TaskState {
                task_id: task_id.clone(),
                status,
                outcome: None,
                assigned_to: None,
                completed_by: None,
                completed_at: None,
                updated_at: timestamp,
            },
        };
        let review = |review_id: &str| ReviewRecord {
            review_id: ReviewId::from(review_id),
            target: ReviewTarget::Task {
                task_id: task_id.clone(),
            },
            reviewer_user_id: reviewer.clone(),
            decision: ReviewDecision::Approved,
            timestamp,
            comment: None,
        };
        let payloads = [
            task_state(TaskStatus::Submitted),
            EventPayload::ReviewRecorded {
                review: review("old_review"),
            },
            task_state(TaskStatus::NeedsCorrection),
            task_state(TaskStatus::Submitted),
            EventPayload::ReviewRecorded {
                review: review("current_review"),
            },
        ];
        let events = payloads
            .into_iter()
            .enumerate()
            .map(|(index, payload)| {
                EventLogEntry::new(
                    index as u64 + 1,
                    image_id.clone(),
                    reviewer.clone(),
                    DatasetRole::Reviewer,
                    timestamp,
                    payload,
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            current_task_reviews_from_events(&events, &task_id),
            vec![review("current_review")]
        );
    }

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
            manual_box_guide_migration: None,
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
            source_memberships: None,
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
            source_memberships: None,
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
        let sequence_before_reclaim = repo
            .load_image_state(&first.image_id)
            .await
            .unwrap()
            .current_sequence;
        let reclaimed = repo
            .reclaim_assignment(
                &user_id,
                &first.assignment_id,
                &task_id,
                AssignmentKind::Annotation,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reclaimed, first);
        assert_eq!(
            repo.load_image_state(&first.image_id)
                .await
                .unwrap()
                .current_sequence,
            sequence_before_reclaim,
            "exact reclaim must not invalidate a browser draft's base sequence"
        );
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
                expires_at: Some(lease_expiration(now())),
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
            manual_box_guide_migration: None,
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
                    source_memberships: None,
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
                    outcome: None,
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
    async fn cancelled_assignment_reopens_as_a_fresh_replayable_attempt() {
        let (_temp, repo, task_id, users) = annotation_repo(1, &["worker"]).await;
        let original = repo
            .assign_next_image(&users[0], &task_id, AssignmentKind::Annotation)
            .await
            .unwrap()
            .unwrap();
        repo.release_assignment(
            &users[0],
            &original.assignment_id,
            &original.image_id,
            &task_id,
            AssignmentKind::Annotation,
        )
        .await
        .unwrap();

        let reopened = repo
            .reopen_annotation_assignment(
                &users[0],
                &original.assignment_id,
                &original.image_id,
                &task_id,
                AssignmentKind::Annotation,
            )
            .await
            .unwrap();
        let retry = repo
            .reopen_annotation_assignment(
                &users[0],
                &original.assignment_id,
                &original.image_id,
                &task_id,
                AssignmentKind::Annotation,
            )
            .await
            .unwrap();

        assert_ne!(reopened.assignment_id, original.assignment_id);
        assert_eq!(retry.assignment_id, reopened.assignment_id);
        let state = repo.rebuild_image_state(&original.image_id).await.unwrap();
        assert_eq!(state.task_states[&task_id].status, TaskStatus::InProgress);
        assert_eq!(
            assignment_status(&state, &original),
            AssignmentStatus::Cancelled
        );
        assert_eq!(
            assignment_status(&state, &reopened),
            AssignmentStatus::Active
        );
    }

    #[tokio::test]
    async fn submitted_assignment_reopens_to_its_previous_annotation_state() {
        let (_temp, repo, task_id, users) = annotation_repo(1, &["worker"]).await;
        let original = repo
            .assign_next_image(&users[0], &task_id, AssignmentKind::Annotation)
            .await
            .unwrap()
            .unwrap();
        repo.complete_assignment(
            &users[0],
            &original.assignment_id,
            &original.image_id,
            &task_id,
            AssignmentKind::Annotation,
        )
        .await
        .unwrap();

        let reopened = repo
            .reopen_annotation_assignment(
                &users[0],
                &original.assignment_id,
                &original.image_id,
                &task_id,
                AssignmentKind::Annotation,
            )
            .await
            .unwrap();

        let state = repo.rebuild_image_state(&original.image_id).await.unwrap();
        assert_eq!(state.task_states[&task_id].status, TaskStatus::InProgress);
        assert_eq!(
            assignment_status(&state, &original),
            AssignmentStatus::Completed
        );
        assert_eq!(
            assignment_status(&state, &reopened),
            AssignmentStatus::Active
        );
    }

    #[tokio::test]
    async fn reopen_preserves_needs_correction_and_rejects_downstream_review_work() {
        let (_temp, repo, task_id, users) = annotation_repo(1, &["worker", "reviewer"]).await;
        let mut metadata = repo.load_dataset_config().await.unwrap();
        metadata.role_assignments[1]
            .roles
            .insert(DatasetRole::Reviewer);
        repo.save_dataset(&metadata).await.unwrap();
        let image_id = ImageId::from("img_0");
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
                    outcome: None,
                    assigned_to: None,
                    completed_by: None,
                    completed_at: None,
                    updated_at: now(),
                },
            },
        )
        .await
        .unwrap();
        let correction = repo
            .assign_next_image(&users[0], &task_id, AssignmentKind::Annotation)
            .await
            .unwrap()
            .unwrap();
        repo.complete_assignment(
            &users[0],
            &correction.assignment_id,
            &image_id,
            &task_id,
            AssignmentKind::Annotation,
        )
        .await
        .unwrap();
        let reopened = repo
            .reopen_annotation_assignment(
                &users[0],
                &correction.assignment_id,
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

        repo.complete_assignment(
            &users[0],
            &reopened.assignment_id,
            &image_id,
            &task_id,
            AssignmentKind::Annotation,
        )
        .await
        .unwrap();
        repo.assign_next_image(&users[1], &task_id, AssignmentKind::Review)
            .await
            .unwrap()
            .unwrap();
        let error = repo
            .reopen_annotation_assignment(
                &users[0],
                &reopened.assignment_id,
                &image_id,
                &task_id,
                AssignmentKind::Annotation,
            )
            .await
            .unwrap_err();
        assert!(matches!(error, StorageError::AssignmentConflict(_)));
    }

    #[tokio::test]
    async fn concurrent_reopen_retries_share_one_renewed_successor() {
        let (_temp, repo, task_id, users) = annotation_repo(1, &["worker"]).await;
        let original = repo
            .assign_next_image(&users[0], &task_id, AssignmentKind::Annotation)
            .await
            .unwrap()
            .unwrap();
        repo.release_assignment(
            &users[0],
            &original.assignment_id,
            &original.image_id,
            &task_id,
            AssignmentKind::Annotation,
        )
        .await
        .unwrap();
        let left_repo = repo.clone();
        let right_repo = repo.clone();
        let left_assignment = original.clone();
        let right_assignment = original.clone();
        let left_task = task_id.clone();
        let right_task = task_id.clone();
        let left_user = users[0].clone();
        let right_user = users[0].clone();

        let (left, right) = tokio::join!(
            async move {
                left_repo
                    .reopen_annotation_assignment(
                        &left_user,
                        &left_assignment.assignment_id,
                        &left_assignment.image_id,
                        &left_task,
                        AssignmentKind::Annotation,
                    )
                    .await
                    .unwrap()
            },
            async move {
                right_repo
                    .reopen_annotation_assignment(
                        &right_user,
                        &right_assignment.assignment_id,
                        &right_assignment.image_id,
                        &right_task,
                        AssignmentKind::Annotation,
                    )
                    .await
                    .unwrap()
            }
        );

        assert_eq!(left.assignment_id, right.assignment_id);
        assert!(left.expires_at.is_some());
        assert!(right.expires_at.is_some());
        let state = repo.rebuild_image_state(&original.image_id).await.unwrap();
        assert_eq!(
            state
                .assignments
                .iter()
                .filter(|assignment| assignment.status == AssignmentStatus::Active)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn released_image_can_be_excluded_from_the_next_claim() {
        let (_temp, repo, task_id, users) = annotation_repo(2, &["worker"]).await;
        let first = repo
            .assign_next_image(&users[0], &task_id, AssignmentKind::Annotation)
            .await
            .unwrap()
            .unwrap();
        repo.release_assignment(
            &users[0],
            &first.assignment_id,
            &first.image_id,
            &task_id,
            AssignmentKind::Annotation,
        )
        .await
        .unwrap();

        let next = repo
            .assign_next_image_excluding(
                &users[0],
                &task_id,
                AssignmentKind::Annotation,
                std::slice::from_ref(&first.image_id),
            )
            .await
            .unwrap()
            .unwrap();

        assert_ne!(next.image_id, first.image_id);
    }

    #[tokio::test]
    async fn claim_retry_renews_the_same_assignment() {
        let (_temp, repo, task_id, users) = annotation_repo(1, &["worker"]).await;
        let first = repo
            .assign_next_image(&users[0], &task_id, AssignmentKind::Annotation)
            .await
            .unwrap()
            .unwrap();
        let retry = repo
            .assign_next_image(&users[0], &task_id, AssignmentKind::Annotation)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(retry.assignment_id, first.assignment_id);
        assert!(retry.updated_at >= first.updated_at);
        assert!(retry.expires_at.unwrap() >= first.expires_at.unwrap());
        let (_, refreshed) = repo
            .append_for_assignment(
                &users[0],
                AssignmentContext {
                    assignment_id: &retry.assignment_id,
                    image_id: &retry.image_id,
                    task_id: &task_id,
                    kind: AssignmentKind::Annotation,
                },
                Vec::new(),
                false,
            )
            .await
            .unwrap();
        assert_eq!(refreshed.assignment_id, first.assignment_id);
        assert!(refreshed.expires_at.unwrap() >= retry.expires_at.unwrap());
        let persisted = repo.load_image_state(&first.image_id).await.unwrap();
        assert_eq!(persisted.assignments[0].expires_at, refreshed.expires_at);
    }

    #[tokio::test]
    async fn expired_annotation_is_cancelled_and_atomically_reclaimed() {
        let (_temp, repo, task_id, users) = annotation_repo(1, &["owner", "next", "other"]).await;
        let first = repo
            .assign_next_image(&users[0], &task_id, AssignmentKind::Annotation)
            .await
            .unwrap()
            .unwrap();
        expire_assignment(&repo, &first, &users[0]).await;

        let next_repo = repo.clone();
        let other_repo = repo.clone();
        let next_task = task_id.clone();
        let other_task = task_id.clone();
        let next_user = users[1].clone();
        let other_user = users[2].clone();
        let (next, other) = tokio::join!(
            async move {
                next_repo
                    .assign_next_image(&next_user, &next_task, AssignmentKind::Annotation)
                    .await
                    .unwrap()
            },
            async move {
                other_repo
                    .assign_next_image(&other_user, &other_task, AssignmentKind::Annotation)
                    .await
                    .unwrap()
            }
        );
        assert_eq!(
            usize::from(next.is_some()) + usize::from(other.is_some()),
            1
        );
        let reclaimed = next.or(other).unwrap();

        assert_ne!(reclaimed.assignment_id, first.assignment_id);
        assert!(reclaimed.assigned_to == users[1] || reclaimed.assigned_to == users[2]);
        let state = repo.load_image_state(&first.image_id).await.unwrap();
        assert_eq!(state.task_states[&task_id].status, TaskStatus::InProgress);
        assert_eq!(
            state
                .assignments
                .iter()
                .find(|assignment| assignment.assignment_id == first.assignment_id)
                .unwrap()
                .status,
            AssignmentStatus::Cancelled
        );
        let events = repo.load_events(&first.image_id).await.unwrap();
        assert!(events.iter().any(|event| matches!(
            &event.payload,
            EventPayload::TaskStateChanged { task_state }
                if task_state.task_id == task_id && task_state.status == TaskStatus::Pending
        )));
    }

    #[tokio::test]
    async fn expired_correction_assignment_preserves_needs_correction() {
        let (_temp, repo, task_id, users) = annotation_repo(1, &["owner", "next"]).await;
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
                    outcome: None,
                    assigned_to: None,
                    completed_by: None,
                    completed_at: None,
                    updated_at: timestamp,
                },
            },
        )
        .await
        .unwrap();
        let first = repo
            .assign_next_image(&users[0], &task_id, AssignmentKind::Annotation)
            .await
            .unwrap()
            .unwrap();
        expire_assignment(&repo, &first, &users[0]).await;

        let reclaimed = repo
            .assign_next_image(&users[1], &task_id, AssignmentKind::Annotation)
            .await
            .unwrap()
            .unwrap();

        assert_ne!(reclaimed.assignment_id, first.assignment_id);
        assert_eq!(
            repo.load_image_state(&image_id).await.unwrap().task_states[&task_id].status,
            TaskStatus::NeedsCorrection
        );
    }

    #[tokio::test]
    async fn expired_owner_cannot_complete_assignment() {
        let (_temp, repo, task_id, users) = annotation_repo(1, &["owner"]).await;
        let assignment = repo
            .assign_next_image(&users[0], &task_id, AssignmentKind::Annotation)
            .await
            .unwrap()
            .unwrap();
        expire_assignment(&repo, &assignment, &users[0]).await;

        let error = repo
            .complete_assignment(
                &users[0],
                &assignment.assignment_id,
                &assignment.image_id,
                &task_id,
                AssignmentKind::Annotation,
            )
            .await
            .unwrap_err();

        assert!(matches!(error, StorageError::AssignmentConflict(_)));
        assert!(error.to_string().contains("expired"));
    }

    #[tokio::test]
    async fn completing_annotation_without_review_completes_task() {
        let (_temp, repo, task_id, users) = annotation_repo(1, &["worker"]).await;
        let mut metadata = repo.load_dataset_config().await.unwrap();
        metadata.tasks[0].review.workflow = ReviewWorkflow::None;
        metadata.tasks[0].review.required_reviews = 0;
        repo.save_dataset(&metadata).await.unwrap();
        let assignment = repo
            .assign_next_image(&users[0], &task_id, AssignmentKind::Annotation)
            .await
            .unwrap()
            .unwrap();

        repo.complete_assignment(
            &users[0],
            &assignment.assignment_id,
            &assignment.image_id,
            &task_id,
            AssignmentKind::Annotation,
        )
        .await
        .unwrap();

        let state = repo.load_image_state(&assignment.image_id).await.unwrap();
        assert_eq!(state.task_states[&task_id].status, TaskStatus::Completed);
    }

    #[tokio::test]
    async fn independent_agreement_claim_is_rejected() {
        let (_temp, repo, task_id, users) = annotation_repo(1, &["worker"]).await;
        let mut metadata = repo.load_dataset_config().await.unwrap();
        metadata.tasks[0].review.workflow = ReviewWorkflow::IndependentAgreement;
        repo.save_dataset(&metadata).await.unwrap();

        let error = repo
            .assign_next_image(&users[0], &task_id, AssignmentKind::Annotation)
            .await
            .unwrap_err();

        assert!(matches!(error, StorageError::InvalidAssignment(_)));
        assert!(error.to_string().contains("not implemented"));
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
                    outcome: None,
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
    async fn annotation_claims_skip_excluded_and_imported_terminal_tasks() {
        let (_temp, repo, task_id, users) = annotation_repo(3, &["worker"]).await;
        let timestamp = now();
        for (index, (coverage, status)) in [
            (ImportCoverage::Excluded, TaskStatus::Pending),
            (ImportCoverage::Complete, TaskStatus::Completed),
            (ImportCoverage::Complete, TaskStatus::Submitted),
        ]
        .into_iter()
        .enumerate()
        {
            let image_id = ImageId::from(format!("img_{index}"));
            let terminal = matches!(status, TaskStatus::Completed | TaskStatus::Submitted);
            repo.append_payload(
                &image_id,
                &Actor {
                    user_id: UserId::from("admin"),
                    role: DatasetRole::DataAdmin,
                },
                EventPayload::ImportInitialized {
                    import_id: ImportId::from(format!("imp_{index}")),
                    annotations: Vec::new(),
                    task_initializations: vec![ImportTaskInitialization {
                        task_id: task_id.clone(),
                        coverage,
                        initial_state: TaskState {
                            task_id: task_id.clone(),
                            status,
                            outcome: terminal.then_some(TaskOutcome::ImportedGroundTruth),
                            assigned_to: None,
                            completed_by: terminal.then(|| UserId::from("admin")),
                            completed_at: terminal.then_some(timestamp),
                            updated_at: timestamp,
                        },
                    }],
                    migration_target_sets: Vec::new(),
                },
            )
            .await
            .unwrap();
            assert!(
                !repo
                    .load_image_state(&image_id)
                    .await
                    .unwrap()
                    .assignment_eligible(&task_id)
            );
        }

        assert!(
            repo.assign_next_image(&users[0], &task_id, AssignmentKind::Annotation)
                .await
                .unwrap()
                .is_none()
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

    #[tokio::test]
    async fn annotation_batch_is_atomic_and_idempotent() {
        let (_temp, repo, task_id, users) = annotation_repo(1, &["worker"]).await;
        let assignment = repo
            .assign_next_image(&users[0], &task_id, AssignmentKind::Annotation)
            .await
            .unwrap()
            .unwrap();
        let annotation = |id: &str| AnnotationVersion {
            annotation_id: AnnotationId::from(id),
            version: 1,
            object_group_id: None,
            origin: AnnotationOrigin::native(),
            task_id: task_id.clone(),
            class_id: ClassId::from("person"),
            annotation_type: AnnotationType::BoundingBox,
            revision_source: RevisionSource::Human {
                action: HumanRevisionKind::Authored,
            },
            geometry: AnnotationGeometry::BoundingBox(BoundingBox {
                x: 0.1,
                y: 0.1,
                width: 0.4,
                height: 0.4,
            }),
            author_user_id: users[0].clone(),
            created_at: now(),
            updated_at: now(),
            deleted: false,
        };
        let create = |annotation: AnnotationVersion| EventPayload::AnnotationVersionCreated {
            annotation,
            previous_version: None,
            reason: None,
        };
        let context = || AssignmentContext {
            assignment_id: &assignment.assignment_id,
            image_id: &assignment.image_id,
            task_id: &task_id,
            kind: AssignmentKind::Annotation,
        };
        let before = repo.load_image_state(&assignment.image_id).await.unwrap();
        let error = repo
            .apply_annotation_batch(
                &users[0],
                context(),
                vec![
                    create(annotation("ann_1")),
                    EventPayload::AnnotationDeleted {
                        annotation_id: AnnotationId::from("missing"),
                        version: 1,
                        reason: None,
                    },
                ],
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(error, StorageError::InvalidAssignment(_)));
        let unchanged = repo.load_image_state(&assignment.image_id).await.unwrap();
        assert_eq!(unchanged.current_sequence, before.current_sequence);
        assert!(unchanged.annotations.is_empty());

        let first_version = annotation("ann_stale_delete");
        let mut second_version = first_version.clone();
        second_version.version = 2;
        second_version.geometry = AnnotationGeometry::BoundingBox(BoundingBox {
            x: 0.2,
            y: 0.2,
            width: 0.3,
            height: 0.3,
        });
        let error = repo
            .apply_annotation_batch(
                &users[0],
                context(),
                vec![
                    create(first_version),
                    EventPayload::AnnotationVersionCreated {
                        annotation: second_version,
                        previous_version: Some(1),
                        reason: Some("move".to_string()),
                    },
                    EventPayload::AnnotationDeleted {
                        annotation_id: AnnotationId::from("ann_stale_delete"),
                        version: 1,
                        reason: None,
                    },
                ],
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(error, StorageError::InvalidAssignment(_)));
        assert_eq!(
            repo.load_image_state(&assignment.image_id)
                .await
                .unwrap()
                .current_sequence,
            before.current_sequence
        );

        let payloads = vec![create(annotation("ann_1")), create(annotation("ann_2"))];
        let saved = repo
            .apply_annotation_batch(&users[0], context(), payloads.clone(), true)
            .await
            .unwrap();
        assert_eq!(saved.active_annotations().count(), 2);
        assert_eq!(saved.assignments[0].status, AssignmentStatus::Completed);

        let retried = repo
            .apply_annotation_batch(&users[0], context(), payloads, true)
            .await
            .unwrap();
        assert_eq!(retried.current_sequence, saved.current_sequence);
        assert_eq!(retried.active_annotations().count(), 2);
    }

    #[tokio::test]
    async fn bbox_correction_is_terminal_idempotent_and_updates_quality_stats() {
        let (_temp, repo, image_id, task_id, annotator, reviewers) =
            correction_repo(AnnotationType::BoundingBox, true).await;
        let first = claim_review(&repo, &image_id, &task_id, &reviewers[0]).await;
        let competing = claim_review(&repo, &image_id, &task_id, &reviewers[1]).await;
        repo.append_payload(
            &image_id,
            &Actor {
                user_id: reviewers[1].clone(),
                role: DatasetRole::Reviewer,
            },
            EventPayload::ReviewRecorded {
                review: ReviewRecord {
                    review_id: ReviewId::from("rev_partial_approval"),
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
        let correction_id = CorrectionId::from("cor_bbox");
        let geometry = AnnotationGeometry::BoundingBox(BoundingBox {
            x: 0.2,
            y: 0.25,
            width: 0.4,
            height: 0.3,
        });

        let event = correct(
            &repo,
            &image_id,
            &task_id,
            &reviewers[0],
            &first,
            &correction_id,
            1,
            geometry.clone(),
        )
        .await
        .unwrap();
        let event_count = repo.load_events(&image_id).await.unwrap().len();
        let retry = correct(
            &repo,
            &image_id,
            &task_id,
            &reviewers[0],
            &first,
            &correction_id,
            1,
            geometry.clone(),
        )
        .await
        .unwrap();

        assert_eq!(retry.event_id, event.event_id);
        assert_eq!(
            repo.load_events(&image_id).await.unwrap().len(),
            event_count
        );
        assert!(matches!(
            event.payload,
            EventPayload::ReviewerCorrectionRecorded { .. }
        ));
        let state = repo.load_image_state(&image_id).await.unwrap();
        let corrected = state
            .current_annotation(&AnnotationId::from("ann_1"))
            .unwrap();
        assert_eq!(corrected.version, 2);
        assert_eq!(corrected.geometry, geometry);
        assert_eq!(corrected.author_user_id, reviewers[0]);
        assert!(matches!(
            &corrected.revision_source,
            RevisionSource::ReviewerCorrection { correction_id: id } if id == &correction_id
        ));
        assert_eq!(state.task_states[&task_id].status, TaskStatus::Completed);
        assert_eq!(
            state.task_states[&task_id].outcome,
            Some(TaskOutcome::ReviewerCorrected)
        );
        assert!(
            state
                .reviews
                .iter()
                .any(|review| review.decision == ReviewDecision::Rejected)
        );
        assert_eq!(
            assignment_status(&state, &first),
            AssignmentStatus::Completed
        );
        assert_eq!(
            assignment_status(&state, &competing),
            AssignmentStatus::Cancelled
        );
        assert_eq!(repo.rebuild_image_state(&image_id).await.unwrap(), state);
        assert!(
            repo.assign_next_image(&annotator, &task_id, AssignmentKind::Annotation)
                .await
                .unwrap()
                .is_none()
        );

        let stats = repo.dataset_stats().await.unwrap();
        assert_eq!(stats.reviewed_tasks, 0);
        assert_eq!(stats.approved_tasks, 0);
        assert_eq!(stats.rejected_tasks, 1);
        assert_eq!(stats.reviewer_corrected_tasks, 1);
        assert_eq!(stats.finalized_tasks, 1);
        assert_eq!(stats.per_task[&task_id].reviewed, 0);
        assert_eq!(stats.per_task[&task_id].rejected, 1);
        assert_eq!(stats.per_task[&task_id].reviewer_corrected, 1);
        assert_eq!(stats.per_task[&task_id].finalized, 1);
    }

    #[tokio::test]
    async fn skeleton_correction_is_server_versioned_and_stale_or_disabled_work_is_rejected() {
        let (_temp, repo, image_id, task_id, _annotator, reviewers) =
            correction_repo(AnnotationType::Skeleton, true).await;
        let assignment = claim_review(&repo, &image_id, &task_id, &reviewers[0]).await;
        let unchanged = AnnotationGeometry::Skeleton(SkeletonGeometry {
            keypoints: vec![KeypointAnnotation {
                name: "nose".to_string(),
                state: KeypointState::Visible,
                point: Some(NormalizedPoint { x: 0.5, y: 0.5 }),
            }],
        });
        let unchanged_error = correct(
            &repo,
            &image_id,
            &task_id,
            &reviewers[0],
            &assignment,
            &CorrectionId::from("cor_unchanged"),
            1,
            unchanged,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            unchanged_error,
            StorageError::InvalidCorrection(_)
        ));

        let mut metadata = repo.load_dataset_config().await.unwrap();
        metadata.tasks[0].skeleton.as_mut().unwrap().allow_hidden = false;
        repo.save_dataset(&metadata).await.unwrap();
        for (correction_id, keypoints) in [
            ("cor_missing", Vec::new()),
            (
                "cor_wrong_name",
                vec![KeypointAnnotation {
                    name: "ear".to_string(),
                    state: KeypointState::Visible,
                    point: Some(NormalizedPoint { x: 0.6, y: 0.4 }),
                }],
            ),
            (
                "cor_hidden",
                vec![KeypointAnnotation {
                    name: "nose".to_string(),
                    state: KeypointState::Hidden,
                    point: Some(NormalizedPoint { x: 0.6, y: 0.4 }),
                }],
            ),
            (
                "cor_absent",
                vec![KeypointAnnotation {
                    name: "nose".to_string(),
                    state: KeypointState::Absent,
                    point: None,
                }],
            ),
        ] {
            let error = correct(
                &repo,
                &image_id,
                &task_id,
                &reviewers[0],
                &assignment,
                &CorrectionId::from(correction_id),
                1,
                AnnotationGeometry::Skeleton(SkeletonGeometry { keypoints }),
            )
            .await
            .unwrap_err();
            assert!(matches!(error, StorageError::InvalidCorrection(_)));
        }
        let geometry = AnnotationGeometry::Skeleton(SkeletonGeometry {
            keypoints: vec![KeypointAnnotation {
                name: "nose".to_string(),
                state: KeypointState::Visible,
                point: Some(NormalizedPoint { x: 0.6, y: 0.4 }),
            }],
        });
        let stale = correct(
            &repo,
            &image_id,
            &task_id,
            &reviewers[0],
            &assignment,
            &CorrectionId::from("cor_stale"),
            0,
            geometry.clone(),
        )
        .await
        .unwrap_err();
        assert!(matches!(stale, StorageError::AssignmentConflict(_)));

        correct(
            &repo,
            &image_id,
            &task_id,
            &reviewers[0],
            &assignment,
            &CorrectionId::from("cor_skeleton"),
            1,
            geometry.clone(),
        )
        .await
        .unwrap();
        let state = repo.load_image_state(&image_id).await.unwrap();
        assert_eq!(
            state
                .current_annotation(&AnnotationId::from("ann_1"))
                .unwrap()
                .geometry,
            geometry
        );

        let (_temp, disabled_repo, image_id, task_id, _annotator, reviewers) =
            correction_repo(AnnotationType::BoundingBox, false).await;
        let assignment = claim_review(&disabled_repo, &image_id, &task_id, &reviewers[0]).await;
        let error = correct(
            &disabled_repo,
            &image_id,
            &task_id,
            &reviewers[0],
            &assignment,
            &CorrectionId::from("cor_disabled"),
            1,
            AnnotationGeometry::BoundingBox(BoundingBox {
                x: 0.2,
                y: 0.2,
                width: 0.2,
                height: 0.2,
            }),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, StorageError::InvalidCorrection(_)));
    }

    #[tokio::test]
    async fn concurrent_corrections_have_one_winner_and_leave_no_active_review_assignment() {
        let (_temp, repo, image_id, task_id, _annotator, reviewers) =
            correction_repo(AnnotationType::BoundingBox, true).await;
        let first = claim_review(&repo, &image_id, &task_id, &reviewers[0]).await;
        let second = claim_review(&repo, &image_id, &task_id, &reviewers[1]).await;
        let first_repo = repo.clone();
        let second_repo = repo.clone();
        let first_image = image_id.clone();
        let second_image = image_id.clone();
        let first_task = task_id.clone();
        let second_task = task_id.clone();
        let first_user = reviewers[0].clone();
        let second_user = reviewers[1].clone();
        let geometry = AnnotationGeometry::BoundingBox(BoundingBox {
            x: 0.3,
            y: 0.3,
            width: 0.2,
            height: 0.2,
        });
        let other_geometry = geometry.clone();

        let (left, right) = tokio::join!(
            async move {
                correct(
                    &first_repo,
                    &first_image,
                    &first_task,
                    &first_user,
                    &first,
                    &CorrectionId::from("cor_first"),
                    1,
                    geometry,
                )
                .await
            },
            async move {
                correct(
                    &second_repo,
                    &second_image,
                    &second_task,
                    &second_user,
                    &second,
                    &CorrectionId::from("cor_second"),
                    1,
                    other_geometry,
                )
                .await
            }
        );

        assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
        let state = repo.load_image_state(&image_id).await.unwrap();
        assert_eq!(state.reviewer_corrections.len(), 1);
        assert_eq!(
            state
                .current_annotation(&AnnotationId::from("ann_1"))
                .unwrap()
                .version,
            2
        );
        assert!(state.assignments.iter().all(|assignment| {
            assignment.kind != AssignmentKind::Review
                || assignment.status != AssignmentStatus::Active
        }));
    }

    #[tokio::test]
    async fn concurrent_final_approvals_cannot_leave_the_task_submitted() {
        let (_temp, repo, image_id, task_id, _annotator, reviewers) =
            correction_repo(AnnotationType::BoundingBox, false).await;
        let mut metadata = repo.load_dataset_config().await.unwrap();
        metadata.tasks[0].review.required_reviews = 2;
        repo.save_dataset(&metadata).await.unwrap();
        let first = claim_review(&repo, &image_id, &task_id, &reviewers[0]).await;
        let second = claim_review(&repo, &image_id, &task_id, &reviewers[1]).await;
        let first_repo = repo.clone();
        let second_repo = repo.clone();
        let first_image = image_id.clone();
        let second_image = image_id.clone();
        let first_task = task_id.clone();
        let second_task = task_id.clone();
        let first_user = reviewers[0].clone();
        let second_user = reviewers[1].clone();

        let (left, right) = tokio::join!(
            async move {
                record_task_approval(
                    &first_repo,
                    &first_image,
                    &first_task,
                    &first_user,
                    &first,
                    "rev_first",
                )
                .await
            },
            async move {
                record_task_approval(
                    &second_repo,
                    &second_image,
                    &second_task,
                    &second_user,
                    &second,
                    "rev_second",
                )
                .await
            }
        );

        assert!(left.is_ok(), "{left:?}");
        assert!(right.is_ok(), "{right:?}");
        let state = repo.load_image_state(&image_id).await.unwrap();
        assert_eq!(state.task_states[&task_id].status, TaskStatus::Completed);
        assert_eq!(
            state.task_states[&task_id].outcome,
            Some(TaskOutcome::Approved)
        );
        assert_eq!(task_approval_count(&state.reviews, &task_id), 2);
        assert!(
            state
                .assignments
                .iter()
                .filter(|assignment| {
                    assignment.task_id == task_id && assignment.kind == AssignmentKind::Review
                })
                .all(|assignment| assignment.status == AssignmentStatus::Completed)
        );
    }

    async fn claim_review(
        repo: &DatasetRepository,
        image_id: &ImageId,
        task_id: &TaskId,
        reviewer: &UserId,
    ) -> Assignment {
        let assignment = repo
            .assign_next_image(reviewer, task_id, AssignmentKind::Review)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&assignment.image_id, image_id);
        assignment
    }

    #[allow(clippy::too_many_arguments)]
    async fn correct(
        repo: &DatasetRepository,
        image_id: &ImageId,
        task_id: &TaskId,
        reviewer: &UserId,
        assignment: &Assignment,
        correction_id: &CorrectionId,
        expected_version: u32,
        geometry: AnnotationGeometry,
    ) -> StorageResult<EventLogEntry> {
        repo.correct_review_annotation(
            reviewer,
            AssignmentContext {
                assignment_id: &assignment.assignment_id,
                image_id,
                task_id,
                kind: AssignmentKind::Review,
            },
            correction_id,
            &AnnotationId::from("ann_1"),
            expected_version,
            geometry,
            Some("quality correction".to_string()),
        )
        .await
    }

    async fn record_task_approval(
        repo: &DatasetRepository,
        image_id: &ImageId,
        task_id: &TaskId,
        reviewer: &UserId,
        assignment: &Assignment,
        review_id: &str,
    ) -> StorageResult<labello_domain::ImageState> {
        repo.record_review_for_assignment(
            reviewer,
            AssignmentContext {
                assignment_id: &assignment.assignment_id,
                image_id,
                task_id,
                kind: AssignmentKind::Review,
            },
            ReviewRecord {
                review_id: ReviewId::from(review_id),
                target: ReviewTarget::Task {
                    task_id: task_id.clone(),
                },
                reviewer_user_id: reviewer.clone(),
                decision: ReviewDecision::Approved,
                timestamp: now(),
                comment: None,
            },
        )
        .await
    }

    fn assignment_status(
        state: &labello_domain::ImageState,
        assignment: &Assignment,
    ) -> AssignmentStatus {
        state
            .assignments
            .iter()
            .find(|candidate| candidate.assignment_id == assignment.assignment_id)
            .unwrap()
            .status
            .clone()
    }

    async fn correction_repo(
        annotation_type: AnnotationType,
        allow_reviewer_corrections: bool,
    ) -> (
        tempfile::TempDir,
        DatasetRepository,
        ImageId,
        TaskId,
        UserId,
        [UserId; 2],
    ) {
        let temp = tempfile::tempdir().unwrap();
        let repo = DatasetRepository::new(temp.path());
        let image_id = ImageId::from("img_1");
        let task_id = TaskId::from(match annotation_type {
            AnnotationType::BoundingBox => "bounding_box:person",
            AnnotationType::Skeleton => "skeleton:person",
        });
        let class_id = ClassId::from("person");
        let annotator = UserId::from("annotator");
        let reviewers = [UserId::from("reviewer_1"), UserId::from("reviewer_2")];
        let mut metadata = DatasetMetadata::new(DatasetId::from("ds"), "Dataset", now());
        metadata.label_classes.push(LabelClass {
            class_id: class_id.clone(),
            name: "Person".to_string(),
            color: "#5eead4".to_string(),
            description: None,
        });
        metadata.tasks.push(TaskDefinition {
            task_id: task_id.clone(),
            name: "Person annotation".to_string(),
            annotation_type: annotation_type.clone(),
            class_ids: vec![class_id.clone()],
            instructions: TutorialContent {
                title: "Instructions".to_string(),
                example_text: "Annotate the person.".to_string(),
                example_images: Vec::new(),
            },
            skeleton: (annotation_type == AnnotationType::Skeleton).then_some(SkeletonSpec {
                keypoints: vec![KeypointSpec {
                    name: "nose".to_string(),
                    required: true,
                }],
                edges: Vec::new(),
                allow_hidden: true,
                allow_absent: false,
            }),
            review: ReviewConfig {
                required_reviews: 1,
                workflow: ReviewWorkflow::Approval,
                allow_reviewer_corrections,
                agreement_threshold: None,
            },
            prelabel_config_ids: Vec::new(),
            manual_box_guide_migration: None,
            enabled: true,
        });
        metadata.role_assignments.push(DatasetRoleAssignment {
            dataset_id: metadata.dataset_id.clone(),
            user_id: annotator.clone(),
            roles: BTreeSet::from([DatasetRole::Annotator]),
            assigned_at: now(),
            assigned_by: None,
        });
        metadata
            .role_assignments
            .extend(reviewers.iter().map(|reviewer| DatasetRoleAssignment {
                dataset_id: metadata.dataset_id.clone(),
                user_id: reviewer.clone(),
                roles: BTreeSet::from([DatasetRole::Reviewer]),
                assigned_at: now(),
                assigned_by: None,
            }));
        repo.initialize(metadata).await.unwrap();
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
                    width: 100,
                    height: 100,
                    media_type: "image/png".to_string(),
                    source_memberships: None,
                },
            )]),
        })
        .await
        .unwrap();
        let timestamp = now();
        let geometry = match annotation_type {
            AnnotationType::BoundingBox => AnnotationGeometry::BoundingBox(BoundingBox {
                x: 0.1,
                y: 0.1,
                width: 0.2,
                height: 0.2,
            }),
            AnnotationType::Skeleton => AnnotationGeometry::Skeleton(SkeletonGeometry {
                keypoints: vec![KeypointAnnotation {
                    name: "nose".to_string(),
                    state: KeypointState::Visible,
                    point: Some(NormalizedPoint { x: 0.5, y: 0.5 }),
                }],
            }),
        };
        repo.append_payload(
            &image_id,
            &Actor {
                user_id: annotator.clone(),
                role: DatasetRole::Annotator,
            },
            EventPayload::AnnotationVersionCreated {
                annotation: AnnotationVersion {
                    annotation_id: AnnotationId::from("ann_1"),
                    version: 1,
                    object_group_id: None,
                    origin: AnnotationOrigin::native(),
                    task_id: task_id.clone(),
                    class_id,
                    annotation_type,
                    revision_source: RevisionSource::Human {
                        action: HumanRevisionKind::Authored,
                    },
                    geometry,
                    author_user_id: annotator.clone(),
                    created_at: timestamp,
                    updated_at: timestamp,
                    deleted: false,
                },
                previous_version: None,
                reason: None,
            },
        )
        .await
        .unwrap();
        repo.append_payload(
            &image_id,
            &Actor {
                user_id: annotator.clone(),
                role: DatasetRole::Annotator,
            },
            EventPayload::TaskStateChanged {
                task_state: TaskState {
                    task_id: task_id.clone(),
                    status: TaskStatus::Submitted,
                    outcome: None,
                    assigned_to: None,
                    completed_by: Some(annotator.clone()),
                    completed_at: Some(timestamp),
                    updated_at: timestamp,
                },
            },
        )
        .await
        .unwrap();
        (temp, repo, image_id, task_id, annotator, reviewers)
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
            manual_box_guide_migration: None,
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
                            source_memberships: None,
                        },
                    )
                })
                .collect(),
        })
        .await
        .unwrap();
        (temp, repo, task_id, users)
    }

    async fn expire_assignment(
        repo: &DatasetRepository,
        assignment: &Assignment,
        user_id: &UserId,
    ) {
        let mut expired = assignment.clone();
        expired.expires_at = Some(now() - std::time::Duration::from_secs(1));
        repo.append_payload(
            &assignment.image_id,
            &Actor {
                user_id: user_id.clone(),
                role: role_for_kind(&assignment.kind),
            },
            EventPayload::AssignmentUpdated {
                assignment: expired,
            },
        )
        .await
        .unwrap();
    }
}
