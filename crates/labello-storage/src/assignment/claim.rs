use super::migration::{has_migration_final_review_by_user, migration_final_approval_count};
use super::*;

const MAX_AVAILABILITY_SCAN_WORKERS: usize = 32;

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
        self.assignment_availability_cache
            .store_batch_if_current(
                generation,
                availabilities
                    .iter()
                    .map(|(kind, tasks)| {
                        (
                            (user_id.clone(), assignment_kind_cache_key(kind).to_string()),
                            tasks.clone(),
                        )
                    })
                    .collect(),
            )
            .await;
        Ok(availabilities)
    }

    async fn cached_assignment_availabilities(
        &self,
        user_id: &UserId,
        kinds: &[AssignmentKind],
        generation: u64,
    ) -> Option<Vec<(AssignmentKind, std::collections::BTreeMap<TaskId, bool>)>> {
        let keys = kinds
            .iter()
            .map(|kind| (user_id.clone(), assignment_kind_cache_key(kind).to_string()))
            .collect::<Vec<_>>();
        self.assignment_availability_cache
            .lookup_batch(&keys, generation)
            .await
            .map(|tasks| kinds.iter().cloned().zip(tasks).collect())
    }

    async fn compute_assignment_availabilities(
        &self,
        user_id: &UserId,
        kinds: &[AssignmentKind],
    ) -> StorageResult<Vec<(AssignmentKind, std::collections::BTreeMap<TaskId, bool>)>> {
        let metadata = std::sync::Arc::new(self.load_dataset().await?);

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

        if work.iter().all(|(_, _, unresolved)| unresolved.is_empty()) {
            return Ok(work
                .into_iter()
                .map(|(kind, availability, _)| (kind, availability))
                .collect());
        }

        let eligible = std::sync::Arc::new(
            work.iter()
                .filter(|(_, _, unresolved)| !unresolved.is_empty())
                .map(|(kind, _, unresolved)| (kind.clone(), unresolved.clone()))
                .collect::<Vec<_>>(),
        );
        let mut image_ids = metadata.images.keys().cloned();
        let mut workers = tokio::task::JoinSet::new();
        for image_id in image_ids.by_ref().take(MAX_AVAILABILITY_SCAN_WORKERS) {
            workers.spawn(self.availability_scan_worker(
                image_id,
                metadata.clone(),
                eligible.clone(),
                user_id.clone(),
            ));
        }

        while let Some(result) = workers.join_next().await {
            let available = result.map_err(|error| {
                StorageError::BackgroundTask(format!(
                    "assignment availability worker failed: {error}"
                ))
            })??;
            for (kind, task_ids) in available {
                let (_, availability, unresolved) = work
                    .iter_mut()
                    .find(|(candidate, _, _)| *candidate == kind)
                    .expect("availability worker must return a requested kind");
                for task_id in task_ids {
                    unresolved.remove(&task_id);
                    availability.insert(task_id, true);
                }
            }
            if work.iter().all(|(_, _, unresolved)| unresolved.is_empty()) {
                workers.abort_all();
                break;
            }
            if let Some(image_id) = image_ids.next() {
                workers.spawn(self.availability_scan_worker(
                    image_id,
                    metadata.clone(),
                    eligible.clone(),
                    user_id.clone(),
                ));
            }
        }
        Ok(work
            .into_iter()
            .map(|(kind, availability, _)| (kind, availability))
            .collect())
    }

    fn availability_scan_worker(
        &self,
        image_id: ImageId,
        metadata: std::sync::Arc<DatasetMetadata>,
        eligible: std::sync::Arc<Vec<(AssignmentKind, std::collections::BTreeSet<TaskId>)>>,
        user_id: UserId,
    ) -> impl Future<Output = StorageResult<Vec<(AssignmentKind, Vec<TaskId>)>>> + Send + 'static
    {
        let repository = self.clone();
        async move {
            let lock = repository.image_lock(&image_id);
            let _guard = lock.lock().await;
            let state = repository.load_image_state(&image_id).await?;
            let now = labello_domain::now();
            let mut available_by_kind = Vec::with_capacity(eligible.len());
            for (kind, task_ids) in eligible.iter() {
                let mut available = Vec::new();
                for task in metadata
                    .tasks
                    .iter()
                    .filter(|task| task_ids.contains(&task.task_id))
                {
                    let status = effective_assignment_status(&state, &task.task_id, kind, now);
                    if repository
                        .image_accepts_assignment(
                            &image_id, &state, task, &user_id, kind, &status, now,
                        )
                        .await?
                    {
                        available.push(task.task_id.clone());
                    }
                }
                available_by_kind.push((kind.clone(), available));
            }
            Ok(available_by_kind)
        }
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

        let image_ids = metadata.images.keys().cloned().collect::<Vec<_>>();
        if image_ids.is_empty() {
            return Ok(None);
        }
        let cursor_key = format!(
            "{user_id}\u{1f}{task_id}\u{1f}{}",
            assignment_kind_cache_key(&kind)
        );
        let start = self
            .assignment_cursors
            .lock()
            .get(&cursor_key)
            .copied()
            .unwrap_or_default()
            % image_ids.len();

        for offset in 0..image_ids.len() {
            let image_index = (start + offset) % image_ids.len();
            let image_id = &image_ids[image_index];
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
                self.assignment_cursors
                    .lock()
                    .insert(cursor_key.clone(), image_index);
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
            self.assignment_cursors
                .lock()
                .insert(cursor_key.clone(), image_index);
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
        if *kind == AssignmentKind::Adjudication {
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
            && self
                .task_is_overrepresented(metadata, &task.task_id, kind)
                .await?
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
        if active_assignment_for_user(&state.assignments, task_id, user_id, kind, now).is_some() {
            return Ok(true);
        }
        if has_conflicting_assignment(&state.assignments, task_id, user_id, kind, now) {
            return Ok(false);
        }
        if !status_matches_kind(status, kind) {
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
        Ok(true)
    }

    async fn task_is_overrepresented(
        &self,
        metadata: &DatasetMetadata,
        selected_task_id: &TaskId,
        kind: &AssignmentKind,
    ) -> StorageResult<bool> {
        let Some(config) = metadata.imbalance.as_ref() else {
            return Ok(false);
        };
        let max_ratio = crate::completion_projection::validated_max_ratio(config)?;
        let counts = if *kind == AssignmentKind::Annotation {
            self.task_annotation_counts().await?
        } else {
            self.task_completion_counts().await?
        };
        let selected = counts.get(selected_task_id).copied().unwrap_or_default();
        let mut other_counts = metadata
            .tasks
            .iter()
            .filter(|task| task.enabled && &task.task_id != selected_task_id)
            .map(|task| counts.get(&task.task_id).copied().unwrap_or_default());
        let Some(min_other) = other_counts
            .next()
            .map(|first| other_counts.fold(first, usize::min))
        else {
            return Ok(false);
        };
        if min_other == 0 {
            Ok(selected > 0)
        } else {
            Ok((selected as f64 / min_other as f64) > max_ratio)
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
