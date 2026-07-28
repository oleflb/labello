impl crate::app::LabelloApp {
    pub(crate) fn initialize_browser_workspace(&mut self) {
        self.runtime.storage_error = None;
        let identity =
            match StorageIdentity::new(&self.config.api_base_url, self.config.user_id.clone()) {
                Ok(identity) => identity,
                Err(error) => {
                    self.runtime.error = Some(error);
                    return;
                }
            };
        let preference = match load_workspace_preference(&identity) {
            Ok(preference) => preference,
            Err(error) => {
                self.storage_failure(error);
                None
            }
        };
        self.runtime.persistence.identity = Some(identity);
        self.runtime.persistence.preference_encoded = preference
            .as_ref()
            .and_then(|preference| serde_json::to_string(preference).ok());
        self.runtime.persistence.preference_desired_encoded =
            self.runtime.persistence.preference_encoded.clone();
        self.runtime.persistence.preference_retry.reset();
        self.runtime.persistence.preference = preference;
        self.runtime.persistence.restoration_attempted = false;
        self.runtime.persistence.expected_assignment = None;
        self.runtime.persistence.recovery = None;
        self.runtime.persistence.last_work_draft = None;
        self.runtime.persistence.desired_work_draft = None;
        self.runtime.persistence.work_ready = None;
        self.runtime.persistence.last_admin_config = None;
        self.runtime.persistence.desired_admin_config = None;
        self.runtime.persistence.admin_delete_desired = false;
        self.runtime.persistence.commands.clear();
        self.queue_persistence(PersistenceCommand::GarbageCollect(labello_domain::now()));
    }

    pub(crate) fn isolate_browser_workspace(&mut self) {
        self.runtime.persistence.identity = None;
        self.runtime.persistence.preference = None;
        self.runtime.persistence.preference_encoded = None;
        self.runtime.persistence.preference_desired_encoded = None;
        self.runtime.persistence.preference_retry.reset();
        self.runtime.persistence.restoration_attempted = false;
        self.runtime.persistence.expected_assignment = None;
        self.runtime.persistence.recovery = None;
        self.runtime.persistence.last_work_draft = None;
        self.runtime.persistence.desired_work_draft = None;
        self.runtime.persistence.work_ready = None;
        self.runtime.persistence.last_admin_config = None;
        self.runtime.persistence.desired_admin_config = None;
        self.runtime.persistence.admin_delete_desired = false;
        self.runtime.persistence.commands.clear();
    }

    pub(crate) fn reopen_previous_workspace(&mut self) {
        if self.runtime.persistence.restoration_attempted {
            return;
        }
        self.runtime.persistence.restoration_attempted = true;
        if self.loading.dataset || self.datasets.requested_view.is_some() {
            return;
        }
        let Some(preference) = self.runtime.persistence.preference.clone() else {
            return;
        };
        let Some(summary) = self
            .datasets
            .summaries
            .iter()
            .find(|summary| summary.dataset_id == preference.dataset_id)
        else {
            self.runtime.notice =
                Some("The previous dataset is no longer available to this account.".to_string());
            return;
        };
        let view = app_view(preference.view);
        let authorized = match view {
            crate::app::AppView::Annotate => summary
                .roles
                .contains(&labello_domain::DatasetRole::Annotator),
            crate::app::AppView::Review => summary
                .roles
                .contains(&labello_domain::DatasetRole::Reviewer),
            crate::app::AppView::Adjudicate => summary
                .roles
                .contains(&labello_domain::DatasetRole::Adjudicator),
            crate::app::AppView::Admin => summary
                .roles
                .contains(&labello_domain::DatasetRole::DataAdmin),
            crate::app::AppView::Stats => !summary.roles.is_empty(),
            crate::app::AppView::Setup => false,
        };
        if !authorized {
            self.runtime.notice = Some(
                "The previous view is no longer authorized; choose an available dataset view."
                    .to_string(),
            );
            return;
        }
        self.runtime.persistence.expected_assignment = preference.assignment_id.clone();
        self.open_dataset(preference.dataset_id, view);
    }

    pub(crate) fn persist_workspace_preference(&mut self) {
        if self.auth.account.is_none()
            || self.datasets.metadata.is_none()
            || self.view == crate::app::AppView::Setup
            || !self.can_open_view(self.view)
        {
            return;
        }
        let Some(identity) = self.runtime.persistence.identity.clone() else {
            return;
        };
        let preference = WorkspacePreference {
            version: PREFERENCE_VERSION,
            dataset_id: self.config.dataset_id.clone(),
            view: stored_view(self.view),
            task_id: self.work.selected_task_id.clone(),
            assignment_id: self
                .work
                .assignment
                .as_ref()
                .map(|assignment| assignment.assignment_id.clone()),
            assignment_image_id: self
                .work
                .assignment
                .as_ref()
                .map(|assignment| assignment.image_id.clone()),
            assignment_kind: self
                .work
                .assignment
                .as_ref()
                .map(|assignment| assignment.kind.clone()),
            drawer: self.work.drawer.map(|drawer| match drawer {
                crate::app::Drawer::Workflow => "workflow".to_string(),
                crate::app::Drawer::Inspector => "inspector".to_string(),
            }),
            workflow_panel_collapsed: self.work.workflow_panel_collapsed,
            show_settings: self.work.show_settings,
            show_tutorial: self.work.show_tutorial,
            selected_annotation: self.work.selected_annotation.clone(),
            canvas: self.work.canvas.stored_transform(),
            availability: self.stored_assignment_availability(),
        };
        let encoded = match serde_json::to_string(&preference) {
            Ok(encoded) => encoded,
            Err(error) => {
                self.runtime.error = Some(format!(
                    "could not encode browser workspace preference: {error}"
                ));
                return;
            }
        };
        if self.runtime.persistence.preference_encoded.as_deref() == Some(&encoded) {
            return;
        }
        if self
            .runtime
            .persistence
            .preference_desired_encoded
            .as_deref()
            != Some(&encoded)
        {
            self.runtime.persistence.preference_desired_encoded = Some(encoded.clone());
            self.runtime.persistence.preference_retry.reset();
        }
        let now = Instant::now();
        if let Some(remaining) = self.runtime.persistence.preference_retry.remaining(now) {
            if let Some(ctx) = self.runtime.repaint_ctx.as_ref() {
                ctx.request_repaint_after(remaining);
            }
            return;
        }
        match save_workspace_preference(&identity, &preference) {
            Ok(()) => {
                self.runtime.persistence.preference = Some(preference);
                self.runtime.persistence.preference_encoded = Some(encoded);
                self.runtime.persistence.preference_retry.reset();
            }
            Err(error) => {
                self.storage_failure(error);
                let delay = self
                    .runtime
                    .persistence
                    .preference_retry
                    .failed(Instant::now());
                if let Some(ctx) = self.runtime.repaint_ctx.as_ref() {
                    ctx.request_repaint_after(delay);
                }
            }
        }
    }

    fn stored_assignment_availability(&self) -> Option<StoredAssignmentAvailability> {
        let checked_at = self.work.availability.checked_at?;
        let kind = self.assignment_kind()?;
        (self.work.availability.resolved
            && self.work.availability.error.is_none()
            && self.work.availability.dataset_id.as_ref() == Some(&self.config.dataset_id)
            && self.work.availability.kind.as_ref() == Some(&kind))
        .then(|| StoredAssignmentAvailability {
            kind,
            tasks: self.work.availability.tasks.clone(),
            checked_at,
        })
    }

    pub(crate) fn restore_cached_assignment_availability(&mut self) -> bool {
        let Some(kind) = self.assignment_kind() else {
            return false;
        };
        let Some(preference) = self.runtime.persistence.preference.clone() else {
            return false;
        };
        let Some(cached) = preference.availability else {
            return false;
        };
        if preference.dataset_id != self.config.dataset_id || cached.kind != kind {
            return false;
        }
        let expected_tasks = self
            .work
            .tasks
            .iter()
            .map(|task| task.task_id.clone())
            .collect::<BTreeSet<_>>();
        if cached.tasks.keys().cloned().collect::<BTreeSet<_>>() != expected_tasks {
            return false;
        }
        let Ok(age) = labello_domain::now()
            .signed_duration_since(cached.checked_at)
            .to_std()
        else {
            return false;
        };
        if age >= crate::app::ASSIGNMENT_AVAILABILITY_CACHE_TTL {
            return false;
        }
        self.work.availability.dataset_id = Some(self.config.dataset_id.clone());
        self.work.availability.kind = Some(kind);
        self.work.availability.tasks = cached.tasks;
        self.work.availability.resolved = true;
        self.work.availability.checked_at = Some(cached.checked_at);
        self.work.availability.loading = false;
        self.work.availability.load_after_resolution = false;
        self.work.availability.refresh_after_load = false;
        self.work.availability.error = None;
        self.work.availability.last_attempt = Some(Instant::now());
        true
    }

    pub(crate) fn apply_assignment_preferences(&mut self) {
        let Some(assignment) = self.work.assignment.as_ref() else {
            return;
        };
        let Some(preference) = self
            .runtime
            .persistence
            .preference
            .as_ref()
            .filter(|preference| {
                preference.dataset_id == self.config.dataset_id
                    && preference.assignment_id.as_ref() == Some(&assignment.assignment_id)
                    && preference.assignment_kind.as_ref() == Some(&assignment.kind)
            })
            .cloned()
        else {
            return;
        };
        self.work.selected_annotation = preference.selected_annotation.filter(|annotation_id| {
            self.work
                .annotations
                .iter()
                .any(|annotation| &annotation.annotation_id == annotation_id && !annotation.deleted)
        });
        self.work.canvas.restore_transform(preference.canvas);
        self.work.drawer = match preference.drawer.as_deref() {
            Some("workflow") => Some(crate::app::Drawer::Workflow),
            Some("inspector") => Some(crate::app::Drawer::Inspector),
            _ => None,
        };
        self.work.workflow_panel_collapsed = preference.workflow_panel_collapsed;
        self.work.show_settings = preference.show_settings;
        self.work.show_tutorial = preference.show_tutorial;
    }

    pub(crate) fn request_work_draft_load(&mut self) {
        let (Some(identity), Some(assignment)) = (
            self.runtime.persistence.identity.as_ref(),
            self.work.assignment.as_ref(),
        ) else {
            return;
        };
        let key = work_draft_key(identity, &self.config.dataset_id, assignment);
        self.runtime.persistence.work_ready = None;
        self.queue_persistence(PersistenceCommand::Load(key));
    }

    pub(crate) fn request_previous_draft_status(&mut self) {
        let (Some(identity), Some(preference)) = (
            self.runtime.persistence.identity.as_ref(),
            self.runtime.persistence.preference.as_ref(),
        ) else {
            return;
        };
        let (Some(assignment_id), Some(image_id), Some(task_id), Some(kind)) = (
            preference.assignment_id.as_ref(),
            preference.assignment_image_id.as_ref(),
            preference.task_id.as_ref(),
            preference.assignment_kind.as_ref(),
        ) else {
            return;
        };
        let key = work_draft_key_parts(
            identity,
            &preference.dataset_id,
            assignment_id,
            image_id,
            task_id,
            kind,
        );
        self.queue_persistence(PersistenceCommand::Load(key));
    }

    pub(crate) fn request_admin_draft_load(&mut self) {
        let Some(identity) = self.runtime.persistence.identity.as_ref() else {
            return;
        };
        let key = admin_draft_key(identity, &self.config.dataset_id);
        self.queue_persistence(PersistenceCommand::Load(key));
    }

    pub(crate) fn queue_current_drafts(&mut self) {
        let (Some(identity), Some(assignment), Some(state)) = (
            self.runtime.persistence.identity.clone(),
            self.work.assignment.clone(),
            self.work.current_state.as_ref(),
        ) else {
            self.queue_admin_draft();
            return;
        };
        if assignment
            .expires_at
            .is_some_and(|expires_at| expires_at <= labello_domain::now())
        {
            if self.runtime.persistence.work_ready.take().is_some() {
                self.clear_current_work_draft(&assignment);
                self.runtime.notice = Some(
                    "The local assignment lease expired; its browser draft was discarded without changing server state."
                        .to_string(),
                );
            }
            return;
        }
        if self.runtime.persistence.work_ready.as_ref() != Some(&assignment.assignment_id)
            || matches!(
                self.runtime.persistence.recovery,
                Some(DraftRecovery::Work(_, _))
            )
        {
            self.queue_admin_draft();
            return;
        }
        if self.work.canvas.is_dragging() {
            self.queue_admin_draft();
            return;
        }
        let payload = match self.view {
            crate::app::AppView::Annotate
                if matches!(
                    self.work.save_status,
                    crate::app::SaveStatus::Dirty
                        | crate::app::SaveStatus::Saving
                        | crate::app::SaveStatus::Retry
                ) =>
            {
                WorkDraftPayload::Annotation(AnnotationDraft {
                    annotations: self.work.annotations.clone(),
                    accepted_prelabels: self.work.accepted_prelabels.clone(),
                    selected_annotation: self.work.selected_annotation.clone(),
                    active_skeleton: self.work.active_skeleton.clone(),
                    skeleton_keypoint_index: self.work.skeleton_keypoint_index,
                    next_keypoint_hidden: self.work.next_keypoint_hidden,
                })
            }
            crate::app::AppView::Review => WorkDraftPayload::Review(ReviewDraft {
                target_annotation: self.work.selected_annotation.clone(),
                correction: self
                    .work
                    .correction_draft
                    .as_ref()
                    .map(StoredCorrectionDraft::from),
            }),
            _ => {
                self.queue_admin_draft();
                return;
            }
        };
        let draft = WorkDraft::new(
            &identity,
            self.config.dataset_id.clone(),
            &assignment,
            state.current_sequence,
            self.work.edit_generation,
            payload,
        );
        let already_persisted = self
            .runtime
            .persistence
            .last_work_draft
            .as_ref()
            .is_some_and(|saved| same_work_draft(saved, &draft));
        let already_desired = self
            .runtime
            .persistence
            .desired_work_draft
            .as_ref()
            .is_some_and(|desired| same_work_draft(desired, &draft));
        if !already_persisted && !already_desired {
            self.runtime.persistence.desired_work_draft = Some(draft.clone());
            self.queue_persistence(PersistenceCommand::Save(Box::new(DraftRecord::Work(
                Box::new(draft),
            ))));
        }
        self.queue_admin_draft();
    }

    fn queue_admin_draft(&mut self) {
        if self.view != crate::app::AppView::Admin || self.loading.admin {
            return;
        }
        let (Some(identity), Some(baseline), Some(config)) = (
            self.runtime.persistence.identity.clone(),
            self.datasets.admin_baseline.as_ref(),
            self.datasets.admin_config.as_ref(),
        ) else {
            return;
        };
        if config == baseline {
            if !self.runtime.persistence.admin_delete_desired
                && (self.runtime.persistence.last_admin_config.is_some()
                    || self.runtime.persistence.desired_admin_config.is_some())
            {
                self.clear_admin_draft();
            }
            return;
        }
        let draft = AdminDraft::new(&identity, self.config.dataset_id.clone(), baseline, config);
        if self.runtime.persistence.last_admin_config.as_ref() == Some(&draft.config)
            || self.runtime.persistence.desired_admin_config.as_ref() == Some(&draft.config)
        {
            return;
        }
        self.runtime.persistence.admin_delete_desired = false;
        self.runtime.persistence.desired_admin_config = Some(draft.config.clone());
        self.queue_persistence(PersistenceCommand::Save(Box::new(DraftRecord::Admin(
            Box::new(draft),
        ))));
    }

    pub(crate) fn clear_current_work_draft(&mut self, assignment: &Assignment) {
        let Some(identity) = self.runtime.persistence.identity.as_ref() else {
            return;
        };
        let key = work_draft_key(identity, &self.config.dataset_id, assignment);
        self.runtime.persistence.desired_work_draft = None;
        self.queue_persistence(PersistenceCommand::Delete(key));
    }

    pub(crate) fn clear_admin_draft(&mut self) {
        let Some(identity) = self.runtime.persistence.identity.as_ref() else {
            return;
        };
        let key = admin_draft_key(identity, &self.config.dataset_id);
        self.runtime.persistence.desired_admin_config = None;
        self.runtime.persistence.admin_delete_desired = true;
        self.queue_persistence(PersistenceCommand::Delete(key));
    }

    pub(crate) fn recover_browser_draft(&mut self) {
        let Some(recovery) = self.runtime.persistence.recovery.clone() else {
            return;
        };
        match recovery {
            DraftRecovery::Work(draft, DraftValidation::Valid) => {
                match draft.payload {
                    WorkDraftPayload::Annotation(draft) => {
                        self.work.annotations = draft.annotations;
                        self.work.accepted_prelabels = draft.accepted_prelabels;
                        self.work.selected_annotation = draft.selected_annotation;
                        self.work.active_skeleton = draft.active_skeleton;
                        self.work.skeleton_keypoint_index = draft.skeleton_keypoint_index;
                        self.work.next_keypoint_hidden = draft.next_keypoint_hidden;
                        self.recompute_modified_annotations();
                        self.work.edit_generation = self.work.edit_generation.wrapping_add(1);
                        self.work.save_status = crate::app::SaveStatus::Dirty;
                        self.work.last_edit_at = Some(Instant::now());
                    }
                    WorkDraftPayload::Review(draft) => {
                        if let Some(target) = draft.target_annotation
                            && self.work.annotations.iter().any(|annotation| {
                                annotation.annotation_id == target && !annotation.deleted
                            })
                        {
                            self.work.selected_annotation = Some(target);
                        }
                        self.work.correction_draft = draft.correction.map(Into::into);
                    }
                }
                self.runtime.notice = Some("Recovered the validated browser draft.".to_string());
                self.runtime.persistence.recovery = None;
            }
            DraftRecovery::Admin(draft, DraftValidation::Valid) => {
                let recovered = draft.config;
                let mut config = self
                    .datasets
                    .admin_baseline
                    .clone()
                    .unwrap_or_else(|| recovered.clone());
                self.runtime.persistence.last_admin_config = Some(recovered.clone());
                config.name = recovered.name;
                config.image_roots = recovered.image_roots;
                config.label_classes = recovered.label_classes;
                config.tasks = recovered.tasks;
                config.imbalance = recovered.imbalance;
                config.prelabel_configs = recovered.prelabel_configs;
                self.datasets.admin_config = Some(config.clone());
                self.runtime.notice = Some("Recovered the validated admin draft.".to_string());
                self.runtime.persistence.recovery = None;
            }
            _ => {}
        }
    }

    pub(crate) fn discard_browser_draft(&mut self) {
        let Some(recovery) = self.runtime.persistence.recovery.take() else {
            return;
        };
        let key = match recovery {
            DraftRecovery::Work(draft, _) => draft.key,
            DraftRecovery::Admin(draft, _) => draft.key,
        };
        self.queue_persistence(PersistenceCommand::Delete(key));
        self.runtime.notice = Some("Browser draft discarded.".to_string());
    }

    pub(crate) fn rebase_work_draft_after_save(&mut self, saved_generation: u64) {
        if self
            .runtime
            .persistence
            .last_work_draft
            .as_ref()
            .is_some_and(|draft| draft.edit_generation <= saved_generation)
        {
            self.runtime.persistence.last_work_draft = None;
        }
        if self
            .runtime
            .persistence
            .desired_work_draft
            .as_ref()
            .is_some_and(|draft| draft.edit_generation <= saved_generation)
        {
            self.runtime.persistence.desired_work_draft = None;
        }
        self.runtime.persistence.commands.retain(|queued| {
            !matches!(
                &queued.command,
                PersistenceCommand::Save(record)
                    if matches!(record.as_ref(), DraftRecord::Work(draft) if draft.edit_generation <= saved_generation)
            )
        });
    }

    pub(crate) fn reset_work_draft_tracking(&mut self) {
        self.runtime.persistence.last_work_draft = None;
        self.runtime.persistence.desired_work_draft = None;
    }
}
