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
mod tests;
