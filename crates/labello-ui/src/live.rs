use std::{future::Future, rc::Rc, time::Duration};

use eframe::egui;
use labello_client::{AuthHeaders, HttpLabelloApi, UpdateDatasetConfigRequest};

use crate::app::{AppView, LabelloApp, LoadedDataset, LoadedImage, SaveStatus, UiMessage};

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
        while let Ok(message) = self.runtime.rx.try_recv() {
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
                        self.config.dataset_id = metadata.dataset_id.clone();
                        self.setup.create_dataset_id = metadata.dataset_id.to_string();
                        self.setup.create_dataset_name = metadata.name.clone();
                        self.runtime.error = None;
                        self.request_dataset_list();
                        self.request_load_dataset();
                    }
                    Err(error) => self.runtime.error = Some(error),
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
                            self.datasets.admin_config = Some(metadata);
                            self.view = AppView::Admin;
                            self.runtime.error = None;
                        }
                        Err(error) => self.runtime.error = Some(error),
                    }
                }
                UiMessage::AdminSaved(result) => match result {
                    Ok(metadata) => {
                        self.datasets.admin_config = Some(metadata);
                        self.runtime.error = None;
                        self.request_load_dataset();
                    }
                    Err(error) => self.runtime.error = Some(error),
                },
                UiMessage::ImageLoaded(result) => {
                    self.loading.image = false;
                    self.queue.set_loading(false);
                    match result {
                        Ok(loaded) => self.apply_loaded_image(ctx, loaded),
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
                UiMessage::IngestFinished(result) => {
                    self.loading.ingesting = false;
                    match result {
                        Ok(report) => {
                            self.runtime.error = Some(format!(
                                "Ingested {} new images from {} discovered files",
                                report.new_images, report.discovered_files
                            ));
                            self.request_admin_dataset();
                            self.request_load_dataset();
                            self.request_stats();
                        }
                        Err(error) => self.runtime.error = Some(error),
                    }
                }
                UiMessage::StatsLoaded(result) => {
                    self.loading.stats = false;
                    match result {
                        Ok(stats) => self.datasets.stats = stats,
                        Err(error) => self.runtime.error = Some(error),
                    }
                }
            }
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

    pub(crate) fn request_dataset_list(&mut self) {
        let Some(api) = self.runtime.api.clone() else {
            return;
        };
        self.loading.datasets = true;
        self.spawn_message(async move {
            UiMessage::DatasetList(api.list_datasets().await.map_err(|error| error.to_string()))
        });
    }

    pub(crate) fn request_create_dataset(&mut self) {
        let Some(api) = self.runtime.api.clone() else {
            return;
        };
        let request = labello_client::CreateDatasetRequest {
            dataset_id: self.setup.create_dataset_id.trim().to_string().into(),
            name: self.setup.create_dataset_name.trim().to_string(),
            admin_user_id: self.config.user_id.clone(),
        };
        self.spawn_message(async move {
            UiMessage::DatasetCreated(
                api.create_dataset(request)
                    .await
                    .map_err(|error| error.to_string()),
            )
        });
    }

    pub(crate) fn request_load_dataset(&mut self) {
        let Some(api) = self.runtime.api.clone() else {
            return;
        };
        let dataset_id = self.config.dataset_id.clone();
        let user_id = self.config.user_id.clone();
        self.loading.dataset = true;
        self.spawn_message(async move {
            let result = async {
                let metadata = api.get_dataset(&dataset_id).await?;
                let keybindings = api.get_keybindings(&dataset_id, &user_id).await?;
                let stats = api.dataset_stats(&dataset_id).await?;
                Ok::<_, labello_client::ClientError>(LoadedDataset {
                    metadata,
                    keybindings,
                    stats,
                })
            }
            .await
            .map_err(|error| error.to_string());
            UiMessage::DatasetLoaded(result)
        });
    }

    pub(crate) fn request_admin_dataset(&mut self) {
        let Some(api) = self.runtime.api.clone() else {
            return;
        };
        let dataset_id = self.config.dataset_id.clone();
        self.loading.admin = true;
        self.spawn_message(async move {
            UiMessage::AdminLoaded(
                api.get_admin_dataset(&dataset_id)
                    .await
                    .map_err(|error| error.to_string()),
            )
        });
    }

    pub(crate) fn request_admin_save(&mut self) {
        let (Some(api), Some(metadata)) =
            (self.runtime.api.clone(), self.datasets.admin_config.clone())
        else {
            return;
        };
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

    pub(crate) fn request_ingest(&mut self) {
        let Some(api) = self.runtime.api.clone() else {
            return;
        };
        let dataset_id = self.config.dataset_id.clone();
        self.loading.ingesting = true;
        self.spawn_message(async move {
            UiMessage::IngestFinished(
                api.ingest_dataset(&dataset_id)
                    .await
                    .map_err(|error| error.to_string()),
            )
        });
    }

    pub(crate) fn request_stats(&mut self) {
        let Some(api) = self.runtime.api.clone() else {
            return;
        };
        let dataset_id = self.config.dataset_id.clone();
        self.loading.stats = true;
        self.datasets.last_stats_request = Some(std::time::Instant::now());
        self.spawn_message(async move {
            UiMessage::StatsLoaded(
                api.dataset_stats(&dataset_id)
                    .await
                    .map_err(|error| error.to_string()),
            )
        });
    }

    fn apply_loaded_dataset(&mut self, loaded: LoadedDataset) {
        self.classes = loaded.metadata.label_classes.clone();
        self.tasks = loaded.metadata.tasks.clone();
        self.datasets.metadata = Some(loaded.metadata);
        self.keybindings = loaded.keybindings;
        self.datasets.stats = loaded.stats;
        self.selected_task = self.selected_task.min(self.tasks.len().saturating_sub(1));
        self.view = AppView::Annotate;
        self.runtime.error = None;
        if self.current.is_none() && !self.tasks.is_empty() {
            self.request_next_image();
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
        #[cfg(not(target_arch = "wasm32"))]
        {
            drop(future);
            let _ = tx.send(UiMessage::DatasetList(Err(
                "live HTTP UI is available in the WASM build".to_string(),
            )));
        }
    }
}
