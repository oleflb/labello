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
                        self.runtime.error = None;
                        self.request_dataset_list();
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
                        Ok(loaded) => self.apply_loaded_dataset(loaded),
                        Err(error) => self.runtime.error = Some(error),
                    }
                }
                UiMessage::AdminLoaded(result) => {
                    self.loading.admin = false;
                    match result {
                        Ok(metadata) => {
                            self.sync_work_config(metadata.clone());
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
                        self.datasets.admin_config = Some(metadata);
                        self.runtime.notice = Some("Admin config saved".to_string());
                        self.runtime.error = None;
                        self.request_dataset_list();
                    }
                    Err(error) => {
                        self.loading.admin = false;
                        self.runtime.error = Some(error);
                    }
                },
                UiMessage::ImageLoaded(result) => {
                    self.loading.image = false;
                    self.queue.set_loading(false);
                    match result {
                        Ok(loaded) => {
                            self.runtime.error = None;
                            self.apply_loaded_image(ctx, loaded);
                        }
                        Err(error) => self.runtime.error = Some(error),
                    }
                }
                UiMessage::SaveFinished(result) => {
                    self.loading.saving = false;
                    match result {
                        Ok(state) => {
                            self.apply_state(state);
                            self.save_status = SaveStatus::Saved;
                            self.runtime.error = None;
                            self.request_stats();
                        }
                        Err(error) => {
                            self.save_status = SaveStatus::Dirty;
                            self.runtime.error = Some(error);
                        }
                    }
                }
                UiMessage::ReviewFinished(result) => {
                    self.loading.saving = false;
                    match result {
                        Ok(()) => {
                            self.runtime.error = None;
                            self.request_stats();
                            self.next_image();
                        }
                        Err(error) => self.runtime.error = Some(error),
                    }
                }
                UiMessage::AdjudicationFinished(result) => {
                    self.loading.saving = false;
                    match result {
                        Ok(()) => {
                            self.runtime.error = None;
                            self.request_stats();
                            self.next_image();
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
        self.loading.dataset = true;
        self.queue_command(UiCommand::CreateDataset {
            dataset_id: self.setup.create_dataset_id.trim().to_string().into(),
            name: self.setup.create_dataset_name.trim().to_string(),
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

    fn apply_loaded_dataset(&mut self, loaded: LoadedDataset) {
        self.sync_work_config(loaded.metadata);
        self.keybindings = loaded.keybindings;
        self.view = AppView::Annotate;
        if self.ensure_valid_task_selection() {
            self.runtime.error = None;
        } else {
            self.runtime.error = Some(
                "No enabled workflow is configured. Ask a data admin to enable at least one class workflow."
                    .to_string(),
            );
        }
        if self.current.is_none() && self.selected_class_id().is_some() {
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
                    self.runtime.notice = Some(format!(
                        "Ingested {} new images from {} discovered files",
                        report.new_images, report.discovered_files
                    ));
                    self.runtime.error = None;
                    self.request_admin_dataset();
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
        self.accepted_prelabels.clear();
        self.selected_annotation = None;
        self.save_status = SaveStatus::Idle;
        self.current_texture = loaded.color_image.map(|image| {
            ctx.load_texture(
                format!("image-{image_id}"),
                image,
                egui::TextureOptions::LINEAR,
            )
        });
    }

    fn apply_state(&mut self, state: labello_domain::ImageState) {
        self.annotations = state.active_annotations().cloned().collect();
        self.persisted_annotations = self
            .annotations
            .iter()
            .map(|annotation| annotation.annotation_id.clone())
            .collect();
        self.current_state = Some(state);
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
