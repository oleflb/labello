impl LabelloApp {
    pub(crate) fn request_dataset_list(&mut self) {
        if self.loading.datasets || self.runtime.api.is_none() {
            return;
        }
        self.loading.datasets = true;
        if self
            .datasets
            .summaries_error
            .take()
            .as_ref()
            .is_some_and(|error| self.runtime.error.as_ref() == Some(error))
        {
            self.runtime.error = None;
        }
        let request = self.request_identity(None);
        self.queue_command(UiCommand::DatasetList { request });
    }

    pub(crate) fn request_create_dataset(&mut self) {
        if self.loading.dataset || self.runtime.api.is_none() {
            return;
        }
        let dataset_id = labello_domain::DatasetId::from(self.setup.create_dataset_id.trim());
        if let Err(error) = dataset_id.validate_path_segment() {
            self.runtime.error = Some(format!("Dataset ID: {error}"));
            return;
        }
        let name = self.setup.create_dataset_name.trim().to_string();
        if name.is_empty() {
            self.runtime.error = Some("Dataset name cannot be empty".to_string());
            return;
        }
        self.loading.dataset = true;
        let request = self.request_identity(Some(dataset_id.clone()));
        self.queue_command(UiCommand::CreateDataset {
            request,
            dataset_id,
            name,
            admin_user_id: self.config.user_id.clone(),
        });
    }

    pub(crate) fn request_load_dataset(&mut self) {
        if self.loading.dataset || self.runtime.api.is_none() {
            return;
        }
        self.runtime.persistence.restoration_attempted = true;
        self.begin_workspace_epoch();
        self.loading.dataset = true;
        let request = self.request_identity(Some(self.config.dataset_id.clone()));
        self.queue_command(UiCommand::LoadDataset {
            request,
            dataset_id: self.config.dataset_id.clone(),
            user_id: self.config.user_id.clone(),
        });
    }

    pub(crate) fn request_admin_dataset(&mut self) {
        self.view = AppView::Admin;
        if self
            .datasets
            .admin_config
            .as_ref()
            .is_some_and(|config| config.dataset_id != self.config.dataset_id)
        {
            self.datasets.admin_config = None;
            self.datasets.admin_baseline = None;
            self.datasets.users.clear();
            self.datasets.users_baseline.clear();
            self.admin = Default::default();
        }
        if self.loading.admin || self.runtime.api.is_none() {
            return;
        }
        self.loading.admin = true;
        self.admin.load_error = None;
        let request = self.request_identity(Some(self.config.dataset_id.clone()));
        self.queue_command(UiCommand::LoadAdmin {
            request,
            dataset_id: self.config.dataset_id.clone(),
        });
    }

    pub(crate) fn request_images(&mut self) {
        if self.loading.images
            || self.loading.admin
            || self.loading.uploading
            || self.loading.ingesting
            || self.runtime.api.is_none()
        {
            return;
        }
        self.admin.image_query.search = non_empty(&self.admin.image_search);
        self.admin.image_query.task_id = self.admin.image_task.clone();
        self.admin.image_query.class_id = self.admin.image_class.clone();
        self.admin.image_query.status = self.admin.image_status.clone();
        self.loading.images = true;
        self.admin.images_error = None;
        let request = self.request_identity(Some(self.config.dataset_id.clone()));
        self.queue_command(UiCommand::LoadImages {
            request,
            dataset_id: self.config.dataset_id.clone(),
            query: self.admin.image_query.clone(),
        });
    }

    pub(crate) fn request_snapshots(&mut self) {
        if self.loading.snapshots || self.loading.creating_snapshot || self.runtime.api.is_none() {
            return;
        }
        self.loading.snapshots = true;
        self.admin.snapshots_error = None;
        let request = self.request_identity(Some(self.config.dataset_id.clone()));
        self.queue_command(UiCommand::LoadSnapshots {
            request,
            dataset_id: self.config.dataset_id.clone(),
        });
    }

    pub(crate) fn request_snapshot_create(&mut self) {
        if self.loading.creating_snapshot || self.loading.snapshots || self.runtime.api.is_none() {
            return;
        }
        self.loading.creating_snapshot = true;
        self.admin.snapshot_action_error = None;
        let request = self.request_identity(Some(self.config.dataset_id.clone()));
        self.queue_command(UiCommand::CreateSnapshot {
            request,
            dataset_id: self.config.dataset_id.clone(),
        });
    }

    pub(crate) fn request_snapshot_download(&mut self, snapshot_id: String, path: String) {
        if self.loading.snapshot_file.is_some() || self.runtime.api.is_none() {
            return;
        }
        self.loading.snapshot_file = Some((snapshot_id.clone(), path.clone()));
        self.admin.snapshot_action_error = None;
        let request = self.request_identity(Some(self.config.dataset_id.clone()));
        self.queue_command(UiCommand::DownloadSnapshot {
            request,
            dataset_id: self.config.dataset_id.clone(),
            snapshot_id,
            path,
        });
    }

    pub(crate) fn request_admin_changes_save(&mut self) {
        if self.loading.admin || self.loading.roles_user.is_some() {
            return;
        }
        let baseline = &self.datasets.users_baseline;
        let mut dirty_users = self
            .datasets
            .users
            .iter()
            .filter(|user| {
                baseline
                    .iter()
                    .find(|saved| saved.account.user_id == user.account.user_id)
                    .is_none_or(|saved| saved.roles != user.roles)
            })
            .map(|user| {
                (
                    user.account.user_id.clone(),
                    user.roles.contains(&labello_domain::DatasetRole::DataAdmin),
                )
            })
            .collect::<Vec<_>>();
        dirty_users.sort_by_key(|(_, remains_admin)| !*remains_admin);
        self.admin.pending_role_saves = dirty_users
            .into_iter()
            .map(|(user_id, _)| user_id)
            .collect();

        if self.datasets.admin_config != self.datasets.admin_baseline {
            if !self.request_admin_save() {
                self.admin.pending_role_saves.clear();
            }
        } else {
            self.request_next_admin_role_save();
        }
    }

    pub(crate) fn request_admin_save(&mut self) -> bool {
        let Some(metadata) = self.datasets.admin_config.clone() else {
            return false;
        };
        if self.loading.admin || self.loading.roles_user.is_some() {
            return false;
        }
        self.loading.admin = true;
        let request = self.request_identity(Some(self.config.dataset_id.clone()));
        self.queue_command(UiCommand::SaveAdmin { request, metadata })
    }

    fn request_next_admin_role_save(&mut self) {
        let Some(user_id) = self.admin.pending_role_saves.pop_front() else {
            self.runtime.notice = Some("Admin changes saved".to_string());
            return;
        };
        if !self.request_role_save(user_id) {
            self.admin.pending_role_saves.clear();
        }
    }

    fn request_role_save(&mut self, user_id: labello_domain::UserId) -> bool {
        if self.loading.admin || self.loading.roles_user.is_some() {
            return false;
        }
        if self.runtime.api.is_none() {
            self.runtime.error = Some("API is not configured".to_string());
            return false;
        }
        let Some(user) = self
            .datasets
            .users
            .iter()
            .find(|user| user.account.user_id == user_id)
        else {
            return false;
        };
        let removes_admin = !user.roles.contains(&labello_domain::DatasetRole::DataAdmin);
        if user_id == self.config.user_id && removes_admin {
            self.runtime.error = Some("You cannot remove your own data admin role.".to_string());
            return false;
        }
        let admin_count = self
            .datasets
            .users
            .iter()
            .filter(|user| user.roles.contains(&labello_domain::DatasetRole::DataAdmin))
            .count();
        let was_admin = self
            .datasets
            .users_baseline
            .iter()
            .find(|user| user.account.user_id == user_id)
            .is_some_and(|user| user.roles.contains(&labello_domain::DatasetRole::DataAdmin));
        if was_admin && removes_admin && admin_count == 0 {
            self.runtime.error = Some("At least one data admin must remain.".to_string());
            return false;
        }
        let roles = user.roles.clone();
        self.loading.roles_user = Some(user_id.clone());
        let request = self.request_identity(Some(self.config.dataset_id.clone()));
        self.queue_command(UiCommand::SaveDatasetRoles {
            request,
            dataset_id: self.config.dataset_id.clone(),
            user_id,
            roles,
        })
    }

    fn sync_role_assignment(&mut self, user: &labello_client::DatasetUser) {
        let assigned_at = labello_domain::now();
        for metadata in [
            self.datasets.metadata.as_mut(),
            self.datasets.admin_config.as_mut(),
            self.datasets.admin_baseline.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            metadata
                .role_assignments
                .retain(|assignment| assignment.user_id != user.account.user_id);
            if !user.roles.is_empty() {
                metadata
                    .role_assignments
                    .push(labello_domain::DatasetRoleAssignment {
                        dataset_id: metadata.dataset_id.clone(),
                        user_id: user.account.user_id.clone(),
                        roles: user.roles.iter().cloned().collect(),
                        assigned_at,
                        assigned_by: Some(self.config.user_id.clone()),
                    });
            }
        }
        if user.account.user_id == self.config.user_id
            && let Some(summary) = self
                .datasets
                .summaries
                .iter_mut()
                .find(|summary| summary.dataset_id == self.config.dataset_id)
        {
            summary.roles = user.roles.clone();
        }
    }

    pub(crate) fn request_ingest(&mut self) {
        if self.admin_mutation_blocked() || self.runtime.api.is_none() {
            return;
        }
        self.loading.ingesting = true;
        self.loading.ingest_polling = false;
        self.loading.ingest_job_id = None;
        self.loading.last_ingest_poll = None;
        self.runtime.notice = Some("Starting ingest...".to_string());
        let request = self.request_identity(Some(self.config.dataset_id.clone()));
        self.queue_command(UiCommand::Ingest {
            request,
            dataset_id: self.config.dataset_id.clone(),
        });
    }

    pub(crate) fn admin_mutation_blocked(&self) -> bool {
        self.loading.ingesting
            || self.loading.uploading
            || self.loading.images
            || self.loading.admin
            || self.loading.roles_user.is_some()
            || self.datasets.admin_config != self.datasets.admin_baseline
            || self.datasets.users != self.datasets.users_baseline
    }

    pub(crate) fn request_stats(&mut self) {
        if self.loading.stats
            || self.runtime.api.is_none()
            || self.loading.image
            || !matches!(self.view, AppView::Stats)
        {
            return;
        }
        let dataset_id = self.config.dataset_id.clone();
        let request = self.request_identity(Some(dataset_id.clone()));
        self.datasets.stats_request_id = request.request_id;
        self.loading.stats = true;
        self.datasets.stats_error = None;
        self.datasets.last_stats_attempt = Some(Instant::now());
        self.datasets.active_stats_request = Some((request.request_id, dataset_id.clone()));
        self.queue_command(UiCommand::Stats {
            request,
            dataset_id,
        });
    }

    pub(crate) fn request_keybindings_save(&mut self) {
        if self.loading.keybindings || self.runtime.api.is_none() {
            return;
        }
        let keybindings = self
            .work
            .shortcut_settings
            .draft
            .clone()
            .unwrap_or_else(|| self.work.keybindings.clone());
        if let Err(error) = keybindings.validate() {
            self.work.shortcut_settings.error = Some(error.to_string());
            self.runtime.error = Some(error.to_string());
            return;
        }
        self.loading.keybindings = true;
        self.work.shortcut_settings.error = None;
        let request = self.request_identity(Some(self.config.dataset_id.clone()));
        self.queue_command(UiCommand::SaveKeybindings {
            request,
            dataset_id: self.config.dataset_id.clone(),
            keybindings,
        });
    }

}
