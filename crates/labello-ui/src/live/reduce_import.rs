impl LabelloApp {
    fn reduce_import_message(
        &mut self,
        _ctx: &egui::Context,
        message: UiMessage,
    ) -> Option<UiMessage> {
        match message {
                UiMessage::ImportCapabilitiesLoaded { result, .. } => {
                    self.import.capabilities_loading = false;
                    match result {
                        Ok(capabilities) => {
                            self.import
                                .normalize_capability_selection(&capabilities);
                            self.import.capabilities = Some(capabilities);
                            self.import.capabilities_error = None;
                        }
                        Err(error) => self.import.capabilities_error = Some(error),
                    }
                }
                UiMessage::ImportJobLoaded { result, .. } => {
                    self.sync_import_busy();
                    match *result {
                        Ok(job) => {
                            let job_changed = self
                                .import
                                .job
                                .as_ref()
                                .is_none_or(|current| current.import_id != job.import_id);
                            let recovered = job_changed
                                && self.import.recovery_import_id == job.import_id.as_str();
                            if recovered {
                                self.import.pending_plan_request = None;
                            }
                            self.import.hydrate_job_contract(&job);
                            if recovered && job.recovery.is_none() {
                                self.import.recovery_contract_gap = true;
                            }
                            let load_diagnostics = job.lifecycle
                                == labello_client::ImportLifecycle::AwaitingDecision
                                && self.import.diagnostics.is_empty();
                            self.import.recovery_import_id = job.import_id.to_string();
                            self.import.screen = crate::import_flow::import_screen(
                                &job,
                                self.import.plan.as_ref(),
                            );
                            self.import.error = None;
                            let polling = matches!(
                                job.lifecycle,
                                labello_client::ImportLifecycle::Preflighting
                                    | labello_client::ImportLifecycle::Building
                                    | labello_client::ImportLifecycle::Verifying
                                    | labello_client::ImportLifecycle::Committing
                            );
                            self.import.poll_after =
                                polling.then(|| Instant::now() + Duration::from_millis(500));
                            self.import.job = Some(job);
                            if load_diagnostics {
                                self.request_import_diagnostics(true);
                            }
                        }
                        Err(error) => self.import.error = Some(error),
                    }
                }
                #[cfg(target_arch = "wasm32")]
                UiMessage::ImportBrowserFilesSelected { result, .. } => {
                    self.sync_import_busy();
                    match result {
                        Ok(files) => self.register_selected_import_files(files),
                        Err(error) => self.import.error = Some(error),
                    }
                }
                UiMessage::ImportFilesRegistered { result, .. } => {
                    self.sync_import_busy();
                    match result {
                        Ok(registered) => {
                            if let Some(job) = self.import.job.as_mut() {
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
                                        .import
                                        .registered_paths
                                        .iter_mut()
                                        .find(|path| path.client_file_id == file.client_file_id)
                                    {
                                        path.file_id = file.file_id.clone();
                                    }
                                    for descriptor in &mut self.import.descriptors {
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
                                        self.import.profile,
                                        labello_client::ImportProfile::UltralyticsYoloDetectV1
                                            | labello_client::ImportProfile::UltralyticsYoloPoseV1
                                    )
                                {
                                    self.import.invalidate_yolo_inspection();
                                }
                                let inspect_completed_yolo = yolo_descriptor_reference_changed
                                    && self.import.descriptors.first().is_some_and(
                                        |descriptor| {
                                            registered.files.iter().any(|file| {
                                                file.file_id == descriptor.descriptor_file_id
                                                    && file.complete
                                            })
                                        },
                                    );
                                self.import.browser_uploads = registered.files;
                                self.upload_next_import_chunk();
                                if inspect_completed_yolo {
                                    self.request_yolo_descriptor_inspection_after_upload();
                                }
                            }
                        }
                        Err(error) => self.import.error = Some(error),
                    }
                }
                UiMessage::ImportSourceBrowsed { request, result } => {
                    if self.import.source_picker.pending_request_id != Some(request.request_id)
                    {
                        return None;
                    }
                    self.import.source_picker.pending_request_id = None;
                    self.import.source_picker.loading = false;
                    match result {
                        Ok(mut page) => {
                            if self.import.source_picker.pending_append
                                && let Some(current) = self.import.source_picker.page.as_mut()
                                && current.relative_path == page.relative_path
                            {
                                current.entries.append(&mut page.entries);
                                current.next_offset = page.next_offset;
                            } else {
                                self.import.source_picker.page = Some(page);
                            }
                            self.import.source_picker.error = None;
                        }
                        Err(error) => self.import.source_picker.error = Some(error),
                    }
                    self.import.source_picker.pending_append = false;
                }
                UiMessage::YoloDescriptorInspected {
                    request,
                    descriptor_file_id,
                    result,
                } => {
                    let current_descriptor = self
                        .import
                        .descriptors
                        .first()
                        .map(|descriptor| descriptor.descriptor_file_id.trim());
                    if self.import.pending_yolo_inspection_request_id
                        != Some(request.request_id)
                        || current_descriptor != Some(descriptor_file_id.trim())
                    {
                        return None;
                    }
                    self.import.pending_yolo_inspection_request_id = None;
                    self.import.yolo_inspection_loading = false;
                    match result {
                        Ok(inspection) => {
                            self.import.yolo_splits = inspection
                                .splits
                                .into_iter()
                                .map(|split| crate::import_flow::ImportYoloSplitDraft {
                                    name: split.name,
                                    usable: split.usable,
                                    selected: split.usable,
                                    issue: split.issue,
                                })
                                .collect();
                            self.import.yolo_inspected_descriptor_file_id =
                                Some(descriptor_file_id);
                            self.import.yolo_inspection_error = self
                                .import
                                .yolo_splits
                                .iter()
                                .all(|split| !split.usable)
                                .then(|| {
                                    "The YAML does not define a usable train, val, or test split."
                                        .to_string()
                                });
                        }
                        Err(error) => {
                            self.import.yolo_splits.clear();
                            self.import.yolo_inspected_descriptor_file_id = None;
                            self.import.yolo_inspection_error = Some(error);
                        }
                    }
                    if self.import.yolo_inspection_retry_after_current {
                        self.import.yolo_inspection_retry_after_current = false;
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
                                        self.import.profile,
                                        labello_client::ImportProfile::UltralyticsYoloDetectV1
                                            | labello_client::ImportProfile::UltralyticsYoloPoseV1
                                    )
                                    && self.import.descriptors.first().is_some_and(
                                        |descriptor| descriptor.descriptor_file_id == _file_id,
                                    )
                                    && self.import.yolo_inspected_descriptor_file_id.is_none();
                            #[cfg(target_arch = "wasm32")]
                            if let Some(file) = self
                                .import
                                .browser_uploads
                                .iter_mut()
                                .find(|file| file.file_id == _file_id)
                            {
                                file.accepted_bytes = _chunk.accepted_offset;
                                file.complete = _chunk.complete;
                            }
                            if let Some(_job) = self.import.job.as_mut() {
                                #[cfg(target_arch = "wasm32")]
                                {
                                    _job.progress.uploaded_files =
                                        self.import
                                            .browser_uploads
                                            .iter()
                                            .filter(|file| file.complete)
                                            .count() as u64;
                                    _job.progress.accepted_bytes = self
                                        .import
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
                        Err(error) => self.import.error = Some(error),
                    }
                }
                UiMessage::ImportSealed { result, .. } => {
                    self.sync_import_busy();
                    match result {
                        Ok(sealed) => {
                            if let Some(job) = self.import.job.as_mut() {
                                job.lifecycle = labello_client::ImportLifecycle::Sealed;
                                job.source_fingerprint = Some(sealed.source_fingerprint);
                                job.progress.total_files = sealed.files;
                                job.progress.total_bytes = sealed.bytes;
                            }
                            self.request_preflight_import(false);
                        }
                        Err(error) => self.import.error = Some(error),
                    }
                }
                UiMessage::ImportPlanUpdated { result, .. } => {
                    self.sync_import_busy();
                    match *result {
                        Ok(plan) => {
                            let requested = self.import.pending_plan_request.take();
                            if requested.as_ref() != plan.accepted_request.as_ref() {
                                self.import.plan = None;
                                self.import.accepted_plan_request = None;
                                self.import.error = Some(
                                    "The server returned a plan for different mapping inputs. Save the current mappings again before commit."
                                        .to_string(),
                                );
                                return None;
                            }
                            self.import.accepted_plan_request = plan.accepted_request.clone();
                            self.import.screen = if plan.commit_ready {
                                crate::import_flow::ImportScreen::Ready
                            } else {
                                crate::import_flow::ImportScreen::Preflight
                            };
                            if let Some(job) = self.import.job.as_mut() {
                                job.plan_hash = Some(plan.plan_hash.clone());
                                job.preflight_report = Some(plan.report.clone());
                            }
                            self.import.plan = Some(plan);
                            self.import.error = None;
                            self.request_import_diagnostics(true);
                        }
                        Err(error) => {
                            self.import.pending_plan_request = None;
                            self.import.error = Some(error);
                        }
                    }
                }
                UiMessage::ImportDiagnosticsLoaded { result, .. } => {
                    self.sync_import_busy();
                    match result {
                        Ok(page) => {
                            self.import.diagnostics.extend(page.diagnostics);
                            self.import.diagnostics_cursor = page.next_cursor;
                        }
                        Err(error) => self.import.error = Some(error),
                    }
                }
                UiMessage::ImportCommitted { result, .. } => {
                    self.sync_import_busy();
                    self.begin_import_epoch();
                    match result {
                        Ok(committed) => {
                            if let Some(job) = self.import.job.as_mut() {
                                job.lifecycle = labello_client::ImportLifecycle::Succeeded;
                                job.destination_dataset_id = committed.dataset_id;
                                job.plan_hash = Some(committed.plan_hash);
                            }
                            self.import.screen = crate::import_flow::ImportScreen::Success;
                            self.import.error = None;
                            self.runtime.notice = Some(if committed.recovered {
                                "Recovered and completed the import".to_string()
                            } else {
                                "Dataset import completed".to_string()
                            });
                            self.request_dataset_list();
                        }
                        Err(error) => {
                            self.import.screen = crate::import_flow::ImportScreen::Failure;
                            self.import.error = Some(error);
                        }
                    }
                }
                UiMessage::ImportCancelled { result, .. } => {
                    self.sync_import_busy();
                    self.begin_import_epoch();
                    match result {
                        Ok(cancelled) => {
                            if let Some(job) = self.import.job.as_mut() {
                                job.lifecycle = cancelled.lifecycle;
                            }
                            self.import.screen = crate::import_flow::ImportScreen::Failure;
                            self.runtime.notice = Some("Import cancelled".to_string());
                        }
                        Err(error) => self.import.error = Some(error),
                    }
                }
            message => return Some(message),
        }
        None
    }
}
