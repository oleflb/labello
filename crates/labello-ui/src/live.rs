use std::{future::Future, rc::Rc};

use eframe::egui;
use labello_client::{
    HttpLabelloApi, IngestJob, IngestJobStatus, OAuthLoginRequest, SetDatasetRolesRequest,
    UpdateDatasetConfigRequest,
};
use web_time::{Duration, Instant};

use crate::app::{
    AppView, ImportRequestIdentity, LabelloApp, LoadedAdmin, LoadedDataset, LoadedImage,
    RequestIdentity, SaveStatus, SetupSection, UiCommand, UiMessage,
};

impl LabelloApp {
    pub(crate) fn rebuild_http_api(&mut self) {
        self.begin_auth_epoch();
        self.begin_import_epoch();
        self.import_flow = Default::default();
        let api = HttpLabelloApi::new(&self.config.api_base_url).and_then(|api| {
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(application_url) = &self.config.application_url {
                return api.with_origin(application_url);
            }
            Ok(api)
        });
        match api {
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
            if let Some(request) = message.import_request().cloned()
                && !self.finish_import_request(&request)
            {
                continue;
            }
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
                        UiMessage::PreviousAssignmentLoaded {
                            assignment: Some(assignment),
                            ..
                        } => self.release_reservation(dataset_id, assignment),
                        _ => {}
                    }
                }
                continue;
            }
            match message {
                UiMessage::ImportCapabilitiesLoaded { result, .. } => {
                    self.import_flow.capabilities_loading = false;
                    match result {
                        Ok(capabilities) => {
                            self.import_flow
                                .normalize_capability_selection(&capabilities);
                            self.import_flow.capabilities = Some(capabilities);
                            self.import_flow.capabilities_error = None;
                        }
                        Err(error) => self.import_flow.capabilities_error = Some(error),
                    }
                }
                UiMessage::ImportJobLoaded { result, .. } => {
                    self.sync_import_busy();
                    match *result {
                        Ok(job) => {
                            let job_changed = self
                                .import_flow
                                .job
                                .as_ref()
                                .is_none_or(|current| current.import_id != job.import_id);
                            let recovered = job_changed
                                && self.import_flow.recovery_import_id == job.import_id.as_str();
                            if recovered {
                                self.import_flow.pending_plan_request = None;
                            }
                            self.import_flow.hydrate_job_contract(&job);
                            if recovered && job.recovery.is_none() {
                                self.import_flow.recovery_contract_gap = true;
                            }
                            let load_diagnostics = job.lifecycle
                                == labello_client::ImportLifecycle::AwaitingDecision
                                && self.import_flow.diagnostics.is_empty();
                            self.import_flow.recovery_import_id = job.import_id.to_string();
                            self.import_flow.screen = crate::import_flow::import_screen(
                                &job,
                                self.import_flow.plan.as_ref(),
                            );
                            self.import_flow.error = None;
                            let polling = matches!(
                                job.lifecycle,
                                labello_client::ImportLifecycle::Preflighting
                                    | labello_client::ImportLifecycle::Building
                                    | labello_client::ImportLifecycle::Verifying
                                    | labello_client::ImportLifecycle::Committing
                            );
                            self.import_flow.poll_after =
                                polling.then(|| Instant::now() + Duration::from_millis(500));
                            self.import_flow.job = Some(job);
                            if load_diagnostics {
                                self.request_import_diagnostics(true);
                            }
                        }
                        Err(error) => self.import_flow.error = Some(error),
                    }
                }
                #[cfg(target_arch = "wasm32")]
                UiMessage::ImportBrowserFilesSelected { result, .. } => {
                    self.sync_import_busy();
                    match result {
                        Ok(files) => self.register_selected_import_files(files),
                        Err(error) => self.import_flow.error = Some(error),
                    }
                }
                UiMessage::ImportFilesRegistered { result, .. } => {
                    self.sync_import_busy();
                    match result {
                        Ok(registered) => {
                            if let Some(job) = self.import_flow.job.as_mut() {
                                job.lifecycle = labello_client::ImportLifecycle::Uploading;
                                job.progress.registered_files = registered.registered_files;
                                job.progress.total_files = registered.registered_files;
                                job.progress.total_bytes = registered.registered_bytes;
                            }
                            #[cfg(target_arch = "wasm32")]
                            {
                                let mut yolo_descriptor_reference_changed = false;
                                for file in &registered.files {
                                    if let Some(path) = self
                                        .import_flow
                                        .registered_paths
                                        .iter_mut()
                                        .find(|path| path.client_file_id == file.client_file_id)
                                    {
                                        path.file_id = file.file_id.clone();
                                    }
                                    for descriptor in &mut self.import_flow.descriptors {
                                        if descriptor.descriptor_file_id == file.client_file_id {
                                            descriptor.descriptor_file_id = file.file_id.clone();
                                            yolo_descriptor_reference_changed = true;
                                        }
                                        if descriptor.image_root_file_id == file.client_file_id {
                                            descriptor.image_root_file_id = file.file_id.clone();
                                        }
                                    }
                                }
                                if yolo_descriptor_reference_changed
                                    && matches!(
                                        self.import_flow.profile,
                                        labello_client::ImportProfile::UltralyticsYoloDetectV1
                                            | labello_client::ImportProfile::UltralyticsYoloPoseV1
                                    )
                                {
                                    self.import_flow.invalidate_yolo_inspection();
                                }
                                let inspect_completed_yolo = yolo_descriptor_reference_changed
                                    && self.import_flow.descriptors.first().is_some_and(
                                        |descriptor| {
                                            registered.files.iter().any(|file| {
                                                file.file_id == descriptor.descriptor_file_id
                                                    && file.complete
                                            })
                                        },
                                    );
                                self.import_flow.browser_uploads = registered.files;
                                self.upload_next_import_chunk();
                                if inspect_completed_yolo {
                                    self.request_yolo_descriptor_inspection_after_upload();
                                }
                            }
                        }
                        Err(error) => self.import_flow.error = Some(error),
                    }
                }
                UiMessage::ImportSourceBrowsed { request, result } => {
                    if self.import_flow.source_picker.pending_request_id != Some(request.request_id)
                    {
                        continue;
                    }
                    self.import_flow.source_picker.pending_request_id = None;
                    self.import_flow.source_picker.loading = false;
                    match result {
                        Ok(mut page) => {
                            if self.import_flow.source_picker.pending_append
                                && let Some(current) = self.import_flow.source_picker.page.as_mut()
                                && current.relative_path == page.relative_path
                            {
                                current.entries.append(&mut page.entries);
                                current.next_offset = page.next_offset;
                            } else {
                                self.import_flow.source_picker.page = Some(page);
                            }
                            self.import_flow.source_picker.error = None;
                        }
                        Err(error) => self.import_flow.source_picker.error = Some(error),
                    }
                    self.import_flow.source_picker.pending_append = false;
                }
                UiMessage::YoloDescriptorInspected {
                    request,
                    descriptor_file_id,
                    result,
                } => {
                    let current_descriptor = self
                        .import_flow
                        .descriptors
                        .first()
                        .map(|descriptor| descriptor.descriptor_file_id.trim());
                    if self.import_flow.pending_yolo_inspection_request_id
                        != Some(request.request_id)
                        || current_descriptor != Some(descriptor_file_id.trim())
                    {
                        continue;
                    }
                    self.import_flow.pending_yolo_inspection_request_id = None;
                    self.import_flow.yolo_inspection_loading = false;
                    match result {
                        Ok(inspection) => {
                            self.import_flow.yolo_splits = inspection
                                .splits
                                .into_iter()
                                .map(|split| crate::import_flow::ImportYoloSplitDraft {
                                    name: split.name,
                                    usable: split.usable,
                                    selected: split.usable,
                                    issue: split.issue,
                                })
                                .collect();
                            self.import_flow.yolo_inspected_descriptor_file_id =
                                Some(descriptor_file_id);
                            self.import_flow.yolo_inspection_error = self
                                .import_flow
                                .yolo_splits
                                .iter()
                                .all(|split| !split.usable)
                                .then(|| {
                                    "The YAML does not define a usable train, val, or test split."
                                        .to_string()
                                });
                        }
                        Err(error) => {
                            self.import_flow.yolo_splits.clear();
                            self.import_flow.yolo_inspected_descriptor_file_id = None;
                            self.import_flow.yolo_inspection_error = Some(error);
                        }
                    }
                    if self.import_flow.yolo_inspection_retry_after_current {
                        self.import_flow.yolo_inspection_retry_after_current = false;
                        self.request_yolo_descriptor_inspection();
                    }
                }
                UiMessage::ImportChunkUploaded {
                    file_id: _file_id,
                    result,
                    ..
                } => {
                    self.sync_import_busy();
                    match result {
                        Ok(_chunk) => {
                            #[cfg(target_arch = "wasm32")]
                            let inspect_completed_yolo =
                                _chunk.complete
                                    && matches!(
                                        self.import_flow.profile,
                                        labello_client::ImportProfile::UltralyticsYoloDetectV1
                                            | labello_client::ImportProfile::UltralyticsYoloPoseV1
                                    )
                                    && self.import_flow.descriptors.first().is_some_and(
                                        |descriptor| descriptor.descriptor_file_id == _file_id,
                                    )
                                    && self.import_flow.yolo_inspected_descriptor_file_id.is_none();
                            #[cfg(target_arch = "wasm32")]
                            if let Some(file) = self
                                .import_flow
                                .browser_uploads
                                .iter_mut()
                                .find(|file| file.file_id == _file_id)
                            {
                                file.accepted_bytes = _chunk.accepted_offset;
                                file.complete = _chunk.complete;
                            }
                            if let Some(_job) = self.import_flow.job.as_mut() {
                                #[cfg(target_arch = "wasm32")]
                                {
                                    _job.progress.uploaded_files =
                                        self.import_flow
                                            .browser_uploads
                                            .iter()
                                            .filter(|file| file.complete)
                                            .count() as u64;
                                    _job.progress.accepted_bytes = self
                                        .import_flow
                                        .browser_uploads
                                        .iter()
                                        .map(|file| file.accepted_bytes)
                                        .sum();
                                }
                            }
                            #[cfg(target_arch = "wasm32")]
                            self.upload_next_import_chunk();
                            #[cfg(target_arch = "wasm32")]
                            if inspect_completed_yolo {
                                self.request_yolo_descriptor_inspection_after_upload();
                            }
                        }
                        Err(error) => self.import_flow.error = Some(error),
                    }
                }
                UiMessage::ImportSealed { result, .. } => {
                    self.sync_import_busy();
                    match result {
                        Ok(sealed) => {
                            if let Some(job) = self.import_flow.job.as_mut() {
                                job.lifecycle = labello_client::ImportLifecycle::Sealed;
                                job.source_fingerprint = Some(sealed.source_fingerprint);
                                job.progress.total_files = sealed.files;
                                job.progress.total_bytes = sealed.bytes;
                            }
                            self.request_preflight_import(false);
                        }
                        Err(error) => self.import_flow.error = Some(error),
                    }
                }
                UiMessage::ImportPlanUpdated { result, .. } => {
                    self.sync_import_busy();
                    match *result {
                        Ok(plan) => {
                            let requested = self.import_flow.pending_plan_request.take();
                            if requested.as_ref() != plan.accepted_request.as_ref() {
                                self.import_flow.plan = None;
                                self.import_flow.accepted_plan_request = None;
                                self.import_flow.error = Some(
                                    "The server returned a plan for different mapping inputs. Save the current mappings again before commit."
                                        .to_string(),
                                );
                                continue;
                            }
                            self.import_flow.accepted_plan_request = plan.accepted_request.clone();
                            self.import_flow.screen = if plan.commit_ready {
                                crate::import_flow::ImportScreen::Ready
                            } else {
                                crate::import_flow::ImportScreen::Preflight
                            };
                            if let Some(job) = self.import_flow.job.as_mut() {
                                job.plan_hash = Some(plan.plan_hash.clone());
                                job.preflight_report = Some(plan.report.clone());
                            }
                            self.import_flow.plan = Some(plan);
                            self.import_flow.error = None;
                            self.request_import_diagnostics(true);
                        }
                        Err(error) => {
                            self.import_flow.pending_plan_request = None;
                            self.import_flow.error = Some(error);
                        }
                    }
                }
                UiMessage::ImportDiagnosticsLoaded { result, .. } => {
                    self.sync_import_busy();
                    match result {
                        Ok(page) => {
                            self.import_flow.diagnostics.extend(page.diagnostics);
                            self.import_flow.diagnostics_cursor = page.next_cursor;
                        }
                        Err(error) => self.import_flow.error = Some(error),
                    }
                }
                UiMessage::ImportCommitted { result, .. } => {
                    self.sync_import_busy();
                    self.begin_import_epoch();
                    match result {
                        Ok(committed) => {
                            if let Some(job) = self.import_flow.job.as_mut() {
                                job.lifecycle = labello_client::ImportLifecycle::Succeeded;
                                job.destination_dataset_id = committed.dataset_id;
                                job.plan_hash = Some(committed.plan_hash);
                            }
                            self.import_flow.screen = crate::import_flow::ImportScreen::Success;
                            self.import_flow.error = None;
                            self.runtime.notice = Some(if committed.recovered {
                                "Recovered and completed the import".to_string()
                            } else {
                                "Dataset import completed".to_string()
                            });
                            self.request_dataset_list();
                        }
                        Err(error) => {
                            self.import_flow.screen = crate::import_flow::ImportScreen::Failure;
                            self.import_flow.error = Some(error);
                        }
                    }
                }
                UiMessage::ImportCancelled { result, .. } => {
                    self.sync_import_busy();
                    self.begin_import_epoch();
                    match result {
                        Ok(cancelled) => {
                            if let Some(job) = self.import_flow.job.as_mut() {
                                job.lifecycle = cancelled.lifecycle;
                            }
                            self.import_flow.screen = crate::import_flow::ImportScreen::Failure;
                            self.runtime.notice = Some("Import cancelled".to_string());
                        }
                        Err(error) => self.import_flow.error = Some(error),
                    }
                }
                UiMessage::MigrationFinished { result, .. } => {
                    self.work.migration.busy = false;
                    match *result {
                        Ok(result) => {
                            let completed = result.assignment.as_ref().is_some_and(|assignment| {
                                assignment.status == labello_domain::AssignmentStatus::Completed
                            });
                            self.apply_state(result.image_state);
                            self.work.migration.cursor = result.cursor;
                            self.work.migration.progress = Some(result.progress);
                            self.work.migration.active_pass_id =
                                result.active_pass.map(|pass| pass.pass_id);
                            if let Some(assignment) = result.assignment {
                                self.assignment = Some(assignment);
                            }
                            self.work.migration.draft = None;
                            self.work.migration.draft_group = None;
                            self.work.migration.keypoint_index = 0;
                            self.work.migration.error = None;
                            if self.view == AppView::Review {
                                self.work.migration.review_index =
                                    self.canonical_migration_review_index();
                            }
                            if completed {
                                self.clear_current_image();
                                self.request_next_image();
                            }
                        }
                        Err(error) => self.work.migration.error = Some(error),
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
                        continue;
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
                                self.import_flow = Default::default();
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
                                self.import_flow = Default::default();
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
                            if self.admin_tools.dataset_id.as_ref() != Some(&self.config.dataset_id)
                            {
                                self.admin_tools = Default::default();
                                self.admin_tools.dataset_id = Some(self.config.dataset_id.clone());
                            }
                            self.admin_tools.load_error = None;
                            self.view = AppView::Admin;
                            self.runtime.error = None;
                            self.request_admin_draft_load();
                            self.request_images();
                            if !self.admin_tools.snapshots_loaded {
                                self.request_snapshots();
                            }
                        }
                        Err(error) => {
                            self.admin_tools.load_error = Some(error.clone());
                            self.runtime.error = Some(error);
                        }
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
                        self.runtime.error = None;
                        self.request_next_admin_role_save();
                    }
                    Err(error) => {
                        self.loading.admin = false;
                        self.admin_tools.pending_role_saves.clear();
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
                            self.runtime.error = None;
                            self.request_next_admin_role_save();
                        }
                        Err(error) => {
                            self.admin_tools.pending_role_saves.clear();
                            self.runtime.error = Some(error);
                        }
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
                    match result {
                        Ok(snapshots) => {
                            self.admin_tools.snapshots = snapshots;
                            self.admin_tools.snapshots_loaded = true;
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
                            self.admin_tools.snapshot_action_error = None;
                            self.request_snapshots();
                        }
                        Err(error) => self.admin_tools.snapshot_action_error = Some(error),
                    }
                }
                UiMessage::SnapshotDownloaded { result, .. } => {
                    self.loading.snapshot_file = None;
                    match result {
                        Ok(file) => match crate::admin::download_snapshot_file(file) {
                            Ok(()) => self.admin_tools.snapshot_action_error = None,
                            Err(error) => self.admin_tools.snapshot_action_error = Some(error),
                        },
                        Err(error) => self.admin_tools.snapshot_action_error = Some(error),
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
                UiMessage::PreviousAssignmentLoaded {
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
                        Ok(loaded) => {
                            if loaded
                                .assignment
                                .expires_at
                                .is_some_and(|expires_at| expires_at <= labello_domain::now())
                            {
                                self.previous_annotation_assignment = None;
                                self.release_reservation(
                                    self.config.dataset_id.clone(),
                                    loaded.assignment,
                                );
                                self.runtime.error = Some(
                                    "The previous assignment lease expired while loading."
                                        .to_string(),
                                );
                                if self.assignment.is_none() {
                                    self.request_next_image();
                                }
                                continue;
                            }
                            let displaced = self.assignment.clone();
                            self.begin_workspace_epoch();
                            self.clear_current_image();
                            if let Some(displaced) = displaced
                                && displaced.assignment_id != loaded.assignment.assignment_id
                            {
                                self.release_reservation(self.config.dataset_id.clone(), displaced);
                            }
                            self.previous_annotation_assignment = None;
                            self.runtime.error = None;
                            self.runtime.notice =
                                Some("Returned to previous assignment".to_string());
                            self.apply_loaded_image(ctx, loaded);
                        }
                        Err(error) => {
                            let expired = assignment.as_ref().is_some_and(|assignment| {
                                assignment.status == labello_domain::AssignmentStatus::Active
                                    && assignment.expires_at.is_some_and(|expires_at| {
                                        expires_at <= labello_domain::now()
                                    })
                            });
                            if expired {
                                self.clear_previous_annotation_assignment();
                            } else if let Some(assignment) = assignment {
                                self.previous_annotation_assignment = Some(assignment);
                            }
                            self.runtime.error = Some(error);
                            if expired && self.assignment.is_none() {
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
                                if let Some(mut assignment) =
                                    self.assignment.clone().filter(|assignment| {
                                        assignment.kind
                                            == labello_domain::AssignmentKind::Annotation
                                    })
                                {
                                    assignment.status = labello_domain::AssignmentStatus::Completed;
                                    self.remember_previous_annotation_assignment(assignment);
                                }
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
                                if assignment.kind == labello_domain::AssignmentKind::Annotation {
                                    let mut assignment = assignment;
                                    assignment.status = labello_domain::AssignmentStatus::Cancelled;
                                    self.remember_previous_annotation_assignment(assignment);
                                }
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
                                    crate::review_sequence::reviewed_object_prefix(
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
                            self.shortcut_settings.error = None;
                            if self.show_settings {
                                self.shortcut_settings.baseline = Some(self.keybindings.clone());
                                self.shortcut_settings.draft = Some(self.keybindings.clone());
                            }
                            self.runtime.notice = Some("Keyboard shortcuts saved".to_string());
                            self.runtime.error = None;
                        }
                        Err(error) => {
                            self.shortcut_settings.error = Some(error.clone());
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
                        continue;
                    }
                    self.loading.uploading = true;
                    self.loading.upload_progress = Some(progress);
                    self.runtime.error = None;
                    ctx.request_repaint();
                }
                UiMessage::FolderUploadFinished { request, result } => {
                    if !self.request_is_current(&request, true) {
                        continue;
                    }
                    self.loading.uploading = false;
                    self.loading.upload_progress = None;
                    match result {
                        Ok(message) => {
                            self.runtime.notice = Some(message);
                            self.runtime.error = None;
                            self.admin_tools.upload_error = None;
                            self.request_admin_dataset();
                            self.request_dataset_list();
                        }
                        Err(error) => {
                            self.admin_tools.upload_error = Some(error.clone());
                            self.runtime.error = Some(error);
                        }
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
            UiCommand::ImportCapabilities { request } => self.spawn_import_message(async move {
                UiMessage::ImportCapabilitiesLoaded {
                    request,
                    result: api
                        .import_capabilities()
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::CreateImport {
                request,
                body,
                idempotency_key,
            } => self.spawn_import_message(async move {
                UiMessage::ImportJobLoaded {
                    request,
                    result: Box::new(
                        api.create_import(body, &idempotency_key)
                            .await
                            .map_err(|error| error.to_string()),
                    ),
                }
            }),
            UiCommand::GetImport { request, import_id } => self.spawn_import_message(async move {
                UiMessage::ImportJobLoaded {
                    request,
                    result: Box::new(
                        api.get_import(&import_id)
                            .await
                            .map_err(|error| error.to_string()),
                    ),
                }
            }),
            UiCommand::RegisterImportFiles {
                request,
                import_id,
                body,
                idempotency_key,
            } => self.spawn_import_message(async move {
                UiMessage::ImportFilesRegistered {
                    request,
                    result: api
                        .register_import_files(&import_id, body, &idempotency_key)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::BrowseImportRoot {
                request,
                root_id,
                body,
            } => self.spawn_import_message(async move {
                UiMessage::ImportSourceBrowsed {
                    request,
                    result: api
                        .browse_server_import_root(&root_id, body)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::BrowseImportSource {
                request,
                import_id,
                body,
            } => self.spawn_import_message(async move {
                UiMessage::ImportSourceBrowsed {
                    request,
                    result: api
                        .browse_import_source(&import_id, body)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::InspectYoloDescriptor {
                request,
                import_id,
                descriptor_file_id,
                body,
            } => self.spawn_import_message(async move {
                UiMessage::YoloDescriptorInspected {
                    request,
                    descriptor_file_id,
                    result: api
                        .inspect_yolo_descriptor(&import_id, body)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::SealImport {
                request,
                import_id,
                body,
                idempotency_key,
            } => self.spawn_import_message(async move {
                UiMessage::ImportSealed {
                    request,
                    result: api
                        .seal_import(&import_id, body, &idempotency_key)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::PreflightImport {
                request,
                import_id,
                body,
                idempotency_key,
            } => self.spawn_import_message(async move {
                UiMessage::ImportJobLoaded {
                    request,
                    result: Box::new(
                        api.preflight_import(&import_id, body, &idempotency_key)
                            .await
                            .map_err(|error| error.to_string()),
                    ),
                }
            }),
            UiCommand::UpdateImportPlan {
                request,
                import_id,
                body,
                idempotency_key,
            } => self.spawn_import_message(async move {
                UiMessage::ImportPlanUpdated {
                    request,
                    result: Box::new(
                        api.update_import_plan(&import_id, body, &idempotency_key)
                            .await
                            .map_err(|error| error.to_string()),
                    ),
                }
            }),
            UiCommand::ImportDiagnostics {
                request,
                import_id,
                query,
            } => self.spawn_import_message(async move {
                UiMessage::ImportDiagnosticsLoaded {
                    request,
                    result: api
                        .import_diagnostics(&import_id, query)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::CommitImport {
                request,
                import_id,
                body,
                idempotency_key,
            } => self.spawn_import_message(async move {
                UiMessage::ImportCommitted {
                    request,
                    result: api
                        .commit_import(&import_id, body, &idempotency_key)
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::CancelImport {
                request,
                import_id,
                idempotency_key,
            } => self.spawn_import_message(async move {
                UiMessage::ImportCancelled {
                    request,
                    result: api
                        .cancel_import(
                            &import_id,
                            labello_client::CancelImportRequest {
                                reason: Some("cancelled by administrator".to_string()),
                            },
                            &idempotency_key,
                        )
                        .await
                        .map_err(|error| error.to_string()),
                }
            }),
            UiCommand::Migration {
                request,
                dataset_id,
                image_id,
                action,
                idempotency_key,
            } => self.spawn_message(request.clone(), async move {
                let result = match action {
                    crate::app::MigrationAction::SaveSkeleton(body) => {
                        api.save_migration_skeleton(&dataset_id, &image_id, body, &idempotency_key)
                            .await
                    }
                    crate::app::MigrationAction::Exclude(body) => {
                        api.exclude_migration_target(&dataset_id, &image_id, body, &idempotency_key)
                            .await
                    }
                    crate::app::MigrationAction::Reopen(body) => {
                        api.reopen_migration_target(&dataset_id, &image_id, body, &idempotency_key)
                            .await
                    }
                    crate::app::MigrationAction::StartPass(body) => {
                        api.start_migration_pass(&dataset_id, &image_id, body, &idempotency_key)
                            .await
                    }
                    crate::app::MigrationAction::Keep(body) => {
                        api.keep_migration_target(&dataset_id, &image_id, body, &idempotency_key)
                            .await
                    }
                    crate::app::MigrationAction::Confirm(body) => {
                        api.confirm_migration(&dataset_id, &image_id, body, &idempotency_key)
                            .await
                    }
                    crate::app::MigrationAction::Review(body) => {
                        api.review_migration(&dataset_id, &image_id, body, &idempotency_key)
                            .await
                    }
                }
                .map_err(|error| error.to_string());
                UiMessage::MigrationFinished {
                    request,
                    result: Box::new(result),
                }
            }),
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

    fn clear_authenticated_state(&mut self) {
        self.begin_import_epoch();
        self.import_flow = Default::default();
        self.auth.account = None;
        self.auth.can_create_datasets = false;
        self.datasets.summaries.clear();
        self.datasets.summaries_error = None;
        self.datasets.metadata = None;
        self.datasets.admin_config = None;
        self.datasets.admin_baseline = None;
        self.datasets.users.clear();
        self.datasets.users_baseline.clear();
        self.datasets.stats = Default::default();
        self.datasets.active_stats_request = None;
        self.datasets.last_stats_attempt = None;
        self.datasets.last_stats_completion = None;
        self.datasets.stats_error = None;
        self.datasets.requested_view = None;
        self.admin_tools = Default::default();
        self.drawer = None;
        self.show_tutorial = false;
        self.shortcut_settings = Default::default();
        self.keybindings = labello_domain::KeybindingSet::defaults_for(self.config.user_id.clone());
        self.previous_annotation_assignment = None;
        self.clear_current_image();
        self.isolate_browser_workspace();
        self.runtime.storage_error = None;
        self.runtime.notice = None;
        self.view = AppView::Setup;
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
        if self.view == AppView::Admin && self.admin_changes_dirty() {
            self.runtime.error =
                Some("Save or discard staged Admin changes before signing out.".to_string());
            return;
        }
        self.clear_previous_annotation_assignment();
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
            let request_id = command
                .import_request()
                .map(|request| request.request_id)
                .unwrap_or_else(|| command.request().request_id);
            self.runtime.active_requests.insert(request_id);
            if let Some(activity) = command.import_activity() {
                self.import_flow
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
        self.import_flow.busy = self
            .import_flow
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
        self.import_flow.active_operations.remove(&request_id);
        match command {
            UiCommand::ImportCapabilities { .. } => {
                self.import_flow.capabilities_loading = false;
                self.import_flow.capabilities_error = Some(error.to_string());
            }
            UiCommand::BrowseImportRoot { .. } | UiCommand::BrowseImportSource { .. } => {
                self.import_flow.source_picker.loading = false;
                self.import_flow.source_picker.pending_request_id = None;
                self.import_flow.source_picker.error = Some(error.to_string());
            }
            UiCommand::InspectYoloDescriptor { .. } => {
                self.import_flow.yolo_inspection_loading = false;
                self.import_flow.pending_yolo_inspection_request_id = None;
                self.import_flow.yolo_inspection_error = Some(error.to_string());
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
                self.import_flow.error = Some(error.to_string());
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
                self.admin_tools.load_error = Some(error.to_string());
            }
            UiCommand::SaveAdmin { .. } => self.loading.admin = false,
            UiCommand::SaveDatasetRoles { .. } => self.loading.roles_user = None,
            UiCommand::LoadImages { .. } => {
                self.loading.images = false;
                self.admin_tools.images_error = Some(error.to_string());
            }
            UiCommand::LoadSnapshots { .. } => {
                self.loading.snapshots = false;
                self.admin_tools.snapshots_error = Some(error.to_string());
            }
            UiCommand::CreateSnapshot { .. } => {
                self.loading.creating_snapshot = false;
                self.admin_tools.snapshot_action_error = Some(error.to_string());
            }
            UiCommand::DownloadSnapshot { .. } => {
                self.loading.snapshot_file = None;
                self.admin_tools.snapshot_action_error = Some(error.to_string());
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
            UiCommand::SaveKeybindings { .. } => {
                self.loading.keybindings = false;
                self.shortcut_settings.error = Some(error.to_string());
            }
            UiCommand::ClaimAssignment { operation_id, .. }
            | UiCommand::ReloadAssignment { operation_id, .. }
            | UiCommand::ReopenAssignment { operation_id, .. } => {
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
        self.import_flow.busy = false;
        self.import_flow.active_operations.clear();
        self.import_flow.poll_after = None;
    }

    fn finish_import_request(&mut self, request: &ImportRequestIdentity) -> bool {
        let owner_matches = request.import_id.as_ref().is_none_or(|owner| {
            self.import_flow
                .job
                .as_ref()
                .is_none_or(|job| &job.import_id == owner)
                || self.import_flow.recovery_import_id == owner.as_str()
        });
        let current = request.auth_epoch == self.auth_epoch
            && request.import_epoch == self.import_epoch
            && owner_matches;
        let active = self.runtime.active_requests.remove(&request.request_id);
        self.import_flow
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
        self.admin_tools.pending_role_saves.clear();
        self.loading.image = false;
        self.loading.saving = false;
        self.loading.ingesting = false;
        self.loading.ingest_polling = false;
        self.loading.ingest_job_id = None;
        self.loading.last_ingest_poll = None;
        self.loading.uploading = false;
        self.loading.upload_progress = None;
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
        self.datasets.requested_view = None;
        self.runtime.persistence.restoration_attempted = false;
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
            self.admin_tools = Default::default();
        }
        if self.loading.admin || self.runtime.api.is_none() {
            return;
        }
        self.loading.admin = true;
        self.admin_tools.load_error = None;
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
        if self.loading.snapshots || self.loading.creating_snapshot || self.runtime.api.is_none() {
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
        if self.loading.creating_snapshot || self.loading.snapshots || self.runtime.api.is_none() {
            return;
        }
        self.loading.creating_snapshot = true;
        self.admin_tools.snapshot_action_error = None;
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
        self.admin_tools.snapshot_action_error = None;
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
        self.admin_tools.pending_role_saves = dirty_users
            .into_iter()
            .map(|(user_id, _)| user_id)
            .collect();

        if self.datasets.admin_config != self.datasets.admin_baseline {
            if !self.request_admin_save() {
                self.admin_tools.pending_role_saves.clear();
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
        let Some(user_id) = self.admin_tools.pending_role_saves.pop_front() else {
            self.runtime.notice = Some("Admin changes saved".to_string());
            return;
        };
        if !self.request_role_save(user_id) {
            self.admin_tools.pending_role_saves.clear();
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
            .shortcut_settings
            .draft
            .clone()
            .unwrap_or_else(|| self.keybindings.clone());
        if let Err(error) = keybindings.validate() {
            self.shortcut_settings.error = Some(error.to_string());
            self.runtime.error = Some(error.to_string());
            return;
        }
        self.loading.keybindings = true;
        self.shortcut_settings.error = None;
        let request = self.request_identity(Some(self.config.dataset_id.clone()));
        self.queue_command(UiCommand::SaveKeybindings {
            request,
            dataset_id: self.config.dataset_id.clone(),
            keybindings,
        });
    }

    fn apply_loaded_dataset(&mut self, loaded: LoadedDataset) {
        self.clear_previous_annotation_assignment();
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
        self.work.migration = Default::default();
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
        if let Some(crate::app::PendingTransition::PreviousAssignment(assignment)) = transition {
            self.previous_annotation_assignment = Some(assignment.clone());
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

    pub(crate) fn apply_state(&mut self, state: labello_domain::ImageState) {
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

    pub(crate) fn spawn_import_message<F>(&self, future: F)
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
        drop(future);
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
