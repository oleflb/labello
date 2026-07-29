impl LabelloApp {
    fn apply_loaded_dataset(&mut self, loaded: LoadedDataset) {
        self.clear_previous_annotation_assignment();
        self.clear_current_image();
        self.sync_work_config(loaded.metadata);
        self.work.keybindings = loaded.keybindings;
        self.work.keybindings.normalize();
        self.work.shortcut_settings = Default::default();
        let requested = self.datasets.requested_view.take().unwrap_or_else(|| {
            [AppView::Annotate, AppView::Review]
                .into_iter()
                .find(|view| self.can_open_view(*view))
                .unwrap_or(AppView::Stats)
        });
        if !self.can_open_view(requested) {
            self.runtime.error = Some(if requested == AppView::Adjudicate {
                crate::app::ADJUDICATION_UNAVAILABLE_MESSAGE.to_string()
            } else {
                format!(
                    "The current user is not authorized for {}.",
                    view_label(requested)
                )
            });
            self.view = AppView::Setup;
            return;
        }
        if matches!(
            requested,
            AppView::Annotate | AppView::Review | AppView::Adjudicate
        ) && !self.ensure_valid_task_selection()
        {
            self.runtime.error = Some(
                "No enabled one-class workflow is configured. Ask a data admin to enable one."
                    .to_string(),
            );
        } else {
            self.runtime.error = None;
        }
        if requested == AppView::Admin {
            self.request_admin_dataset();
        } else {
            self.view = requested;
            if self.work_view() && self.selected_task().is_some() {
                self.restore_cached_assignment_availability();
                self.request_next_image();
            } else if self.view == AppView::Stats {
                self.request_stats();
            }
        }
    }

    fn handle_ingest_job(&mut self, result: Result<IngestJob, String>) {
        self.loading.ingest_polling = false;
        match result {
            Ok(job) => match job.status {
                IngestJobStatus::Running => {
                    self.loading.ingesting = true;
                    self.loading.ingest_job_id = Some(job.job_id);
                    self.loading.last_ingest_poll = Some(Instant::now());
                    self.runtime.notice = Some("Ingest running...".to_string());
                    self.runtime.error = None;
                }
                IngestJobStatus::Completed => {
                    self.loading.ingesting = false;
                    self.loading.ingest_job_id = None;
                    self.loading.last_ingest_poll = None;
                    let report = job.report.unwrap_or_default();
                    self.bump_dataset_image_count(report.new_images);
                    self.runtime.notice = Some(format!(
                        "Ingest complete: {} new, {} duplicate, {} changed, {} unreadable ({} discovered)",
                        report.new_images,
                        report.duplicate_files.len(),
                        report.changed_paths.len(),
                        report.unreadable_files.len(),
                        report.discovered_files,
                    ));
                    self.runtime.error = None;
                    self.request_dataset_list();
                    if self.view == AppView::Admin {
                        self.request_admin_dataset();
                    }
                    let load_after_resolution = self.work_view() && self.work.current.is_none();
                    self.assignment_availability_mutation_completed(
                        &self.config.dataset_id.clone(),
                        load_after_resolution,
                    );
                }
                IngestJobStatus::Failed => {
                    self.loading.ingesting = false;
                    self.loading.ingest_job_id = None;
                    self.loading.last_ingest_poll = None;
                    self.runtime.error =
                        Some(job.error.unwrap_or_else(|| "ingest failed".to_string()));
                    self.assignment_availability_mutation_completed(
                        &self.config.dataset_id.clone(),
                        false,
                    );
                }
            },
            Err(error) => {
                self.loading.ingesting = false;
                self.loading.ingest_job_id = None;
                self.loading.last_ingest_poll = None;
                self.runtime.error = Some(error);
                self.assignment_availability_mutation_completed(
                    &self.config.dataset_id.clone(),
                    false,
                );
            }
        }
    }

    fn upsert_dataset_summary(&mut self, metadata: &labello_domain::DatasetMetadata) {
        let metadata_roles = metadata
            .role_assignments
            .iter()
            .find(|assignment| assignment.user_id == self.config.user_id)
            .map(|assignment| assignment.roles.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let existing = self
            .datasets
            .summaries
            .iter_mut()
            .find(|summary| summary.dataset_id == metadata.dataset_id);
        match existing {
            Some(summary) => {
                summary.name = metadata.name.clone();
                if !metadata.images.is_empty() {
                    summary.total_images = metadata.images.len();
                }
            }
            None => self
                .datasets
                .summaries
                .push(labello_client::DatasetSummary {
                    dataset_id: metadata.dataset_id.clone(),
                    name: metadata.name.clone(),
                    roles: metadata_roles,
                    total_images: metadata.images.len(),
                }),
        }
    }

    fn bump_dataset_image_count(&mut self, new_images: usize) {
        if new_images == 0 {
            return;
        }
        if let Some(summary) = self
            .datasets
            .summaries
            .iter_mut()
            .find(|summary| summary.dataset_id == self.config.dataset_id)
        {
            summary.total_images += new_images;
        }
    }

    fn apply_loaded_image(&mut self, ctx: &egui::Context, loaded: LoadedImage) {
        let image_id = loaded.queued.image.image_id.clone();
        self.work.migration = Default::default();
        self.work.assignment = Some(loaded.assignment);
        self.work.current = Some(loaded.queued);
        self.work.current_state = Some(loaded.state.clone());
        self.work.annotations = loaded.annotations;
        self.work.persisted_annotations = self
            .work
            .annotations
            .iter()
            .map(|annotation| annotation.annotation_id.clone())
            .collect();
        self.work.modified_annotations.clear();
        self.work.accepted_prelabels.clear();
        self.work.selected_prelabel = None;
        self.work.selected_annotation = None;
        self.work.active_skeleton = None;
        self.work.skeleton_keypoint_index = 0;
        self.work.next_keypoint_hidden = false;
        self.work.review_index = 0;
        self.work.review_rejected = false;
        self.work.correction_draft = None;
        self.work.save_status = SaveStatus::Idle;
        self.work.edit_generation = 0;
        self.work.last_edit_at = None;
        self.work.undo_stack.clear();
        self.work.redo_stack.clear();
        self.work.canvas.fit_view();
        self.runtime.persistence.work_ready = None;
        self.work.current_texture = loaded.color_image.map(|image| {
            ctx.load_texture(
                format!("image-{image_id}"),
                image,
                egui::TextureOptions::LINEAR,
            )
        });
        if self.view == AppView::Review {
            self.work.review_index = self
                .work
                .selected_task_id
                .as_ref()
                .map(|task_id| {
                    crate::review_sequence::reviewed_object_prefix(
                        &loaded.state,
                        task_id,
                        &self.config.user_id,
                    )
                })
                .unwrap_or(0);
        }
        self.apply_assignment_preferences();
        self.sync_review_selection();
        if let Some(state) = self.work.current_state.clone() {
            self.renew_assignment_from_state(&state);
        }
        self.request_work_draft_load();
        self.request_prefetch();
    }

    fn finish_annotation_transition(
        &mut self,
        ctx: &egui::Context,
        released_image_id: Option<labello_domain::ImageId>,
    ) {
        let transition = self.work.pending_transition.take();
        if self.view == AppView::Annotate
            && transition == Some(crate::app::PendingTransition::NextAssignment)
        {
            self.open_next_annotation_assignment(ctx, released_image_id);
            return;
        }
        if let Some(crate::app::PendingTransition::PreviousAssignment(assignment)) = transition {
            self.work.previous_annotation_assignment = Some(assignment.clone());
            self.clear_current_image();
            self.request_reopen_assignment(assignment);
            return;
        }
        if let Some(transition) = transition {
            self.execute_transition(transition);
        } else {
            self.clear_current_image();
        }
    }

    fn open_next_annotation_assignment(
        &mut self,
        ctx: &egui::Context,
        released_image_id: Option<labello_domain::ImageId>,
    ) {
        self.work.one_shot_excluded_image_id = released_image_id;
        if self.view == AppView::Annotate {
            while let Some(loaded) = self.work.queue.pop_prepared() {
                if loaded.assignment.status == labello_domain::AssignmentStatus::Active
                    && loaded
                        .assignment
                        .expires_at
                        .is_none_or(|expires_at| expires_at > labello_domain::now())
                {
                    self.apply_loaded_image(ctx, loaded);
                    return;
                }
            }
        }
        self.clear_current_image();
        self.request_next_image();
    }

    pub(crate) fn apply_state(&mut self, state: labello_domain::ImageState) {
        self.renew_assignment_from_state(&state);
        self.work.annotations = state.active_annotations().cloned().collect();
        self.work.persisted_annotations = self
            .work
            .annotations
            .iter()
            .map(|annotation| annotation.annotation_id.clone())
            .collect();
        self.work.current_state = Some(state);
        self.work.modified_annotations.clear();
    }

    pub(crate) fn sync_review_selection(&mut self) {
        if self.view != AppView::Review {
            return;
        }
        let selected = self
            .current_review_annotation()
            .map(|annotation| annotation.annotation_id.clone());
        self.work.selected_annotation = selected;
        if self.work.correction_draft.as_ref().is_some_and(|draft| {
            self.work.selected_annotation.as_ref() != Some(&draft.annotation_id)
        }) {
            self.work.correction_draft = None;
        }
    }

    fn renew_assignment_from_state(&mut self, state: &labello_domain::ImageState) {
        let Some(current) = self.work.assignment.as_ref() else {
            return;
        };
        let renewed = state.assignments.iter().find(|candidate| {
            candidate.image_id == current.image_id
                && candidate.task_id == current.task_id
                && candidate.kind == current.kind
                && candidate.assigned_to == self.config.user_id
                && candidate.status == labello_domain::AssignmentStatus::Active
        });
        if let Some(renewed) = renewed {
            self.work.assignment = Some(renewed.clone());
        }
    }

    fn matches_operation(
        &self,
        operation_id: u64,
        assignment_id: &labello_domain::AssignmentId,
    ) -> bool {
        self.work.active_operation_id == Some(operation_id)
            && self
                .work
                .assignment
                .as_ref()
                .is_some_and(|assignment| assignment.assignment_id == *assignment_id)
    }

}
