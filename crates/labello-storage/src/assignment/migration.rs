use labello_domain::{
    AnnotationGeometry, AnnotationId, AnnotationOrigin, AnnotationType, AnnotationVersion,
    Assignment, AssignmentId, AssignmentKind, AssignmentStatus, ClassId, DatasetMetadata,
    DatasetRole, EventId, EventLogEntry, EventPayload, HumanRevisionKind, ImageId, ImageRecord,
    ImageState, MigrationConfirmation, MigrationCursor, MigrationDependencyKind,
    MigrationDisposition, MigrationDispositionStatus, MigrationExclusion, MigrationExclusionReason,
    MigrationHash, MigrationPass, MigrationPassId, MigrationPassItem, MigrationPassItemAction,
    ObjectGroupId, ReviewDecision, ReviewId, ReviewRecord, ReviewTarget, ReviewWorkflow,
    RevisionSource, SkeletonGeometry, TaskDefinition, TaskId, TaskOutcome, TaskState, TaskStatus,
    Timestamp, UserId, migration_confirmation_hash, require_role,
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
        let (metadata, _image) = self.load_migration_inputs(context.image_id).await?;
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
        validate_migration_task(&metadata, context.task_id)?;
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

    async fn load_migration_inputs(
        &self,
        image_id: &ImageId,
    ) -> StorageResult<(DatasetMetadata, ImageRecord)> {
        let metadata = self.load_dataset_config().await?;
        let image = self.load_image_record(image_id).await?;
        Ok((metadata, image))
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the migration command boundary keeps actor, assignment, and version inputs explicit"
    )]
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
        let (metadata, image) = self.load_migration_inputs(context.image_id).await?;
        let (task, guide_task, image_dimensions) =
            migration_metadata(&metadata, &image, context.task_id)?;
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

    pub async fn add_migration_skeleton(
        &self,
        user_id: &UserId,
        context: AssignmentContext<'_>,
        pass_id: Option<&MigrationPassId>,
        skeleton: SkeletonGeometry,
        idempotency_key: &str,
    ) -> StorageResult<ManualMigrationCommandResult> {
        validate_idempotency_key(idempotency_key)?;
        let (metadata, image) = self.load_migration_inputs(context.image_id).await?;
        let (task, _, image_dimensions) = migration_metadata(&metadata, &image, context.task_id)?;
        require_annotation_context(&metadata, user_id, &context)?;
        let lock = self.image_lock(context.image_id);
        let _guard = lock.lock().await;
        let mut state = self.load_image_state(context.image_id).await?;
        let primary_id = command_event_id(user_id, context.assignment_id, idempotency_key, None);
        if let Some(command) = find_command(self, context.image_id, &primary_id).await? {
            let matches = matches!(
                &command.primary.payload,
                EventPayload::AnnotationVersionCreated { annotation, previous_version: None, .. }
                    if annotation.task_id == *context.task_id
                        && annotation.object_group_id.is_none()
                        && annotation.geometry == AnnotationGeometry::Skeleton(skeleton.clone())
            ) && assignment_pass(
                &command.before,
                context.assignment_id,
                context.task_id,
            ) == pass_id;
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
        if assignment_pass(&state, context.assignment_id, context.task_id) != pass_id {
            return Err(conflict(
                "migration discovery pass does not match the assignment's active pass",
            ));
        }
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
        if state.migration_cursor(context.task_id, pass_id)? != MigrationCursor::FullImage {
            return Err(conflict(
                "missing objects can be added only during full-image confirmation",
            ));
        }
        validate_exact_one(&state, task)?;

        let annotation = AnnotationVersion {
            annotation_id: AnnotationId::from(format!(
                "ann_migration_discovered_{}",
                command_digest(user_id, context.assignment_id, idempotency_key)
            )),
            version: 1,
            object_group_id: None,
            origin: AnnotationOrigin::native(),
            task_id: context.task_id.clone(),
            class_id: only_class(task)?.clone(),
            annotation_type: AnnotationType::Skeleton,
            revision_source: RevisionSource::Human {
                action: HumanRevisionKind::Authored,
            },
            geometry: AnnotationGeometry::Skeleton(skeleton),
            author_user_id: user_id.clone(),
            created_at: now,
            updated_at: now,
            deleted: false,
        };
        annotation
            .validate_for_task(task, image_dimensions)
            .map_err(|error| StorageError::InvalidAssignment(error.to_string()))?;
        let mut payloads = Vec::new();
        push_simulated(
            &mut state,
            &mut payloads,
            user_id,
            DatasetRole::Annotator,
            now,
            EventPayload::AnnotationVersionCreated {
                annotation: annotation.clone(),
                previous_version: None,
                reason: Some("object discovered during full-image migration review".to_string()),
            },
        )?;
        reopen_terminal_migration(&mut state, context.task_id, now, &mut payloads, user_id)?;
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
        command_result(
            state,
            context.task_id,
            pass_id,
            Some(assignment),
            Some(annotation.annotation_id),
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the migration command boundary keeps actor, assignment, and version inputs explicit"
    )]
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
        let (metadata, image) = self.load_migration_inputs(context.image_id).await?;
        let (task, guide_task, _) = migration_metadata(&metadata, &image, context.task_id)?;
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
        let (metadata, image) = self.load_migration_inputs(context.image_id).await?;
        let (task, guide_task, _) = migration_metadata(&metadata, &image, context.task_id)?;
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

    pub async fn revisit_migration_target(
        &self,
        user_id: &UserId,
        context: AssignmentContext<'_>,
        pass_id: Option<&MigrationPassId>,
        expected: &MigrationTargetExpectation,
        idempotency_key: &str,
    ) -> StorageResult<ManualMigrationCommandResult> {
        validate_idempotency_key(idempotency_key)?;
        let (metadata, image) = self.load_migration_inputs(context.image_id).await?;
        let (task, guide_task, _) = migration_metadata(&metadata, &image, context.task_id)?;
        require_annotation_context(&metadata, user_id, &context)?;
        let lock = self.image_lock(context.image_id);
        let _guard = lock.lock().await;
        let state = self.load_image_state(context.image_id).await?;
        let primary_id = command_event_id(user_id, context.assignment_id, idempotency_key, None);
        if let Some(command) = find_command(self, context.image_id, &primary_id).await? {
            let matches = matches!(
                &command.primary.payload,
                EventPayload::MigrationDependencyMarked {
                    task_id,
                    object_group_id,
                    marker,
                } if task_id == context.task_id
                    && object_group_id == &expected.object_group_id
                    && marker.event_id == primary_id
            ) && expectation_matches(&command.before, context.task_id, expected)
                && assignment_pass(&command.before, context.assignment_id, context.task_id)
                    == pass_id;
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
        if assignment_pass(&state, context.assignment_id, context.task_id) != pass_id {
            return Err(conflict(
                "migration revisit pass does not match the assignment's active pass",
            ));
        }
        let target = migration_target(&state, context.task_id, &expected.object_group_id)?;
        validate_target_identity(&state, task, guide_task, target, expected)?;
        if has_dependency(&state, context.task_id, &expected.object_group_id) {
            return Err(conflict("migration target already requires correction"));
        }
        if matches!(
            current_disposition(&state, context.task_id, &expected.object_group_id)?.status,
            MigrationDispositionStatus::Pending
        ) {
            return Err(conflict("pending migration target cannot be revisited"));
        }
        match state.migration_cursor(context.task_id, pass_id)? {
            MigrationCursor::Object { sequence_index, .. }
                if target.sequence_index < sequence_index => {}
            MigrationCursor::FullImage => {}
            _ => {
                return Err(conflict(
                    "only a previously resolved migration target can be revisited",
                ));
            }
        }
        let guide = state
            .current_annotation(&target.guide_annotation_id)
            .ok_or_else(|| conflict("migration guide is missing"))?;
        let marker = labello_domain::MigrationDependencyMarker {
            marker_version: 1,
            kind: if guide.deleted {
                MigrationDependencyKind::GuideUnavailable
            } else {
                MigrationDependencyKind::CorrectionRequired
            },
            required_disposition_version: expected.expected_disposition_version,
            event_id: primary_id,
            timestamp: now,
        };
        let payloads = vec![
            EventPayload::MigrationDependencyMarked {
                task_id: context.task_id.clone(),
                object_group_id: expected.object_group_id.clone(),
                marker,
            },
            {
                renew(&mut assignment, now);
                EventPayload::AssignmentUpdated {
                    assignment: assignment.clone(),
                }
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
        let (metadata, image) = self.load_migration_inputs(context.image_id).await?;
        migration_metadata(&metadata, &image, context.task_id)?;
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
        let (metadata, image) = self.load_migration_inputs(context.image_id).await?;
        let (task, guide_task, _) = migration_metadata(&metadata, &image, context.task_id)?;
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
        let (metadata, image) = self.load_migration_inputs(context.image_id).await?;
        let (task, _, _) = migration_metadata(&metadata, &image, context.task_id)?;
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

    #[allow(
        clippy::too_many_arguments,
        reason = "the migration review boundary keeps actor, assignment, and decision inputs explicit"
    )]
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
        let (metadata, image) = self.load_migration_inputs(context.image_id).await?;
        let (task, _, _) = migration_metadata(&metadata, &image, context.task_id)?;
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

    #[allow(
        clippy::too_many_arguments,
        reason = "the transaction helper keeps lock-protected command inputs visible"
    )]
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
    image: &ImageRecord,
    task_id: &TaskId,
) -> StorageResult<(
    &'a TaskDefinition,
    &'a TaskDefinition,
    labello_domain::ImageDimensions,
)> {
    let task = validate_migration_task(metadata, task_id)?;
    let config = task.manual_box_guide_migration.as_ref().expect("validated");
    let guide = metadata.task(&config.guide_task_id).ok_or_else(|| {
        StorageError::InvalidAssignment("manual migration guide task is missing".to_string())
    })?;
    task.validate_manual_migration(guide)
        .map_err(StorageError::Domain)?;
    Ok((task, guide, image.dimensions()))
}

fn validate_migration_task<'a>(
    metadata: &'a DatasetMetadata,
    task_id: &TaskId,
) -> StorageResult<&'a TaskDefinition> {
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
        let discovered = annotation.object_group_id.is_none()
            && annotation.class_id == *only_class(task).expect("migration class validated")
            && annotation.annotation_type == AnnotationType::Skeleton
            && matches!(
                annotation.origin,
                AnnotationOrigin::Native { legacy_v2: false }
            )
            && matches!(
                annotation.revision_source,
                RevisionSource::Human {
                    action: HumanRevisionKind::Authored | HumanRevisionKind::Edited
                }
            );
        if !valid && !discovered {
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

#[allow(
    clippy::too_many_arguments,
    reason = "dependency clearing is a pure transition over explicit migration facts"
)]
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

fn assignment_pass<'a>(
    state: &'a ImageState,
    assignment_id: &AssignmentId,
    task_id: &TaskId,
) -> Option<&'a MigrationPassId> {
    state
        .migration_passes
        .values()
        .filter(|pass| pass.assignment_id == *assignment_id && pass.task_id == *task_id)
        .max_by_key(|pass| pass.started_at)
        .map(|pass| &pass.pass_id)
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
mod tests;
