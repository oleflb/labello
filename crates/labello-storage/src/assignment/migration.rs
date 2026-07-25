use labello_domain::{
    AnnotationGeometry, AnnotationOrigin, AnnotationType, AnnotationVersion, Assignment,
    AssignmentId, AssignmentKind, AssignmentStatus, ClassId, DatasetMetadata, DatasetRole, EventId,
    EventLogEntry, EventPayload, HumanRevisionKind, ImageId, ImageState, MigrationConfirmation,
    MigrationCursor, MigrationDependencyKind, MigrationDisposition, MigrationDispositionStatus,
    MigrationExclusion, MigrationExclusionReason, MigrationHash, MigrationPass, MigrationPassId,
    MigrationPassItem, MigrationPassItemAction, ObjectGroupId, ReviewDecision, ReviewId,
    ReviewRecord, ReviewTarget, ReviewWorkflow, RevisionSource, SkeletonGeometry, TaskDefinition,
    TaskId, TaskOutcome, TaskState, TaskStatus, Timestamp, UserId, migration_confirmation_hash,
    require_role,
};

use super::{
    AssignmentContext, DatasetRepository, StorageError, StorageResult, exact_active_assignment,
    lease_expiration,
};

const MAX_IDEMPOTENCY_KEY_BYTES: usize = 200;
const MAX_EXCLUSION_NOTE_BYTES: usize = 2_000;
const MAX_REVIEW_COMMENT_BYTES: usize = 2_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationTargetExpectation {
    pub object_group_id: ObjectGroupId,
    pub expected_guide_annotation_version: u32,
    pub expected_guide_deleted: bool,
    pub expected_disposition_version: u32,
    pub expected_skeleton_version: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ManualMigrationProgress {
    pub expected: u64,
    pub annotated: u64,
    pub excluded: u64,
    pub pending: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MigrationReviewTarget {
    Disposition {
        object_group_id: ObjectGroupId,
        disposition_version: u32,
    },
    Confirmation {
        confirmation_hash: MigrationHash,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ManualMigrationCommandResult {
    pub image_state: ImageState,
    pub cursor: MigrationCursor,
    pub progress: ManualMigrationProgress,
    pub active_pass: Option<MigrationPass>,
    pub confirmation: Option<MigrationConfirmation>,
    pub assignment: Option<Assignment>,
    pub annotation_id: Option<labello_domain::AnnotationId>,
}

impl DatasetRepository {
    pub async fn current_manual_migration(
        &self,
        user_id: &UserId,
        context: AssignmentContext<'_>,
        pass_id: Option<&MigrationPassId>,
    ) -> StorageResult<ManualMigrationCommandResult> {
        let metadata = self.load_dataset().await?;
        let role = migration_role(&context.kind)?;
        require_role(
            &metadata.role_assignments,
            &metadata.dataset_id,
            user_id,
            role,
        )?;
        let lock = self.image_lock(context.image_id);
        let _guard = lock.lock().await;
        let state = self.load_image_state(context.image_id).await?;
        validate_migration_task(&metadata, context.image_id, context.task_id)?;
        exact_active_assignment(
            &state.assignments,
            context.assignment_id,
            context.image_id,
            context.task_id,
            user_id,
            &context.kind,
            labello_domain::now(),
        )?;
        if context.kind == AssignmentKind::Review {
            let events = self.load_events(context.image_id).await?;
            let cursor = match canonical_review_target(&state, &events, context.task_id, user_id)? {
                CanonicalReviewTarget::Object {
                    object_group_id, ..
                } => MigrationCursor::Object {
                    sequence_index: migration_target(&state, context.task_id, &object_group_id)?
                        .sequence_index,
                    object_group_id,
                },
                CanonicalReviewTarget::Confirmation { .. } => MigrationCursor::FullImage,
            };
            let mut result = command_result(state, context.task_id, None, None, None)?;
            result.cursor = cursor;
            Ok(result)
        } else {
            command_result(state, context.task_id, pass_id, None, None)
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn save_migration_skeleton(
        &self,
        user_id: &UserId,
        context: AssignmentContext<'_>,
        pass_id: Option<&MigrationPassId>,
        expected: &MigrationTargetExpectation,
        skeleton: SkeletonGeometry,
        idempotency_key: &str,
    ) -> StorageResult<ManualMigrationCommandResult> {
        validate_idempotency_key(idempotency_key)?;
        let metadata = self.load_dataset().await?;
        let (task, guide_task, image_dimensions) =
            migration_metadata(&metadata, context.image_id, context.task_id)?;
        require_annotation_context(&metadata, user_id, &context)?;
        let lock = self.image_lock(context.image_id);
        let _guard = lock.lock().await;
        let mut state = self.load_image_state(context.image_id).await?;
        let primary_id = command_event_id(user_id, context.assignment_id, idempotency_key, None);
        if let Some(command) = find_command(self, context.image_id, &primary_id).await? {
            let matches = matches!(
                &command.primary.payload,
                EventPayload::AnnotationVersionCreated { annotation, .. }
                    if annotation.task_id == *context.task_id
                        && annotation.object_group_id.as_ref() == Some(&expected.object_group_id)
                        && annotation.geometry == AnnotationGeometry::Skeleton(skeleton.clone())
            ) && expectation_matches(&command.before, context.task_id, expected)
                && pass_request_matches(&command.events, pass_id);
            return replay_retry(
                matches,
                state,
                context.task_id,
                pass_id,
                Some(context.assignment_id),
                command.primary.payload.task_annotation_id(),
            );
        }

        let now = labello_domain::now();
        let mut assignment =
            validate_annotation_command(&state, &context, user_id, pass_id, expected, now)?;
        let target = migration_target(&state, context.task_id, &expected.object_group_id)?.clone();
        validate_target_identity(&state, task, guide_task, &target, expected)?;
        if expected.expected_guide_deleted {
            return Err(conflict("the canonical migration guide is deleted"));
        }

        let current = state
            .current_annotation(&target.reserved_skeleton_annotation_id)
            .cloned();
        let mut payloads = Vec::new();
        if has_dependency(&state, context.task_id, &expected.object_group_id) {
            make_dependency_clearable(
                &mut state,
                context.task_id,
                &expected.object_group_id,
                &target,
                &mut payloads,
                user_id,
                now,
            )?;
        }

        let current = state
            .current_annotation(&target.reserved_skeleton_annotation_id)
            .cloned()
            .or(current);
        let (version, previous_version, created_at, action) = match current.as_ref() {
            Some(annotation) => (
                annotation.version + 1,
                Some(annotation.version),
                annotation.created_at,
                if annotation.geometry == AnnotationGeometry::Skeleton(skeleton.clone()) {
                    HumanRevisionKind::AcceptedUnchanged
                } else {
                    HumanRevisionKind::Edited
                },
            ),
            None => (1, None, now, HumanRevisionKind::Authored),
        };
        let annotation = AnnotationVersion {
            annotation_id: target.reserved_skeleton_annotation_id.clone(),
            version,
            object_group_id: Some(target.object_group_id.clone()),
            origin: AnnotationOrigin::native(),
            task_id: context.task_id.clone(),
            class_id: only_class(task)?.clone(),
            annotation_type: AnnotationType::Skeleton,
            revision_source: RevisionSource::Human { action },
            geometry: AnnotationGeometry::Skeleton(skeleton),
            author_user_id: user_id.clone(),
            created_at,
            updated_at: now,
            deleted: false,
        };
        annotation
            .validate_for_task(task, image_dimensions)
            .map_err(|error| StorageError::InvalidAssignment(error.to_string()))?;
        push_simulated(
            &mut state,
            &mut payloads,
            user_id,
            DatasetRole::Annotator,
            now,
            EventPayload::AnnotationVersionCreated {
                annotation: annotation.clone(),
                previous_version,
                reason: None,
            },
        )?;
        let disposition = next_disposition(
            &state,
            context.task_id,
            &expected.object_group_id,
            MigrationDispositionStatus::Annotated {
                skeleton_annotation_id: annotation.annotation_id.clone(),
                skeleton_version: annotation.version,
            },
        )?;
        push_simulated(
            &mut state,
            &mut payloads,
            user_id,
            DatasetRole::Annotator,
            now,
            EventPayload::MigrationDispositionChanged {
                task_id: context.task_id.clone(),
                object_group_id: expected.object_group_id.clone(),
                disposition: disposition.clone(),
            },
        )?;
        if let Some(pass_id) = pass_id {
            let item = pass_item(
                &state,
                context.task_id,
                &expected.object_group_id,
                MigrationPassItemAction::Annotated,
                primary_id.clone(),
            )?;
            push_simulated(
                &mut state,
                &mut payloads,
                user_id,
                DatasetRole::Annotator,
                now,
                EventPayload::MigrationPassItemRecorded {
                    pass_id: pass_id.clone(),
                    item,
                },
            )?;
        }
        reopen_terminal_migration(&mut state, context.task_id, now, &mut payloads, user_id)?;
        renew(&mut assignment, now);
        payloads.push(EventPayload::AssignmentUpdated {
            assignment: assignment.clone(),
        });
        let primary_index = payloads.iter().position(|payload| {
            matches!(payload, EventPayload::AnnotationVersionCreated { annotation: value, .. } if value.annotation_id == annotation.annotation_id && value.version == annotation.version)
        }).expect("skeleton command contains its annotation event");
        let state = self
            .append_migration_command_unlocked(
                context.image_id,
                user_id,
                DatasetRole::Annotator,
                idempotency_key,
                context.assignment_id,
                payloads,
                primary_index,
                now,
            )
            .await?;
        command_result(
            state,
            context.task_id,
            pass_id,
            Some(assignment),
            Some(annotation.annotation_id),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn exclude_migration_target(
        &self,
        user_id: &UserId,
        context: AssignmentContext<'_>,
        pass_id: Option<&MigrationPassId>,
        expected: &MigrationTargetExpectation,
        reason: MigrationExclusionReason,
        note: Option<String>,
        idempotency_key: &str,
    ) -> StorageResult<ManualMigrationCommandResult> {
        validate_idempotency_key(idempotency_key)?;
        validate_note(reason, note.as_deref())?;
        let metadata = self.load_dataset().await?;
        let (task, guide_task, _) =
            migration_metadata(&metadata, context.image_id, context.task_id)?;
        require_annotation_context(&metadata, user_id, &context)?;
        let lock = self.image_lock(context.image_id);
        let _guard = lock.lock().await;
        let state = self.load_image_state(context.image_id).await?;
        let primary_id = command_event_id(user_id, context.assignment_id, idempotency_key, None);
        if let Some(command) = find_command(self, context.image_id, &primary_id).await? {
            let matches = matches!(
                &command.primary.payload,
                EventPayload::MigrationDispositionChanged { task_id, object_group_id, disposition }
                    if task_id == context.task_id
                        && object_group_id == &expected.object_group_id
                        && matches!(&disposition.status, MigrationDispositionStatus::Excluded { exclusion } if exclusion.reason == reason && exclusion.note == note)
            ) && expectation_matches(&command.before, context.task_id, expected)
                && pass_request_matches(&command.events, pass_id);
            return replay_retry(
                matches,
                state,
                context.task_id,
                pass_id,
                Some(context.assignment_id),
                None,
            );
        }
        let now = labello_domain::now();
        let mut assignment =
            validate_annotation_command(&state, &context, user_id, pass_id, expected, now)?;
        let target = migration_target(&state, context.task_id, &expected.object_group_id)?.clone();
        validate_target_identity(&state, task, guide_task, &target, expected)?;
        let mut next = state.clone();
        let mut payloads = Vec::new();
        if let Some(skeleton) = next
            .current_annotation(&target.reserved_skeleton_annotation_id)
            .filter(|annotation| !annotation.deleted)
            .cloned()
        {
            push_simulated(
                &mut next,
                &mut payloads,
                user_id,
                DatasetRole::Annotator,
                now,
                EventPayload::AnnotationDeleted {
                    annotation_id: skeleton.annotation_id,
                    version: skeleton.version,
                    reason: Some("manual migration exclusion".to_string()),
                },
            )?;
        }
        let disposition = next_disposition(
            &next,
            context.task_id,
            &expected.object_group_id,
            MigrationDispositionStatus::Excluded {
                exclusion: MigrationExclusion {
                    reason,
                    event_id: primary_id.clone(),
                    actor_user_id: user_id.clone(),
                    timestamp: now,
                    note,
                },
            },
        )?;
        push_simulated(
            &mut next,
            &mut payloads,
            user_id,
            DatasetRole::Annotator,
            now,
            EventPayload::MigrationDispositionChanged {
                task_id: context.task_id.clone(),
                object_group_id: expected.object_group_id.clone(),
                disposition,
            },
        )?;
        if let Some(marker) = next
            .migration_dependencies
            .get(context.task_id)
            .and_then(|markers| markers.get(&expected.object_group_id))
            .cloned()
            && (!expected.expected_guide_deleted
                || marker.kind == MigrationDependencyKind::GuideUnavailable)
        {
            push_simulated(
                &mut next,
                &mut payloads,
                user_id,
                DatasetRole::Annotator,
                now,
                EventPayload::MigrationDependencyCleared {
                    task_id: context.task_id.clone(),
                    object_group_id: expected.object_group_id.clone(),
                    marker_version: marker.marker_version,
                },
            )?;
        }
        if let Some(pass_id) = pass_id {
            let item = pass_item(
                &next,
                context.task_id,
                &expected.object_group_id,
                MigrationPassItemAction::Excluded,
                primary_id.clone(),
            )?;
            push_simulated(
                &mut next,
                &mut payloads,
                user_id,
                DatasetRole::Annotator,
                now,
                EventPayload::MigrationPassItemRecorded {
                    pass_id: pass_id.clone(),
                    item,
                },
            )?;
        }
        reopen_terminal_migration(&mut next, context.task_id, now, &mut payloads, user_id)?;
        renew(&mut assignment, now);
        payloads.push(EventPayload::AssignmentUpdated {
            assignment: assignment.clone(),
        });
        let primary_index = payloads
            .iter()
            .position(|payload| matches!(payload, EventPayload::MigrationDispositionChanged { disposition, .. } if matches!(disposition.status, MigrationDispositionStatus::Excluded { .. })))
            .expect("exclusion command contains a disposition event");
        let state = self
            .append_migration_command_unlocked(
                context.image_id,
                user_id,
                DatasetRole::Annotator,
                idempotency_key,
                context.assignment_id,
                payloads,
                primary_index,
                now,
            )
            .await?;
        command_result(state, context.task_id, pass_id, Some(assignment), None)
    }

    pub async fn reopen_migration_target(
        &self,
        user_id: &UserId,
        context: AssignmentContext<'_>,
        pass_id: Option<&MigrationPassId>,
        expected: &MigrationTargetExpectation,
        idempotency_key: &str,
    ) -> StorageResult<ManualMigrationCommandResult> {
        validate_idempotency_key(idempotency_key)?;
        let metadata = self.load_dataset().await?;
        let (task, guide_task, _) =
            migration_metadata(&metadata, context.image_id, context.task_id)?;
        require_annotation_context(&metadata, user_id, &context)?;
        let lock = self.image_lock(context.image_id);
        let _guard = lock.lock().await;
        let state = self.load_image_state(context.image_id).await?;
        let primary_id = command_event_id(user_id, context.assignment_id, idempotency_key, None);
        if let Some(command) = find_command(self, context.image_id, &primary_id).await? {
            let matches = matches!(&command.primary.payload, EventPayload::MigrationDispositionReopened { task_id, object_group_id, .. } if task_id == context.task_id && object_group_id == &expected.object_group_id)
                && expectation_matches(&command.before, context.task_id, expected);
            return replay_retry(
                matches,
                state,
                context.task_id,
                pass_id,
                Some(context.assignment_id),
                None,
            );
        }
        let now = labello_domain::now();
        let mut assignment =
            validate_annotation_command(&state, &context, user_id, pass_id, expected, now)?;
        let target = migration_target(&state, context.task_id, &expected.object_group_id)?;
        validate_target_identity(&state, task, guide_task, target, expected)?;
        let current = current_disposition(&state, context.task_id, &expected.object_group_id)?;
        if !matches!(current.status, MigrationDispositionStatus::Excluded { .. }) {
            return Err(conflict("only an exclusion can be reopened"));
        }
        let disposition = MigrationDisposition {
            disposition_version: current.disposition_version + 1,
            status: MigrationDispositionStatus::Pending,
        };
        let mut payloads = vec![EventPayload::MigrationDispositionReopened {
            task_id: context.task_id.clone(),
            object_group_id: expected.object_group_id.clone(),
            disposition,
        }];
        let mut next = state.clone();
        simulate_payloads(&mut next, user_id, DatasetRole::Annotator, now, &payloads)?;
        reopen_terminal_migration(&mut next, context.task_id, now, &mut payloads, user_id)?;
        renew(&mut assignment, now);
        payloads.push(EventPayload::AssignmentUpdated {
            assignment: assignment.clone(),
        });
        let state = self
            .append_migration_command_unlocked(
                context.image_id,
                user_id,
                DatasetRole::Annotator,
                idempotency_key,
                context.assignment_id,
                payloads,
                0,
                now,
            )
            .await?;
        command_result(state, context.task_id, pass_id, Some(assignment), None)
    }

    pub async fn start_migration_pass(
        &self,
        user_id: &UserId,
        context: AssignmentContext<'_>,
        expected_target_set_hash: &MigrationHash,
        expected_state_hash: &MigrationHash,
        idempotency_key: &str,
    ) -> StorageResult<ManualMigrationCommandResult> {
        validate_idempotency_key(idempotency_key)?;
        let metadata = self.load_dataset().await?;
        migration_metadata(&metadata, context.image_id, context.task_id)?;
        require_annotation_context(&metadata, user_id, &context)?;
        let lock = self.image_lock(context.image_id);
        let _guard = lock.lock().await;
        let state = self.load_image_state(context.image_id).await?;
        let primary_id = command_event_id(user_id, context.assignment_id, idempotency_key, None);
        if let Some(command) = find_command(self, context.image_id, &primary_id).await? {
            let EventPayload::MigrationPassStarted { pass } = &command.primary.payload else {
                return Err(idempotency_conflict());
            };
            let matches = pass.assignment_id == *context.assignment_id
                && pass.task_id == *context.task_id
                && pass.expected_target_set_hash == *expected_target_set_hash
                && pass.starting_state_hash == *expected_state_hash;
            return replay_retry(
                matches,
                state,
                context.task_id,
                Some(&pass.pass_id),
                Some(context.assignment_id),
                None,
            );
        }
        let now = labello_domain::now();
        let mut assignment = exact_active_assignment(
            &state.assignments,
            context.assignment_id,
            context.image_id,
            context.task_id,
            user_id,
            &AssignmentKind::Annotation,
            now,
        )?
        .clone();
        ensure_annotation_status(&state, context.task_id)?;
        if state.migration_cursor(context.task_id, None)? != MigrationCursor::FullImage {
            return Err(conflict(
                "a correction pass can start only from full-image review",
            ));
        }
        for pass in state.migration_passes.values().filter(|pass| {
            pass.task_id == *context.task_id && pass.assignment_id == *context.assignment_id
        }) {
            if state.migration_cursor(context.task_id, Some(&pass.pass_id))?
                != MigrationCursor::FullImage
            {
                return Err(conflict(
                    "another migration correction pass is still active",
                ));
            }
        }
        let set = state
            .migration_target_sets
            .get(context.task_id)
            .ok_or_else(|| {
                StorageError::InvalidAssignment("migration target set is missing".to_string())
            })?;
        let current_hash = state.current_migration_state_hash(context.task_id)?;
        if &set.target_set_hash != expected_target_set_hash || &current_hash != expected_state_hash
        {
            return Err(conflict("migration pass hashes are stale"));
        }
        let pass = MigrationPass {
            pass_id: command_pass_id(user_id, context.assignment_id, idempotency_key),
            assignment_id: context.assignment_id.clone(),
            task_id: context.task_id.clone(),
            expected_target_set_hash: expected_target_set_hash.clone(),
            starting_state_hash: expected_state_hash.clone(),
            actor_user_id: user_id.clone(),
            started_at: now,
            items: Vec::new(),
        };
        renew(&mut assignment, now);
        let payloads = vec![
            EventPayload::MigrationPassStarted { pass: pass.clone() },
            EventPayload::AssignmentUpdated {
                assignment: assignment.clone(),
            },
        ];
        let state = self
            .append_migration_command_unlocked(
                context.image_id,
                user_id,
                DatasetRole::Annotator,
                idempotency_key,
                context.assignment_id,
                payloads,
                0,
                now,
            )
            .await?;
        command_result(
            state,
            context.task_id,
            Some(&pass.pass_id),
            Some(assignment),
            None,
        )
    }

    pub async fn keep_migration_target(
        &self,
        user_id: &UserId,
        context: AssignmentContext<'_>,
        pass_id: &MigrationPassId,
        expected: &MigrationTargetExpectation,
        idempotency_key: &str,
    ) -> StorageResult<ManualMigrationCommandResult> {
        validate_idempotency_key(idempotency_key)?;
        let metadata = self.load_dataset().await?;
        let (task, guide_task, _) =
            migration_metadata(&metadata, context.image_id, context.task_id)?;
        require_annotation_context(&metadata, user_id, &context)?;
        let lock = self.image_lock(context.image_id);
        let _guard = lock.lock().await;
        let state = self.load_image_state(context.image_id).await?;
        let primary_id = command_event_id(user_id, context.assignment_id, idempotency_key, None);
        if let Some(command) = find_command(self, context.image_id, &primary_id).await? {
            let matches = matches!(&command.primary.payload, EventPayload::MigrationPassItemRecorded { pass_id: value, item } if value == pass_id && item.object_group_id == expected.object_group_id && item.action == MigrationPassItemAction::Kept)
                && expectation_matches(&command.before, context.task_id, expected);
            return replay_retry(
                matches,
                state,
                context.task_id,
                Some(pass_id),
                Some(context.assignment_id),
                None,
            );
        }
        let now = labello_domain::now();
        let mut assignment =
            validate_annotation_command(&state, &context, user_id, Some(pass_id), expected, now)?;
        let target = migration_target(&state, context.task_id, &expected.object_group_id)?;
        validate_target_identity(&state, task, guide_task, target, expected)?;
        if has_dependency(&state, context.task_id, &expected.object_group_id)
            || expected.expected_guide_deleted
            || matches!(
                current_disposition(&state, context.task_id, &expected.object_group_id)?.status,
                MigrationDispositionStatus::Pending
            )
        {
            return Err(conflict("the current migration target cannot be kept"));
        }
        let item = pass_item(
            &state,
            context.task_id,
            &expected.object_group_id,
            MigrationPassItemAction::Kept,
            primary_id,
        )?;
        renew(&mut assignment, now);
        let payloads = vec![
            EventPayload::MigrationPassItemRecorded {
                pass_id: pass_id.clone(),
                item,
            },
            EventPayload::AssignmentUpdated {
                assignment: assignment.clone(),
            },
        ];
        let state = self
            .append_migration_command_unlocked(
                context.image_id,
                user_id,
                DatasetRole::Annotator,
                idempotency_key,
                context.assignment_id,
                payloads,
                0,
                now,
            )
            .await?;
        command_result(
            state,
            context.task_id,
            Some(pass_id),
            Some(assignment),
            None,
        )
    }

    pub async fn confirm_and_submit_migration(
        &self,
        user_id: &UserId,
        context: AssignmentContext<'_>,
        target_set_hash: &MigrationHash,
        state_hash: &MigrationHash,
        confirmation_hash: &MigrationHash,
        idempotency_key: &str,
    ) -> StorageResult<ManualMigrationCommandResult> {
        validate_idempotency_key(idempotency_key)?;
        let metadata = self.load_dataset().await?;
        let (task, _, _) = migration_metadata(&metadata, context.image_id, context.task_id)?;
        require_annotation_context(&metadata, user_id, &context)?;
        let lock = self.image_lock(context.image_id);
        let _guard = lock.lock().await;
        let state = self.load_image_state(context.image_id).await?;
        let primary_id = command_event_id(user_id, context.assignment_id, idempotency_key, None);
        if let Some(command) = find_command(self, context.image_id, &primary_id).await? {
            let matches = matches!(&command.primary.payload, EventPayload::MigrationFullImageConfirmed { confirmation } if confirmation.task_id == *context.task_id && confirmation.target_set_hash == *target_set_hash && confirmation.state_hash == *state_hash && confirmation.confirmation_hash == *confirmation_hash);
            return replay_retry(
                matches,
                state,
                context.task_id,
                None,
                Some(context.assignment_id),
                None,
            );
        }
        let now = labello_domain::now();
        let mut assignment = exact_active_assignment(
            &state.assignments,
            context.assignment_id,
            context.image_id,
            context.task_id,
            user_id,
            &AssignmentKind::Annotation,
            now,
        )?
        .clone();
        ensure_annotation_status(&state, context.task_id)?;
        if state.migration_cursor(context.task_id, None)? != MigrationCursor::FullImage {
            return Err(conflict(
                "migration cannot be submitted from an object phase",
            ));
        }
        if state.migration_passes.values().any(|pass| {
            pass.task_id == *context.task_id
                && pass.assignment_id == *context.assignment_id
                && state
                    .migration_cursor(context.task_id, Some(&pass.pass_id))
                    .is_ok_and(|cursor| cursor != MigrationCursor::FullImage)
        }) {
            return Err(conflict("migration correction pass is incomplete"));
        }
        validate_exact_one(&state, task)?;
        let set = &state.migration_target_sets[context.task_id];
        let current_state_hash = state.current_migration_state_hash(context.task_id)?;
        let current_confirmation_hash =
            migration_confirmation_hash(&set.target_set_hash, &current_state_hash)?;
        if target_set_hash != &set.target_set_hash
            || state_hash != &current_state_hash
            || confirmation_hash != &current_confirmation_hash
        {
            return Err(conflict("migration confirmation digest is stale"));
        }
        let confirmation = MigrationConfirmation {
            task_id: context.task_id.clone(),
            target_set_hash: target_set_hash.clone(),
            state_hash: state_hash.clone(),
            confirmation_hash: confirmation_hash.clone(),
            actor_user_id: user_id.clone(),
            timestamp: now,
        };
        let status = if task.review.workflow == ReviewWorkflow::None {
            TaskStatus::Completed
        } else {
            TaskStatus::Submitted
        };
        assignment.status = AssignmentStatus::Completed;
        assignment.updated_at = now;
        let payloads = vec![
            EventPayload::MigrationFullImageConfirmed {
                confirmation: confirmation.clone(),
            },
            EventPayload::TaskStateChanged {
                task_state: TaskState {
                    task_id: context.task_id.clone(),
                    status: status.clone(),
                    outcome: (status == TaskStatus::Completed)
                        .then_some(TaskOutcome::AnnotationCompleted),
                    assigned_to: Some(user_id.clone()),
                    completed_by: Some(user_id.clone()),
                    completed_at: Some(now),
                    updated_at: now,
                },
            },
            EventPayload::AssignmentUpdated {
                assignment: assignment.clone(),
            },
        ];
        let state = self
            .append_migration_command_unlocked(
                context.image_id,
                user_id,
                DatasetRole::Annotator,
                idempotency_key,
                context.assignment_id,
                payloads,
                0,
                now,
            )
            .await?;
        command_result(state, context.task_id, None, Some(assignment), None)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn review_migration(
        &self,
        user_id: &UserId,
        context: AssignmentContext<'_>,
        target: &MigrationReviewTarget,
        decision: ReviewDecision,
        comment: Option<String>,
        idempotency_key: &str,
    ) -> StorageResult<ManualMigrationCommandResult> {
        validate_idempotency_key(idempotency_key)?;
        validate_comment(comment.as_deref())?;
        let metadata = self.load_dataset().await?;
        let (task, _, _) = migration_metadata(&metadata, context.image_id, context.task_id)?;
        if context.kind != AssignmentKind::Review {
            return Err(StorageError::InvalidAssignment(
                "migration review requires a review assignment".to_string(),
            ));
        }
        require_role(
            &metadata.role_assignments,
            &metadata.dataset_id,
            user_id,
            DatasetRole::Reviewer,
        )?;
        if task.review.workflow != ReviewWorkflow::Approval {
            return Err(StorageError::InvalidAssignment(
                "migration review is not enabled for this task".to_string(),
            ));
        }
        let lock = self.image_lock(context.image_id);
        let _guard = lock.lock().await;
        let state = self.load_image_state(context.image_id).await?;
        let primary_id = command_event_id(user_id, context.assignment_id, idempotency_key, None);
        if let Some(command) = find_command(self, context.image_id, &primary_id).await? {
            let matches = matches!(&command.primary.payload, EventPayload::ReviewRecorded { review } if review.decision == decision && review.comment == comment && review_request_matches(review, &command.before, context.task_id, target));
            return replay_retry(
                matches,
                state,
                context.task_id,
                None,
                Some(context.assignment_id),
                None,
            );
        }
        let now = labello_domain::now();
        let mut assignment = exact_active_assignment(
            &state.assignments,
            context.assignment_id,
            context.image_id,
            context.task_id,
            user_id,
            &AssignmentKind::Review,
            now,
        )?
        .clone();
        if state
            .task_states
            .get(context.task_id)
            .map(|value| &value.status)
            != Some(&TaskStatus::Submitted)
        {
            return Err(conflict("migration task is no longer submitted for review"));
        }
        let events = self.load_events(context.image_id).await?;
        let canonical = canonical_review_target(&state, &events, context.task_id, user_id)?;
        let review_target = match (canonical, target) {
            (
                CanonicalReviewTarget::Object {
                    object_group_id,
                    review_target,
                    disposition_version,
                },
                MigrationReviewTarget::Disposition {
                    object_group_id: requested_group,
                    disposition_version: requested_version,
                },
            ) if object_group_id == *requested_group
                && disposition_version == *requested_version =>
            {
                review_target
            }
            (
                CanonicalReviewTarget::Confirmation { confirmation_hash },
                MigrationReviewTarget::Confirmation {
                    confirmation_hash: requested_hash,
                },
            ) if confirmation_hash == *requested_hash => ReviewTarget::MigrationConfirmation {
                task_id: context.task_id.clone(),
                confirmation_hash,
            },
            _ => {
                return Err(conflict(
                    "migration review target is not the canonical next target",
                ));
            }
        };
        let review = ReviewRecord {
            review_id: command_review_id(user_id, context.assignment_id, idempotency_key),
            target: review_target.clone(),
            reviewer_user_id: user_id.clone(),
            decision: decision.clone(),
            timestamp: now,
            comment,
        };
        let mut payloads = vec![EventPayload::ReviewRecorded {
            review: review.clone(),
        }];
        let final_decision = matches!(review_target, ReviewTarget::MigrationConfirmation { .. });
        if decision == ReviewDecision::Rejected {
            if let ReviewTarget::AnnotationVersion { annotation_id, .. } = &review_target {
                let group_id = state.migration_target_sets[context.task_id]
                    .targets
                    .iter()
                    .find(|target| &target.reserved_skeleton_annotation_id == annotation_id)
                    .map(|target| target.object_group_id.clone())
                    .expect("canonical skeleton review belongs to a migration target");
                payloads.push(correction_marker_payload(
                    &state,
                    context.task_id,
                    &group_id,
                    &primary_id,
                    now,
                )?);
            } else if let ReviewTarget::MigrationDisposition {
                object_group_id, ..
            } = &review_target
            {
                payloads.push(correction_marker_payload(
                    &state,
                    context.task_id,
                    object_group_id,
                    &primary_id,
                    now,
                )?);
            }
            payloads.push(task_state_payload(
                context.task_id,
                TaskStatus::NeedsCorrection,
                None,
                now,
            ));
            finish_review_assignments(
                &state,
                context.task_id,
                context.assignment_id,
                now,
                &mut payloads,
            );
            assignment.status = AssignmentStatus::Completed;
            assignment.updated_at = now;
        } else if final_decision {
            let approvals = current_confirmation_approvals(
                &events,
                context.task_id,
                match &review_target {
                    ReviewTarget::MigrationConfirmation {
                        confirmation_hash, ..
                    } => confirmation_hash,
                    _ => unreachable!(),
                },
            );
            assignment.status = AssignmentStatus::Completed;
            assignment.updated_at = now;
            payloads.push(EventPayload::AssignmentUpdated {
                assignment: assignment.clone(),
            });
            if approvals + 1 >= task.review.required_reviews {
                payloads.push(EventPayload::TaskStateChanged {
                    task_state: TaskState {
                        task_id: context.task_id.clone(),
                        status: TaskStatus::Completed,
                        outcome: Some(TaskOutcome::Approved),
                        assigned_to: None,
                        completed_by: Some(user_id.clone()),
                        completed_at: Some(now),
                        updated_at: now,
                    },
                });
                cancel_competing_reviews(
                    &state,
                    context.task_id,
                    context.assignment_id,
                    now,
                    &mut payloads,
                );
            }
        } else {
            renew(&mut assignment, now);
            payloads.push(EventPayload::AssignmentUpdated {
                assignment: assignment.clone(),
            });
        }
        let state = self
            .append_migration_command_unlocked(
                context.image_id,
                user_id,
                DatasetRole::Reviewer,
                idempotency_key,
                context.assignment_id,
                payloads,
                0,
                now,
            )
            .await?;
        command_result(state, context.task_id, None, Some(assignment), None)
    }

    #[allow(clippy::too_many_arguments)]
    async fn append_migration_command_unlocked(
        &self,
        image_id: &ImageId,
        user_id: &UserId,
        role: DatasetRole,
        idempotency_key: &str,
        assignment_id: &AssignmentId,
        payloads: Vec<EventPayload>,
        primary_index: usize,
        timestamp: Timestamp,
    ) -> StorageResult<ImageState> {
        let mut state = self.load_image_state(image_id).await?;
        let events = payloads
            .into_iter()
            .enumerate()
            .map(|(index, mut payload)| {
                let suffix = (index != primary_index).then_some(index as u32);
                let event_id = command_event_id(user_id, assignment_id, idempotency_key, suffix);
                match &mut payload {
                    EventPayload::MigrationDispositionChanged { disposition, .. } => {
                        if let MigrationDispositionStatus::Excluded { exclusion } =
                            &mut disposition.status
                        {
                            exclusion.event_id = event_id.clone();
                        }
                    }
                    EventPayload::MigrationPassItemRecorded { item, .. } => {
                        item.event_id = event_id.clone();
                    }
                    _ => {}
                }
                let mut event = EventLogEntry::new(
                    0,
                    image_id.clone(),
                    user_id.clone(),
                    role.clone(),
                    timestamp,
                    payload,
                );
                event.event_id = event_id;
                event
            })
            .collect::<Vec<_>>();
        self.append_resequenced_events(image_id, &mut state, &events)
            .await?;
        Ok(state)
    }

    pub async fn append_admin_repair_payload(
        &self,
        user_id: &UserId,
        image_id: &ImageId,
        expected_sequence: u64,
        payload: EventPayload,
    ) -> StorageResult<EventLogEntry> {
        let metadata = self.load_dataset().await?;
        require_role(
            &metadata.role_assignments,
            &metadata.dataset_id,
            user_id,
            DatasetRole::DataAdmin,
        )?;
        let lock = self.image_lock(image_id);
        let _guard = lock.lock().await;
        let state = self.load_image_state(image_id).await?;
        if state.current_sequence != expected_sequence {
            return Err(conflict("admin repair state changed concurrently"));
        }
        validate_admin_repair_migration_mutation(&metadata, &state, &payload)?;
        self.append_payloads_unlocked(
            image_id,
            &labello_domain::Actor {
                user_id: user_id.clone(),
                role: DatasetRole::DataAdmin,
            },
            vec![payload],
        )
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| StorageError::InvalidAssignment("admin repair was not appended".into()))
    }
}

pub(crate) fn append_guide_invalidation_payloads(
    state: &ImageState,
    payloads: &mut Vec<EventPayload>,
    now: Timestamp,
) {
    let changed_ids = payloads
        .iter()
        .filter_map(|payload| match payload {
            EventPayload::AnnotationVersionCreated { annotation, .. } => {
                Some(&annotation.annotation_id)
            }
            EventPayload::AnnotationDeleted { annotation_id, .. } => Some(annotation_id),
            EventPayload::ReviewerCorrectionRecorded { annotation, .. } => {
                Some(&annotation.annotation_id)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let directly_mutated_tasks = payloads
        .iter()
        .filter_map(|payload| match payload {
            EventPayload::MigrationDispositionChanged { task_id, .. }
            | EventPayload::MigrationDispositionReopened { task_id, .. }
            | EventPayload::MigrationDependencyMarked { task_id, .. }
            | EventPayload::MigrationDependencyCleared { task_id, .. } => Some(task_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    let affected = state
        .migration_target_sets
        .iter()
        .filter(|(task_id, set)| {
            directly_mutated_tasks.contains(task_id)
                || set.targets.iter().any(|target| {
                    changed_ids.contains(&&target.guide_annotation_id)
                        || changed_ids.contains(&&target.reserved_skeleton_annotation_id)
                })
        })
        .map(|(task_id, _)| task_id.clone())
        .collect::<Vec<_>>();
    for task_id in affected {
        if state.task_states.get(&task_id).is_some_and(|task_state| {
            matches!(
                task_state.status,
                TaskStatus::Submitted | TaskStatus::Completed
            )
        }) {
            payloads.push(task_state_payload(
                &task_id,
                TaskStatus::NeedsCorrection,
                None,
                now,
            ));
            cancel_competing_reviews(state, &task_id, &AssignmentId::from(""), now, payloads);
        }
    }
}

fn validate_admin_repair_migration_mutation(
    metadata: &DatasetMetadata,
    state: &ImageState,
    payload: &EventPayload,
) -> StorageResult<()> {
    let (annotation_id, task_id) = match payload {
        EventPayload::AnnotationVersionCreated { annotation, .. } => {
            (Some(&annotation.annotation_id), Some(&annotation.task_id))
        }
        EventPayload::AnnotationDeleted { annotation_id, .. } => (
            Some(annotation_id),
            state
                .current_annotation(annotation_id)
                .map(|annotation| &annotation.task_id),
        ),
        _ => (None, None),
    };
    let manual_task = task_id.is_some_and(|task_id| {
        metadata
            .task(task_id)
            .is_some_and(|task| task.manual_box_guide_migration.is_some())
    });
    let reserved_target = annotation_id.is_some_and(|annotation_id| {
        state.migration_target_sets.values().any(|set| {
            set.targets
                .iter()
                .any(|target| target.reserved_skeleton_annotation_id == *annotation_id)
        })
    });
    if manual_task || reserved_target {
        Err(StorageError::InvalidAssignment(
            "manual migration skeletons can be changed only by migration commands".into(),
        ))
    } else {
        Ok(())
    }
}

fn migration_role(kind: &AssignmentKind) -> StorageResult<DatasetRole> {
    match kind {
        AssignmentKind::Annotation => Ok(DatasetRole::Annotator),
        AssignmentKind::Review => Ok(DatasetRole::Reviewer),
        AssignmentKind::Adjudication => Err(StorageError::InvalidAssignment(
            "manual migration does not use adjudication assignments".to_string(),
        )),
    }
}

fn require_annotation_context(
    metadata: &DatasetMetadata,
    user_id: &UserId,
    context: &AssignmentContext<'_>,
) -> StorageResult<()> {
    if context.kind != AssignmentKind::Annotation {
        return Err(StorageError::InvalidAssignment(
            "manual migration mutation requires an annotation assignment".to_string(),
        ));
    }
    require_role(
        &metadata.role_assignments,
        &metadata.dataset_id,
        user_id,
        DatasetRole::Annotator,
    )?;
    Ok(())
}

fn migration_metadata<'a>(
    metadata: &'a DatasetMetadata,
    image_id: &ImageId,
    task_id: &TaskId,
) -> StorageResult<(
    &'a TaskDefinition,
    &'a TaskDefinition,
    labello_domain::ImageDimensions,
)> {
    let task = validate_migration_task(metadata, image_id, task_id)?;
    let config = task.manual_box_guide_migration.as_ref().expect("validated");
    let guide = metadata.task(&config.guide_task_id).ok_or_else(|| {
        StorageError::InvalidAssignment("manual migration guide task is missing".to_string())
    })?;
    task.validate_manual_migration(guide)
        .map_err(StorageError::Domain)?;
    let image = metadata.images.get(image_id).ok_or_else(|| {
        StorageError::InvalidAssignment(format!("image {image_id} does not belong to the dataset"))
    })?;
    Ok((task, guide, image.dimensions()))
}

fn validate_migration_task<'a>(
    metadata: &'a DatasetMetadata,
    image_id: &ImageId,
    task_id: &TaskId,
) -> StorageResult<&'a TaskDefinition> {
    if !metadata.images.contains_key(image_id) {
        return Err(StorageError::InvalidAssignment(format!(
            "image {image_id} does not belong to the dataset"
        )));
    }
    let task = metadata.task(task_id).ok_or_else(|| {
        StorageError::InvalidAssignment(format!("task {task_id} does not belong to the dataset"))
    })?;
    if task.manual_box_guide_migration.is_none() {
        return Err(StorageError::InvalidAssignment(format!(
            "task {task_id} is not a manual box-guide migration"
        )));
    }
    Ok(task)
}

fn validate_annotation_command(
    state: &ImageState,
    context: &AssignmentContext<'_>,
    user_id: &UserId,
    pass_id: Option<&MigrationPassId>,
    expected: &MigrationTargetExpectation,
    now: Timestamp,
) -> StorageResult<Assignment> {
    let assignment = exact_active_assignment(
        &state.assignments,
        context.assignment_id,
        context.image_id,
        context.task_id,
        user_id,
        &AssignmentKind::Annotation,
        now,
    )?
    .clone();
    ensure_annotation_status(state, context.task_id)?;
    if let Some(pass_id) = pass_id {
        let pass = state
            .migration_passes
            .get(pass_id)
            .ok_or_else(|| conflict(format!("migration pass {pass_id} does not exist")))?;
        if pass.assignment_id != *context.assignment_id
            || pass.task_id != *context.task_id
            || pass.actor_user_id != *user_id
        {
            return Err(conflict(
                "migration pass does not belong to this assignment, task, and user",
            ));
        }
    }
    let cursor = state.migration_cursor(context.task_id, pass_id)?;
    if !matches!(cursor, MigrationCursor::Object { object_group_id, .. } if object_group_id == expected.object_group_id)
    {
        return Err(conflict(
            "requested migration group is not the canonical current target",
        ));
    }
    Ok(assignment)
}

fn ensure_annotation_status(state: &ImageState, task_id: &TaskId) -> StorageResult<()> {
    let status = state
        .task_states
        .get(task_id)
        .map(|value| &value.status)
        .unwrap_or(&TaskStatus::Pending);
    if matches!(status, TaskStatus::InProgress | TaskStatus::NeedsCorrection) {
        Ok(())
    } else {
        Err(conflict("migration task is not open for annotation"))
    }
}

fn validate_target_identity(
    state: &ImageState,
    task: &TaskDefinition,
    guide_task: &TaskDefinition,
    target: &labello_domain::MigrationTarget,
    expected: &MigrationTargetExpectation,
) -> StorageResult<()> {
    let guide = state
        .current_annotation(&target.guide_annotation_id)
        .ok_or_else(|| StorageError::InvalidAssignment("migration guide is missing".to_string()))?;
    let disposition = current_disposition(state, &task.task_id, &target.object_group_id)?;
    let active_skeleton = state
        .current_annotation(&target.reserved_skeleton_annotation_id)
        .filter(|annotation| !annotation.deleted);
    if guide.task_id != guide_task.task_id
        || guide.class_id != *only_class(task)?
        || guide.class_id != *only_class(guide_task)?
        || guide.annotation_type != AnnotationType::BoundingBox
        || guide.object_group_id.as_ref() != Some(&target.object_group_id)
        || guide.version != expected.expected_guide_annotation_version
        || guide.deleted != expected.expected_guide_deleted
        || disposition.disposition_version != expected.expected_disposition_version
        || active_skeleton.map(|annotation| annotation.version)
            != expected.expected_skeleton_version
    {
        return Err(conflict(
            "migration target expectation is stale or inconsistent",
        ));
    }
    validate_exact_one(state, task)
}

fn validate_exact_one(state: &ImageState, task: &TaskDefinition) -> StorageResult<()> {
    let set = state
        .migration_target_sets
        .get(&task.task_id)
        .ok_or_else(|| {
            StorageError::InvalidAssignment("migration target set is missing".to_string())
        })?;
    for annotation in state
        .active_annotations()
        .filter(|annotation| annotation.task_id == task.task_id)
    {
        let valid = set.targets.iter().any(|target| {
            annotation.annotation_id == target.reserved_skeleton_annotation_id
                && annotation.object_group_id.as_ref() == Some(&target.object_group_id)
                && annotation.class_id == *only_class(task).expect("migration class validated")
                && annotation.annotation_type == AnnotationType::Skeleton
        });
        if !valid {
            return Err(conflict(
                "manual migration contains an unexpected active skeleton",
            ));
        }
    }
    Ok(())
}

fn only_class(task: &TaskDefinition) -> StorageResult<&ClassId> {
    match task.class_ids.as_slice() {
        [class_id] => Ok(class_id),
        _ => Err(StorageError::InvalidAssignment(format!(
            "migration task {} must have exactly one class",
            task.task_id
        ))),
    }
}

fn migration_target<'a>(
    state: &'a ImageState,
    task_id: &TaskId,
    object_group_id: &ObjectGroupId,
) -> StorageResult<&'a labello_domain::MigrationTarget> {
    state
        .migration_target_sets
        .get(task_id)
        .and_then(|set| {
            set.targets
                .iter()
                .find(|target| &target.object_group_id == object_group_id)
        })
        .ok_or_else(|| {
            StorageError::InvalidAssignment(format!(
                "group {object_group_id} is not a migration target for task {task_id}"
            ))
        })
}

fn current_disposition<'a>(
    state: &'a ImageState,
    task_id: &TaskId,
    object_group_id: &ObjectGroupId,
) -> StorageResult<&'a MigrationDisposition> {
    state
        .migration_dispositions
        .get(task_id)
        .and_then(|values| values.get(object_group_id))
        .ok_or_else(|| {
            StorageError::InvalidAssignment("migration disposition is missing".to_string())
        })
}

fn next_disposition(
    state: &ImageState,
    task_id: &TaskId,
    object_group_id: &ObjectGroupId,
    status: MigrationDispositionStatus,
) -> StorageResult<MigrationDisposition> {
    Ok(MigrationDisposition {
        disposition_version: current_disposition(state, task_id, object_group_id)?
            .disposition_version
            + 1,
        status,
    })
}

fn has_dependency(state: &ImageState, task_id: &TaskId, group_id: &ObjectGroupId) -> bool {
    state
        .migration_dependencies
        .get(task_id)
        .is_some_and(|values| values.contains_key(group_id))
}

#[allow(clippy::too_many_arguments)]
fn make_dependency_clearable(
    state: &mut ImageState,
    task_id: &TaskId,
    group_id: &ObjectGroupId,
    target: &labello_domain::MigrationTarget,
    payloads: &mut Vec<EventPayload>,
    user_id: &UserId,
    now: Timestamp,
) -> StorageResult<()> {
    let marker = state.migration_dependencies[task_id][group_id].clone();
    if state
        .current_annotation(&target.guide_annotation_id)
        .is_none_or(|guide| guide.deleted)
    {
        return Err(conflict(
            "migration dependency cannot clear while its guide is deleted",
        ));
    }
    match current_disposition(state, task_id, group_id)?.status {
        MigrationDispositionStatus::Annotated { .. } => {
            let skeleton = state
                .current_annotation(&target.reserved_skeleton_annotation_id)
                .filter(|annotation| !annotation.deleted)
                .cloned()
                .ok_or_else(|| conflict("annotated migration skeleton is missing"))?;
            push_simulated(
                state,
                payloads,
                user_id,
                DatasetRole::Annotator,
                now,
                EventPayload::AnnotationDeleted {
                    annotation_id: skeleton.annotation_id,
                    version: skeleton.version,
                    reason: Some("manual migration dependency correction".to_string()),
                },
            )?;
        }
        MigrationDispositionStatus::Excluded { .. } => {
            let disposition = MigrationDisposition {
                disposition_version: current_disposition(state, task_id, group_id)?
                    .disposition_version
                    + 1,
                status: MigrationDispositionStatus::Pending,
            };
            push_simulated(
                state,
                payloads,
                user_id,
                DatasetRole::Annotator,
                now,
                EventPayload::MigrationDispositionReopened {
                    task_id: task_id.clone(),
                    object_group_id: group_id.clone(),
                    disposition,
                },
            )?;
        }
        MigrationDispositionStatus::Pending => {
            return Err(conflict(
                "pending dependency requires an audited exclusion before it can be cleared",
            ));
        }
    }
    push_simulated(
        state,
        payloads,
        user_id,
        DatasetRole::Annotator,
        now,
        EventPayload::MigrationDependencyCleared {
            task_id: task_id.clone(),
            object_group_id: group_id.clone(),
            marker_version: marker.marker_version,
        },
    )
}

fn pass_item(
    state: &ImageState,
    task_id: &TaskId,
    group_id: &ObjectGroupId,
    action: MigrationPassItemAction,
    event_id: EventId,
) -> StorageResult<MigrationPassItem> {
    let target = migration_target(state, task_id, group_id)?;
    let guide = state
        .current_annotation(&target.guide_annotation_id)
        .ok_or_else(|| StorageError::InvalidAssignment("migration guide is missing".to_string()))?;
    Ok(MigrationPassItem {
        object_group_id: group_id.clone(),
        guide_annotation_version: guide.version,
        guide_deleted: guide.deleted,
        disposition_version: current_disposition(state, task_id, group_id)?.disposition_version,
        action,
        event_id,
    })
}

fn push_simulated(
    state: &mut ImageState,
    payloads: &mut Vec<EventPayload>,
    user_id: &UserId,
    role: DatasetRole,
    now: Timestamp,
    payload: EventPayload,
) -> StorageResult<()> {
    let mut event = EventLogEntry::new(
        state.current_sequence + 1,
        state.image_id.clone(),
        user_id.clone(),
        role,
        now,
        payload.clone(),
    );
    if let EventPayload::MigrationDispositionChanged { disposition, .. } = &payload
        && let MigrationDispositionStatus::Excluded { exclusion } = &disposition.status
    {
        event.event_id = exclusion.event_id.clone();
    }
    state.apply_event(&event)?;
    payloads.push(payload);
    Ok(())
}

fn simulate_payloads(
    state: &mut ImageState,
    user_id: &UserId,
    role: DatasetRole,
    now: Timestamp,
    payloads: &[EventPayload],
) -> StorageResult<()> {
    for payload in payloads {
        let event = EventLogEntry::new(
            state.current_sequence + 1,
            state.image_id.clone(),
            user_id.clone(),
            role.clone(),
            now,
            payload.clone(),
        );
        state.apply_event(&event)?;
    }
    Ok(())
}

fn reopen_terminal_migration(
    state: &mut ImageState,
    task_id: &TaskId,
    now: Timestamp,
    payloads: &mut Vec<EventPayload>,
    user_id: &UserId,
) -> StorageResult<()> {
    if state.task_states.get(task_id).is_some_and(|task_state| {
        matches!(
            task_state.status,
            TaskStatus::Submitted | TaskStatus::Completed
        )
    }) {
        let payload = task_state_payload(task_id, TaskStatus::NeedsCorrection, None, now);
        push_simulated(
            state,
            payloads,
            user_id,
            DatasetRole::Annotator,
            now,
            payload,
        )?;
        cancel_competing_reviews(state, task_id, &AssignmentId::from(""), now, payloads);
    }
    Ok(())
}

fn correction_marker_payload(
    state: &ImageState,
    task_id: &TaskId,
    group_id: &ObjectGroupId,
    event_id: &EventId,
    now: Timestamp,
) -> StorageResult<EventPayload> {
    let existing = state
        .migration_dependencies
        .get(task_id)
        .and_then(|markers| markers.get(group_id));
    Ok(EventPayload::MigrationDependencyMarked {
        task_id: task_id.clone(),
        object_group_id: group_id.clone(),
        marker: labello_domain::MigrationDependencyMarker {
            marker_version: existing.map_or(1, |marker| marker.marker_version + 1),
            kind: MigrationDependencyKind::CorrectionRequired,
            required_disposition_version: current_disposition(state, task_id, group_id)?
                .disposition_version,
            event_id: event_id.clone(),
            timestamp: now,
        },
    })
}

fn task_state_payload(
    task_id: &TaskId,
    status: TaskStatus,
    completed_by: Option<UserId>,
    now: Timestamp,
) -> EventPayload {
    EventPayload::TaskStateChanged {
        task_state: TaskState {
            task_id: task_id.clone(),
            status,
            outcome: None,
            assigned_to: None,
            completed_by,
            completed_at: None,
            updated_at: now,
        },
    }
}

fn finish_review_assignments(
    state: &ImageState,
    task_id: &TaskId,
    current_id: &AssignmentId,
    now: Timestamp,
    payloads: &mut Vec<EventPayload>,
) {
    for assignment in state.assignments.iter().filter(|assignment| {
        assignment.task_id == *task_id
            && assignment.kind == AssignmentKind::Review
            && assignment.status == AssignmentStatus::Active
    }) {
        let mut assignment = assignment.clone();
        assignment.status = if assignment.assignment_id == *current_id {
            AssignmentStatus::Completed
        } else {
            AssignmentStatus::Cancelled
        };
        assignment.updated_at = now;
        payloads.push(EventPayload::AssignmentUpdated { assignment });
    }
}

fn cancel_competing_reviews(
    state: &ImageState,
    task_id: &TaskId,
    current_id: &AssignmentId,
    now: Timestamp,
    payloads: &mut Vec<EventPayload>,
) {
    for assignment in state.assignments.iter().filter(|assignment| {
        assignment.task_id == *task_id
            && assignment.kind == AssignmentKind::Review
            && assignment.status == AssignmentStatus::Active
            && assignment.assignment_id != *current_id
    }) {
        let mut assignment = assignment.clone();
        assignment.status = AssignmentStatus::Cancelled;
        assignment.updated_at = now;
        payloads.push(EventPayload::AssignmentUpdated { assignment });
    }
}

#[derive(Debug)]
enum CanonicalReviewTarget {
    Object {
        object_group_id: ObjectGroupId,
        disposition_version: u32,
        review_target: ReviewTarget,
    },
    Confirmation {
        confirmation_hash: MigrationHash,
    },
}

fn canonical_review_target(
    state: &ImageState,
    events: &[EventLogEntry],
    task_id: &TaskId,
    reviewer: &UserId,
) -> StorageResult<CanonicalReviewTarget> {
    let round = current_migration_reviews(events, task_id);
    let set = state.migration_target_sets.get(task_id).ok_or_else(|| {
        StorageError::InvalidAssignment("migration target set is missing".to_string())
    })?;
    let mut targets = set.targets.iter().collect::<Vec<_>>();
    targets.sort_by_key(|target| target.sequence_index);
    for target in targets {
        let disposition = current_disposition(state, task_id, &target.object_group_id)?;
        let review_target = match &disposition.status {
            MigrationDispositionStatus::Annotated {
                skeleton_annotation_id,
                skeleton_version,
            } => ReviewTarget::AnnotationVersion {
                annotation_id: skeleton_annotation_id.clone(),
                version: *skeleton_version,
            },
            MigrationDispositionStatus::Excluded { .. } => ReviewTarget::MigrationDisposition {
                task_id: task_id.clone(),
                object_group_id: target.object_group_id.clone(),
                disposition_version: disposition.disposition_version,
            },
            MigrationDispositionStatus::Pending => {
                return Err(conflict(
                    "submitted migration contains a pending disposition",
                ));
            }
        };
        let approved = round.iter().any(|review| {
            review.reviewer_user_id == *reviewer
                && review.decision == ReviewDecision::Approved
                && review.target == review_target
        });
        if !approved {
            return Ok(CanonicalReviewTarget::Object {
                object_group_id: target.object_group_id.clone(),
                disposition_version: disposition.disposition_version,
                review_target,
            });
        }
    }
    let confirmation = state
        .migration_confirmations
        .get(task_id)
        .ok_or_else(|| conflict("migration full-image confirmation is missing or invalidated"))?;
    Ok(CanonicalReviewTarget::Confirmation {
        confirmation_hash: confirmation.confirmation_hash.clone(),
    })
}

fn current_migration_reviews<'a>(
    events: &'a [EventLogEntry],
    task_id: &TaskId,
) -> Vec<&'a ReviewRecord> {
    let start = events.iter().rposition(|event| {
        matches!(&event.payload, EventPayload::TaskStateChanged { task_state } if task_state.task_id == *task_id && task_state.status == TaskStatus::Submitted)
    });
    if start.is_some_and(|start| {
        events.iter().skip(start + 1).any(|event| {
            matches!(&event.payload, EventPayload::TaskStateChanged { task_state }
                if task_state.task_id == *task_id
                    && matches!(task_state.status, TaskStatus::Pending | TaskStatus::InProgress | TaskStatus::NeedsCorrection))
        })
    }) {
        return Vec::new();
    }
    events
        .iter()
        .skip(start.map_or(0, |index| index + 1))
        .filter_map(|event| match &event.payload {
            EventPayload::ReviewRecorded { review }
                if matches!(&review.target,
                    ReviewTarget::AnnotationVersion { annotation_id, .. }
                        if events_target_annotation(events, task_id, annotation_id)
                ) || matches!(&review.target,
                    ReviewTarget::MigrationDisposition { task_id: reviewed, .. }
                    | ReviewTarget::MigrationConfirmation { task_id: reviewed, .. }
                        if reviewed == task_id
                ) =>
            {
                Some(review)
            }
            _ => None,
        })
        .collect()
}

fn events_target_annotation(
    events: &[EventLogEntry],
    task_id: &TaskId,
    annotation_id: &labello_domain::AnnotationId,
) -> bool {
    events.iter().any(|event| {
        matches!(&event.payload, EventPayload::AnnotationVersionCreated { annotation, .. } if annotation.task_id == *task_id && annotation.annotation_id == *annotation_id)
    })
}

fn current_confirmation_approvals(
    events: &[EventLogEntry],
    task_id: &TaskId,
    hash: &MigrationHash,
) -> u32 {
    current_migration_reviews(events, task_id)
        .into_iter()
        .filter(|review| {
            review.decision == ReviewDecision::Approved
                && matches!(&review.target, ReviewTarget::MigrationConfirmation { task_id: reviewed, confirmation_hash } if reviewed == task_id && confirmation_hash == hash)
        })
        .map(|review| &review.reviewer_user_id)
        .collect::<std::collections::BTreeSet<_>>()
        .len() as u32
}

pub(super) fn has_migration_final_review_by_user(
    events: &[EventLogEntry],
    task_id: &TaskId,
    user_id: &UserId,
) -> bool {
    current_migration_reviews(events, task_id)
        .into_iter()
        .any(|review| {
            review.reviewer_user_id == *user_id
                && matches!(
                    &review.target,
                    ReviewTarget::MigrationConfirmation {
                        task_id: reviewed, ..
                    } if reviewed == task_id
                )
        })
}

pub(super) fn migration_final_approval_count(events: &[EventLogEntry], task_id: &TaskId) -> u32 {
    current_migration_reviews(events, task_id)
        .into_iter()
        .filter(|review| {
            review.decision == ReviewDecision::Approved
                && matches!(
                    &review.target,
                    ReviewTarget::MigrationConfirmation {
                        task_id: reviewed, ..
                    } if reviewed == task_id
                )
        })
        .map(|review| &review.reviewer_user_id)
        .collect::<std::collections::BTreeSet<_>>()
        .len() as u32
}

fn review_request_matches(
    review: &ReviewRecord,
    state: &ImageState,
    task_id: &TaskId,
    requested: &MigrationReviewTarget,
) -> bool {
    match requested {
        MigrationReviewTarget::Disposition {
            object_group_id,
            disposition_version,
        } => match &review.target {
            ReviewTarget::MigrationDisposition {
                task_id: reviewed,
                object_group_id: group,
                disposition_version: version,
            } => reviewed == task_id && group == object_group_id && version == disposition_version,
            ReviewTarget::AnnotationVersion {
                annotation_id,
                version,
            } => state
                .migration_target_sets
                .get(task_id)
                .and_then(|set| {
                    set.targets
                        .iter()
                        .find(|target| &target.object_group_id == object_group_id)
                })
                .is_some_and(|target| {
                    &target.reserved_skeleton_annotation_id == annotation_id
                        && current_disposition(state, task_id, object_group_id).is_ok_and(
                            |disposition| {
                                disposition.disposition_version == *disposition_version
                                    && matches!(
                                        disposition.status,
                                        MigrationDispositionStatus::Annotated {
                                            skeleton_version,
                                            ..
                                        } if skeleton_version == *version
                                    )
                            },
                        )
                }),
            _ => false,
        },
        MigrationReviewTarget::Confirmation { confirmation_hash } => {
            matches!(&review.target, ReviewTarget::MigrationConfirmation { task_id: reviewed, confirmation_hash: hash } if reviewed == task_id && hash == confirmation_hash)
        }
    }
}

fn command_result(
    state: ImageState,
    task_id: &TaskId,
    pass_id: Option<&MigrationPassId>,
    assignment: Option<Assignment>,
    annotation_id: Option<labello_domain::AnnotationId>,
) -> StorageResult<ManualMigrationCommandResult> {
    let cursor = state.migration_cursor(task_id, pass_id)?;
    let dispositions = state.migration_dispositions.get(task_id).ok_or_else(|| {
        StorageError::InvalidAssignment("migration dispositions are missing".to_string())
    })?;
    let mut progress = ManualMigrationProgress {
        expected: dispositions.len() as u64,
        ..ManualMigrationProgress::default()
    };
    for disposition in dispositions.values() {
        match disposition.status {
            MigrationDispositionStatus::Pending => progress.pending += 1,
            MigrationDispositionStatus::Annotated { .. } => progress.annotated += 1,
            MigrationDispositionStatus::Excluded { .. } => progress.excluded += 1,
        }
    }
    let active_pass = pass_id.and_then(|pass_id| state.migration_passes.get(pass_id).cloned());
    let confirmation = state.migration_confirmations.get(task_id).cloned();
    Ok(ManualMigrationCommandResult {
        image_state: state,
        cursor,
        progress,
        active_pass,
        confirmation,
        assignment,
        annotation_id,
    })
}

fn replay_retry(
    matches: bool,
    state: ImageState,
    task_id: &TaskId,
    pass_id: Option<&MigrationPassId>,
    assignment_id: Option<&AssignmentId>,
    annotation_id: Option<labello_domain::AnnotationId>,
) -> StorageResult<ManualMigrationCommandResult> {
    if !matches {
        return Err(idempotency_conflict());
    }
    let assignment = assignment_id.and_then(|id| {
        state
            .assignments
            .iter()
            .find(|assignment| &assignment.assignment_id == id)
            .cloned()
    });
    command_result(state, task_id, pass_id, assignment, annotation_id)
}

struct PersistedCommand {
    primary: EventLogEntry,
    before: ImageState,
    events: Vec<EventLogEntry>,
}

async fn find_command(
    repository: &DatasetRepository,
    image_id: &ImageId,
    event_id: &EventId,
) -> StorageResult<Option<PersistedCommand>> {
    let events = repository.load_events(image_id).await?;
    let Some(primary_index) = events.iter().position(|event| &event.event_id == event_id) else {
        return Ok(None);
    };
    let secondary_prefix = format!("{}_", event_id.as_str());
    let belongs_to_command = |event: &EventLogEntry| {
        event.event_id == *event_id || event.event_id.as_str().starts_with(&secondary_prefix)
    };
    let first_index = (0..=primary_index)
        .rev()
        .take_while(|index| belongs_to_command(&events[*index]))
        .last()
        .unwrap_or(primary_index);
    let last_index = (primary_index..events.len())
        .take_while(|index| belongs_to_command(&events[*index]))
        .last()
        .unwrap_or(primary_index);
    let before = labello_domain::rebuild_state(image_id.clone(), &events[..first_index])?;
    Ok(Some(PersistedCommand {
        primary: events[primary_index].clone(),
        before,
        events: events[first_index..=last_index].to_vec(),
    }))
}

fn expectation_matches(
    state: &ImageState,
    task_id: &TaskId,
    expected: &MigrationTargetExpectation,
) -> bool {
    let Ok(target) = migration_target(state, task_id, &expected.object_group_id) else {
        return false;
    };
    let Some(guide) = state.current_annotation(&target.guide_annotation_id) else {
        return false;
    };
    let Ok(disposition) = current_disposition(state, task_id, &expected.object_group_id) else {
        return false;
    };
    let skeleton_version = state
        .current_annotation(&target.reserved_skeleton_annotation_id)
        .filter(|annotation| !annotation.deleted)
        .map(|annotation| annotation.version);
    guide.version == expected.expected_guide_annotation_version
        && guide.deleted == expected.expected_guide_deleted
        && disposition.disposition_version == expected.expected_disposition_version
        && skeleton_version == expected.expected_skeleton_version
}

fn pass_request_matches(events: &[EventLogEntry], pass_id: Option<&MigrationPassId>) -> bool {
    let persisted = events.iter().find_map(|event| match &event.payload {
        EventPayload::MigrationPassItemRecorded { pass_id, .. } => Some(pass_id),
        _ => None,
    });
    persisted == pass_id
}

fn validate_idempotency_key(key: &str) -> StorageResult<()> {
    if key.is_empty()
        || key.len() > MAX_IDEMPOTENCY_KEY_BYTES
        || key.bytes().any(|byte| byte.is_ascii_control())
    {
        Err(StorageError::InvalidAssignment(
            "idempotency key must contain 1..=200 non-control bytes".to_string(),
        ))
    } else {
        Ok(())
    }
}

fn validate_note(reason: MigrationExclusionReason, note: Option<&str>) -> StorageResult<()> {
    if note.is_some_and(|note| note.len() > MAX_EXCLUSION_NOTE_BYTES)
        || (reason == MigrationExclusionReason::Other
            && note.is_none_or(|note| note.trim().is_empty()))
    {
        Err(StorageError::InvalidAssignment(
            "migration exclusion note is missing or exceeds 2000 bytes".to_string(),
        ))
    } else {
        Ok(())
    }
}

fn validate_comment(comment: Option<&str>) -> StorageResult<()> {
    if comment.is_some_and(|comment| comment.len() > MAX_REVIEW_COMMENT_BYTES) {
        Err(StorageError::InvalidAssignment(
            "migration review comment exceeds 2000 bytes".to_string(),
        ))
    } else {
        Ok(())
    }
}

fn command_digest(user_id: &UserId, assignment_id: &AssignmentId, key: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"labello:migration-command-idempotency:v1\0");
    for value in [user_id.as_str(), assignment_id.as_str(), key] {
        hasher.update(&(value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn command_event_id(
    user_id: &UserId,
    assignment_id: &AssignmentId,
    key: &str,
    suffix: Option<u32>,
) -> EventId {
    let digest = command_digest(user_id, assignment_id, key);
    EventId::from(match suffix {
        Some(suffix) => format!("evt_idem_{digest}_{suffix}"),
        None => format!("evt_idem_{digest}"),
    })
}

fn command_pass_id(user_id: &UserId, assignment_id: &AssignmentId, key: &str) -> MigrationPassId {
    MigrationPassId::from(format!(
        "migpass_{}",
        command_digest(user_id, assignment_id, key)
    ))
}

fn command_review_id(user_id: &UserId, assignment_id: &AssignmentId, key: &str) -> ReviewId {
    ReviewId::from(format!(
        "rev_idem_{}",
        command_digest(user_id, assignment_id, key)
    ))
}

fn renew(assignment: &mut Assignment, now: Timestamp) {
    assignment.updated_at = now;
    assignment.expires_at = Some(lease_expiration(now));
}

fn conflict(message: impl Into<String>) -> StorageError {
    StorageError::AssignmentConflict(message.into())
}

fn idempotency_conflict() -> StorageError {
    conflict("idempotency key was already used for a different migration command")
}

trait EventPayloadExt {
    fn task_annotation_id(&self) -> Option<labello_domain::AnnotationId>;
}

impl EventPayloadExt for EventPayload {
    fn task_annotation_id(&self) -> Option<labello_domain::AnnotationId> {
        match self {
            EventPayload::AnnotationVersionCreated { annotation, .. } => {
                Some(annotation.annotation_id.clone())
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use labello_domain::{
        Actor, AnnotationId, BoundingBox, DatasetId, DatasetRoleAssignment, HumanRevisionKind,
        ImageRecord, ImagesIndex, ImportCoverage, ImportGeometryProvenance, ImportId,
        ImportTaskInitialization, ImportedOrigin, KeypointAnnotation, KeypointSpec, KeypointState,
        LabelClass, ManualBoxGuideMigration, MigrationCardinality, MigrationHashContext,
        MigrationSequence, MigrationTarget, MigrationTargetSetInitialization, NormalizedPoint,
        ReviewConfig, SCHEMA_VERSION, SkeletonSpec, SourceProfile, TutorialContent,
        migration_target_set_hash, rebuild_state,
    };

    use super::*;

    struct Fixture {
        _temp: tempfile::TempDir,
        repository: DatasetRepository,
        image_id: ImageId,
        guide_task_id: TaskId,
        task_id: TaskId,
        annotator: UserId,
        reviewers: [UserId; 2],
        targets: Vec<MigrationTarget>,
    }

    #[tokio::test]
    async fn migration_commands_are_canonical_idempotent_atomic_and_replayable() {
        let fixture = fixture(ReviewWorkflow::None, 2).await;
        let assignment = fixture
            .repository
            .assign_next_image(
                &fixture.annotator,
                &fixture.task_id,
                AssignmentKind::Annotation,
            )
            .await
            .unwrap()
            .unwrap();
        let initial = fixture
            .repository
            .current_manual_migration(&fixture.annotator, context(&assignment), None)
            .await
            .unwrap();
        assert_eq!(
            initial.cursor,
            MigrationCursor::Object {
                object_group_id: fixture.targets[0].object_group_id.clone(),
                sequence_index: 0,
            }
        );

        let later = expectation(&initial.image_state, &fixture.task_id, &fixture.targets[1]);
        assert!(
            fixture
                .repository
                .exclude_migration_target(
                    &fixture.annotator,
                    context(&assignment),
                    None,
                    &later,
                    MigrationExclusionReason::ObjectNotPresent,
                    None,
                    "jump",
                )
                .await
                .is_err(),
            "naming a later object must not move the server cursor"
        );

        let first = expectation(&initial.image_state, &fixture.task_id, &fixture.targets[0]);
        let geometry = skeleton(0.2);
        let saved = fixture
            .repository
            .save_migration_skeleton(
                &fixture.annotator,
                context(&assignment),
                None,
                &first,
                geometry.clone(),
                "save-first",
            )
            .await
            .unwrap();
        let sequence_after_save = saved.image_state.current_sequence;
        let retried = fixture
            .repository
            .save_migration_skeleton(
                &fixture.annotator,
                context(&assignment),
                None,
                &first,
                geometry,
                "save-first",
            )
            .await
            .unwrap();
        assert_eq!(retried.image_state.current_sequence, sequence_after_save);
        assert!(
            fixture
                .repository
                .save_migration_skeleton(
                    &fixture.annotator,
                    context(&assignment),
                    None,
                    &first,
                    skeleton(0.9),
                    "save-first",
                )
                .await
                .is_err(),
            "reusing a key for different content must conflict"
        );

        let second = expectation(&saved.image_state, &fixture.task_id, &fixture.targets[1]);
        let excluded = fixture
            .repository
            .exclude_migration_target(
                &fixture.annotator,
                context(&assignment),
                None,
                &second,
                MigrationExclusionReason::Other,
                Some("occluded by equipment".to_string()),
                "exclude-second",
            )
            .await
            .unwrap();
        assert_eq!(excluded.cursor, MigrationCursor::FullImage);
        assert_eq!(excluded.progress.annotated, 1);
        assert_eq!(excluded.progress.excluded, 1);

        let target_hash = excluded.image_state.migration_target_sets[&fixture.task_id]
            .target_set_hash
            .clone();
        let state_hash = excluded
            .image_state
            .current_migration_state_hash(&fixture.task_id)
            .unwrap();
        let pass = fixture
            .repository
            .start_migration_pass(
                &fixture.annotator,
                context(&assignment),
                &target_hash,
                &state_hash,
                "start-pass",
            )
            .await
            .unwrap();
        let pass_id = pass.active_pass.as_ref().unwrap().pass_id.clone();
        let pass_sequence = pass.image_state.current_sequence;
        let retried_pass = fixture
            .repository
            .start_migration_pass(
                &fixture.annotator,
                context(&assignment),
                &target_hash,
                &state_hash,
                "start-pass",
            )
            .await
            .unwrap();
        assert_eq!(retried_pass.active_pass.unwrap().pass_id, pass_id);
        assert_eq!(retried_pass.image_state.current_sequence, pass_sequence);

        let keep = expectation(&pass.image_state, &fixture.task_id, &fixture.targets[0]);
        let kept = fixture
            .repository
            .keep_migration_target(
                &fixture.annotator,
                context(&assignment),
                &pass_id,
                &keep,
                "keep-first",
            )
            .await
            .unwrap();
        let reopen = expectation(&kept.image_state, &fixture.task_id, &fixture.targets[1]);
        let reopened = fixture
            .repository
            .reopen_migration_target(
                &fixture.annotator,
                context(&assignment),
                Some(&pass_id),
                &reopen,
                "reopen-second",
            )
            .await
            .unwrap();
        assert!(matches!(
            reopened.cursor,
            MigrationCursor::Object { ref object_group_id, .. }
                if object_group_id == &fixture.targets[1].object_group_id
        ));
        let reexclude = expectation(&reopened.image_state, &fixture.task_id, &fixture.targets[1]);
        let completed_pass = fixture
            .repository
            .exclude_migration_target(
                &fixture.annotator,
                context(&assignment),
                Some(&pass_id),
                &reexclude,
                MigrationExclusionReason::NoValidSkeleton,
                None,
                "reexclude-second",
            )
            .await
            .unwrap();
        assert_eq!(completed_pass.cursor, MigrationCursor::FullImage);

        let target_hash = completed_pass.image_state.migration_target_sets[&fixture.task_id]
            .target_set_hash
            .clone();
        let state_hash = completed_pass
            .image_state
            .current_migration_state_hash(&fixture.task_id)
            .unwrap();
        let confirmation_hash = migration_confirmation_hash(&target_hash, &state_hash).unwrap();
        let submitted = fixture
            .repository
            .confirm_and_submit_migration(
                &fixture.annotator,
                context(&assignment),
                &target_hash,
                &state_hash,
                &confirmation_hash,
                "confirm",
            )
            .await
            .unwrap();
        let mut final_sequence = submitted.image_state.current_sequence;
        let retry = fixture
            .repository
            .confirm_and_submit_migration(
                &fixture.annotator,
                context(&assignment),
                &target_hash,
                &state_hash,
                &confirmation_hash,
                "confirm",
            )
            .await
            .unwrap();
        assert_eq!(retry.image_state.current_sequence, final_sequence);
        assert_eq!(
            retry.image_state.task_states[&fixture.task_id].status,
            TaskStatus::Completed
        );
        let reopened_assignment = fixture
            .repository
            .reopen_annotation_assignment(
                &fixture.annotator,
                &assignment.assignment_id,
                &fixture.image_id,
                &fixture.task_id,
                AssignmentKind::Annotation,
            )
            .await
            .unwrap();
        let reopened_state = fixture
            .repository
            .load_image_state(&fixture.image_id)
            .await
            .unwrap();
        assert_eq!(reopened_assignment.status, AssignmentStatus::Active);
        assert_eq!(
            reopened_state.task_states[&fixture.task_id].status,
            TaskStatus::NeedsCorrection
        );
        assert!(
            !reopened_state
                .migration_confirmations
                .contains_key(&fixture.task_id)
        );
        final_sequence = reopened_state.current_sequence;

        tokio::fs::remove_file(fixture.repository.state_path(&fixture.image_id))
            .await
            .unwrap();
        let reloaded = fixture
            .repository
            .load_image_state(&fixture.image_id)
            .await
            .unwrap();
        let events = fixture
            .repository
            .load_events(&fixture.image_id)
            .await
            .unwrap();
        assert_eq!(
            reloaded,
            rebuild_state(fixture.image_id.clone(), &events).unwrap()
        );
        assert_eq!(reloaded.current_sequence, final_sequence);
        assert!(
            events
                .iter()
                .all(|event| event.actor_user_id != UserId::from(""))
        );
    }

    #[tokio::test]
    async fn concurrent_exact_version_writes_have_one_winner() {
        let fixture = fixture(ReviewWorkflow::None, 1).await;
        let assignment = fixture
            .repository
            .assign_next_image(
                &fixture.annotator,
                &fixture.task_id,
                AssignmentKind::Annotation,
            )
            .await
            .unwrap()
            .unwrap();
        let state = fixture
            .repository
            .load_image_state(&fixture.image_id)
            .await
            .unwrap();
        let expected = expectation(&state, &fixture.task_id, &fixture.targets[0]);
        let left_repository = fixture.repository.clone();
        let right_repository = fixture.repository.clone();
        let left_assignment = assignment.clone();
        let right_assignment = assignment.clone();
        let left_expected = expected.clone();
        let right_expected = expected;
        let annotator = fixture.annotator.clone();
        let (left, right) = tokio::join!(
            async {
                left_repository
                    .save_migration_skeleton(
                        &annotator,
                        context(&left_assignment),
                        None,
                        &left_expected,
                        skeleton(0.3),
                        "concurrent-left",
                    )
                    .await
            },
            async {
                right_repository
                    .save_migration_skeleton(
                        &fixture.annotator,
                        context(&right_assignment),
                        None,
                        &right_expected,
                        skeleton(0.7),
                        "concurrent-right",
                    )
                    .await
            }
        );
        assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
        let state = fixture
            .repository
            .load_image_state(&fixture.image_id)
            .await
            .unwrap();
        assert_eq!(
            state
                .active_annotations()
                .filter(|annotation| annotation.task_id == fixture.task_id)
                .count(),
            1
        );
        assert_eq!(
            state.migration_dispositions[&fixture.task_id][&fixture.targets[0].object_group_id]
                .disposition_version,
            2
        );
    }

    #[tokio::test]
    async fn exclusion_deletes_skeleton_atomically_and_survives_reload() {
        let fixture = fixture(ReviewWorkflow::None, 1).await;
        let assignment = claim_annotator(&fixture).await;
        let initial = fixture
            .repository
            .load_image_state(&fixture.image_id)
            .await
            .unwrap();
        let expected = expectation(&initial, &fixture.task_id, &fixture.targets[0]);
        let resolved = fixture
            .repository
            .save_migration_skeleton(
                &fixture.annotator,
                context(&assignment),
                None,
                &expected,
                skeleton(0.4),
                "atomic-save",
            )
            .await
            .unwrap();
        let target_hash = resolved.image_state.migration_target_sets[&fixture.task_id]
            .target_set_hash
            .clone();
        let state_hash = resolved
            .image_state
            .current_migration_state_hash(&fixture.task_id)
            .unwrap();
        let pass = fixture
            .repository
            .start_migration_pass(
                &fixture.annotator,
                context(&assignment),
                &target_hash,
                &state_hash,
                "atomic-pass",
            )
            .await
            .unwrap();
        let pass_id = pass.active_pass.unwrap().pass_id;
        let expected = expectation(&pass.image_state, &fixture.task_id, &fixture.targets[0]);
        let excluded = fixture
            .repository
            .exclude_migration_target(
                &fixture.annotator,
                context(&assignment),
                Some(&pass_id),
                &expected,
                MigrationExclusionReason::InvalidSourceBox,
                None,
                "atomic-exclude",
            )
            .await
            .unwrap();
        let skeleton = excluded
            .image_state
            .current_annotation(&fixture.targets[0].reserved_skeleton_annotation_id)
            .unwrap();
        assert!(skeleton.deleted);
        let events = fixture
            .repository
            .load_events(&fixture.image_id)
            .await
            .unwrap();
        assert!(events.windows(2).any(|pair| {
            matches!(pair[0].payload, EventPayload::AnnotationDeleted { .. })
                && matches!(
                    pair[1].payload,
                    EventPayload::MigrationDispositionChanged { .. }
                )
        }));
        let rebuilt = rebuild_state(fixture.image_id.clone(), &events).unwrap();
        assert_eq!(rebuilt, excluded.image_state);
    }

    #[tokio::test]
    async fn deleted_guide_resolves_only_through_atomic_exclusion() {
        let fixture = fixture(ReviewWorkflow::None, 1).await;
        let assignment = claim_annotator(&fixture).await;
        let initial = fixture
            .repository
            .load_image_state(&fixture.image_id)
            .await
            .unwrap();
        let resolved = fixture
            .repository
            .save_migration_skeleton(
                &fixture.annotator,
                context(&assignment),
                None,
                &expectation(&initial, &fixture.task_id, &fixture.targets[0]),
                skeleton(0.4),
                "deleted-guide-save",
            )
            .await
            .unwrap();
        let guide = resolved
            .image_state
            .current_annotation(&fixture.targets[0].guide_annotation_id)
            .unwrap();
        fixture
            .repository
            .append_admin_repair_payload(
                &UserId::from("admin"),
                &fixture.image_id,
                resolved.image_state.current_sequence,
                EventPayload::AnnotationDeleted {
                    annotation_id: guide.annotation_id.clone(),
                    version: guide.version,
                    reason: Some("invalid imported guide".to_string()),
                },
            )
            .await
            .unwrap();
        let deleted = fixture
            .repository
            .load_image_state(&fixture.image_id)
            .await
            .unwrap();
        let expected = expectation(&deleted, &fixture.task_id, &fixture.targets[0]);
        assert!(expected.expected_guide_deleted);
        assert!(
            fixture
                .repository
                .keep_migration_target(
                    &fixture.annotator,
                    context(&assignment),
                    &MigrationPassId::from("missing"),
                    &expected,
                    "deleted-guide-keep",
                )
                .await
                .is_err()
        );
        assert!(
            fixture
                .repository
                .save_migration_skeleton(
                    &fixture.annotator,
                    context(&assignment),
                    None,
                    &expected,
                    skeleton(0.5),
                    "deleted-guide-annotate",
                )
                .await
                .is_err()
        );
        let excluded = fixture
            .repository
            .exclude_migration_target(
                &fixture.annotator,
                context(&assignment),
                None,
                &expected,
                MigrationExclusionReason::InvalidSourceBox,
                Some("guide was deleted by data repair".to_string()),
                "deleted-guide-exclude",
            )
            .await
            .unwrap();
        assert_eq!(excluded.cursor, MigrationCursor::FullImage);
        assert!(
            !excluded.image_state.migration_dependencies[&fixture.task_id]
                .contains_key(&fixture.targets[0].object_group_id)
        );
        assert!(
            excluded
                .image_state
                .current_annotation(&fixture.targets[0].reserved_skeleton_annotation_id)
                .unwrap()
                .deleted
        );
        let events = fixture
            .repository
            .load_events(&fixture.image_id)
            .await
            .unwrap();
        assert_eq!(
            rebuild_state(fixture.image_id.clone(), &events).unwrap(),
            excluded.image_state
        );
    }

    #[tokio::test]
    async fn correction_pass_action_can_be_redone_for_new_exact_versions() {
        let fixture = fixture(ReviewWorkflow::None, 1).await;
        let assignment = claim_annotator(&fixture).await;
        let initial = fixture
            .repository
            .load_image_state(&fixture.image_id)
            .await
            .unwrap();
        let resolved = fixture
            .repository
            .save_migration_skeleton(
                &fixture.annotator,
                context(&assignment),
                None,
                &expectation(&initial, &fixture.task_id, &fixture.targets[0]),
                skeleton(0.4),
                "redo-save",
            )
            .await
            .unwrap();
        let target_hash = resolved.image_state.migration_target_sets[&fixture.task_id]
            .target_set_hash
            .clone();
        let state_hash = resolved
            .image_state
            .current_migration_state_hash(&fixture.task_id)
            .unwrap();
        let pass = fixture
            .repository
            .start_migration_pass(
                &fixture.annotator,
                context(&assignment),
                &target_hash,
                &state_hash,
                "redo-pass",
            )
            .await
            .unwrap();
        let pass_id = pass.active_pass.unwrap().pass_id;
        let kept = fixture
            .repository
            .keep_migration_target(
                &fixture.annotator,
                context(&assignment),
                &pass_id,
                &expectation(&pass.image_state, &fixture.task_id, &fixture.targets[0]),
                "redo-keep",
            )
            .await
            .unwrap();
        assert_eq!(kept.cursor, MigrationCursor::FullImage);

        let guide_assignment = open_guide_assignment(&fixture).await;
        let state = fixture
            .repository
            .load_image_state(&fixture.image_id)
            .await
            .unwrap();
        let guide = state
            .current_annotation(&fixture.targets[0].guide_annotation_id)
            .unwrap();
        let mut corrected_guide = guide.clone();
        corrected_guide.version += 1;
        corrected_guide.geometry = AnnotationGeometry::BoundingBox(BoundingBox {
            x: 0.2,
            y: 0.1,
            width: 0.2,
            height: 0.2,
        });
        corrected_guide.revision_source = RevisionSource::Human {
            action: HumanRevisionKind::Edited,
        };
        corrected_guide.author_user_id = fixture.annotator.clone();
        corrected_guide.updated_at = labello_domain::now();
        let changed = fixture
            .repository
            .apply_annotation_batch(
                &fixture.annotator,
                context(&guide_assignment),
                vec![EventPayload::AnnotationVersionCreated {
                    annotation: corrected_guide,
                    previous_version: Some(guide.version),
                    reason: Some("guide correction after keep".to_string()),
                }],
                false,
            )
            .await
            .unwrap();
        assert!(matches!(
            changed
                .migration_cursor(&fixture.task_id, Some(&pass_id))
                .unwrap(),
            MigrationCursor::Object { .. }
        ));
        let redone = fixture
            .repository
            .save_migration_skeleton(
                &fixture.annotator,
                context(&assignment),
                Some(&pass_id),
                &expectation(&changed, &fixture.task_id, &fixture.targets[0]),
                skeleton(0.6),
                "redo-corrected",
            )
            .await
            .unwrap();
        assert_eq!(redone.cursor, MigrationCursor::FullImage);
        let items = &redone.active_pass.unwrap().items;
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].object_group_id, items[1].object_group_id);
        assert_ne!(
            (
                items[0].guide_annotation_version,
                items[0].disposition_version
            ),
            (
                items[1].guide_annotation_version,
                items[1].disposition_version
            )
        );
    }

    #[tokio::test]
    async fn concurrent_admin_repairs_use_one_exact_state_winner() {
        let fixture = fixture(ReviewWorkflow::None, 1).await;
        let state = fixture
            .repository
            .load_image_state(&fixture.image_id)
            .await
            .unwrap();
        let guide = state
            .current_annotation(&fixture.targets[0].guide_annotation_id)
            .unwrap();
        let repair = |x| {
            let mut annotation = guide.clone();
            annotation.version += 1;
            annotation.geometry = AnnotationGeometry::BoundingBox(BoundingBox {
                x,
                y: 0.1,
                width: 0.2,
                height: 0.2,
            });
            annotation.revision_source = RevisionSource::Human {
                action: HumanRevisionKind::Edited,
            };
            annotation.author_user_id = UserId::from("admin");
            annotation.updated_at = labello_domain::now();
            EventPayload::AnnotationVersionCreated {
                annotation,
                previous_version: Some(guide.version),
                reason: Some("concurrent admin repair".to_string()),
            }
        };
        let left = fixture.repository.clone();
        let right = fixture.repository.clone();
        let image_id = fixture.image_id.clone();
        let admin = UserId::from("admin");
        let expected_sequence = state.current_sequence;
        let (left, right) = tokio::join!(
            left.append_admin_repair_payload(&admin, &image_id, expected_sequence, repair(0.2),),
            right.append_admin_repair_payload(
                &admin,
                &fixture.image_id,
                expected_sequence,
                repair(0.3),
            )
        );
        assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
        let repaired = fixture
            .repository
            .rebuild_image_state(&fixture.image_id)
            .await
            .unwrap();
        assert_eq!(
            repaired
                .current_annotation(&fixture.targets[0].guide_annotation_id)
                .unwrap()
                .version,
            2
        );
        assert_eq!(
            repaired.migration_dependencies[&fixture.task_id][&fixture.targets[0].object_group_id]
                .kind,
            MigrationDependencyKind::CorrectionRequired
        );
    }

    #[tokio::test]
    async fn migration_review_is_sequential_and_rejection_cancels_competitors() {
        let fixture = fixture(ReviewWorkflow::Approval, 2).await;
        let annotation = claim_annotator(&fixture).await;
        let mut state = fixture
            .repository
            .load_image_state(&fixture.image_id)
            .await
            .unwrap();
        for (index, target) in fixture.targets.iter().enumerate() {
            let expected = expectation(&state, &fixture.task_id, target);
            state = if index == 0 {
                fixture
                    .repository
                    .save_migration_skeleton(
                        &fixture.annotator,
                        context(&annotation),
                        None,
                        &expected,
                        skeleton(0.4),
                        "review-save",
                    )
                    .await
                    .unwrap()
                    .image_state
            } else {
                fixture
                    .repository
                    .exclude_migration_target(
                        &fixture.annotator,
                        context(&annotation),
                        None,
                        &expected,
                        MigrationExclusionReason::ObjectNotPresent,
                        None,
                        "review-exclude",
                    )
                    .await
                    .unwrap()
                    .image_state
            };
        }
        let target_hash = state.migration_target_sets[&fixture.task_id]
            .target_set_hash
            .clone();
        let state_hash = state
            .current_migration_state_hash(&fixture.task_id)
            .unwrap();
        let confirmation_hash = migration_confirmation_hash(&target_hash, &state_hash).unwrap();
        fixture
            .repository
            .confirm_and_submit_migration(
                &fixture.annotator,
                context(&annotation),
                &target_hash,
                &state_hash,
                &confirmation_hash,
                "review-submit",
            )
            .await
            .unwrap();
        let first_review = fixture
            .repository
            .assign_next_image(
                &fixture.reviewers[0],
                &fixture.task_id,
                AssignmentKind::Review,
            )
            .await
            .unwrap()
            .unwrap();
        let second_review = fixture
            .repository
            .assign_next_image(
                &fixture.reviewers[1],
                &fixture.task_id,
                AssignmentKind::Review,
            )
            .await
            .unwrap()
            .unwrap();
        let state = fixture
            .repository
            .load_image_state(&fixture.image_id)
            .await
            .unwrap();
        let second_version = state.migration_dispositions[&fixture.task_id]
            [&fixture.targets[1].object_group_id]
            .disposition_version;
        assert!(
            fixture
                .repository
                .review_migration(
                    &fixture.reviewers[0],
                    context(&first_review),
                    &MigrationReviewTarget::Disposition {
                        object_group_id: fixture.targets[1].object_group_id.clone(),
                        disposition_version: second_version,
                    },
                    ReviewDecision::Approved,
                    None,
                    "review-jump",
                )
                .await
                .is_err()
        );
        let first_version = state.migration_dispositions[&fixture.task_id]
            [&fixture.targets[0].object_group_id]
            .disposition_version;
        fixture
            .repository
            .review_migration(
                &fixture.reviewers[0],
                context(&first_review),
                &MigrationReviewTarget::Disposition {
                    object_group_id: fixture.targets[0].object_group_id.clone(),
                    disposition_version: first_version,
                },
                ReviewDecision::Approved,
                None,
                "approve-first",
            )
            .await
            .unwrap();
        let rejected = fixture
            .repository
            .review_migration(
                &fixture.reviewers[0],
                context(&first_review),
                &MigrationReviewTarget::Disposition {
                    object_group_id: fixture.targets[1].object_group_id.clone(),
                    disposition_version: second_version,
                },
                ReviewDecision::Rejected,
                Some("object is not absent".to_string()),
                "reject-second",
            )
            .await
            .unwrap();
        let retried = fixture
            .repository
            .review_migration(
                &fixture.reviewers[0],
                context(&first_review),
                &MigrationReviewTarget::Disposition {
                    object_group_id: fixture.targets[1].object_group_id.clone(),
                    disposition_version: second_version,
                },
                ReviewDecision::Rejected,
                Some("object is not absent".to_string()),
                "reject-second",
            )
            .await
            .unwrap();
        assert_eq!(
            retried.image_state.current_sequence,
            rejected.image_state.current_sequence
        );
        assert_eq!(
            rejected.image_state.task_states[&fixture.task_id].status,
            TaskStatus::NeedsCorrection
        );
        assert_eq!(
            assignment_status(&rejected.image_state, &first_review),
            AssignmentStatus::Completed
        );
        assert_eq!(
            assignment_status(&rejected.image_state, &second_review),
            AssignmentStatus::Cancelled
        );
        assert_eq!(
            rejected.image_state.migration_dependencies[&fixture.task_id]
                [&fixture.targets[1].object_group_id]
                .kind,
            MigrationDependencyKind::CorrectionRequired
        );
    }

    #[tokio::test]
    async fn reopening_submitted_migration_cancels_active_reviews() {
        let fixture = fixture(ReviewWorkflow::Approval, 1).await;
        let annotation = claim_annotator(&fixture).await;
        let state = fixture
            .repository
            .load_image_state(&fixture.image_id)
            .await
            .unwrap();
        let expected = expectation(&state, &fixture.task_id, &fixture.targets[0]);
        let resolved = fixture
            .repository
            .exclude_migration_target(
                &fixture.annotator,
                context(&annotation),
                None,
                &expected,
                MigrationExclusionReason::ObjectNotPresent,
                None,
                "reopen-exclude",
            )
            .await
            .unwrap();
        let target_hash = resolved.image_state.migration_target_sets[&fixture.task_id]
            .target_set_hash
            .clone();
        let state_hash = resolved
            .image_state
            .current_migration_state_hash(&fixture.task_id)
            .unwrap();
        let confirmation_hash = migration_confirmation_hash(&target_hash, &state_hash).unwrap();
        fixture
            .repository
            .confirm_and_submit_migration(
                &fixture.annotator,
                context(&annotation),
                &target_hash,
                &state_hash,
                &confirmation_hash,
                "reopen-submit",
            )
            .await
            .unwrap();
        let review = fixture
            .repository
            .assign_next_image(
                &fixture.reviewers[0],
                &fixture.task_id,
                AssignmentKind::Review,
            )
            .await
            .unwrap()
            .unwrap();

        let reopened = fixture
            .repository
            .reopen_annotation_assignment(
                &fixture.annotator,
                &annotation.assignment_id,
                &fixture.image_id,
                &fixture.task_id,
                AssignmentKind::Annotation,
            )
            .await
            .unwrap();
        let state = fixture
            .repository
            .load_image_state(&fixture.image_id)
            .await
            .unwrap();
        assert_eq!(reopened.status, AssignmentStatus::Active);
        assert_eq!(
            state.task_states[&fixture.task_id].status,
            TaskStatus::NeedsCorrection
        );
        assert_eq!(
            assignment_status(&state, &review),
            AssignmentStatus::Cancelled
        );
    }

    #[tokio::test]
    async fn final_review_is_digest_bound_and_guide_change_reopens_terminal_migration() {
        let fixture = fixture(ReviewWorkflow::Approval, 1).await;
        let annotation = claim_annotator(&fixture).await;
        let state = fixture
            .repository
            .load_image_state(&fixture.image_id)
            .await
            .unwrap();
        let expected = expectation(&state, &fixture.task_id, &fixture.targets[0]);
        let resolved = fixture
            .repository
            .save_migration_skeleton(
                &fixture.annotator,
                context(&annotation),
                None,
                &expected,
                skeleton(0.5),
                "digest-save",
            )
            .await
            .unwrap();
        let target_hash = resolved.image_state.migration_target_sets[&fixture.task_id]
            .target_set_hash
            .clone();
        let state_hash = resolved
            .image_state
            .current_migration_state_hash(&fixture.task_id)
            .unwrap();
        let confirmation_hash = migration_confirmation_hash(&target_hash, &state_hash).unwrap();
        fixture
            .repository
            .confirm_and_submit_migration(
                &fixture.annotator,
                context(&annotation),
                &target_hash,
                &state_hash,
                &confirmation_hash,
                "digest-submit",
            )
            .await
            .unwrap();
        let review = fixture
            .repository
            .assign_next_image(
                &fixture.reviewers[0],
                &fixture.task_id,
                AssignmentKind::Review,
            )
            .await
            .unwrap()
            .unwrap();
        let disposition_version = fixture
            .repository
            .load_image_state(&fixture.image_id)
            .await
            .unwrap()
            .migration_dispositions[&fixture.task_id][&fixture.targets[0].object_group_id]
            .disposition_version;
        fixture
            .repository
            .review_migration(
                &fixture.reviewers[0],
                context(&review),
                &MigrationReviewTarget::Disposition {
                    object_group_id: fixture.targets[0].object_group_id.clone(),
                    disposition_version,
                },
                ReviewDecision::Approved,
                None,
                "digest-object",
            )
            .await
            .unwrap();
        assert!(
            fixture
                .repository
                .review_migration(
                    &fixture.reviewers[0],
                    context(&review),
                    &MigrationReviewTarget::Confirmation {
                        confirmation_hash: state_hash.clone(),
                    },
                    ReviewDecision::Approved,
                    None,
                    "wrong-digest",
                )
                .await
                .is_err()
        );
        let approved = fixture
            .repository
            .review_migration(
                &fixture.reviewers[0],
                context(&review),
                &MigrationReviewTarget::Confirmation {
                    confirmation_hash: confirmation_hash.clone(),
                },
                ReviewDecision::Approved,
                None,
                "right-digest",
            )
            .await
            .unwrap();
        assert_eq!(
            approved.image_state.task_states[&fixture.task_id].status,
            TaskStatus::Completed
        );

        let guide_assignment = open_guide_assignment(&fixture).await;
        let state = fixture
            .repository
            .load_image_state(&fixture.image_id)
            .await
            .unwrap();
        let current = state
            .current_annotation(&fixture.targets[0].guide_annotation_id)
            .unwrap();
        let mut corrected = current.clone();
        corrected.version += 1;
        corrected.revision_source = RevisionSource::Human {
            action: HumanRevisionKind::Edited,
        };
        corrected.author_user_id = fixture.annotator.clone();
        corrected.updated_at = labello_domain::now();
        corrected.geometry = AnnotationGeometry::BoundingBox(BoundingBox {
            x: 0.15,
            y: 0.15,
            width: 0.2,
            height: 0.2,
        });
        fixture
            .repository
            .apply_annotation_batch(
                &fixture.annotator,
                context(&guide_assignment),
                vec![EventPayload::AnnotationVersionCreated {
                    annotation: corrected,
                    previous_version: Some(current.version),
                    reason: Some("source correction".to_string()),
                }],
                false,
            )
            .await
            .unwrap();
        let reopened = fixture
            .repository
            .load_image_state(&fixture.image_id)
            .await
            .unwrap();
        assert_eq!(
            reopened.task_states[&fixture.task_id].status,
            TaskStatus::NeedsCorrection
        );
        assert!(
            !reopened
                .migration_confirmations
                .contains_key(&fixture.task_id)
        );
        assert_eq!(
            reopened.migration_dependencies[&fixture.task_id][&fixture.targets[0].object_group_id]
                .kind,
            MigrationDependencyKind::CorrectionRequired
        );
    }

    #[tokio::test]
    async fn admin_guide_repair_invalidates_submitted_migration_atomically() {
        let fixture = fixture(ReviewWorkflow::Approval, 1).await;
        let review = prepare_submitted_migration(&fixture).await;
        let state = fixture
            .repository
            .load_image_state(&fixture.image_id)
            .await
            .unwrap();
        let current = state
            .current_annotation(&fixture.targets[0].guide_annotation_id)
            .unwrap();
        let mut corrected = current.clone();
        corrected.version += 1;
        corrected.revision_source = RevisionSource::Human {
            action: HumanRevisionKind::Edited,
        };
        corrected.author_user_id = UserId::from("admin");
        corrected.updated_at = labello_domain::now();
        corrected.geometry = AnnotationGeometry::BoundingBox(BoundingBox {
            x: 0.2,
            y: 0.1,
            width: 0.2,
            height: 0.2,
        });
        fixture
            .repository
            .append_payload(
                &fixture.image_id,
                &Actor {
                    user_id: UserId::from("admin"),
                    role: DatasetRole::DataAdmin,
                },
                EventPayload::AnnotationVersionCreated {
                    annotation: corrected,
                    previous_version: Some(current.version),
                    reason: Some("admin guide repair".to_string()),
                },
            )
            .await
            .unwrap();
        assert_guide_invalidation(&fixture, &review).await;
    }

    #[tokio::test]
    async fn annotation_batch_guide_change_invalidates_submitted_migration_atomically() {
        let fixture = fixture(ReviewWorkflow::Approval, 1).await;
        let review = prepare_submitted_migration(&fixture).await;
        let guide_assignment = open_guide_assignment(&fixture).await;
        let state = fixture
            .repository
            .load_image_state(&fixture.image_id)
            .await
            .unwrap();
        let current = state
            .current_annotation(&fixture.targets[0].guide_annotation_id)
            .unwrap();
        let mut corrected = current.clone();
        corrected.version += 1;
        corrected.revision_source = RevisionSource::Human {
            action: HumanRevisionKind::Edited,
        };
        corrected.author_user_id = fixture.annotator.clone();
        corrected.updated_at = labello_domain::now();
        corrected.geometry = AnnotationGeometry::BoundingBox(BoundingBox {
            x: 0.22,
            y: 0.1,
            width: 0.2,
            height: 0.2,
        });
        fixture
            .repository
            .append_for_assignment(
                &fixture.annotator,
                context(&guide_assignment),
                vec![EventPayload::AnnotationVersionCreated {
                    annotation: corrected,
                    previous_version: Some(current.version),
                    reason: Some("batch guide correction".to_string()),
                }],
                false,
            )
            .await
            .unwrap();
        assert_guide_invalidation(&fixture, &review).await;
    }

    #[tokio::test]
    async fn offline_guide_mutation_invalidates_submitted_migration_atomically() {
        let fixture = fixture(ReviewWorkflow::Approval, 1).await;
        let review = prepare_submitted_migration(&fixture).await;
        let state = fixture
            .repository
            .load_image_state(&fixture.image_id)
            .await
            .unwrap();
        let guide = state
            .current_annotation(&fixture.targets[0].guide_annotation_id)
            .unwrap();
        let result = fixture
            .repository
            .sync_offline_events(labello_domain::OfflineSyncRequest::new(
                DatasetId::from("ds"),
                fixture.annotator.clone(),
                vec![labello_domain::OfflineMutationFragment {
                    image_id: fixture.image_id.clone(),
                    base_sequence: state.current_sequence,
                    mutations: vec![labello_domain::OfflineMutation::AnnotationUpsert {
                        annotation_id: guide.annotation_id.clone(),
                        expected_version: Some(guide.version),
                        task_id: guide.task_id.clone(),
                        class_id: guide.class_id.clone(),
                        annotation_type: guide.annotation_type.clone(),
                        source: labello_domain::OfflineAnnotationSource::Human,
                        geometry: AnnotationGeometry::BoundingBox(BoundingBox {
                            x: 0.25,
                            y: 0.1,
                            width: 0.2,
                            height: 0.2,
                        }),
                        reason: Some("offline guide correction".to_string()),
                    }],
                }],
            ))
            .await
            .unwrap();
        assert_eq!(result.merged_events, 1);
        assert_guide_invalidation(&fixture, &review).await;
    }

    #[tokio::test]
    async fn reviewer_guide_correction_invalidates_submitted_migration_atomically() {
        let fixture = fixture(ReviewWorkflow::Approval, 1).await;
        let mut metadata = fixture.repository.load_dataset_config().await.unwrap();
        metadata
            .tasks
            .iter_mut()
            .find(|task| task.task_id == fixture.guide_task_id)
            .unwrap()
            .review = ReviewConfig {
            required_reviews: 1,
            workflow: ReviewWorkflow::Approval,
            allow_reviewer_corrections: true,
            agreement_threshold: None,
        };
        fixture.repository.save_dataset(&metadata).await.unwrap();
        let migration_review = prepare_submitted_migration(&fixture).await;

        let guide_assignment = open_guide_assignment(&fixture).await;
        fixture
            .repository
            .complete_assignment(
                &fixture.annotator,
                &guide_assignment.assignment_id,
                &fixture.image_id,
                &fixture.guide_task_id,
                AssignmentKind::Annotation,
            )
            .await
            .unwrap();
        let guide_review = fixture
            .repository
            .assign_next_image(
                &fixture.reviewers[1],
                &fixture.guide_task_id,
                AssignmentKind::Review,
            )
            .await
            .unwrap()
            .unwrap();
        fixture
            .repository
            .correct_review_annotation(
                &fixture.reviewers[1],
                context(&guide_review),
                &labello_domain::CorrectionId::from("cor_guide"),
                &fixture.targets[0].guide_annotation_id,
                1,
                AnnotationGeometry::BoundingBox(BoundingBox {
                    x: 0.3,
                    y: 0.1,
                    width: 0.2,
                    height: 0.2,
                }),
                Some("reviewer guide correction".to_string()),
            )
            .await
            .unwrap();
        assert_guide_invalidation(&fixture, &migration_review).await;
    }

    async fn fixture(workflow: ReviewWorkflow, target_count: usize) -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let repository = DatasetRepository::new(temp.path());
        let image_id = ImageId::from("img_1");
        let guide_task_id = TaskId::from("bounding_box:person");
        let task_id = TaskId::from("skeleton:person");
        let class_id = ClassId::from("person");
        let annotator = UserId::from("annotator");
        let reviewers = [UserId::from("reviewer_1"), UserId::from("reviewer_2")];
        let mut metadata =
            DatasetMetadata::new(DatasetId::from("ds"), "Dataset", labello_domain::now());
        metadata.label_classes.push(LabelClass {
            class_id: class_id.clone(),
            name: "Person".to_string(),
            color: "#ffffff".to_string(),
            description: None,
        });
        metadata.tasks.push(TaskDefinition {
            task_id: guide_task_id.clone(),
            name: "Person boxes".to_string(),
            annotation_type: AnnotationType::BoundingBox,
            class_ids: vec![class_id.clone()],
            instructions: tutorial(),
            skeleton: None,
            review: ReviewConfig::default(),
            prelabel_config_ids: Vec::new(),
            manual_box_guide_migration: None,
            enabled: true,
        });
        metadata.tasks.push(TaskDefinition {
            task_id: task_id.clone(),
            name: "Person skeletons".to_string(),
            annotation_type: AnnotationType::Skeleton,
            class_ids: vec![class_id.clone()],
            instructions: tutorial(),
            skeleton: Some(SkeletonSpec {
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
                workflow,
                allow_reviewer_corrections: false,
                agreement_threshold: None,
            },
            prelabel_config_ids: Vec::new(),
            manual_box_guide_migration: Some(ManualBoxGuideMigration {
                guide_task_id: guide_task_id.clone(),
                cardinality: MigrationCardinality::ExactlyOne,
                allow_exclusion: true,
                sequence: MigrationSequence::ImportedSpatialOrderV1,
            }),
            enabled: true,
        });
        metadata.role_assignments.push(DatasetRoleAssignment {
            dataset_id: metadata.dataset_id.clone(),
            user_id: annotator.clone(),
            roles: BTreeSet::from([DatasetRole::Annotator]),
            assigned_at: labello_domain::now(),
            assigned_by: None,
        });
        metadata.role_assignments.push(DatasetRoleAssignment {
            dataset_id: metadata.dataset_id.clone(),
            user_id: UserId::from("admin"),
            roles: BTreeSet::from([DatasetRole::DataAdmin]),
            assigned_at: labello_domain::now(),
            assigned_by: None,
        });
        metadata
            .role_assignments
            .extend(reviewers.iter().map(|reviewer| DatasetRoleAssignment {
                dataset_id: metadata.dataset_id.clone(),
                user_id: reviewer.clone(),
                roles: BTreeSet::from([DatasetRole::Reviewer]),
                assigned_at: labello_domain::now(),
                assigned_by: None,
            }));
        repository.initialize(metadata).await.unwrap();
        repository
            .save_images_index(&ImagesIndex {
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

        let import_id = ImportId::from("import_1");
        let timestamp = labello_domain::now();
        let targets = (0..target_count)
            .map(|index| MigrationTarget {
                object_group_id: ObjectGroupId::from(format!("group_{index}")),
                guide_annotation_id: AnnotationId::from(format!("box_{index}")),
                reserved_skeleton_annotation_id: AnnotationId::from(format!("skeleton_{index}")),
                sequence_index: index as u64,
            })
            .collect::<Vec<_>>();
        let annotations = targets
            .iter()
            .enumerate()
            .map(|(index, target)| AnnotationVersion {
                annotation_id: target.guide_annotation_id.clone(),
                version: 1,
                object_group_id: Some(target.object_group_id.clone()),
                origin: AnnotationOrigin::Imported {
                    imported: ImportedOrigin {
                        import_id: import_id.clone(),
                        source_profile: SourceProfile {
                            profile_id: "test".to_string(),
                            profile_version: 1,
                        },
                        source_namespace: "test".to_string(),
                        source_object_key: format!("object_{index}"),
                        geometry_provenance: ImportGeometryProvenance::Direct,
                    },
                },
                task_id: guide_task_id.clone(),
                class_id: class_id.clone(),
                annotation_type: AnnotationType::BoundingBox,
                revision_source: RevisionSource::Import {
                    import_id: import_id.clone(),
                },
                geometry: AnnotationGeometry::BoundingBox(BoundingBox {
                    x: 0.1 + index as f32 * 0.3,
                    y: 0.1,
                    width: 0.2,
                    height: 0.2,
                }),
                author_user_id: UserId::from("importer"),
                created_at: timestamp,
                updated_at: timestamp,
                deleted: false,
            })
            .collect::<Vec<_>>();
        let target_set_hash = migration_target_set_hash(
            &MigrationHashContext {
                dataset_id: &DatasetId::from("ds"),
                image_id: &image_id,
                guide_task_id: &guide_task_id,
                target_task_id: &task_id,
            },
            &targets,
        )
        .unwrap();
        repository
            .append_payload(
                &image_id,
                &Actor {
                    user_id: UserId::from("importer"),
                    role: DatasetRole::DataAdmin,
                },
                EventPayload::ImportInitialized {
                    import_id,
                    annotations,
                    task_initializations: vec![
                        ImportTaskInitialization {
                            task_id: guide_task_id.clone(),
                            coverage: ImportCoverage::Complete,
                            initial_state: TaskState {
                                task_id: guide_task_id.clone(),
                                status: TaskStatus::Completed,
                                outcome: Some(TaskOutcome::ImportedGroundTruth),
                                assigned_to: None,
                                completed_by: Some(UserId::from("importer")),
                                completed_at: Some(timestamp),
                                updated_at: timestamp,
                            },
                        },
                        ImportTaskInitialization {
                            task_id: task_id.clone(),
                            coverage: ImportCoverage::Incomplete,
                            initial_state: TaskState::new(task_id.clone(), timestamp),
                        },
                    ],
                    migration_target_sets: vec![MigrationTargetSetInitialization {
                        dataset_id: DatasetId::from("ds"),
                        guide_task_id: guide_task_id.clone(),
                        target_task_id: task_id.clone(),
                        target_set_hash,
                        targets: targets.clone(),
                    }],
                },
            )
            .await
            .unwrap();
        Fixture {
            _temp: temp,
            repository,
            image_id,
            guide_task_id,
            task_id,
            annotator,
            reviewers,
            targets,
        }
    }

    async fn claim_annotator(fixture: &Fixture) -> Assignment {
        fixture
            .repository
            .assign_next_image(
                &fixture.annotator,
                &fixture.task_id,
                AssignmentKind::Annotation,
            )
            .await
            .unwrap()
            .unwrap()
    }

    async fn prepare_submitted_migration(fixture: &Fixture) -> Assignment {
        let annotation = claim_annotator(fixture).await;
        let state = fixture
            .repository
            .load_image_state(&fixture.image_id)
            .await
            .unwrap();
        let resolved = fixture
            .repository
            .save_migration_skeleton(
                &fixture.annotator,
                context(&annotation),
                None,
                &expectation(&state, &fixture.task_id, &fixture.targets[0]),
                skeleton(0.5),
                "invalidation-save",
            )
            .await
            .unwrap();
        let target_hash = resolved.image_state.migration_target_sets[&fixture.task_id]
            .target_set_hash
            .clone();
        let state_hash = resolved
            .image_state
            .current_migration_state_hash(&fixture.task_id)
            .unwrap();
        let confirmation_hash = migration_confirmation_hash(&target_hash, &state_hash).unwrap();
        fixture
            .repository
            .confirm_and_submit_migration(
                &fixture.annotator,
                context(&annotation),
                &target_hash,
                &state_hash,
                &confirmation_hash,
                "invalidation-submit",
            )
            .await
            .unwrap();
        let review = fixture
            .repository
            .assign_next_image(
                &fixture.reviewers[0],
                &fixture.task_id,
                AssignmentKind::Review,
            )
            .await
            .unwrap()
            .unwrap();
        let disposition_version = fixture
            .repository
            .load_image_state(&fixture.image_id)
            .await
            .unwrap()
            .migration_dispositions[&fixture.task_id][&fixture.targets[0].object_group_id]
            .disposition_version;
        fixture
            .repository
            .review_migration(
                &fixture.reviewers[0],
                context(&review),
                &MigrationReviewTarget::Disposition {
                    object_group_id: fixture.targets[0].object_group_id.clone(),
                    disposition_version,
                },
                ReviewDecision::Approved,
                None,
                "invalidation-review",
            )
            .await
            .unwrap();
        review
    }

    async fn assert_guide_invalidation(fixture: &Fixture, review: &Assignment) {
        let state = fixture
            .repository
            .rebuild_image_state(&fixture.image_id)
            .await
            .unwrap();
        assert_eq!(
            state.task_states[&fixture.task_id].status,
            TaskStatus::NeedsCorrection
        );
        assert!(!state.migration_confirmations.contains_key(&fixture.task_id));
        assert_eq!(
            state.migration_dependencies[&fixture.task_id][&fixture.targets[0].object_group_id]
                .kind,
            MigrationDependencyKind::CorrectionRequired
        );
        assert_eq!(
            assignment_status(&state, review),
            AssignmentStatus::Cancelled
        );
        let events = fixture
            .repository
            .load_events(&fixture.image_id)
            .await
            .unwrap();
        assert!(current_migration_reviews(&events, &fixture.task_id).is_empty());
    }

    async fn open_guide_assignment(fixture: &Fixture) -> Assignment {
        let now = labello_domain::now();
        let assignment = Assignment {
            assignment_id: AssignmentId::generate(),
            image_id: fixture.image_id.clone(),
            task_id: fixture.guide_task_id.clone(),
            assigned_to: fixture.annotator.clone(),
            kind: AssignmentKind::Annotation,
            status: AssignmentStatus::Active,
            expires_at: Some(lease_expiration(now)),
            created_at: now,
            updated_at: now,
        };
        fixture
            .repository
            .append_payloads_unlocked(
                &fixture.image_id,
                &Actor {
                    user_id: fixture.annotator.clone(),
                    role: DatasetRole::Annotator,
                },
                vec![
                    EventPayload::TaskStateChanged {
                        task_state: TaskState {
                            task_id: fixture.guide_task_id.clone(),
                            status: TaskStatus::InProgress,
                            outcome: None,
                            assigned_to: Some(fixture.annotator.clone()),
                            completed_by: None,
                            completed_at: None,
                            updated_at: now,
                        },
                    },
                    EventPayload::AssignmentUpdated {
                        assignment: assignment.clone(),
                    },
                ],
            )
            .await
            .unwrap();
        assignment
    }

    fn context(assignment: &Assignment) -> AssignmentContext<'_> {
        AssignmentContext {
            assignment_id: &assignment.assignment_id,
            image_id: &assignment.image_id,
            task_id: &assignment.task_id,
            kind: assignment.kind.clone(),
        }
    }

    fn expectation(
        state: &ImageState,
        task_id: &TaskId,
        target: &MigrationTarget,
    ) -> MigrationTargetExpectation {
        let guide = state
            .current_annotation(&target.guide_annotation_id)
            .unwrap();
        let skeleton = state
            .current_annotation(&target.reserved_skeleton_annotation_id)
            .filter(|annotation| !annotation.deleted);
        MigrationTargetExpectation {
            object_group_id: target.object_group_id.clone(),
            expected_guide_annotation_version: guide.version,
            expected_guide_deleted: guide.deleted,
            expected_disposition_version: state.migration_dispositions[task_id]
                [&target.object_group_id]
                .disposition_version,
            expected_skeleton_version: skeleton.map(|annotation| annotation.version),
        }
    }

    fn skeleton(x: f32) -> SkeletonGeometry {
        SkeletonGeometry {
            keypoints: vec![KeypointAnnotation {
                name: "nose".to_string(),
                state: KeypointState::Visible,
                point: Some(NormalizedPoint { x, y: 0.5 }),
            }],
        }
    }

    fn tutorial() -> TutorialContent {
        TutorialContent {
            title: "Instructions".to_string(),
            example_text: "Annotate".to_string(),
            example_images: Vec::new(),
        }
    }

    fn assignment_status(state: &ImageState, assignment: &Assignment) -> AssignmentStatus {
        state
            .assignments
            .iter()
            .find(|candidate| candidate.assignment_id == assignment.assignment_id)
            .unwrap()
            .status
            .clone()
    }
}
