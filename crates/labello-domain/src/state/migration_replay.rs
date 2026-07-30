use super::*;

impl ImageState {
    pub(super) fn apply_migration_disposition(
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
                        .and_then(|markers| markers.get(object_group_id))
                        .is_some_and(|marker| {
                            marker.kind != crate::MigrationDependencyKind::ManualSelection
                        })
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

    pub(super) fn apply_dependency_marker(
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

    pub(super) fn apply_migration_pass_started(
        &mut self,
        pass: &MigrationPass,
    ) -> DomainResult<()> {
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

    pub(super) fn apply_migration_pass_item(
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

    pub(super) fn apply_migration_confirmation(
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

    pub(super) fn validate_migration_terminal(&self, task_id: &TaskId) -> DomainResult<()> {
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

    pub(super) fn validate_migration_resolution(&self, task_id: &TaskId) -> DomainResult<()> {
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
            if !expected && !is_discovered_migration_skeleton(annotation) {
                return Err(DomainError::InvalidMigration(format!(
                    "task {task_id} contains an unexpected migration skeleton {}",
                    annotation.annotation_id
                )));
            }
        }
        Ok(())
    }

    pub(super) fn migration_target(
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
        let discovered = self
            .active_annotations()
            .filter(|annotation| {
                annotation.task_id == *task_id && is_discovered_migration_skeleton(annotation)
            })
            .collect::<Vec<_>>();
        migration_state_hash_with_discovered(&set.target_set_hash, &values, &discovered)
    }
}

fn is_discovered_migration_skeleton(annotation: &AnnotationVersion) -> bool {
    annotation.object_group_id.is_none()
        && annotation.annotation_type == AnnotationType::Skeleton
        && matches!(
            annotation.origin,
            AnnotationOrigin::Native { legacy_v2: false }
        )
        && matches!(
            annotation.revision_source,
            RevisionSource::Human {
                action: HumanRevisionKind::Authored
                    | HumanRevisionKind::Edited
                    | HumanRevisionKind::AcceptedUnchanged
            }
        )
}
