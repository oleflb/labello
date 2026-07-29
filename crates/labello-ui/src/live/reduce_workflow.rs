impl LabelloApp {
    fn reduce_workflow_message(
        &mut self,
        ctx: &egui::Context,
        message: UiMessage,
    ) -> Option<UiMessage> {
        match message {
                UiMessage::ImageLoaded {
                    request: _,
                    operation_id,
                    assignment,
                    result,
                } => {
                    if self.work.active_load_id != Some(operation_id) {
                        return None;
                    }
                    self.work.active_load_id = None;
                    self.loading.image = false;
                    match *result {
                        Ok(Some(loaded)) => {
                            self.work.one_shot_excluded_image_id = None;
                            self.runtime.error = None;
                            self.runtime.notice = None;
                            if let Some(expected) =
                                self.runtime.persistence.expected_assignment.take()
                                && loaded.assignment.assignment_id != expected
                            {
                                self.runtime.notice = Some(
                                    "The previous assignment was no longer active; opened the server-assigned work without restoring its old draft."
                                        .to_string(),
                                );
                                self.request_previous_draft_status();
                            }
                            self.apply_loaded_image(ctx, loaded);
                            self.refresh_assignment_availability_if_due();
                        }
                        Ok(None) => {
                            self.work.one_shot_excluded_image_id = None;
                            self.runtime.persistence.expected_assignment = None;
                            self.work.assignment = None;
                            self.runtime.error = None;
                            self.runtime.notice = Some(
                                match self.view {
                                    AppView::Annotate => {
                                        "No annotation work is currently available."
                                    }
                                    AppView::Review => "No reviews are currently waiting.",
                                    AppView::Adjudicate => {
                                        "No adjudications are currently waiting."
                                    }
                                    _ => "No work is currently available.",
                                }
                                .to_string(),
                            );
                            self.request_assignment_availability();
                        }
                        Err(error) => {
                            if assignment.is_some() {
                                self.work.one_shot_excluded_image_id = None;
                            }
                            self.runtime.persistence.expected_assignment = None;
                            self.work.assignment = assignment;
                            self.runtime.error = Some(error);
                            self.request_assignment_availability();
                        }
                    }
                }
                UiMessage::PreviousAssignmentLoaded {
                    request: _,
                    operation_id,
                    assignment,
                    result,
                } => {
                    if self.work.active_load_id != Some(operation_id) {
                        return None;
                    }
                    self.work.active_load_id = None;
                    self.loading.image = false;
                    match *result {
                        Ok(loaded) => {
                            let displaced = self.work.assignment.clone();
                            self.begin_workspace_epoch();
                            self.clear_current_image();
                            if let Some(displaced) = displaced
                                && displaced.assignment_id != loaded.assignment.assignment_id
                            {
                                self.release_reservation(self.config.dataset_id.clone(), displaced);
                            }
                            self.work.previous_annotation_assignment = None;
                            self.runtime.error = None;
                            self.runtime.notice =
                                Some("Returned to previous assignment".to_string());
                            self.apply_loaded_image(ctx, loaded);
                            self.request_assignment_availability();
                        }
                        Err(error) => {
                            let normalized_error = error.to_ascii_lowercase();
                            let expired = normalized_error.contains("lease")
                                && normalized_error.contains("expired");
                            if expired {
                                self.clear_previous_annotation_assignment();
                            } else if let Some(assignment) = assignment {
                                self.work.previous_annotation_assignment = Some(assignment);
                            }
                            self.runtime.error = Some(error);
                            if expired && self.work.assignment.is_none() {
                                self.request_next_image();
                            }
                        }
                    }
                }
                UiMessage::PrefetchLoaded {
                    request: _,
                    operation_id,
                    result,
                } => {
                    if self.work.active_prefetch_id != Some(operation_id) {
                        return None;
                    }
                    self.work.active_prefetch_id = None;
                    self.work.queue.set_loading(false);
                    match *result {
                        Ok(Some(loaded))
                            if loaded.assignment.kind
                                == labello_domain::AssignmentKind::Annotation
                                && loaded.assignment.status
                                    == labello_domain::AssignmentStatus::Active
                                && self.work.assignment.as_ref().is_some_and(|current| {
                                    current.task_id == loaded.assignment.task_id
                                        && current.image_id != loaded.assignment.image_id
                                })
                                && !self
                                    .work
                                    .queue
                                    .prepared_image_ids()
                                    .contains(&loaded.assignment.image_id) =>
                        {
                            self.work.one_shot_excluded_image_id = None;
                            self.work.queue.clear_failure();
                            self.work.queue.push_prepared(loaded);
                            self.request_prefetch();
                        }
                        Ok(Some(loaded)) => {
                            self.work.one_shot_excluded_image_id = None;
                            self.release_reservation(
                                self.config.dataset_id.clone(),
                                loaded.assignment,
                            );
                            let retry_delay = Duration::from_secs(3);
                            self.work.queue.mark_failed_after(retry_delay);
                            ctx.request_repaint_after(retry_delay);
                        }
                        Ok(None) => {
                            self.work.one_shot_excluded_image_id = None;
                            let retry_delay = Duration::from_secs(15);
                            self.work.queue.mark_failed_after(retry_delay);
                            ctx.request_repaint_after(retry_delay);
                        }
                        Err(_) => {
                            self.work.queue.mark_failed();
                            ctx.request_repaint_after(Duration::from_secs(1));
                        }
                    }
                }
                UiMessage::ReservationReleased { result, .. } => {
                    if result.is_err() {
                        self.runtime.notice = Some(
                            "A prepared assignment could not be released; its lease will expire."
                                .to_string(),
                        );
                    } else {
                        self.refresh_assignment_availability_if_due();
                    }
                }
                UiMessage::SaveFinished {
                    request,
                    operation_id,
                    assignment_id,
                    edit_generation,
                    completed,
                    result,
                } => {
                    if !self.matches_operation(operation_id, &assignment_id) {
                        return None;
                    }
                    self.work.active_operation_id = None;
                    self.loading.saving = false;
                    match *result {
                        Ok(state) => {
                            if self.work.edit_generation == edit_generation {
                                if let Some(assignment) = self.work.assignment.as_ref() {
                                    let assignment = assignment.clone();
                                    self.clear_current_work_draft(&assignment);
                                }
                                self.apply_state(state);
                                self.work.save_status = SaveStatus::Saved;
                            } else {
                                self.renew_assignment_from_state(&state);
                                self.work.persisted_annotations =
                                    state.annotations.keys().cloned().collect();
                                self.work.current_state = Some(state);
                                self.recompute_modified_annotations();
                                self.work.save_status = SaveStatus::Dirty;
                                self.rebase_work_draft_after_save(edit_generation);
                            }
                            self.runtime.error = None;
                            self.request_stats();
                            if completed {
                                if let Some(mut assignment) =
                                    self.work.assignment.clone().filter(|assignment| {
                                        assignment.kind
                                            == labello_domain::AssignmentKind::Annotation
                                    })
                                {
                                    assignment.status = labello_domain::AssignmentStatus::Completed;
                                    self.remember_previous_annotation_assignment(assignment);
                                }
                                let load_after_resolution = matches!(
                                    self.work.pending_transition.as_ref(),
                                    Some(
                                        crate::app::PendingTransition::NextAssignment
                                            | crate::app::PendingTransition::Workflow(_)
                                            | crate::app::PendingTransition::View(
                                                AppView::Annotate
                                                    | AppView::Review
                                                    | AppView::Adjudicate
                                            )
                                    )
                                );
                                self.finish_annotation_transition(ctx, None);
                                self.assignment_availability_mutation_completed(
                                    request
                                        .dataset_id
                                        .as_ref()
                                        .expect("annotation mutations are dataset-scoped"),
                                    load_after_resolution,
                                );
                            }
                        }
                        Err(error) => {
                            self.work.save_status = if self.work.edit_generation == edit_generation
                            {
                                SaveStatus::Retry
                            } else {
                                SaveStatus::Dirty
                            };
                            if completed {
                                self.work.pending_transition = None;
                                self.assignment_availability_mutation_completed(
                                    request
                                        .dataset_id
                                        .as_ref()
                                        .expect("annotation mutations are dataset-scoped"),
                                    false,
                                );
                            }
                            self.runtime.error = Some(error);
                        }
                    }
                }
                UiMessage::ReleaseFinished {
                    request: _,
                    operation_id,
                    assignment_id,
                    result,
                } => {
                    if !self.matches_operation(operation_id, &assignment_id) {
                        return None;
                    }
                    self.work.active_operation_id = None;
                    self.loading.saving = false;
                    match result {
                        Ok(()) => {
                            let released_image_id = self
                                .work
                                .assignment
                                .as_ref()
                                .map(|assignment| assignment.image_id.clone());
                            if let Some(assignment) = self.work.assignment.clone() {
                                self.clear_current_work_draft(&assignment);
                                if assignment.kind == labello_domain::AssignmentKind::Annotation {
                                    let mut assignment = assignment;
                                    assignment.status = labello_domain::AssignmentStatus::Cancelled;
                                    self.remember_previous_annotation_assignment(assignment);
                                }
                            }
                            self.runtime.error = None;
                            self.finish_annotation_transition(ctx, released_image_id);
                            self.refresh_assignment_availability_if_due();
                        }
                        Err(error) => {
                            self.work.pending_transition = None;
                            self.runtime.error = Some(error);
                        }
                    }
                }
                UiMessage::ReviewFinished {
                    request,
                    operation_id,
                    assignment_id,
                    phase,
                    decision,
                    result,
                } => {
                    if !self.matches_operation(operation_id, &assignment_id) {
                        return None;
                    }
                    self.work.active_operation_id = None;
                    self.loading.saving = false;
                    match *result {
                        Ok(state) => {
                            let completed_assignment = self.work.assignment.clone();
                            self.runtime.error = None;
                            self.apply_state(state);
                            self.work.review_index = self
                                .work
                                .selected_task_id
                                .as_ref()
                                .map(|task_id| {
                                    crate::review_sequence::reviewed_object_prefix(
                                        self.work
                                            .current_state
                                            .as_ref()
                                            .expect("state was applied"),
                                        task_id,
                                        &self.config.user_id,
                                    )
                                })
                                .unwrap_or(0);
                            if let Some(assignment) = completed_assignment {
                                self.clear_current_work_draft(&assignment);
                            }
                            match phase {
                                crate::app::ReviewPhase::Object
                                    if decision == labello_domain::ReviewDecision::Approved =>
                                {
                                    self.discard_correction();
                                    self.sync_review_selection();
                                }
                                crate::app::ReviewPhase::Object => {
                                    self.work.review_rejected = true;
                                    self.request_full_image_review(
                                        labello_domain::ReviewDecision::Rejected,
                                    );
                                }
                                crate::app::ReviewPhase::FullImage => {
                                    self.request_stats();
                                    self.clear_current_image();
                                }
                            }
                            self.assignment_availability_mutation_completed(
                                request
                                    .dataset_id
                                    .as_ref()
                                    .expect("review mutations are dataset-scoped"),
                                phase == crate::app::ReviewPhase::FullImage,
                            );
                        }
                        Err(error) => {
                            self.work.pending_transition = None;
                            self.assignment_availability_mutation_completed(
                                request
                                    .dataset_id
                                    .as_ref()
                                    .expect("review mutations are dataset-scoped"),
                                false,
                            );
                            self.runtime.error = Some(error);
                        }
                    }
                }
                UiMessage::CorrectionFinished {
                    request,
                    operation_id,
                    assignment_id,
                    result,
                } => {
                    if !self.matches_operation(operation_id, &assignment_id) {
                        return None;
                    }
                    self.work.active_operation_id = None;
                    self.loading.saving = false;
                    match result {
                        Ok(()) => {
                            if let Some(assignment) = self.work.assignment.clone() {
                                self.clear_current_work_draft(&assignment);
                            }
                            self.runtime.error = None;
                            self.request_stats();
                            self.clear_current_image();
                            self.assignment_availability_mutation_completed(
                                request
                                    .dataset_id
                                    .as_ref()
                                    .expect("correction mutations are dataset-scoped"),
                                true,
                            );
                        }
                        Err(error) => {
                            self.work.pending_transition = None;
                            self.assignment_availability_mutation_completed(
                                request
                                    .dataset_id
                                    .as_ref()
                                    .expect("correction mutations are dataset-scoped"),
                                false,
                            );
                            self.runtime.error = Some(error);
                        }
                    }
                }
                UiMessage::AdjudicationFinished {
                    request,
                    operation_id,
                    assignment_id,
                    result,
                } => {
                    if !self.matches_operation(operation_id, &assignment_id) {
                        return None;
                    }
                    self.work.active_operation_id = None;
                    self.loading.saving = false;
                    match result {
                        Ok(()) => {
                            self.runtime.error = None;
                            self.request_stats();
                            self.clear_current_image();
                            self.assignment_availability_mutation_completed(
                                request
                                    .dataset_id
                                    .as_ref()
                                    .expect("adjudication mutations are dataset-scoped"),
                                true,
                            );
                        }
                        Err(error) => {
                            self.work.pending_transition = None;
                            self.assignment_availability_mutation_completed(
                                request
                                    .dataset_id
                                    .as_ref()
                                    .expect("adjudication mutations are dataset-scoped"),
                                false,
                            );
                            self.runtime.error = Some(error);
                        }
                    }
                }
                UiMessage::IngestJobLoaded { result, .. } => self.handle_ingest_job(result),
            message => return Some(message),
        }
        None
    }
}
