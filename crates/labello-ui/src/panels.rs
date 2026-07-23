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
        let dataset_name = self
            .datasets
            .metadata
            .as_ref()
            .filter(|metadata| metadata.dataset_id == self.config.dataset_id)
            .map(|metadata| metadata.name.as_str())
            .or_else(|| {
                self.datasets
                    .summaries
                    .iter()
                    .find(|summary| summary.dataset_id == self.config.dataset_id)
                    .map(|summary| summary.name.as_str())
            })
            .unwrap_or(self.config.dataset_id.as_str());
        let dataset_label = format!("Dataset {dataset_name}");
        let save_status = self.work_view().then(|| {
            (
                status_text(self.save_status),
                status_color(self.save_status),
            )
        });
        let runtime_status = if let Some(error) = &self.runtime.storage_error {
            Some((error.as_str(), theme::AMBER))
        } else if let Some(error) = &self.runtime.error {
            Some((error.as_str(), theme::AMBER))
        } else {
            self.runtime
                .notice
                .as_deref()
                .map(|notice| (notice, theme::TEAL))
        };
        let show_identity = |ui: &mut egui::Ui| {
            ui.label(
                RichText::new("Labello")
                    .size(22.0)
                    .strong()
                    .color(theme::TEXT),
            );
            bounded_badge(
                ui,
                &dataset_label,
                theme::BLUE,
                if layout == LayoutMode::Compact {
                    96.0
                } else {
                    220.0
                },
            );
        };
        if layout == LayoutMode::Compact {
            ui.horizontal_wrapped(|ui| {
                show_identity(ui);
                if let Some((text, color)) = save_status {
                    bounded_badge(ui, text, color, 56.0);
                }
                if let Some((message, color)) = runtime_status {
                    status_message(ui, message, color);
                }
            });
        } else {
            ui.horizontal(|ui| {
                show_identity(ui);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some((text, color)) = save_status {
                        bounded_badge(ui, text, color, 72.0);
                    }
                    if let Some((message, color)) = runtime_status {
                        status_message(ui, message, color);
                    }
                });
            });
        }
        ui.add_space(2.0);
        if layout == LayoutMode::Compact {
            ui.horizontal_wrapped(|ui| {
                self.compact_navigation(ui);
                if self.work_view() {
                    ui.menu_button("Panels", |ui| {
                        if ui
                            .add(
                                egui::Button::new("Workflow").shortcut_text(self.shortcut_text(
                                    ui.ctx(),
                                    labello_domain::UserAction::ToggleWorkflowPanel,
                                )),
                            )
                            .clicked()
                        {
                            self.trigger_user_action(
                                labello_domain::UserAction::ToggleWorkflowPanel,
                            );
                            ui.close();
                        }
                        if ui
                            .add(
                                egui::Button::new("Inspector").shortcut_text(self.shortcut_text(
                                    ui.ctx(),
                                    labello_domain::UserAction::ToggleInspectorPanel,
                                )),
                            )
                            .clicked()
                        {
                            self.trigger_user_action(
                                labello_domain::UserAction::ToggleInspectorPanel,
                            );
                            ui.close();
                        }
                        if ui
                            .add(egui::Button::new("Settings").shortcut_text(
                                self.shortcut_text(
                                    ui.ctx(),
                                    labello_domain::UserAction::OpenSettings,
                                ),
                            ))
                            .clicked()
                        {
                            self.open_shortcut_settings();
                            ui.close();
                        }
                        if self.auth.account.is_some() {
                            ui.separator();
                            if ui
                                .add_enabled(!self.loading.logout, egui::Button::new("Sign out"))
                                .clicked()
                            {
                                self.request_logout();
                                ui.close();
                            }
                        }
                    });
                }
                if !self.work_view()
                    && self.auth.account.is_some()
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
                if ui
                    .add(egui::Button::new("Workflow").shortcut_text(
                        self.shortcut_text(
                            ui.ctx(),
                            labello_domain::UserAction::ToggleWorkflowPanel,
                        ),
                    ))
                    .clicked()
                {
                    self.trigger_user_action(labello_domain::UserAction::ToggleWorkflowPanel);
                }
                if ui
                    .add(egui::Button::new("Inspector").shortcut_text(
                        self.shortcut_text(
                            ui.ctx(),
                            labello_domain::UserAction::ToggleInspectorPanel,
                        ),
                    ))
                    .clicked()
                {
                    self.trigger_user_action(labello_domain::UserAction::ToggleInspectorPanel);
                }
            }
            if self.work_view()
                && ui
                    .add(egui::Button::new("Settings").shortcut_text(
                        self.shortcut_text(ui.ctx(), labello_domain::UserAction::OpenSettings),
                    ))
                    .clicked()
            {
                self.open_shortcut_settings();
            }
            if layout == LayoutMode::Wide {
                self.workspace_actions(ui, layout);
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

    pub(crate) fn workspace_actions(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
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
                self.trigger_user_action(labello_domain::UserAction::UndoEdit);
            }
            if ui
                .add_enabled(
                    ready && !self.redo_stack.is_empty(),
                    egui::Button::new("Redo"),
                )
                .on_hover_text("Redo the last undone edit (Ctrl/Cmd+Shift+Z or Ctrl+Y).")
                .clicked()
            {
                self.trigger_user_action(labello_domain::UserAction::RedoEdit);
            }
            if ui
                .add_enabled(
                    ready && matches!(self.save_status, SaveStatus::Dirty | SaveStatus::Retry),
                    egui::Button::new("Save"),
                )
                .on_hover_text("Save edits and keep this assignment active.")
                .clicked()
            {
                self.trigger_user_action(labello_domain::UserAction::SaveAnnotations);
            }
            if ui
                .add_enabled(ready, egui::Button::new("Submit & next"))
                .on_hover_text("Save, complete this assignment, and claim another.")
                .clicked()
            {
                self.trigger_user_action(labello_domain::UserAction::NextImage);
            }
        }
        if layout != LayoutMode::Wide
            && self.view == AppView::Review
            && self.correction_draft.is_none()
        {
            self.review_decision_buttons(ui);
        }
        if layout != LayoutMode::Wide && self.view == AppView::Adjudicate {
            self.adjudication_decision_buttons(ui, false);
        }
        if ui
            .add_enabled(ready, egui::Button::new("Skip"))
            .on_hover_text("Release this assignment and claim another.")
            .clicked()
        {
            self.trigger_user_action(labello_domain::UserAction::SkipAssignment);
        }
        if ui
            .add(egui::Button::selectable(self.show_tutorial, "Tutorial"))
            .clicked()
        {
            self.trigger_user_action(labello_domain::UserAction::OpenTutorial);
        }
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
                self.trigger_user_action(labello_domain::UserAction::NextImage);
            }
            if self.view == AppView::Review && self.correction_draft.is_none() {
                self.review_decision_buttons(ui);
            }
            if self.view == AppView::Adjudicate {
                self.adjudication_decision_buttons(ui, true);
            }
            ui.menu_button(
                if self.view == AppView::Annotate {
                    "More actions"
                } else {
                    "More"
                },
                |ui| {
                    if self.view == AppView::Annotate {
                        if ui
                            .add_enabled(
                                ready && !self.undo_stack.is_empty(),
                                egui::Button::new("Undo").shortcut_text(
                                    self.shortcut_text(
                                        ui.ctx(),
                                        labello_domain::UserAction::UndoEdit,
                                    ),
                                ),
                            )
                            .clicked()
                        {
                            self.trigger_user_action(labello_domain::UserAction::UndoEdit);
                            ui.close();
                        }
                        if ui
                            .add_enabled(
                                ready && !self.redo_stack.is_empty(),
                                egui::Button::new("Redo").shortcut_text(
                                    self.shortcut_text(
                                        ui.ctx(),
                                        labello_domain::UserAction::RedoEdit,
                                    ),
                                ),
                            )
                            .clicked()
                        {
                            self.trigger_user_action(labello_domain::UserAction::RedoEdit);
                            ui.close();
                        }
                        if ui
                            .add_enabled(
                                ready
                                    && matches!(
                                        self.save_status,
                                        SaveStatus::Dirty | SaveStatus::Retry
                                    ),
                                egui::Button::new("Save").shortcut_text(self.shortcut_text(
                                    ui.ctx(),
                                    labello_domain::UserAction::SaveAnnotations,
                                )),
                            )
                            .clicked()
                        {
                            self.trigger_user_action(labello_domain::UserAction::SaveAnnotations);
                            ui.close();
                        }
                    }
                    if ui
                        .add_enabled(
                            ready,
                            egui::Button::new("Skip").shortcut_text(self.shortcut_text(
                                ui.ctx(),
                                labello_domain::UserAction::SkipAssignment,
                            )),
                        )
                        .clicked()
                    {
                        self.trigger_user_action(labello_domain::UserAction::SkipAssignment);
                        ui.close();
                    }
                    if ui
                        .add(
                            egui::Button::selectable(self.show_tutorial, "Tutorial").shortcut_text(
                                self.shortcut_text(
                                    ui.ctx(),
                                    labello_domain::UserAction::OpenTutorial,
                                ),
                            ),
                        )
                        .clicked()
                    {
                        self.trigger_user_action(labello_domain::UserAction::OpenTutorial);
                        ui.close();
                    }
                },
            );
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
            let frame = if selected {
                theme::card_frame()
                    .fill(Color32::from_rgb(32, 48, 76))
                    .stroke(egui::Stroke::new(1.5, theme::TEAL.gamma_multiply(0.75)))
            } else {
                theme::card_frame()
            };
            frame.show(ui, |ui| {
                if ui
                    .selectable_label(selected, RichText::new(workflow.label()).strong())
                    .on_hover_text(format!(
                        "Previous: {} · Next: {}",
                        self.shortcut_text(
                            ui.ctx(),
                            labello_domain::UserAction::SelectPreviousWorkflow,
                        ),
                        self.shortcut_text(
                            ui.ctx(),
                            labello_domain::UserAction::SelectNextWorkflow,
                        )
                    ))
                    .clicked()
                    && !selected
                {
                    self.request_transition(PendingTransition::Workflow(workflow.task_id.clone()));
                }
                badge(
                    ui,
                    annotation_type_label(&workflow.annotation_type),
                    theme::BLUE,
                );
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
            if self.view == AppView::Annotate {
                let status = if self.queue.failed() {
                    "Prepared queue refill failed; retrying".to_string()
                } else if self.queue.is_loading() {
                    format!(
                        "Prepared queue: {} of {} ready, loading next",
                        self.queue.len(),
                        self.queue.queue_size()
                    )
                } else {
                    format!(
                        "Prepared queue: {} of {} ready",
                        self.queue.len(),
                        self.queue.queue_size()
                    )
                };
                ui.small(status);
            }
        } else if self.loading.image {
            ui.spinner();
            ui.label("Claiming work...");
        } else {
            ui.label(RichText::new("No active assignment").color(theme::MUTED));
        }
    }

    pub(crate) fn right_panel(&mut self, ui: &mut egui::Ui, show_primary_actions: bool) {
        ui.heading(RichText::new("Inspector").color(theme::TEXT));
        let active_count = self
            .annotations
            .iter()
            .filter(|annotation| {
                !annotation.deleted && self.annotation_matches_selected_workflow(annotation)
            })
            .count();
        metric(ui, "Active annotations", active_count.to_string());
        if self.view == AppView::Annotate {
            self.annotation_object_actions(ui);
        }
        if self.view == AppView::Annotate && self.tool == Tool::Keypoints {
            self.keypoint_actions(ui);
        }
        match self.view {
            AppView::Annotate => self.prelabel_panel(ui),
            AppView::Review => self.review_actions(ui, show_primary_actions),
            AppView::Adjudicate => self.adjudication_actions(ui, show_primary_actions),
            AppView::Setup | AppView::Admin | AppView::Stats => {}
        }
    }

    fn annotation_object_actions(&mut self, ui: &mut egui::Ui) {
        let objects = self
            .annotations
            .iter()
            .filter(|annotation| {
                !annotation.deleted && self.annotation_matches_selected_workflow(annotation)
            })
            .enumerate()
            .map(|(index, annotation)| {
                let class_name = self.class_name(&annotation.class_id);
                let geometry = match &annotation.geometry {
                    AnnotationGeometry::BoundingBox(bbox) => format!(
                        "box at {:.0}%, {:.0}%, size {:.0}% by {:.0}%",
                        bbox.x * 100.0,
                        bbox.y * 100.0,
                        bbox.width * 100.0,
                        bbox.height * 100.0
                    ),
                    AnnotationGeometry::Skeleton(skeleton) => format!(
                        "skeleton with {} of {} keypoints placed",
                        skeleton
                            .keypoints
                            .iter()
                            .filter(|keypoint| keypoint.point.is_some())
                            .count(),
                        skeleton.keypoints.len()
                    ),
                };
                (
                    annotation.annotation_id.clone(),
                    format!("Object {}: {class_name}, {geometry}", index + 1),
                )
            })
            .collect::<Vec<_>>();
        if objects.is_empty() {
            ui.label(RichText::new("Draw or accept an object to inspect it.").color(theme::MUTED));
            return;
        }

        if self.selected_annotation.is_some()
            && !objects
                .iter()
                .any(|(annotation_id, _)| Some(annotation_id) == self.selected_annotation.as_ref())
        {
            self.selected_annotation = None;
        }

        ui.separator();
        ui.label(RichText::new("Objects").strong());
        for (annotation_id, label) in objects {
            let selected = self.selected_annotation.as_ref() == Some(&annotation_id);
            if ui
                .selectable_label(selected, label)
                .on_hover_text(format!(
                    "Previous: {} · Next: {}",
                    self.shortcut_text(ui.ctx(), labello_domain::UserAction::SelectPreviousObject,),
                    self.shortcut_text(ui.ctx(), labello_domain::UserAction::SelectNextObject,)
                ))
                .clicked()
            {
                self.selected_annotation = Some(annotation_id);
            }
        }
        if self.selected_annotation.is_some()
            && ui
                .add(
                    egui::Button::new("Delete selected annotation").shortcut_text(
                        self.shortcut_text(ui.ctx(), labello_domain::UserAction::DeleteAnnotation),
                    ),
                )
                .clicked()
        {
            self.delete_selected();
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
                            ui.checkbox(&mut self.next_keypoint_hidden, "Hidden")
                                .on_hover_text(format!(
                                    "Shortcut: {}",
                                    self.shortcut_text(
                                        ui.ctx(),
                                        labello_domain::UserAction::ToggleKeypointHidden,
                                    )
                                ));
                        }
                        if spec.allow_absent
                            && self.active_skeleton.is_some()
                            && ui
                                .add(egui::Button::new("Mark absent").shortcut_text(
                                    self.shortcut_text(
                                        ui.ctx(),
                                        labello_domain::UserAction::MarkKeypointAbsent,
                                    ),
                                ))
                                .clicked()
                        {
                            self.skip_keypoint();
                        }
                    });
                });
            }
        }
    }

    fn review_actions(&mut self, ui: &mut egui::Ui, show_primary_actions: bool) {
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
        if show_primary_actions {
            ui.horizontal_wrapped(|ui| self.review_decision_buttons(ui));
        }
    }

    fn review_decision_buttons(&mut self, ui: &mut egui::Ui) {
        let ready = self.assignment.is_some() && !self.loading.saving;
        let (approve, reject) = if self.current_review_annotation().is_none() {
            ("Complete review", "Send back")
        } else {
            ("Approve object", "Reject object & finish")
        };
        if ui.add_enabled(ready, egui::Button::new(approve)).clicked() {
            self.request_review(ReviewDecision::Approved);
        }
        if ui.add_enabled(ready, egui::Button::new(reject)).clicked() {
            self.request_review(ReviewDecision::Rejected);
        }
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

    fn adjudication_actions(&mut self, ui: &mut egui::Ui, show_primary_actions: bool) {
        let candidates = self
            .annotations
            .iter()
            .filter(|annotation| {
                !annotation.deleted && self.annotation_matches_selected_workflow(annotation)
            })
            .count();
        metric(ui, "Candidate annotations", candidates.to_string());
        if candidates == 0 {
            ui.colored_label(
                theme::AMBER,
                "This assignment has no annotation candidates to adjudicate.",
            );
        }
        if show_primary_actions {
            ui.horizontal_wrapped(|ui| self.adjudication_decision_buttons(ui, false));
        }
    }

    fn adjudication_decision_buttons(&mut self, ui: &mut egui::Ui, compact: bool) {
        let has_candidates = self.annotations.iter().any(|annotation| {
            !annotation.deleted && self.annotation_matches_selected_workflow(annotation)
        });
        let ready = self.assignment.is_some() && !self.loading.saving;
        let (accept, correct) = if compact {
            ("Accept all", "Send back")
        } else {
            ("Accept all annotations", "Send back for correction")
        };
        if ui
            .add_enabled(ready && has_candidates, egui::Button::new(accept))
            .clicked()
        {
            self.request_adjudication(AdjudicationDecision::AcceptAnnotation);
        }
        if ui.add_enabled(ready, egui::Button::new(correct)).clicked() {
            self.request_adjudication(AdjudicationDecision::NeedsCorrection);
        }
    }

    pub(crate) fn central(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
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
        if let Some(current) = self.current.clone() {
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                self.work_context_header(ui, &current, layout);
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
                    if ui
                        .add(egui::Button::new("Retry image load").shortcut_text(
                            self.shortcut_text(
                                ui.ctx(),
                                labello_domain::UserAction::RetryImageLoad,
                            ),
                        ))
                        .clicked()
                    {
                        self.retry_assignment_load();
                    }
                } else {
                    ui.label(match self.view {
                        AppView::Annotate => "No annotation assignments available.",
                        AppView::Review => "No review assignments available.",
                        AppView::Adjudicate => "No adjudication assignments available.",
                        _ => "No assignments available.",
                    });
                    if ui
                        .add(egui::Button::new("Retry image load").shortcut_text(
                            self.shortcut_text(
                                ui.ctx(),
                                labello_domain::UserAction::RetryImageLoad,
                            ),
                        ))
                        .clicked()
                    {
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
                (screen.height() * 0.7).max(240.0)
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
                                egui::vec2(0.0, -24.0)
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
                                .max_height(ui.available_height())
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
                                egui::vec2(0.0, -24.0)
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
                                .max_height(ui.available_height())
                                .show(ui, |ui| self.right_panel(ui, false));
                        });
                    if !open {
                        self.drawer = None;
                    }
                }
                None => {}
            }
        }
        self.tutorial_overlay(ctx);
        self.draft_recovery_modal(ctx);
        self.transition_modal(ctx);
        self.settings_modal(ctx);
    }

    fn work_context_header(
        &mut self,
        ui: &mut egui::Ui,
        current: &crate::queue::QueuedImage,
        layout: LayoutMode,
    ) {
        self.image_metadata_row(ui, current, layout);
        if layout == LayoutMode::Compact
            && let Some(workflow) = self.selected_workflow()
        {
            let label = workflow.label();
            let outer_width = ui.available_width().min(220.0);
            bounded_badge(ui, &label, theme::TEAL, (outer_width - 18.0).max(24.0));
        }
        if self.view == AppView::Annotate {
            self.canvas_controls(ui, layout);
        }
    }

    fn image_metadata_row(
        &self,
        ui: &mut egui::Ui,
        current: &crate::queue::QueuedImage,
        layout: LayoutMode,
    ) {
        let dimensions = format!("{} x {}", current.image.width, current.image.height);
        let workflow = (layout != LayoutMode::Compact)
            .then(|| self.selected_workflow().map(|workflow| workflow.label()))
            .flatten();
        let workflow_outer_width = workflow
            .as_ref()
            .map(|_| {
                if layout == LayoutMode::Wide {
                    220.0
                } else {
                    160.0
                }
            })
            .unwrap_or_default();
        let dimensions_width = 76.0;
        let metadata_height = 34.0;

        ui.horizontal(|ui| {
            let gaps = if workflow.is_some() { 2.0 } else { 1.0 } * ui.spacing().item_spacing.x;
            let filename_width =
                (ui.available_width() - dimensions_width - workflow_outer_width - gaps).max(40.0);
            ui.add_sized(
                [filename_width, metadata_height],
                egui::Label::new(RichText::new(&current.image.file_name).strong()).truncate(),
            )
            .on_hover_text(&current.image.file_name);
            ui.add_sized(
                [dimensions_width, metadata_height],
                egui::Label::new(RichText::new(dimensions).color(theme::MUTED)),
            );
            if let Some(workflow) = workflow {
                bounded_badge(
                    ui,
                    &workflow,
                    theme::TEAL,
                    (workflow_outer_width - 18.0).max(24.0),
                );
            }
        });
    }

    fn canvas_controls(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
        ui.horizontal(|ui| {
            let show_shortcuts = layout == LayoutMode::Wide;
            let pan_shortcut =
                self.shortcut_text(ui.ctx(), labello_domain::UserAction::TogglePanMode);
            let mut pan = egui::Button::selectable(self.canvas.pan_mode(), "Pan")
                .min_size(egui::vec2(44.0, 44.0));
            if show_shortcuts {
                pan = pan.shortcut_text(&pan_shortcut);
            }
            if ui
                .add_enabled(self.canvas.can_pan(), pan)
                .on_disabled_hover_text("Zoom in before enabling Pan mode.")
                .on_hover_text(format!("Pan the image ({pan_shortcut})."))
                .clicked()
            {
                self.trigger_user_action(labello_domain::UserAction::TogglePanMode);
            }

            let zoom_out_shortcut =
                self.shortcut_text(ui.ctx(), labello_domain::UserAction::ZoomOut);
            let mut zoom_out = egui::Button::new(if show_shortcuts { "Zoom out" } else { "−" })
                .min_size(egui::vec2(44.0, 44.0));
            if show_shortcuts {
                zoom_out = zoom_out.shortcut_text(&zoom_out_shortcut);
            }
            let zoom_out_response = ui
                .add(zoom_out)
                .on_hover_text(format!("Zoom out ({zoom_out_shortcut})."));
            zoom_out_response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Zoom out")
            });
            if zoom_out_response.clicked() {
                self.trigger_user_action(labello_domain::UserAction::ZoomOut);
            }

            ui.add_sized(
                [48.0, 44.0],
                egui::Label::new(format!("{:.0}%", self.canvas.current_zoom() * 100.0)),
            );

            let zoom_in_shortcut = self.shortcut_text(ui.ctx(), labello_domain::UserAction::ZoomIn);
            let mut zoom_in = egui::Button::new(if show_shortcuts { "Zoom in" } else { "+" })
                .min_size(egui::vec2(44.0, 44.0));
            if show_shortcuts {
                zoom_in = zoom_in.shortcut_text(&zoom_in_shortcut);
            }
            let zoom_in_response = ui
                .add(zoom_in)
                .on_hover_text(format!("Zoom in ({zoom_in_shortcut})."));
            zoom_in_response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Zoom in")
            });
            if zoom_in_response.clicked() {
                self.trigger_user_action(labello_domain::UserAction::ZoomIn);
            }

            let fit_shortcut = self.shortcut_text(ui.ctx(), labello_domain::UserAction::FitImage);
            let mut fit = egui::Button::new("Fit").min_size(egui::vec2(44.0, 44.0));
            if show_shortcuts {
                fit = fit.shortcut_text(&fit_shortcut);
            }
            if ui
                .add(fit)
                .on_hover_text(format!("Fit and center the image ({fit_shortcut})."))
                .clicked()
            {
                self.trigger_user_action(labello_domain::UserAction::FitImage);
            }

            if layout != LayoutMode::Compact && self.canvas.pan_mode() {
                ui.label(RichText::new("Primary-drag to pan").color(theme::TEAL));
            }
        });
    }

    fn tutorial_overlay(&mut self, ctx: &egui::Context) {
        if !self.show_tutorial {
            return;
        }
        let Some((title, text)) = self.selected_task().map(|task| {
            (
                task.instructions.title.clone(),
                task.instructions.example_text.clone(),
            )
        }) else {
            return;
        };
        let screen = ctx.content_rect();
        let mut open = true;
        egui::Window::new("Tutorial")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 12.0))
            .max_width((screen.width() - 24.0).clamp(240.0, 420.0))
            .max_height((screen.height() - 24.0).clamp(240.0, 560.0))
            .constrain_to(screen)
            .show(ctx, |ui| {
                ui.heading(title);
                egui::ScrollArea::vertical().show(ui, |ui| ui.label(text));
            });
        if !open {
            self.show_tutorial = false;
        }
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
        egui::Modal::new(egui::Id::new("draft-recovery-modal")).show(ctx, |ui| {
                ui.set_max_width((ctx.content_rect().width() - 48.0).clamp(240.0, 560.0));
                ui.heading(title);
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
        let discards_edits = pending == PendingTransition::NextAssignment
            && self.view == AppView::Annotate
            && matches!(self.save_status, SaveStatus::Dirty | SaveStatus::Retry);
        if pending == PendingTransition::NextAssignment && !discards_edits {
            return;
        }
        egui::Modal::new(egui::Id::new("assignment-transition-modal")).show(ctx, |ui| {
            ui.set_max_width((ctx.content_rect().width() - 48.0).clamp(240.0, 560.0));
            ui.heading(if discards_edits {
                "Unsaved annotation changes"
            } else {
                "Switch active assignment?"
            });
            ui.label(format!("Current workflow: {current}"));
            ui.label(format!("Pending destination: {destination}"));
            if discards_edits {
                ui.colored_label(
                    theme::AMBER,
                    "Skipping now will discard annotation changes that have not been saved.",
                );
            }
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                if self.view == AppView::Annotate
                    && ui
                        .add_enabled(!self.loading.saving, egui::Button::new("Submit and switch"))
                        .clicked()
                {
                    self.submit_pending_transition();
                }
                if ui
                    .add_enabled(
                        !self.loading.saving,
                        egui::Button::new(if discards_edits {
                            "Discard edits and skip"
                        } else {
                            "Release and switch"
                        }),
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
        if self.shortcut_settings.draft.is_none() {
            let mut draft = self.keybindings.clone();
            draft.normalize();
            self.shortcut_settings.baseline = Some(draft.clone());
            self.shortcut_settings.draft = Some(draft);
        }
        if !self.loading.keybindings
            && let Some(action) = self.shortcut_settings.recording
        {
            let captured = ctx.input(|input| {
                input.events.iter().rev().find_map(|event| match event {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        repeat: false,
                        modifiers,
                        ..
                    } => Some((*key, *modifiers)),
                    _ => None,
                })
            });
            if let Some((key, modifiers)) = captured {
                if key == egui::Key::Escape {
                    self.shortcut_settings.recording = None;
                } else if let Some(draft) = self.shortcut_settings.draft.as_mut() {
                    draft.bindings.insert(
                        action,
                        labello_domain::KeyChord {
                            key: key.name().to_string(),
                            ctrl: false,
                            shift: modifiers.shift,
                            alt: modifiers.alt,
                            command: modifiers.command || modifiers.ctrl,
                        },
                    );
                    self.shortcut_settings.recording = None;
                }
            }
        }
        let mut open = self.show_settings;
        let screen = ctx.content_rect();
        let width = 720.0_f32.min((screen.width() - 24.0).max(240.0));
        let mut record = None;
        let mut reset_action = None;
        let mut save = false;
        let mut cancel = false;
        let mut reset_all = false;
        let window = egui::Window::new("Settings")
            .default_width(width)
            .max_width(width)
            .max_height((screen.height() - 24.0).max(240.0))
            .constrain_to(screen);
        let window = if self.loading.keybindings {
            window
        } else {
            window.open(&mut open)
        };
        window.show(ctx, |ui| {
            ui.heading("Keyboard shortcuts");
            ui.label(
                RichText::new("Choose an action, then press its new key combination.")
                    .color(theme::MUTED),
            );
            if let Some(error) = &self.shortcut_settings.error {
                ui.colored_label(theme::RED, format!("Could not save shortcuts: {error}"));
            }
            ui.add_space(6.0);
            let search_label = ui.label("Search actions");
            ui.add(
                egui::TextEdit::singleline(&mut self.shortcut_settings.search)
                    .hint_text("Search by action or category")
                    .desired_width(f32::INFINITY),
            )
            .labelled_by(search_label.id);
            ui.add_space(8.0);
            let conflicts = self
                .shortcut_settings
                .draft
                .as_ref()
                .map(|draft| draft.conflicts())
                .unwrap_or_default();
            let conflicting_actions = conflicts
                .iter()
                .flat_map(|(_, actions)| actions.iter().copied())
                .collect::<std::collections::BTreeSet<_>>();
            let query = self.shortcut_settings.search.trim().to_ascii_lowercase();
            egui::ScrollArea::vertical()
                .max_height((screen.height() - 300.0).clamp(180.0, 520.0))
                .show(ui, |ui| {
                    let mut current_category = "";
                    for action in labello_domain::UserAction::ACTIVE {
                        let label = action_label(&action);
                        let category = action_category(action);
                        let description = action_description(action);
                        if !query.is_empty()
                            && !label.to_ascii_lowercase().contains(&query)
                            && !category.to_ascii_lowercase().contains(&query)
                            && !description.to_ascii_lowercase().contains(&query)
                        {
                            continue;
                        }
                        if category != current_category {
                            if !current_category.is_empty() {
                                ui.add_space(8.0);
                            }
                            current_category = category;
                            ui.heading(RichText::new(category).size(16.0));
                        }
                        let Some(chord) = self
                            .shortcut_settings
                            .draft
                            .as_ref()
                            .and_then(|draft| draft.bindings.get(&action))
                        else {
                            continue;
                        };
                        let recording = self.shortcut_settings.recording == Some(action);
                        let conflict = conflicting_actions.contains(&action);
                        theme::card_frame().show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(label).strong());
                                    ui.small(RichText::new(description).color(theme::MUTED));
                                });
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let reset_response = ui.add_enabled(
                                            !self.loading.keybindings,
                                            egui::Button::new("Reset")
                                                .min_size(egui::vec2(64.0, 44.0)),
                                        );
                                        reset_response.widget_info(|| {
                                            egui::WidgetInfo::labeled(
                                                egui::WidgetType::Button,
                                                true,
                                                format!("Reset {label}"),
                                            )
                                        });
                                        if reset_response.clicked() {
                                            reset_action = Some(action);
                                        }
                                        let text = if recording {
                                            "Press shortcut…".to_string()
                                        } else {
                                            format_chord(ctx, chord)
                                        };
                                        let record_response = ui
                                            .add_enabled(
                                                !self.loading.keybindings,
                                                egui::Button::new(&text)
                                                    .selected(recording)
                                                    .min_size(egui::vec2(140.0, 44.0)),
                                            )
                                            .on_hover_text(format!("Record shortcut for {label}"));
                                        record_response.widget_info(|| {
                                            egui::WidgetInfo::labeled(
                                                egui::WidgetType::Button,
                                                true,
                                                format!("Record shortcut for {label}: {text}"),
                                            )
                                        });
                                        if record_response.clicked() {
                                            record = Some(action);
                                        }
                                    },
                                );
                            });
                            if conflict {
                                ui.colored_label(theme::RED, "Conflicts in this context");
                            }
                        });
                        ui.add_space(4.0);
                    }
                });
            if !conflicts.is_empty() {
                ui.colored_label(
                    theme::RED,
                    format!(
                        "Resolve {} shortcut conflict(s) before saving.",
                        conflicts.len()
                    ),
                );
            }
            let dirty = self.shortcut_settings.draft != self.shortcut_settings.baseline;
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        !self.loading.keybindings,
                        egui::Button::new("Restore all defaults"),
                    )
                    .clicked()
                {
                    reset_all = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            dirty && conflicts.is_empty() && !self.loading.keybindings,
                            egui::Button::new(if self.loading.keybindings {
                                "Saving…"
                            } else {
                                "Save changes"
                            }),
                        )
                        .clicked()
                    {
                        save = true;
                    }
                    if ui
                        .add_enabled(!self.loading.keybindings, egui::Button::new("Cancel"))
                        .clicked()
                    {
                        cancel = true;
                    }
                });
                if dirty {
                    ui.label(RichText::new("Unsaved changes").color(theme::AMBER));
                }
            });
        });
        if let Some(action) = record {
            self.shortcut_settings.recording = Some(action);
        }
        if let Some(action) = reset_action {
            let default = labello_domain::KeybindingSet::defaults_for(self.config.user_id.clone())
                .bindings
                .get(&action)
                .cloned();
            if let (Some(draft), Some(default)) = (self.shortcut_settings.draft.as_mut(), default) {
                draft.bindings.insert(action, default);
            }
        }
        if reset_all {
            self.shortcut_settings.draft = Some(labello_domain::KeybindingSet::defaults_for(
                self.config.user_id.clone(),
            ));
            self.shortcut_settings.recording = None;
        }
        if save {
            self.request_keybindings_save();
        }
        let dirty = self.shortcut_settings.draft != self.shortcut_settings.baseline;
        if cancel || !open {
            if dirty {
                self.shortcut_settings.confirm_discard = true;
                self.show_settings = true;
            } else {
                self.show_settings = false;
                self.shortcut_settings.draft = None;
                self.shortcut_settings.baseline = None;
                self.shortcut_settings.error = None;
            }
        }
        if self.shortcut_settings.confirm_discard {
            egui::Modal::new(egui::Id::new("discard-shortcut-settings")).show(ctx, |ui| {
                ui.heading("Discard shortcut changes?");
                ui.label("Your recorded shortcuts have not been saved.");
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Keep editing").clicked() {
                        self.shortcut_settings.confirm_discard = false;
                    }
                    if ui.button("Discard changes").clicked() {
                        self.shortcut_settings.confirm_discard = false;
                        self.shortcut_settings.draft = None;
                        self.shortcut_settings.baseline = None;
                        self.shortcut_settings.error = None;
                        self.show_settings = false;
                    }
                });
            });
        }
    }

    pub(crate) fn visible_prelabels(&self) -> Vec<labello_domain::PrelabelSuggestion> {
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
        if self.selected_prelabel.as_ref().is_none_or(|selected| {
            !prelabels
                .iter()
                .any(|suggestion| &suggestion.suggestion_id == selected)
        }) {
            self.selected_prelabel = prelabels
                .first()
                .map(|suggestion| suggestion.suggestion_id.clone());
        }
        if prelabels.is_empty() {
            ui.label(RichText::new("No suggestions for this image.").color(theme::MUTED));
        }
        for suggestion in &prelabels {
            let selected = self.selected_prelabel.as_ref() == Some(&suggestion.suggestion_id);
            let frame = if selected {
                theme::card_frame().stroke(egui::Stroke::new(1.5, theme::TEAL))
            } else {
                theme::card_frame()
            };
            frame.show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .selectable_label(selected, suggestion.class_id.to_string())
                        .clicked()
                    {
                        self.selected_prelabel = Some(suggestion.suggestion_id.clone());
                    }
                    badge(
                        ui,
                        &format!("{:.0}%", suggestion.confidence * 100.0),
                        theme::TEAL,
                    );
                    if ui
                        .add_enabled(!self.loading.saving, egui::Button::new("Accept"))
                        .on_hover_text(format!(
                            "Shortcut: {}",
                            self.shortcut_text(
                                ui.ctx(),
                                labello_domain::UserAction::AcceptPrelabel,
                            )
                        ))
                        .clicked()
                    {
                        self.accept_prelabel(suggestion);
                        self.selected_prelabel = self
                            .visible_prelabels()
                            .first()
                            .map(|suggestion| suggestion.suggestion_id.clone());
                    }
                    if ui
                        .add_enabled(!self.loading.saving, egui::Button::new("Discard"))
                        .on_hover_text(format!(
                            "Shortcut: {}",
                            self.shortcut_text(
                                ui.ctx(),
                                labello_domain::UserAction::DiscardPrelabel,
                            )
                        ))
                        .clicked()
                    {
                        self.discard_prelabel(suggestion.suggestion_id.clone());
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

fn status_message(ui: &mut egui::Ui, message: &str, color: Color32) {
    ui.add_sized(
        [ui.available_width().min(520.0), 24.0],
        egui::Label::new(RichText::new(message).color(color)).truncate(),
    );
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
        UserAction::UndoEdit => "Undo annotation edit",
        UserAction::RedoEdit => "Redo annotation edit",
        UserAction::SkipAssignment => "Skip assignment",
        UserAction::ToggleWorkflowPanel => "Toggle Workflow panel",
        UserAction::ToggleInspectorPanel => "Toggle Inspector panel",
        UserAction::OpenSettings => "Open shortcut settings",
        UserAction::SelectPreviousWorkflow => "Previous workflow",
        UserAction::SelectNextWorkflow => "Next workflow",
        UserAction::SelectPreviousObject => "Previous object",
        UserAction::SelectNextObject => "Next object",
        UserAction::SelectPreviousPrelabel => "Previous prelabel",
        UserAction::SelectNextPrelabel => "Next prelabel",
        UserAction::AcceptPrelabel => "Accept active prelabel",
        UserAction::DiscardPrelabel => "Discard active prelabel",
        UserAction::ToggleKeypointHidden => "Toggle keypoint hidden",
        UserAction::MarkKeypointAbsent => "Mark keypoint absent",
        UserAction::RetryImageLoad => "Retry image load",
        UserAction::TogglePanMode => "Toggle Pan mode",
        UserAction::ZoomIn => "Zoom in",
        UserAction::ZoomOut => "Zoom out",
        UserAction::FitImage => "Fit image",
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

fn action_category(action: labello_domain::UserAction) -> &'static str {
    use labello_domain::UserAction;
    match action {
        UserAction::NextImage
        | UserAction::UndoEdit
        | UserAction::RedoEdit
        | UserAction::SaveAnnotations
        | UserAction::SkipAssignment => "Assignment",
        UserAction::SelectPreviousWorkflow
        | UserAction::SelectNextWorkflow
        | UserAction::SelectPreviousObject
        | UserAction::SelectNextObject
        | UserAction::DeleteAnnotation
        | UserAction::ToggleKeypointHidden
        | UserAction::MarkKeypointAbsent => "Annotation",
        UserAction::SelectPreviousPrelabel
        | UserAction::SelectNextPrelabel
        | UserAction::AcceptPrelabel
        | UserAction::DiscardPrelabel => "Prelabels",
        UserAction::TogglePanMode
        | UserAction::ZoomIn
        | UserAction::ZoomOut
        | UserAction::FitImage => "Canvas",
        UserAction::OpenTutorial
        | UserAction::ToggleWorkflowPanel
        | UserAction::ToggleInspectorPanel
        | UserAction::OpenSettings
        | UserAction::RetryImageLoad => "Workspace",
        UserAction::AcceptReviewObject | UserAction::RejectReviewObject => "Review",
        UserAction::PreviousImage
        | UserAction::SelectBoundingBoxTool
        | UserAction::SelectKeypointTool
        | UserAction::ToggleOfflineMode => "Legacy",
    }
}

fn action_description(action: labello_domain::UserAction) -> &'static str {
    use labello_domain::UserAction;
    match action {
        UserAction::NextImage => "Save, complete, and claim another image.",
        UserAction::UndoEdit => "Reverse the last annotation edit.",
        UserAction::RedoEdit => "Restore the last undone edit.",
        UserAction::SaveAnnotations => "Save without leaving the assignment.",
        UserAction::SkipAssignment => "Release this image and claim another.",
        UserAction::DeleteAnnotation => "Delete the selected object.",
        UserAction::OpenTutorial => "Show or hide workflow instructions.",
        UserAction::ToggleWorkflowPanel => "Open or close workflow navigation.",
        UserAction::ToggleInspectorPanel => "Open or close object controls.",
        UserAction::OpenSettings => "Open this keyboard shortcut editor.",
        UserAction::SelectPreviousWorkflow => "Cycle to the previous enabled workflow.",
        UserAction::SelectNextWorkflow => "Cycle to the next enabled workflow.",
        UserAction::SelectPreviousObject => "Select the previous annotation.",
        UserAction::SelectNextObject => "Select the next annotation.",
        UserAction::SelectPreviousPrelabel => "Highlight the previous suggestion.",
        UserAction::SelectNextPrelabel => "Highlight the next suggestion.",
        UserAction::AcceptPrelabel => "Convert the active suggestion to an annotation.",
        UserAction::DiscardPrelabel => "Hide the active suggestion.",
        UserAction::ToggleKeypointHidden => "Toggle visibility for the next keypoint.",
        UserAction::MarkKeypointAbsent => "Skip an allowed optional keypoint.",
        UserAction::RetryImageLoad => "Try to claim and load an image again.",
        UserAction::TogglePanMode => "Use primary drag to move a zoomed image.",
        UserAction::ZoomIn => "Increase canvas magnification.",
        UserAction::ZoomOut => "Decrease canvas magnification.",
        UserAction::FitImage => "Fit and center the image.",
        UserAction::AcceptReviewObject => "Approve the current review object.",
        UserAction::RejectReviewObject => "Reject the current review object.",
        UserAction::PreviousImage
        | UserAction::SelectBoundingBoxTool
        | UserAction::SelectKeypointTool
        | UserAction::ToggleOfflineMode => "No longer used.",
    }
}

fn format_chord(ctx: &egui::Context, chord: &labello_domain::KeyChord) -> String {
    let Some(key) = egui::Key::from_name(&chord.key) else {
        return chord.to_string();
    };
    let mut modifiers = egui::Modifiers::NONE;
    modifiers.command = chord.ctrl || chord.command;
    modifiers.shift = chord.shift;
    modifiers.alt = chord.alt;
    ctx.format_shortcut(&egui::KeyboardShortcut::new(modifiers, key))
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
