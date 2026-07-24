use std::collections::{BTreeMap, BTreeSet};

use eframe::egui::{self, RichText};
use labello_client::DatasetUser;
use labello_domain::{
    AgreementMetric, AgreementThreshold, AnnotationType, BrowserAcceleration, ClassId,
    DatasetMetadata, DatasetRole, DatasetRoleAssignment, DatasetSnapshot, ImageExplorerItem,
    ImbalanceConfig, KeypointSpec, LabelClass, ModelSpec, OutputProcessing, PrelabelConfig,
    PrelabelConfigId, PrelabelExecution, ReviewConfig, ReviewWorkflow, SkeletonEdge, SkeletonSpec,
    TaskDefinition, TaskId, TaskStatus, TutorialContent, UserId,
};

use crate::{
    app::{AdminSection, LabelloApp, LayoutMode},
    theme,
};

impl AdminSection {
    const ALL: [Self; 6] = [
        Self::Overview,
        Self::People,
        Self::Images,
        Self::Schema,
        Self::Automation,
        Self::Backups,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::People => "People",
            Self::Images => "Images",
            Self::Schema => "Schema",
            Self::Automation => "Automation",
            Self::Backups => "Backups",
        }
    }
}

impl LabelloApp {
    pub(crate) fn admin_view(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
        let config_dirty = self.datasets.admin_config != self.datasets.admin_baseline;
        let permissions_dirty = self.datasets.users != self.datasets.users_baseline;
        let load_error = self.admin_tools.load_error.clone();
        let status_text = if self.loading.admin {
            if self.datasets.admin_config.is_some() {
                "Refreshing admin config"
            } else {
                "Loading admin config"
            }
        } else if load_error.is_some() {
            if self.datasets.admin_config.is_some() {
                "Admin refresh failed"
            } else {
                "Admin config unavailable"
            }
        } else if config_dirty {
            "Configuration changes staged"
        } else if permissions_dirty {
            "Permission changes staged"
        } else {
            "Admin config saved"
        };
        let status_color =
            if self.loading.admin || load_error.is_some() || config_dirty || permissions_dirty {
                theme::WARNING
            } else {
                theme::SUCCESS
            };
        let can_reload = !self.loading.admin
            && self.loading.roles_user.is_none()
            && !self.loading.uploading
            && !self.loading.ingesting
            && !config_dirty
            && !permissions_dirty;
        let mut reload = false;
        theme::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let mut header = |ui: &mut egui::Ui| {
                ui.vertical(|ui| {
                    ui.heading("Dataset Admin");
                    ui.label(
                        RichText::new(
                            "Manage access, inspect images, and configure labeling workflows.",
                        )
                        .color(theme::TEXT_MUTED),
                    );
                });
                if self.loading.admin {
                    ui.spinner();
                }
                ui.label(RichText::new(status_text).color(status_color).strong());
                if self.datasets.admin_config.is_some()
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
            if layout == LayoutMode::Compact {
                ui.vertical(header);
            } else {
                ui.horizontal_wrapped(&mut header);
            }
        });
        if reload {
            self.request_admin_dataset();
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
            LayoutMode::Compact => {
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
            LayoutMode::Medium => {
                ui.horizontal_wrapped(|ui| {
                    for section in AdminSection::ALL {
                        if ui
                            .add_sized(
                                [110.0, 44.0],
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
                });
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
            !self.loading.admin && !self.loading.uploading && !self.loading.ingesting,
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
                !self.loading.admin && !self.loading.uploading && !self.loading.ingesting,
                |ui| {
                    theme::card_frame().show(ui, |ui| {
                        ui.set_max_width(640.0_f32.min(ui.available_width()));
                        ui.heading("Dataset details");
                        theme::labeled_text_field(ui, "Dataset name", &mut config.name, 44.0)
                            .on_hover_text("Human-readable name stored in labello.dataset.toml.");
                        show_issues(ui, &dataset_name_issues(&config.name));
                    });
                },
            );
        }

        let issues = self
            .datasets
            .admin_config
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

    fn admin_images(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
        ui.heading("Images");
        ui.label(
            RichText::new("Manage image roots, ingestion, uploads, and indexed image state.")
                .color(theme::TEXT_MUTED),
        );
        let config_dirty = self.datasets.admin_config != self.datasets.admin_baseline;
        let permissions_dirty = self.datasets.users != self.datasets.users_baseline;
        let busy = self.loading.admin
            || self.loading.roles_user.is_some()
            || self.loading.images
            || self.loading.uploading
            || self.loading.ingesting;
        let mut ingest = false;
        let mut upload_folder = false;
        if let Some(config) = self.datasets.admin_config.as_mut() {
            ui.add_enabled_ui(!busy, |ui| {
                theme::card_frame().show(ui, |ui| {
                    ui.set_max_width(640.0_f32.min(ui.available_width()));
                    ui.heading("Image roots and ingestion");
                    edit_string_list(
                        ui,
                        &mut config.image_roots,
                        "Root path",
                        "Add image root",
                        "images",
                    );
                    ui.horizontal_wrapped(|ui| {
                        if theme::primary_button(
                            ui,
                            !config_dirty && !permissions_dirty,
                            egui::Button::new("Pick folder and upload"),
                        )
                        .on_hover_text("Open a browser folder picker, upload files to a new dataset-relative root, then ingest them.")
                        .clicked()
                        {
                            upload_folder = true;
                        }
                        if theme::quiet_button(
                            ui,
                            !config_dirty && !permissions_dirty,
                            egui::Button::new("Run ingest"),
                        )
                        .on_hover_text("Scan configured image roots and update the dataset image index.")
                        .clicked()
                        {
                            ingest = true;
                        }
                    });
                    if let Some(progress) = self.loading.upload_progress.as_ref() {
                        ui.add(
                            egui::ProgressBar::new(progress.fraction())
                                .desired_width(ui.available_width().min(460.0))
                                .text(progress.label()),
                        );
                        if progress.current_batch > 0 {
                            ui.small(format!("Batch {}", progress.current_batch));
                        }
                    }
                    if let Some(error) = &self.admin_tools.upload_error {
                        theme::inline_message(
                            ui,
                            theme::Intent::Error,
                            format!("Folder upload failed: {error}"),
                        );
                    }
                    if config_dirty {
                        ui.label(
                            RichText::new("Save or discard root changes before uploading or ingesting.")
                                .color(theme::WARNING),
                        );
                    }
                    ui.small("Paths are relative to the dataset root and may be edited in labello.dataset.toml.");
                    show_issues(ui, &image_root_issues(&config.image_roots));
                });
            });
        }
        if ingest {
            self.request_ingest();
        }
        if upload_folder {
            self.request_folder_upload();
        }
        self.images_section(ui, layout);
    }

    fn admin_schema(&mut self, ui: &mut egui::Ui) {
        ui.heading("Schema");
        ui.label(
            RichText::new("Configure label classes, skeletons, and labeling workflows.")
                .color(theme::TEXT_MUTED),
        );
        let enabled = !self.loading.admin
            && self.loading.roles_user.is_none()
            && !self.loading.uploading
            && !self.loading.ingesting;
        ui.scope(|ui| {
            ui.set_max_width(ui.available_width().min(640.0));
            if let Some(config) = self.datasets.admin_config.as_mut() {
                ui.add_enabled_ui(enabled, |ui| {
                    edit_quick_workflows(ui, config);
                    edit_labels(ui, &mut config.label_classes, &mut config.tasks);
                    edit_tasks(
                        ui,
                        &mut config.tasks,
                        &config.label_classes,
                        &config.prelabel_configs,
                    );
                });
            }
        });
    }

    fn admin_automation(&mut self, ui: &mut egui::Ui) {
        ui.heading("Automation");
        ui.label(
            RichText::new("Configure prelabels and assignment balancing.").color(theme::TEXT_MUTED),
        );
        let enabled = !self.loading.admin
            && self.loading.roles_user.is_none()
            && !self.loading.uploading
            && !self.loading.ingesting;
        ui.scope(|ui| {
            ui.set_max_width(ui.available_width().min(640.0));
            if let Some(config) = self.datasets.admin_config.as_mut() {
                ui.add_enabled_ui(enabled, |ui| {
                    edit_prelabels(ui, &mut config.prelabel_configs, &mut config.tasks);
                    edit_imbalance(ui, &mut config.imbalance);
                });
            }
        });
    }

    pub(crate) fn admin_status_bar(&mut self, ui: &mut egui::Ui) {
        let config_dirty = self.datasets.admin_config != self.datasets.admin_baseline;
        let permissions_dirty = self.datasets.users != self.datasets.users_baseline;
        let issues = self
            .datasets
            .admin_config
            .as_ref()
            .map(|config| config_issues(config, &self.config.user_id))
            .unwrap_or_default();
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(if config_dirty {
                    "Unsaved admin changes"
                } else {
                    "Unsaved permission changes"
                })
                .color(theme::AMBER)
                .strong(),
            );
            if config_dirty {
                if theme::primary_button(
                    ui,
                    issues.is_empty() && !self.loading.admin,
                    egui::Button::new("Save Admin Config"),
                )
                .on_disabled_hover_text("Fix validation errors before saving.")
                .clicked()
                {
                    self.request_admin_save();
                }
                if theme::danger_button(
                    ui,
                    !self.loading.admin,
                    egui::Button::new("Discard staged changes"),
                )
                .clicked()
                {
                    self.datasets.admin_config = self.datasets.admin_baseline.clone();
                    self.clear_admin_draft();
                    self.runtime.notice = Some("Staged admin changes discarded".to_string());
                }
            }
            if !issues.is_empty() {
                ui.label(
                    RichText::new(format!("{} validation error(s)", issues.len()))
                        .color(theme::DANGER),
                );
            }
            if permissions_dirty {
                ui.label("Save changed permissions in the People section.");
            }
        });
    }

    pub(crate) fn admin_status_height(&self, layout: LayoutMode) -> f32 {
        if layout == LayoutMode::Wide {
            return 68.0;
        }
        let config_dirty = self.datasets.admin_config != self.datasets.admin_baseline;
        let permissions_dirty = self.datasets.users != self.datasets.users_baseline;
        let has_issues = self
            .datasets
            .admin_config
            .as_ref()
            .is_some_and(|config| !config_issues(config, &self.config.user_id).is_empty());
        (if layout == LayoutMode::Compact && config_dirty {
            164.0
        } else {
            68.0
        }) + if has_issues { 24.0 } else { 0.0 }
            + if permissions_dirty { 44.0 } else { 0.0 }
    }

    fn people_section(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
        ui.heading("People");
        theme::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!("{} users", self.datasets.users.len()))
                        .color(theme::INFO),
                );
            });
            ui.label(
                RichText::new("Grant dataset roles to people who have signed in to this server.")
                    .color(theme::TEXT_MUTED),
            );
            let search_label = ui.label("Search people");
            ui.add_sized(
                [ui.available_width().min(480.0), 44.0],
                egui::TextEdit::singleline(&mut self.admin_tools.people_search)
                    .hint_text("Name, login, or user ID"),
            )
            .labelled_by(search_label.id);
            if self.loading.admin && self.datasets.users.is_empty() {
                ui.spinner();
                return;
            }
            let search = self.admin_tools.people_search.trim().to_lowercase();
            let current_user = self.config.user_id.clone();
            let admin_loading =
                self.loading.admin || self.loading.uploading || self.loading.ingesting;
            let baseline = self.datasets.users_baseline.clone();
            let admin_count = self
                .datasets
                .users
                .iter()
                .filter(|user| user.roles.contains(&DatasetRole::DataAdmin))
                .count();
            let saving = self.loading.roles_user.clone();
            let mut save_user = None;
            let mut visible_users = 0;
            if layout == LayoutMode::Wide {
                egui::Grid::new("admin-people-grid")
                    .num_columns(4)
                    .striped(true)
                    .spacing([theme::SPACE_4, theme::SPACE_2])
                    .show(ui, |ui| {
                        for heading in ["Person", "Roles", "Status", "Action"] {
                            ui.label(RichText::new(heading).strong().color(theme::TEXT_MUTED));
                        }
                        ui.end_row();
                        for user in self
                            .datasets
                            .users
                            .iter_mut()
                            .filter(|user| user_matches_search(user, &search))
                        {
                            visible_users += 1;
                            ui.vertical(|ui| {
                                ui.set_width(180.0);
                                user_identity(ui, user);
                            });
                            ui.horizontal_wrapped(|ui| {
                                ui.set_min_width(300.0);
                                edit_user_roles(
                                    ui,
                                    user,
                                    &current_user,
                                    admin_count,
                                    admin_loading,
                                    saving.as_ref(),
                                );
                            });
                            let dirty = user_permissions_dirty(user, &baseline);
                            let this_saving = saving.as_ref() == Some(&user.account.user_id);
                            ui.label(
                                RichText::new(if this_saving {
                                    "Saving"
                                } else if dirty {
                                    "Staged"
                                } else {
                                    "Saved"
                                })
                                .color(if dirty || this_saving {
                                    theme::WARNING
                                } else {
                                    theme::SUCCESS
                                }),
                            );
                            if save_permissions_button(
                                ui,
                                user,
                                dirty,
                                admin_loading,
                                saving.as_ref(),
                            ) {
                                save_user = Some(user.account.user_id.clone());
                            }
                            ui.end_row();
                        }
                    });
            } else {
                for user in self
                    .datasets
                    .users
                    .iter_mut()
                    .filter(|user| user_matches_search(user, &search))
                {
                    visible_users += 1;
                    ui.add_space(theme::SPACE_1);
                    theme::inset_frame().show(ui, |ui| {
                        user_identity(ui, user);
                        ui.add_space(theme::SPACE_1);
                        ui.horizontal_wrapped(|ui| {
                            edit_user_roles(
                                ui,
                                user,
                                &current_user,
                                admin_count,
                                admin_loading,
                                saving.as_ref(),
                            );
                        });
                        let dirty = user_permissions_dirty(user, &baseline);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(if saving.as_ref() == Some(&user.account.user_id) {
                                    "Saving"
                                } else if dirty {
                                    "Changes staged"
                                } else {
                                    "Permissions saved"
                                })
                                .color(if dirty {
                                    theme::WARNING
                                } else {
                                    theme::TEXT_MUTED
                                }),
                            );
                            if save_permissions_button(
                                ui,
                                user,
                                dirty,
                                admin_loading,
                                saving.as_ref(),
                            ) {
                                save_user = Some(user.account.user_id.clone());
                            }
                        });
                    });
                }
            }
            if visible_users == 0 && !self.datasets.users.is_empty() {
                theme::empty_state(
                    ui,
                    "No matching people",
                    "Change the search to show more accounts.",
                    None,
                );
            }
            if self.datasets.users.is_empty() && !self.loading.admin {
                theme::empty_state(
                    ui,
                    "No people yet",
                    "People appear here after they sign in to this server.",
                    None,
                );
            }
            if let Some(user_id) = save_user {
                self.request_role_save(user_id);
            }
        });
    }

    fn images_section(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
        let controls_enabled = !self.loading.images
            && !self.loading.admin
            && !self.loading.uploading
            && !self.loading.ingesting;
        theme::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.heading("Image explorer");
            ui.label(
                RichText::new("Search the indexed images and inspect workflow state.")
                    .color(theme::TEXT_MUTED),
            );
            let compact_filters = ui.available_width() < 600.0;
            let search_label = ui.label("Search images");
            let search = ui
                .add_sized(
                    [ui.available_width(), 44.0],
                    egui::TextEdit::singleline(&mut self.admin_tools.image_search)
                        .hint_text("File name or path"),
                )
                .labelled_by(search_label.id);
            if search.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                self.admin_tools.image_query.page = 1;
                self.request_images();
            }
            let control_width = if compact_filters {
                ui.available_width()
            } else {
                140.0
            };
            let mut show_filters = |ui: &mut egui::Ui| {
                ui.vertical(|ui| {
                    let label = ui.label("Status filter");
                    egui::ComboBox::from_id_salt("image-explorer-status")
                        .width(control_width)
                        .selected_text(
                            self.admin_tools
                                .image_status
                                .as_ref()
                                .map(task_status_label)
                                .unwrap_or("Any status"),
                        )
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.admin_tools.image_status,
                                None,
                                "Any status",
                            );
                            for status in task_statuses() {
                                let label = task_status_label(&status);
                                ui.selectable_value(
                                    &mut self.admin_tools.image_status,
                                    Some(status),
                                    label,
                                );
                            }
                        })
                        .response
                        .labelled_by(label.id);
                });

                let tasks = self
                    .datasets
                    .admin_config
                    .as_ref()
                    .map(|config| config.tasks.clone())
                    .unwrap_or_default();
                ui.vertical(|ui| {
                    let label = ui.label("Workflow filter");
                    egui::ComboBox::from_id_salt("image-explorer-task")
                        .width(control_width)
                        .selected_text(
                            self.admin_tools
                                .image_task
                                .as_ref()
                                .and_then(|task_id| {
                                    tasks
                                        .iter()
                                        .find(|task| &task.task_id == task_id)
                                        .map(|task| task.name.as_str())
                                })
                                .unwrap_or("Any task"),
                        )
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.admin_tools.image_task, None, "Any task");
                            for task in &tasks {
                                ui.selectable_value(
                                    &mut self.admin_tools.image_task,
                                    Some(task.task_id.clone()),
                                    &task.name,
                                );
                            }
                        })
                        .response
                        .labelled_by(label.id);
                });

                let classes = self
                    .datasets
                    .admin_config
                    .as_ref()
                    .map(|config| config.label_classes.clone())
                    .unwrap_or_default();
                ui.vertical(|ui| {
                    let label = ui.label("Class filter");
                    egui::ComboBox::from_id_salt("image-explorer-class")
                        .width(control_width)
                        .selected_text(
                            self.admin_tools
                                .image_class
                                .as_ref()
                                .and_then(|class_id| {
                                    classes
                                        .iter()
                                        .find(|class| &class.class_id == class_id)
                                        .map(|class| class.name.as_str())
                                })
                                .unwrap_or("Any class"),
                        )
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.admin_tools.image_class,
                                None,
                                "Any class",
                            );
                            for class in &classes {
                                ui.selectable_value(
                                    &mut self.admin_tools.image_class,
                                    Some(class.class_id.clone()),
                                    &class.name,
                                );
                            }
                        })
                        .response
                        .labelled_by(label.id);
                });
                if theme::primary_button(ui, controls_enabled, egui::Button::new("Apply filters"))
                    .clicked()
                {
                    self.admin_tools.image_query.page = 1;
                    self.request_images();
                }
                if theme::quiet_button(
                    ui,
                    controls_enabled,
                    egui::Button::new(
                        if self.admin_tools.images_error.is_some()
                            && self.admin_tools.images.is_none()
                        {
                            "Retry image load"
                        } else {
                            "Refresh images"
                        },
                    ),
                )
                .clicked()
                {
                    self.request_images();
                }
                if self.loading.images {
                    ui.spinner();
                    ui.small(if self.admin_tools.images.is_some() {
                        "Refreshing images..."
                    } else {
                        "Loading images..."
                    });
                }
            };
            if compact_filters {
                ui.vertical(&mut show_filters);
            } else {
                ui.horizontal_wrapped(show_filters);
            }
            if let Some(error) = &self.admin_tools.images_error {
                theme::inline_message(
                    ui,
                    if self.admin_tools.images.is_some() {
                        theme::Intent::Warning
                    } else {
                        theme::Intent::Error
                    },
                    if self.admin_tools.images.is_some() {
                        format!("Showing saved image results. Refresh failed: {error}")
                    } else {
                        format!("Could not load images: {error}")
                    },
                );
            }
            if let Some(page) = self.admin_tools.images.clone() {
                let previous = page.page > 1;
                let next = page.page < page.total_pages;
                let current_page = page.page;
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} images | Page {} of {}",
                            page.total_items,
                            page.page,
                            page.total_pages.max(1)
                        ))
                        .strong(),
                    );
                    if ui
                        .add_enabled(
                            previous && controls_enabled,
                            egui::Button::new("Previous images"),
                        )
                        .clicked()
                    {
                        self.admin_tools.image_query.page = current_page.saturating_sub(1);
                        self.request_images();
                    }
                    if ui
                        .add_enabled(next && controls_enabled, egui::Button::new("Next images"))
                        .clicked()
                    {
                        self.admin_tools.image_query.page = current_page + 1;
                        self.request_images();
                    }
                });
                if page.items.is_empty() {
                    if !self.loading.images {
                        theme::empty_state(
                            ui,
                            "No matching images",
                            "Change the filters or ingest more images.",
                            None,
                        );
                    }
                } else if layout == LayoutMode::Wide {
                    admin_image_grid(ui, &page.items);
                } else {
                    for item in &page.items {
                        ui.add_space(theme::SPACE_1);
                        admin_image_card(ui, item);
                    }
                }
            } else if !self.loading.images && self.admin_tools.images_error.is_none() {
                theme::empty_state(
                    ui,
                    "No image results",
                    "Apply filters or refresh to load indexed images.",
                    None,
                );
            }
        });
    }

    fn snapshots_section(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
        ui.heading("Backups");
        ui.label(
            RichText::new(
                "Create and download native dataset snapshots. Image bytes are not included.",
            )
            .color(theme::TEXT_MUTED),
        );
        theme::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                if theme::primary_button(
                    ui,
                    !self.loading.creating_snapshot
                        && !self.loading.snapshots
                        && self.loading.snapshot_file.is_none(),
                    egui::Button::new(if self.loading.creating_snapshot {
                        "Creating snapshot..."
                    } else {
                        "Create snapshot"
                    }),
                )
                .clicked()
                {
                    self.request_snapshot_create();
                }
                if theme::quiet_button(
                    ui,
                    !self.loading.snapshots && !self.loading.creating_snapshot,
                    egui::Button::new(
                        if self.admin_tools.snapshots_error.is_some()
                            && !self.admin_tools.snapshots_loaded
                        {
                            "Retry backup load"
                        } else {
                            "Refresh backups"
                        },
                    ),
                )
                .clicked()
                {
                    self.request_snapshots();
                }
                if self.loading.snapshots {
                    ui.spinner();
                    ui.small(
                        if self.admin_tools.snapshots_loaded
                            || !self.admin_tools.snapshots.is_empty()
                        {
                            "Refreshing backups..."
                        } else {
                            "Loading backups..."
                        },
                    );
                }
            });

            if let Some(error) = &self.admin_tools.snapshots_error {
                theme::inline_message(
                    ui,
                    if self.admin_tools.snapshots_loaded || !self.admin_tools.snapshots.is_empty() {
                        theme::Intent::Warning
                    } else {
                        theme::Intent::Error
                    },
                    if self.admin_tools.snapshots_loaded {
                        format!("Showing the last loaded backups. Refresh failed: {error}")
                    } else if !self.admin_tools.snapshots.is_empty() {
                        format!("Showing newly created backups. Catalog refresh failed: {error}")
                    } else {
                        format!("Could not load backups: {error}")
                    },
                );
            }
            if let Some(error) = &self.admin_tools.snapshot_action_error {
                theme::inline_message(
                    ui,
                    theme::Intent::Error,
                    format!("Backup action failed: {error}"),
                );
            }

            if !self.admin_tools.snapshots_loaded
                && !self.loading.snapshots
                && self.admin_tools.snapshots_error.is_none()
            {
                theme::empty_state(
                    ui,
                    "Backups are not loaded",
                    "Refresh to load the available dataset snapshots.",
                    None,
                );
            } else if self.admin_tools.snapshots.is_empty()
                && self.admin_tools.snapshots_loaded
                && !self.loading.snapshots
                && self.admin_tools.snapshots_error.is_none()
            {
                theme::empty_state(
                    ui,
                    "No snapshots yet",
                    "Create a snapshot to preserve the current dataset state.",
                    None,
                );
            }

            let snapshots = self.admin_tools.snapshots.clone();
            let download = if snapshots.is_empty() {
                None
            } else if layout == LayoutMode::Wide {
                admin_snapshot_grid(ui, &snapshots, self.loading.snapshot_file.as_ref())
            } else {
                admin_snapshot_cards(ui, &snapshots, self.loading.snapshot_file.as_ref())
            };
            if let Some((snapshot_id, path)) = download {
                self.request_snapshot_download(snapshot_id, path);
            }
        });
    }

    pub(crate) fn stats_view(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
        let has_data = self.datasets.last_stats_completion.is_some();
        let initial_loading = self.loading.stats && !has_data;
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Live Statistics")
                    .size(theme::PAGE_TITLE_SIZE)
                    .strong(),
            );
            if has_data
                && theme::quiet_button(ui, !self.loading.stats, egui::Button::new("Refresh now"))
                    .on_hover_text(
                        "Refresh statistics immediately. They also refresh automatically.",
                    )
                    .clicked()
            {
                self.request_stats();
            }
        });
        if initial_loading {
            theme::card_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Loading statistics...").strong());
                });
                ui.label(
                    RichText::new("Fetching the first dataset summary.").color(theme::TEXT_MUTED),
                );
            });
            return;
        }
        if !has_data {
            let (title, explanation, action) = if let Some(error) = &self.datasets.stats_error {
                (
                    "Statistics unavailable",
                    format!("The first statistics request failed: {error}"),
                    "Retry statistics",
                )
            } else {
                (
                    "Statistics have not loaded",
                    "Load the current dataset summary and activity history.".to_string(),
                    "Load statistics",
                )
            };
            if theme::empty_state(ui, title, &explanation, Some(egui::Button::new(action))) {
                self.request_stats();
            }
            return;
        }
        ui.horizontal_wrapped(|ui| {
            if self.loading.stats {
                ui.label(RichText::new("Refreshing statistics").color(theme::TEXT_MUTED));
            }
            if let Some(completed) = self.datasets.last_stats_completion {
                let seconds = completed.elapsed().as_secs();
                ui.small(match seconds {
                    0 => "Updated just now".to_string(),
                    1 => "Updated 1 second ago".to_string(),
                    _ => format!("Updated {seconds} seconds ago"),
                });
            }
        });
        if let Some(error) = &self.datasets.stats_error {
            theme::inline_message(
                ui,
                theme::Intent::Warning,
                format!("Statistics may be stale. Last refresh failed: {error}"),
            );
        }
        let compact = layout == LayoutMode::Compact;
        let task_names = self
            .tasks
            .iter()
            .map(|task| (task.task_id.clone(), task.name.clone()))
            .collect::<BTreeMap<_, _>>();
        let class_names = self
            .classes
            .iter()
            .map(|class| (class.class_id.clone(), class.name.clone()))
            .collect::<BTreeMap<_, _>>();
        ui.add_space(8.0);
        let metrics = [
            ("Images", self.datasets.stats.total_images),
            ("Completed", self.datasets.stats.completed_tasks),
            ("Pending", self.datasets.stats.pending_tasks),
            ("Reviewed", self.datasets.stats.reviewed_tasks),
            ("Unreviewed", self.datasets.stats.unreviewed_tasks),
            ("Approved", self.datasets.stats.approved_tasks),
            ("Rejected", self.datasets.stats.rejected_tasks),
            (
                if compact {
                    "Corrected"
                } else {
                    "Reviewer corrected"
                },
                self.datasets.stats.reviewer_corrected_tasks,
            ),
            ("Finalized", self.datasets.stats.finalized_tasks),
        ];
        let minimum_card_width = if compact { 124.0 } else { 160.0 };
        let column_count = (((ui.available_width() + 10.0) / (minimum_card_width + 10.0)).floor()
            as usize)
            .clamp(1, 4);
        for row in metrics.chunks(column_count) {
            ui.columns(column_count, |columns| {
                for (column, (label, value)) in columns.iter_mut().zip(row) {
                    theme::metric(column, label, value.to_string());
                }
            });
        }
        ui.add_space(12.0);
        theme::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.heading("Per Task");
            let rows = &self.datasets.stats.per_task;
            if rows.is_empty() {
                theme::empty_state(
                    ui,
                    "No enabled tasks",
                    "Enable a labeling workflow to collect task statistics.",
                    None,
                );
            } else if compact {
                for (task_id, stats) in rows {
                    theme::inset_frame().show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.label(
                            RichText::new(
                                task_names
                                    .get(task_id)
                                    .map(String::as_str)
                                    .unwrap_or(task_id.as_str()),
                            )
                            .strong(),
                        );
                        ui.label(format!(
                            "Pending: {}  Unreviewed: {}  Reviewed: {}",
                            stats.pending, stats.unreviewed, stats.reviewed
                        ));
                        ui.label(format!(
                            "Approved: {}  Rejected: {}  Reviewer corrected: {}",
                            stats.approved, stats.rejected, stats.reviewer_corrected
                        ));
                        ui.label(format!(
                            "Finalized: {}  Done: {}",
                            stats.finalized, stats.completed
                        ));
                    });
                }
            } else {
                egui::ScrollArea::horizontal()
                    .id_salt("stats_tasks_horizontal")
                    .show(ui, |ui| {
                        stats_task_grid(ui, rows, &task_names);
                    });
            }
        });
        theme::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.heading("Per Class");
            let rows = &self.datasets.stats.per_class;
            if rows.is_empty() {
                theme::empty_state(
                    ui,
                    "No classes configured",
                    "Add a class to collect class-level statistics.",
                    None,
                );
            } else if compact {
                for (class_id, stats) in rows {
                    theme::inset_frame().show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.label(
                            RichText::new(
                                class_names
                                    .get(class_id)
                                    .map(String::as_str)
                                    .unwrap_or(class_id.as_str()),
                            )
                            .strong(),
                        );
                        ui.label(format!(
                            "Annotations: {}  Completed tasks: {}",
                            stats.annotations, stats.completed_tasks
                        ));
                    });
                }
            } else {
                egui::ScrollArea::horizontal()
                    .id_salt("stats_classes_horizontal")
                    .show(ui, |ui| {
                        stats_class_grid(ui, rows, &class_names);
                    });
            }
        });
        theme::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.heading("Throughput");
            if self.datasets.stats.throughput.is_empty() {
                theme::empty_state(
                    ui,
                    "No recorded activity",
                    "Throughput appears after annotations are created or reviews are recorded.",
                    None,
                );
            } else {
                stats_throughput_chart(ui, &self.datasets.stats.throughput);
            }
        });
    }
}

fn user_identity(ui: &mut egui::Ui, user: &DatasetUser) {
    ui.label(RichText::new(&user.account.display_name).strong());
    if let Some(login) = &user.account.github_login {
        ui.label(RichText::new(format!("@{login}")).color(theme::MUTED));
    }
    ui.small(format!("ID: {}", user.account.user_id));
}

fn user_matches_search(user: &DatasetUser, search: &str) -> bool {
    search.is_empty()
        || user.account.display_name.to_lowercase().contains(search)
        || user
            .account
            .user_id
            .as_str()
            .to_lowercase()
            .contains(search)
        || user
            .account
            .github_login
            .as_deref()
            .is_some_and(|login| login.to_lowercase().contains(search))
}

fn edit_user_roles(
    ui: &mut egui::Ui,
    user: &mut DatasetUser,
    current_user: &UserId,
    admin_count: usize,
    admin_loading: bool,
    saving: Option<&UserId>,
) {
    for (role, label) in [
        (DatasetRole::Annotator, "Annotator"),
        (DatasetRole::Reviewer, "Reviewer"),
        (DatasetRole::Adjudicator, "Adjudicator"),
        (DatasetRole::DataAdmin, "Data admin"),
    ] {
        let is_admin_role = role == DatasetRole::DataAdmin;
        let role_enabled = !admin_loading
            && saving.is_none()
            && !(is_admin_role
                && user.roles.contains(&role)
                && (&user.account.user_id == current_user || admin_count == 1));
        let mut enabled = user.roles.contains(&role);
        let response = ui
            .add_enabled(role_enabled, egui::Checkbox::new(&mut enabled, label))
            .on_disabled_hover_text(if &user.account.user_id == current_user {
                "You cannot remove your own data admin role."
            } else {
                "At least one data admin must remain."
            });
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Checkbox,
                role_enabled,
                enabled,
                format!(
                    "{label} role for {} ({})",
                    user.account.display_name, user.account.user_id
                ),
            )
        });
        if response.changed() {
            if enabled {
                user.roles.push(role);
                user.roles.sort();
                user.roles.dedup();
            } else {
                user.roles.retain(|existing| existing != &role);
            }
        }
    }
}

fn user_permissions_dirty(user: &DatasetUser, baseline: &[DatasetUser]) -> bool {
    baseline
        .iter()
        .find(|existing| existing.account.user_id == user.account.user_id)
        .is_none_or(|existing| existing.roles != user.roles)
}

fn save_permissions_button(
    ui: &mut egui::Ui,
    user: &DatasetUser,
    dirty: bool,
    admin_loading: bool,
    saving: Option<&UserId>,
) -> bool {
    let this_saving = saving == Some(&user.account.user_id);
    let enabled = dirty && saving.is_none() && !admin_loading;
    let response = theme::primary_button(
        ui,
        enabled,
        egui::Button::new(if this_saving {
            "Saving..."
        } else {
            "Save permissions"
        }),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            enabled,
            format!(
                "Save permissions for {} ({})",
                user.account.display_name, user.account.user_id
            ),
        )
    });
    response.clicked()
}

fn task_statuses() -> [TaskStatus; 6] {
    [
        TaskStatus::Pending,
        TaskStatus::InProgress,
        TaskStatus::Submitted,
        TaskStatus::Completed,
        TaskStatus::NeedsCorrection,
        TaskStatus::AdjudicationRequired,
    ]
}

fn task_status_label(status: &TaskStatus) -> &'static str {
    match status {
        TaskStatus::Pending => "Pending",
        TaskStatus::InProgress => "In progress",
        TaskStatus::Submitted => "Submitted",
        TaskStatus::Completed => "Completed",
        TaskStatus::NeedsCorrection => "Needs correction",
        TaskStatus::AdjudicationRequired => "Adjudication required",
    }
}

fn task_status_summary(statuses: &[TaskStatus]) -> String {
    let summary = task_statuses()
        .into_iter()
        .filter_map(|status| {
            let count = statuses
                .iter()
                .filter(|current| **current == status)
                .count();
            (count > 0).then(|| format!("{} {count}", task_status_label(&status)))
        })
        .collect::<Vec<_>>()
        .join(" | ");
    if summary.is_empty() {
        "No workflow status".to_string()
    } else {
        summary
    }
}

fn image_classes(item: &ImageExplorerItem) -> String {
    let classes = item
        .class_ids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    if classes.is_empty() {
        "None".to_string()
    } else {
        classes
    }
}

fn image_status_details(item: &ImageExplorerItem) -> String {
    let details = item
        .task_statuses
        .iter()
        .map(|(task, status)| format!("{task}: {}", task_status_label(status)))
        .collect::<Vec<_>>()
        .join("\n");
    if details.is_empty() {
        "No workflow status".to_string()
    } else {
        details
    }
}

fn admin_image_card(ui: &mut egui::Ui, item: &ImageExplorerItem) {
    let statuses = item.task_statuses.values().cloned().collect::<Vec<_>>();
    theme::inset_frame().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&item.image.file_name).strong());
            ui.label(
                RichText::new(format!(
                    "{} x {} | {}",
                    item.image.width,
                    item.image.height,
                    human_bytes(item.image.byte_size)
                ))
                .color(theme::INFO),
            );
        });
        ui.add(
            egui::Label::new(RichText::new(&item.image.canonical_path).color(theme::TEXT_MUTED))
                .truncate(),
        )
        .on_hover_text(&item.image.canonical_path);
        ui.label(
            RichText::new(format!(
                "{} | Classes: {}",
                task_status_summary(&statuses),
                image_classes(item)
            ))
            .color(theme::TEXT_MUTED),
        )
        .on_hover_text(image_status_details(item));
    });
}

fn admin_image_grid(ui: &mut egui::Ui, items: &[ImageExplorerItem]) {
    egui::ScrollArea::horizontal()
        .id_salt("admin-image-grid-scroll")
        .show(ui, |ui| {
            egui::Grid::new("admin-image-grid")
                .num_columns(5)
                .striped(true)
                .spacing([theme::SPACE_4, theme::SPACE_2])
                .show(ui, |ui| {
                    for heading in ["Image", "Dimensions", "Path", "Classes", "Workflow"] {
                        ui.label(RichText::new(heading).strong().color(theme::TEXT_MUTED));
                    }
                    ui.end_row();
                    for item in items {
                        let statuses = item.task_statuses.values().cloned().collect::<Vec<_>>();
                        ui.add_sized(
                            [180.0, 44.0],
                            egui::Label::new(RichText::new(&item.image.file_name).strong())
                                .truncate(),
                        )
                        .on_hover_text(&item.image.file_name);
                        ui.add_sized(
                            [120.0, 44.0],
                            egui::Label::new(format!(
                                "{} x {} | {}",
                                item.image.width,
                                item.image.height,
                                human_bytes(item.image.byte_size)
                            )),
                        );
                        ui.add_sized(
                            [240.0, 44.0],
                            egui::Label::new(&item.image.canonical_path).truncate(),
                        )
                        .on_hover_text(&item.image.canonical_path);
                        ui.add_sized(
                            [130.0, 44.0],
                            egui::Label::new(image_classes(item)).truncate(),
                        );
                        ui.add_sized(
                            [170.0, 44.0],
                            egui::Label::new(task_status_summary(&statuses)).truncate(),
                        )
                        .on_hover_text(image_status_details(item));
                        ui.end_row();
                    }
                });
        });
}

fn snapshot_expanded(ui: &egui::Ui, snapshot_id: &str) -> (egui::Id, bool) {
    let id = egui::Id::new(("admin-snapshot-files", snapshot_id));
    let expanded = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(id).unwrap_or(false));
    (id, expanded)
}

fn set_snapshot_expanded(ui: &egui::Ui, id: egui::Id, expanded: bool) {
    ui.ctx().data_mut(|data| data.insert_temp(id, expanded));
}

fn admin_snapshot_grid(
    ui: &mut egui::Ui,
    snapshots: &[DatasetSnapshot],
    active_download: Option<&(String, String)>,
) -> Option<(String, String)> {
    let mut download = None;
    egui::ScrollArea::horizontal()
        .id_salt("admin-snapshot-grid-scroll")
        .show(ui, |ui| {
            egui::Grid::new("admin-snapshot-grid")
                .num_columns(5)
                .striped(true)
                .spacing([theme::SPACE_4, theme::SPACE_2])
                .show(ui, |ui| {
                    for heading in ["Snapshot", "Created", "Files", "Size", "Details"] {
                        ui.label(RichText::new(heading).strong().color(theme::TEXT_MUTED));
                    }
                    ui.end_row();
                    for snapshot in snapshots {
                        let (expanded_id, mut expanded) =
                            snapshot_expanded(ui, &snapshot.snapshot_id);
                        ui.add_sized(
                            [200.0, 44.0],
                            egui::Label::new(RichText::new(&snapshot.snapshot_id).strong())
                                .truncate(),
                        )
                        .on_hover_text(&snapshot.snapshot_id);
                        ui.add_sized(
                            [180.0, 44.0],
                            egui::Label::new(
                                snapshot.created_at.format("%Y-%m-%d %H:%M UTC").to_string(),
                            ),
                        );
                        ui.add_sized(
                            [64.0, 44.0],
                            egui::Label::new(snapshot.files.len().to_string()),
                        );
                        ui.add_sized(
                            [100.0, 44.0],
                            egui::Label::new(human_bytes(snapshot.total_bytes)),
                        );
                        let details_label = if expanded { "Hide files" } else { "Show files" };
                        let details = ui.add_sized([110.0, 44.0], egui::Button::new(details_label));
                        details.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                true,
                                format!("{details_label} for snapshot {}", snapshot.snapshot_id),
                            )
                        });
                        if details.clicked() {
                            expanded = !expanded;
                            set_snapshot_expanded(ui, expanded_id, expanded);
                        }
                        ui.end_row();

                        if expanded {
                            for file in &snapshot.files {
                                let downloading = active_download
                                    == Some(&(snapshot.snapshot_id.clone(), file.path.clone()));
                                ui.add_sized(
                                    [200.0, 44.0],
                                    egui::Label::new(&file.path).truncate(),
                                )
                                .on_hover_text(&file.path);
                                ui.label("");
                                ui.label("File");
                                ui.label(human_bytes(file.byte_size));
                                let download_enabled = active_download.is_none();
                                let response = ui.add_enabled(
                                    download_enabled,
                                    egui::Button::new(if downloading {
                                        "Downloading..."
                                    } else {
                                        "Download"
                                    })
                                    .min_size(egui::vec2(110.0, 44.0)),
                                );
                                response.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::WidgetType::Button,
                                        download_enabled,
                                        format!(
                                            "Download {} from snapshot {}",
                                            file.path, snapshot.snapshot_id
                                        ),
                                    )
                                });
                                if response.clicked() {
                                    download =
                                        Some((snapshot.snapshot_id.clone(), file.path.clone()));
                                }
                                ui.end_row();
                            }
                        }
                    }
                });
        });
    download
}

fn admin_snapshot_cards(
    ui: &mut egui::Ui,
    snapshots: &[DatasetSnapshot],
    active_download: Option<&(String, String)>,
) -> Option<(String, String)> {
    let mut download = None;
    for snapshot in snapshots {
        ui.add_space(theme::SPACE_1);
        theme::inset_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new(&snapshot.snapshot_id).strong());
            ui.small(format!(
                "{} | {} files | {} total",
                snapshot.created_at.format("%Y-%m-%d %H:%M UTC"),
                snapshot.files.len(),
                human_bytes(snapshot.total_bytes)
            ));
            let (expanded_id, mut expanded) = snapshot_expanded(ui, &snapshot.snapshot_id);
            let details_label = if expanded { "Hide files" } else { "Show files" };
            let details = theme::quiet_button(ui, true, egui::Button::new(details_label));
            details.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    true,
                    format!("{details_label} for snapshot {}", snapshot.snapshot_id),
                )
            });
            if details.clicked() {
                expanded = !expanded;
                set_snapshot_expanded(ui, expanded_id, expanded);
            }
            if expanded {
                for file in &snapshot.files {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("{} ({})", file.path, human_bytes(file.byte_size)));
                        let downloading = active_download
                            == Some(&(snapshot.snapshot_id.clone(), file.path.clone()));
                        let download_enabled = active_download.is_none();
                        let response = ui.add_enabled(
                            download_enabled,
                            egui::Button::new(if downloading {
                                "Downloading..."
                            } else {
                                "Download"
                            })
                            .min_size(egui::vec2(110.0, 44.0)),
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                download_enabled,
                                format!(
                                    "Download {} from snapshot {}",
                                    file.path, snapshot.snapshot_id
                                ),
                            )
                        });
                        if response.clicked() {
                            download = Some((snapshot.snapshot_id.clone(), file.path.clone()));
                        }
                    });
                }
            }
        });
    }
    download
}

fn human_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn download_snapshot_file(_file: labello_client::SnapshotFile) -> Result<(), String> {
    Err("Snapshot downloads are available in the browser build.".to_string())
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn download_snapshot_file(file: labello_client::SnapshotFile) -> Result<(), String> {
    use wasm_bindgen::JsCast;

    let bytes = js_sys::Uint8Array::from(file.bytes.as_slice());
    let parts = js_sys::Array::new();
    parts.push(&bytes);
    let blob = web_sys::Blob::new_with_u8_array_sequence(&parts).map_err(js_error)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js_error)?;
    let result = (|| {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| "missing browser document".to_string())?;
        let anchor = document
            .create_element("a")
            .map_err(js_error)?
            .dyn_into::<web_sys::HtmlAnchorElement>()
            .map_err(|_| "failed to create download link".to_string())?;
        anchor.set_href(&url);
        anchor.set_download(file.file_name.rsplit('/').next().unwrap_or("snapshot-file"));
        anchor.click();
        Ok(())
    })();
    let _ = web_sys::Url::revoke_object_url(&url);
    result
}

#[cfg(target_arch = "wasm32")]
fn js_error(error: wasm_bindgen::JsValue) -> String {
    error.as_string().unwrap_or_else(|| format!("{error:?}"))
}

fn edit_quick_workflows(ui: &mut egui::Ui, config: &mut DatasetMetadata) {
    theme::card_frame().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.heading("Class Workflows");
        ui.label(
            RichText::new("Fast path: create a class and its worker-visible task together.")
                .color(theme::MUTED),
        );
        ui.horizontal_wrapped(|ui| {
            if ui.button("Add bounding box class workflow").clicked() {
                add_class_workflow(config, AnnotationType::BoundingBox);
            }
            if ui.button("Add skeleton class workflow").clicked() {
                add_class_workflow(config, AnnotationType::Skeleton);
            }
        });
        ui.add_space(8.0);
        let labels = config.label_classes.clone();
        if labels.is_empty() {
            ui.small("No classes yet. Use one of the buttons above to create the first workflow.");
        }
        for label in labels {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(&label.name).strong());
                ui.small(format!("{}", label.class_id));
                let annotation_type = AnnotationType::BoundingBox;
                let exists = has_task_for_class(config, &label.class_id, &annotation_type);
                if ui
                    .add_enabled(!exists, egui::Button::new("Add bounding_box task"))
                    .clicked()
                {
                    add_task_for_class(config, &label, annotation_type);
                }
                let annotation_type = AnnotationType::Skeleton;
                let exists = has_task_for_class(config, &label.class_id, &annotation_type);
                if ui
                    .add_enabled(!exists, egui::Button::new("Add skeleton task"))
                    .clicked()
                {
                    add_task_for_class(config, &label, annotation_type);
                }
            });
        }
    });
}

fn add_class_workflow(config: &mut DatasetMetadata, annotation_type: AnnotationType) {
    let index = config.label_classes.len() + 1;
    let class = LabelClass {
        class_id: ClassId::from(next_class_id(config)),
        name: if index == 1 {
            "Object".to_string()
        } else {
            format!("Object {index}")
        },
        color: default_class_color(index),
        description: None,
    };
    config.label_classes.push(class.clone());
    add_task_for_class(config, &class, annotation_type);
}

fn add_task_for_class(
    config: &mut DatasetMetadata,
    class: &LabelClass,
    annotation_type: AnnotationType,
) {
    if has_task_for_class(config, &class.class_id, &annotation_type) {
        return;
    }
    config
        .tasks
        .push(workflow_task_for_class(class, annotation_type));
}

fn workflow_task_for_class(class: &LabelClass, annotation_type: AnnotationType) -> TaskDefinition {
    let task_id = match annotation_type {
        AnnotationType::BoundingBox => format!("bounding_box:{}", class.class_id),
        AnnotationType::Skeleton => format!("skeleton:{}", class.class_id),
    };
    let name = match annotation_type {
        AnnotationType::BoundingBox => format!("{} bounding boxes", class.name),
        AnnotationType::Skeleton => format!("{} skeletons", class.name),
    };
    let skeleton = (annotation_type == AnnotationType::Skeleton).then(starter_skeleton_spec);
    TaskDefinition {
        task_id: TaskId::from(task_id),
        name,
        annotation_type,
        class_ids: vec![class.class_id.clone()],
        instructions: TutorialContent {
            title: "Label every visible object".to_string(),
            example_text: "Annotate every visible instance of the configured class.".to_string(),
            example_images: Vec::new(),
        },
        skeleton,
        review: ReviewConfig::default(),
        prelabel_config_ids: Vec::new(),
        enabled: true,
    }
}

fn starter_skeleton_spec() -> SkeletonSpec {
    SkeletonSpec {
        keypoints: vec![KeypointSpec {
            name: "keypoint_1".to_string(),
            required: true,
        }],
        edges: Vec::new(),
        allow_hidden: false,
        allow_absent: false,
    }
}

fn has_task_for_class(
    config: &DatasetMetadata,
    class_id: &ClassId,
    annotation_type: &AnnotationType,
) -> bool {
    config
        .tasks
        .iter()
        .any(|task| &task.annotation_type == annotation_type && task.class_ids.contains(class_id))
}

fn next_class_id(config: &DatasetMetadata) -> String {
    for index in 1.. {
        let candidate = if index == 1 {
            "object".to_string()
        } else {
            format!("object_{index}")
        };
        if !config
            .label_classes
            .iter()
            .any(|class| class.class_id.as_str() == candidate)
        {
            return candidate;
        }
    }
    unreachable!()
}

fn default_class_color(index: usize) -> String {
    const COLORS: [&str; 6] = [
        "#5eead4", "#60a5fa", "#fbbf24", "#f472b6", "#a78bfa", "#34d399",
    ];
    COLORS[(index - 1) % COLORS.len()].to_string()
}

fn edit_string_list(
    ui: &mut egui::Ui,
    values: &mut Vec<String>,
    label: &str,
    button: &str,
    default: &str,
) {
    let mut remove = None;
    for (index, value) in values.iter_mut().enumerate() {
        ui.horizontal_wrapped(|ui| {
            let field_label = ui.label(label);
            ui.add_sized(
                [ui.available_width().min(360.0), 44.0],
                egui::TextEdit::singleline(value),
            )
            .labelled_by(field_label.id)
            .on_hover_text("Dataset-relative path under the dataset root.");
            if destructive_button(ui, "Remove", format!("{label} '{value}'")) {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        values.remove(index);
    }
    if ui
        .button(button)
        .on_hover_text("Add another entry.")
        .clicked()
    {
        values.push(default.to_string());
    }
}

fn edit_labels(ui: &mut egui::Ui, labels: &mut Vec<LabelClass>, tasks: &mut [TaskDefinition]) {
    theme::card_frame().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.heading("Classes");
        ui.label(
            RichText::new("Classes define the objects annotators can label.").color(theme::MUTED),
        );
        let mut remove = None;
        let wide = ui.available_width() >= 600.0;
        for (index, label) in labels.iter_mut().enumerate() {
            ui.add_space(4.0);
            theme::inset_frame().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                if edit_class_card(ui, index, label, tasks, wide) {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            let removed = labels.remove(index).class_id;
            for task in tasks.iter_mut() {
                task.class_ids.retain(|class_id| class_id != &removed);
            }
        }
        if ui.button("Add class").clicked() {
            labels.push(LabelClass {
                class_id: ClassId::from(next_numbered_id(
                    "class",
                    labels.iter().map(|label| label.class_id.as_str()),
                )),
                name: "New class".to_string(),
                color: default_class_color(labels.len() + 1),
                description: None,
            });
        }
        show_issues(ui, &class_issues(labels));
    });
}

fn edit_class_card(
    ui: &mut egui::Ui,
    index: usize,
    label: &mut LabelClass,
    tasks: &mut [TaskDefinition],
    wide: bool,
) -> bool {
    let mut class_id = label.class_id.to_string();
    let mut description = label.description.clone().unwrap_or_default();
    let mut remove = false;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("Class {}", index + 1)).strong());
        ui.label(RichText::new(&label.name).color(theme::BLUE));
        remove = destructive_button(
            ui,
            "Remove class",
            format!("class '{}' ({})", label.name, label.class_id),
        );
    });

    let (id_changed, description_changed) = if wide {
        let mut id_changed = false;
        let mut description_changed = false;
        let spacing = ui.spacing().item_spacing.x;
        let unit = (ui.available_width() - 3.0 * spacing) / 6.0;
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(unit, 68.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let field_label = ui.label("Name");
                    ui.add(egui::TextEdit::singleline(&mut label.name).desired_width(unit))
                        .labelled_by(field_label.id)
                        .on_hover_text("Display name shown to annotators.");
                },
            );
            ui.allocate_ui_with_layout(
                egui::vec2(unit, 68.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let field_label = ui.label("ID");
                    id_changed = ui
                        .add(egui::TextEdit::singleline(&mut class_id).desired_width(unit))
                        .labelled_by(field_label.id)
                        .on_hover_text("Stable class id used by annotations and linked workflows.")
                        .changed();
                },
            );
            ui.allocate_ui_with_layout(
                egui::vec2(unit, 68.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let field_label = ui.label("Color");
                    ui.add(egui::TextEdit::singleline(&mut label.color).desired_width(unit))
                        .labelled_by(field_label.id)
                        .on_hover_text("Class color as a hex value, for example #5eead4.");
                },
            );
            ui.allocate_ui_with_layout(
                egui::vec2(3.0 * unit, 68.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let field_label = ui.label("Description");
                    description_changed = ui
                        .add(egui::TextEdit::singleline(&mut description).desired_width(3.0 * unit))
                        .labelled_by(field_label.id)
                        .on_hover_text("Optional guidance about what belongs in this class.")
                        .changed();
                },
            );
        });
        (id_changed, description_changed)
    } else {
        theme::labeled_text_field(ui, "Name", &mut label.name, 44.0)
            .on_hover_text("Display name shown to annotators.");
        let id_changed = theme::labeled_text_field(ui, "ID", &mut class_id, 44.0)
            .on_hover_text("Stable class id used by annotations and linked workflows.")
            .changed();
        theme::labeled_text_field(ui, "Color", &mut label.color, 44.0)
            .on_hover_text("Class color as a hex value, for example #5eead4.");
        let description_changed =
            theme::labeled_text_field(ui, "Description", &mut description, 44.0)
                .on_hover_text("Optional guidance about what belongs in this class.")
                .changed();
        (id_changed, description_changed)
    };

    if id_changed {
        let previous = label.class_id.clone();
        let updated = ClassId::from(class_id);
        label.class_id = updated.clone();
        for task in tasks.iter_mut() {
            for class_id in &mut task.class_ids {
                if class_id == &previous {
                    *class_id = updated.clone();
                }
            }
        }
    }
    if description_changed {
        label.description = (!description.trim().is_empty()).then_some(description);
    }

    remove
}

fn edit_tasks(
    ui: &mut egui::Ui,
    tasks: &mut Vec<TaskDefinition>,
    labels: &[LabelClass],
    prelabels: &[PrelabelConfig],
) {
    theme::card_frame().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.heading("Labeling Workflows");
        ui.label(
            RichText::new(
                "Each workflow is one annotation type and one class. Annotators choose between these workflows before claiming work.",
            )
            .color(theme::MUTED),
        );
        let mut remove = None;
        for (index, task) in tasks.iter_mut().enumerate() {
            normalize_task_annotation(task);
            let class_name = task
                .class_ids
                .first()
                .and_then(|class_id| labels.iter().find(|label| &label.class_id == class_id))
                .map(|label| label.name.as_str())
                .unwrap_or("No class");
            let summary = format!(
                "{} | {} | {} | {}",
                task.name,
                task.annotation_type,
                class_name,
                if task.enabled { "Enabled" } else { "Disabled" }
            );
            ui.add_space(4.0);
            theme::inset_frame()
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    egui::CollapsingHeader::new(summary)
                        .id_salt(("workflow-editor", index))
                        .default_open(false)
                        .show(ui, |ui| {
                            let remove_clicked = if ui.available_width() >= 760.0 {
                                let mut remove_clicked = false;
                                ui.columns(2, |columns| {
                                    remove_clicked = edit_workflow_basics(
                                        &mut columns[0],
                                        index,
                                        task,
                                        labels,
                                        prelabels,
                                    );
                                    edit_workflow_instructions(&mut columns[1], task);
                                });
                                remove_clicked
                            } else {
                                let remove_clicked =
                                    edit_workflow_basics(ui, index, task, labels, prelabels);
                                edit_workflow_instructions(ui, task);
                                remove_clicked
                            };
                            if remove_clicked {
                                remove = Some(index);
                            }
                        });
                });
        }
        if let Some(index) = remove {
            tasks.remove(index);
        }
        if ui.button("Add workflow").clicked() {
            let class_ids = labels
                .first()
                .map(|label| vec![label.class_id.clone()])
                .unwrap_or_default();
            tasks.push(TaskDefinition {
                task_id: TaskId::from(next_numbered_id(
                    "task",
                    tasks.iter().map(|task| task.task_id.as_str()),
                )),
                name: "New task".to_string(),
                annotation_type: AnnotationType::BoundingBox,
                class_ids,
                instructions: TutorialContent {
                    title: "Instructions".to_string(),
                    example_text: "Describe what annotators should label.".to_string(),
                    example_images: Vec::new(),
                },
                skeleton: None,
                review: ReviewConfig::default(),
                prelabel_config_ids: Vec::new(),
                enabled: true,
            });
        }
        show_issues(ui, &task_issues(tasks, labels, prelabels));
    });
}

fn edit_workflow_basics(
    ui: &mut egui::Ui,
    index: usize,
    task: &mut TaskDefinition,
    labels: &[LabelClass],
    prelabels: &[PrelabelConfig],
) -> bool {
    ui.label(RichText::new("Workflow").color(theme::BLUE).strong());
    let mut task_id = task.task_id.to_string();
    if theme::labeled_text_field(ui, "Task ID", &mut task_id, 44.0)
        .on_hover_text("Stable task id used by assignments and event logs.")
        .changed()
    {
        task.task_id = TaskId::from(task_id);
    }
    theme::labeled_text_field(ui, "Name", &mut task.name, 44.0)
        .on_hover_text("Task name shown in the work panel.");
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut task.enabled, "Enabled");
        ui.label("Annotation type");
        let mut annotation_type = task.annotation_type.clone();
        egui::ComboBox::from_id_salt(format!("task-type-{index}"))
            .selected_text(annotation_type.to_string())
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut annotation_type,
                    AnnotationType::BoundingBox,
                    "bounding_box",
                );
                ui.selectable_value(&mut annotation_type, AnnotationType::Skeleton, "skeleton");
            });
        if annotation_type != task.annotation_type {
            set_task_annotation_type(task, annotation_type);
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label("Class");
        if labels.is_empty() {
            ui.label(RichText::new("Add a class first.").color(theme::RED));
        } else {
            let mut selected = task.class_ids.first().cloned();
            let selected_text = selected
                .as_ref()
                .and_then(|class_id| labels.iter().find(|label| &label.class_id == class_id))
                .map(|label| label.name.clone())
                .unwrap_or_else(|| "Select a class".to_string());
            egui::ComboBox::from_id_salt(format!("task-class-{index}"))
                .selected_text(selected_text)
                .show_ui(ui, |ui| {
                    for label in labels {
                        ui.selectable_value(
                            &mut selected,
                            Some(label.class_id.clone()),
                            &label.name,
                        );
                    }
                });
            if selected.as_ref() != task.class_ids.first()
                || task.class_ids.len() != usize::from(selected.is_some())
            {
                task.class_ids = selected.into_iter().collect();
            }
        }
    });
    if task.annotation_type == AnnotationType::Skeleton
        && let Some(skeleton) = task.skeleton.as_mut()
    {
        edit_skeleton(ui, index, skeleton);
    }
    ui.label("Prelabel sources");
    if prelabels.is_empty() {
        ui.small("No prelabel sources configured.");
    }
    for prelabel in prelabels {
        let mut enabled = task.prelabel_config_ids.contains(&prelabel.config_id);
        if ui
            .checkbox(
                &mut enabled,
                format!("{} ({})", prelabel.name, prelabel.config_id),
            )
            .changed()
        {
            if enabled {
                task.prelabel_config_ids.push(prelabel.config_id.clone());
            } else {
                task.prelabel_config_ids
                    .retain(|config_id| config_id != &prelabel.config_id);
            }
        }
    }
    edit_review(ui, index, task);
    destructive_button(
        ui,
        "Remove workflow",
        format!("workflow '{}' ({})", task.name, task.task_id),
    )
}

fn edit_workflow_instructions(ui: &mut egui::Ui, task: &mut TaskDefinition) {
    ui.label(
        RichText::new("Annotator instructions")
            .color(theme::BLUE)
            .strong(),
    );
    theme::labeled_text_field(ui, "Title", &mut task.instructions.title, 44.0)
        .on_hover_text("Tutorial/instruction title.");
    ui.label("Tutorial instructions");
    ui.add(
        egui::TextEdit::multiline(&mut task.instructions.example_text)
            .desired_width(ui.available_width())
            .desired_rows(3),
    )
    .on_hover_text("Instructions annotators see in the tutorial panel.");
    ui.label("Tutorial example images");
    edit_string_list(
        ui,
        &mut task.instructions.example_images,
        "Image path",
        "Add example image",
        "tutorial/example.png",
    );
}

fn normalize_task_annotation(task: &mut TaskDefinition) {
    match task.annotation_type {
        AnnotationType::BoundingBox => task.skeleton = None,
        AnnotationType::Skeleton => {
            task.skeleton.get_or_insert_with(starter_skeleton_spec);
        }
    }
}

fn set_task_annotation_type(task: &mut TaskDefinition, annotation_type: AnnotationType) {
    task.annotation_type = annotation_type;
    normalize_task_annotation(task);
    if let Some(agreement) = task.review.agreement_threshold.as_mut() {
        agreement.metric = match task.annotation_type {
            AnnotationType::BoundingBox => AgreementMetric::Iou,
            AnnotationType::Skeleton => AgreementMetric::KeypointMeanDistance,
        };
    }
}

fn edit_skeleton(ui: &mut egui::Ui, task_index: usize, skeleton: &mut SkeletonSpec) {
    egui::CollapsingHeader::new("Skeleton configuration")
        .id_salt(("skeleton-configuration", task_index))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut skeleton.allow_hidden, "Allow hidden keypoints")
                    .on_hover_text(
                        "Annotators may mark a keypoint as hidden behind another object.",
                    );
                ui.checkbox(&mut skeleton.allow_absent, "Allow absent keypoints")
                    .on_hover_text(
                        "Annotators may mark a keypoint as outside the image or absent.",
                    );
            });

            ui.label(RichText::new("Keypoints").strong());
            let mut remove_keypoint = None;
            let mut renames = Vec::new();
            for (keypoint_index, keypoint) in skeleton.keypoints.iter_mut().enumerate() {
                let previous_name = keypoint.name.clone();
                ui.horizontal_wrapped(|ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut keypoint.name)
                        .on_hover_text("Unique keypoint name used by skeleton edges.");
                    ui.checkbox(&mut keypoint.required, "Required");
                    if destructive_button(
                        ui,
                        "Remove keypoint",
                        format!("keypoint '{}'", keypoint.name),
                    ) {
                        remove_keypoint = Some(keypoint_index);
                    }
                });
                if keypoint.name != previous_name {
                    renames.push((previous_name, keypoint.name.clone()));
                }
            }
            for (previous, updated) in renames {
                for edge in &mut skeleton.edges {
                    if edge.from == previous {
                        edge.from = updated.clone();
                    }
                    if edge.to == previous {
                        edge.to = updated.clone();
                    }
                }
            }
            if let Some(index) = remove_keypoint {
                let removed = skeleton.keypoints.remove(index).name;
                skeleton
                    .edges
                    .retain(|edge| edge.from != removed && edge.to != removed);
            }
            if ui.button("Add keypoint").clicked() {
                skeleton.keypoints.push(KeypointSpec {
                    name: next_keypoint_name(skeleton),
                    required: true,
                });
            }

            ui.add_space(4.0);
            ui.label(RichText::new("Edges").strong());
            let keypoint_names: Vec<_> = skeleton
                .keypoints
                .iter()
                .map(|keypoint| keypoint.name.clone())
                .collect();
            let mut remove_edge = None;
            for (edge_index, edge) in skeleton.edges.iter_mut().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    ui.label("From");
                    egui::ComboBox::from_id_salt(format!(
                        "skeleton-edge-from-{task_index}-{edge_index}"
                    ))
                    .selected_text(&edge.from)
                    .show_ui(ui, |ui| {
                        for name in &keypoint_names {
                            ui.selectable_value(&mut edge.from, name.clone(), name);
                        }
                    });
                    ui.label("To");
                    egui::ComboBox::from_id_salt(format!(
                        "skeleton-edge-to-{task_index}-{edge_index}"
                    ))
                    .selected_text(&edge.to)
                    .show_ui(ui, |ui| {
                        for name in &keypoint_names {
                            ui.selectable_value(&mut edge.to, name.clone(), name);
                        }
                    });
                    if destructive_button(
                        ui,
                        "Remove edge",
                        format!("edge '{} -> {}'", edge.from, edge.to),
                    ) {
                        remove_edge = Some(edge_index);
                    }
                });
            }
            if let Some(index) = remove_edge {
                skeleton.edges.remove(index);
            }
            let next_edge = next_skeleton_edge(skeleton);
            if ui
                .add_enabled(next_edge.is_some(), egui::Button::new("Add edge"))
                .on_hover_text(if next_edge.is_some() {
                    "Connect two keypoints that are not already connected."
                } else {
                    "Add at least two keypoints, or remove an existing edge first."
                })
                .clicked()
                && let Some(edge) = next_edge
            {
                skeleton.edges.push(edge);
            }

            show_issues(ui, &skeleton_issues(skeleton, "Skeleton"));
        });
}

fn next_keypoint_name(skeleton: &SkeletonSpec) -> String {
    next_numbered_id(
        "keypoint",
        skeleton
            .keypoints
            .iter()
            .map(|keypoint| keypoint.name.as_str()),
    )
}

fn next_skeleton_edge(skeleton: &SkeletonSpec) -> Option<SkeletonEdge> {
    for (from_index, from) in skeleton.keypoints.iter().enumerate() {
        for to in skeleton.keypoints.iter().skip(from_index + 1) {
            if from.name == to.name {
                continue;
            }
            let candidate = canonical_edge(&from.name, &to.name);
            let exists = skeleton
                .edges
                .iter()
                .any(|edge| canonical_edge(&edge.from, &edge.to) == candidate);
            if !exists {
                return Some(SkeletonEdge {
                    from: from.name.clone(),
                    to: to.name.clone(),
                });
            }
        }
    }
    None
}

fn canonical_edge<'a>(from: &'a str, to: &'a str) -> (&'a str, &'a str) {
    if from <= to { (from, to) } else { (to, from) }
}

fn edit_review(ui: &mut egui::Ui, task_index: usize, task: &mut TaskDefinition) {
    egui::CollapsingHeader::new("Review configuration")
        .id_salt(("review-configuration", task_index))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Workflow");
                let previous = task.review.workflow.clone();
                egui::ComboBox::from_id_salt(format!("review-workflow-{task_index}"))
                    .selected_text(review_workflow_name(&task.review.workflow))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut task.review.workflow,
                            ReviewWorkflow::None,
                            "none",
                        );
                        ui.selectable_value(
                            &mut task.review.workflow,
                            ReviewWorkflow::Approval,
                            "approval",
                        );
                        ui.selectable_value(
                            &mut task.review.workflow,
                            ReviewWorkflow::IndependentAgreement,
                            "independent agreement",
                        );
                    });
                if task.review.workflow != previous {
                    match task.review.workflow {
                        ReviewWorkflow::None => {
                            task.review.required_reviews = 0;
                            task.review.agreement_threshold = None;
                        }
                        ReviewWorkflow::Approval => {
                            task.review.required_reviews = task.review.required_reviews.max(1);
                            task.review.agreement_threshold = None;
                        }
                        ReviewWorkflow::IndependentAgreement => {
                            task.review.required_reviews = task.review.required_reviews.max(2);
                            task.review.agreement_threshold = Some(default_agreement(task));
                        }
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Required reviews");
                ui.add(
                    egui::DragValue::new(&mut task.review.required_reviews)
                        .range(0..=100)
                        .speed(1),
                )
                .on_hover_text("Number of completed reviews required for this task.");
                ui.checkbox(
                    &mut task.review.allow_reviewer_corrections,
                    "Allow reviewer correction",
                );
            });
            if task.review.workflow == ReviewWorkflow::IndependentAgreement {
                let mut enabled = task.review.agreement_threshold.is_some();
                if ui
                    .checkbox(&mut enabled, "Use agreement threshold")
                    .changed()
                {
                    task.review.agreement_threshold = enabled.then(|| default_agreement(task));
                }
                if let Some(agreement) = task.review.agreement_threshold.as_mut() {
                    agreement.metric = match task.annotation_type {
                        AnnotationType::BoundingBox => AgreementMetric::Iou,
                        AnnotationType::Skeleton => AgreementMetric::KeypointMeanDistance,
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Agreement metric");
                        ui.label(match agreement.metric {
                            AgreementMetric::Iou => "intersection over union",
                            AgreementMetric::KeypointMeanDistance => "keypoint mean distance",
                        });
                        ui.add(
                            egui::Slider::new(&mut agreement.threshold, 0.0..=1.0)
                                .text("threshold"),
                        );
                    });
                }
            }
        });
}

fn default_agreement(task: &TaskDefinition) -> AgreementThreshold {
    AgreementThreshold {
        metric: match task.annotation_type {
            AnnotationType::BoundingBox => AgreementMetric::Iou,
            AnnotationType::Skeleton => AgreementMetric::KeypointMeanDistance,
        },
        threshold: 0.5,
    }
}

fn review_workflow_name(workflow: &ReviewWorkflow) -> &'static str {
    match workflow {
        ReviewWorkflow::None => "none",
        ReviewWorkflow::Approval => "approval",
        ReviewWorkflow::IndependentAgreement => "independent agreement",
    }
}

fn edit_prelabels(
    ui: &mut egui::Ui,
    configs: &mut Vec<PrelabelConfig>,
    tasks: &mut [TaskDefinition],
) {
    theme::card_frame().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.heading("Prelabels");
        let mut remove = None;
        for (index, config) in configs.iter_mut().enumerate() {
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                ui.label("Prelabel ID");
                let mut config_id = config.config_id.to_string();
                if ui
                    .text_edit_singleline(&mut config_id)
                    .on_hover_text("Stable prelabel config id referenced by tasks.")
                    .changed()
                {
                    let previous = config.config_id.clone();
                    let updated = PrelabelConfigId::from(config_id);
                    config.config_id = updated.clone();
                    for task in tasks.iter_mut() {
                        for config_id in &mut task.prelabel_config_ids {
                            if config_id == &previous {
                                *config_id = updated.clone();
                            }
                        }
                    }
                }
                ui.label("Name");
                ui.text_edit_singleline(&mut config.name)
                    .on_hover_text("Display name for this prelabel source.");
                ui.checkbox(
                    &mut config.available_to_annotators,
                    "Available to annotators",
                );
                if destructive_button(
                    ui,
                    "Remove prelabel",
                    format!("prelabel '{}' ({})", config.name, config.config_id),
                ) {
                    remove = Some(index);
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Model ID");
                ui.text_edit_singleline(&mut config.model.model_id)
                    .on_hover_text("Stable model id.");
                ui.label("Model name");
                ui.text_edit_singleline(&mut config.model.display_name)
                    .on_hover_text("Model display name.");
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Location");
                ui.text_edit_singleline(&mut config.model.location)
                    .on_hover_text("Server/browser model location, depending on execution mode.");
            });
            ui.add(
                egui::Slider::new(
                    &mut config.output_processing.confidence_threshold,
                    0.0..=1.0,
                )
                .text("confidence"),
            );
        }
        if let Some(index) = remove {
            let removed = configs.remove(index).config_id;
            for task in tasks.iter_mut() {
                task.prelabel_config_ids
                    .retain(|config_id| config_id != &removed);
            }
        }
        if ui.button("Add browser prelabel config").clicked() {
            configs.push(PrelabelConfig {
                config_id: PrelabelConfigId::from(next_numbered_id(
                    "prelabel",
                    configs.iter().map(|config| config.config_id.as_str()),
                )),
                name: "New prelabel".to_string(),
                model: ModelSpec {
                    model_id: "model".to_string(),
                    display_name: "Model".to_string(),
                    version: None,
                    location: "models/model.onnx".to_string(),
                },
                execution: PrelabelExecution::BrowserLocal {
                    acceleration: BrowserAcceleration::WasmCpuFallback,
                },
                output_processing: OutputProcessing {
                    confidence_threshold: 0.5,
                    suppress_overlaps_iou: None,
                },
                available_to_annotators: true,
            });
        }
        show_issues(ui, &prelabel_issues(configs));
    });
}

fn edit_imbalance(ui: &mut egui::Ui, imbalance: &mut Option<ImbalanceConfig>) {
    theme::card_frame().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.heading("Assignment Balance");
        ui.label(
            RichText::new("Limit how unevenly work may be distributed across classes.")
                .color(theme::MUTED),
        );
        let mut configured = imbalance.is_some();
        if ui
            .checkbox(&mut configured, "Configure imbalance limits")
            .changed()
        {
            *imbalance = configured.then(ImbalanceConfig::default);
        }
        if let Some(imbalance) = imbalance.as_mut() {
            ui.horizontal_wrapped(|ui| {
                ui.label("Maximum class ratio");
                ui.add(
                    egui::DragValue::new(&mut imbalance.max_ratio)
                        .range(1.0..=1000.0)
                        .speed(0.1),
                )
                .on_hover_text(
                    "Largest allowed ratio between over- and under-represented classes.",
                );
                ui.checkbox(&mut imbalance.enforce, "Enforce limit");
            });
            show_issues(ui, &imbalance_issues(Some(imbalance)));
        }
    });
}

fn next_numbered_id<'a>(prefix: &str, values: impl Iterator<Item = &'a str>) -> String {
    let values: BTreeSet<_> = values.collect();
    for index in 1.. {
        let candidate = format!("{prefix}_{index}");
        if !values.contains(candidate.as_str()) {
            return candidate;
        }
    }
    unreachable!()
}

fn config_issues(config: &DatasetMetadata, current_user: &UserId) -> Vec<String> {
    let mut issues = dataset_issues(&config.name, &config.image_roots);
    issues.extend(class_issues(&config.label_classes));
    issues.extend(task_issues(
        &config.tasks,
        &config.label_classes,
        &config.prelabel_configs,
    ));
    issues.extend(prelabel_issues(&config.prelabel_configs));
    issues.extend(imbalance_issues(config.imbalance.as_ref()));
    issues.extend(role_issues(
        &config.role_assignments,
        &config.dataset_id,
        current_user,
    ));
    issues
}

fn dataset_issues(name: &str, image_roots: &[String]) -> Vec<String> {
    let mut issues = dataset_name_issues(name);
    issues.extend(image_root_issues(image_roots));
    issues
}

fn dataset_name_issues(name: &str) -> Vec<String> {
    let mut issues = Vec::new();
    if name.trim().is_empty() {
        issues.push("Dataset: enter a non-empty dataset name.".to_string());
    }
    issues
}

fn image_root_issues(image_roots: &[String]) -> Vec<String> {
    let mut issues = Vec::new();
    if image_roots.is_empty() {
        issues.push("Image roots: add at least one dataset-relative root path.".to_string());
    }
    for (index, root) in image_roots.iter().enumerate() {
        if !is_safe_relative_path(root) {
            issues.push(format!(
                "Image roots: root {} must be a non-empty relative path without '..' or backslashes.",
                index + 1
            ));
        }
    }
    issues
}

fn class_issues(labels: &[LabelClass]) -> Vec<String> {
    let mut issues = Vec::new();
    let mut ids = BTreeSet::new();
    for (index, label) in labels.iter().enumerate() {
        let context = format!("Class {}", index + 1);
        validate_id(&mut issues, &context, label.class_id.as_str());
        if !ids.insert(label.class_id.as_str()) {
            issues.push(format!(
                "Classes: class ID '{}' is duplicated; choose a unique ID.",
                label.class_id
            ));
        }
        if label.name.trim().is_empty() {
            issues.push(format!("{context}: enter a non-empty class name."));
        }
        if !is_hex_color(&label.color) {
            issues.push(format!(
                "{context}: color '{}' is invalid; use # followed by six hexadecimal digits.",
                label.color
            ));
        }
    }
    issues
}

fn task_issues(
    tasks: &[TaskDefinition],
    labels: &[LabelClass],
    prelabels: &[PrelabelConfig],
) -> Vec<String> {
    let mut issues = Vec::new();
    let class_ids: BTreeSet<_> = labels.iter().map(|label| &label.class_id).collect();
    let prelabel_ids: BTreeSet<_> = prelabels.iter().map(|config| &config.config_id).collect();
    let mut task_ids = BTreeSet::new();
    for (index, task) in tasks.iter().enumerate() {
        let context = format!("Workflow {}", index + 1);
        validate_id(&mut issues, &context, task.task_id.as_str());
        if !task_ids.insert(task.task_id.as_str()) {
            issues.push(format!(
                "Workflows: task ID '{}' is duplicated; choose a unique ID.",
                task.task_id
            ));
        }
        if task.name.trim().is_empty() {
            issues.push(format!("{context}: enter a non-empty task name."));
        }
        if task.enabled && task.class_ids.len() != 1 {
            issues.push(format!(
                "{context}: enabled workflow '{}' must select exactly one class.",
                task.task_id
            ));
        } else if task.class_ids.len() > 1 {
            issues.push(format!(
                "{context}: workflow '{}' can reference only one class.",
                task.task_id
            ));
        }
        let mut linked_classes = BTreeSet::new();
        for class_id in &task.class_ids {
            if !class_ids.contains(class_id) {
                issues.push(format!(
                    "{context}: task '{}' references missing class '{}'; select an existing class or remove the reference.",
                    task.task_id, class_id
                ));
            }
            if !linked_classes.insert(class_id) {
                issues.push(format!(
                    "{context}: class '{}' is selected more than once.",
                    class_id
                ));
            }
        }
        let mut linked_prelabels = BTreeSet::new();
        for config_id in &task.prelabel_config_ids {
            if !prelabel_ids.contains(config_id) {
                issues.push(format!(
                    "{context}: task '{}' references missing prelabel '{}'; select an existing source or remove the reference.",
                    task.task_id, config_id
                ));
            }
            if !linked_prelabels.insert(config_id) {
                issues.push(format!(
                    "{context}: prelabel '{}' is selected more than once.",
                    config_id
                ));
            }
        }
        if task.instructions.title.trim().is_empty() {
            issues.push(format!("{context}: enter a non-empty instruction title."));
        }
        for (image_index, path) in task.instructions.example_images.iter().enumerate() {
            if !is_safe_relative_path(path) {
                issues.push(format!(
                    "{context}: tutorial image path {} must be a non-empty dataset-relative path without '..' or backslashes.",
                    image_index + 1
                ));
            }
        }
        if task.annotation_type == AnnotationType::Skeleton {
            if let Some(skeleton) = task.skeleton.as_ref() {
                issues.extend(skeleton_issues(skeleton, &format!("{context} skeleton")));
            } else {
                issues.push(format!(
                    "{context}: skeleton task '{}' needs a skeleton specification.",
                    task.task_id
                ));
            }
        }
        validate_review(&mut issues, &context, task);
    }
    issues
}

fn skeleton_issues(skeleton: &SkeletonSpec, context: &str) -> Vec<String> {
    let mut issues = Vec::new();
    if skeleton.keypoints.is_empty() {
        issues.push(format!("{context}: add at least one keypoint."));
    }

    let mut keypoint_names = BTreeSet::new();
    for (index, keypoint) in skeleton.keypoints.iter().enumerate() {
        if keypoint.name.trim().is_empty() {
            issues.push(format!(
                "{context}: keypoint {} needs a non-empty name.",
                index + 1
            ));
        }
        if !keypoint_names.insert(keypoint.name.as_str()) {
            issues.push(format!(
                "{context}: keypoint name '{}' is duplicated; choose a unique name.",
                keypoint.name
            ));
        }
    }

    let mut edges = BTreeSet::new();
    for (index, edge) in skeleton.edges.iter().enumerate() {
        let edge_context = format!("{context} edge {}", index + 1);
        if !keypoint_names.contains(edge.from.as_str()) {
            issues.push(format!(
                "{edge_context}: from endpoint '{}' is not an existing keypoint.",
                edge.from
            ));
        }
        if !keypoint_names.contains(edge.to.as_str()) {
            issues.push(format!(
                "{edge_context}: to endpoint '{}' is not an existing keypoint.",
                edge.to
            ));
        }
        if edge.from == edge.to {
            issues.push(format!(
                "{edge_context}: from and to must be different keypoints."
            ));
        }
        if !edges.insert(canonical_edge(&edge.from, &edge.to)) {
            issues.push(format!(
                "{edge_context}: edge '{} - {}' is duplicated.",
                edge.from, edge.to
            ));
        }
    }
    issues
}

fn validate_review(issues: &mut Vec<String>, context: &str, task: &TaskDefinition) {
    match task.review.workflow {
        ReviewWorkflow::None => {}
        ReviewWorkflow::Approval if task.review.required_reviews == 0 => issues.push(format!(
            "{context}: approval workflow requires at least one review."
        )),
        ReviewWorkflow::IndependentAgreement if task.review.required_reviews < 2 => issues.push(
            format!("{context}: independent agreement requires at least two reviews."),
        ),
        _ => {}
    }
    if task.review.workflow == ReviewWorkflow::IndependentAgreement
        && task.review.agreement_threshold.is_none()
    {
        issues.push(format!(
            "{context}: enable an agreement threshold for independent agreement."
        ));
    }
    if task.review.workflow == ReviewWorkflow::IndependentAgreement
        && let Some(agreement) = &task.review.agreement_threshold
    {
        if !agreement.threshold.is_finite() || !(0.0..=1.0).contains(&agreement.threshold) {
            issues.push(format!(
                "{context}: agreement threshold must be between 0 and 1."
            ));
        }
        let metric_matches = matches!(
            (&task.annotation_type, &agreement.metric),
            (AnnotationType::BoundingBox, AgreementMetric::Iou)
                | (
                    AnnotationType::Skeleton,
                    AgreementMetric::KeypointMeanDistance
                )
        );
        if !metric_matches {
            issues.push(format!(
                "{context}: agreement metric must match the annotation type."
            ));
        }
    }
}

fn prelabel_issues(configs: &[PrelabelConfig]) -> Vec<String> {
    let mut issues = Vec::new();
    let mut ids = BTreeSet::new();
    for (index, config) in configs.iter().enumerate() {
        let context = format!("Prelabel {}", index + 1);
        validate_id(&mut issues, &context, config.config_id.as_str());
        if !ids.insert(config.config_id.as_str()) {
            issues.push(format!(
                "Prelabels: prelabel ID '{}' is duplicated; choose a unique ID.",
                config.config_id
            ));
        }
        if config.name.trim().is_empty() {
            issues.push(format!("{context}: enter a non-empty prelabel name."));
        }
        if config.model.model_id.trim().is_empty() {
            issues.push(format!("{context}: enter a non-empty model ID."));
        }
        if config.model.display_name.trim().is_empty() {
            issues.push(format!("{context}: enter a non-empty model name."));
        }
        if config.model.location.trim().is_empty() {
            issues.push(format!("{context}: enter a model location."));
        }
        let confidence = config.output_processing.confidence_threshold;
        if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
            issues.push(format!(
                "{context}: confidence threshold must be between 0 and 1."
            ));
        }
        if let Some(iou) = config.output_processing.suppress_overlaps_iou
            && (!iou.is_finite() || !(0.0..=1.0).contains(&iou))
        {
            issues.push(format!(
                "{context}: overlap suppression IoU must be between 0 and 1."
            ));
        }
    }
    issues
}

fn imbalance_issues(imbalance: Option<&ImbalanceConfig>) -> Vec<String> {
    let Some(imbalance) = imbalance else {
        return Vec::new();
    };
    if imbalance.max_ratio.is_finite() && imbalance.max_ratio >= 1.0 {
        Vec::new()
    } else {
        vec![
            "Assignment balance: maximum class ratio must be a finite value of at least 1."
                .to_string(),
        ]
    }
}

fn role_issues(
    assignments: &[DatasetRoleAssignment],
    dataset_id: &labello_domain::DatasetId,
    current_user: &UserId,
) -> Vec<String> {
    let mut issues = Vec::new();
    let mut users = BTreeSet::new();
    for (index, assignment) in assignments.iter().enumerate() {
        let context = format!("Role assignment {}", index + 1);
        validate_id(&mut issues, &context, assignment.user_id.as_str());
        if !users.insert(assignment.user_id.as_str()) {
            issues.push(format!(
                "Access roles: user '{}' has duplicate assignments; combine their roles into one row.",
                assignment.user_id
            ));
        }
        if assignment.dataset_id != *dataset_id {
            issues.push(format!(
                "{context}: assignment belongs to another dataset; remove and recreate it."
            ));
        }
        if assignment.roles.is_empty() {
            issues.push(format!("{context}: select at least one role."));
        }
    }
    let has_admin = assignments.iter().any(|assignment| {
        assignment.dataset_id == *dataset_id && assignment.roles.contains(&DatasetRole::DataAdmin)
    });
    if !has_admin {
        issues.push("Access roles: assign at least one data admin.".to_string());
    }
    let current_user_is_admin = assignments.iter().any(|assignment| {
        assignment.dataset_id == *dataset_id
            && assignment.user_id == *current_user
            && assignment.roles.contains(&DatasetRole::DataAdmin)
    });
    if !current_user_is_admin {
        issues.push(format!(
            "Access roles: keep data_admin enabled for the current user '{}'.",
            current_user
        ));
    }
    issues
}

fn validate_id(issues: &mut Vec<String>, context: &str, value: &str) {
    if value.is_empty() {
        issues.push(format!("{context}: enter a non-empty ID."));
    } else if !is_safe_id(value) {
        issues.push(format!(
            "{context}: ID '{value}' is unsafe; use one path-safe segment under 256 bytes with no '/', '\\', control characters, '.' or '..'."
        ));
    }
}

fn is_safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && !matches!(value, "." | "..")
        && !value
            .bytes()
            .any(|byte| byte.is_ascii_control() || matches!(byte, b'/' | b'\\'))
}

fn is_safe_relative_path(value: &str) -> bool {
    !value.trim().is_empty()
        && value == value.trim()
        && !value.starts_with('/')
        && !value.contains('\\')
        && !value.bytes().any(|byte| byte.is_ascii_control())
        && !value.split('/').any(|part| part.is_empty() || part == "..")
}

fn is_hex_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}

fn destructive_button(ui: &mut egui::Ui, label: &str, item: String) -> bool {
    let response = theme::danger_button(ui, true, egui::Button::new(label));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{label}: {item}"))
    });
    let modal_id = response.id.with("confirmation");
    if response.clicked() {
        ui.ctx().data_mut(|data| data.insert_temp(modal_id, true));
    }
    if !ui
        .ctx()
        .data(|data| data.get_temp::<bool>(modal_id).unwrap_or(false))
    {
        return false;
    }

    let mut confirmed = false;
    egui::Modal::new(modal_id).show(ui.ctx(), |ui| {
        ui.set_max_width((ui.ctx().content_rect().width() - 48.0).clamp(240.0, 480.0));
        ui.heading("Confirm removal");
        ui.label(format!(
            "Remove {item} from the staged dataset configuration? Related staged references will also be removed when required."
        ));
        ui.horizontal_wrapped(|ui| {
            if theme::danger_button(ui, true, egui::Button::new("Confirm removal")).clicked() {
                confirmed = true;
                ui.ctx().data_mut(|data| data.remove::<bool>(modal_id));
            }
            if theme::quiet_button(ui, true, egui::Button::new("Cancel")).clicked() {
                ui.ctx().data_mut(|data| data.remove::<bool>(modal_id));
            }
        });
    });
    confirmed
}

fn show_issues(ui: &mut egui::Ui, issues: &[String]) {
    for issue in issues {
        ui.label(RichText::new(format!("- {issue}")).color(theme::DANGER));
    }
}

fn stats_task_grid(
    ui: &mut egui::Ui,
    rows: &BTreeMap<TaskId, labello_domain::TaskStats>,
    task_names: &BTreeMap<TaskId, String>,
) {
    egui::Grid::new("stats-task-grid")
        .num_columns(9)
        .striped(true)
        .spacing([theme::SPACE_3, theme::SPACE_1])
        .show(ui, |ui| {
            stats_name_cell(ui, "Task", 180.0, true);
            for heading in [
                "Pending",
                "Unreviewed",
                "Reviewed",
                "Approved",
                "Rejected",
                "Corrected",
                "Finalized",
                "Done",
            ] {
                stats_number_cell(ui, heading, 84.0, true);
            }
            ui.end_row();

            for (task_id, stats) in rows {
                stats_name_cell(
                    ui,
                    task_names
                        .get(task_id)
                        .map(String::as_str)
                        .unwrap_or(task_id.as_str()),
                    180.0,
                    false,
                );
                for value in [
                    stats.pending,
                    stats.unreviewed,
                    stats.reviewed,
                    stats.approved,
                    stats.rejected,
                    stats.reviewer_corrected,
                    stats.finalized,
                    stats.completed,
                ] {
                    stats_number_cell(ui, value, 84.0, false);
                }
                ui.end_row();
            }
        });
}

fn stats_class_grid(
    ui: &mut egui::Ui,
    rows: &BTreeMap<ClassId, labello_domain::ClassStats>,
    class_names: &BTreeMap<ClassId, String>,
) {
    egui::Grid::new("stats-class-grid")
        .num_columns(3)
        .striped(true)
        .spacing([theme::SPACE_3, theme::SPACE_1])
        .show(ui, |ui| {
            stats_name_cell(ui, "Class", 220.0, true);
            stats_number_cell(ui, "Annotations", 130.0, true);
            stats_number_cell(ui, "Completed tasks", 140.0, true);
            ui.end_row();

            for (class_id, stats) in rows {
                stats_name_cell(
                    ui,
                    class_names
                        .get(class_id)
                        .map(String::as_str)
                        .unwrap_or(class_id.as_str()),
                    220.0,
                    false,
                );
                stats_number_cell(ui, stats.annotations, 130.0, false);
                stats_number_cell(ui, stats.completed_tasks, 140.0, false);
                ui.end_row();
            }
        });
}

fn stats_name_cell(ui: &mut egui::Ui, value: &str, width: f32, header: bool) {
    let text = if header {
        RichText::new(value).strong().color(theme::TEXT_MUTED)
    } else {
        RichText::new(value).strong().color(theme::TEXT)
    };
    ui.add_sized(
        [width, 44.0],
        egui::Label::new(text).truncate().halign(egui::Align::Min),
    );
}

fn stats_number_cell(ui: &mut egui::Ui, value: impl ToString, width: f32, header: bool) {
    let text = if header {
        RichText::new(value.to_string())
            .strong()
            .color(theme::TEXT_MUTED)
    } else {
        RichText::new(value.to_string())
            .monospace()
            .color(theme::TEXT)
    };
    ui.add_sized(
        [width, 44.0],
        egui::Label::new(text).truncate().halign(egui::Align::Max),
    );
}

fn stats_throughput_chart(ui: &mut egui::Ui, points: &[labello_domain::ThroughputPoint]) {
    let points = points.iter().rev().take(14).rev().collect::<Vec<_>>();
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Annotations").strong().color(theme::ACCENT));
        ui.label(RichText::new("Reviews").strong().color(theme::INFO));
        ui.label(
            RichText::new("Daily annotation and review activity")
                .size(theme::SUPPORTING_SIZE)
                .color(theme::TEXT_MUTED),
        );
    });
    let available_width = ui.available_width();
    egui::ScrollArea::horizontal()
        .id_salt("stats-throughput-chart-scroll")
        .show(ui, |ui| {
            let width = available_width.max(42.0 + points.len() as f32 * 48.0);
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(width, 184.0), egui::Sense::hover());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Daily throughput chart")
            });

            let maximum = points
                .iter()
                .flat_map(|point| [point.annotations, point.reviews])
                .max()
                .unwrap_or(0)
                .max(1);
            let axis_width = stats_axis_width(maximum);
            let plot = egui::Rect::from_min_max(
                egui::pos2(rect.left() + axis_width, rect.top() + 8.0),
                egui::pos2(rect.right() - 8.0, rect.bottom() - 26.0),
            );
            let painter = ui.painter_at(rect);
            let font = egui::FontId::new(theme::SUPPORTING_SIZE, egui::FontFamily::Monospace);
            let tick_fractions: &[f32] = if maximum == 1 {
                &[0.0, 1.0]
            } else {
                &[0.0, 0.5, 1.0]
            };
            for &fraction in tick_fractions {
                let y = plot.bottom() - plot.height() * fraction;
                painter.line_segment(
                    [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
                    egui::Stroke::new(1.0, theme::BORDER),
                );
                painter.text(
                    egui::pos2(plot.left() - 6.0, y),
                    egui::Align2::RIGHT_CENTER,
                    (maximum as f32 * fraction).round() as usize,
                    font.clone(),
                    theme::TEXT_MUTED,
                );
            }

            let group_width = plot.width() / points.len() as f32;
            let bar_width = (group_width * 0.26).clamp(4.0, 18.0);
            for (index, point) in points.iter().enumerate() {
                let left = plot.left() + index as f32 * group_width;
                let center = left + group_width * 0.5;
                for (value, x, color) in [
                    (point.annotations, center - bar_width - 1.0, theme::ACCENT),
                    (point.reviews, center + 1.0, theme::INFO),
                ] {
                    if value > 0 {
                        let height = plot.height() * value as f32 / maximum as f32;
                        painter.rect_filled(
                            egui::Rect::from_min_max(
                                egui::pos2(x, plot.bottom() - height),
                                egui::pos2(x + bar_width, plot.bottom()),
                            ),
                            egui::CornerRadius::same(2),
                            color,
                        );
                    }
                }
                painter.text(
                    egui::pos2(center, plot.bottom() + 6.0),
                    egui::Align2::CENTER_TOP,
                    point.day.get(5..).unwrap_or(&point.day),
                    font.clone(),
                    theme::TEXT_MUTED,
                );

                let detail = format!(
                    "{}: {} {}, {} {}",
                    point.day,
                    point.annotations,
                    if point.annotations == 1 {
                        "annotation"
                    } else {
                        "annotations"
                    },
                    point.reviews,
                    if point.reviews == 1 {
                        "review"
                    } else {
                        "reviews"
                    }
                );
                let hit = egui::Rect::from_min_max(
                    egui::pos2(left, plot.top()),
                    egui::pos2(left + group_width, rect.bottom()),
                );
                let response = ui
                    .interact(
                        hit,
                        ui.id().with(("throughput-point", index)),
                        egui::Sense::hover(),
                    )
                    .on_hover_text(detail.clone());
                response.widget_info(move || {
                    egui::WidgetInfo::labeled(egui::WidgetType::Label, true, detail.clone())
                });
            }
        });
}

fn stats_axis_width(maximum: usize) -> f32 {
    (maximum.to_string().len() as f32 * 8.0 + 12.0).max(34.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_skeleton_is_valid() {
        let skeleton = starter_skeleton_spec();

        assert!(skeleton_issues(&skeleton, "Skeleton").is_empty());
        assert_eq!(skeleton.keypoints.len(), 1);
        assert!(skeleton.keypoints[0].required);
    }

    #[test]
    fn statistics_axis_gutter_scales_with_large_values() {
        assert_eq!(stats_axis_width(1), 34.0);
        assert!(stats_axis_width(12_345) >= 52.0);
    }

    #[test]
    fn skeleton_validation_rejects_invalid_keypoints_and_edges() {
        let skeleton = SkeletonSpec {
            keypoints: vec![
                KeypointSpec {
                    name: "joint".to_string(),
                    required: true,
                },
                KeypointSpec {
                    name: "joint".to_string(),
                    required: false,
                },
                KeypointSpec {
                    name: " ".to_string(),
                    required: false,
                },
            ],
            edges: vec![
                SkeletonEdge {
                    from: "joint".to_string(),
                    to: "joint".to_string(),
                },
                SkeletonEdge {
                    from: "missing".to_string(),
                    to: "joint".to_string(),
                },
                SkeletonEdge {
                    from: "joint".to_string(),
                    to: "missing".to_string(),
                },
            ],
            allow_hidden: true,
            allow_absent: true,
        };

        let issues = skeleton_issues(&skeleton, "Skeleton").join("\n");
        assert!(issues.contains("non-empty name"));
        assert!(issues.contains("duplicated; choose a unique name"));
        assert!(issues.contains("from and to must be different"));
        assert!(issues.contains("from endpoint 'missing'"));
        assert!(issues.contains("to endpoint 'missing'"));
    }

    #[test]
    fn skeleton_validation_requires_a_keypoint() {
        let mut skeleton = starter_skeleton_spec();
        skeleton.keypoints.clear();

        assert!(
            skeleton_issues(&skeleton, "Skeleton")
                .iter()
                .any(|issue| issue.contains("add at least one keypoint"))
        );
    }

    #[test]
    fn skeleton_validation_treats_reversed_edges_as_duplicates() {
        let skeleton = SkeletonSpec {
            keypoints: vec![
                KeypointSpec {
                    name: "left".to_string(),
                    required: true,
                },
                KeypointSpec {
                    name: "right".to_string(),
                    required: true,
                },
            ],
            edges: vec![
                SkeletonEdge {
                    from: "left".to_string(),
                    to: "right".to_string(),
                },
                SkeletonEdge {
                    from: "right".to_string(),
                    to: "left".to_string(),
                },
            ],
            allow_hidden: false,
            allow_absent: false,
        };

        assert!(
            skeleton_issues(&skeleton, "Skeleton")
                .iter()
                .any(|issue| issue.contains("is duplicated"))
        );
    }

    #[test]
    fn switching_annotation_type_initializes_and_clears_skeleton() {
        let class = LabelClass {
            class_id: ClassId::from("person"),
            name: "Person".to_string(),
            color: "#5eead4".to_string(),
            description: None,
        };
        let mut task = workflow_task_for_class(&class, AnnotationType::BoundingBox);

        set_task_annotation_type(&mut task, AnnotationType::Skeleton);
        assert!(task.skeleton.is_some());
        assert!(skeleton_issues(task.skeleton.as_ref().unwrap(), "Skeleton").is_empty());

        set_task_annotation_type(&mut task, AnnotationType::BoundingBox);
        assert!(task.skeleton.is_none());
    }

    #[test]
    fn disabled_quick_workflow_is_not_duplicated() {
        let class = LabelClass {
            class_id: ClassId::from("person"),
            name: "Person".to_string(),
            color: "#5eead4".to_string(),
            description: None,
        };
        let mut config = DatasetMetadata::new(
            labello_domain::DatasetId::from("demo"),
            "Demo",
            labello_domain::now(),
        );
        config.label_classes.push(class.clone());
        let mut task = workflow_task_for_class(&class, AnnotationType::BoundingBox);
        task.enabled = false;
        config.tasks.push(task);

        assert!(has_task_for_class(
            &config,
            &class.class_id,
            &AnnotationType::BoundingBox
        ));
        add_task_for_class(&mut config, &class, AnnotationType::BoundingBox);
        assert_eq!(config.tasks.len(), 1);
    }

    #[test]
    fn enabled_workflows_require_exactly_one_class() {
        let person = LabelClass {
            class_id: ClassId::from("person"),
            name: "Person".to_string(),
            color: "#5eead4".to_string(),
            description: None,
        };
        let vehicle = LabelClass {
            class_id: ClassId::from("vehicle"),
            name: "Vehicle".to_string(),
            color: "#60a5fa".to_string(),
            description: None,
        };
        let mut task = workflow_task_for_class(&person, AnnotationType::BoundingBox);
        task.class_ids.push(vehicle.class_id.clone());

        assert!(
            task_issues(&[task], &[person, vehicle], &[])
                .iter()
                .any(|issue| issue.contains("exactly one class"))
        );
    }

    #[test]
    fn task_status_summary_groups_statuses_in_workflow_order() {
        assert_eq!(task_status_summary(&[]), "No workflow status");
        assert_eq!(
            task_status_summary(&[
                TaskStatus::Completed,
                TaskStatus::Pending,
                TaskStatus::Pending,
            ]),
            "Pending 2 | Completed 1"
        );
    }
}
