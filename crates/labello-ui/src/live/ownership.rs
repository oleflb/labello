impl LabelloApp {
    pub(crate) fn queue_command(&mut self, command: UiCommand) -> bool {
        if self.runtime.commands.len() < 64 {
            if command.invalidates_assignment_availability() {
                self.invalidate_assignment_availability(command.request().dataset_id.as_ref());
            }
            let request_id = command
                .import_request()
                .map(|request| request.request_id)
                .unwrap_or_else(|| command.request().request_id);
            self.runtime.active_requests.insert(request_id);
            if let Some(activity) = command.import_activity() {
                self.import
                    .active_operations
                    .insert(request_id, activity);
            }
            self.runtime.commands.push_back(command);
            true
        } else {
            self.rollback_command(
                &command,
                "The request queue is full; the operation was not started.",
            );
            false
        }
    }

    fn sync_import_busy(&mut self) {
        self.import.busy = self
            .import
            .active_operations
            .values()
            .any(|activity| activity.blocks_controls());
    }

    fn rollback_command(&mut self, command: &UiCommand, error: &str) {
        let request_id = command
            .import_request()
            .map(|request| request.request_id)
            .unwrap_or_else(|| command.request().request_id);
        self.runtime.active_requests.remove(&request_id);
        self.import.active_operations.remove(&request_id);
        match command {
            UiCommand::ImportCapabilities { .. } => {
                self.import.capabilities_loading = false;
                self.import.capabilities_error = Some(error.to_string());
            }
            UiCommand::BrowseImportRoot { .. } | UiCommand::BrowseImportSource { .. } => {
                self.import.source_picker.loading = false;
                self.import.source_picker.pending_request_id = None;
                self.import.source_picker.error = Some(error.to_string());
            }
            UiCommand::InspectYoloDescriptor { .. } => {
                self.import.yolo_inspection_loading = false;
                self.import.pending_yolo_inspection_request_id = None;
                self.import.yolo_inspection_error = Some(error.to_string());
            }
            UiCommand::CreateImport { .. }
            | UiCommand::GetImport { .. }
            | UiCommand::RegisterImportFiles { .. }
            | UiCommand::SealImport { .. }
            | UiCommand::PreflightImport { .. }
            | UiCommand::UpdateImportPlan { .. }
            | UiCommand::ImportDiagnostics { .. }
            | UiCommand::CommitImport { .. }
            | UiCommand::CancelImport { .. } => {
                self.sync_import_busy();
                self.import.error = Some(error.to_string());
            }
            UiCommand::Migration { .. } => {
                self.work.migration.busy = false;
                self.work.migration.error = Some(error.to_string());
            }
            UiCommand::AuthOptions { .. } => {
                self.loading.session = false;
                self.auth.options_checked = true;
                self.auth.checked = true;
            }
            UiCommand::Session { .. } | UiCommand::LocalAdminLogin { .. } => {
                self.loading.session = false;
                self.auth.active_session_request_id = None;
                self.auth.local_admin_login_pending = false;
                self.auth.checked = true;
            }
            UiCommand::Logout { .. } => self.loading.logout = false,
            UiCommand::GithubLogin { .. } => {}
            UiCommand::DatasetList { .. } => {
                self.loading.datasets = false;
                self.datasets.summaries_error = Some(error.to_string());
            }
            UiCommand::CreateDataset { .. } | UiCommand::LoadDataset { .. } => {
                self.loading.dataset = false
            }
            UiCommand::LoadAdmin { .. } => {
                self.loading.admin = false;
                self.admin.load_error = Some(error.to_string());
            }
            UiCommand::SaveAdmin { .. } => self.loading.admin = false,
            UiCommand::SaveDatasetRoles { .. } => self.loading.roles_user = None,
            UiCommand::LoadImages { .. } => {
                self.loading.images = false;
                self.admin.images_error = Some(error.to_string());
            }
            UiCommand::LoadSnapshots { .. } => {
                self.loading.snapshots = false;
                self.admin.snapshots_error = Some(error.to_string());
            }
            UiCommand::CreateSnapshot { .. } => {
                self.loading.creating_snapshot = false;
                self.admin.snapshot_action_error = Some(error.to_string());
            }
            UiCommand::DownloadSnapshot { .. } => {
                self.loading.snapshot_file = None;
                self.admin.snapshot_action_error = Some(error.to_string());
            }
            UiCommand::Ingest { .. } => {
                self.loading.ingesting = false;
                self.loading.ingest_polling = false;
                self.loading.ingest_job_id = None;
            }
            UiCommand::PollIngest { .. } => self.loading.ingest_polling = false,
            UiCommand::Stats { .. } => {
                self.loading.stats = false;
                self.datasets.active_stats_request = None;
                self.datasets.stats_error = Some(error.to_string());
            }
            UiCommand::AssignmentAvailability { .. } => {
                self.work.availability.loading = false;
                self.work.availability.tasks.clear();
                self.work.availability.resolved = false;
                self.work.availability.checked_at = None;
                self.work.availability.error = Some(error.to_string());
            }
            UiCommand::SaveKeybindings { .. } => {
                self.loading.keybindings = false;
                self.work.shortcut_settings.error = Some(error.to_string());
            }
            UiCommand::ClaimAssignment { operation_id, .. }
            | UiCommand::ReloadAssignment { operation_id, .. }
            | UiCommand::ReopenAssignment { operation_id, .. } => {
                if self.work.active_load_id == Some(*operation_id) {
                    self.work.active_load_id = None;
                    self.loading.image = false;
                }
            }
            UiCommand::PrefetchAssignment { operation_id, .. } => {
                if self.work.active_prefetch_id == Some(*operation_id) {
                    self.work.active_prefetch_id = None;
                    self.work.queue.set_loading(false);
                    self.work.queue.mark_failed();
                }
            }
            UiCommand::ReleaseReservation { .. } => {}
            UiCommand::SaveAnnotations { operation_id, .. }
            | UiCommand::ReleaseAssignment { operation_id, .. }
            | UiCommand::Review { operation_id, .. }
            | UiCommand::Correction { operation_id, .. }
            | UiCommand::Adjudication { operation_id, .. } => {
                if self.work.active_operation_id == Some(*operation_id) {
                    self.work.active_operation_id = None;
                    self.loading.saving = false;
                    self.work.pending_transition = None;
                    if matches!(command, UiCommand::SaveAnnotations { .. }) {
                        self.work.save_status = SaveStatus::Retry;
                    }
                }
            }
        }
        self.runtime.error = Some(error.to_string());
    }

    fn request_identity(
        &mut self,
        dataset_id: Option<labello_domain::DatasetId>,
    ) -> RequestIdentity {
        RequestIdentity {
            auth_epoch: self.auth_epoch,
            workspace_epoch: self.workspace_epoch,
            request_id: self.next_operation(),
            dataset_id,
        }
    }

    pub(crate) fn import_request_identity(
        &mut self,
        import_id: Option<labello_domain::ImportId>,
    ) -> ImportRequestIdentity {
        ImportRequestIdentity {
            auth_epoch: self.auth_epoch,
            import_epoch: self.import_epoch,
            request_id: self.next_operation(),
            import_id,
        }
    }

    pub(crate) fn begin_import_epoch(&mut self) {
        self.import_epoch = self.import_epoch.wrapping_add(1);
        let queued_import_requests = self
            .runtime
            .commands
            .iter()
            .filter_map(|command| command.import_request().map(|request| request.request_id))
            .collect::<Vec<_>>();
        self.runtime
            .commands
            .retain(|command| command.import_request().is_none());
        for request_id in queued_import_requests {
            self.runtime.active_requests.remove(&request_id);
        }
        self.import.busy = false;
        self.import.active_operations.clear();
        self.import.poll_after = None;
    }

    fn finish_import_request(&mut self, request: &ImportRequestIdentity) -> bool {
        let owner_matches = request.import_id.as_ref().is_none_or(|owner| {
            self.import
                .job
                .as_ref()
                .is_none_or(|job| &job.import_id == owner)
                || self.import.recovery_import_id == owner.as_str()
        });
        let current = request.auth_epoch == self.auth_epoch
            && request.import_epoch == self.import_epoch
            && owner_matches;
        let active = self.runtime.active_requests.remove(&request.request_id);
        self.import
            .active_operations
            .remove(&request.request_id);
        current && active
    }

    pub(crate) fn operation_identity(
        &self,
        operation_id: u64,
        dataset_id: labello_domain::DatasetId,
    ) -> RequestIdentity {
        RequestIdentity {
            auth_epoch: self.auth_epoch,
            workspace_epoch: self.workspace_epoch,
            request_id: operation_id,
            dataset_id: Some(dataset_id),
        }
    }

    fn finish_request(
        &mut self,
        request: &RequestIdentity,
        requires_current_dataset: bool,
    ) -> bool {
        self.request_is_current(request, requires_current_dataset)
            && self.runtime.active_requests.remove(&request.request_id)
    }

    fn request_is_current(
        &self,
        request: &RequestIdentity,
        requires_current_dataset: bool,
    ) -> bool {
        request.auth_epoch == self.auth_epoch
            && request.workspace_epoch == self.workspace_epoch
            && (!requires_current_dataset
                || request
                    .dataset_id
                    .as_ref()
                    .is_none_or(|dataset_id| dataset_id == &self.config.dataset_id))
    }

    fn invalidate_async_ownership(&mut self) {
        self.runtime.commands.clear();
        self.runtime.active_requests.clear();
        self.auth.active_session_request_id = None;
        self.datasets.active_stats_request = None;
        self.loading.session = false;
        self.loading.logout = false;
        self.loading.datasets = false;
        self.loading.dataset = false;
        self.loading.admin = false;
        self.loading.roles_user = None;
        self.admin.pending_role_saves.clear();
        self.loading.image = false;
        self.loading.saving = false;
        self.loading.ingesting = false;
        self.loading.ingest_polling = false;
        self.loading.ingest_job_id = None;
        self.loading.last_ingest_poll = None;
        self.loading.uploading = false;
        self.loading.upload_progress = None;
        self.loading.stats = false;
        self.work.availability.loading = false;
        self.loading.keybindings = false;
        self.loading.images = false;
        self.loading.snapshots = false;
        self.loading.creating_snapshot = false;
        self.loading.snapshot_file = None;
        self.work.active_load_id = None;
        self.work.active_prefetch_id = None;
        self.work.active_operation_id = None;
        self.work.queue.set_loading(false);
        self.release_prepared_assignments();
        self.work.one_shot_excluded_image_id = None;
        if self.work.save_status == SaveStatus::Saving {
            self.work.save_status = SaveStatus::Retry;
        }
    }

    pub(crate) fn begin_auth_epoch(&mut self) {
        self.auth_epoch = self.auth_epoch.wrapping_add(1);
        self.workspace_epoch = self.workspace_epoch.wrapping_add(1);
        self.invalidate_async_ownership();
        self.work.availability = Default::default();
        self.datasets.requested_view = None;
        self.runtime.persistence.restoration_attempted = false;
    }

    pub(crate) fn begin_workspace_epoch(&mut self) {
        self.workspace_epoch = self.workspace_epoch.wrapping_add(1);
        self.invalidate_async_ownership();
        self.reset_assignment_availability_for_workspace();
    }

}
