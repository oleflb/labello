impl LabelloApp {
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
                        if self.admin.snapshots_error.is_some()
                            && !self.admin.snapshots_loaded
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
                        if self.admin.snapshots_loaded
                            || !self.admin.snapshots.is_empty()
                        {
                            "Refreshing backups..."
                        } else {
                            "Loading backups..."
                        },
                    );
                }
            });

            if let Some(error) = &self.admin.snapshots_error {
                theme::inline_message(
                    ui,
                    if self.admin.snapshots_loaded || !self.admin.snapshots.is_empty() {
                        theme::Intent::Warning
                    } else {
                        theme::Intent::Error
                    },
                    if self.admin.snapshots_loaded {
                        format!("Showing the last loaded backups. Refresh failed: {error}")
                    } else if !self.admin.snapshots.is_empty() {
                        format!("Showing newly created backups. Catalog refresh failed: {error}")
                    } else {
                        format!("Could not load backups: {error}")
                    },
                );
            }
            if let Some(error) = &self.admin.snapshot_action_error {
                theme::inline_message(
                    ui,
                    theme::Intent::Error,
                    format!("Backup action failed: {error}"),
                );
            }

            if !self.admin.snapshots_loaded
                && !self.loading.snapshots
                && self.admin.snapshots_error.is_none()
            {
                theme::empty_state(
                    ui,
                    "Backups are not loaded",
                    "Refresh to load the available dataset snapshots.",
                    None,
                );
            } else if self.admin.snapshots.is_empty()
                && self.admin.snapshots_loaded
                && !self.loading.snapshots
                && self.admin.snapshots_error.is_none()
            {
                theme::empty_state(
                    ui,
                    "No snapshots yet",
                    "Create a snapshot to preserve the current dataset state.",
                    None,
                );
            }

            let snapshots = self.admin.snapshots.clone();
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
