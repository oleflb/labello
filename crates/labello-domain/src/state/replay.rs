use super::*;

impl ImageState {
    pub fn apply_event(&mut self, event: &EventLogEntry) -> DomainResult<()> {
        if event.image_id != self.image_id {
            return Err(DomainError::ImageMismatch {
                expected: self.image_id.to_string(),
                found: event.image_id.to_string(),
            });
        }
        let expected = self.current_sequence + 1;
        if event.event_sequence != expected {
            return Err(DomainError::InvalidEventSequence {
                expected,
                found: event.event_sequence,
            });
        }
        event.validate_shape()?;
        match &event.payload {
            EventPayload::AnnotationVersionCreated {
                annotation,
                previous_version,
                ..
            } => {
                self.apply_annotation_version(annotation.clone(), *previous_version)?;
                self.invalidate_migration_target_annotation(&annotation.annotation_id);
                if previous_version.is_some() {
                    self.mark_changed_guide(annotation, event);
                }
            }
            EventPayload::AnnotationDeleted {
                annotation_id,
                version,
                ..
            } => {
                let versions = self
                    .annotations
                    .get_mut(annotation_id)
                    .ok_or_else(|| DomainError::MissingAnnotation(annotation_id.to_string()))?;
                let current = versions
                    .last_mut()
                    .ok_or_else(|| DomainError::MissingAnnotation(annotation_id.to_string()))?;
                if current.version != *version || current.deleted {
                    return Err(DomainError::MissingAnnotationVersion {
                        annotation_id: annotation_id.to_string(),
                        version: *version,
                    });
                }
                current.deleted = true;
                self.apply_annotation_deletion(annotation_id, event);
            }
            EventPayload::TaskStateChanged { task_state } => {
                self.apply_task_state(task_state)?;
            }
            EventPayload::ReviewRecorded { review } => self.reviews.push(review.clone()),
            EventPayload::ReviewerCorrectionRecorded {
                correction,
                annotation,
                review,
                task_state,
                assignments,
            } => {
                self.apply_reviewer_correction(
                    correction,
                    annotation,
                    review,
                    task_state,
                    assignments,
                    event,
                )?;
            }
            EventPayload::AdjudicationRecorded { adjudication } => {
                self.adjudications.push(adjudication.clone());
            }
            EventPayload::AssignmentUpdated { assignment } => {
                if let Some(existing) = self
                    .assignments
                    .iter_mut()
                    .find(|candidate| candidate.assignment_id == assignment.assignment_id)
                {
                    *existing = assignment.clone();
                } else {
                    self.assignments.push(assignment.clone());
                }
            }
            EventPayload::ImportInitialized {
                import_id,
                annotations,
                task_initializations,
                migration_target_sets,
            } => self.apply_import_initialization(
                import_id,
                annotations,
                task_initializations,
                migration_target_sets,
            )?,
            EventPayload::ImportedTaskReopened { task_state, reason } => {
                let previous = self.task_states.get(&task_state.task_id).ok_or_else(|| {
                    DomainError::InvalidImport("imported task state is missing".into())
                })?;
                if reason.trim().is_empty()
                    || !matches!(
                        self.import_coverage.get(&task_state.task_id),
                        Some(ImportCoverage::Complete | ImportCoverage::VerifiedEmpty)
                    )
                    || !matches!(
                        previous.status,
                        TaskStatus::Completed | TaskStatus::Submitted
                    )
                    || !matches!(
                        task_state.status,
                        TaskStatus::Pending | TaskStatus::Submitted
                    )
                {
                    return Err(DomainError::InvalidImport(
                        "imported task reopen transition is invalid".into(),
                    ));
                }
                self.apply_task_state(task_state)?;
            }
            EventPayload::ImportCoverageIncluded { task_state, reason } => {
                if reason.trim().is_empty()
                    || self.import_coverage.get(&task_state.task_id)
                        != Some(&ImportCoverage::Excluded)
                    || task_state.status != TaskStatus::Pending
                    || !self
                        .included_import_tasks
                        .insert(task_state.task_id.clone())
                {
                    return Err(DomainError::InvalidImport(
                        "excluded import coverage include transition is invalid".into(),
                    ));
                }
                self.apply_task_state(task_state)?;
            }
            EventPayload::MigrationDispositionChanged {
                task_id,
                object_group_id,
                disposition,
            } => self.apply_migration_disposition(
                task_id,
                object_group_id,
                disposition,
                false,
                event,
            )?,
            EventPayload::MigrationDispositionReopened {
                task_id,
                object_group_id,
                disposition,
            } => self.apply_migration_disposition(
                task_id,
                object_group_id,
                disposition,
                true,
                event,
            )?,
            EventPayload::MigrationDependencyMarked {
                task_id,
                object_group_id,
                marker,
            } => self.apply_dependency_marker(task_id, object_group_id, marker)?,
            EventPayload::MigrationDependencyCleared {
                task_id,
                object_group_id,
                marker_version,
            } => {
                let existing = self
                    .migration_dependencies
                    .get(task_id)
                    .and_then(|markers| markers.get(object_group_id))
                    .cloned()
                    .ok_or_else(|| DomainError::InvalidMigration("dependency is missing".into()))?;
                if existing.marker_version != *marker_version {
                    return Err(DomainError::InvalidMigration(
                        "dependency marker version is stale".into(),
                    ));
                }
                let guide = self
                    .migration_target(task_id, object_group_id)
                    .ok()
                    .and_then(|target| self.current_annotation(&target.guide_annotation_id));
                let disposition = self
                    .migration_dispositions
                    .get(task_id)
                    .and_then(|values| values.get(object_group_id));
                let disposition_is_newer = disposition.is_some_and(|disposition| {
                    disposition.disposition_version > existing.required_disposition_version
                });
                let dependency_is_resolved = match existing.kind {
                    MigrationDependencyKind::CorrectionRequired => {
                        guide.is_some_and(|guide| !guide.deleted) && disposition_is_newer
                    }
                    MigrationDependencyKind::GuideUnavailable => {
                        disposition_is_newer
                            && guide.is_some_and(|guide| {
                                !guide.deleted
                                    || disposition.is_some_and(|disposition| {
                                        matches!(
                                            disposition.status,
                                            MigrationDispositionStatus::Excluded { .. }
                                        )
                                    })
                            })
                    }
                };
                if !dependency_is_resolved {
                    return Err(DomainError::InvalidMigration(
                        "migration dependency cannot clear before a compatible new disposition"
                            .into(),
                    ));
                }
                self.migration_dependencies
                    .get_mut(task_id)
                    .expect("validated above")
                    .remove(object_group_id);
                self.migration_confirmations.remove(task_id);
            }
            EventPayload::MigrationPassStarted { pass } => {
                self.apply_migration_pass_started(pass)?;
            }
            EventPayload::MigrationPassItemRecorded { pass_id, item } => {
                self.apply_migration_pass_item(pass_id, item)?;
            }
            EventPayload::MigrationFullImageConfirmed { confirmation } => {
                self.apply_migration_confirmation(confirmation, event)?;
            }
        }
        self.current_sequence = event.event_sequence;
        Ok(())
    }

    pub(super) fn apply_task_state(&mut self, task_state: &TaskState) -> DomainResult<()> {
        let terminal = matches!(
            task_state.status,
            TaskStatus::Submitted | TaskStatus::Completed
        );
        if self.migration_target_sets.contains_key(&task_state.task_id) {
            if terminal {
                self.validate_migration_terminal(&task_state.task_id)?;
            } else {
                self.migration_confirmations.remove(&task_state.task_id);
            }
        }
        if terminal
            && self.active_annotations().any(|annotation| {
                annotation.task_id == task_state.task_id
                    && matches!(
                        &annotation.origin,
                        AnnotationOrigin::Imported { imported }
                            if matches!(
                                imported.geometry_provenance,
                                crate::ImportGeometryProvenance::Derived { .. }
                            )
                    )
                    && matches!(annotation.revision_source, RevisionSource::Import { .. })
            })
        {
            return Err(DomainError::InvalidImport(format!(
                "task {} contains derived objects without per-object human acceptance or edit",
                task_state.task_id
            )));
        }
        self.task_states
            .insert(task_state.task_id.clone(), task_state.clone());
        Ok(())
    }
}
