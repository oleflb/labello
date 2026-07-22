use eframe::egui::{self, Color32, RichText};
use labello_domain::{AdjudicationDecision, AnnotationGeometry, KeypointState, ReviewDecision};

use crate::{
    app::{
        AppView, Drawer, LabelloApp, LayoutMode, PendingTransition, SaveStatus, Tool,
        annotation_type_label,
    },
    canvas::{CanvasAction, CanvasInteraction, show_canvas_configured},
    theme,
};

impl LabelloApp {
    pub(crate) fn top_bar(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Labello")
                    .size(22.0)
                    .strong()
                    .color(theme::TEXT),
            );
            bounded_badge(
                ui,
                &format!("Dataset {}", self.config.dataset_id),
                theme::BLUE,
                if layout == LayoutMode::Compact {
                    132.0
                } else {
                    220.0
                },
            );
            if self.work_view() {
                badge(
                    ui,
                    status_text(self.save_status),
                    status_color(self.save_status),
                );
            }
            if let Some(error) = &self.runtime.storage_error {
                ui.colored_label(theme::AMBER, bounded_message(error));
            } else if let Some(error) = &self.runtime.error {
                ui.colored_label(theme::AMBER, bounded_message(error));
            } else if let Some(notice) = &self.runtime.notice {
                ui.colored_label(theme::TEAL, bounded_message(notice));
            }
        });
        ui.add_space(2.0);
        if layout == LayoutMode::Compact {
            ui.horizontal_wrapped(|ui| {
                self.compact_navigation(ui);
                if self.work_view() {
                    ui.menu_button("Panels", |ui| {
                        if ui.button("Workflow").clicked() {
                            self.drawer = Some(Drawer::Workflow);
                            ui.close();
                        }
                        if ui.button("Inspector").clicked() {
                            self.drawer = Some(Drawer::Inspector);
                            ui.close();
                        }
                        if ui.button("Settings").clicked() {
                            self.show_settings = true;
                            ui.close();
                        }
                    });
                }
                if self.auth.account.is_some()
                    && ui
                        .add_enabled(!self.loading.logout, egui::Button::new("Sign out"))
                        .clicked()
                {
                    self.request_logout();
                }
            });
            return;
        }
        ui.horizontal_wrapped(|ui| {
            self.mode_toolbar(ui);
            if layout != LayoutMode::Wide && self.work_view() {
                if ui.button("Workflow").clicked() {
                    self.drawer = if self.drawer == Some(Drawer::Workflow) {
                        None
                    } else {
                        Some(Drawer::Workflow)
                    };
                }
                if ui.button("Inspector").clicked() {
                    self.drawer = if self.drawer == Some(Drawer::Inspector) {
                        None
                    } else {
                        Some(Drawer::Inspector)
                    };
                }
            }
            if self.work_view() && ui.button("Settings").clicked() {
                self.show_settings = true;
            }
            if layout == LayoutMode::Wide {
                self.workspace_actions(ui);
            }
            if let Some(account) = &self.auth.account {
                ui.separator();
                if layout == LayoutMode::Wide || !self.work_view() {
                    ui.add_sized(
                        [180.0, 44.0],
                        egui::Label::new(&account.display_name).truncate(),
                    );
                }
                if ui
                    .add_enabled(!self.loading.logout, egui::Button::new("Sign out"))
                    .clicked()
                {
                    self.request_logout();
                }
            }
        });
    }

    pub(crate) fn workspace_actions(&mut self, ui: &mut egui::Ui) {
        if !self.work_view() {
            return;
        }
        let ready = (self.assignment.is_some() || self.runtime.api.is_none())
            && !self.loading.saving
            && !self.loading.image
            && self.pending_transition.is_none();
        if self.view == AppView::Annotate {
            if ui
                .add_enabled(
                    ready && !self.undo_stack.is_empty(),
                    egui::Button::new("Undo"),
                )
                .on_hover_text("Undo the last annotation edit (Ctrl/Cmd+Z).")
                .clicked()
            {
                self.undo();
            }
            if ui
                .add_enabled(
                    ready && !self.redo_stack.is_empty(),
                    egui::Button::new("Redo"),
                )
                .on_hover_text("Redo the last undone edit (Ctrl/Cmd+Shift+Z or Ctrl+Y).")
                .clicked()
            {
                self.redo();
            }
            if ui
                .add_enabled(
                    ready && matches!(self.save_status, SaveStatus::Dirty | SaveStatus::Retry),
                    egui::Button::new("Save"),
                )
                .on_hover_text("Save edits and keep this assignment active.")
                .clicked()
            {
                self.autosave();
            }
            if ui
                .add_enabled(ready, egui::Button::new("Submit & next"))
                .on_hover_text("Save, complete this assignment, and claim another.")
                .clicked()
            {
                self.submit_and_advance();
            }
        }
        if ui
            .add_enabled(ready, egui::Button::new("Skip"))
            .on_hover_text("Release this assignment and claim another.")
            .clicked()
        {
            self.skip_assignment();
        }
        ui.toggle_value(&mut self.show_tutorial, "Tutorial");
    }

    pub(crate) fn compact_workspace_actions(&mut self, ui: &mut egui::Ui) {
        let ready = (self.assignment.is_some() || self.runtime.api.is_none())
            && !self.loading.saving
            && !self.loading.image
            && self.pending_transition.is_none();
        ui.horizontal_wrapped(|ui| {
            if self.view == AppView::Annotate
                && ui
                    .add_enabled(ready, egui::Button::new("Submit & next"))
                    .clicked()
            {
                self.submit_and_advance();
            }
            ui.menu_button("More actions", |ui| {
                if self.view == AppView::Annotate {
                    if ui
                        .add_enabled(
                            ready && !self.undo_stack.is_empty(),
                            egui::Button::new("Undo"),
                        )
                        .clicked()
                    {
                        self.undo();
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            ready && !self.redo_stack.is_empty(),
                            egui::Button::new("Redo"),
                        )
                        .clicked()
                    {
                        self.redo();
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            ready
                                && matches!(
                                    self.save_status,
                                    SaveStatus::Dirty | SaveStatus::Retry
                                ),
                            egui::Button::new("Save"),
                        )
                        .clicked()
                    {
                        self.autosave();
                        ui.close();
                    }
                }
                if ui.add_enabled(ready, egui::Button::new("Skip")).clicked() {
                    self.skip_assignment();
                    ui.close();
                }
                if ui
                    .toggle_value(&mut self.show_tutorial, "Tutorial")
                    .clicked()
                {
                    ui.close();
                }
            });
        });
    }

    pub(crate) fn task_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading(RichText::new("Workflow").color(theme::TEXT));
        ui.label(RichText::new("Choose one enabled class workflow.").color(theme::MUTED));
        ui.add_space(8.0);
        let workflows = self.workflow_choices();
        if workflows.is_empty() {
            ui.colored_label(theme::AMBER, "No enabled one-class workflows configured.");
        }
        for workflow in workflows {
            let selected = self.selected_task_id.as_ref() == Some(&workflow.task_id);
            theme::card_frame().show(ui, |ui| {
                if ui
                    .selectable_label(selected, RichText::new(workflow.label()).strong())
                    .clicked()
                    && !selected
                {
                    self.request_transition(PendingTransition::Workflow(workflow.task_id.clone()));
                }
                ui.label(annotation_type_label(&workflow.annotation_type));
                ui.small(format!("Task: {}", workflow.task_name));
                if selected {
                    badge(ui, "Current", theme::TEAL);
                }
            });
            ui.add_space(6.0);
        }
        ui.separator();
        ui.label(RichText::new("Assignment").strong());
        if self.runtime.api.is_none() {
            ui.label("Demo image");
            ui.small("Local demo state; changes are not persisted.");
        } else if self.assignment.is_some() {
            ui.label("Active assignment");
            ui.small("Reserved for you until you submit or skip it.");
        } else if self.loading.image {
            ui.spinner();
            ui.label("Claiming work...");
        } else {
            ui.label(RichText::new("No active assignment").color(theme::MUTED));
        }
    }

    pub(crate) fn right_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading(RichText::new("Inspector").color(theme::TEXT));
        let active_count = self
            .annotations
            .iter()
            .filter(|annotation| {
                !annotation.deleted && self.annotation_matches_selected_workflow(annotation)
            })
            .count();
        metric(ui, "Active annotations", active_count.to_string());
        if self.view == AppView::Annotate && self.tool == Tool::Keypoints {
            self.keypoint_actions(ui);
        }
        match self.view {
            AppView::Annotate => self.prelabel_panel(ui),
            AppView::Review => self.review_actions(ui),
            AppView::Adjudicate => self.adjudication_actions(ui),
            AppView::Setup | AppView::Admin | AppView::Stats => {}
        }
    }

    fn keypoint_actions(&mut self, ui: &mut egui::Ui) {
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
            if let Some(spec) = spec {
                ui.add_enabled_ui(!self.loading.saving, |ui| {
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
                });
            }
        }
    }

    fn review_actions(&mut self, ui: &mut egui::Ui) {
        let ready = self.assignment.is_some() && !self.loading.saving;
        if self.correction_draft.is_some() {
            self.correction_actions(ui, ready);
            return;
        }
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
        if self.can_correct_review_object() {
            ui.add_space(8.0);
            if ui
                .add_enabled(ready, egui::Button::new("Correct object"))
                .on_hover_text("Edit this existing object without returning it to the annotator.")
                .clicked()
            {
                self.start_correction();
            }
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(ready, egui::Button::new("Approve  Y"))
                .clicked()
            {
                self.request_review(ReviewDecision::Approved);
            }
            if ui
                .add_enabled(ready, egui::Button::new("Reject  N"))
                .clicked()
            {
                self.request_review(ReviewDecision::Rejected);
            }
        });
    }

    fn correction_actions(&mut self, ui: &mut egui::Ui, ready: bool) {
        ui.separator();
        ui.heading("Correction mode");
        ui.label("Only the highlighted existing object can be edited.");

        let (can_undo, geometry_changed) = self
            .correction_draft
            .as_ref()
            .map(|draft| (!draft.geometry_history.is_empty(), draft.geometry_changed()))
            .unwrap_or_default();
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(ready && can_undo, egui::Button::new("Undo correction"))
                .clicked()
            {
                self.undo_correction();
            }
            if ui
                .add_enabled(ready, egui::Button::new("Discard correction"))
                .clicked()
            {
                self.discard_correction();
            }
            if ui
                .add_enabled(
                    ready && geometry_changed,
                    egui::Button::new("Correct & finalize"),
                )
                .on_disabled_hover_text("Move, resize, or change a keypoint before finalizing.")
                .clicked()
            {
                self.request_correction();
            }
        });

        let skeleton_keypoints = self.correction_draft.as_ref().and_then(|draft| {
            let AnnotationGeometry::Skeleton(skeleton) = &draft.edited_geometry else {
                return None;
            };
            Some(
                skeleton
                    .keypoints
                    .iter()
                    .enumerate()
                    .map(|(index, keypoint)| (index, keypoint.name.clone(), keypoint.state.clone()))
                    .collect::<Vec<_>>(),
            )
        });
        if let Some(keypoints) = skeleton_keypoints {
            ui.label("Select and drag an existing keypoint:");
            for (index, name, state) in keypoints {
                let selected = self
                    .correction_draft
                    .as_ref()
                    .is_some_and(|draft| draft.selected_keypoint == Some(index));
                if ui
                    .selectable_label(
                        selected,
                        format!("{name} ({})", keypoint_state_label(&state)),
                    )
                    .clicked()
                {
                    self.select_correction_keypoint(index);
                }
            }
            self.correction_keypoint_state(ui, ready);
        } else {
            ui.label("Drag inside the box to move it, or drag a handle to resize it.");
        }

        if let Some(draft) = self.correction_draft.as_mut() {
            ui.label("Reason (optional)");
            ui.add_enabled(
                ready,
                egui::TextEdit::multiline(&mut draft.reason)
                    .desired_rows(2)
                    .hint_text("What was corrected?"),
            );
        }
    }

    fn correction_keypoint_state(&mut self, ui: &mut egui::Ui, ready: bool) {
        let Some((index, current, has_point, required)) =
            self.correction_draft.as_ref().and_then(|draft| {
                let index = draft.selected_keypoint?;
                let AnnotationGeometry::Skeleton(skeleton) = &draft.edited_geometry else {
                    return None;
                };
                let keypoint = skeleton.keypoints.get(index)?;
                let required = self
                    .selected_task()
                    .and_then(|task| task.skeleton.as_ref())
                    .and_then(|spec| spec.keypoints.get(index))
                    .is_some_and(|spec| spec.required);
                Some((
                    index,
                    keypoint.state.clone(),
                    keypoint.point.is_some(),
                    required,
                ))
            })
        else {
            return;
        };
        let (allow_hidden, allow_absent) = self
            .selected_task()
            .and_then(|task| task.skeleton.as_ref())
            .map(|spec| (spec.allow_hidden, spec.allow_absent))
            .unwrap_or_default();
        ui.label(format!("Keypoint {} visibility", index + 1));
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    ready && has_point,
                    egui::Button::selectable(current == KeypointState::Visible, "Visible"),
                )
                .clicked()
            {
                self.set_correction_keypoint_state(KeypointState::Visible);
            }
            if ui
                .add_enabled(
                    ready && allow_hidden && has_point,
                    egui::Button::selectable(current == KeypointState::Hidden, "Hidden"),
                )
                .clicked()
            {
                self.set_correction_keypoint_state(KeypointState::Hidden);
            }
            if ui
                .add_enabled(
                    ready && allow_absent && !required,
                    egui::Button::selectable(current == KeypointState::Absent, "Absent"),
                )
                .clicked()
            {
                self.set_correction_keypoint_state(KeypointState::Absent);
            }
        });
    }

    fn adjudication_actions(&mut self, ui: &mut egui::Ui) {
        let ready = self.assignment.is_some() && !self.loading.saving;
        if ui
            .add_enabled(ready, egui::Button::new("Adjudicate accept"))
            .clicked()
        {
            self.request_adjudication(AdjudicationDecision::AcceptAnnotation);
        }
        if ui
            .add_enabled(ready, egui::Button::new("Needs correction"))
            .clicked()
        {
            self.request_adjudication(AdjudicationDecision::NeedsCorrection);
        }
    }

    pub(crate) fn central(&mut self, ui: &mut egui::Ui) {
        match self.view {
            AppView::Setup => {
                centered_scroll(ui, 760.0, |ui| self.setup_view(ui));
                return;
            }
            AppView::Admin => {
                centered_scroll(ui, 1100.0, |ui| self.admin_view(ui));
                return;
            }
            AppView::Stats => {
                centered_scroll(ui, 1100.0, |ui| self.stats_view(ui));
                return;
            }
            AppView::Annotate | AppView::Review | AppView::Adjudicate => {}
        }
        if self.show_tutorial
            && let Some(task) = self.selected_task()
        {
            theme::card_frame().show(ui, |ui| {
                ui.heading(&task.instructions.title);
                ui.label(&task.instructions.example_text);
            });
            ui.add_space(8.0);
        }
        if let Some(current) = self.current.clone() {
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::Label::new(RichText::new(&current.image.file_name).strong()).truncate(),
                );
                ui.label(
                    RichText::new(format!(
                        "{} x {}",
                        current.image.width, current.image.height
                    ))
                    .color(theme::MUTED),
                );
                if let Some(workflow) = self.selected_workflow() {
                    badge(ui, &workflow.label(), theme::TEAL);
                }
            });
            ui.add_space(2.0);
            let texture = self.current_texture.clone();
            let mut annotations = self
                .annotations
                .iter()
                .filter(|annotation| self.annotation_matches_selected_workflow(annotation))
                .cloned()
                .collect::<Vec<_>>();
            if let Some(draft) = self.correction_draft.as_ref()
                && let Some(annotation) = annotations
                    .iter_mut()
                    .find(|annotation| annotation.annotation_id == draft.annotation_id)
            {
                annotation.geometry = draft.edited_geometry.clone();
            }
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
            let prelabels = self.visible_prelabels();
            let annotator_editable =
                self.view == AppView::Annotate && self.pending_transition.is_none();
            let correction_interaction = self.correction_draft.as_ref().map(|draft| {
                let mut interaction = CanvasInteraction::correction(draft.selected_keypoint);
                interaction.editable = !self.loading.saving;
                interaction
            });
            let interaction = correction_interaction
                .unwrap_or_else(|| CanvasInteraction::annotations(annotator_editable));
            let bounding_box_tool = self.tool == Tool::BoundingBox;
            let selected_annotation = self.selected_annotation.clone();
            if self.view == AppView::Review {
                let review_annotation = selected_annotation.as_ref().and_then(|id| {
                    annotations
                        .iter()
                        .find(|annotation| !annotation.deleted && &annotation.annotation_id == id)
                });
                self.canvas.set_review_focus(review_annotation);
            } else {
                self.canvas.clear_review_focus();
            }
            let action = show_canvas_configured(
                ui,
                &mut self.canvas,
                texture.as_ref(),
                &annotations,
                [current.image.width, current.image.height],
                bounding_box_tool,
                selected_annotation.as_ref(),
                interaction,
                &skeleton_edges,
                &prelabels,
            );
            if annotator_editable {
                match action {
                    Some(CanvasAction::CreateBoundingBox(bbox)) => self.create_bbox(bbox),
                    Some(CanvasAction::PlaceKeypoint(point)) => self.place_keypoint(point),
                    Some(CanvasAction::Select(id)) => self.selected_annotation = Some(id),
                    Some(CanvasAction::EditBoundingBox(edit)) => self.edit_bbox(edit),
                    Some(CanvasAction::SelectKeypoint(_)) | Some(CanvasAction::EditKeypoint(_)) => {
                    }
                    None => {}
                }
            } else if self.correction_draft.is_some() {
                match action {
                    Some(CanvasAction::EditBoundingBox(edit)) => self.edit_correction_bbox(edit),
                    Some(CanvasAction::SelectKeypoint(selection)) => {
                        if self
                            .correction_draft
                            .as_ref()
                            .is_some_and(|draft| draft.annotation_id == selection.annotation_id)
                        {
                            self.select_correction_keypoint(selection.keypoint_index);
                        }
                    }
                    Some(CanvasAction::EditKeypoint(edit)) => self.edit_correction_keypoint(edit),
                    Some(CanvasAction::CreateBoundingBox(_))
                    | Some(CanvasAction::PlaceKeypoint(_))
                    | Some(CanvasAction::Select(_))
                    | None => {}
                }
            }
        } else {
            ui.vertical_centered(|ui| {
                if self.loading.dataset {
                    ui.spinner();
                    ui.label("Opening dataset...");
                } else if self.loading.image {
                    ui.spinner();
                    ui.label("Loading assignment...");
                } else if let Some(error) = self.runtime.error.clone() {
                    ui.colored_label(theme::AMBER, error);
                    if ui.button("Retry image load").clicked() {
                        self.retry_assignment_load();
                    }
                } else {
                    ui.label(match self.view {
                        AppView::Annotate => "No annotation assignments available.",
                        AppView::Review => "No review assignments available.",
                        AppView::Adjudicate => "No adjudication assignments available.",
                        _ => "No assignments available.",
                    });
                    if ui.button("Retry image load").clicked() {
                        self.retry_assignment_load();
                    }
                }
            });
        }
    }

    pub(crate) fn overlays(&mut self, ctx: &egui::Context, layout: LayoutMode) {
        if layout != LayoutMode::Wide {
            let screen = ctx.content_rect();
            let compact = layout == LayoutMode::Compact;
            let width = if compact {
                (screen.width() - 16.0).max(240.0)
            } else {
                340.0_f32.min(screen.width() - 24.0)
            };
            let max_height = if compact {
                (screen.height() * 0.58).max(240.0)
            } else {
                (screen.height() - 24.0).max(240.0)
            };
            match self.drawer {
                Some(Drawer::Workflow) => {
                    let mut open = true;
                    egui::Window::new("Workflow drawer")
                        .open(&mut open)
                        .anchor(
                            if compact {
                                egui::Align2::CENTER_BOTTOM
                            } else {
                                egui::Align2::LEFT_CENTER
                            },
                            if compact {
                                egui::vec2(0.0, -8.0)
                            } else {
                                egui::vec2(12.0, 0.0)
                            },
                        )
                        .default_width(width)
                        .max_width(width)
                        .max_height(max_height)
                        .constrain_to(screen)
                        .show(ctx, |ui| {
                            egui::ScrollArea::vertical()
                                .max_height(max_height - 48.0)
                                .show(ui, |ui| self.task_panel(ui));
                        });
                    if !open {
                        self.drawer = None;
                    }
                }
                Some(Drawer::Inspector) => {
                    let mut open = true;
                    egui::Window::new("Inspector drawer")
                        .open(&mut open)
                        .anchor(
                            if compact {
                                egui::Align2::CENTER_BOTTOM
                            } else {
                                egui::Align2::RIGHT_CENTER
                            },
                            if compact {
                                egui::vec2(0.0, -8.0)
                            } else {
                                egui::vec2(-12.0, 0.0)
                            },
                        )
                        .default_width(width)
                        .max_width(width)
                        .max_height(max_height)
                        .constrain_to(screen)
                        .show(ctx, |ui| {
                            egui::ScrollArea::vertical()
                                .max_height(max_height - 48.0)
                                .show(ui, |ui| self.right_panel(ui));
                        });
                    if !open {
                        self.drawer = None;
                    }
                }
                None => {}
            }
        }
        self.draft_recovery_modal(ctx);
        self.transition_modal(ctx);
        self.settings_modal(ctx);
    }

    fn draft_recovery_modal(&mut self, ctx: &egui::Context) {
        let Some(recovery) = self.runtime.persistence.recovery.clone() else {
            return;
        };
        let (title, timestamp, validation) = match recovery {
            crate::persistence::DraftRecovery::Work(draft, validation) => {
                ("Unsaved assignment draft", draft.updated_at, validation)
            }
            crate::persistence::DraftRecovery::Admin(draft, validation) => {
                ("Unsaved admin draft", draft.updated_at, validation)
            }
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .max_width((ctx.content_rect().width() - 24.0).max(240.0))
            .max_height((ctx.content_rect().height() - 24.0).max(240.0))
            .constrain_to(ctx.content_rect())
            .show(ctx, |ui| {
                ui.label(format!(
                    "Saved {}",
                    timestamp.format("%Y-%m-%d %H:%M:%S UTC")
                ));
                match validation {
                    crate::persistence::DraftValidation::Valid => {
                        ui.label(
                            "The server assignment and base event sequence match exactly. Recover or discard this draft.",
                        );
                        ui.horizontal_wrapped(|ui| {
                            if ui.button("Recover draft").clicked() {
                                self.recover_browser_draft();
                            }
                            if ui.button("Discard draft").clicked() {
                                self.discard_browser_draft();
                            }
                        });
                    }
                    crate::persistence::DraftValidation::Expired(message)
                    | crate::persistence::DraftValidation::Conflict(message) => {
                        ui.colored_label(theme::AMBER, message);
                        ui.label(
                            "Recovery is disabled so this draft cannot overwrite newer server state.",
                        );
                        ui.horizontal_wrapped(|ui| {
                            ui.add_enabled(false, egui::Button::new("Recover draft"));
                            if ui.button("Discard status").clicked() {
                                self.discard_browser_draft();
                            }
                            if ui.button("Keep status").clicked() {
                                self.runtime.persistence.recovery = None;
                            }
                        });
                    }
                }
            });
    }

    fn transition_modal(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.pending_transition.clone() else {
            return;
        };
        let current = self
            .selected_workflow()
            .map(|workflow| workflow.label())
            .unwrap_or_else(|| "No workflow".to_string());
        let destination = self.transition_label(&pending);
        egui::Window::new("Switch active assignment?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .max_width((ctx.content_rect().width() - 24.0).max(240.0))
            .max_height((ctx.content_rect().height() - 24.0).max(240.0))
            .constrain_to(ctx.content_rect())
            .show(ctx, |ui| {
                ui.label(format!("Current workflow: {current}"));
                ui.label(format!("Pending destination: {destination}"));
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    if self.view == AppView::Annotate
                        && ui
                            .add_enabled(
                                !self.loading.saving,
                                egui::Button::new("Submit and switch"),
                            )
                            .clicked()
                    {
                        self.submit_pending_transition();
                    }
                    if ui
                        .add_enabled(
                            !self.loading.saving,
                            egui::Button::new("Release and switch"),
                        )
                        .clicked()
                    {
                        self.release_pending_transition();
                    }
                    if ui
                        .add_enabled(!self.loading.saving, egui::Button::new("Cancel"))
                        .clicked()
                    {
                        self.cancel_pending_transition();
                    }
                });
            });
    }

    fn transition_label(&self, transition: &PendingTransition) -> String {
        match transition {
            PendingTransition::NextAssignment => "Next assignment".to_string(),
            PendingTransition::Workflow(task_id) => self
                .workflow_choices()
                .into_iter()
                .find(|workflow| workflow.task_id == *task_id)
                .map(|workflow| workflow.label())
                .unwrap_or_else(|| task_id.to_string()),
            PendingTransition::View(view) => view_label(*view).to_string(),
        }
    }

    fn settings_modal(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut open = self.show_settings;
        let screen = ctx.content_rect();
        let width = 560.0_f32.min((screen.width() - 24.0).max(240.0));
        egui::Window::new("Settings")
            .open(&mut open)
            .default_width(width)
            .max_width(width)
            .max_height((screen.height() - 24.0).max(240.0))
            .constrain_to(screen)
            .show(ctx, |ui| {
                ui.heading("Keyboard shortcuts");
                egui::ScrollArea::vertical()
                    .max_height((screen.height() - 190.0).clamp(160.0, 360.0))
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
                                    egui::TextEdit::singleline(&mut chord.key).desired_width(88.0),
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
                ui.horizontal_wrapped(|ui| {
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
                });
            });
        self.show_settings = open;
    }

    fn visible_prelabels(&self) -> Vec<labello_domain::PrelabelSuggestion> {
        if self.view != AppView::Annotate {
            return Vec::new();
        }
        self.current
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
                    .collect()
            })
            .unwrap_or_default()
    }

    fn prelabel_panel(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.heading("Prelabels");
        let prelabels = self.visible_prelabels();
        if prelabels.is_empty() {
            ui.label(RichText::new("No suggestions for this image.").color(theme::MUTED));
        }
        for suggestion in &prelabels {
            theme::card_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(suggestion.class_id.to_string());
                    badge(
                        ui,
                        &format!("{:.0}%", suggestion.confidence * 100.0),
                        theme::TEAL,
                    );
                    if ui
                        .add_enabled(!self.loading.saving, egui::Button::new("Accept"))
                        .clicked()
                    {
                        self.accept_prelabel(suggestion);
                    }
                    if ui
                        .add_enabled(!self.loading.saving, egui::Button::new("Discard"))
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

fn centered_scroll(ui: &mut egui::Ui, max_width: f32, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        let width = ui.available_width().min(max_width);
        let inset = ((ui.available_width() - width) * 0.5).max(0.0);
        ui.horizontal(|ui| {
            ui.add_space(inset);
            ui.vertical(|ui| {
                ui.set_width(width);
                add_contents(ui);
            });
        });
    });
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

fn bounded_badge(ui: &mut egui::Ui, text: &str, color: Color32, width: f32) {
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
            ui.add_sized(
                [width, 24.0],
                egui::Label::new(RichText::new(text).color(color).strong()).truncate(),
            );
        });
}

fn bounded_message(message: &str) -> String {
    const MAX_CHARS: usize = 70;
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
        UserAction::NextImage => "Submit and next",
        UserAction::PreviousImage => "Previous image",
        UserAction::SaveAnnotations => "Save annotations",
        UserAction::DeleteAnnotation => "Delete annotation",
        UserAction::SelectBoundingBoxTool => "Bounding-box tool",
        UserAction::SelectKeypointTool => "Keypoint tool",
        UserAction::AcceptReviewObject => "Approve review object",
        UserAction::RejectReviewObject => "Reject review object",
        UserAction::OpenTutorial => "Open tutorial",
        UserAction::ToggleOfflineMode => "Offline mode",
    }
}

fn view_label(view: AppView) -> &'static str {
    match view {
        AppView::Setup => "Setup",
        AppView::Annotate => "Annotate",
        AppView::Review => "Review",
        AppView::Adjudicate => "Adjudicate",
        AppView::Admin => "Admin",
        AppView::Stats => "Stats",
    }
}

fn status_text(status: SaveStatus) -> &'static str {
    match status {
        SaveStatus::Idle => "Idle",
        SaveStatus::Dirty => "Unsaved",
        SaveStatus::Saved => "Saved",
        SaveStatus::Saving => "Saving",
        SaveStatus::Retry => "Retry",
    }
}

fn status_color(status: SaveStatus) -> Color32 {
    match status {
        SaveStatus::Idle => theme::MUTED,
        SaveStatus::Dirty => theme::AMBER,
        SaveStatus::Saved => theme::TEAL,
        SaveStatus::Saving => theme::BLUE,
        SaveStatus::Retry => theme::RED,
    }
}

fn keypoint_state_label(state: &KeypointState) -> &'static str {
    match state {
        KeypointState::Visible => "visible",
        KeypointState::Hidden => "hidden",
        KeypointState::Absent => "absent",
    }
}
