use eframe::egui::{self, Color32, RichText};
use labello_domain::AnnotationType;

use crate::{
    app::{LabelloApp, SaveStatus, Tool},
    canvas::{CanvasAction, show_canvas},
    theme,
};

impl LabelloApp {
    pub(crate) fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            ui.label(
                RichText::new("Labello")
                    .size(22.0)
                    .strong()
                    .color(theme::TEXT),
            );
            ui.add_space(8.0);
            badge(
                ui,
                &format!("Dataset {}", self.config.dataset_id),
                theme::BLUE,
            );
            badge(
                ui,
                status_text(self.save_status),
                status_color(self.save_status),
            );
            if self.offline {
                badge(ui, "Offline bundle", theme::AMBER);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Next image").clicked() {
                    self.next_image();
                }
                if ui.button("Save").clicked() {
                    self.autosave();
                }
                ui.toggle_value(&mut self.show_tutorial, "Tutorial");
                ui.toggle_value(&mut self.offline, "Offline");
            });
        });
    }

    pub(crate) fn task_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading(RichText::new("Task Focus").color(theme::TEXT));
        ui.label(
            RichText::new("Only the active task's classes and tools are shown.")
                .color(theme::MUTED),
        );
        ui.add_space(8.0);
        for (index, task) in self.tasks.iter().enumerate() {
            let selected = self.selected_task == index;
            theme::card_frame().show(ui, |ui| {
                if ui
                    .selectable_label(selected, RichText::new(&task.name).strong())
                    .clicked()
                {
                    self.selected_task = index;
                    self.tool = match task.annotation_type {
                        AnnotationType::BoundingBox => Tool::BoundingBox,
                        AnnotationType::Skeleton => Tool::Keypoints,
                    };
                }
                ui.small(format!("{} classes", task.class_ids.len()));
            });
        }
        ui.add_space(10.0);
        ui.label(RichText::new("Tools").strong());
        ui.selectable_value(&mut self.tool, Tool::BoundingBox, "Bounding box");
        ui.selectable_value(&mut self.tool, Tool::Keypoints, "Keypoints");
        ui.separator();
        ui.label(RichText::new("Classes").strong());
        for class in &self.classes {
            badge(ui, &class.name, theme::TEAL);
        }
        ui.separator();
        ui.label(RichText::new("Image Queue").strong());
        let mut queue_size = self.queue.queue_size();
        if ui
            .add(egui::Slider::new(&mut queue_size, 1..=12).text("preload"))
            .changed()
        {
            self.queue.set_queue_size(queue_size);
            self.replenish_demo_queue();
        }
        if self.queue.is_loading() || self.current.is_none() {
            ui.colored_label(theme::AMBER, "Loading next image...");
        } else {
            ui.colored_label(theme::MUTED, format!("{} images ready", self.queue.len()));
        }
    }

    pub(crate) fn right_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading(RichText::new("Review & Sync").color(theme::TEXT));
        let active_count = self
            .annotations
            .iter()
            .filter(|annotation| !annotation.deleted)
            .count();
        metric(ui, "Active annotations", active_count.to_string());
        metric(ui, "Review cursor", (self.review_index + 1).to_string());
        ui.horizontal(|ui| {
            if ui.button("Approve y").clicked() {
                self.review_index = self.review_index.saturating_add(1);
            }
            if ui.button("Reject n").clicked() {
                self.save_status = SaveStatus::Dirty;
            }
        });
        ui.separator();
        ui.heading("Prelabels");
        let prelabels = self
            .current
            .as_ref()
            .map(|image| image.prelabels.clone())
            .unwrap_or_default();
        if prelabels.is_empty() {
            ui.label(RichText::new("No suggestions for this image.").color(theme::MUTED));
        }
        for suggestion in &prelabels {
            theme::card_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("{}", suggestion.class_id));
                    badge(
                        ui,
                        &format!("{:.0}%", suggestion.confidence * 100.0),
                        theme::TEAL,
                    );
                    if ui.button("Accept").clicked() {
                        self.accept_prelabel(suggestion);
                    }
                    if ui.button("Discard").clicked() {
                        self.accepted_prelabels
                            .push(suggestion.suggestion_id.clone());
                    }
                });
            });
        }
        ui.separator();
        ui.heading("Keybindings");
        egui::ScrollArea::vertical()
            .max_height(180.0)
            .show(ui, |ui| {
                for (action, chord) in &self.keybindings.bindings {
                    ui.label(RichText::new(format!("{action:?}: {chord}")).color(theme::MUTED));
                }
            });
    }

    pub(crate) fn central(&mut self, ui: &mut egui::Ui) {
        if self.show_tutorial
            && let Some(task) = self.selected_task()
        {
            theme::card_frame().show(ui, |ui| {
                ui.heading(&task.instructions.title);
                ui.label(&task.instructions.example_text);
                for image in &task.instructions.example_images {
                    ui.small(format!("Example image: {image}"));
                }
            });
            ui.add_space(12.0);
        }
        if let Some(current) = &self.current {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(&current.image.file_name)
                        .strong()
                        .color(theme::TEXT),
                );
                ui.label(
                    RichText::new(format!(
                        "{} x {}",
                        current.image.width, current.image.height
                    ))
                    .color(theme::MUTED),
                );
            });
            ui.add_space(8.0);
            let action = show_canvas(
                ui,
                &mut self.canvas,
                &self.annotations,
                [current.image.width, current.image.height],
                self.tool == Tool::BoundingBox,
            );
            match action {
                Some(CanvasAction::CreateBoundingBox(bbox)) => self.create_bbox(bbox),
                Some(CanvasAction::Select(id)) => self.selected_annotation = Some(id),
                None => {}
            }
        } else {
            ui.vertical_centered(|ui| {
                ui.spinner();
                ui.label("Waiting for the image queue...");
            });
        }
    }
}

fn badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(
            color.r(),
            color.g(),
            color.b(),
            36,
        ))
        .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.55)))
        .corner_radius(egui::CornerRadius::same(18))
        .inner_margin(egui::Margin::symmetric(9, 4))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(color).strong());
        });
}

fn metric(ui: &mut egui::Ui, label: &str, value: String) {
    theme::card_frame().show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(label).color(theme::MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(value).strong().color(theme::TEXT));
            });
        });
    });
}

fn status_text(status: SaveStatus) -> &'static str {
    match status {
        SaveStatus::Idle => "Idle",
        SaveStatus::Dirty => "Unsaved",
        SaveStatus::Saved => "Saved",
        SaveStatus::Syncing => "Sync pending",
    }
}

fn status_color(status: SaveStatus) -> Color32 {
    match status {
        SaveStatus::Idle => theme::MUTED,
        SaveStatus::Dirty => theme::AMBER,
        SaveStatus::Saved => theme::TEAL,
        SaveStatus::Syncing => theme::BLUE,
    }
}
