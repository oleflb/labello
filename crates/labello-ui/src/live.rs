use std::{
    future::Future,
    rc::Rc,
    time::{Duration, Instant},
};

use eframe::egui;
use labello_client::{
    AuthHeaders, HttpLabelloApi, IngestJob, IngestJobStatus, UpdateDatasetConfigRequest,
};

use crate::app::{
    AppView, LabelloApp, LoadedDataset, LoadedImage, SaveStatus, UiCommand, UiMessage,
};

impl LabelloApp {
    pub(crate) fn rebuild_http_api(&mut self) {
        match HttpLabelloApi::new(&self.config.api_base_url).map(|api| {
            api.with_auth(AuthHeaders {
                user_id: Some(self.config.user_id.clone()),
                role: Some(self.config.role.clone()),
                dev_token: if self.config.dev_token.is_empty() {
                    None
                } else {
                    Some(self.config.dev_token.clone())
                },
            })
        }) {
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
        for _ in 0..8 {
            let Ok(message) = self.runtime.rx.try_recv() else {
                break;
            };
            match message {
                UiMessage::DatasetList(result) => {
                    self.loading.datasets = false;
                    match result {
                        Ok(datasets) => {
                            self.datasets.summaries = datasets;
                            self.runtime.error = None;
                        }
                        Err(error) => self.runtime.error = Some(error),
                    }
                }
                UiMessage::DatasetCreated(result) => match result {
                    Ok(metadata) => {
                        self.loading.dataset = false;
                        self.config.dataset_id = metadata.dataset_id.clone();
                        self.setup.create_dataset_id = metadata.dataset_id.to_string();
                        self.setup.create_dataset_name = metadata.name.clone();
                        self.upsert_dataset_summary(&metadata);
                        self.runtime.error = None;
                        self.request_load_dataset();
                    }
                    Err(error) => {
                        self.loading.dataset = false;
                        self.runtime.error = Some(error);
                    }
                },
                UiMessage::DatasetLoaded(result) => {
                    self.loading.dataset = false;
                    match result {
                        Ok(loaded) => {
                            self.upsert_dataset_summary(&loaded.metadata);
                            self.apply_loaded_dataset(loaded);
                        }
                        Err(error) => self.runtime.error = Some(error),
                    }
                }
                UiMessage::AdminLoaded(result) => {
                    self.loading.admin = false;
                    match result {
                        Ok(metadata) => {
                            self.sync_work_config(metadata.clone());
                            self.upsert_dataset_summary(&metadata);
                            self.datasets.admin_baseline = Some(metadata.clone());
                            self.datasets.admin_config = Some(metadata);
                            self.view = AppView::Admin;
                            self.runtime.error = None;
                        }
                        Err(error) => self.runtime.error = Some(error),
                    }
                }
                UiMessage::AdminSaved(result) => match result {
                    Ok(metadata) => {
                        self.loading.admin = false;
                        self.sync_work_config(metadata.clone());
                        self.upsert_dataset_summary(&metadata);
                        self.datasets.admin_baseline = Some(metadata.clone());
                        self.datasets.admin_config = Some(metadata);
                        self.runtime.notice = Some("Admin config saved".to_string());
                        self.runtime.error = None;
                    }
                    Err(error) => {
                        self.loading.admin = false;
                        self.runtime.error = Some(error);
                    }
                },
                UiMessage::ImageLoaded { generation, result } => {
                    if generation != self.load_generation {
                        continue;
                    }
                    self.loading.image = false;
                    self.queue.set_loading(false);
                    match result {
                        Ok(Some(loaded)) => {
                            self.runtime.error = None;
                            self.runtime.notice = None;
                            self.apply_loaded_image(ctx, loaded);
                        }
                        Ok(None) => {
                            self.runtime.error = None;
                            self.runtime.notice = Some(
                                match self.queue_mode {
                                    crate::app::QueueMode::Annotate => {
                                        "No annotation work is currently available."
                                    }
                                    crate::app::QueueMode::Review => {
                                        "No reviews are currently waiting."
                                    }
                                    crate::app::QueueMode::Adjudicate => {
                                        "No adjudications are currently waiting."
                                    }
                                }
                                .to_string(),
                            );
                        }
                        Err(error) => self.runtime.error = Some(error),
                    }
                }
                UiMessage::SaveFinished { image_id, result } => {
                    if self
                        .current
                        .as_ref()
                        .is_none_or(|current| current.image.image_id != image_id)
                    {
                        continue;
                    }
                    self.loading.saving = false;
                    match result {
                        Ok(state) => {
                            self.apply_state(state);
                            self.save_status = SaveStatus::Saved;
                            self.runtime.error = None;
                            self.request_stats();
                            if let Some(transition) = self.pending_transition.take() {
                                self.execute_transition(transition);
                            }
                        }
                        Err(error) => {
                            self.save_status = SaveStatus::Dirty;
                            self.runtime.error = Some(error);
                        }
                    }
                }
                UiMessage::ReviewFinished {
                    image_id,
                    phase,
                    decision,
                    result,
                } => {
                    if self
                        .current
                        .as_ref()
                        .is_none_or(|current| current.image.image_id != image_id)
                    {
                        continue;
                    }
                    self.loading.saving = false;
                    match result {
                        Ok(()) => {
                            self.runtime.error = None;
                            match phase {
                                crate::app::ReviewPhase::Object
                                    if decision == labello_domain::ReviewDecision::Approved =>
                                {
                                    self.review_index = self.review_index.saturating_add(1);
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
                                    self.execute_transition(
                                        crate::app::PendingTransition::NextImage,
                                    );
                                }
                            }
                        }
                        Err(error) => self.runtime.error = Some(error),
                    }
                }
                UiMessage::AdjudicationFinished { image_id, result } => {
                    if self
                        .current
                        .as_ref()
                        .is_none_or(|current| current.image.image_id != image_id)
                    {
                        continue;
                    }
                    self.loading.saving = false;
                    match result {
                        Ok(()) => {
                            self.runtime.error = None;
                            self.request_stats();
                            self.execute_transition(crate::app::PendingTransition::NextImage);
                        }
                        Err(error) => self.runtime.error = Some(error),
                    }
                }
                UiMessage::IngestJobLoaded(result) => self.handle_ingest_job(result),
                UiMessage::StatsLoaded(result) => {
                    self.loading.stats = false;
                    match result {
                        Ok(stats) => self.datasets.stats = stats,
                        Err(error) => self.runtime.error = Some(error),
                    }
                }
                UiMessage::KeybindingsSaved(result) => {
                    self.loading.keybindings = false;
                    match result {
                        Ok(keybindings) => {
                            self.keybindings = keybindings;
                            self.runtime.notice = Some("Keyboard shortcuts saved".to_string());
                            self.runtime.error = None;
                        }
                        Err(error) => self.runtime.error = Some(error),
                    }
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
            }
        }
    }

    pub(crate) fn start_next_command(&mut self) {
        let Some(command) = self.runtime.commands.pop_front() else {
            return;
        };
        let Some(api) = self.runtime.api.clone() else {
            self.runtime.error = Some("API is not configured".to_string());
            return;
        };
        match command {
            UiCommand::DatasetList => self.spawn_message(async move {
                UiMessage::DatasetList(api.list_datasets().await.map_err(|error| error.to_string()))
            }),
            UiCommand::CreateDataset {
                dataset_id,
                name,
                admin_user_id,
            } => self.spawn_message(async move {
                UiMessage::DatasetCreated(
                    api.create_dataset(labello_client::CreateDatasetRequest {
                        dataset_id,
                        name,
                        admin_user_id,
                    })
                    .await
                    .map_err(|error| error.to_string()),
                )
            }),
            UiCommand::LoadDataset {
                dataset_id,
                user_id,
            } => self.spawn_message(async move {
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
                UiMessage::DatasetLoaded(result)
            }),
            UiCommand::LoadAdmin { dataset_id } => self.spawn_message(async move {
                UiMessage::AdminLoaded(
                    api.get_admin_dataset(&dataset_id)
                        .await
                        .map_err(|error| error.to_string()),
                )
            }),
            UiCommand::SaveAdmin { metadata } => {
                let dataset_id = metadata.dataset_id.clone();
                let request = UpdateDatasetConfigRequest::from_metadata(&metadata);
                self.spawn_message(async move {
                    UiMessage::AdminSaved(
                        api.update_dataset_config(&dataset_id, request)
                            .await
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            UiCommand::Ingest { dataset_id } => self.spawn_message(async move {
                UiMessage::IngestJobLoaded(
                    api.start_ingest_job(&dataset_id)
                        .await
                        .map_err(|error| error.to_string()),
                )
            }),
            UiCommand::PollIngest { dataset_id, job_id } => self.spawn_message(async move {
                UiMessage::IngestJobLoaded(
                    api.get_ingest_job(&dataset_id, &job_id)
                        .await
                        .map_err(|error| error.to_string()),
                )
            }),
            UiCommand::Stats { dataset_id } => self.spawn_message(async move {
                UiMessage::StatsLoaded(
                    api.dataset_stats(&dataset_id)
                        .await
                        .map_err(|error| error.to_string()),
                )
            }),
            UiCommand::SaveKeybindings {
                dataset_id,
                keybindings,
            } => self.spawn_message(async move {
                UiMessage::KeybindingsSaved(
                    api.save_keybindings(&dataset_id, keybindings)
                        .await
                        .map_err(|error| error.to_string()),
                )
            }),
            command => self.start_workflow_command(api, command),
        }
    }

    pub(crate) fn start_setup_load(&mut self) {
        if self.runtime.api.is_some() && !self.setup.started {
            self.setup.started = true;
            self.request_dataset_list();
        }
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
            .last_stats_request
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
        if due && self.runtime.commands.len() < 64 {
            self.loading.ingest_polling = true;
            self.queue_command(UiCommand::PollIngest {
                dataset_id: self.config.dataset_id.clone(),
                job_id,
            });
        }
    }

    pub(crate) fn queue_command(&mut self, command: UiCommand) {
        if self.runtime.commands.len() < 64 {
            self.runtime.commands.push_back(command);
        }
    }

    pub(crate) fn request_dataset_list(&mut self) {
        if self.loading.datasets || self.runtime.api.is_none() {
            return;
        }
        self.loading.datasets = true;
        self.queue_command(UiCommand::DatasetList);
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
        self.queue_command(UiCommand::CreateDataset {
            dataset_id,
            name,
            admin_user_id: self.config.user_id.clone(),
        });
    }

    pub(crate) fn request_load_dataset(&mut self) {
        if self.loading.dataset || self.runtime.api.is_none() {
            return;
        }
        self.loading.dataset = true;
        self.queue_command(UiCommand::LoadDataset {
            dataset_id: self.config.dataset_id.clone(),
            user_id: self.config.user_id.clone(),
        });
    }

    pub(crate) fn request_admin_dataset(&mut self) {
        if self.loading.admin || self.runtime.api.is_none() {
            return;
        }
        self.loading.admin = true;
        self.queue_command(UiCommand::LoadAdmin {
            dataset_id: self.config.dataset_id.clone(),
        });
    }

    pub(crate) fn request_admin_save(&mut self) {
        let Some(metadata) = self.datasets.admin_config.clone() else {
            return;
        };
        if self.loading.admin {
            return;
        }
        self.loading.admin = true;
        self.queue_command(UiCommand::SaveAdmin { metadata });
    }

    pub(crate) fn request_ingest(&mut self) {
        if self.loading.ingesting || self.runtime.api.is_none() {
            return;
        }
        self.loading.ingesting = true;
        self.loading.ingest_polling = false;
        self.loading.ingest_job_id = None;
        self.loading.last_ingest_poll = None;
        self.runtime.notice = Some("Starting ingest...".to_string());
        self.queue_command(UiCommand::Ingest {
            dataset_id: self.config.dataset_id.clone(),
        });
    }

    pub(crate) fn request_stats(&mut self) {
        if self.loading.stats
            || self.runtime.api.is_none()
            || self.loading.image
            || !matches!(self.view, AppView::Stats)
        {
            return;
        }
        self.loading.stats = true;
        self.datasets.last_stats_request = Some(std::time::Instant::now());
        self.queue_command(UiCommand::Stats {
            dataset_id: self.config.dataset_id.clone(),
        });
    }

    pub(crate) fn request_keybindings_save(&mut self) {
        if self.loading.keybindings || self.runtime.api.is_none() {
            return;
        }
        if let Err(error) = self.keybindings.validate_conflicts() {
            self.runtime.error = Some(error.to_string());
            return;
        }
        self.loading.keybindings = true;
        self.queue_command(UiCommand::SaveKeybindings {
            dataset_id: self.config.dataset_id.clone(),
            keybindings: self.keybindings.clone(),
        });
    }

    fn apply_loaded_dataset(&mut self, loaded: LoadedDataset) {
        self.clear_current_image();
        self.sync_work_config(loaded.metadata);
        self.keybindings = loaded.keybindings;
        self.keybindings
            .bindings
            .remove(&labello_domain::UserAction::PreviousImage);
        self.keybindings
            .bindings
            .remove(&labello_domain::UserAction::ToggleOfflineMode);
        if !self.can_use_queue_mode(self.queue_mode) {
            self.queue_mode = [
                crate::app::QueueMode::Annotate,
                crate::app::QueueMode::Review,
                crate::app::QueueMode::Adjudicate,
            ]
            .into_iter()
            .find(|mode| self.can_use_queue_mode(*mode))
            .unwrap_or(crate::app::QueueMode::Annotate);
        }
        self.view = if self.can_use_queue_mode(self.queue_mode) {
            AppView::Annotate
        } else {
            AppView::Stats
        };
        if self.ensure_valid_task_selection() {
            self.runtime.error = None;
        } else {
            self.runtime.error = Some(
                "No enabled workflow is configured. Ask a data admin to enable at least one class workflow."
                    .to_string(),
            );
        }
        if self.view == AppView::Annotate && self.selected_class_id().is_some() {
            self.request_next_image();
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
                    if self.view == AppView::Annotate && self.current.is_none() {
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
        let roles = metadata
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
                if !roles.is_empty() {
                    summary.roles = roles;
                }
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
                    roles,
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
        self.selected_annotation = None;
        self.review_index = 0;
        self.review_rejected = false;
        self.save_status = SaveStatus::Idle;
        self.current_texture = loaded.color_image.map(|image| {
            ctx.load_texture(
                format!("image-{image_id}"),
                image,
                egui::TextureOptions::LINEAR,
            )
        });
        self.sync_review_selection();
    }

    fn apply_state(&mut self, state: labello_domain::ImageState) {
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
        if self.queue_mode != crate::app::QueueMode::Review {
            return;
        }
        let selected = self
            .annotations
            .iter()
            .filter(|annotation| {
                !annotation.deleted && self.annotation_matches_selected_workflow(annotation)
            })
            .nth(self.review_index)
            .map(|annotation| annotation.annotation_id.clone());
        self.selected_annotation = selected;
    }

    pub(crate) fn spawn_message<F>(&self, future: F)
    where
        F: Future<Output = UiMessage> + 'static,
    {
        let tx = self.runtime.tx.clone();
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(async move {
            let _ = tx.send(future.await);
        });
        #[cfg(all(not(target_arch = "wasm32"), test))]
        {
            let _ = tx.send(poll_ready(future));
        }
        #[cfg(all(not(target_arch = "wasm32"), not(test)))]
        {
            drop(future);
            let _ = tx.send(UiMessage::DatasetList(Err(
                "live HTTP UI is available in the WASM build".to_string(),
            )));
        }
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
        Poll::Pending => UiMessage::DatasetList(Err(
            "test fake API future did not complete immediately".to_string(),
        )),
    }
}
