use super::*;

impl ImageState {
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
        if let Some(target) = targets.iter().copied().find(|target| {
            matches!(
                self.migration_dispositions[task_id][&target.object_group_id].status,
                MigrationDispositionStatus::Pending
            ) && self
                .migration_dependencies
                .get(task_id)
                .and_then(|markers| markers.get(&target.object_group_id))
                .is_some_and(|marker| {
                    marker.kind == crate::MigrationDependencyKind::ManualSelection
                })
        }) {
            return Ok(crate::MigrationCursor::Object {
                object_group_id: target.object_group_id.clone(),
                sequence_index: target.sequence_index,
            });
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
}
