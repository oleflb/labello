impl LabelloApp {
    pub(crate) fn admin_view(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
        let config_dirty = self.datasets.admin_config != self.datasets.admin_baseline;
        let permissions_dirty = self.datasets.users != self.datasets.users_baseline;
        let changes_dirty = config_dirty || permissions_dirty;
        let admin_busy = self.loading.admin || self.loading.roles_user.is_some();
        let load_error = self.admin_tools.load_error.clone();
        let issues = self
            .staged_admin_config()
            .as_ref()
            .map(|config| config_issues(config, &self.config.user_id))
            .unwrap_or_default();
        let mut status_text = if admin_busy {
            if self.loading.roles_user.is_some() {
                "Saving Admin changes".to_string()
            } else if self.datasets.admin_config.is_some() {
                "Saving or refreshing Admin changes".to_string()
            } else {
                "Loading Admin configuration".to_string()
            }
        } else if load_error.is_some() {
            if self.datasets.admin_config.is_some() {
                "Admin refresh failed".to_string()
            } else {
                "Admin configuration unavailable".to_string()
            }
        } else if config_dirty && permissions_dirty {
            "Admin changes staged".to_string()
        } else if config_dirty {
            "Configuration changes staged".to_string()
        } else if permissions_dirty {
            "Permission changes staged".to_string()
        } else {
            "Admin changes saved".to_string()
        };
        if changes_dirty && !issues.is_empty() {
            status_text.push_str(&format!("; {} validation error(s)", issues.len()));
        }
        let status_color = if load_error.is_some() {
            theme::DANGER
        } else if admin_busy {
            theme::INFO
        } else if changes_dirty {
            theme::WARNING
        } else {
            theme::SUCCESS
        };
        let can_reload = !self.loading.admin
            && self.loading.roles_user.is_none()
            && !self.loading.uploading
            && !self.loading.ingesting
            && !changes_dirty;
        let idle = !self.loading.admin
            && self.loading.roles_user.is_none()
            && !self.loading.uploading
            && !self.loading.ingesting;
        let can_save = changes_dirty && issues.is_empty() && idle;
        let mut reload = false;
        let mut save = false;
        let mut discard = false;
        theme::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let title = |ui: &mut egui::Ui| {
                ui.vertical(|ui| {
                    ui.heading("Dataset Admin");
                    ui.label(
                        RichText::new(
                            "Manage access, inspect images, and configure labeling workflows.",
                        )
                        .color(theme::TEXT_MUTED),
                    );
                });
            };
            let mut actions = |ui: &mut egui::Ui| {
                admin_status_indicator(ui, &status_text, status_color, admin_busy);
                if changes_dirty {
                    let save_response = theme::primary_button(
                        ui,
                        can_save,
                        egui::Button::new(RichText::new("✓").size(18.0))
                            .min_size(egui::vec2(44.0, 44.0)),
                    )
                    .on_hover_text(if can_save {
                        "Save all staged configuration and permission changes."
                    } else if !issues.is_empty() {
                        "Fix validation errors before saving."
                    } else {
                        "Wait for the active Admin operation to finish."
                    });
                    save_response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            can_save,
                            "Save Admin changes",
                        )
                    });
                    save = save_response.clicked();

                    let discard_response = theme::danger_button(
                        ui,
                        idle,
                        egui::Button::new(RichText::new("×").size(20.0))
                            .min_size(egui::vec2(44.0, 44.0)),
                    )
                    .on_hover_text("Discard all staged configuration and permission changes.");
                    discard_response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            idle,
                            "Discard staged changes",
                        )
                    });
                    discard = discard_response.clicked();
                } else if self.datasets.admin_config.is_some()
                    && theme::quiet_button(
                        ui,
                        can_reload,
                        egui::Button::new("Reload"),
                    )
                    .on_hover_text(if !can_reload {
                        "Finish the active operation and save or discard staged changes before reloading."
                    } else {
                        "Reload configuration from the server."
                    })
                    .clicked()
                {
                    reload = true;
                }
            };
            if layout != LayoutMode::Wide {
                ui.vertical(|ui| {
                    title(ui);
                    ui.horizontal_wrapped(actions);
                });
            } else {
                ui.horizontal(|ui| {
                    title(ui);
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| actions(ui),
                    );
                });
            }
        });
        if reload {
            self.request_admin_dataset();
        }
        if save {
            self.request_admin_changes_save();
        }
        if discard {
            self.admin_tools.confirm_discard = true;
        }
        ui.add_space(theme::SPACE_2);
        if let Some(error) = load_error {
            ui.horizontal_wrapped(|ui| {
                theme::inline_message(
                    ui,
                    if self.datasets.admin_config.is_some() {
                        theme::Intent::Warning
                    } else {
                        theme::Intent::Error
                    },
                    if self.datasets.admin_config.is_some() {
                        format!("Showing saved admin data. Refresh failed: {error}")
                    } else {
                        format!("Admin load failed: {error}")
                    },
                );
                if theme::quiet_button(ui, can_reload, egui::Button::new("Retry admin load"))
                    .clicked()
                {
                    self.request_admin_dataset();
                }
            });
        }
        if self.datasets.admin_config.is_none() {
            if self.loading.admin {
                theme::card_frame().show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.spinner();
                    ui.label("Loading admin configuration...");
                });
            } else if self.admin_tools.load_error.is_none()
                && theme::empty_state(
                    ui,
                    "Admin config is not loaded",
                    "Load the dataset configuration before editing it.",
                    Some(egui::Button::new("Load admin config")),
                )
            {
                self.request_admin_dataset();
            }
            return;
        }
        if let Some(config) = self.datasets.admin_config.as_mut() {
            for task in &mut config.tasks {
                normalize_task_annotation(task);
            }
        }

        if layout == LayoutMode::Wide {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                ui.vertical(|ui| {
                    ui.set_width(160.0);
                    self.admin_navigation(ui, layout);
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    self.admin_section(ui, layout);
                });
            });
        } else {
            self.admin_navigation(ui, layout);
            ui.add_space(theme::SPACE_4);
            self.admin_section(ui, layout);
        }
    }

    fn admin_navigation(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
        let response = ui.vertical(|ui| match layout {
            LayoutMode::Compact | LayoutMode::Medium => {
                let label = ui.label("Admin section");
                egui::ComboBox::from_id_salt("admin-section")
                    .width(ui.available_width())
                    .selected_text(self.admin_tools.section.label())
                    .show_ui(ui, |ui| {
                        for section in AdminSection::ALL {
                            ui.selectable_value(
                                &mut self.admin_tools.section,
                                section,
                                section.label(),
                            );
                        }
                    })
                    .response
                    .labelled_by(label.id);
            }
            LayoutMode::Wide => {
                ui.label(
                    RichText::new("Admin sections")
                        .strong()
                        .color(theme::TEXT_MUTED),
                );
                ui.add_space(theme::SPACE_1);
                for section in AdminSection::ALL {
                    if ui
                        .add_sized(
                            [ui.available_width(), 44.0],
                            egui::Button::selectable(
                                self.admin_tools.section == section,
                                section.label(),
                            ),
                        )
                        .clicked()
                    {
                        self.admin_tools.section = section;
                    }
                }
            }
        });
        response.response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Admin navigation")
        });
    }

    fn admin_section(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
        match self.admin_tools.section {
            AdminSection::Overview => self.admin_overview(ui),
            AdminSection::People => self.people_section(ui, layout),
            AdminSection::Images => self.admin_images(ui, layout),
            AdminSection::Schema => self.admin_schema(ui),
            AdminSection::Automation => self.admin_automation(ui),
            AdminSection::Backups => self.snapshots_section(ui, layout),
        }
    }
}

fn admin_status_indicator(ui: &mut egui::Ui, status: &str, color: egui::Color32, loading: bool) {
    let response = if loading {
        ui.spinner()
    } else {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(18.0, 44.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 5.0, color);
        response
    }
    .on_hover_text(status);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Other, true, status.to_string())
    });
}
