impl LabelloApp {
    pub(crate) fn request_import_capabilities(&mut self) {
        if self.runtime.api.is_none()
            || self.auth.account.is_none()
            || !self.auth.can_create_datasets
            || self.import_flow.capabilities_loading
            || self.import_flow.capabilities.is_some()
        {
            return;
        }
        self.import_flow.capabilities_loading = true;
        self.import_flow.capabilities_error = None;
        let request = self.import_request_identity(None);
        self.queue_command(UiCommand::ImportCapabilities { request });
    }

    pub(crate) fn refresh_import_if_due(&mut self) {
        let should_poll = self.import_flow.open
            && !self
                .import_flow
                .active_operations
                .values()
                .any(|activity| *activity == ImportActivity::LoadStatus)
            && self.import_flow.job.as_ref().is_some_and(|job| {
                matches!(
                    job.lifecycle,
                    ImportLifecycle::Preflighting
                        | ImportLifecycle::Building
                        | ImportLifecycle::Verifying
                        | ImportLifecycle::Committing
                )
            })
            && self
                .import_flow
                .poll_after
                .is_none_or(|deadline| web_time::Instant::now() >= deadline);
        if should_poll {
            self.request_import_poll();
        }
    }


    pub(crate) fn request_create_import(&mut self) {
        let Some(capabilities) = self.import_flow.capabilities.as_ref() else {
            return;
        };
        if !capabilities
            .profiles
            .iter()
            .any(|entry| entry.enabled && entry.profile == self.import_flow.profile)
            || !capabilities
                .transports
                .iter()
                .any(|entry| entry.enabled && entry.transport == self.import_flow.transport)
        {
            self.import_flow.error = Some(
                "The selected profile or transport is not advertised by the server.".to_string(),
            );
            return;
        }
        self.begin_import_epoch();
        self.import_flow.busy = true;
        self.import_flow.error = None;
        let source = match self.import_flow.transport {
            ImportTransport::BrowserFolder => ImportSourceSelection::BrowserFolder,
            ImportTransport::ServerDirectory => ImportSourceSelection::ServerDirectory {
                import_root_id: self.import_flow.server_root_id.clone(),
                relative_path: self.import_flow.server_relative_path.trim().to_string(),
            },
            ImportTransport::Unknown => return,
        };
        let request = self.import_request_identity(None);
        let key = import_key("create", request.request_id);
        self.queue_command(UiCommand::CreateImport {
            request,
            body: CreateImportRequest {
                destination_dataset_id: DatasetId::from(self.import_flow.destination_id.trim()),
                destination_name: self.import_flow.destination_name.trim().to_string(),
                profile: self.import_flow.profile,
                source,
                attestations: self.import_attestations(),
            },
            idempotency_key: key,
        });
    }

    pub(crate) fn request_seal_import(&mut self) {
        let Some(import_id) = self
            .import_flow
            .job
            .as_ref()
            .map(|job| job.import_id.clone())
        else {
            return;
        };
        if let Some(error) = self.import_descriptor_error() {
            self.import_flow.error = Some(error);
            return;
        }
        self.import_flow.busy = true;
        self.import_flow.error = None;
        let request = self.import_request_identity(Some(import_id.clone()));
        let key = import_key("seal", request.request_id);
        let yolo = !is_coco_profile(self.import_flow.profile);
        let selected_splits = if yolo {
            self.import_flow
                .yolo_splits
                .iter()
                .filter(|split| split.usable && split.selected)
                .map(|split| split.name.clone())
                .collect::<Vec<_>>()
        } else {
            self.import_flow
                .descriptors
                .iter()
                .map(|descriptor| descriptor.split.trim().to_string())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect()
        };
        let yolo_descriptor_split = selected_splits.first().cloned().unwrap_or_default();
        let descriptors = self
            .import_flow
            .descriptors
            .iter()
            .map(|descriptor| ImportDescriptorSelection {
                descriptor_file_id: descriptor.descriptor_file_id.trim().to_string(),
                kind: descriptor.kind,
                release: descriptor.release.trim().to_string(),
                split: if yolo {
                    yolo_descriptor_split.clone()
                } else {
                    descriptor.split.trim().to_string()
                },
                image_root_file_id: (!descriptor.image_root_file_id.trim().is_empty())
                    .then(|| descriptor.image_root_file_id.trim().to_string()),
                pairing_group: (!descriptor.pairing_group.trim().is_empty())
                    .then(|| descriptor.pairing_group.trim().to_string()),
            })
            .collect::<Vec<_>>();
        self.queue_command(UiCommand::SealImport {
            request,
            import_id,
            body: SealImportRequest {
                source: ImportSourceConfiguration {
                    source_namespace: self.import_flow.source_namespace.trim().to_string(),
                    descriptors,
                    selected_splits,
                    selected_category_keys: Vec::new(),
                },
                attestations: self.import_attestations(),
            },
            idempotency_key: key,
        });
    }

    pub(crate) fn request_yolo_descriptor_inspection(&mut self) {
        let Some(import_id) = self
            .import_flow
            .job
            .as_ref()
            .map(|job| job.import_id.clone())
        else {
            return;
        };
        let Some(descriptor_file_id) = self
            .import_flow
            .descriptors
            .first()
            .map(|descriptor| descriptor.descriptor_file_id.trim().to_string())
            .filter(|reference| !reference.is_empty())
        else {
            return;
        };
        if self.import_flow.yolo_inspection_loading {
            return;
        }
        self.import_flow.invalidate_yolo_inspection();
        self.import_flow.yolo_inspection_loading = true;
        let request = self.import_request_identity(Some(import_id.clone()));
        self.import_flow.pending_yolo_inspection_request_id = Some(request.request_id);
        self.queue_command(UiCommand::InspectYoloDescriptor {
            request,
            import_id,
            descriptor_file_id: descriptor_file_id.clone(),
            body: labello_client::InspectYoloDescriptorRequest { descriptor_file_id },
        });
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn request_yolo_descriptor_inspection_after_upload(&mut self) {
        if self.import_flow.yolo_inspection_loading {
            self.import_flow.yolo_inspection_retry_after_current = true;
        } else {
            self.request_yolo_descriptor_inspection();
        }
    }

    pub(crate) fn open_import_source_picker(&mut self, target: ImportSourcePickerTarget) {
        self.import_flow.source_picker = ImportSourcePickerState {
            target: Some(target),
            ..Default::default()
        };
        let initial_path = match target {
            ImportSourcePickerTarget::DatasetFolder => String::new(),
            ImportSourcePickerTarget::Descriptor(_)
            | ImportSourcePickerTarget::CocoImageRoot(_) => {
                match self.import_flow.server_relative_path.as_str() {
                    "" | "." => String::new(),
                    path => path.to_string(),
                }
            }
        };
        self.request_import_source_browse(initial_path, 0);
    }

    fn request_import_source_browse(&mut self, relative_path: String, offset: u32) {
        let Some(target) = self.import_flow.source_picker.target else {
            return;
        };
        if self.import_flow.source_picker.loading {
            return;
        }
        let request = match target {
            ImportSourcePickerTarget::DatasetFolder => self.import_request_identity(None),
            ImportSourcePickerTarget::Descriptor(_)
            | ImportSourcePickerTarget::CocoImageRoot(_) => {
                let import_id = self
                    .import_flow
                    .job
                    .as_ref()
                    .map(|job| job.import_id.clone());
                self.import_request_identity(import_id)
            }
        };
        self.import_flow.source_picker.loading = true;
        self.import_flow.source_picker.error = None;
        self.import_flow.source_picker.pending_request_id = Some(request.request_id);
        self.import_flow.source_picker.pending_append = offset > 0;
        if offset == 0 {
            self.import_flow.source_picker.relative_path = relative_path.clone();
        }
        match target {
            ImportSourcePickerTarget::DatasetFolder => {
                let root_id = self.import_flow.server_root_id.clone();
                if root_id.is_empty() {
                    self.import_flow.source_picker.loading = false;
                    self.import_flow.source_picker.error =
                        Some("Choose a server import root first.".to_string());
                    return;
                }
                self.queue_command(UiCommand::BrowseImportRoot {
                    request,
                    root_id,
                    body: labello_client::BrowseServerImportRootRequest {
                        relative_path,
                        offset,
                    },
                });
            }
            ImportSourcePickerTarget::Descriptor(_)
            | ImportSourcePickerTarget::CocoImageRoot(_) => {
                let Some(import_id) = self
                    .import_flow
                    .job
                    .as_ref()
                    .map(|job| job.import_id.clone())
                else {
                    self.import_flow.source_picker.loading = false;
                    return;
                };
                let mode = match target {
                    ImportSourcePickerTarget::Descriptor(_) => {
                        labello_client::ImportSourceBrowseMode::Descriptors
                    }
                    ImportSourcePickerTarget::CocoImageRoot(_) => {
                        labello_client::ImportSourceBrowseMode::Images
                    }
                    ImportSourcePickerTarget::DatasetFolder => unreachable!(),
                };
                self.queue_command(UiCommand::BrowseImportSource {
                    request,
                    import_id,
                    body: labello_client::BrowseImportSourceRequest {
                        relative_path,
                        offset,
                        mode,
                    },
                });
            }
        }
    }

    fn import_source_picker_modal(&mut self, ctx: &egui::Context) {
        let Some(target) = self.import_flow.source_picker.target else {
            return;
        };
        let screen = ctx.content_rect();
        let width = (screen.width() - 32.0).clamp(1.0, 680.0);
        let max_height = (screen.height() - 32.0).max(1.0);
        let page = self.import_flow.source_picker.page.clone();
        let requested_path = self.import_flow.source_picker.relative_path.clone();
        let loading = self.import_flow.source_picker.loading;
        let error = self.import_flow.source_picker.error.clone();
        let mut navigate = None;
        let mut select_folder = false;
        let mut selected_folder = None;
        let mut selected_file = None;
        let mut load_more = None;
        let mut close = false;
        let response = theme::modal(ctx, egui::Id::new("import-source-picker")).show(ctx, |ui| {
            ui.set_width(width);
            ui.set_max_height(max_height);
            ui.heading(match target {
                ImportSourcePickerTarget::DatasetFolder => "Choose dataset folder",
                ImportSourcePickerTarget::Descriptor(_) => "Choose descriptor file",
                ImportSourcePickerTarget::CocoImageRoot(_) => "Choose an image in the COCO root",
            });
            let current = page
                .as_ref()
                .map(|page| page.relative_path.as_str())
                .unwrap_or(&requested_path);
            ui.label(format!(
                "Current folder: {}",
                if current.is_empty() { "/" } else { current }
            ));
            ui.horizontal_wrapped(|ui| {
                if !current.is_empty() && ui.button("Up one folder").clicked() {
                    navigate = Some(parent_source_directory(current));
                }
                if target == ImportSourcePickerTarget::DatasetFolder
                    && theme::primary_button(ui, true, egui::Button::new("Select this folder"))
                        .clicked()
                {
                    select_folder = true;
                }
            });
            if let Some(error) = &error {
                theme::inline_message(ui, theme::Intent::Warning, error);
                if ui.button("Retry").clicked() {
                    navigate = Some(current.to_string());
                }
            }
            if loading && page.is_none() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Loading server source...");
                });
            }
            let entries = page
                .as_ref()
                .map(|page| page.entries.as_slice())
                .unwrap_or(&[]);
            egui::ScrollArea::vertical()
                .id_salt("import-source-picker-entries")
                .max_height((max_height - 180.0).max(1.0))
                .show(ui, |ui| {
                    if entries.is_empty() && !loading && error.is_none() {
                        ui.label("This folder has no matching entries.");
                    }
                    for entry in entries {
                        match entry.kind {
                            labello_client::ImportBrowseEntryKind::Directory => {
                                if target == ImportSourcePickerTarget::DatasetFolder {
                                    ui.horizontal_wrapped(|ui| {
                                        if ui
                                            .button(format!("Open folder {}", entry.name))
                                            .on_hover_text(&entry.relative_path)
                                            .clicked()
                                        {
                                            navigate = Some(entry.relative_path.clone());
                                        }
                                        if ui
                                            .button(format!("Select folder {}", entry.name))
                                            .on_hover_text(&entry.relative_path)
                                            .clicked()
                                        {
                                            selected_folder = Some(entry.relative_path.clone());
                                        }
                                    });
                                } else if ui
                                    .add_sized(
                                        [ui.available_width(), 44.0],
                                        egui::Button::new(format!("Open folder {}", entry.name)),
                                    )
                                    .on_hover_text(&entry.relative_path)
                                    .clicked()
                                {
                                    navigate = Some(entry.relative_path.clone());
                                }
                            }
                            labello_client::ImportBrowseEntryKind::File => {
                                if ui
                                    .add_sized(
                                        [ui.available_width(), 44.0],
                                        egui::Button::new(format!("Select {}", entry.name)),
                                    )
                                    .on_hover_text(&entry.relative_path)
                                    .clicked()
                                {
                                    selected_file = Some(entry.clone());
                                }
                            }
                        }
                    }
                });
            if let Some(offset) = page.as_ref().and_then(|page| page.next_offset)
                && ui
                    .add_enabled(!loading, egui::Button::new("Load more"))
                    .clicked()
            {
                load_more = Some((current.to_string(), offset));
            }
            if ui.button("Close picker").clicked() {
                close = true;
            }
        });
        response.response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Window, true, "Server source picker")
        });
        if select_folder {
            let selected = page
                .as_ref()
                .map(|page| page.relative_path.clone())
                .unwrap_or(requested_path);
            self.import_flow.server_relative_path = if selected.is_empty() {
                ".".to_string()
            } else {
                selected
            };
            close = true;
        }
        if let Some(selected) = selected_folder {
            self.import_flow.server_relative_path = selected;
            close = true;
        }
        if let Some(entry) = selected_file
            && let Some(file_id) = entry.file_id
        {
            let selected_path = RegisteredImportPath {
                client_file_id: String::new(),
                file_id: file_id.clone(),
                relative_path: entry.relative_path,
            };
            if let Some(existing) = self
                .import_flow
                .registered_paths
                .iter_mut()
                .find(|path| path.file_id == file_id)
            {
                *existing = selected_path;
            } else {
                self.import_flow.registered_paths.push(selected_path);
            }
            match target {
                ImportSourcePickerTarget::DatasetFolder => {}
                ImportSourcePickerTarget::Descriptor(index) => {
                    if let Some(descriptor) = self.import_flow.descriptors.get_mut(index) {
                        descriptor.descriptor_file_id = file_id;
                    }
                    if !is_coco_profile(self.import_flow.profile) {
                        self.import_flow.invalidate_yolo_inspection();
                        self.request_yolo_descriptor_inspection();
                    }
                }
                ImportSourcePickerTarget::CocoImageRoot(index) => {
                    if let Some(descriptor) = self.import_flow.descriptors.get_mut(index) {
                        descriptor.image_root_file_id = file_id;
                    }
                }
            }
            close = true;
        }
        if let Some(path) = navigate {
            self.import_flow.source_picker.page = None;
            self.request_import_source_browse(path, 0);
        } else if let Some((path, offset)) = load_more {
            self.request_import_source_browse(path, offset);
        }
        if close || response.should_close() {
            self.import_flow.source_picker = Default::default();
        }
    }

    pub(crate) fn request_preflight_import(&mut self, restart: bool) {
        let Some(import_id) = self
            .import_flow
            .job
            .as_ref()
            .map(|job| job.import_id.clone())
        else {
            return;
        };
        self.import_flow.busy = true;
        self.import_flow.screen = ImportScreen::Preflight;
        let request = self.import_request_identity(Some(import_id.clone()));
        let key = import_key("preflight", request.request_id);
        self.queue_command(UiCommand::PreflightImport {
            request,
            import_id,
            body: StartImportPreflightRequest { restart },
            idempotency_key: key,
        });
    }

    pub(crate) fn request_update_import_plan(&mut self) {
        let Some(import_id) = self
            .import_flow
            .job
            .as_ref()
            .map(|job| job.import_id.clone())
        else {
            return;
        };
        if !self.import_mappings_complete() {
            self.import_flow.error = Some(
                "Every discovered category needs one complete, uniquely keyed mapping.".to_string(),
            );
            return;
        }
        let body = self.import_plan_request();
        self.import_flow.busy = true;
        self.import_flow.plan = None;
        self.import_flow.accepted_plan_request = None;
        self.import_flow.diagnostics.clear();
        self.import_flow.diagnostics_cursor = None;
        self.import_flow.pending_plan_request = Some(body.clone());
        let request = self.import_request_identity(Some(import_id.clone()));
        let key = import_key("plan", request.request_id);
        self.queue_command(UiCommand::UpdateImportPlan {
            request,
            import_id,
            body,
            idempotency_key: key,
        });
    }

    pub(crate) fn request_commit_import(&mut self) {
        if !self.import_plan_is_current()
            || !self.import_plan_covers_all_categories()
            || !self.import_mappings_complete()
        {
            self.import_flow.error = Some(
                "Mappings changed or the accepted plan omits discovered categories/tasks. Save exact source mappings and wait for a complete matching plan before committing."
                    .to_string(),
            );
            return;
        }
        let Some((import_id, plan_hash)) = self
            .import_flow
            .plan
            .as_ref()
            .map(|plan| (plan.import_id.clone(), plan.plan_hash.clone()))
        else {
            return;
        };
        self.import_flow.busy = true;
        self.import_flow.screen = ImportScreen::Running;
        if let Some(job) = self.import_flow.job.as_mut() {
            job.lifecycle = ImportLifecycle::Building;
            job.progress.phase = labello_client::ImportProgressPhase::Build;
        }
        self.import_flow.poll_after =
            Some(web_time::Instant::now() + web_time::Duration::from_millis(500));
        let request = self.import_request_identity(Some(import_id.clone()));
        let key = import_key("commit", request.request_id);
        self.queue_command(UiCommand::CommitImport {
            request,
            import_id,
            body: CommitImportRequest { plan_hash },
            idempotency_key: key,
        });
    }

    pub(crate) fn request_cancel_import(&mut self) {
        let Some(import_id) = self
            .import_flow
            .job
            .as_ref()
            .map(|job| job.import_id.clone())
        else {
            self.begin_import_epoch();
            self.import_flow.reset_job();
            return;
        };
        self.import_flow.busy = true;
        let request = self.import_request_identity(Some(import_id.clone()));
        let key = import_key("cancel", request.request_id);
        self.queue_command(UiCommand::CancelImport {
            request,
            import_id,
            idempotency_key: key,
        });
    }

    pub(crate) fn request_import_poll(&mut self) {
        let Some(import_id) = self
            .import_flow
            .job
            .as_ref()
            .map(|job| job.import_id.clone())
        else {
            return;
        };
        self.import_flow.poll_after = None;
        let request = self.import_request_identity(Some(import_id.clone()));
        self.queue_command(UiCommand::GetImport { request, import_id });
    }

    pub(crate) fn request_import_diagnostics(&mut self, restart: bool) {
        let Some(import_id) = self
            .import_flow
            .job
            .as_ref()
            .map(|job| job.import_id.clone())
        else {
            return;
        };
        if restart {
            self.import_flow.diagnostics.clear();
            self.import_flow.diagnostics_cursor = None;
        }
        self.import_flow.busy = true;
        let request = self.import_request_identity(Some(import_id.clone()));
        self.queue_command(UiCommand::ImportDiagnostics {
            request,
            import_id,
            query: labello_client::ImportDiagnosticsQuery {
                cursor: self.import_flow.diagnostics_cursor.clone(),
                limit: self
                    .import_flow
                    .capabilities
                    .as_ref()
                    .map(|capabilities| capabilities.limits.max_diagnostic_page_size.min(100))
                    .unwrap_or(100),
                code: None,
                severity: None,
            },
        });
    }

    pub(crate) fn request_import_recovery(&mut self) {
        self.begin_import_epoch();
        let recovery_import_id = self.import_flow.recovery_import_id.trim().to_string();
        self.import_flow.reset_job();
        self.import_flow.recovery_import_id = recovery_import_id.clone();
        let import_id = labello_domain::ImportId::from(recovery_import_id);
        self.import_flow.busy = true;
        let request = self.import_request_identity(Some(import_id.clone()));
        self.queue_command(UiCommand::GetImport { request, import_id });
    }

    fn request_retry_import(&mut self) {
        let phase = self
            .import_flow
            .job
            .as_ref()
            .and_then(|job| job.failure.as_ref())
            .map(|failure| failure.phase);
        if matches!(
            phase,
            Some(
                labello_client::ImportProgressPhase::Build
                    | labello_client::ImportProgressPhase::Verification
                    | labello_client::ImportProgressPhase::Commit
            )
        ) && self.import_flow.plan.is_some()
        {
            self.request_commit_import();
        } else {
            self.request_preflight_import(true);
        }
    }

    pub(crate) fn request_import_folder_selection(&mut self) {
        #[cfg(target_arch = "wasm32")]
        {
            let Some(import_id) = self
                .import_flow
                .job
                .as_ref()
                .map(|job| job.import_id.clone())
            else {
                return;
            };
            let request = self.import_request_identity(Some(import_id));
            self.runtime.active_requests.insert(request.request_id);
            self.import_flow
                .active_operations
                .insert(request.request_id, ImportActivity::SelectFolder);
            self.import_flow.busy = true;
            let limits = self
                .import_flow
                .capabilities
                .as_ref()
                .map(|capabilities| capabilities.limits.clone())
                .unwrap_or_default();
            if let Err(error) =
                crate::import_flow::browser::pick_import_folder(self, request.clone(), limits)
            {
                self.runtime.active_requests.remove(&request.request_id);
                self.import_flow
                    .active_operations
                    .remove(&request.request_id);
                self.import_flow.busy = false;
                self.import_flow.error = Some(error);
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.import_flow.error =
                Some("Browser folder selection is available in the WebAssembly build.".to_string());
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn register_selected_import_files(&mut self, files: Vec<BrowserImportFile>) {
        let Some(import_id) = self
            .import_flow
            .job
            .as_ref()
            .map(|job| job.import_id.clone())
        else {
            return;
        };
        let limits = self
            .import_flow
            .capabilities
            .as_ref()
            .map(|capabilities| capabilities.limits.clone())
            .unwrap_or_default();
        let total_bytes = files.iter().map(|file| file.byte_size).sum::<u64>();
        if files.is_empty()
            || files.len() as u64 > limits.max_browser_files
            || total_bytes > limits.max_browser_bytes
            || files
                .iter()
                .any(|file| file.byte_size > limits.max_single_file_bytes)
        {
            self.import_flow.busy = false;
            self.import_flow.error = Some(
                "Selected folder is empty or exceeds the advertised browser import limits."
                    .to_string(),
            );
            return;
        }
        self.import_flow.browser_files = files
            .iter()
            .map(|file| (file.client_file_id.clone(), file.file.clone()))
            .collect();
        self.import_flow.registered_paths = files
            .iter()
            .map(|file| RegisteredImportPath {
                client_file_id: file.client_file_id.clone(),
                file_id: String::new(),
                relative_path: file.relative_path.clone(),
            })
            .collect();
        let body = labello_client::RegisterImportFilesRequest {
            files: files
                .into_iter()
                .map(|file| labello_client::ImportFileRegistration {
                    client_file_id: file.client_file_id,
                    relative_path: file.relative_path,
                    byte_size: file.byte_size,
                    blake3: Some(file.blake3),
                })
                .collect(),
        };
        let request = self.import_request_identity(Some(import_id.clone()));
        let key = import_key("register", request.request_id);
        self.import_flow.busy = true;
        self.queue_command(UiCommand::RegisterImportFiles {
            request,
            import_id,
            body,
            idempotency_key: key,
        });
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn upload_next_import_chunk(&mut self) {
        let Some(import_id_value) = self
            .import_flow
            .job
            .as_ref()
            .map(|job| job.import_id.clone())
        else {
            return;
        };
        let Some(file) = self
            .import_flow
            .browser_uploads
            .iter()
            .find(|file| !file.complete)
            .cloned()
        else {
            self.import_flow.busy = false;
            return;
        };
        let Some(source) = self
            .import_flow
            .browser_files
            .get(&file.client_file_id)
            .cloned()
        else {
            self.import_flow.busy = false;
            self.import_flow.error = Some(
                "Upload source is no longer selected. Reselect the same folder to continue."
                    .to_string(),
            );
            return;
        };
        let Some(uploader) = self.runtime.import_chunk_uploader.clone() else {
            self.import_flow.busy = false;
            self.import_flow.error =
                Some("Raw browser import transport is unavailable.".to_string());
            return;
        };
        let Some(csrf_token) = self.runtime.api.as_ref().and_then(|api| api.csrf_token()) else {
            self.import_flow.busy = false;
            self.import_flow.error =
                Some("Import upload requires an authenticated session.".to_string());
            return;
        };
        let chunk_bytes = self
            .import_flow
            .capabilities
            .as_ref()
            .map(|capabilities| capabilities.limits.upload_chunk_bytes)
            .unwrap_or(8 * 1024 * 1024)
            .max(1);
        let offset = file.accepted_bytes;
        let length = (file.byte_size - offset).min(chunk_bytes);
        let request = self.import_request_identity(Some(import_id_value.clone()));
        self.runtime.active_requests.insert(request.request_id);
        self.import_flow
            .active_operations
            .insert(request.request_id, ImportActivity::UploadChunk);
        self.import_flow.busy = true;
        let api_base_url = self.config.api_base_url.clone();
        let import_id = import_id_value.to_string();
        let file_id = file.file_id.clone();
        let idempotency_key = import_key("chunk", request.request_id);
        self.spawn_import_message(async move {
            let result = async {
                let bytes = browser::read_file_range(&source, offset, length).await?;
                let digest = blake3::hash(&bytes).to_hex().to_string();
                uploader(RawImportChunkRequest {
                    api_base_url,
                    import_id,
                    file_id: file_id.clone(),
                    offset,
                    length,
                    digest,
                    bytes,
                    csrf_token,
                    idempotency_key,
                })
                .await
            }
            .await;
            crate::app::UiMessage::ImportChunkUploaded {
                request,
                file_id,
                result,
            }
        });
    }


    fn restart_import_setup(&mut self) {
        self.begin_import_epoch();
        self.import_flow.reset_job();
        self.import_flow.open = true;
    }
}
