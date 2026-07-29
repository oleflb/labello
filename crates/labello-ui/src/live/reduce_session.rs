impl LabelloApp {
    fn reduce_session_message(
        &mut self,
        ctx: &egui::Context,
        message: UiMessage,
    ) -> Option<UiMessage> {
        match message {
                UiMessage::MigrationFinished { request, result } => {
                    self.work.migration.busy = false;
                    let pending_activate_target =
                        self.work.migration.pending_activate_target.take();
                    match *result {
                        Ok(result) => {
                            let completed_assignment =
                                result.assignment.clone().filter(|assignment| {
                                    assignment.status
                                        == labello_domain::AssignmentStatus::Completed
                                });
                            self.apply_state(result.image_state);
                            self.work.migration.cursor = result.cursor;
                            let pending_activate_target =
                                pending_activate_target.filter(|target| {
                                    !matches!(
                                        self.work.migration.cursor.as_ref(),
                                        Some(labello_domain::MigrationCursor::Object {
                                            object_group_id,
                                            ..
                                        }) if object_group_id == target
                                    )
                                });
                            self.work.migration.inspected_group_id =
                                pending_activate_target.clone();
                            self.work.migration.pending_revisit_target = None;
                            self.work.migration.adding_missing_object = false;
                            self.work.migration.progress = Some(result.progress);
                            self.work.migration.active_pass_id =
                                result.active_pass.map(|pass| pass.pass_id);
                            if let Some(assignment) = result.assignment {
                                self.work.assignment = Some(assignment);
                            }
                            self.work.migration.draft = None;
                            self.work.migration.draft_group = None;
                            self.work.migration.draft_dirty = false;
                            self.work.migration.keypoint_index = 0;
                            self.work.migration.exclusion_note.clear();
                            self.work.migration.exclusion_dirty = false;
                            self.work.migration.error = None;
                            if self.view == AppView::Review {
                                self.work.migration.review_index =
                                    self.canonical_migration_review_index();
                            }
                            let migration_completed = completed_assignment.is_some();
                            if let Some(assignment) = completed_assignment {
                                self.remember_previous_annotation_assignment(assignment);
                                self.open_next_annotation_assignment(ctx, None);
                            }
                            if let Some(target) = pending_activate_target {
                                self.work.migration.pending_activate_target =
                                    Some(target.clone());
                                self.request_revisit_migration_target(target);
                            }
                            if migration_completed {
                                self.assignment_availability_mutation_completed(
                                    request
                                        .dataset_id
                                        .as_ref()
                                        .expect("migration mutations are dataset-scoped"),
                                    true,
                                );
                            }
                        }
                        Err(error) => {
                            if pending_activate_target.is_some() {
                                self.work.migration.inspected_group_id = None;
                            }
                            self.work.migration.error = Some(error);
                            self.assignment_availability_mutation_completed(
                                request
                                    .dataset_id
                                    .as_ref()
                                    .expect("migration mutations are dataset-scoped"),
                                false,
                            );
                        }
                    }
                }
                UiMessage::AuthOptionsLoaded { result, .. } => {
                    self.loading.session = false;
                    self.auth.options_checked = true;
                    match result {
                        Ok(options) => {
                            self.auth.options = options;
                            self.runtime.error = None;
                        }
                        Err(error) => {
                            self.clear_authenticated_state();
                            self.auth.checked = true;
                            self.runtime.error = Some(error);
                        }
                    }
                }
                UiMessage::SessionLoaded { request, result } => {
                    if self.auth.active_session_request_id != Some(request.request_id) {
                        return None;
                    }
                    let show_error = self.auth.local_admin_login_pending;
                    self.auth.active_session_request_id = None;
                    self.auth.local_admin_login_pending = false;
                    self.loading.session = false;
                    self.auth.checked = true;
                    match result {
                        Ok(session) => {
                            let account = session.account;
                            if self.auth.account.as_ref().map(|current| &current.user_id)
                                != Some(&account.user_id)
                            {
                                self.begin_import_epoch();
                                self.import = Default::default();
                            }
                            self.config.user_id = account.user_id.clone();
                            self.work.keybindings = labello_domain::KeybindingSet::defaults_for(
                                account.user_id.clone(),
                            );
                            self.auth.account = Some(account);
                            self.auth.can_create_datasets = session.can_create_datasets;
                            self.setup.section = SetupSection::Datasets;
                            self.runtime.error = None;
                            self.initialize_browser_workspace();
                            self.request_dataset_list();
                        }
                        Err(error) => {
                            if self.auth.account.is_some() {
                                self.begin_import_epoch();
                                self.import = Default::default();
                            }
                            self.auth.account = None;
                            self.auth.can_create_datasets = false;
                            self.datasets.summaries.clear();
                            self.datasets.summaries_error = None;
                            if show_error {
                                self.runtime.error = Some(error);
                            } else {
                                self.runtime.error = None;
                            }
                        }
                    }
                }
                UiMessage::LogoutFinished { result, .. } => {
                    self.loading.logout = false;
                    match result {
                        Ok(()) => {
                            self.clear_authenticated_state();
                            self.runtime.notice = Some("Signed out".to_string());
                            self.runtime.error = None;
                        }
                        Err(error) => self.runtime.error = Some(error),
                    }
                }
                UiMessage::GithubLoginUrl { result, .. } => match result {
                    Ok(url) => ctx.open_url(egui::OpenUrl::same_tab(url)),
                    Err(error) => self.runtime.error = Some(error),
                },
                UiMessage::DatasetList { result, .. } => {
                    self.loading.datasets = false;
                    match result {
                        Ok(datasets) => {
                            self.datasets.summaries = datasets;
                            self.datasets.summaries_error = None;
                            self.reopen_previous_workspace();
                        }
                        Err(error) => {
                            self.datasets.summaries_error = Some(error.clone());
                            self.runtime.error = Some(error);
                        }
                    }
                }
                UiMessage::DatasetCreated { result, .. } => match *result {
                    Ok(metadata) => {
                        self.loading.dataset = false;
                        if self.config.dataset_id != metadata.dataset_id {
                            self.loading.stats = false;
                            self.datasets.active_stats_request = None;
                            self.datasets.last_stats_attempt = None;
                            self.datasets.last_stats_completion = None;
                            self.datasets.stats_error = None;
                            self.datasets.stats = labello_domain::DatasetStats::default();
                        }
                        self.config.dataset_id = metadata.dataset_id.clone();
                        self.setup.create_dataset_id = metadata.dataset_id.to_string();
                        self.setup.create_dataset_name = metadata.name.clone();
                        self.upsert_dataset_summary(&metadata);
                        self.runtime.error = None;
                        self.datasets.requested_view = Some(AppView::Admin);
                        self.request_load_dataset();
                        self.request_dataset_list();
                    }
                    Err(error) => {
                        self.loading.dataset = false;
                        self.runtime.error = Some(error);
                    }
                },
                UiMessage::DatasetLoaded { result, .. } => {
                    self.loading.dataset = false;
                    match *result {
                        Ok(loaded) => {
                            self.upsert_dataset_summary(&loaded.metadata);
                            self.apply_loaded_dataset(loaded);
                        }
                        Err(error) => self.runtime.error = Some(error),
                    }
                }
                UiMessage::AdminLoaded { result, .. } => {
                    self.loading.admin = false;
                    match *result {
                        Ok(loaded) => {
                            self.sync_work_config(loaded.metadata.clone());
                            self.upsert_dataset_summary(&loaded.metadata);
                            self.datasets.admin_baseline = Some(loaded.metadata.clone());
                            self.datasets.admin_config = Some(loaded.metadata);
                            self.datasets.users_baseline = loaded.users.clone();
                            self.datasets.users = loaded.users;
                            if self.admin.dataset_id.as_ref() != Some(&self.config.dataset_id)
                            {
                                self.admin = Default::default();
                                self.admin.dataset_id = Some(self.config.dataset_id.clone());
                            }
                            self.admin.load_error = None;
                            self.view = AppView::Admin;
                            self.runtime.error = None;
                            self.request_admin_draft_load();
                            self.request_images();
                            if !self.admin.snapshots_loaded {
                                self.request_snapshots();
                            }
                        }
                        Err(error) => {
                            self.admin.load_error = Some(error.clone());
                            self.runtime.error = Some(error);
                        }
                    }
                }
                UiMessage::AdminSaved { request, result } => match *result {
                    Ok(metadata) => {
                        self.loading.admin = false;
                        self.sync_work_config(metadata.clone());
                        self.upsert_dataset_summary(&metadata);
                        self.datasets.admin_baseline = Some(metadata.clone());
                        self.datasets.admin_config = Some(metadata);
                        self.clear_admin_draft();
                        self.runtime.error = None;
                        self.request_next_admin_role_save();
                        self.assignment_availability_mutation_completed(
                            request
                                .dataset_id
                                .as_ref()
                                .expect("admin mutations are dataset-scoped"),
                            false,
                        );
                    }
                    Err(error) => {
                        self.loading.admin = false;
                        self.admin.pending_role_saves.clear();
                        self.runtime.error = Some(error);
                        self.assignment_availability_mutation_completed(
                            request
                                .dataset_id
                                .as_ref()
                                .expect("admin mutations are dataset-scoped"),
                            false,
                        );
                    }
                },
                UiMessage::DatasetRolesSaved { request, result } => {
                    self.loading.roles_user = None;
                    match result {
                        Ok(user) => {
                            replace_dataset_user(&mut self.datasets.users, user.clone());
                            replace_dataset_user(&mut self.datasets.users_baseline, user.clone());
                            self.sync_role_assignment(&user);
                            self.runtime.error = None;
                            self.request_next_admin_role_save();
                            self.assignment_availability_mutation_completed(
                                request
                                    .dataset_id
                                    .as_ref()
                                    .expect("role mutations are dataset-scoped"),
                                false,
                            );
                        }
                        Err(error) => {
                            self.admin.pending_role_saves.clear();
                            self.runtime.error = Some(error);
                            self.assignment_availability_mutation_completed(
                                request
                                    .dataset_id
                                    .as_ref()
                                    .expect("role mutations are dataset-scoped"),
                                false,
                            );
                        }
                    }
                }
                UiMessage::ImagesLoaded { result, .. } => {
                    self.loading.images = false;
                    match result {
                        Ok(page) => {
                            self.admin.image_query.page = page.page;
                            self.admin.images = Some(page);
                            self.admin.images_error = None;
                        }
                        Err(error) => self.admin.images_error = Some(error),
                    }
                }
                UiMessage::SnapshotsLoaded { result, .. } => {
                    self.loading.snapshots = false;
                    match result {
                        Ok(snapshots) => {
                            self.admin.snapshots = snapshots;
                            self.admin.snapshots_loaded = true;
                            self.admin.snapshots_error = None;
                        }
                        Err(error) => self.admin.snapshots_error = Some(error),
                    }
                }
                UiMessage::SnapshotCreated { result, .. } => {
                    self.loading.creating_snapshot = false;
                    match result {
                        Ok(snapshot) => {
                            self.admin
                                .snapshots
                                .retain(|existing| existing.snapshot_id != snapshot.snapshot_id);
                            self.admin.snapshots.insert(0, snapshot);
                            self.admin.snapshot_action_error = None;
                            self.request_snapshots();
                        }
                        Err(error) => self.admin.snapshot_action_error = Some(error),
                    }
                }
                UiMessage::SnapshotDownloaded { result, .. } => {
                    self.loading.snapshot_file = None;
                    match result {
                        Ok(file) => match crate::admin::download_snapshot_file(file) {
                            Ok(()) => self.admin.snapshot_action_error = None,
                            Err(error) => self.admin.snapshot_action_error = Some(error),
                        },
                        Err(error) => self.admin.snapshot_action_error = Some(error),
                    }
                }
            message => return Some(message),
        }
        None
    }
}
