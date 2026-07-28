impl LabelloApp {
    fn admin_overview(&mut self, ui: &mut egui::Ui) {
        ui.heading("Overview");
        ui.label(
            RichText::new("Review dataset health before opening a focused admin section.")
                .color(theme::TEXT_MUTED),
        );
        let image_count = self
            .datasets
            .summaries
            .iter()
            .find(|dataset| dataset.dataset_id == self.config.dataset_id)
            .map(|dataset| dataset.total_images)
            .unwrap_or_default();
        let workflow_count = self
            .datasets
            .admin_config
            .as_ref()
            .map(|config| config.tasks.iter().filter(|task| task.enabled).count())
            .unwrap_or_default();
        let metrics = [
            ("Users", self.datasets.users.len()),
            ("Indexed images", image_count),
            ("Active workflows", workflow_count),
        ];
        let columns = (((ui.available_width() + theme::SPACE_2) / (200.0 + theme::SPACE_2)).floor()
            as usize)
            .clamp(1, 3);
        for row in metrics.chunks(columns) {
            ui.columns(columns, |columns| {
                for (column, (label, value)) in columns.iter_mut().zip(row) {
                    theme::metric(column, label, value.to_string());
                }
            });
        }
        let (action_label, action_section) = if workflow_count == 0 {
            ("Configure workflows", AdminSection::Schema)
        } else if image_count == 0 {
            ("Add images", AdminSection::Images)
        } else {
            ("Explore images", AdminSection::Images)
        };
        if theme::primary_button(
            ui,
            !self.loading.admin
                && self.loading.roles_user.is_none()
                && !self.loading.uploading
                && !self.loading.ingesting,
            egui::Button::new(action_label),
        )
        .clicked()
        {
            self.admin_tools.section = action_section;
        }

        if self.loading.uploading {
            theme::inline_message(ui, theme::Intent::Info, "Folder upload is in progress.");
        } else if self.loading.ingesting {
            theme::inline_message(ui, theme::Intent::Info, "Dataset ingest is in progress.");
        } else if let Some(notice) = self.runtime.notice.as_deref().filter(|notice| {
            notice.starts_with("Ingest ")
                || notice.starts_with("Uploading ")
                || notice.starts_with("Uploaded ")
        }) {
            theme::inline_message(ui, theme::Intent::Success, notice);
        }
        if let Some(error) = &self.admin_tools.upload_error {
            theme::inline_message(
                ui,
                theme::Intent::Error,
                format!("Folder upload failed: {error}"),
            );
        }

        if let Some(config) = self.datasets.admin_config.as_mut() {
            ui.add_enabled_ui(
                !self.loading.admin
                    && self.loading.roles_user.is_none()
                    && !self.loading.uploading
                    && !self.loading.ingesting,
                |ui| {
                    theme::card_frame().show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.vertical(|ui| {
                            ui.set_max_width(640.0_f32.min(ui.available_width()));
                            ui.heading("Dataset details");
                            theme::labeled_text_field(
                                ui,
                                "Dataset name",
                                &mut config.name,
                                theme::COMPACT_TEXT_FIELD_HEIGHT,
                            )
                            .on_hover_text("Human-readable name stored in labello.dataset.toml.");
                            show_issues(ui, &dataset_name_issues(&config.name));
                        });
                    });
                },
            );
        }

        let issues = self
            .staged_admin_config()
            .as_ref()
            .map(|config| config_issues(config, &self.config.user_id))
            .unwrap_or_default();
        theme::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.heading("Validation summary");
            if issues.is_empty() {
                theme::inline_message(
                    ui,
                    theme::Intent::Success,
                    "Configuration is ready to save.",
                );
            } else {
                theme::inline_message(
                    ui,
                    theme::Intent::Error,
                    format!("{} configuration error(s) need attention.", issues.len()),
                );
                show_issues(ui, &issues);
            }
        });
    }
}
