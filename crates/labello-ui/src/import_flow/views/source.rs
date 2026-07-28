impl LabelloApp {
    fn import_source_step(&mut self, ui: &mut egui::Ui, capabilities: &ImportCapabilities) {
        self.import_flow
            .normalize_capability_selection(capabilities);
        ui.label(RichText::new("Destination").strong());
        theme::labeled_text_field(
            ui,
            "Dataset ID",
            &mut self.import_flow.destination_id,
            theme::COMPACT_TEXT_FIELD_HEIGHT,
        );
        theme::labeled_text_field(
            ui,
            "Dataset name",
            &mut self.import_flow.destination_name,
            theme::COMPACT_TEXT_FIELD_HEIGHT,
        );
        ui.label(RichText::new("Source profile").strong());
        let previous_profile = self.import_flow.profile;
        egui::ComboBox::from_label("Import profile")
            .selected_text(profile_label(self.import_flow.profile))
            .show_ui(ui, |ui| {
                for profile in capabilities
                    .profiles
                    .iter()
                    .filter(|profile| profile.enabled)
                {
                    ui.selectable_value(
                        &mut self.import_flow.profile,
                        profile.profile,
                        if profile.display_name.is_empty() {
                            profile_label(profile.profile)
                        } else {
                            &profile.display_name
                        },
                    );
                }
            });
        if self.import_flow.profile != previous_profile {
            self.import_flow.plan = None;
            self.import_flow.accepted_plan_request = None;
            self.import_flow.categories.clear();
            self.import_flow.descriptors = vec![descriptor_draft(self.import_flow.profile)];
            self.import_flow.invalidate_yolo_inspection();
        }
        ui.label(RichText::new("Transport").strong());
        let previous_transport = self.import_flow.transport;
        for transport in capabilities
            .transports
            .iter()
            .filter(|transport| transport.enabled)
        {
            ui.radio_value(
                &mut self.import_flow.transport,
                transport.transport,
                transport_label(transport.transport),
            );
        }
        if self.import_flow.transport != previous_transport {
            self.import_flow.registered_paths.clear();
            self.import_flow.descriptors = vec![descriptor_draft(self.import_flow.profile)];
            self.import_flow.invalidate_yolo_inspection();
        }
        if self.import_flow.transport == ImportTransport::ServerDirectory {
            let previous_root = self.import_flow.server_root_id.clone();
            egui::ComboBox::from_label("Server import root")
                .selected_text(
                    capabilities
                        .server_roots
                        .iter()
                        .find(|root| root.root_id == self.import_flow.server_root_id)
                        .map(|root| root.display_name.as_str())
                        .unwrap_or("Choose a root"),
                )
                .show_ui(ui, |ui| {
                    for root in &capabilities.server_roots {
                        ui.selectable_value(
                            &mut self.import_flow.server_root_id,
                            root.root_id.clone(),
                            &root.display_name,
                        );
                    }
                });
            if self.import_flow.server_root_id != previous_root {
                self.import_flow.server_relative_path.clear();
                self.import_flow.source_picker = Default::default();
            }
            status_row(
                ui,
                "Dataset folder",
                match self.import_flow.server_relative_path.as_str() {
                    "" => "Not selected".to_string(),
                    "." => "/".to_string(),
                    path => path.to_string(),
                },
            );
            if ui
                .add_enabled(
                    !self.import_flow.server_root_id.is_empty(),
                    egui::Button::new(if self.import_flow.server_relative_path.is_empty() {
                        "Choose dataset folder"
                    } else {
                        "Change dataset folder"
                    }),
                )
                .clicked()
            {
                self.open_import_source_picker(ImportSourcePickerTarget::DatasetFolder);
            }
        } else {
            theme::inline_message(
                ui,
                theme::Intent::Info,
                "The browser folder is selected after the import is registered. Reselect the same folder to resume an interrupted upload.",
            );
        }
        ui.label(RichText::new("Attestations").strong());
        ui.checkbox(
            &mut self.import_flow.ground_truth,
            "I attest that these labels are ground truth",
        );
        ui.checkbox(
            &mut self.import_flow.exhaustive,
            "I attest that labels are exhaustive for the stated coverage",
        );
        theme::labeled_text_field(
            ui,
            "Coverage scope (comma separated)",
            &mut self.import_flow.coverage_scope,
            theme::COMPACT_TEXT_FIELD_HEIGHT,
        );
        theme::labeled_text_field(
            ui,
            "Provenance",
            &mut self.import_flow.provenance,
            theme::COMPACT_TEXT_FIELD_HEIGHT,
        );
        let dataset_id = DatasetId::from(self.import_flow.destination_id.trim());
        let valid = dataset_id.validate_path_segment().is_ok()
            && !self.import_flow.destination_name.trim().is_empty()
            && self.import_flow.ground_truth
            && !self.import_flow.provenance.trim().is_empty()
            && (self.import_flow.transport == ImportTransport::BrowserFolder
                || (!self.import_flow.server_root_id.is_empty()
                    && !self.import_flow.server_relative_path.trim().is_empty()));
        if theme::primary_button(
            ui,
            valid && !self.import_flow.busy,
            egui::Button::new("Register import"),
        )
        .on_disabled_hover_text("Complete the destination, source, and required attestations.")
        .clicked()
        {
            self.request_create_import();
        }
    }

    fn import_transport_step(&mut self, ui: &mut egui::Ui) {
        if let Some(job) = &self.import_flow.job {
            status_row(ui, "Import", job.import_id.to_string());
            status_row(ui, "Status", lifecycle_label(job.lifecycle));
            if job.transport == ImportTransport::BrowserFolder {
                status_row(
                    ui,
                    "Upload progress",
                    format!(
                        "{} of {} files, {} of {} bytes",
                        job.progress.uploaded_files,
                        job.progress.total_files,
                        job.progress.accepted_bytes,
                        job.progress.total_bytes
                    ),
                );
                if ui
                    .add_enabled(
                        !self.import_flow.busy,
                        egui::Button::new(if job.progress.registered_files == 0 {
                            "Choose folder and upload"
                        } else {
                            "Reselect folder and resume"
                        }),
                    )
                    .clicked()
                {
                    self.request_import_folder_selection();
                }
            }
        }
        ui.separator();
        ui.label(RichText::new("Source configuration").strong());
        theme::labeled_text_field(
            ui,
            "Source namespace",
            &mut self.import_flow.source_namespace,
            theme::COMPACT_TEXT_FIELD_HEIGHT,
        );
        let browser_paths = self.import_flow.registered_paths.clone();
        let browser_transport = self.import_flow.transport == ImportTransport::BrowserFolder;
        let profile = self.import_flow.profile;
        let coco = is_coco_profile(profile);
        let mut open_picker = None;
        if coco {
            let descriptor_count = self.import_flow.descriptors.len();
            let mut remove = None;
            for (index, descriptor) in self.import_flow.descriptors.iter_mut().enumerate() {
                ui.push_id(("import-descriptor", index), |ui| {
                    ui.label(RichText::new(format!("Descriptor {}", index + 1)).strong());
                    egui::ComboBox::from_label("Descriptor kind")
                        .selected_text(descriptor_kind_label(descriptor.kind))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut descriptor.kind,
                                ImportDescriptorKind::CocoInstances,
                                descriptor_kind_label(ImportDescriptorKind::CocoInstances),
                            );
                            if profile == ImportProfile::CocoKeypointsGtV1 {
                                ui.selectable_value(
                                    &mut descriptor.kind,
                                    ImportDescriptorKind::CocoKeypoints,
                                    descriptor_kind_label(ImportDescriptorKind::CocoKeypoints),
                                );
                            }
                        });
                    if browser_transport {
                        source_file_selector(
                            ui,
                            "Descriptor file",
                            &mut descriptor.descriptor_file_id,
                            &browser_paths,
                            |path| descriptor_path_matches(profile, path),
                        );
                    } else if server_source_file_picker(
                        ui,
                        "Descriptor file",
                        &descriptor.descriptor_file_id,
                        &browser_paths,
                        "Choose descriptor file",
                    ) {
                        open_picker = Some(ImportSourcePickerTarget::Descriptor(index));
                    }
                    theme::labeled_text_field(
                        ui,
                        "Release",
                        &mut descriptor.release,
                        theme::COMPACT_TEXT_FIELD_HEIGHT,
                    );
                    theme::labeled_text_field(
                        ui,
                        "Split",
                        &mut descriptor.split,
                        theme::COMPACT_TEXT_FIELD_HEIGHT,
                    );
                    theme::labeled_text_field(
                        ui,
                        "Pairing group (optional)",
                        &mut descriptor.pairing_group,
                        theme::COMPACT_TEXT_FIELD_HEIGHT,
                    );
                    if browser_transport {
                        source_file_selector(
                            ui,
                            "Exact COCO image root",
                            &mut descriptor.image_root_file_id,
                            &browser_paths,
                            is_image_path,
                        );
                    } else if server_source_file_picker(
                        ui,
                        "Exact COCO image root",
                        &descriptor.image_root_file_id,
                        &browser_paths,
                        "Choose image in root",
                    ) {
                        open_picker = Some(ImportSourcePickerTarget::CocoImageRoot(index));
                    }
                    ui.small(
                        "Select a registered image directly inside the exact root referenced by COCO file_name values.",
                    );
                    if descriptor_count > 1 && ui.button("Remove descriptor").clicked() {
                        remove = Some(index);
                    }
                    ui.separator();
                });
            }
            if let Some(index) = remove {
                self.import_flow.descriptors.remove(index);
                self.import_flow.source_picker = Default::default();
            }
            if ui.button("Add COCO descriptor").clicked() {
                self.import_flow
                    .descriptors
                    .push(descriptor_draft(self.import_flow.profile));
            }
        } else {
            if self.import_flow.descriptors.len() != 1 {
                self.import_flow.descriptors = vec![descriptor_draft(profile)];
                self.import_flow.invalidate_yolo_inspection();
            }
            let mut descriptor_changed = false;
            let mut inspect_after_edit = false;
            if let Some(descriptor) = self.import_flow.descriptors.first_mut() {
                ui.label(RichText::new("YOLO source").strong());
                let previous = descriptor.descriptor_file_id.clone();
                if browser_transport {
                    inspect_after_edit = source_file_selector(
                        ui,
                        "Dataset YAML",
                        &mut descriptor.descriptor_file_id,
                        &browser_paths,
                        |path| descriptor_path_matches(profile, path),
                    );
                } else if server_source_file_picker(
                    ui,
                    "Dataset YAML",
                    &descriptor.descriptor_file_id,
                    &browser_paths,
                    "Choose descriptor file",
                ) {
                    open_picker = Some(ImportSourcePickerTarget::Descriptor(0));
                }
                descriptor_changed = previous != descriptor.descriptor_file_id;
                theme::labeled_text_field(
                    ui,
                    "Release",
                    &mut descriptor.release,
                    theme::COMPACT_TEXT_FIELD_HEIGHT,
                );
            }
            if descriptor_changed {
                self.import_flow.invalidate_yolo_inspection();
            }
            if inspect_after_edit {
                self.request_yolo_descriptor_inspection();
            }
            let descriptor_selected = self
                .import_flow
                .descriptors
                .first()
                .is_some_and(|descriptor| !descriptor.descriptor_file_id.trim().is_empty());
            let inspect_label = if self.import_flow.yolo_inspection_error.is_some() {
                "Retry split inspection"
            } else if self.import_flow.yolo_inspected_descriptor_file_id.is_some() {
                "Refresh splits"
            } else {
                "Inspect YAML splits"
            };
            if ui
                .add_enabled(
                    descriptor_selected && !self.import_flow.yolo_inspection_loading,
                    egui::Button::new(inspect_label),
                )
                .clicked()
            {
                self.request_yolo_descriptor_inspection();
            }
            ui.label(RichText::new("Splits to import").strong());
            if self.import_flow.yolo_inspection_loading {
                ui.small("Descriptor inspection is in progress.");
            }
            for split in &mut self.import_flow.yolo_splits {
                ui.add_enabled(
                    split.usable,
                    egui::Checkbox::new(&mut split.selected, &split.name),
                );
                if let Some(issue) = &split.issue {
                    ui.small(issue);
                }
            }
            if descriptor_selected
                && !self.import_flow.yolo_inspection_loading
                && self.import_flow.yolo_splits.is_empty()
                && self.import_flow.yolo_inspection_error.is_none()
            {
                ui.small("Inspect the YAML to discover its train, val, and test splits.");
            }
            ui.separator();
        }
        if let Some(target) = open_picker {
            self.open_import_source_picker(target);
        }
        let descriptor_error = self.import_descriptor_error();
        if let Some(error) = &descriptor_error {
            theme::inline_message(ui, theme::Intent::Warning, error);
        }
        if theme::primary_button(
            ui,
            !self.import_flow.busy && descriptor_error.is_none(),
            egui::Button::new("Seal source and run preflight"),
        )
        .clicked()
        {
            self.request_seal_import();
        }
        if ui
            .add_enabled(!self.import_flow.busy, egui::Button::new("Cancel import"))
            .clicked()
        {
            self.request_cancel_import();
        }
    }

}
