use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    AdjudicationRecord, AnnotationId, AnnotationOrigin, AnnotationVersion, Assignment,
    AssignmentKind, AssignmentStatus, DomainError, DomainResult, EventLogEntry, EventPayload,
    HumanRevisionKind, ImageId, ImportCoverage, ImportId, MigrationConfirmation,
    MigrationDependencyKind, MigrationDependencyMarker, MigrationDisposition,
    MigrationDispositionStatus, MigrationHashContext, MigrationHashStateTarget, MigrationPass,
    MigrationPassId, MigrationTargetSetInitialization, ObjectGroupId, ReviewDecision, ReviewRecord,
    ReviewTarget, ReviewerCorrectionRecord, RevisionSource, SCHEMA_VERSION, TaskId, TaskOutcome,
    TaskState, TaskStatus, migration_confirmation_hash, migration_state_hash,
    migration_target_set_hash,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImageState {
    pub schema_version: u32,
    pub image_id: ImageId,
    pub current_sequence: u64,
    pub annotations: BTreeMap<AnnotationId, Vec<AnnotationVersion>>,
    pub reviews: Vec<ReviewRecord>,
    pub reviewer_corrections: Vec<ReviewerCorrectionRecord>,
    pub adjudications: Vec<AdjudicationRecord>,
    pub task_states: BTreeMap<TaskId, TaskState>,
    pub assignments: Vec<Assignment>,
    #[serde(default)]
    pub import_ids: BTreeSet<ImportId>,
    #[serde(default)]
    pub import_coverage: BTreeMap<TaskId, ImportCoverage>,
    #[serde(default)]
    pub included_import_tasks: BTreeSet<TaskId>,
    #[serde(default)]
    pub migration_target_sets: BTreeMap<TaskId, MigrationTargetSetInitialization>,
    #[serde(default)]
    pub migration_dispositions: BTreeMap<TaskId, BTreeMap<ObjectGroupId, MigrationDisposition>>,
    #[serde(default)]
    pub migration_dependencies:
        BTreeMap<TaskId, BTreeMap<ObjectGroupId, MigrationDependencyMarker>>,
    #[serde(default)]
    pub migration_passes: BTreeMap<MigrationPassId, MigrationPass>,
    #[serde(default)]
    pub migration_confirmations: BTreeMap<TaskId, MigrationConfirmation>,
}

impl ImageState {
    pub fn new(image_id: ImageId) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            image_id,
            current_sequence: 0,
            annotations: BTreeMap::new(),
            reviews: Vec::new(),
            reviewer_corrections: Vec::new(),
            adjudications: Vec::new(),
            task_states: BTreeMap::new(),
            assignments: Vec::new(),
            import_ids: BTreeSet::new(),
            import_coverage: BTreeMap::new(),
            included_import_tasks: BTreeSet::new(),
            migration_target_sets: BTreeMap::new(),
            migration_dispositions: BTreeMap::new(),
            migration_dependencies: BTreeMap::new(),
            migration_passes: BTreeMap::new(),
            migration_confirmations: BTreeMap::new(),
        }
    }

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

    pub fn current_annotation(&self, annotation_id: &AnnotationId) -> Option<&AnnotationVersion> {
        self.annotations
            .get(annotation_id)
            .and_then(|versions| versions.last())
    }

    pub fn active_annotations(&self) -> impl Iterator<Item = &AnnotationVersion> {
        self.annotations
            .values()
            .filter_map(|versions| versions.last())
            .filter(|v| !v.deleted)
    }

    pub fn assignment_eligible(&self, task_id: &TaskId) -> bool {
        if self.import_coverage.get(task_id) == Some(&ImportCoverage::Excluded)
            && !self.included_import_tasks.contains(task_id)
        {
            return false;
        }
        matches!(
            self.task_states.get(task_id).map(|state| &state.status),
            None | Some(TaskStatus::Pending | TaskStatus::InProgress | TaskStatus::NeedsCorrection)
        )
    }

    pub fn included_in_completion_denominator(&self, task_id: &TaskId) -> bool {
        self.import_coverage.get(task_id) != Some(&ImportCoverage::Excluded)
            || self.included_import_tasks.contains(task_id)
    }

    pub fn migration_cursor(
        &self,
        task_id: &TaskId,
        pass_id: Option<&MigrationPassId>,
    ) -> DomainResult<crate::MigrationCursor> {
        let set = self.migration_target_sets.get(task_id).ok_or_else(|| {
            DomainError::InvalidMigration("migration target set is missing".into())
        })?;
        let mut targets = set.targets.iter().collect::<Vec<_>>();
        targets.sort_by_key(|target| target.sequence_index);
        let pass = pass_id
            .map(|pass_id| {
                self.migration_passes.get(pass_id).ok_or_else(|| {
                    DomainError::InvalidMigration(format!("migration pass {pass_id} is missing"))
                })
            })
            .transpose()?;
        if pass.is_some_and(|pass| pass.task_id != *task_id) {
            return Err(DomainError::InvalidMigration(
                "migration pass belongs to another task".into(),
            ));
        }
        for target in targets {
            let guide = self
                .current_annotation(&target.guide_annotation_id)
                .ok_or_else(|| {
                    DomainError::InvalidMigration("migration guide is missing".into())
                })?;
            let disposition = &self.migration_dispositions[task_id][&target.object_group_id];
            let dependency = self
                .migration_dependencies
                .get(task_id)
                .and_then(|markers| markers.get(&target.object_group_id));
            let unresolved = if let Some(pass) = pass {
                dependency.is_some()
                    || !pass.items.iter().any(|item| {
                        item.matches_target_state(
                            &target.object_group_id,
                            guide.version,
                            guide.deleted,
                            disposition.disposition_version,
                        )
                    })
            } else {
                dependency.is_some()
                    || matches!(disposition.status, MigrationDispositionStatus::Pending)
            };
            if unresolved {
                return Ok(crate::MigrationCursor::Object {
                    object_group_id: target.object_group_id.clone(),
                    sequence_index: target.sequence_index,
                });
            }
        }
        Ok(crate::MigrationCursor::FullImage)
    }

    fn apply_annotation_version(
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

    fn apply_import_initialization(
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

    fn apply_task_state(&mut self, task_state: &TaskState) -> DomainResult<()> {
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

    fn apply_migration_disposition(
        &mut self,
        task_id: &TaskId,
        object_group_id: &ObjectGroupId,
        disposition: &MigrationDisposition,
        reopened: bool,
        event: &EventLogEntry,
    ) -> DomainResult<()> {
        let target = self.migration_target(task_id, object_group_id)?.clone();
        let current = self
            .migration_dispositions
            .get(task_id)
            .and_then(|values| values.get(object_group_id))
            .ok_or_else(|| {
                DomainError::InvalidMigration("migration disposition is missing".into())
            })?;
        if disposition.disposition_version != current.disposition_version + 1
            || reopened != matches!(disposition.status, MigrationDispositionStatus::Pending)
            || (reopened && !matches!(current.status, MigrationDispositionStatus::Excluded { .. }))
        {
            return Err(DomainError::InvalidMigration(
                "migration disposition transition is invalid".into(),
            ));
        }
        match &disposition.status {
            MigrationDispositionStatus::Pending => {}
            MigrationDispositionStatus::Annotated {
                skeleton_annotation_id,
                skeleton_version,
            } => {
                if skeleton_annotation_id != &target.reserved_skeleton_annotation_id
                    || self
                        .migration_dependencies
                        .get(task_id)
                        .is_some_and(|markers| markers.contains_key(object_group_id))
                {
                    return Err(DomainError::InvalidMigration(
                        "annotated disposition has the wrong reserved ID or dependency".into(),
                    ));
                }
                let skeleton =
                    self.current_annotation(skeleton_annotation_id)
                        .ok_or_else(|| {
                            DomainError::InvalidMigration("migration skeleton is missing".into())
                        })?;
                if skeleton.version != *skeleton_version
                    || skeleton.deleted
                    || skeleton.task_id != *task_id
                    || skeleton.object_group_id.as_ref() != Some(object_group_id)
                    || !matches!(skeleton.origin, AnnotationOrigin::Native { .. })
                    || !matches!(
                        skeleton.revision_source,
                        RevisionSource::Human {
                            action: HumanRevisionKind::Authored
                                | HumanRevisionKind::Edited
                                | HumanRevisionKind::AcceptedUnchanged
                        }
                    )
                {
                    return Err(DomainError::InvalidMigration(
                        "migration skeleton does not satisfy the expected target".into(),
                    ));
                }
            }
            MigrationDispositionStatus::Excluded { exclusion } => {
                if exclusion.event_id != event.event_id
                    || exclusion.actor_user_id != event.actor_user_id
                    || exclusion.timestamp != event.timestamp
                {
                    return Err(DomainError::InvalidMigration(
                        "migration exclusion audit does not match its event".into(),
                    ));
                }
                if self
                    .current_annotation(&target.reserved_skeleton_annotation_id)
                    .is_some_and(|annotation| !annotation.deleted)
                {
                    return Err(DomainError::InvalidMigration(
                        "migration skeleton must be deleted before exclusion".into(),
                    ));
                }
                if exclusion.reason == crate::MigrationExclusionReason::Other
                    && exclusion.note.as_deref().is_none_or(str::is_empty)
                {
                    return Err(DomainError::InvalidMigration(
                        "other migration exclusion requires a note".into(),
                    ));
                }
                if exclusion
                    .note
                    .as_ref()
                    .is_some_and(|note| note.len() > 2_000)
                {
                    return Err(DomainError::InvalidMigration(
                        "migration exclusion note exceeds 2000 bytes".into(),
                    ));
                }
            }
        }
        self.migration_dispositions
            .get_mut(task_id)
            .expect("validated above")
            .insert(object_group_id.clone(), disposition.clone());
        self.migration_confirmations.remove(task_id);
        Ok(())
    }

    fn apply_dependency_marker(
        &mut self,
        task_id: &TaskId,
        object_group_id: &ObjectGroupId,
        marker: &MigrationDependencyMarker,
    ) -> DomainResult<()> {
        self.migration_target(task_id, object_group_id)?;
        let markers = self
            .migration_dependencies
            .entry(task_id.clone())
            .or_default();
        let expected = markers
            .get(object_group_id)
            .map_or(1, |existing| existing.marker_version + 1);
        if marker.marker_version != expected {
            return Err(DomainError::InvalidMigration(
                "migration dependency marker version is invalid".into(),
            ));
        }
        let disposition = &self.migration_dispositions[task_id][object_group_id];
        if marker.required_disposition_version != disposition.disposition_version {
            return Err(DomainError::InvalidMigration(
                "migration dependency does not bind the current disposition".into(),
            ));
        }
        markers.insert(object_group_id.clone(), marker.clone());
        self.migration_confirmations.remove(task_id);
        Ok(())
    }

    fn apply_migration_pass_started(&mut self, pass: &MigrationPass) -> DomainResult<()> {
        if self.migration_passes.contains_key(&pass.pass_id) {
            return Err(DomainError::InvalidMigration(format!(
                "migration pass {} already exists",
                pass.pass_id
            )));
        }
        let set = self
            .migration_target_sets
            .get(&pass.task_id)
            .ok_or_else(|| {
                DomainError::InvalidMigration("migration target set is missing".into())
            })?;
        if pass.expected_target_set_hash != set.target_set_hash
            || pass.starting_state_hash != self.current_migration_state_hash(&pass.task_id)?
            || !pass.items.is_empty()
        {
            return Err(DomainError::InvalidMigration(
                "migration pass start hashes or items are invalid".into(),
            ));
        }
        self.migration_passes
            .insert(pass.pass_id.clone(), pass.clone());
        Ok(())
    }

    fn apply_migration_pass_item(
        &mut self,
        pass_id: &MigrationPassId,
        item: &crate::MigrationPassItem,
    ) -> DomainResult<()> {
        let pass = self.migration_passes.get(pass_id).ok_or_else(|| {
            DomainError::InvalidMigration(format!("migration pass {pass_id} is missing"))
        })?;
        let target = self.migration_target(&pass.task_id, &item.object_group_id)?;
        let guide = self
            .current_annotation(&target.guide_annotation_id)
            .ok_or_else(|| DomainError::InvalidMigration("migration guide is missing".into()))?;
        let disposition = &self.migration_dispositions[&pass.task_id][&item.object_group_id];
        if guide.version != item.guide_annotation_version
            || guide.deleted != item.guide_deleted
            || disposition.disposition_version != item.disposition_version
            || self
                .migration_dependencies
                .get(&pass.task_id)
                .is_some_and(|markers| markers.contains_key(&item.object_group_id))
            || pass.items.iter().any(|existing| {
                existing.matches_target_state(
                    &item.object_group_id,
                    item.guide_annotation_version,
                    item.guide_deleted,
                    item.disposition_version,
                )
            })
        {
            return Err(DomainError::InvalidMigration(
                "migration pass item targets stale or duplicate state".into(),
            ));
        }
        let action_matches = matches!(
            (&item.action, &disposition.status),
            (
                crate::MigrationPassItemAction::Kept | crate::MigrationPassItemAction::Annotated,
                MigrationDispositionStatus::Annotated { .. }
            ) | (
                crate::MigrationPassItemAction::Kept | crate::MigrationPassItemAction::Excluded,
                MigrationDispositionStatus::Excluded { .. }
            )
        );
        let deleted_guide_exclusion = guide.deleted
            && item.action == crate::MigrationPassItemAction::Excluded
            && matches!(
                disposition.status,
                MigrationDispositionStatus::Excluded { .. }
            );
        if !action_matches || (guide.deleted && !deleted_guide_exclusion) {
            return Err(DomainError::InvalidMigration(
                "migration pass action does not match the current resolution".into(),
            ));
        }
        self.migration_passes
            .get_mut(pass_id)
            .expect("validated above")
            .items
            .push(item.clone());
        Ok(())
    }

    fn apply_migration_confirmation(
        &mut self,
        confirmation: &MigrationConfirmation,
        event: &EventLogEntry,
    ) -> DomainResult<()> {
        let set = self
            .migration_target_sets
            .get(&confirmation.task_id)
            .ok_or_else(|| {
                DomainError::InvalidMigration("migration target set is missing".into())
            })?;
        let state_hash = self.current_migration_state_hash(&confirmation.task_id)?;
        let expected = migration_confirmation_hash(&set.target_set_hash, &state_hash)?;
        let all_resolved = self
            .validate_migration_resolution(&confirmation.task_id)
            .is_ok();
        let has_dependencies = self
            .migration_dependencies
            .get(&confirmation.task_id)
            .is_some_and(|markers| !markers.is_empty());
        if confirmation.target_set_hash != set.target_set_hash
            || confirmation.state_hash != state_hash
            || confirmation.confirmation_hash != expected
            || confirmation.actor_user_id != event.actor_user_id
            || confirmation.timestamp != event.timestamp
            || !all_resolved
            || has_dependencies
        {
            return Err(DomainError::InvalidMigration(
                "migration confirmation is stale or unresolved".into(),
            ));
        }
        self.migration_confirmations
            .insert(confirmation.task_id.clone(), confirmation.clone());
        Ok(())
    }

    fn validate_migration_terminal(&self, task_id: &TaskId) -> DomainResult<()> {
        self.validate_migration_resolution(task_id)?;
        let set = &self.migration_target_sets[task_id];
        let state_hash = self.current_migration_state_hash(task_id)?;
        let confirmation_hash = migration_confirmation_hash(&set.target_set_hash, &state_hash)?;
        let confirmation = self.migration_confirmations.get(task_id).ok_or_else(|| {
            DomainError::InvalidMigration(
                "migration submission requires a fresh canonical confirmation".into(),
            )
        })?;
        if confirmation.target_set_hash != set.target_set_hash
            || confirmation.state_hash != state_hash
            || confirmation.confirmation_hash != confirmation_hash
        {
            return Err(DomainError::InvalidMigration(
                "migration submission confirmation is stale".into(),
            ));
        }
        Ok(())
    }

    fn validate_migration_resolution(&self, task_id: &TaskId) -> DomainResult<()> {
        let set = self.migration_target_sets.get(task_id).ok_or_else(|| {
            DomainError::InvalidMigration("migration target set is missing".into())
        })?;
        let dispositions = &self.migration_dispositions[task_id];
        for target in &set.targets {
            let guide = self
                .current_annotation(&target.guide_annotation_id)
                .ok_or_else(|| {
                    DomainError::InvalidMigration("migration guide is missing".into())
                })?;
            if self
                .migration_dependencies
                .get(task_id)
                .is_some_and(|markers| markers.contains_key(&target.object_group_id))
            {
                return Err(DomainError::InvalidMigration(
                    "migration guide is unavailable or invalidated".into(),
                ));
            }
            match &dispositions[&target.object_group_id].status {
                MigrationDispositionStatus::Pending => {
                    return Err(DomainError::InvalidMigration(
                        "migration target is pending".into(),
                    ));
                }
                MigrationDispositionStatus::Annotated {
                    skeleton_annotation_id,
                    skeleton_version,
                } => {
                    if guide.deleted {
                        return Err(DomainError::InvalidMigration(
                            "migration guide is unavailable or invalidated".into(),
                        ));
                    }
                    let skeleton =
                        self.current_annotation(skeleton_annotation_id)
                            .ok_or_else(|| {
                                DomainError::InvalidMigration(
                                    "migration skeleton is missing".into(),
                                )
                            })?;
                    if skeleton.annotation_id != target.reserved_skeleton_annotation_id
                        || skeleton.version != *skeleton_version
                        || skeleton.deleted
                        || skeleton.task_id != *task_id
                        || skeleton.object_group_id.as_ref() != Some(&target.object_group_id)
                    {
                        return Err(DomainError::InvalidMigration(
                            "migration annotated disposition is stale".into(),
                        ));
                    }
                }
                MigrationDispositionStatus::Excluded { .. } => {}
            }
        }
        for annotation in self
            .active_annotations()
            .filter(|annotation| annotation.task_id == *task_id)
        {
            let expected = set.targets.iter().any(|target| {
                target.reserved_skeleton_annotation_id == annotation.annotation_id
                    && annotation.object_group_id.as_ref() == Some(&target.object_group_id)
                    && matches!(
                        dispositions[&target.object_group_id].status,
                        MigrationDispositionStatus::Annotated { .. }
                    )
            });
            if !expected {
                return Err(DomainError::InvalidMigration(format!(
                    "task {task_id} contains an unexpected migration skeleton {}",
                    annotation.annotation_id
                )));
            }
        }
        Ok(())
    }

    fn migration_target(
        &self,
        task_id: &TaskId,
        object_group_id: &ObjectGroupId,
    ) -> DomainResult<&crate::MigrationTarget> {
        self.migration_target_sets
            .get(task_id)
            .and_then(|set| {
                set.targets
                    .iter()
                    .find(|target| &target.object_group_id == object_group_id)
            })
            .ok_or_else(|| {
                DomainError::InvalidMigration(format!(
                    "migration target {object_group_id} is not expected for task {task_id}"
                ))
            })
    }

    pub fn current_migration_state_hash(
        &self,
        task_id: &TaskId,
    ) -> DomainResult<crate::MigrationHash> {
        let set = self.migration_target_sets.get(task_id).ok_or_else(|| {
            DomainError::InvalidMigration("migration target set is missing".into())
        })?;
        let empty_markers = BTreeMap::new();
        let markers = self
            .migration_dependencies
            .get(task_id)
            .unwrap_or(&empty_markers);
        let dispositions = self.migration_dispositions.get(task_id).ok_or_else(|| {
            DomainError::InvalidMigration("migration dispositions are missing".into())
        })?;
        let mut values = Vec::with_capacity(set.targets.len());
        for target in &set.targets {
            let guide = self
                .current_annotation(&target.guide_annotation_id)
                .ok_or_else(|| {
                    DomainError::InvalidMigration("migration guide is missing".into())
                })?;
            values.push(MigrationHashStateTarget {
                target,
                guide_annotation_version: guide.version,
                guide_deleted: guide.deleted,
                dependency_marker: markers.get(&target.object_group_id),
                disposition: &dispositions[&target.object_group_id],
            });
        }
        migration_state_hash(&set.target_set_hash, &values)
    }

    fn mark_changed_guide(&mut self, annotation: &AnnotationVersion, event: &EventLogEntry) {
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

    fn invalidate_migration_target_annotation(&mut self, annotation_id: &AnnotationId) {
        let affected = self
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
        for task_id in affected {
            self.migration_confirmations.remove(&task_id);
        }
    }

    fn apply_annotation_deletion(&mut self, annotation_id: &AnnotationId, event: &EventLogEntry) {
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

    fn apply_reviewer_correction(
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

pub fn rebuild_state(image_id: ImageId, events: &[EventLogEntry]) -> DomainResult<ImageState> {
    let mut state = ImageState::new(image_id);
    for event in events {
        state.apply_event(event)?;
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use crate::{
        AnnotationGeometry, AnnotationOrigin, AnnotationType, BoundingBox, DatasetRole,
        HumanRevisionKind, RevisionSource, UserId, now,
    };

    use super::*;

    #[test]
    fn replays_annotation_versions_at_event_boundaries() {
        let image_id = ImageId::from("img_test");
        let user_id = UserId::from("user_1");
        let annotation_id = AnnotationId::from("ann_1");
        let first = AnnotationVersion {
            annotation_id: annotation_id.clone(),
            version: 1,
            object_group_id: None,
            origin: AnnotationOrigin::native(),
            task_id: TaskId::from("bounding_box:person"),
            class_id: crate::ClassId::from("person"),
            annotation_type: AnnotationType::BoundingBox,
            revision_source: RevisionSource::Human {
                action: HumanRevisionKind::Authored,
            },
            geometry: AnnotationGeometry::BoundingBox(BoundingBox {
                x: 0.1,
                y: 0.1,
                width: 0.2,
                height: 0.2,
            }),
            author_user_id: user_id.clone(),
            created_at: now(),
            updated_at: now(),
            deleted: false,
        };
        let mut second = first.clone();
        second.version = 2;
        second.geometry = AnnotationGeometry::BoundingBox(BoundingBox {
            x: 0.2,
            y: 0.1,
            width: 0.2,
            height: 0.2,
        });
        let events = vec![
            EventLogEntry::new(
                1,
                image_id.clone(),
                user_id.clone(),
                DatasetRole::Annotator,
                now(),
                EventPayload::AnnotationVersionCreated {
                    annotation: first,
                    previous_version: None,
                    reason: None,
                },
            ),
            EventLogEntry::new(
                2,
                image_id.clone(),
                user_id,
                DatasetRole::Annotator,
                now(),
                EventPayload::AnnotationVersionCreated {
                    annotation: second,
                    previous_version: Some(1),
                    reason: Some("move".to_string()),
                },
            ),
        ];
        let state_after_first = rebuild_state(image_id.clone(), &events[..1]).unwrap();
        assert_eq!(
            state_after_first
                .current_annotation(&annotation_id)
                .unwrap()
                .version,
            1
        );
        let mut state_after_second = rebuild_state(image_id, &events).unwrap();
        assert_eq!(
            state_after_second
                .current_annotation(&annotation_id)
                .unwrap()
                .version,
            2
        );

        let stale_delete = EventLogEntry::new(
            3,
            state_after_second.image_id.clone(),
            UserId::from("user_1"),
            DatasetRole::Annotator,
            now(),
            EventPayload::AnnotationDeleted {
                annotation_id,
                version: 1,
                reason: None,
            },
        );
        assert!(state_after_second.apply_event(&stale_delete).is_err());
    }

    #[test]
    fn replays_reviewer_correction_as_one_terminal_rejection() {
        let image_id = ImageId::from("img_correction");
        let task_id = TaskId::from("bounding_box:person");
        let annotation_id = AnnotationId::from("ann_1");
        let annotator = UserId::from("annotator");
        let reviewer = UserId::from("reviewer");
        let correction_id = crate::CorrectionId::from("cor_1");
        let assignment_id = crate::AssignmentId::from("asg_1");
        let timestamp = now();
        let first = AnnotationVersion {
            annotation_id: annotation_id.clone(),
            version: 1,
            object_group_id: None,
            origin: AnnotationOrigin::native(),
            task_id: task_id.clone(),
            class_id: crate::ClassId::from("person"),
            annotation_type: AnnotationType::BoundingBox,
            revision_source: RevisionSource::Human {
                action: HumanRevisionKind::Authored,
            },
            geometry: AnnotationGeometry::BoundingBox(BoundingBox {
                x: 0.1,
                y: 0.1,
                width: 0.2,
                height: 0.2,
            }),
            author_user_id: annotator.clone(),
            created_at: timestamp,
            updated_at: timestamp,
            deleted: false,
        };
        let corrected = AnnotationVersion {
            version: 2,
            revision_source: RevisionSource::ReviewerCorrection {
                correction_id: correction_id.clone(),
            },
            geometry: AnnotationGeometry::BoundingBox(BoundingBox {
                x: 0.2,
                y: 0.2,
                width: 0.3,
                height: 0.3,
            }),
            author_user_id: reviewer.clone(),
            updated_at: timestamp,
            ..first.clone()
        };
        let assignment = Assignment {
            assignment_id: assignment_id.clone(),
            image_id: image_id.clone(),
            task_id: task_id.clone(),
            assigned_to: reviewer.clone(),
            kind: AssignmentKind::Review,
            status: AssignmentStatus::Completed,
            expires_at: Some(timestamp + std::time::Duration::from_secs(60)),
            created_at: timestamp,
            updated_at: timestamp,
        };
        let correction = ReviewerCorrectionRecord {
            correction_id: correction_id.clone(),
            assignment_id,
            annotation_id: annotation_id.clone(),
            previous_version: 1,
            corrected_version: 2,
            task_id: task_id.clone(),
            reviewer_user_id: reviewer.clone(),
            timestamp,
            reason: Some("box was too small".to_string()),
        };
        let events = vec![
            EventLogEntry::new(
                1,
                image_id.clone(),
                annotator,
                DatasetRole::Annotator,
                timestamp,
                EventPayload::AnnotationVersionCreated {
                    annotation: first,
                    previous_version: None,
                    reason: None,
                },
            ),
            EventLogEntry::new(
                2,
                image_id.clone(),
                reviewer.clone(),
                DatasetRole::Reviewer,
                timestamp,
                EventPayload::ReviewerCorrectionRecorded {
                    correction,
                    annotation: Box::new(corrected),
                    review: ReviewRecord {
                        review_id: crate::ReviewId::from("rev_1"),
                        target: ReviewTarget::AnnotationVersion {
                            annotation_id: annotation_id.clone(),
                            version: 1,
                        },
                        reviewer_user_id: reviewer.clone(),
                        decision: ReviewDecision::Rejected,
                        timestamp,
                        comment: Some("box was too small".to_string()),
                    },
                    task_state: TaskState {
                        task_id: task_id.clone(),
                        status: TaskStatus::Completed,
                        outcome: Some(TaskOutcome::ReviewerCorrected),
                        assigned_to: None,
                        completed_by: Some(reviewer),
                        completed_at: Some(timestamp),
                        updated_at: timestamp,
                    },
                    assignments: vec![assignment],
                },
            ),
        ];

        let state = rebuild_state(image_id, &events).unwrap();

        assert_eq!(state.current_annotation(&annotation_id).unwrap().version, 2);
        assert_eq!(state.reviews[0].decision, ReviewDecision::Rejected);
        assert_eq!(state.reviewer_corrections.len(), 1);
        assert_eq!(
            state.task_states[&task_id].outcome,
            Some(TaskOutcome::ReviewerCorrected)
        );
        assert_eq!(state.assignments[0].status, AssignmentStatus::Completed);
    }
}
