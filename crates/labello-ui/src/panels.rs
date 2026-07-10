use eframe::egui::{self, Color32, RichText};
use labello_domain::{AdjudicationDecision, ReviewDecision};

use crate::{
    app::{
        AppView, LabelloApp, PendingTransition, QueueMode, SaveStatus, Tool, annotation_type_label,
    },
    canvas::{CanvasAction, show_canvas_interactive},
    theme,
};

impl LabelloApp {
    pub(crate) fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.set_max_height(32.0);
        ui.horizontal_wrapped(|ui| {
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
            if self.view == AppView::Annotate {
                badge(
                    ui,
                    status_text(self.save_status),
                    status_color(self.save_status),
                );
            }
            if let Some(error) = &self.runtime.error {
                ui.colored_label(theme::AMBER, bounded_message(error));
            } else if let Some(notice) = &self.runtime.notice {
                ui.colored_label(theme::TEAL, bounded_message(notice));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.mode_toolbar(ui);
                if self.view == AppView::Annotate
                    && ui
                        .add_enabled(
                            self.current.is_some() && !self.loading.saving && !self.loading.image,
                            egui::Button::new("Next image"),
                        )
                        .on_hover_text("Save pending edits, then load the next assigned image.")
                        .clicked()
                {
                    self.next_image();
                }
                if self.view == AppView::Annotate && self.queue_mode == QueueMode::Annotate {
                    if ui
                        .add_enabled(
                            self.current.is_some()
                                && self.save_status == SaveStatus::Dirty
                                && !self.loading.saving,
                            egui::Button::new("Save"),
                        )
                        .on_hover_text("Persist current annotation edits as event-log entries.")
                        .clicked()
                    {
                        self.autosave();
                    }
                    if ui
                        .add_enabled(
                            self.current.is_some() && !self.loading.saving,
                            egui::Button::new("Submit & next"),
                        )
                        .on_hover_text("Save annotations and mark the active task as submitted.")
                        .clicked()
                    {
                        self.submit_and_advance();
                    }
                }
                if self.view == AppView::Annotate {
                    ui.toggle_value(&mut self.show_tutorial, "Tutorial");
                }
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
                    && !selected
                {
                    self.request_transition(PendingTransition::Workflow {
                        task_index: workflow.task_index,
                        class_id: workflow.class_id.clone(),
                    });
                }
                ui.small(format!("Task: {}", workflow.task_name));
            });
        }
        ui.add_space(10.0);
        ui.label(RichText::new("Queue Mode").strong());
        for (mode, label) in [
            (QueueMode::Annotate, "Annotate"),
            (QueueMode::Review, "Review"),
            (QueueMode::Adjudicate, "Adjudicate"),
        ] {
            if self.can_use_queue_mode(mode)
                && ui
                    .selectable_label(self.queue_mode == mode, label)
                    .clicked()
            {
                self.set_queue_mode(mode);
            }
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
        ui.label("Assignments are reserved one at a time to avoid duplicate work.");
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
        if self.queue_mode == QueueMode::Annotate && self.tool == Tool::Keypoints {
            let spec = self.selected_task().and_then(|task| task.skeleton.clone());
            let next_keypoint = spec.as_ref().and_then(|skeleton| {
                skeleton
                    .keypoints
                    .get(self.skeleton_keypoint_index)
                    .map(|keypoint| keypoint.name.clone())
            });
            if let Some(name) = next_keypoint {
                metric(
                    ui,
                    if self.active_skeleton.is_some() {
                        "Place keypoint"
                    } else {
                        "Start skeleton"
                    },
                    name,
                );
                ui.small(
                    "Tap keypoints in order. The next tap starts a new object after completion.",
                );
                if let Some(spec) = spec {
                    ui.horizontal(|ui| {
                        if spec.allow_hidden {
                            ui.checkbox(&mut self.next_keypoint_hidden, "Hidden");
                        }
                        if spec.allow_absent
                            && self.active_skeleton.is_some()
                            && ui.button("Mark absent").clicked()
                        {
                            self.skip_keypoint();
                        }
                    });
                }
            }
        }
        match self.queue_mode {
            QueueMode::Annotate => self.prelabel_panel(ui),
            QueueMode::Review => {
                let total = self
                    .annotations
                    .iter()
                    .filter(|annotation| {
                        !annotation.deleted && self.annotation_matches_selected_workflow(annotation)
                    })
                    .count();
                if self.review_index < total {
                    metric(
                        ui,
                        "Object review",
                        format!("{} of {total}", self.review_index + 1),
                    );
                    ui.label("The active object is highlighted on the canvas.");
                } else {
                    metric(ui, "Final check", "Full image".to_string());
                    ui.label("Check for missed objects before completing this review.");
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.loading.saving, egui::Button::new("Approve  Y"))
                        .on_hover_text(if self.review_index < total {
                            "Approve the highlighted object."
                        } else {
                            "Confirm that no objects were missed."
                        })
                        .clicked()
                    {
                        if self.runtime.api.is_some() {
                            self.request_review(ReviewDecision::Approved);
                        } else {
                            self.review_index = self.review_index.saturating_add(1);
                        }
                    }
                    if ui
                        .add_enabled(!self.loading.saving, egui::Button::new("Reject  N"))
                        .on_hover_text(if self.review_index < total {
                            "Reject the highlighted object and request correction."
                        } else {
                            "Report a missed or incorrect object."
                        })
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
        ui.heading("Keyboard shortcuts");
        egui::ScrollArea::vertical()
            .max_height(260.0)
            .show(ui, |ui| {
                for (action, chord) in &mut self.keybindings.bindings {
                    if matches!(
                        action,
                        labello_domain::UserAction::PreviousImage
                            | labello_domain::UserAction::ToggleOfflineMode
                    ) {
                        continue;
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.label(action_label(action));
                        ui.add(
                            egui::TextEdit::singleline(&mut chord.key)
                                .desired_width(88.0)
                                .hint_text("Key"),
                        );
                        ui.checkbox(&mut chord.ctrl, "Ctrl");
                        ui.checkbox(&mut chord.shift, "Shift");
                        ui.checkbox(&mut chord.alt, "Alt");
                        ui.checkbox(&mut chord.command, "Cmd");
                    });
                }
            });
        let conflicts = self.keybindings.validate_conflicts().err();
        if let Some(error) = conflicts.as_ref() {
            ui.colored_label(theme::RED, error.to_string());
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    conflicts.is_none() && !self.loading.keybindings,
                    egui::Button::new("Save shortcuts"),
                )
                .clicked()
            {
                self.request_keybindings_save();
            }
            if ui.button("Reset defaults").clicked() {
                self.keybindings.reset_to_defaults();
                self.keybindings
                    .bindings
                    .remove(&labello_domain::UserAction::PreviousImage);
                self.keybindings
                    .bindings
                    .remove(&labello_domain::UserAction::ToggleOfflineMode);
            }
            if self.loading.keybindings {
                ui.spinner();
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
        if ui.available_width() < 900.0 {
            ui.horizontal(|ui| {
                egui::CollapsingHeader::new("Workflow")
                    .default_open(false)
                    .show(ui, |ui| self.task_panel(ui));
                egui::CollapsingHeader::new("Review and tools")
                    .default_open(false)
                    .show(ui, |ui| self.right_panel(ui));
            });
            ui.separator();
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
            let selected_annotation = self.selected_annotation.clone();
            let editable = self.queue_mode == QueueMode::Annotate;
            let skeleton_edges = self
                .selected_task()
                .and_then(|task| task.skeleton.as_ref())
                .map(|skeleton| {
                    skeleton
                        .edges
                        .iter()
                        .map(|edge| (edge.from.clone(), edge.to.clone()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let prelabels = self
                .current
                .as_ref()
                .map(|current| {
                    current
                        .prelabels
                        .iter()
                        .filter(|suggestion| {
                            !self.accepted_prelabels.contains(&suggestion.suggestion_id)
                                && self
                                    .selected_task()
                                    .is_some_and(|task| task.task_id == suggestion.task_id)
                                && self.selected_class_id() == Some(&suggestion.class_id)
                        })
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let action = show_canvas_interactive(
                ui,
                &mut self.canvas,
                texture.as_ref(),
                &annotations,
                [current.image.width, current.image.height],
                bounding_box_tool,
                selected_annotation.as_ref(),
                editable,
                &skeleton_edges,
                &prelabels,
            );
            match action {
                Some(CanvasAction::CreateBoundingBox(bbox)) => self.create_bbox(bbox),
                Some(CanvasAction::PlaceKeypoint(point)) => self.place_keypoint(point),
                Some(CanvasAction::Select(id)) => self.selected_annotation = Some(id),
                Some(CanvasAction::EditBoundingBox(edit)) => self.edit_bbox(edit),
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

fn action_label(action: &labello_domain::UserAction) -> &'static str {
    use labello_domain::UserAction;
    match action {
        UserAction::NextImage => "Shortcut: next image",
        UserAction::PreviousImage => "Shortcut: previous image",
        UserAction::SaveAnnotations => "Shortcut: save annotations",
        UserAction::DeleteAnnotation => "Shortcut: delete annotation",
        UserAction::SelectBoundingBoxTool => "Shortcut: bounding-box tool",
        UserAction::SelectKeypointTool => "Shortcut: keypoint tool",
        UserAction::AcceptReviewObject => "Shortcut: approve review object",
        UserAction::RejectReviewObject => "Shortcut: reject review object",
        UserAction::OpenTutorial => "Shortcut: open tutorial",
        UserAction::ToggleOfflineMode => "Shortcut: offline mode",
    }
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
