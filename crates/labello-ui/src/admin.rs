use std::collections::{BTreeMap, BTreeSet};

use eframe::egui::{self, RichText};
use labello_client::DatasetUser;
use labello_domain::{
    AgreementMetric, AgreementThreshold, AnnotationType, BrowserAcceleration, ClassId,
    DatasetMetadata, DatasetRole, DatasetRoleAssignment, ImbalanceConfig, KeypointSpec, LabelClass,
    ModelSpec, OutputProcessing, PrelabelConfig, PrelabelConfigId, PrelabelExecution, ReviewConfig,
    ReviewWorkflow, SkeletonEdge, SkeletonSpec, TaskDefinition, TaskId, TaskStatus,
    TutorialContent, UserId,
};

use crate::{
    app::{LabelloApp, LayoutMode},
    theme,
};

impl LabelloApp {
    pub(crate) fn admin_view(&mut self, ui: &mut egui::Ui) {
        let config_dirty = self.datasets.admin_config != self.datasets.admin_baseline;
        let permissions_dirty = self.datasets.users != self.datasets.users_baseline;
        let load_error = self.admin_tools.load_error.clone();
        let user_count = self.datasets.users.len();
        let image_count = self
            .admin_tools
            .images
            .as_ref()
            .map(|page| page.total_items.to_string())
            .unwrap_or_else(|| "-".to_string());
        let workflow_count = self
            .datasets
            .admin_config
            .as_ref()
            .map(|config| config.tasks.iter().filter(|task| task.enabled).count())
            .unwrap_or_default();
        theme::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.vertical(|ui| {
                    ui.heading("Dataset Admin");
                    ui.label(
                        RichText::new(
                            "Manage access, inspect images, and configure labeling workflows.",
                        )
                        .color(theme::MUTED),
                    );
                });
                if self.loading.admin {
                    ui.spinner();
                }
                ui.label(
                    RichText::new(
                        if self.loading.admin && self.datasets.admin_config.is_none() {
                            "Loading admin config"
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
                        },
                    )
                    .color(
                        if self.loading.admin
                            || load_error.is_some()
                            || config_dirty
                            || permissions_dirty
                        {
                            theme::AMBER
                        } else {
                            theme::TEAL
                        },
                    )
                    .strong(),
                );
                if self.datasets.admin_config.is_some()
                    && theme::quiet_button(
                        ui,
                        !config_dirty && !permissions_dirty,
                        egui::Button::new("Reload"),
                    )
                    .on_hover_text(if config_dirty || permissions_dirty {
                        "Discard or save staged changes before reloading."
                    } else {
                        "Reload configuration from the server."
                    })
                    .clicked()
                {
                    self.request_admin_dataset();
                }
            });
            if self.datasets.admin_config.is_some() {
                ui.add_space(8.0);
                ui.columns(3, |columns| {
                    theme::metric(&mut columns[0], "Users", user_count.to_string());
                    theme::metric(&mut columns[1], "Indexed images", image_count);
                    theme::metric(
                        &mut columns[2],
                        "Active workflows",
                        workflow_count.to_string(),
                    );
                });
            }
        });
        ui.add_space(8.0);
        if let Some(error) = load_error {
            ui.horizontal_wrapped(|ui| {
                theme::inline_message(
                    ui,
                    theme::Intent::Error,
                    format!("Admin load failed: {error}"),
                );
                if theme::quiet_button(
                    ui,
                    !self.loading.admin,
                    egui::Button::new("Retry admin load"),
                )
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
        self.people_section(ui);
        self.images_section(ui);
        self.snapshots_section(ui);
        let current_user = self.config.user_id.clone();
        let admin_loading = self.loading.admin;
        let roles_saving = self.loading.roles_user.is_some();
        let ingesting_now = self.loading.ingesting;
        let uploading_now = self.loading.uploading;
        let upload_progress = self.loading.upload_progress.clone();
        let Some(config) = self.datasets.admin_config.as_mut() else {
            return;
        };

        let mut ingest = false;
        let mut upload_folder = false;
        ui.add_space(8.0);
        ui.heading("Dataset configuration");
        ui.add_enabled_ui(
            !admin_loading && !roles_saving && !uploading_now && !ingesting_now,
            |ui| {
            ui.label(
                RichText::new(
                    "Edits are staged here. Review the validation summary before saving them.",
                )
                .color(theme::MUTED),
            );
            theme::card_frame().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.heading("Dataset Details");
                theme::labeled_text_field(ui, "Dataset name", &mut config.name, 44.0)
                    .on_hover_text("Human-readable name stored in labello.dataset.toml.");
                show_issues(ui, &dataset_name_issues(&config.name));
            });

            theme::card_frame().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.heading("Image Roots");
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
                    if uploading_now {
                        ui.spinner();
                    }
                });
                if let Some(progress) = upload_progress.as_ref() {
                    ui.add(
                        egui::ProgressBar::new(progress.fraction())
                            .desired_width(ui.available_width().min(460.0))
                            .text(progress.label()),
                    );
                    if progress.current_batch > 0 {
                        ui.small(format!("Batch {}", progress.current_batch));
                    }
                }
                if config_dirty {
                    ui.label(
                        RichText::new("Save or discard root changes before uploading or ingesting.")
                            .color(theme::AMBER),
                    );
                }
                ui.small("Paths are relative to the dataset root and may be edited in labello.dataset.toml.");
                show_issues(ui, &image_root_issues(&config.image_roots));
            });

            edit_quick_workflows(ui, config);
            edit_labels(ui, &mut config.label_classes, &mut config.tasks);
            edit_tasks(
                ui,
                &mut config.tasks,
                &config.label_classes,
                &config.prelabel_configs,
            );
            edit_prelabels(ui, &mut config.prelabel_configs, &mut config.tasks);
            edit_imbalance(ui, &mut config.imbalance);
            let issues = config_issues(config, &current_user);
            theme::card_frame().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.heading("Validation Summary");
                if issues.is_empty() {
                    theme::inline_message(
                        ui,
                        theme::Intent::Success,
                        "Configuration is ready to save.",
                    );
                } else {
                    ui.label(
                        RichText::new(format!(
                            "Fix {} configuration error(s) before saving:",
                            issues.len()
                        ))
                        .color(theme::RED)
                        .strong(),
                    );
                    show_issues(ui, &issues);
                }
            });

            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                if theme::primary_button(
                    ui,
                        !config_dirty && !permissions_dirty,
                        egui::Button::new("Run Ingest"),
                    )
                    .on_hover_text("Scan configured image roots and update the dataset image index.")
                    .clicked()
                {
                    ingest = true;
                }
                if ingesting_now {
                    ui.spinner();
                    ui.small("Ingest running...");
                }
            });
            },
        );
        if ingest {
            self.request_ingest();
        }
        if upload_folder {
            self.request_folder_upload();
        }
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

    fn people_section(&mut self, ui: &mut egui::Ui) {
        theme::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.heading("People");
                ui.label(
                    RichText::new(format!("{} users", self.datasets.users.len()))
                        .color(theme::BLUE),
                );
            });
            ui.label(
                RichText::new("Grant dataset roles to people who have signed in to this server.")
                    .color(theme::MUTED),
            );
            if self.loading.admin && self.datasets.users.is_empty() {
                ui.spinner();
                return;
            }
            let current_user = self.config.user_id.clone();
            let admin_loading = self.loading.admin;
            let baseline = self.datasets.users_baseline.clone();
            let admin_count = self
                .datasets
                .users
                .iter()
                .filter(|user| user.roles.contains(&DatasetRole::DataAdmin))
                .count();
            let saving = self.loading.roles_user.clone();
            let mut save_user = None;
            for user in &mut self.datasets.users {
                ui.add_space(4.0);
                theme::inset_frame().show(ui, |ui| {
                    let compact = ui.available_width() < 760.0;
                    let save_clicked = if compact {
                        ui.vertical(|ui| {
                            user_identity(ui, user);
                            ui.add_space(4.0);
                            ui.horizontal_wrapped(|ui| {
                                user_role_controls(
                                    ui,
                                    user,
                                    &baseline,
                                    &current_user,
                                    admin_count,
                                    admin_loading,
                                    saving.as_ref(),
                                )
                            })
                            .inner
                        })
                        .inner
                    } else {
                        ui.horizontal_wrapped(|ui| {
                            ui.vertical(|ui| {
                                ui.set_width(210.0);
                                user_identity(ui, user);
                            });
                            ui.horizontal_wrapped(|ui| {
                                user_role_controls(
                                    ui,
                                    user,
                                    &baseline,
                                    &current_user,
                                    admin_count,
                                    admin_loading,
                                    saving.as_ref(),
                                )
                            })
                            .inner
                        })
                        .inner
                    };
                    if save_clicked {
                        save_user = Some(user.account.user_id.clone());
                    }
                });
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

    fn images_section(&mut self, ui: &mut egui::Ui) {
        theme::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            egui::CollapsingHeader::new("Images")
                .default_open(false)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("Search the indexed images and inspect workflow state.")
                            .color(theme::MUTED),
                    );
                    let compact_filters = ui.available_width() < 600.0;
                    let search_label = ui.label("Search images");
                    let search = ui
                        .add(
                            egui::TextEdit::singleline(&mut self.admin_tools.image_search)
                                .hint_text("File name or path")
                                .desired_width(ui.available_width()),
                        )
                        .labelled_by(search_label.id);
                    if search.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))
                    {
                        self.admin_tools.image_query.page = 1;
                        self.request_images();
                    }
                    let control_width = if compact_filters {
                        ui.available_width()
                    } else {
                        140.0
                    };
                    let mut show_filters = |ui: &mut egui::Ui| {
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
                            });

                        let tasks = self
                            .datasets
                            .admin_config
                            .as_ref()
                            .map(|config| config.tasks.clone())
                            .unwrap_or_default();
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
                                ui.selectable_value(
                                    &mut self.admin_tools.image_task,
                                    None,
                                    "Any task",
                                );
                                for task in &tasks {
                                    ui.selectable_value(
                                        &mut self.admin_tools.image_task,
                                        Some(task.task_id.clone()),
                                        &task.name,
                                    );
                                }
                            });

                        let classes = self
                            .datasets
                            .admin_config
                            .as_ref()
                            .map(|config| config.label_classes.clone())
                            .unwrap_or_default();
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
                            });
                        if theme::primary_button(
                            ui,
                            !self.loading.images,
                            egui::Button::new("Apply filters"),
                        )
                        .clicked()
                        {
                            self.admin_tools.image_query.page = 1;
                            self.request_images();
                        }
                        if theme::quiet_button(
                            ui,
                            !self.loading.images,
                            egui::Button::new("Refresh images"),
                        )
                        .clicked()
                        {
                            self.request_images();
                        }
                        if self.loading.images {
                            ui.spinner();
                        }
                    };
                    if compact_filters {
                        ui.vertical(&mut show_filters);
                    } else {
                        ui.horizontal_wrapped(show_filters);
                    }
                    if let Some(error) = &self.admin_tools.images_error {
                        theme::inline_message(ui, theme::Intent::Error, error);
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
                                    previous && !self.loading.images,
                                    egui::Button::new("Previous images"),
                                )
                                .clicked()
                            {
                                self.admin_tools.image_query.page = current_page.saturating_sub(1);
                                self.request_images();
                            }
                            if ui
                                .add_enabled(
                                    next && !self.loading.images,
                                    egui::Button::new("Next images"),
                                )
                                .clicked()
                            {
                                self.admin_tools.image_query.page = current_page + 1;
                                self.request_images();
                            }
                        });
                        if page.items.is_empty() {
                            theme::empty_state(
                                ui,
                                "No matching images",
                                "Change the filters or ingest more images.",
                                None,
                            );
                        }
                        for item in &page.items {
                            ui.add_space(4.0);
                            let status_details = item
                                .task_statuses
                                .iter()
                                .map(|(task, status)| {
                                    format!("{task}: {}", task_status_label(status))
                                })
                                .collect::<Vec<_>>()
                                .join("\n");
                            let statuses = item.task_statuses.values().cloned().collect::<Vec<_>>();
                            let classes = item
                                .class_ids
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join(", ");
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
                                        .color(theme::BLUE),
                                    );
                                });
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&item.image.canonical_path)
                                            .color(theme::MUTED),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(&item.image.canonical_path);
                                let class_summary = if classes.is_empty() {
                                    "No classes".to_string()
                                } else {
                                    format!("Classes: {classes}")
                                };
                                ui.label(
                                    RichText::new(format!(
                                        "{} | {class_summary}",
                                        task_status_summary(&statuses)
                                    ))
                                    .color(theme::MUTED),
                                )
                                .on_hover_text(
                                    if status_details.is_empty() {
                                        "No workflow status".to_string()
                                    } else {
                                        status_details
                                    },
                                );
                            });
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
        });
    }

    fn snapshots_section(&mut self, ui: &mut egui::Ui) {
        theme::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            egui::CollapsingHeader::new("Backups / Snapshots")
                .default_open(false)
                .show(ui, |ui| {
                ui.label(
                    RichText::new("Create and download native dataset snapshots. Image bytes are not included.")
                        .color(theme::MUTED),
                );
                ui.horizontal_wrapped(|ui| {
                    if theme::primary_button(
                        ui,
                            !self.loading.creating_snapshot,
                            egui::Button::new("Create snapshot"),
                        )
                        .clicked()
                    {
                        self.request_snapshot_create();
                    }
                    if theme::quiet_button(
                        ui,
                            !self.loading.snapshots,
                            egui::Button::new("Refresh snapshots"),
                        )
                        .clicked()
                    {
                        self.request_snapshots();
                    }
                    if self.loading.creating_snapshot || self.loading.snapshots {
                        ui.spinner();
                    }
                });
                if let Some(error) = &self.admin_tools.snapshots_error {
                    theme::inline_message(ui, theme::Intent::Error, error);
                }
                if self.admin_tools.snapshots.is_empty()
                    && self.admin_tools.snapshots_loaded
                    && !self.loading.snapshots
                {
                    theme::empty_state(
                        ui,
                        "No snapshots yet",
                        "Create a snapshot to preserve the current dataset state.",
                        None,
                    );
                }
                let snapshots = self.admin_tools.snapshots.clone();
                for snapshot in snapshots {
                    ui.separator();
                    ui.label(RichText::new(&snapshot.snapshot_id).strong());
                    ui.small(format!(
                        "{} | {} files | {} total",
                        snapshot.created_at.format("%Y-%m-%d %H:%M UTC"),
                        snapshot.files.len(),
                        human_bytes(snapshot.total_bytes)
                    ));
                    for file in snapshot.files {
                        let downloading = self.loading.snapshot_file.as_ref()
                            == Some(&(snapshot.snapshot_id.clone(), file.path.clone()));
                        ui.horizontal_wrapped(|ui| {
                            ui.small(format!("{} ({})", file.path, human_bytes(file.byte_size)));
                            if ui
                                .add_enabled(
                                    self.loading.snapshot_file.is_none(),
                                    egui::Button::new(if downloading { "Downloading..." } else { "Download" }),
                                )
                                .clicked()
                            {
                                self.request_snapshot_download(
                                    snapshot.snapshot_id.clone(),
                                    file.path.clone(),
                                );
                            }
                        });
                    }
                }
                });
        });
    }

    pub(crate) fn stats_view(&mut self, ui: &mut egui::Ui) {
        let initial_loading = self.loading.stats && self.datasets.last_stats_completion.is_none();
        ui.horizontal_wrapped(|ui| {
            ui.heading("Live Statistics");
            if self.loading.stats {
                ui.spinner();
            }
            if theme::quiet_button(ui, !self.loading.stats, egui::Button::new("Refresh now"))
                .on_hover_text("Refresh statistics immediately. They also refresh automatically.")
                .clicked()
            {
                self.request_stats();
            }
        });
        if let Some(error) = &self.datasets.stats_error {
            theme::inline_message(
                ui,
                theme::Intent::Error,
                format!("Statistics refresh failed: {error}"),
            );
        }
        if initial_loading {
            theme::card_frame().show(ui, |ui| {
                ui.label("Loading statistics...");
            });
            return;
        }
        if self.datasets.last_stats_completion.is_none() {
            if self.datasets.stats_error.is_none() {
                theme::card_frame().show(ui, |ui| {
                    ui.label("Statistics have not loaded.");
                });
            }
            return;
        }
        if let Some(completed) = self.datasets.last_stats_completion {
            ui.small(format!(
                "Updated {} second(s) ago",
                completed.elapsed().as_secs()
            ));
        }
        let compact = ui.available_width() < 600.0;
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
                "Reviewer corrected",
                self.datasets.stats.reviewer_corrected_tasks,
            ),
            ("Finalized", self.datasets.stats.finalized_tasks),
        ];
        let minimum_card_width = 180.0;
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
                    theme::card_frame().show(ui, |ui| {
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
                            "Completed: {}  Pending: {}  Finalized: {}",
                            stats.completed, stats.pending, stats.finalized
                        ));
                        ui.label(format!(
                            "Reviewed: {}  Unreviewed: {}  Approved: {}",
                            stats.reviewed, stats.unreviewed, stats.approved
                        ));
                        ui.label(format!(
                            "Rejected: {}  Reviewer corrected: {}",
                            stats.rejected, stats.reviewer_corrected
                        ));
                    });
                }
            } else {
                egui::ScrollArea::horizontal()
                    .id_salt("stats_tasks_horizontal")
                    .show(ui, |ui| {
                        ui.set_min_width(980.0);
                        stats_task_row(
                            ui,
                            "Task",
                            "Done",
                            "Pending",
                            "Reviewed",
                            "Unreviewed",
                            "Approved",
                            "Rejected",
                            "Corrected",
                            "Finalized",
                            true,
                        );
                        for (task_id, stats) in rows {
                            stats_task_row(
                                ui,
                                task_names
                                    .get(task_id)
                                    .map(String::as_str)
                                    .unwrap_or(task_id.as_str()),
                                &stats.completed.to_string(),
                                &stats.pending.to_string(),
                                &stats.reviewed.to_string(),
                                &stats.unreviewed.to_string(),
                                &stats.approved.to_string(),
                                &stats.rejected.to_string(),
                                &stats.reviewer_corrected.to_string(),
                                &stats.finalized.to_string(),
                                false,
                            );
                        }
                    });
            }
        });
        theme::card_frame().show(ui, |ui| {
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
                    theme::card_frame().show(ui, |ui| {
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
                        ui.set_min_width(520.0);
                        stats_class_row(ui, "Class", "Annotations", "Completed tasks", true);
                        for (class_id, stats) in rows {
                            stats_class_row(
                                ui,
                                class_names
                                    .get(class_id)
                                    .map(String::as_str)
                                    .unwrap_or(class_id.as_str()),
                                &stats.annotations.to_string(),
                                &stats.completed_tasks.to_string(),
                                false,
                            );
                        }
                    });
            }
        });
        theme::card_frame().show(ui, |ui| {
            ui.heading("Throughput");
            if self.datasets.stats.throughput.is_empty() {
                theme::empty_state(
                    ui,
                    "No completed activity",
                    "Throughput appears after annotations or reviews are completed.",
                    None,
                );
            }
            for point in self.datasets.stats.throughput.iter().rev().take(14).rev() {
                ui.label(format!(
                    "{}: {} annotations, {} reviews",
                    point.day, point.annotations, point.reviews
                ));
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

fn user_role_controls(
    ui: &mut egui::Ui,
    user: &mut DatasetUser,
    baseline: &[DatasetUser],
    current_user: &UserId,
    admin_count: usize,
    admin_loading: bool,
    saving: Option<&UserId>,
) -> bool {
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
        if ui
            .add_enabled(role_enabled, egui::Checkbox::new(&mut enabled, label))
            .on_disabled_hover_text(if &user.account.user_id == current_user {
                "You cannot remove your own data admin role."
            } else {
                "At least one data admin must remain."
            })
            .changed()
        {
            if enabled {
                user.roles.push(role);
                user.roles.sort();
                user.roles.dedup();
            } else {
                user.roles.retain(|existing| existing != &role);
            }
        }
    }
    let dirty = baseline
        .iter()
        .find(|existing| existing.account.user_id == user.account.user_id)
        .is_none_or(|existing| existing.roles != user.roles);
    let this_saving = saving == Some(&user.account.user_id);
    theme::primary_button(
        ui,
        dirty && saving.is_none() && !admin_loading,
        egui::Button::new(if this_saving {
            "Saving..."
        } else {
            "Save permissions"
        }),
    )
    .clicked()
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
            ui.label(label);
            ui.text_edit_singleline(value)
                .on_hover_text("Dataset-relative path under the dataset root.");
            if destructive_button(ui, "Remove") {
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
        let wide = ui.available_width() >= 820.0;
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
        remove = destructive_button(ui, "Remove class");
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
    destructive_button(ui, "Remove workflow")
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
                    if destructive_button(ui, "Remove keypoint") {
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
                    if destructive_button(ui, "Remove edge") {
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
                if destructive_button(ui, "Remove prelabel") {
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

fn destructive_button(ui: &mut egui::Ui, label: &str) -> bool {
    theme::danger_button(ui, true, egui::Button::new(format!("Double-click {label}")))
        .on_hover_text("Double-click to confirm this removal.")
        .double_clicked()
}

fn show_issues(ui: &mut egui::Ui, issues: &[String]) {
    for issue in issues {
        ui.label(RichText::new(format!("- {issue}")).color(theme::DANGER));
    }
}

#[allow(clippy::too_many_arguments)]
fn stats_task_row(
    ui: &mut egui::Ui,
    task: &str,
    completed: &str,
    pending: &str,
    reviewed: &str,
    unreviewed: &str,
    approved: &str,
    rejected: &str,
    corrected: &str,
    finalized: &str,
    header: bool,
) {
    ui.horizontal(|ui| {
        stats_cell(ui, task, 180.0, header);
        for value in [
            completed, pending, reviewed, unreviewed, approved, rejected, corrected, finalized,
        ] {
            stats_cell(ui, value, 84.0, header);
        }
    });
}

fn stats_class_row(
    ui: &mut egui::Ui,
    class: &str,
    annotations: &str,
    completed: &str,
    header: bool,
) {
    ui.horizontal(|ui| {
        stats_cell(ui, class, 220.0, header);
        stats_cell(ui, annotations, 130.0, header);
        stats_cell(ui, completed, 140.0, header);
    });
}

fn stats_cell(ui: &mut egui::Ui, value: &str, width: f32, header: bool) {
    let text = if header {
        RichText::new(value).strong().color(theme::MUTED)
    } else {
        RichText::new(value).color(theme::TEXT)
    };
    ui.add_sized([width, 32.0], egui::Label::new(text).truncate());
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
