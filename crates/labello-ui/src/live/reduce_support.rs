impl LabelloApp {
    fn reduce_support_message(
        &mut self,
        ctx: &egui::Context,
        message: UiMessage,
    ) -> Option<UiMessage> {
        match message {
                UiMessage::StatsLoaded { request, result } => {
                    let dataset_id = request.dataset_id?;
                    if !self.datasets.active_stats_request.as_ref().is_some_and(
                        |(active_request_id, active_dataset_id)| {
                            *active_request_id == request.request_id
                                && active_dataset_id == &dataset_id
                        },
                    ) || self.config.dataset_id != dataset_id
                    {
                        return None;
                    }
                    self.datasets.active_stats_request = None;
                    self.loading.stats = false;
                    match result {
                        Ok(stats) => {
                            self.datasets.stats = stats;
                            self.datasets.last_stats_completion = Some(Instant::now());
                            self.datasets.stats_error = None;
                        }
                        Err(error) => self.datasets.stats_error = Some(error),
                    }
                }
                UiMessage::AssignmentAvailabilityLoaded { result, .. } => {
                    self.work.availability.loading = false;
                    self.work.availability.last_attempt = Some(Instant::now());
                    if std::mem::take(&mut self.work.availability.refresh_after_load) {
                        let migration_active = self.manual_migration_active();
                        if !migration_active {
                            self.work.availability.tasks.clear();
                        }
                        self.work.availability.resolved = false;
                        self.work.availability.checked_at = None;
                        self.work.availability.error = None;
                        if !self.work.migration.busy && !migration_active {
                            self.work.availability.last_attempt = None;
                            self.request_assignment_availability();
                        }
                        return None;
                    }
                    match result {
                        Ok(availability)
                            if self.assignment_kind().as_ref() == Some(&availability.kind) =>
                        {
                            let checked_at = labello_domain::now();
                            for related in availability.related {
                                self.cache_assignment_availability(
                                    self.config.dataset_id.clone(),
                                    related.kind,
                                    related.tasks,
                                    checked_at,
                                );
                            }
                            self.work.availability.dataset_id =
                                Some(self.config.dataset_id.clone());
                            self.work.availability.kind = Some(availability.kind);
                            self.work.availability.tasks = availability.tasks;
                            self.work.availability.resolved = true;
                            self.work.availability.checked_at = Some(checked_at);
                            self.work.availability.error = None;
                            if self.work.availability.load_after_resolution {
                                self.request_next_image();
                            }
                        }
                        Ok(_) => {
                            self.work.availability.tasks.clear();
                            self.work.availability.resolved = false;
                            self.work.availability.checked_at = None;
                            self.work.availability.error =
                                Some("Availability response did not match this workspace.".into());
                        }
                        Err(error) => {
                            self.work.availability.tasks.clear();
                            self.work.availability.resolved = false;
                            self.work.availability.checked_at = None;
                            self.work.availability.error = Some(error);
                        }
                    }
                }
                UiMessage::KeybindingsSaved { result, .. } => {
                    self.loading.keybindings = false;
                    match result {
                        Ok(keybindings) => {
                            self.work.keybindings = keybindings;
                            self.work.shortcut_settings.error = None;
                            if self.work.show_settings {
                                self.work.shortcut_settings.baseline =
                                    Some(self.work.keybindings.clone());
                                self.work.shortcut_settings.draft =
                                    Some(self.work.keybindings.clone());
                            }
                            self.runtime.notice = Some("Keyboard shortcuts saved".to_string());
                            self.runtime.error = None;
                        }
                        Err(error) => {
                            self.work.shortcut_settings.error = Some(error.clone());
                            self.runtime.error = Some(error);
                        }
                    }
                }
                UiMessage::RequestFailed { error, .. } => {
                    self.invalidate_async_ownership();
                    self.runtime.error = Some(error);
                }
                UiMessage::FolderUploadProgress { request, progress } => {
                    if !self.request_is_current(&request, true) {
                        return None;
                    }
                    self.loading.uploading = true;
                    self.loading.upload_progress = Some(progress);
                    self.runtime.error = None;
                    ctx.request_repaint();
                }
                UiMessage::FolderUploadFinished { request, result } => {
                    if !self.request_is_current(&request, true) {
                        return None;
                    }
                    self.loading.uploading = false;
                    self.loading.upload_progress = None;
                    match result {
                        Ok(message) => {
                            self.runtime.notice = Some(message);
                            self.runtime.error = None;
                            self.admin.upload_error = None;
                            self.request_admin_dataset();
                            self.request_dataset_list();
                        }
                        Err(error) => {
                            self.admin.upload_error = Some(error.clone());
                            self.runtime.error = Some(error);
                        }
                    }
                }
                UiMessage::PersistenceFinished(completion) => {
                    self.handle_persistence_completion(*completion);
                }
            message => return Some(message),
        }
        None
    }
}
