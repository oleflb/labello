use super::*;

impl ImageState {
    pub(super) fn apply_annotation_version(
        &mut self,
        annotation: AnnotationVersion,
        previous_version: Option<u32>,
    ) -> DomainResult<()> {
        let versions = self
            .annotations
            .entry(annotation.annotation_id.clone())
            .or_default();
        if let Some(previous_version) = previous_version {
            let Some(previous) = versions.last() else {
                return Err(DomainError::MissingAnnotation(
                    annotation.annotation_id.to_string(),
                ));
            };
            if previous.version != previous_version || annotation.version != previous_version + 1 {
                return Err(DomainError::MissingAnnotationVersion {
                    annotation_id: annotation.annotation_id.to_string(),
                    version: previous_version,
                });
            }
            if previous.origin != annotation.origin
                || previous.object_group_id != annotation.object_group_id
            {
                return Err(DomainError::InvalidImport(format!(
                    "annotation {} changed immutable origin or object group",
                    annotation.annotation_id
                )));
            }
        } else if annotation.version != 1 || !versions.is_empty() {
            return Err(DomainError::InvalidGeometry(format!(
                "annotation {} version chain is invalid",
                annotation.annotation_id
            )));
        }
        versions.push(annotation);
        Ok(())
    }

    pub(super) fn apply_import_initialization(
        &mut self,
        import_id: &ImportId,
        annotations: &[AnnotationVersion],
        task_initializations: &[crate::ImportTaskInitialization],
        target_sets: &[MigrationTargetSetInitialization],
    ) -> DomainResult<()> {
        if annotations.len() > crate::MAX_IMPORT_ANNOTATIONS_PER_EVENT
            || task_initializations.len() > crate::MAX_IMPORT_TASKS_PER_EVENT
            || target_sets
                .iter()
                .any(|set| set.targets.len() > crate::MAX_MIGRATION_TARGETS_PER_EVENT)
        {
            return Err(DomainError::InvalidImport(
                "compact import event exceeds a bounded vector limit".into(),
            ));
        }
        if !self.import_ids.insert(import_id.clone()) {
            return Err(DomainError::InvalidImport(format!(
                "import {import_id} is already initialized"
            )));
        }
        for annotation in annotations {
            let valid_origin = matches!(
                &annotation.origin,
                AnnotationOrigin::Imported { imported } if &imported.import_id == import_id
            );
            let valid_revision = matches!(
                &annotation.revision_source,
                RevisionSource::Import { import_id: revision_import_id }
                    if revision_import_id == import_id
            );
            if !valid_origin || !valid_revision || annotation.version != 1 {
                return Err(DomainError::InvalidImport(format!(
                    "annotation {} is not an initial version from import {}",
                    annotation.annotation_id, import_id
                )));
            }
            self.apply_annotation_version(annotation.clone(), None)?;
        }
        for initialization in task_initializations {
            if initialization.task_id != initialization.initial_state.task_id
                || self
                    .import_coverage
                    .insert(initialization.task_id.clone(), initialization.coverage)
                    .is_some()
            {
                return Err(DomainError::InvalidImport(format!(
                    "task {} has duplicate or inconsistent import initialization",
                    initialization.task_id
                )));
            }
            if matches!(
                initialization.coverage,
                ImportCoverage::Incomplete | ImportCoverage::Excluded
            ) && initialization.initial_state.status != TaskStatus::Pending
            {
                return Err(DomainError::InvalidImport(format!(
                    "task {} cannot initialize non-authoritative coverage as {:?}",
                    initialization.task_id, initialization.initial_state.status
                )));
            }
            if initialization.initial_state.status == TaskStatus::Completed
                && (!matches!(
                    initialization.coverage,
                    ImportCoverage::Complete | ImportCoverage::VerifiedEmpty
                ) || initialization.initial_state.outcome
                    != Some(TaskOutcome::ImportedGroundTruth))
            {
                return Err(DomainError::InvalidImport(format!(
                    "task {} has an invalid imported completion",
                    initialization.task_id
                )));
            }
            self.apply_task_state(&initialization.initial_state)?;
        }
        for set in target_sets {
            if self
                .task_states
                .get(&set.target_task_id)
                .is_some_and(|task_state| {
                    matches!(
                        task_state.status,
                        TaskStatus::Submitted | TaskStatus::Completed
                    )
                })
            {
                return Err(DomainError::InvalidMigration(
                    "manual migration cannot import a terminal task without canonical confirmation"
                        .into(),
                ));
            }
            if self.migration_target_sets.contains_key(&set.target_task_id) {
                return Err(DomainError::InvalidMigration(format!(
                    "task {} already has migration targets",
                    set.target_task_id
                )));
            }
            let expected_hash = migration_target_set_hash(
                &MigrationHashContext {
                    dataset_id: &set.dataset_id,
                    image_id: &self.image_id,
                    guide_task_id: &set.guide_task_id,
                    target_task_id: &set.target_task_id,
                },
                &set.targets,
            )?;
            if expected_hash != set.target_set_hash {
                return Err(DomainError::InvalidMigration(
                    "migration target-set hash does not match targets".into(),
                ));
            }
            let mut dispositions = BTreeMap::new();
            for target in &set.targets {
                let guide = self
                    .current_annotation(&target.guide_annotation_id)
                    .ok_or_else(|| {
                        DomainError::InvalidMigration(format!(
                            "guide annotation {} is missing",
                            target.guide_annotation_id
                        ))
                    })?;
                if guide.task_id != set.guide_task_id
                    || guide.object_group_id.as_ref() != Some(&target.object_group_id)
                    || dispositions
                        .insert(
                            target.object_group_id.clone(),
                            MigrationDisposition::pending(),
                        )
                        .is_some()
                {
                    return Err(DomainError::InvalidMigration(format!(
                        "migration target {} has an invalid guide or duplicate group",
                        target.object_group_id
                    )));
                }
            }
            self.migration_dispositions
                .insert(set.target_task_id.clone(), dispositions);
            self.migration_target_sets
                .insert(set.target_task_id.clone(), set.clone());
        }
        Ok(())
    }

    pub(super) fn mark_changed_guide(
        &mut self,
        annotation: &AnnotationVersion,
        event: &EventLogEntry,
    ) {
        let affected = self
            .migration_target_sets
            .iter()
            .flat_map(|(task_id, set)| {
                set.targets
                    .iter()
                    .filter(|target| target.guide_annotation_id == annotation.annotation_id)
                    .map(move |target| (task_id.clone(), target.object_group_id.clone()))
            })
            .collect::<Vec<_>>();
        for (task_id, group_id) in affected {
            let version = self
                .migration_dependencies
                .get(&task_id)
                .and_then(|markers| markers.get(&group_id))
                .map_or(1, |marker| marker.marker_version + 1);
            let required_disposition_version =
                self.migration_dispositions[&task_id][&group_id].disposition_version;
            self.migration_dependencies
                .entry(task_id.clone())
                .or_default()
                .insert(
                    group_id,
                    MigrationDependencyMarker {
                        marker_version: version,
                        kind: MigrationDependencyKind::CorrectionRequired,
                        required_disposition_version,
                        event_id: event.event_id.clone(),
                        timestamp: event.timestamp,
                    },
                );
            self.migration_confirmations.remove(&task_id);
        }
    }

    pub(super) fn invalidate_migration_target_annotation(&mut self, annotation_id: &AnnotationId) {
        let mut affected = self
            .migration_target_sets
            .iter()
            .filter(|(_, set)| {
                set.targets.iter().any(|target| {
                    target.guide_annotation_id == *annotation_id
                        || target.reserved_skeleton_annotation_id == *annotation_id
                })
            })
            .map(|(task_id, _)| task_id.clone())
            .collect::<Vec<_>>();
        if let Some(annotation) = self.current_annotation(annotation_id)
            && self.migration_target_sets.contains_key(&annotation.task_id)
        {
            affected.push(annotation.task_id.clone());
        }
        affected.sort();
        affected.dedup();
        for task_id in affected {
            self.migration_confirmations.remove(&task_id);
        }
    }

    pub(super) fn apply_annotation_deletion(
        &mut self,
        annotation_id: &AnnotationId,
        event: &EventLogEntry,
    ) {
        let affected = self
            .migration_target_sets
            .iter()
            .flat_map(|(task_id, set)| {
                set.targets.iter().filter_map(move |target| {
                    if &target.guide_annotation_id == annotation_id {
                        Some((task_id.clone(), target.object_group_id.clone(), true))
                    } else if &target.reserved_skeleton_annotation_id == annotation_id {
                        Some((task_id.clone(), target.object_group_id.clone(), false))
                    } else {
                        None
                    }
                })
            })
            .collect::<Vec<_>>();
        if let Some(task_id) = self
            .current_annotation(annotation_id)
            .map(|annotation| annotation.task_id.clone())
            && self.migration_target_sets.contains_key(&task_id)
            && !affected
                .iter()
                .any(|(affected_task_id, _, _)| affected_task_id == &task_id)
        {
            // Discovered skeletons have no migration disposition to update,
            // but their removal still invalidates a prior full-image digest.
            self.migration_confirmations.remove(&task_id);
        }
        for (task_id, group_id, guide) in affected {
            if guide {
                let version = self
                    .migration_dependencies
                    .get(&task_id)
                    .and_then(|markers| markers.get(&group_id))
                    .map_or(1, |marker| marker.marker_version + 1);
                let required_disposition_version =
                    self.migration_dispositions[&task_id][&group_id].disposition_version;
                self.migration_dependencies
                    .entry(task_id.clone())
                    .or_default()
                    .insert(
                        group_id,
                        MigrationDependencyMarker {
                            marker_version: version,
                            kind: MigrationDependencyKind::GuideUnavailable,
                            required_disposition_version,
                            event_id: event.event_id.clone(),
                            timestamp: event.timestamp,
                        },
                    );
            } else if let Some(disposition) = self
                .migration_dispositions
                .get_mut(&task_id)
                .and_then(|values| values.get_mut(&group_id))
            {
                disposition.disposition_version += 1;
                disposition.status = MigrationDispositionStatus::Pending;
            }
            self.migration_confirmations.remove(&task_id);
        }
    }

    pub(super) fn apply_reviewer_correction(
        &mut self,
        correction: &ReviewerCorrectionRecord,
        annotation: &AnnotationVersion,
        review: &ReviewRecord,
        task_state: &TaskState,
        assignments: &[Assignment],
        event: &EventLogEntry,
    ) -> DomainResult<()> {
        let valid_review = review.decision == ReviewDecision::Rejected
            && review.reviewer_user_id == correction.reviewer_user_id
            && matches!(
                &review.target,
                ReviewTarget::AnnotationVersion {
                    annotation_id,
                    version,
                } if annotation_id == &correction.annotation_id
                    && *version == correction.previous_version
            );
        let valid_annotation = annotation.annotation_id == correction.annotation_id
            && annotation.task_id == correction.task_id
            && annotation.version == correction.corrected_version
            && correction.corrected_version == correction.previous_version + 1
            && annotation.author_user_id == correction.reviewer_user_id
            && matches!(
                &annotation.revision_source,
                crate::RevisionSource::ReviewerCorrection { correction_id }
                    if correction_id == &correction.correction_id
            );
        let valid_task_state = task_state.task_id == correction.task_id
            && task_state.status == TaskStatus::Completed
            && task_state.outcome == Some(TaskOutcome::ReviewerCorrected)
            && task_state.completed_by.as_ref() == Some(&correction.reviewer_user_id);
        let valid_assignments = assignments.iter().any(|assignment| {
            assignment.assignment_id == correction.assignment_id
                && assignment.assigned_to == correction.reviewer_user_id
                && assignment.status == AssignmentStatus::Completed
        }) && assignments.iter().all(|assignment| {
            assignment.image_id == self.image_id
                && assignment.task_id == correction.task_id
                && assignment.kind == AssignmentKind::Review
                && matches!(
                    assignment.status,
                    AssignmentStatus::Completed | AssignmentStatus::Cancelled
                )
        });
        if !valid_review || !valid_annotation || !valid_task_state || !valid_assignments {
            return Err(DomainError::InvalidReviewerCorrection(
                correction.correction_id.to_string(),
            ));
        }
        if self
            .reviewer_corrections
            .iter()
            .any(|candidate| candidate.correction_id == correction.correction_id)
        {
            return Err(DomainError::DuplicateReviewerCorrection(
                correction.correction_id.to_string(),
            ));
        }

        self.apply_annotation_version(annotation.clone(), Some(correction.previous_version))?;
        self.mark_changed_guide(annotation, event);
        self.reviews.push(review.clone());
        self.reviewer_corrections.push(correction.clone());
        self.task_states
            .insert(task_state.task_id.clone(), task_state.clone());
        for assignment in assignments {
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
        Ok(())
    }
}
