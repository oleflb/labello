impl LabelloApp {
    pub(crate) fn import_setup_section(&mut self, ui: &mut egui::Ui) {
        if !self.import.open {
            self.import.open = true;
            if self.import.destination_id.is_empty() {
                self.import.destination_id = "imported-dataset".to_string();
                self.import.destination_name = "Imported dataset".to_string();
            }
        }
        self.import.normalize_mapping_draft();
        self.import.sync_seed_workflow_confirmation_scope();
        self.sync_import_decision_screen();
        ui.heading("Import dataset");
        ui.label(
            RichText::new("Register, validate, and import an existing dataset.")
                .color(theme::TEXT_MUTED),
        );
        ui.add_space(theme::SPACE_2);
        self.request_import_capabilities();
        theme::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            self.import_progress_overview(ui);
            ui.separator();
            let Some(capabilities) = self.import.capabilities.clone() else {
                if self.import.capabilities_loading {
                    ui.small("Checking dataset import capability...");
                } else if let Some(error) = self.import.capabilities_error.clone() {
                    theme::inline_message(ui, theme::Intent::Warning, error);
                }
                return;
            };
            if !capabilities.available {
                if let Some(reason) = capabilities.unavailable_reason {
                    ui.small(format!("Dataset import unavailable: {reason}"));
                }
                return;
            }
            self.import_flow_contents(ui, &capabilities);
        });
        self.import_source_picker_modal(ui.ctx());
    }

    fn sync_import_decision_screen(&mut self) {
        if self.import.pending_plan_request.is_some()
            || !self
                .import
                .job
                .as_ref()
                .is_some_and(|job| job.lifecycle == ImportLifecycle::AwaitingDecision)
            || self.import.plan.is_none()
            || !matches!(
                self.import.screen,
                ImportScreen::Preflight | ImportScreen::Ready
            )
        {
            return;
        }
        let ready = self
            .import
            .plan
            .as_ref()
            .is_some_and(|plan| plan.commit_ready)
            && self.import_plan_is_current()
            && self.import_mapping_validation().is_valid()
            && self.import_plan_covers_all_categories();
        self.import.screen = if ready {
            ImportScreen::Ready
        } else {
            ImportScreen::Preflight
        };
    }

    fn import_flow_contents(&mut self, ui: &mut egui::Ui, capabilities: &ImportCapabilities) {
        if let Some(error) = self.import.error.clone() {
            theme::inline_message(ui, theme::Intent::Error, error);
        }
        if self.import.recovery_contract_gap
            && matches!(
                self.import.screen,
                ImportScreen::Configure
                    | ImportScreen::Preflight
                    | ImportScreen::Ready
                    | ImportScreen::Failure
            )
        {
            theme::inline_message(
                ui,
                theme::Intent::Error,
                "This recovered job does not include its attestations, source descriptors, category identities/schema, or accepted mapping request in the current API contract. Unsafe continuation is disabled.",
            );
            if theme::primary_button(
                ui,
                !self.import.busy,
                egui::Button::new("Restart import setup"),
            )
            .clicked()
            {
                self.restart_import_setup();
            }
            return;
        }
        match self.import.screen {
            ImportScreen::Source => self.import_source_step(ui, capabilities),
            ImportScreen::Configure => self.import_transport_step(ui),
            ImportScreen::Preflight | ImportScreen::Ready => self.import_preflight_step(ui),
            ImportScreen::Running => self.import_running_step(ui),
            ImportScreen::Failure => self.import_failure_step(ui),
            ImportScreen::Success => self.import_success_step(ui),
        }
        ui.separator();
        ui.collapsing("Recover an import", |ui| {
            theme::labeled_text_field(
                ui,
                "Import ID",
                &mut self.import.recovery_import_id,
                theme::COMPACT_TEXT_FIELD_HEIGHT,
            );
            if ui
                .add_enabled(
                    !self.import.busy
                        && !self.import.recovery_import_id.trim().is_empty(),
                    egui::Button::new("Resume import"),
                )
                .clicked()
            {
                self.request_import_recovery();
            }
        });
    }

    fn import_progress_overview(&self, ui: &mut egui::Ui) {
        let activity = self.current_import_activity();
        let show_activity_label = ui.available_width() >= 520.0;
        ui.horizontal(|ui| {
            ui.heading("Import progress");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(activity) = activity {
                    let status = format!("{} | {}", activity.label(), activity.operation());
                    let response = ui.spinner().on_hover_text(&status);
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Other, true, status.clone())
                    });
                    if show_activity_label {
                        ui.label(
                            RichText::new(activity.label())
                                .small()
                                .color(theme::TEXT_MUTED),
                        );
                    }
                }
            });
        });

        let active_stage = current_import_stage(&self.import);
        let active_progress = self.active_stage_progress(active_stage, activity);
        let pill_width = 98.0;
        let columns = (((ui.available_width() + theme::SPACE_2) / (pill_width + theme::SPACE_2))
            .floor() as usize)
            .clamp(1, ImportStage::ALL.len());
        for row in ImportStage::ALL.chunks(columns) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = theme::SPACE_2;
                for &stage in row {
                    let status = import_stage_status(&self.import, stage);
                    let fraction = match status {
                        ImportStageStatus::Complete | ImportStageStatus::Failed => Some(1.0),
                        ImportStageStatus::Pending => Some(0.0),
                        ImportStageStatus::Active => active_progress.fraction,
                    };
                    import_stage_pill(ui, stage, status, fraction);
                }
            });
        }
        ui.add_space(theme::SPACE_2);
        ui.label(
            RichText::new(import_step_label(self.import.screen)).color(theme::TEXT_MUTED),
        );
        let progress_color = if self.import.screen == ImportScreen::Failure {
            theme::DANGER
        } else if self.import.screen == ImportScreen::Success {
            theme::SUCCESS
        } else {
            theme::ACCENT
        };
        ui.label(RichText::new(&active_progress.label).strong());
        if let Some(fraction) = active_progress.fraction {
            let progress = egui::ProgressBar::new(fraction)
                .desired_height(18.0)
                .fill(progress_color);
            let show_value = self.import.screen != ImportScreen::Failure;
            let response = ui.add(if show_value {
                progress.show_percentage()
            } else {
                progress.text("Blocked")
            });
            response.widget_info(|| {
                let mut info = egui::WidgetInfo::labeled(
                    egui::WidgetType::ProgressIndicator,
                    true,
                    active_progress.label.clone(),
                );
                if show_value {
                    info.value = Some((fraction.clamp(0.0, 1.0) * 100.0).floor() as f64);
                }
                info
            });
        } else {
            indeterminate_import_progress(ui, &active_progress.label, progress_color);
        }
    }

    fn current_import_activity(&self) -> Option<ImportActivity> {
        self.import
            .active_operations
            .values()
            .copied()
            .max_by_key(|activity| activity.priority())
            .or_else(|| {
                self.import
                    .capabilities_loading
                    .then_some(ImportActivity::CheckCapabilities)
            })
            .or_else(|| {
                self.import
                    .source_picker
                    .loading
                    .then_some(ImportActivity::BrowseSource)
            })
            .or_else(|| {
                self.import
                    .yolo_inspection_loading
                    .then_some(ImportActivity::InspectDescriptor)
            })
            .or_else(|| {
                (self.import.screen == ImportScreen::Success && self.loading.datasets)
                    .then_some(ImportActivity::RefreshDatasets)
            })
            .or_else(|| {
                self.import
                    .busy
                    .then_some(match self.import.screen {
                        ImportScreen::Source => ImportActivity::Create,
                        ImportScreen::Configure => ImportActivity::Seal,
                        ImportScreen::Preflight => ImportActivity::Preflight,
                        ImportScreen::Ready => ImportActivity::UpdatePlan,
                        ImportScreen::Running => ImportActivity::Commit,
                        ImportScreen::Failure => ImportActivity::LoadStatus,
                        ImportScreen::Success => ImportActivity::RefreshDatasets,
                    })
            })
    }

    fn active_stage_progress(
        &self,
        stage: ImportStage,
        activity: Option<ImportActivity>,
    ) -> ActiveStageProgress {
        if self.import.screen == ImportScreen::Failure {
            return ActiveStageProgress {
                label: format!("{} needs attention", stage.label()),
                fraction: Some(1.0),
            };
        }
        if self.import.screen == ImportScreen::Success {
            return ActiveStageProgress {
                label: "Import complete".to_string(),
                fraction: Some(1.0),
            };
        }

        match stage {
            ImportStage::Source => {
                if matches!(activity, Some(ImportActivity::Create)) {
                    return ActiveStageProgress {
                        label: activity.unwrap().label().to_string(),
                        fraction: None,
                    };
                }
                let dataset_id = DatasetId::from(self.import.destination_id.trim());
                let source_selected = self.import.transport == ImportTransport::BrowserFolder
                    || (!self.import.server_root_id.is_empty()
                        && !self.import.server_relative_path.trim().is_empty());
                let complete = [
                    dataset_id.validate_path_segment().is_ok(),
                    !self.import.destination_name.trim().is_empty(),
                    source_selected,
                    self.import.ground_truth,
                    !self.import.provenance.trim().is_empty(),
                ]
                .into_iter()
                .filter(|ready| *ready)
                .count();
                ActiveStageProgress {
                    label: format!("Source setup: {complete} of 5 requirements complete"),
                    fraction: Some(complete as f32 / 5.0),
                }
            }
            ImportStage::Configure => {
                if let Some(job) = &self.import.job
                    && job.transport == ImportTransport::BrowserFolder
                    && job.progress.total_bytes > 0
                    && job.progress.accepted_bytes < job.progress.total_bytes
                {
                    return ActiveStageProgress {
                        label: format!(
                            "Uploading source: {} of {} files, {} of {}",
                            job.progress.uploaded_files,
                            job.progress.total_files,
                            import_human_bytes(job.progress.accepted_bytes),
                            import_human_bytes(job.progress.total_bytes),
                        ),
                        fraction: Some(
                            job.progress.accepted_bytes as f32 / job.progress.total_bytes as f32,
                        ),
                    };
                }
                if let Some(activity) = activity {
                    return ActiveStageProgress {
                        label: activity.label().to_string(),
                        fraction: None,
                    };
                }
                let upload_ready = self.import.transport == ImportTransport::ServerDirectory
                    || self.import.job.as_ref().is_some_and(|job| {
                        job.progress.total_files > 0
                            && job.progress.uploaded_files == job.progress.total_files
                            && job.progress.accepted_bytes == job.progress.total_bytes
                    });
                let complete = [
                    upload_ready,
                    !self.import.source_namespace.trim().is_empty(),
                    self.import_descriptor_error().is_none(),
                ]
                .into_iter()
                .filter(|ready| *ready)
                .count();
                ActiveStageProgress {
                    label: format!("Source configuration: {complete} of 3 requirements complete"),
                    fraction: Some(complete as f32 / 3.0),
                }
            }
            ImportStage::Preflight => {
                if let Some(activity) = activity {
                    return ActiveStageProgress {
                        label: activity.label().to_string(),
                        fraction: None,
                    };
                }
                let report = self
                    .import
                    .plan
                    .as_ref()
                    .map(|plan| &plan.report)
                    .or_else(|| {
                        self.import
                            .job
                            .as_ref()
                            .and_then(|job| job.preflight_report.as_ref())
                    });
                let acknowledgements_complete = report.is_some_and(|report| {
                    report.diagnostics.iter().all(|diagnostic| {
                        !diagnostic.impact.requires_acknowledgement
                            || self.import.acknowledgements.contains(&diagnostic.code)
                    })
                });
                let complete = [
                    report.is_some(),
                    self.import_mappings_complete(),
                    acknowledgements_complete,
                ]
                .into_iter()
                .filter(|ready| *ready)
                .count();
                ActiveStageProgress {
                    label: format!("Preflight review: {complete} of 3 requirements complete"),
                    fraction: Some(complete as f32 / 3.0),
                }
            }
            ImportStage::Ready => ActiveStageProgress {
                label: "Preflight accepted; ready to import".to_string(),
                fraction: Some(1.0),
            },
            ImportStage::Import => {
                let counters = self.import.job.as_ref().and_then(|job| {
                    let total = job
                        .progress
                        .total_images
                        .saturating_add(job.progress.total_objects);
                    let complete = job
                        .progress
                        .processed_images
                        .saturating_add(job.progress.processed_objects);
                    (total > 0 && complete < total).then_some((complete, total))
                });
                match counters {
                    Some((complete, total)) => ActiveStageProgress {
                        label: format!("Building dataset: {complete} of {total} records processed"),
                        fraction: Some(complete as f32 / total as f32),
                    },
                    None => ActiveStageProgress {
                        label: activity.map_or_else(
                            || {
                                self.import.job.as_ref().map_or_else(
                                    || "Building and publishing dataset".to_string(),
                                    |job| lifecycle_label(job.lifecycle).to_string(),
                                )
                            },
                            |activity| activity.label().to_string(),
                        ),
                        fraction: None,
                    },
                }
            }
        }
    }

}
