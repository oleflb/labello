use super::migration::{has_migration_final_review_by_user, migration_final_approval_count};
use super::*;

fn assignment_kind_cache_key(kind: &AssignmentKind) -> &'static str {
    match kind {
        AssignmentKind::Annotation => "annotation",
        AssignmentKind::Review => "review",
        AssignmentKind::Adjudication => "adjudication",
    }
}

impl DatasetRepository {
    pub async fn assignment_availability(
        &self,
        user_id: &UserId,
        kind: AssignmentKind,
    ) -> StorageResult<std::collections::BTreeMap<TaskId, bool>> {
        Ok(self
            .assignment_availabilities(user_id, kind.clone())
            .await?
            .into_iter()
            .find_map(|(computed_kind, tasks)| (computed_kind == kind).then_some(tasks))
            .expect("the required assignment kind must be included"))
    }

    pub async fn assignment_availabilities(
        &self,
        user_id: &UserId,
        requested_kind: AssignmentKind,
    ) -> StorageResult<Vec<(AssignmentKind, std::collections::BTreeMap<TaskId, bool>)>> {
        let config = self.load_dataset_config().await?;
        require_role(
            &config.role_assignments,
            &config.dataset_id,
            user_id,
            role_for_kind(&requested_kind),
        )?;
        let requested_generation = self.assignment_availability_cache.generation();
        let kinds = [
            AssignmentKind::Annotation,
            AssignmentKind::Review,
            AssignmentKind::Adjudication,
        ]
        .into_iter()
        .filter(|candidate| {
            config.role_assignments.iter().any(|assignment| {
                assignment.dataset_id == config.dataset_id
                    && assignment.user_id == *user_id
                    && assignment.roles.contains(&role_for_kind(candidate))
            })
        })
        .collect::<Vec<_>>();
        if let Some(cached) = self
            .cached_assignment_availabilities(user_id, &kinds, requested_generation)
            .await
        {
            return Ok(cached);
        }

        let _refresh = self.assignment_availability_cache.lock_refresh().await;
        let generation = self.assignment_availability_cache.generation();
        if let Some(cached) = self
            .cached_assignment_availabilities(user_id, &kinds, generation)
            .await
        {
            return Ok(cached);
        }

        #[cfg(test)]
        self.assignment_availability_cache.record_scan();
        let availabilities = self
            .compute_assignment_availabilities(user_id, &kinds)
            .await?;
        for (kind, tasks) in &availabilities {
            self.assignment_availability_cache
                .store(
                    (user_id.clone(), assignment_kind_cache_key(kind).to_string()),
                    generation,
                    tasks.clone(),
                )
                .await;
        }
        Ok(availabilities)
    }

    async fn cached_assignment_availabilities(
        &self,
        user_id: &UserId,
        kinds: &[AssignmentKind],
        generation: u64,
    ) -> Option<Vec<(AssignmentKind, std::collections::BTreeMap<TaskId, bool>)>> {
        let mut cached = Vec::with_capacity(kinds.len());
        for kind in kinds {
            let key = (user_id.clone(), assignment_kind_cache_key(kind).to_string());
            cached.push((
                kind.clone(),
                self.assignment_availability_cache
                    .get(&key, generation)
                    .await?,
            ));
        }
        Some(cached)
    }

    async fn compute_assignment_availabilities(
        &self,
        user_id: &UserId,
        kinds: &[AssignmentKind],
    ) -> StorageResult<Vec<(AssignmentKind, std::collections::BTreeMap<TaskId, bool>)>> {
        let metadata = self.load_dataset().await?;

        let mut work = Vec::with_capacity(kinds.len());
        for kind in kinds {
            let mut availability = std::collections::BTreeMap::new();
            let mut unresolved = std::collections::BTreeSet::new();
            for task in &metadata.tasks {
                if self.task_accepts_assignment(&metadata, task, kind).await? {
                    unresolved.insert(task.task_id.clone());
                }
                availability.insert(task.task_id.clone(), false);
            }
            work.push((kind.clone(), availability, unresolved));
        }

        for image_id in metadata.images.keys() {
            if work.iter().all(|(_, _, unresolved)| unresolved.is_empty()) {
                break;
            }
            let lock = self.image_lock(image_id);
            let _guard = lock.lock().await;
            let state = self.load_image_state(image_id).await?;
            let now = labello_domain::now();
            for (kind, availability, unresolved) in &mut work {
                let kind = kind.clone();
                let pending = unresolved.clone();
                let mut available = Vec::new();
                for task in metadata
                    .tasks
                    .iter()
                    .filter(|task| pending.contains(&task.task_id))
                {
                    let status = effective_assignment_status(&state, &task.task_id, &kind, now);
                    if self
                        .image_accepts_assignment(
                            image_id, &state, task, user_id, &kind, &status, now,
                        )
                        .await?
                    {
                        available.push(task.task_id.clone());
                    }
                }
                for task_id in available {
                    unresolved.remove(&task_id);
                    availability.insert(task_id, true);
                }
            }
        }
        Ok(work
            .into_iter()
            .map(|(kind, availability, _)| (kind, availability))
            .collect())
    }

    /// Return an exact still-active assignment without renewing it. Browser
    /// restoration uses this before loading image state so the validation base
    /// sequence is not changed by the reclaim itself.
    pub async fn reclaim_assignment(
        &self,
        user_id: &UserId,
        assignment_id: &AssignmentId,
        task_id: &TaskId,
        kind: AssignmentKind,
    ) -> StorageResult<Option<Assignment>> {
        let metadata = self.load_dataset().await?;
        let required_role = role_for_kind(&kind);
        require_role(
            &metadata.role_assignments,
            &metadata.dataset_id,
            user_id,
            required_role,
        )?;
        let task = metadata
            .task(task_id)
            .ok_or_else(|| StorageError::Unauthorized(format!("task {task_id} does not exist")))?;
        if !task.enabled {
            return Ok(None);
        }
        for image_id in metadata.images.keys() {
            let state = self.load_image_state(image_id).await?;
            if !state
                .assignments
                .iter()
                .any(|assignment| assignment.assignment_id == *assignment_id)
            {
                continue;
            }
            let status = state
                .task_states
                .get(task_id)
                .map(|task_state| &task_state.status)
                .unwrap_or(&TaskStatus::Pending);
            let eligible = match &kind {
                AssignmentKind::Annotation => state.assignment_eligible(task_id),
                AssignmentKind::Review => status == &TaskStatus::Submitted,
                AssignmentKind::Adjudication => status == &TaskStatus::AdjudicationRequired,
            };
            if !eligible {
                return Ok(None);
            }
            return Ok(exact_active_assignment(
                &state.assignments,
                assignment_id,
                image_id,
                task_id,
                user_id,
                &kind,
                labello_domain::now(),
            )
            .ok()
            .cloned());
        }
        Ok(None)
    }

    pub async fn assign_next_image(
        &self,
        user_id: &UserId,
        task_id: &TaskId,
        kind: AssignmentKind,
    ) -> StorageResult<Option<Assignment>> {
        self.assign_next_image_excluding(user_id, task_id, kind, &[])
            .await
    }

    pub async fn assign_next_image_excluding(
        &self,
        user_id: &UserId,
        task_id: &TaskId,
        kind: AssignmentKind,
        excluded_image_ids: &[ImageId],
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
        if !self.task_accepts_assignment(&metadata, task, &kind).await? {
            return Ok(None);
        }

        for image_id in metadata.images.keys() {
            if excluded_image_ids.contains(image_id) {
                continue;
            }
            let lock = self.image_lock(image_id);
            let _guard = lock.lock().await;
            let state = self.load_image_state(image_id).await?;
            let now = labello_domain::now();
            let actor = Actor {
                user_id: user_id.clone(),
                role: required_role.clone(),
            };
            let mut payloads = expired_assignment_payloads(&state.assignments, task_id, &kind, now);
            let status = effective_assignment_status(&state, task_id, &kind, now);
            if kind == AssignmentKind::Annotation
                && status == TaskStatus::Pending
                && state
                    .task_states
                    .get(task_id)
                    .is_some_and(|state| state.status == TaskStatus::InProgress)
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
            if !self
                .image_accepts_assignment(image_id, &state, task, user_id, &kind, &status, now)
                .await?
            {
                if !payloads.is_empty() {
                    self.append_payloads_unlocked(image_id, &actor, payloads)
                        .await?;
                }
                continue;
            }
            if let Some(assignment) =
                active_assignment_for_user(&state.assignments, task_id, user_id, &kind, now)
            {
                let mut assignment = assignment.clone();
                renew_assignment(&mut assignment, now);
                payloads.push(EventPayload::AssignmentUpdated {
                    assignment: assignment.clone(),
                });
                self.append_payloads_unlocked(image_id, &actor, payloads)
                    .await?;
                return Ok(Some(assignment));
            }
            let assignment = Assignment {
                assignment_id: AssignmentId::generate(),
                image_id: image_id.clone(),
                task_id: task.task_id.clone(),
                assigned_to: user_id.clone(),
                kind: kind.clone(),
                status: AssignmentStatus::Active,
                expires_at: Some(lease_expiration(now)),
                created_at: now,
                updated_at: now,
            };
            payloads.push(EventPayload::AssignmentUpdated {
                assignment: assignment.clone(),
            });
            if kind == AssignmentKind::Annotation && status == TaskStatus::Pending {
                let task_state = TaskState {
                    task_id: task_id.clone(),
                    status: TaskStatus::InProgress,
                    outcome: None,
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

    async fn task_accepts_assignment(
        &self,
        metadata: &DatasetMetadata,
        task: &TaskDefinition,
        kind: &AssignmentKind,
    ) -> StorageResult<bool> {
        if !task.enabled {
            return Ok(false);
        }
        if task.review.workflow == ReviewWorkflow::IndependentAgreement {
            return Err(StorageError::InvalidAssignment(format!(
                "independent agreement workflow is not implemented for task {}",
                task.task_id
            )));
        }
        if task.class_ids.len() != 1 {
            return Err(StorageError::InvalidAssignment(format!(
                "enabled task {} must have exactly one class",
                task.task_id
            )));
        }
        if *kind == AssignmentKind::Review && task.review.workflow == ReviewWorkflow::None {
            return Ok(false);
        }
        if metadata
            .imbalance
            .as_ref()
            .is_some_and(|config| config.enforce)
            && self.task_is_overrepresented(&task.task_id).await?
        {
            return Ok(false);
        }
        Ok(true)
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "assignment eligibility is a leaf policy check over explicit claim inputs"
    )]
    async fn image_accepts_assignment(
        &self,
        image_id: &ImageId,
        state: &labello_domain::ImageState,
        task: &TaskDefinition,
        user_id: &UserId,
        kind: &AssignmentKind,
        status: &TaskStatus,
        now: labello_domain::Timestamp,
    ) -> StorageResult<bool> {
        let task_id = &task.task_id;
        if *kind == AssignmentKind::Annotation && !state.assignment_eligible(task_id) {
            return Ok(false);
        }
        if *kind == AssignmentKind::Review {
            let already_final = if task.manual_box_guide_migration.is_some() {
                let events = self.load_events(image_id).await?;
                has_migration_final_review_by_user(&events, task_id, user_id)
                    || migration_final_approval_count(&events, task_id)
                        >= task.review.required_reviews
            } else {
                let reviews = self.current_task_reviews(image_id, task_id).await?;
                has_task_review_by_user(&reviews, task_id, user_id)
                    || task_approval_count(&reviews, task_id) >= task.review.required_reviews
            };
            if already_final {
                return Ok(false);
            }
        }
        if active_assignment_for_user(&state.assignments, task_id, user_id, kind, now).is_some() {
            return Ok(true);
        }
        if has_conflicting_assignment(&state.assignments, task_id, user_id, kind, now) {
            return Ok(false);
        }
        Ok(status_matches_kind(status, kind))
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

fn effective_assignment_status(
    state: &labello_domain::ImageState,
    task_id: &TaskId,
    kind: &AssignmentKind,
    now: labello_domain::Timestamp,
) -> TaskStatus {
    let status = state
        .task_states
        .get(task_id)
        .map(|state| state.status.clone())
        .unwrap_or(TaskStatus::Pending);
    if *kind == AssignmentKind::Annotation
        && status == TaskStatus::InProgress
        && state.assignments.iter().any(|assignment| {
            assignment.task_id == *task_id
                && assignment.kind == *kind
                && assignment.status == AssignmentStatus::Active
                && assignment_is_expired(assignment, now)
        })
        && !has_active_unexpired_assignment(&state.assignments, task_id, kind, now)
    {
        TaskStatus::Pending
    } else {
        status
    }
}

fn active_assignment_for_user<'a>(
    assignments: &'a [Assignment],
    task_id: &TaskId,
    user_id: &UserId,
    kind: &AssignmentKind,
    now: labello_domain::Timestamp,
) -> Option<&'a Assignment> {
    assignments.iter().find(|assignment| {
        &assignment.task_id == task_id
            && &assignment.kind == kind
            && assignment.status == AssignmentStatus::Active
            && &assignment.assigned_to == user_id
            && !assignment_is_expired(assignment, now)
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
    now: labello_domain::Timestamp,
) -> bool {
    if *kind == AssignmentKind::Review {
        return false;
    }
    assignments.iter().any(|assignment| {
        &assignment.task_id == task_id
            && &assignment.kind == kind
            && assignment.status == AssignmentStatus::Active
            && &assignment.assigned_to != user_id
            && !assignment_is_expired(assignment, now)
    })
}

fn has_active_unexpired_assignment(
    assignments: &[Assignment],
    task_id: &TaskId,
    kind: &AssignmentKind,
    now: labello_domain::Timestamp,
) -> bool {
    assignments.iter().any(|assignment| {
        assignment.task_id == *task_id
            && assignment.kind == *kind
            && assignment.status == AssignmentStatus::Active
            && !assignment_is_expired(assignment, now)
    })
}

fn expired_assignment_payloads(
    assignments: &[Assignment],
    task_id: &TaskId,
    kind: &AssignmentKind,
    now: labello_domain::Timestamp,
) -> Vec<EventPayload> {
    assignments
        .iter()
        .filter(|assignment| {
            assignment.task_id == *task_id
                && assignment.kind == *kind
                && assignment.status == AssignmentStatus::Active
                && assignment_is_expired(assignment, now)
        })
        .map(|assignment| {
            let mut assignment = assignment.clone();
            assignment.status = AssignmentStatus::Cancelled;
            assignment.updated_at = now;
            EventPayload::AssignmentUpdated { assignment }
        })
        .collect()
}
