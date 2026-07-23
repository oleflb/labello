use std::{future::Future, rc::Rc};

use eframe::egui;
use labello_client::{
    HttpLabelloApi, IngestJob, IngestJobStatus, OAuthLoginRequest, SetDatasetRolesRequest,
    UpdateDatasetConfigRequest,
};
use web_time::{Duration, Instant};

use crate::app::{
    AppView, LabelloApp, LoadedAdmin, LoadedDataset, LoadedImage, RequestIdentity, SaveStatus,
    UiCommand, UiMessage,
};

impl LabelloApp {
    pub(crate) fn rebuild_http_api(&mut self) {
        self.begin_auth_epoch();
        match HttpLabelloApi::new(&self.config.api_base_url) {
            Ok(api) => {
                self.runtime.api = Some(Rc::new(api));
                self.runtime.error = None;
            }
            Err(error) => {
                self.runtime.api = None;
                self.runtime.error = Some(error.to_string());
            }
        }
    }

    pub(crate) fn process_messages(&mut self, ctx: &egui::Context) {
        self.runtime.repaint_ctx = Some(ctx.clone());
        let mut processed = 0;
        for _ in 0..8 {
            let Ok(message) = self.runtime.rx.try_recv() else {
                break;
            };
            processed += 1;
            let requires_current_dataset = !matches!(&message, UiMessage::DatasetCreated { .. });
            if let Some(request) = message.request().cloned()
                && !self.finish_request(&request, requires_current_dataset)
            {
                if let Some(dataset_id) = request.dataset_id {
                    match message {
                        UiMessage::PrefetchLoaded { result, .. } => {
                            if let Ok(Some(loaded)) = *result {
                                self.release_reservation(dataset_id, loaded.assignment);
                            }
                        }
                        UiMessage::ImageLoaded {
                            assignment: Some(assignment),
                            ..
                        } => self.release_reservation(dataset_id, assignment),
                        _ => {}
                    }
                }
                continue;
            }
            match message {
                UiMessage::AuthOptionsLoaded { result, .. } => {
                    self.loading.session = false;
                    self.auth.options_checked = true;
                    match result {
                        Ok(options) => {
                            self.auth.options = options;
                            self.runtime.error = None;
                        }
                        Err(error) => {
                            self.auth.checked = true;
                            self.runtime.error = Some(error);
                        }
                    }
                }
                UiMessage::SessionLoaded { request, result } => {
                    if self.auth.active_session_request_id != Some(request.request_id) {
                        continue;
                    }
                    let show_error = self.auth.local_admin_login_pending;
                    self.auth.active_session_request_id = None;
                    self.auth.local_admin_login_pending = false;
                    self.loading.session = false;
                    self.auth.checked = true;
                    match result {
                        Ok(account) => {
                            self.config.user_id = account.user_id.clone();
                            self.work.keybindings = labello_domain::KeybindingSet::defaults_for(
                                account.user_id.clone(),
                            );
                            self.auth.account = Some(account);
                            self.runtime.error = None;
                            self.initialize_browser_workspace();
                            self.request_dataset_list();
                        }
                        Err(error) => {
                            self.auth.account = None;
                            self.datasets.summaries.clear();
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
                            self.auth.account = None;
                            self.datasets.summaries.clear();
                            self.datasets.metadata = None;
                            self.datasets.admin_config = None;
                            self.datasets.admin_baseline = None;
                            self.datasets.users.clear();
                            self.datasets.users_baseline.clear();
                            self.admin_tools = Default::default();
                            self.clear_current_image();
                            self.isolate_browser_workspace();
                            self.runtime.storage_error = None;
                            self.view = AppView::Setup;
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
                            self.runtime.error = None;
                            self.reopen_previous_workspace();
                        }
                        Err(error) => self.runtime.error = Some(error),
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
                            if self.admin_tools.dataset_id.as_ref() != Some(&self.config.dataset_id)
                            {
                                self.admin_tools = Default::default();
                                self.admin_tools.dataset_id = Some(self.config.dataset_id.clone());
                            }
                            self.view = AppView::Admin;
                            self.runtime.error = None;
                            self.request_admin_draft_load();
                            if self.admin_tools.images.is_none() {
                                self.request_images();
                            }
                            if !self.admin_tools.snapshots_loaded {
                                self.request_snapshots();
                            }
                        }
                        Err(error) => self.runtime.error = Some(error),
                    }
                }
                UiMessage::AdminSaved { result, .. } => match *result {
                    Ok(metadata) => {
                        self.loading.admin = false;
                        self.sync_work_config(metadata.clone());
                        self.upsert_dataset_summary(&metadata);
                        self.datasets.admin_baseline = Some(metadata.clone());
                        self.datasets.admin_config = Some(metadata);
                        self.clear_admin_draft();
                        self.runtime.notice = Some("Admin config saved".to_string());
                        self.runtime.error = None;
                    }
                    Err(error) => {
                        self.loading.admin = false;
                        self.runtime.error = Some(error);
                    }
                },
                UiMessage::DatasetRolesSaved { result, .. } => {
                    self.loading.roles_user = None;
                    match result {
                        Ok(user) => {
                            replace_dataset_user(&mut self.datasets.users, user.clone());
                            replace_dataset_user(&mut self.datasets.users_baseline, user.clone());
                            self.sync_role_assignment(&user);
                            self.runtime.notice = Some(format!(
                                "Permissions saved for {}",
                                user.account.display_name
                            ));
                            self.runtime.error = None;
                        }
                        Err(error) => self.runtime.error = Some(error),
                    }
                }
                UiMessage::ImagesLoaded { result, .. } => {
                    self.loading.images = false;
                    match result {
                        Ok(page) => {
                            self.admin_tools.image_query.page = page.page;
                            self.admin_tools.images = Some(page);
                            self.admin_tools.images_error = None;
                        }
                        Err(error) => self.admin_tools.images_error = Some(error),
                    }
                }
                UiMessage::SnapshotsLoaded { result, .. } => {
                    self.loading.snapshots = false;
                    self.admin_tools.snapshots_loaded = true;
                    match result {
                        Ok(snapshots) => {
                            self.admin_tools.snapshots = snapshots;
                            self.admin_tools.snapshots_error = None;
                        }
                        Err(error) => self.admin_tools.snapshots_error = Some(error),
                    }
                }
                UiMessage::SnapshotCreated { result, .. } => {
                    self.loading.creating_snapshot = false;
                    match result {
                        Ok(snapshot) => {
                            self.admin_tools
                                .snapshots
                                .retain(|existing| existing.snapshot_id != snapshot.snapshot_id);
                            self.admin_tools.snapshots.insert(0, snapshot);
                            self.admin_tools.snapshots_loaded = true;
                            self.admin_tools.snapshots_error = None;
                        }
                        Err(error) => self.admin_tools.snapshots_error = Some(error),
                    }
                }
                UiMessage::SnapshotDownloaded { result, .. } => {
                    self.loading.snapshot_file = None;
                    match result {
                        Ok(file) => match crate::admin::download_snapshot_file(file) {
                            Ok(()) => self.admin_tools.snapshots_error = None,
                            Err(error) => self.admin_tools.snapshots_error = Some(error),
                        },
                        Err(error) => self.admin_tools.snapshots_error = Some(error),
                    }
                }
                UiMessage::ImageLoaded {
                    request: _,
                    operation_id,
                    assignment,
                    result,
                } => {
                    if self.active_load_id != Some(operation_id) {
                        continue;
                    }
                    self.active_load_id = None;
                    self.loading.image = false;
                    match *result {
                        Ok(Some(loaded)) => {
                            self.one_shot_excluded_image_id = None;
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
                        }
                        Ok(None) => {
                            self.one_shot_excluded_image_id = None;
                            self.runtime.persistence.expected_assignment = None;
                            self.assignment = None;
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
                        }
                        Err(error) => {
                            if assignment.is_some() {
                                self.one_shot_excluded_image_id = None;
                            }
                            self.runtime.persistence.expected_assignment = None;
                            self.assignment = assignment;
                            self.runtime.error = Some(error);
                        }
                    }
                }
                UiMessage::PrefetchLoaded {
                    request: _,
                    operation_id,
                    result,
                } => {
                    if self.active_prefetch_id != Some(operation_id) {
                        continue;
                    }
                    self.active_prefetch_id = None;
                    self.queue.set_loading(false);
                    match *result {
                        Ok(Some(loaded))
                            if loaded.assignment.kind
                                == labello_domain::AssignmentKind::Annotation
                                && loaded.assignment.status
                                    == labello_domain::AssignmentStatus::Active
                                && loaded.assignment.expires_at.is_none_or(|expires_at| {
                                    expires_at > labello_domain::now()
                                })
                                && self.assignment.as_ref().is_some_and(|current| {
                                    current.task_id == loaded.assignment.task_id
                                        && current.image_id != loaded.assignment.image_id
                                })
                                && !self
                                    .queue
                                    .prepared_image_ids()
                                    .contains(&loaded.assignment.image_id) =>
                        {
                            self.one_shot_excluded_image_id = None;
                            self.queue.clear_failure();
                            self.queue.push_prepared(loaded);
                            self.request_prefetch();
                        }
                        Ok(Some(loaded)) => {
                            self.one_shot_excluded_image_id = None;
                            self.release_reservation(
                                self.config.dataset_id.clone(),
                                loaded.assignment,
                            );
                        }
                        Ok(None) => {
                            self.one_shot_excluded_image_id = None;
                            self.queue.clear_failure();
                        }
                        Err(_) => {
                            self.queue.mark_failed();
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
                    }
                }
                UiMessage::SaveFinished {
                    request: _,
                    operation_id,
                    assignment_id,
                    edit_generation,
                    completed,
                    result,
                } => {
                    if !self.matches_operation(operation_id, &assignment_id) {
                        continue;
                    }
                    self.active_operation_id = None;
                    self.loading.saving = false;
                    match *result {
                        Ok(state) => {
                            if self.edit_generation == edit_generation {
                                if let Some(assignment) = self.assignment.as_ref() {
                                    let assignment = assignment.clone();
                                    self.clear_current_work_draft(&assignment);
                                }
                                self.apply_state(state);
                                self.save_status = SaveStatus::Saved;
                            } else {
                                self.renew_assignment_from_state(&state);
                                self.persisted_annotations =
                                    state.annotations.keys().cloned().collect();
                                self.current_state = Some(state);
                                self.recompute_modified_annotations();
                                self.save_status = SaveStatus::Dirty;
                                self.rebase_work_draft_after_save(edit_generation);
                            }
                            self.runtime.error = None;
                            self.request_stats();
                            if completed {
                                self.finish_annotation_transition(ctx, None);
                            }
                        }
                        Err(error) => {
                            self.save_status = if self.edit_generation == edit_generation {
                                SaveStatus::Retry
                            } else {
                                SaveStatus::Dirty
                            };
                            if completed {
                                self.pending_transition = None;
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
                        continue;
                    }
                    self.active_operation_id = None;
                    self.loading.saving = false;
                    match result {
                        Ok(()) => {
                            let released_image_id = self
                                .assignment
                                .as_ref()
                                .map(|assignment| assignment.image_id.clone());
                            if let Some(assignment) = self.assignment.clone() {
                                self.clear_current_work_draft(&assignment);
                            }
                            self.runtime.error = None;
                            self.finish_annotation_transition(ctx, released_image_id);
                        }
                        Err(error) => {
                            self.pending_transition = None;
                            self.runtime.error = Some(error);
                        }
                    }
                }
                UiMessage::ReviewFinished {
                    request: _,
                    operation_id,
                    assignment_id,
                    phase,
                    decision,
                    result,
                } => {
                    if !self.matches_operation(operation_id, &assignment_id) {
                        continue;
                    }
                    self.active_operation_id = None;
                    self.loading.saving = false;
                    match *result {
                        Ok(state) => {
                            let completed_assignment = self.assignment.clone();
                            self.runtime.error = None;
                            self.apply_state(state);
                            self.review_index = self
                                .selected_task_id
                                .as_ref()
                                .map(|task_id| {
                                    crate::persistence::reviewed_object_prefix(
                                        self.current_state.as_ref().expect("state was applied"),
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
                                    self.review_rejected = true;
                                    self.request_full_image_review(
                                        labello_domain::ReviewDecision::Rejected,
                                    );
                                }
                                crate::app::ReviewPhase::FullImage => {
                                    self.request_stats();
                                    self.clear_current_image();
                                    self.execute_transition(
                                        crate::app::PendingTransition::NextAssignment,
                                    );
                                }
                            }
                        }
                        Err(error) => {
                            self.pending_transition = None;
                            self.runtime.error = Some(error);
                        }
                    }
                }
                UiMessage::CorrectionFinished {
                    request: _,
                    operation_id,
                    assignment_id,
                    result,
                } => {
                    if !self.matches_operation(operation_id, &assignment_id) {
                        continue;
                    }
                    self.active_operation_id = None;
                    self.loading.saving = false;
                    match result {
                        Ok(()) => {
                            if let Some(assignment) = self.assignment.clone() {
                                self.clear_current_work_draft(&assignment);
                            }
                            self.runtime.error = None;
                            self.request_stats();
                            self.clear_current_image();
                            self.execute_transition(crate::app::PendingTransition::NextAssignment);
                        }
                        Err(error) => {
                            self.pending_transition = None;
                            self.runtime.error = Some(error);
                        }
                    }
                }
                UiMessage::AdjudicationFinished {
                    request: _,
                    operation_id,
                    assignment_id,
                    result,
                } => {
                    if !self.matches_operation(operation_id, &assignment_id) {
                        continue;
                    }
                    self.active_operation_id = None;
                    self.loading.saving = false;
                    match result {
                        Ok(()) => {
                            self.runtime.error = None;
                            self.request_stats();
                            self.clear_current_image();
                            self.execute_transition(crate::app::PendingTransition::NextAssignment);
                        }
                        Err(error) => {
                            self.pending_transition = None;
                            self.runtime.error = Some(error);
                        }
                    }
                }
                UiMessage::IngestJobLoaded { result, .. } => self.handle_ingest_job(result),
                UiMessage::StatsLoaded { request, result } => {
                    let Some(dataset_id) = request.dataset_id else {
                        continue;
                    };
                    if !self.datasets.active_stats_request.as_ref().is_some_and(
                        |(active_request_id, active_dataset_id)| {
                            *active_request_id == request.request_id
                                && active_dataset_id == &dataset_id
                        },
                    ) || self.config.dataset_id != dataset_id
                    {
                        continue;
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
                UiMessage::KeybindingsSaved { result, .. } => {
                    self.loading.keybindings = false;
                    match result {
                        Ok(keybindings) => {
                            self.keybindings = keybindings;
                            if self.show_settings {
                                self.shortcut_settings.baseline = Some(self.keybindings.clone());
                                self.shortcut_settings.draft = Some(self.keybindings.clone());
                            }
                            self.runtime.notice = Some("Keyboard shortcuts saved".to_string());
                            self.runtime.error = None;
                        }
                        Err(error) => self.runtime.error = Some(error),
                    }
                }
                UiMessage::RequestFailed { error, .. } => {
                    self.invalidate_async_ownership();
                    self.runtime.error = Some(error);
                }
                UiMessage::FolderUploadProgress(progress) => {
                    self.loading.uploading = true;
                    self.loading.upload_progress = Some(progress);
                    self.runtime.error = None;
                    ctx.request_repaint();
                }
                UiMessage::FolderUploadFinished(result) => {
                    self.loading.uploading = false;
                    self.loading.upload_progress = None;
                    match result {
                        Ok(message) => {
                            self.runtime.notice = Some(message);
                            self.runtime.error = None;
                            self.request_admin_dataset();
                            self.request_dataset_list();
                        }
                        Err(error) => self.runtime.error = Some(error),
                    }
                }
                UiMessage::PersistenceFinished(completion) => {
                    self.handle_persistence_completion(*completion);
                }
            }
        }
        if processed == 8 {
            ctx.request_repaint();
        }
    }

    pub(crate) fn start_next_command(&mut self) {
        let Some(command) = self.runtime.commands.pop_front() else {
            return;
        };
        let Some(api) = self.runtime.api.clone() else {
            self.rollback_command(&command, "API is not configured");
            return;
        };
        match command {
            UiCommand::AuthOptions { request } => self.spawn_message(request.clone(), async move {
                UiMessage::AuthOptionsLoaded {
                    request,
                    result: api.auth_options().await.map_err(|error| error.to_string()),
                }
            }),
            UiCommand::Session { request } => self.spawn_message(request.clone(), async move {
                UiMessage::SessionLoaded {
                    request,
                    result: api.me().await.map_err(|error| error.to_string()),
                }
            }),
            UiCommand::LocalAdminLogin { request } => {
                self.spawn_message(request.clone(), async move {
                    UiMessage::SessionLoaded {
                        request,
                        result: api
                            .local_admin_login()
                            .await
                            .map_err(|error| error.to_string()),
                    }
                })
            }
            UiCommand::Logout { request } => self.spawn_message(request.clone(), async move {
                UiMessage::LogoutFinished {
                    request,
                    result: api.logout().await.map_err(|error| error.to_string()),
                }
            }),
            UiCommand::GithubLogin { request, return_to } => {
                self.spawn_message(request.clone(), async move {
                    UiMessage::GithubLoginUrl {
                        request,
                        result: api
                            .github_login_url(OAuthLoginRequest { return_to })
                            .await
                            .map_err(|error| error.to_string()),
                    }
                })
            }
            UiCommand::DatasetList { request } => self.spawn_message(request.clone(), async move {
                UiMessage::DatasetList {
                    request,
                    result: api.list_datasets().await.map_err(|error| error.to_string()),
                }
            }),
            UiCommand::CreateDataset {
                request,
                dataset_id,
                name,
                admin_user_id,
            } => self.spawn_message(request.clone(), async move {
                UiMessage::DatasetCreated {
                    request,
                    result: Box::new(
                        api.create_dataset(labello_client::CreateDatasetRequest {
                            dataset_id,
                            name,
                            admin_user_id,
                        })
                        .await
                        .map_err(|error| error.to_string()),
                    ),
                }
            }),
            UiCommand::LoadDataset {
                request,
                dataset_id,
                user_id,
            } => self.spawn_message(request.clone(), async move {
                let result = async {
                    let metadata = api.get_dataset(&dataset_id).await?;
                    let keybindings = api.get_keybindings(&dataset_id, &user_id).await?;
                    Ok::<_, labello_client::ClientError>(LoadedDataset {
                        metadata,
                        keybindings,
                    })
                }
                .await
                .map_err(|error| error.to_string());
                UiMessage::DatasetLoaded {
                    request,
                    result: Box::new(result),
                }
            }),
            UiCommand::LoadAdmin {
                request,
                dataset_id,
            } => self.spawn_message(request.clone(), async move {
                let result = async {
                    let metadata = api.get_admin_dataset(&dataset_id).await?;
                    let users = api.list_dataset_users(&dataset_id).await?;
                    Ok::<_, labello_client::ClientError>(LoadedAdmin { metadata, users })
                }
                .await
                .map_err(|error| error.to_string());
                UiMessage::AdminLoaded {
                    request,
                    result: Box::new(result),
                }
            }),
            UiCommand::SaveAdmin { request, metadata } => {
                let dataset_id = metadata.dataset_id.clone();
                let update = UpdateDatasetConfigRequest::from_metadata(&metadata);
                self.spawn_message(request.clone(), async move {
                    UiMessage::AdminSaved {
                        request,
                        result: Box::new(
                            api.update_dataset_config(&dataset_id, update)
                                .await
                                .map_err(|error| error.to_string()),
                        ),
                    }
                });
            }
            UiCommand::SaveDatasetRoles {
                request,
                dataset_id,
                user_id,
                roles,
            } => self.spawn_message(request.clone(), async move {
                UiMessage::DatasetRolesSaved {
                    request,
                    result: api
                        .set_dataset_roles(&dataset_id, SetDatasetRolesRequest { user_id, roles })
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::LoadImages {
                request,
                dataset_id,
                query,
            } => self.spawn_message(request.clone(), async move {
                UiMessage::ImagesLoaded {
                    request,
                    result: api
                        .list_images(&dataset_id, query)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::LoadSnapshots {
                request,
                dataset_id,
            } => self.spawn_message(request.clone(), async move {
                UiMessage::SnapshotsLoaded {
                    request,
                    result: api
                        .list_snapshots(&dataset_id)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::CreateSnapshot {
                request,
                dataset_id,
            } => self.spawn_message(request.clone(), async move {
                UiMessage::SnapshotCreated {
                    request,
                    result: api
                        .create_snapshot(&dataset_id)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::DownloadSnapshot {
                request,
                dataset_id,
                snapshot_id,
                path,
            } => self.spawn_message(request.clone(), async move {
                UiMessage::SnapshotDownloaded {
                    request,
                    result: api
                        .get_snapshot_file(&dataset_id, &snapshot_id, &path)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::Ingest {
                request,
                dataset_id,
            } => self.spawn_message(request.clone(), async move {
                UiMessage::IngestJobLoaded {
                    request,
                    result: api
                        .start_ingest_job(&dataset_id)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::PollIngest {
                request,
                dataset_id,
                job_id,
            } => self.spawn_message(request.clone(), async move {
                UiMessage::IngestJobLoaded {
                    request,
                    result: api
                        .get_ingest_job(&dataset_id, &job_id)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::Stats {
                request,
                dataset_id,
            } => self.spawn_message(request.clone(), async move {
                let result = api
                    .dataset_stats(&dataset_id)
                    .await
                    .map_err(|error| error.to_string());
                UiMessage::StatsLoaded { request, result }
            }),
            UiCommand::SaveKeybindings {
                request,
                dataset_id,
                keybindings,
            } => self.spawn_message(request.clone(), async move {
                UiMessage::KeybindingsSaved {
                    request,
                    result: api
                        .save_keybindings(&dataset_id, keybindings)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            command => self.start_workflow_command(api, command),
        }
    }

    pub(crate) fn start_setup_load(&mut self) {
        if self.runtime.api.is_some() && !self.auth.options_checked && !self.loading.session {
            self.request_auth_options();
        } else if self.runtime.api.is_some()
            && self.auth.options_checked
            && !self.auth.checked
            && !self.loading.session
        {
            self.request_session();
        }
    }

    pub(crate) fn request_auth_options(&mut self) {
        if self.runtime.api.is_none() {
            return;
        }
        self.begin_auth_epoch();
        let request = self.request_identity(None);
        self.auth.options = labello_client::AuthOptions {
            github_oauth: false,
            local_admin_login: false,
        };
        self.auth.options_checked = false;
        self.auth.checked = false;
        self.loading.session = true;
        self.queue_command(UiCommand::AuthOptions { request });
    }

    pub(crate) fn request_logout(&mut self) {
        if self.loading.logout || self.runtime.api.is_none() {
            return;
        }
        self.begin_auth_epoch();
        self.loading.logout = true;
        let request = self.request_identity(None);
        self.queue_command(UiCommand::Logout { request });
    }

    pub(crate) fn request_github_login(&mut self) {
        if self.runtime.api.is_some() {
            let request = self.request_identity(None);
            self.queue_command(UiCommand::GithubLogin {
                request,
                return_to: self.config.application_url.clone(),
            });
        }
    }

    pub(crate) fn request_session(&mut self) {
        if self.runtime.api.is_none() {
            return;
        }
        self.begin_auth_epoch();
        let request = self.request_identity(None);
        self.auth.session_request_id = request.request_id;
        self.auth.active_session_request_id = Some(request.request_id);
        self.auth.local_admin_login_pending = false;
        self.auth.checked = false;
        self.loading.session = true;
        self.queue_command(UiCommand::Session { request });
    }

    pub(crate) fn request_local_admin_login(&mut self) {
        if self.loading.session || self.runtime.api.is_none() {
            return;
        }
        self.begin_auth_epoch();
        let request = self.request_identity(None);
        self.auth.session_request_id = request.request_id;
        self.auth.active_session_request_id = Some(request.request_id);
        self.auth.local_admin_login_pending = true;
        self.auth.checked = false;
        self.loading.session = true;
        self.queue_command(UiCommand::LocalAdminLogin { request });
    }

    pub(crate) fn refresh_stats_if_due(&mut self) {
        if self.runtime.api.is_none()
            || self.datasets.metadata.is_none()
            || self.loading.stats
            || matches!(self.view, AppView::Setup)
            || !matches!(self.view, AppView::Stats)
        {
            return;
        }
        let due = self
            .datasets
            .last_stats_attempt
            .is_none_or(|last| last.elapsed() >= Duration::from_secs(3));
        if due {
            self.request_stats();
        }
    }

    pub(crate) fn refresh_ingest_if_due(&mut self) {
        if !self.loading.ingesting || self.loading.ingest_polling {
            return;
        }
        let Some(job_id) = self.loading.ingest_job_id.clone() else {
            return;
        };
        let due = self
            .loading
            .last_ingest_poll
            .is_none_or(|last| last.elapsed() >= Duration::from_millis(500));
        if due {
            self.loading.ingest_polling = true;
            let request = self.request_identity(Some(self.config.dataset_id.clone()));
            self.queue_command(UiCommand::PollIngest {
                request,
                dataset_id: self.config.dataset_id.clone(),
                job_id,
            });
        }
    }

    pub(crate) fn queue_command(&mut self, command: UiCommand) -> bool {
        if self.runtime.commands.len() < 64 {
            self.runtime
                .active_requests
                .insert(command.request().request_id);
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

    fn rollback_command(&mut self, command: &UiCommand, error: &str) {
        self.runtime
            .active_requests
            .remove(&command.request().request_id);
        match command {
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
            UiCommand::DatasetList { .. } => self.loading.datasets = false,
            UiCommand::CreateDataset { .. } | UiCommand::LoadDataset { .. } => {
                self.loading.dataset = false
            }
            UiCommand::LoadAdmin { .. } | UiCommand::SaveAdmin { .. } => self.loading.admin = false,
            UiCommand::SaveDatasetRoles { .. } => self.loading.roles_user = None,
            UiCommand::LoadImages { .. } => self.loading.images = false,
            UiCommand::LoadSnapshots { .. } => self.loading.snapshots = false,
            UiCommand::CreateSnapshot { .. } => self.loading.creating_snapshot = false,
            UiCommand::DownloadSnapshot { .. } => self.loading.snapshot_file = None,
            UiCommand::Ingest { .. } => {
                self.loading.ingesting = false;
                self.loading.ingest_polling = false;
                self.loading.ingest_job_id = None;
            }
            UiCommand::PollIngest { .. } => self.loading.ingest_polling = false,
            UiCommand::Stats { .. } => {
                self.loading.stats = false;
                self.datasets.active_stats_request = None;
            }
            UiCommand::SaveKeybindings { .. } => self.loading.keybindings = false,
            UiCommand::ClaimAssignment { operation_id, .. }
            | UiCommand::ReloadAssignment { operation_id, .. } => {
                if self.active_load_id == Some(*operation_id) {
                    self.active_load_id = None;
                    self.loading.image = false;
                }
            }
            UiCommand::PrefetchAssignment { operation_id, .. } => {
                if self.active_prefetch_id == Some(*operation_id) {
                    self.active_prefetch_id = None;
                    self.queue.set_loading(false);
                    self.queue.mark_failed();
                }
            }
            UiCommand::ReleaseReservation { .. } => {}
            UiCommand::SaveAnnotations { operation_id, .. }
            | UiCommand::ReleaseAssignment { operation_id, .. }
            | UiCommand::Review { operation_id, .. }
            | UiCommand::Correction { operation_id, .. }
            | UiCommand::Adjudication { operation_id, .. } => {
                if self.active_operation_id == Some(*operation_id) {
                    self.active_operation_id = None;
                    self.loading.saving = false;
                    self.pending_transition = None;
                    if matches!(command, UiCommand::SaveAnnotations { .. }) {
                        self.save_status = SaveStatus::Retry;
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
        request.auth_epoch == self.auth_epoch
            && request.workspace_epoch == self.workspace_epoch
            && (!requires_current_dataset
                || request
                    .dataset_id
                    .as_ref()
                    .is_none_or(|dataset_id| dataset_id == &self.config.dataset_id))
            && self.runtime.active_requests.remove(&request.request_id)
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
        self.loading.image = false;
        self.loading.saving = false;
        self.loading.ingesting = false;
        self.loading.ingest_polling = false;
        self.loading.ingest_job_id = None;
        self.loading.last_ingest_poll = None;
        self.loading.stats = false;
        self.loading.keybindings = false;
        self.loading.images = false;
        self.loading.snapshots = false;
        self.loading.creating_snapshot = false;
        self.loading.snapshot_file = None;
        self.active_load_id = None;
        self.active_prefetch_id = None;
        self.active_operation_id = None;
        self.queue.set_loading(false);
        self.release_prepared_assignments();
        self.one_shot_excluded_image_id = None;
        if self.save_status == SaveStatus::Saving {
            self.save_status = SaveStatus::Retry;
        }
    }

    pub(crate) fn begin_auth_epoch(&mut self) {
        self.auth_epoch = self.auth_epoch.wrapping_add(1);
        self.workspace_epoch = self.workspace_epoch.wrapping_add(1);
        self.invalidate_async_ownership();
    }

    pub(crate) fn begin_workspace_epoch(&mut self) {
        self.workspace_epoch = self.workspace_epoch.wrapping_add(1);
        self.invalidate_async_ownership();
    }

    pub(crate) fn request_dataset_list(&mut self) {
        if self.loading.datasets || self.runtime.api.is_none() {
            return;
        }
        self.loading.datasets = true;
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
        if self.loading.admin || self.runtime.api.is_none() {
            return;
        }
        self.loading.admin = true;
        let request = self.request_identity(Some(self.config.dataset_id.clone()));
        self.queue_command(UiCommand::LoadAdmin {
            request,
            dataset_id: self.config.dataset_id.clone(),
        });
    }

    pub(crate) fn request_images(&mut self) {
        if self.loading.images || self.runtime.api.is_none() {
            return;
        }
        self.admin_tools.image_query.search = non_empty(&self.admin_tools.image_search);
        self.admin_tools.image_query.task_id = self.admin_tools.image_task.clone();
        self.admin_tools.image_query.class_id = self.admin_tools.image_class.clone();
        self.admin_tools.image_query.status = self.admin_tools.image_status.clone();
        self.loading.images = true;
        self.admin_tools.images_error = None;
        let request = self.request_identity(Some(self.config.dataset_id.clone()));
        self.queue_command(UiCommand::LoadImages {
            request,
            dataset_id: self.config.dataset_id.clone(),
            query: self.admin_tools.image_query.clone(),
        });
    }

    pub(crate) fn request_snapshots(&mut self) {
        if self.loading.snapshots || self.runtime.api.is_none() {
            return;
        }
        self.loading.snapshots = true;
        self.admin_tools.snapshots_error = None;
        let request = self.request_identity(Some(self.config.dataset_id.clone()));
        self.queue_command(UiCommand::LoadSnapshots {
            request,
            dataset_id: self.config.dataset_id.clone(),
        });
    }

    pub(crate) fn request_snapshot_create(&mut self) {
        if self.loading.creating_snapshot || self.runtime.api.is_none() {
            return;
        }
        self.loading.creating_snapshot = true;
        self.admin_tools.snapshots_error = None;
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
        self.admin_tools.snapshots_error = None;
        let request = self.request_identity(Some(self.config.dataset_id.clone()));
        self.queue_command(UiCommand::DownloadSnapshot {
            request,
            dataset_id: self.config.dataset_id.clone(),
            snapshot_id,
            path,
        });
    }

    pub(crate) fn request_admin_save(&mut self) {
        let Some(metadata) = self.datasets.admin_config.clone() else {
            return;
        };
        if self.loading.admin || self.loading.roles_user.is_some() {
            return;
        }
        self.loading.admin = true;
        let request = self.request_identity(Some(self.config.dataset_id.clone()));
        self.queue_command(UiCommand::SaveAdmin { request, metadata });
    }

    pub(crate) fn request_role_save(&mut self, user_id: labello_domain::UserId) {
        if self.loading.admin || self.loading.roles_user.is_some() || self.runtime.api.is_none() {
            return;
        }
        let Some(user) = self
            .datasets
            .users
            .iter()
            .find(|user| user.account.user_id == user_id)
        else {
            return;
        };
        let removes_admin = !user.roles.contains(&labello_domain::DatasetRole::DataAdmin);
        if user_id == self.config.user_id && removes_admin {
            self.runtime.error = Some("You cannot remove your own data admin role.".to_string());
            return;
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
            return;
        }
        let roles = user.roles.clone();
        self.loading.roles_user = Some(user_id.clone());
        let request = self.request_identity(Some(self.config.dataset_id.clone()));
        self.queue_command(UiCommand::SaveDatasetRoles {
            request,
            dataset_id: self.config.dataset_id.clone(),
            user_id,
            roles,
        });
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
            .shortcut_settings
            .draft
            .clone()
            .unwrap_or_else(|| self.keybindings.clone());
        if let Err(error) = keybindings.validate() {
            self.runtime.error = Some(error.to_string());
            return;
        }
        self.loading.keybindings = true;
        let request = self.request_identity(Some(self.config.dataset_id.clone()));
        self.queue_command(UiCommand::SaveKeybindings {
            request,
            dataset_id: self.config.dataset_id.clone(),
            keybindings,
        });
    }

    fn apply_loaded_dataset(&mut self, loaded: LoadedDataset) {
        self.clear_current_image();
        self.sync_work_config(loaded.metadata);
        self.keybindings = loaded.keybindings;
        self.keybindings.normalize();
        self.shortcut_settings = Default::default();
        let requested = self.datasets.requested_view.take().unwrap_or_else(|| {
            [AppView::Annotate, AppView::Review, AppView::Adjudicate]
                .into_iter()
                .find(|view| self.can_open_view(*view))
                .unwrap_or(AppView::Stats)
        });
        if !self.can_open_view(requested) {
            self.runtime.error = Some(format!(
                "The current user is not authorized for {}.",
                view_label(requested)
            ));
            self.view = AppView::Setup;
            return;
        }
        if self.ensure_valid_task_selection() {
            self.runtime.error = None;
        } else {
            self.runtime.error = Some(
                "No enabled one-class workflow is configured. Ask a data admin to enable one."
                    .to_string(),
            );
        }
        if requested == AppView::Admin {
            self.request_admin_dataset();
        } else {
            self.view = requested;
            if self.work_view() && self.selected_task().is_some() {
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
                    if self.work_view() && self.current.is_none() {
                        self.request_next_image();
                    }
                }
                IngestJobStatus::Failed => {
                    self.loading.ingesting = false;
                    self.loading.ingest_job_id = None;
                    self.loading.last_ingest_poll = None;
                    self.runtime.error =
                        Some(job.error.unwrap_or_else(|| "ingest failed".to_string()));
                }
            },
            Err(error) => {
                self.loading.ingesting = false;
                self.loading.ingest_job_id = None;
                self.loading.last_ingest_poll = None;
                self.runtime.error = Some(error);
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
        self.assignment = Some(loaded.assignment);
        self.current = Some(loaded.queued);
        self.current_state = Some(loaded.state.clone());
        self.annotations = loaded.annotations;
        self.persisted_annotations = self
            .annotations
            .iter()
            .map(|annotation| annotation.annotation_id.clone())
            .collect();
        self.modified_annotations.clear();
        self.accepted_prelabels.clear();
        self.selected_prelabel = None;
        self.selected_annotation = None;
        self.active_skeleton = None;
        self.skeleton_keypoint_index = 0;
        self.next_keypoint_hidden = false;
        self.review_index = 0;
        self.review_rejected = false;
        self.correction_draft = None;
        self.save_status = SaveStatus::Idle;
        self.edit_generation = 0;
        self.last_edit_at = None;
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.canvas.fit_view();
        self.runtime.persistence.work_ready = None;
        self.current_texture = loaded.color_image.map(|image| {
            ctx.load_texture(
                format!("image-{image_id}"),
                image,
                egui::TextureOptions::LINEAR,
            )
        });
        if self.view == AppView::Review {
            self.review_index = self
                .selected_task_id
                .as_ref()
                .map(|task_id| {
                    crate::persistence::reviewed_object_prefix(
                        &loaded.state,
                        task_id,
                        &self.config.user_id,
                    )
                })
                .unwrap_or(0);
        }
        self.apply_assignment_preferences();
        self.sync_review_selection();
        if let Some(state) = self.current_state.clone() {
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
        let transition = self.pending_transition.take();
        if self.view == AppView::Annotate
            && transition == Some(crate::app::PendingTransition::NextAssignment)
        {
            self.one_shot_excluded_image_id = released_image_id;
            while let Some(loaded) = self.queue.pop_prepared() {
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
            self.clear_current_image();
            self.request_next_image();
            return;
        }
        if let Some(transition) = transition {
            self.execute_transition(transition);
        } else {
            self.clear_current_image();
        }
    }

    fn apply_state(&mut self, state: labello_domain::ImageState) {
        self.renew_assignment_from_state(&state);
        self.annotations = state.active_annotations().cloned().collect();
        self.persisted_annotations = self
            .annotations
            .iter()
            .map(|annotation| annotation.annotation_id.clone())
            .collect();
        self.current_state = Some(state);
        self.modified_annotations.clear();
    }

    pub(crate) fn sync_review_selection(&mut self) {
        if self.view != AppView::Review {
            return;
        }
        let selected = self
            .current_review_annotation()
            .map(|annotation| annotation.annotation_id.clone());
        self.selected_annotation = selected;
        if self
            .correction_draft
            .as_ref()
            .is_some_and(|draft| self.selected_annotation.as_ref() != Some(&draft.annotation_id))
        {
            self.correction_draft = None;
        }
    }

    fn renew_assignment_from_state(&mut self, state: &labello_domain::ImageState) {
        let Some(current) = self.assignment.as_ref() else {
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
            self.assignment = Some(renewed.clone());
        }
    }

    fn matches_operation(
        &self,
        operation_id: u64,
        assignment_id: &labello_domain::AssignmentId,
    ) -> bool {
        self.active_operation_id == Some(operation_id)
            && self
                .assignment
                .as_ref()
                .is_some_and(|assignment| assignment.assignment_id == *assignment_id)
    }

    pub(crate) fn spawn_message<F>(&self, _request: RequestIdentity, future: F)
    where
        F: Future<Output = UiMessage> + 'static,
    {
        let tx = self.runtime.tx.clone();
        let repaint_ctx = self.runtime.repaint_ctx.clone();
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(async move {
            let _ = tx.send(future.await);
            if let Some(ctx) = repaint_ctx {
                ctx.request_repaint();
            }
        });
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(spawn) = &self.runtime.native_task_spawner {
                spawn(Box::pin(async move {
                    let _ = tx.send(future.await);
                    if let Some(ctx) = repaint_ctx {
                        ctx.request_repaint();
                    }
                }));
                return;
            }
        }
        #[cfg(all(not(target_arch = "wasm32"), test))]
        {
            let _ = tx.send(poll_ready(future));
            if let Some(ctx) = repaint_ctx {
                ctx.request_repaint();
            }
        }
        #[cfg(all(not(target_arch = "wasm32"), not(test)))]
        {
            drop(future);
            let _ = tx.send(UiMessage::RequestFailed {
                request: _request,
                error: "live HTTP UI is available in the WASM build".to_string(),
            });
            if let Some(ctx) = repaint_ctx {
                ctx.request_repaint();
            }
        }
    }
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn replace_dataset_user(
    users: &mut Vec<labello_client::DatasetUser>,
    updated: labello_client::DatasetUser,
) {
    if let Some(user) = users
        .iter_mut()
        .find(|user| user.account.user_id == updated.account.user_id)
    {
        *user = updated;
    } else {
        users.push(updated);
    }
}

fn view_label(view: AppView) -> &'static str {
    match view {
        AppView::Setup => "setup",
        AppView::Annotate => "annotation",
        AppView::Review => "review",
        AppView::Adjudicate => "adjudication",
        AppView::Admin => "administration",
        AppView::Stats => "statistics",
    }
}

#[cfg(all(not(target_arch = "wasm32"), test))]
fn poll_ready<F>(future: F) -> UiMessage
where
    F: Future<Output = UiMessage> + 'static,
{
    use std::{
        pin::Pin,
        task::{Context, Poll, Waker},
    };

    let mut future = Pin::from(Box::new(future));
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(message) => message,
        Poll::Pending => panic!("test fake API future did not complete immediately"),
    }
}
