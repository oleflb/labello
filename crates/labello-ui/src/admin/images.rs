impl LabelloApp {
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
                    ui.set_min_width(ui.available_width());
                    ui.vertical(|ui| {
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

    fn images_section(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
        let controls_enabled = !self.loading.images
            && !self.loading.admin
            && self.loading.roles_user.is_none()
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
                    [ui.available_width(), theme::COMPACT_TEXT_FIELD_HEIGHT],
                    theme::singleline_text_edit(&mut self.admin_tools.image_search)
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
