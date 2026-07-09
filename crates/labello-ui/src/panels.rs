use eframe::egui::{self, Color32, RichText};
use labello_domain::{AdjudicationDecision, ReviewDecision};

use crate::{
    app::{
        AppView, IMAGE_QUEUE_SIZE, LabelloApp, QueueMode, SaveStatus, Tool, annotation_type_label,
    },
    canvas::{CanvasAction, show_canvas},
    theme,
};

impl LabelloApp {
    pub(crate) fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.set_max_height(32.0);
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
            if let Some(error) = &self.runtime.error {
                ui.colored_label(theme::AMBER, bounded_message(error));
            } else if let Some(notice) = &self.runtime.notice {
                ui.colored_label(theme::TEAL, bounded_message(notice));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.mode_toolbar(ui);
                if ui
                    .button("Next image")
                    .on_hover_text("Save pending edits if needed and load the next assigned image.")
                    .clicked()
                {
                    self.next_image();
                }
                if self.queue_mode == QueueMode::Annotate {
                    if ui
                        .button("Save")
                        .on_hover_text("Persist current annotation edits as event-log entries.")
                        .clicked()
                    {
                        self.autosave();
                    }
                    if ui
                        .button("Submit")
                        .on_hover_text("Save annotations and mark the active task as submitted.")
                        .clicked()
                    {
                        self.request_save(true);
                    }
                }
                ui.toggle_value(&mut self.show_tutorial, "Tutorial");
                ui.toggle_value(&mut self.offline, "Offline");
            });
        });
    }

    pub(crate) fn task_panel(&mut self, ui: &mut egui::Ui) {
        if matches!(self.view, AppView::Setup | AppView::Admin | AppView::Stats) {
            return;
        }
        ui.heading(RichText::new("Workflow Focus").color(theme::TEXT));
        ui.label(
            RichText::new("Pick one class and annotation type for this work session.")
                .color(theme::MUTED),
        );
        ui.add_space(8.0);
        let selected_class_id = self.selected_class_id().cloned();
        let workflows = self.workflow_choices();
        if workflows.is_empty() {
            ui.colored_label(theme::AMBER, "No enabled workflows configured.");
        }
        for workflow in workflows {
            let selected = self.selected_task == workflow.task_index
                && selected_class_id.as_ref() == Some(&workflow.class_id);
            theme::card_frame().show(ui, |ui| {
                if ui
                    .selectable_label(selected, RichText::new(workflow.label()).strong())
                    .clicked()
                    && self.select_workflow(workflow.task_index, workflow.class_id.clone())
                {
                    self.clear_current_image();
                    self.request_next_image();
                }
                ui.small(format!("Task: {}", workflow.task_name));
            });
        }
        ui.add_space(10.0);
        ui.label(RichText::new("Queue Mode").strong());
        if ui
            .selectable_label(self.queue_mode == QueueMode::Annotate, "Annotate")
            .clicked()
        {
            self.set_queue_mode(QueueMode::Annotate);
        }
        if ui
            .selectable_label(self.queue_mode == QueueMode::Review, "Review")
            .clicked()
        {
            self.set_queue_mode(QueueMode::Review);
        }
        if ui
            .selectable_label(self.queue_mode == QueueMode::Adjudicate, "Adjudicate")
            .clicked()
        {
            self.set_queue_mode(QueueMode::Adjudicate);
        }
        ui.separator();
        if self.queue_mode == QueueMode::Annotate {
            ui.label(RichText::new("Tools").strong());
            if let Some(workflow) = self.selected_workflow() {
                badge(
                    ui,
                    annotation_type_label(&workflow.annotation_type),
                    theme::BLUE,
                );
            }
            ui.separator();
            ui.label(RichText::new("Classes").strong());
            if let Some(workflow) = self.selected_workflow() {
                badge(ui, &workflow.class_name, theme::TEAL);
            }
            ui.separator();
        }
        ui.label(RichText::new("Image Queue").strong());
        ui.label(format!("Queue size: {IMAGE_QUEUE_SIZE}"));
        if self.loading.dataset || self.queue.is_loading() || self.current.is_none() {
            ui.colored_label(theme::AMBER, "Loading next image...");
        } else {
            ui.colored_label(theme::MUTED, format!("{} images ready", self.queue.len()));
        }
    }

    pub(crate) fn right_panel(&mut self, ui: &mut egui::Ui) {
        if matches!(self.view, AppView::Setup | AppView::Admin | AppView::Stats) {
            return;
        }
        ui.heading(RichText::new("Review & Sync").color(theme::TEXT));
        let active_count = self
            .annotations
            .iter()
            .filter(|annotation| {
                !annotation.deleted && self.annotation_matches_selected_workflow(annotation)
            })
            .count();
        metric(ui, "Active annotations", active_count.to_string());
        match self.queue_mode {
            QueueMode::Annotate => self.prelabel_panel(ui),
            QueueMode::Review => {
                metric(ui, "Review cursor", (self.review_index + 1).to_string());
                ui.horizontal(|ui| {
                    if ui
                        .button("Approve y")
                        .on_hover_text("Approve the current review target.")
                        .clicked()
                    {
                        if self.runtime.api.is_some() {
                            self.request_review(ReviewDecision::Approved);
                        } else {
                            self.review_index = self.review_index.saturating_add(1);
                        }
                    }
                    if ui
                        .button("Reject n")
                        .on_hover_text("Reject the current review target and request correction.")
                        .clicked()
                    {
                        if self.runtime.api.is_some() {
                            self.request_review(ReviewDecision::Rejected);
                        } else {
                            self.save_status = SaveStatus::Dirty;
                        }
                    }
                });
            }
            QueueMode::Adjudicate => {
                ui.horizontal(|ui| {
                    if ui
                        .button("Adjudicate accept")
                        .on_hover_text("Resolve adjudication by accepting annotations.")
                        .clicked()
                    {
                        self.request_adjudication(AdjudicationDecision::AcceptAnnotation);
                    }
                    if ui
                        .button("Needs correction")
                        .on_hover_text(
                            "Resolve adjudication by sending the task back for correction.",
                        )
                        .clicked()
                    {
                        self.request_adjudication(AdjudicationDecision::NeedsCorrection);
                    }
                });
            }
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
        match self.view {
            AppView::Setup => {
                self.setup_view(ui);
                return;
            }
            AppView::Admin => {
                self.admin_view(ui);
                return;
            }
            AppView::Stats => {
                self.stats_view(ui);
                return;
            }
            AppView::Annotate => {}
        }
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
        if let Some(current) = self.current.clone() {
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
            let texture = self.current_texture.clone();
            let annotations = self
                .annotations
                .iter()
                .filter(|annotation| self.annotation_matches_selected_workflow(annotation))
                .cloned()
                .collect::<Vec<_>>();
            let bounding_box_tool = self.tool == Tool::BoundingBox;
            let action = show_canvas(
                ui,
                &mut self.canvas,
                texture.as_ref(),
                &annotations,
                [current.image.width, current.image.height],
                bounding_box_tool,
            );
            match action {
                Some(CanvasAction::CreateBoundingBox(bbox)) => self.create_bbox(bbox),
                Some(CanvasAction::Select(id)) => self.selected_annotation = Some(id),
                None => {}
            }
        } else {
            ui.vertical_centered(|ui| {
                if self.loading.dataset {
                    ui.spinner();
                    ui.label("Opening dataset...");
                } else if self.loading.image || self.queue.is_loading() {
                    ui.spinner();
                    ui.label("Loading next image...");
                } else if let Some(error) = self.runtime.error.clone() {
                    ui.colored_label(theme::AMBER, error);
                    if ui.button("Retry image load").clicked() {
                        self.request_next_image();
                    }
                } else {
                    ui.label(match self.queue_mode {
                        QueueMode::Annotate => "No annotation images available.",
                        QueueMode::Review => "No review images available.",
                        QueueMode::Adjudicate => "No adjudication images available.",
                    });
                    if ui.button("Retry image load").clicked() {
                        self.request_next_image();
                    }
                }
            });
        }
    }

    fn prelabel_panel(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.heading("Prelabels");
        let task_id = self.selected_task().map(|task| task.task_id.clone());
        let class_id = self.selected_class_id().cloned();
        let prelabels = self
            .current
            .as_ref()
            .map(|image| {
                image
                    .prelabels
                    .iter()
                    .filter(|suggestion| {
                        task_id.as_ref() == Some(&suggestion.task_id)
                            && class_id.as_ref() == Some(&suggestion.class_id)
                            && !self.accepted_prelabels.contains(&suggestion.suggestion_id)
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
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
                    if ui
                        .button("Accept")
                        .on_hover_text("Convert this suggestion into an annotation.")
                        .clicked()
                    {
                        self.accept_prelabel(suggestion);
                    }
                    if ui
                        .button("Discard")
                        .on_hover_text("Hide this suggestion for the current image.")
                        .clicked()
                    {
                        self.accepted_prelabels
                            .push(suggestion.suggestion_id.clone());
                    }
                });
            });
        }
    }
}

fn badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    ui.set_max_height(26.0);
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
            ui.set_max_height(24.0);
            ui.label(RichText::new(text).color(color).strong());
        });
}

fn bounded_message(message: &str) -> String {
    const MAX_CHARS: usize = 90;
    if message.chars().count() <= MAX_CHARS {
        message.to_string()
    } else {
        format!("{}...", message.chars().take(MAX_CHARS).collect::<String>())
    }
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
