use eframe::egui::{self, RichText};
use labello_domain::{AdjudicationDecision, AnnotationGeometry, KeypointState, ReviewDecision};

use crate::{
    app::{
        AppView, Drawer, LabelloApp, LayoutMode, PendingTransition, SaveStatus, Tool,
        annotation_type_label,
    },
    canvas::{CanvasAction, CanvasInteraction, show_canvas_styled},
    theme,
};

impl LabelloApp {
    pub(crate) fn app_bar(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
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
            .unwrap_or(self.config.dataset_id.as_str())
            .to_owned();
        let save_status = self.work_view().then(|| {
            (
                status_text(self.save_status),
                status_intent(self.save_status),
            )
        });
        let runtime_status = if let Some(error) = &self.runtime.storage_error {
            Some((error.clone(), theme::Intent::Warning))
        } else if let Some(error) = &self.runtime.error {
            Some((error.clone(), theme::Intent::Warning))
        } else {
            self.runtime
                .notice
                .clone()
                .map(|notice| (notice, theme::Intent::Success))
        };
        let dataset_label = format!("Dataset {dataset_name}");
        let account = self
            .auth
            .account
            .as_ref()
            .map(|account| account.display_name.clone());

        let bar_rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(bar_rect, egui::Sense::hover());
        let dataset_width = if layout == LayoutMode::Compact {
            46.0
        } else {
            142.0
        };
        let dataset_rect = egui::Rect::from_center_size(
            bar_rect.center(),
            egui::vec2(dataset_width + 18.0, bar_rect.height()),
        );
        let mut center_ui = ui.new_child(egui::UiBuilder::new().max_rect(dataset_rect).layout(
            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
        ));
        let dataset_response = theme::bounded_badge(
            &mut center_ui,
            if layout == LayoutMode::Compact {
                &dataset_name
            } else {
                &dataset_label
            },
            theme::Intent::Info,
            dataset_width,
        )
        .on_hover_text("Current dataset");
        dataset_response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, dataset_label.clone())
        });

        let side_gap = theme::SPACE_2;
        let left_rect = egui::Rect::from_min_max(
            bar_rect.min,
            egui::pos2(dataset_rect.left() - side_gap, bar_rect.bottom()),
        );
        let mut left_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(left_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        if layout == LayoutMode::Wide && self.work_view() {
            left_ui.menu_button("Navigation", |ui| self.navigation_menu_contents(ui));
            left_ui.menu_button("Workspace", |ui| self.workspace_menu_contents(ui, layout));
        } else if layout != LayoutMode::Wide {
            left_ui.menu_button("Menu", |ui| self.application_menu_contents(ui, layout));
        }

        let right_rect = egui::Rect::from_min_max(
            egui::pos2(dataset_rect.right() + side_gap, bar_rect.top()),
            bar_rect.max,
        );
        let mut right_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(right_rect)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        if layout == LayoutMode::Wide
            && self.work_view()
            && let Some(account) = account.as_ref()
        {
            if theme::quiet_button(
                &mut right_ui,
                !self.loading.logout,
                egui::Button::new("Sign out").min_size(egui::vec2(84.0, 44.0)),
            )
            .clicked()
            {
                self.request_logout();
            }
            right_ui.add_sized([160.0, 44.0], egui::Label::new(account).truncate());
        }
        if layout != LayoutMode::Compact
            && let Some((text, intent)) = save_status
        {
            theme::bounded_badge(&mut right_ui, text, intent, 72.0);
        }
        if let Some((message, intent)) = runtime_status.as_ref() {
            status_message(&mut right_ui, message, *intent);
        } else if layout == LayoutMode::Compact
            && let Some((text, intent)) = save_status
        {
            let response = theme::bounded_badge(
                &mut right_ui,
                compact_status_text(self.save_status),
                intent,
                38.0,
            )
            .on_hover_text(text);
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
        }
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Application bar")
        });
    }

    fn application_menu_contents(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
        ui.set_min_width(theme::MENU_WIDTH);
        ui.menu_button("Navigation", |ui| self.navigation_menu_contents(ui));
        if self.work_view() {
            ui.separator();
            ui.menu_button("Workspace", |ui| self.workspace_menu_contents(ui, layout));
        }
        if let Some(account) = self
            .auth
            .account
            .as_ref()
            .map(|account| account.display_name.clone())
        {
            ui.separator();
            ui.label(RichText::new(account).strong());
            if ui
                .add_enabled(
                    !self.loading.logout,
                    egui::Button::new("Sign out").min_size(egui::vec2(ui.available_width(), 44.0)),
                )
                .clicked()
            {
                self.request_logout();
                ui.close();
            }
        }
    }

    fn workspace_menu_contents(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
        let panel_actions = [
            (
                "Workflow panel",
                labello_domain::UserAction::ToggleWorkflowPanel,
                self.drawer == Some(Drawer::Workflow),
            ),
            (
                "Inspector panel",
                labello_domain::UserAction::ToggleInspectorPanel,
                self.drawer == Some(Drawer::Inspector),
            ),
        ];
        for (label, action, selected) in panel_actions
            .into_iter()
            .filter(|_| layout != LayoutMode::Wide)
            .chain(std::iter::once((
                "Tutorial",
                labello_domain::UserAction::OpenTutorial,
                self.show_tutorial,
            )))
        {
            if ui
                .add(
                    egui::Button::new(label)
                        .selected(selected)
                        .shortcut_text(self.shortcut_text(ui.ctx(), action))
                        .min_size(egui::vec2(theme::MENU_WIDTH, 44.0)),
                )
                .clicked()
            {
                self.trigger_user_action(action);
                ui.close();
            }
        }
        if ui
            .add(
                egui::Button::new("Settings")
                    .shortcut_text(
                        self.shortcut_text(ui.ctx(), labello_domain::UserAction::OpenSettings),
                    )
                    .min_size(egui::vec2(theme::MENU_WIDTH, 44.0)),
            )
            .clicked()
        {
            self.open_shortcut_settings();
            ui.close();
        }
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
            let show_previous = self.previous_annotation_assignment.is_some()
                && !matches!(self.save_status, SaveStatus::Dirty | SaveStatus::Retry);
            if show_previous {
                if ui
                    .add_enabled(
                        self.runtime.api.is_some()
                            && !self.loading.saving
                            && !self.loading.image
                            && self.pending_transition.is_none(),
                        egui::Button::new("Previous"),
                    )
                    .on_hover_text("Return to the last skipped or submitted assignment.")
                    .clicked()
                {
                    self.trigger_user_action(labello_domain::UserAction::PreviousImage);
                }
            } else if ui
                .add_enabled(
                    ready && matches!(self.save_status, SaveStatus::Dirty | SaveStatus::Retry),
                    egui::Button::new("Save"),
                )
                .on_hover_text("Save edits and keep this assignment active.")
                .clicked()
            {
                self.trigger_user_action(labello_domain::UserAction::SaveAnnotations);
            }
            if theme::primary_button(ui, ready, egui::Button::new("Submit & next"))
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
        if self.view == AppView::Annotate {
            ui.menu_button("More actions", |ui| {
                if self.previous_annotation_assignment.is_some()
                    && matches!(self.save_status, SaveStatus::Dirty | SaveStatus::Retry)
                    && ui
                        .add_enabled(
                            ready,
                            egui::Button::new("Previous assignment").shortcut_text(
                                self.shortcut_text(
                                    ui.ctx(),
                                    labello_domain::UserAction::PreviousImage,
                                ),
                            ),
                        )
                        .clicked()
                {
                    self.trigger_user_action(labello_domain::UserAction::PreviousImage);
                    ui.close();
                }
                if ui
                    .add_enabled(
                        ready && !self.undo_stack.is_empty(),
                        egui::Button::new("Undo").shortcut_text(
                            self.shortcut_text(ui.ctx(), labello_domain::UserAction::UndoEdit),
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
                            self.shortcut_text(ui.ctx(), labello_domain::UserAction::RedoEdit),
                        ),
                    )
                    .clicked()
                {
                    self.trigger_user_action(labello_domain::UserAction::RedoEdit);
                    ui.close();
                }
            });
        }
    }

    pub(crate) fn compact_workspace_actions(&mut self, ui: &mut egui::Ui) {
        let ready = (self.assignment.is_some() || self.runtime.api.is_none())
            && !self.loading.saving
            && !self.loading.image
            && self.pending_transition.is_none();
        ui.horizontal_wrapped(|ui| {
            if self.view == AppView::Annotate
                && theme::primary_button(ui, ready, egui::Button::new("Submit & next")).clicked()
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
                                self.previous_annotation_assignment.is_some()
                                    && self.runtime.api.is_some()
                                    && !self.loading.saving
                                    && !self.loading.image
                                    && self.pending_transition.is_none(),
                                egui::Button::new("Previous assignment").shortcut_text(
                                    self.shortcut_text(
                                        ui.ctx(),
                                        labello_domain::UserAction::PreviousImage,
                                    ),
                                ),
                            )
                            .clicked()
                        {
                            self.trigger_user_action(labello_domain::UserAction::PreviousImage);
                            ui.close();
                        }
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
            theme::inline_message(
                ui,
                theme::Intent::Warning,
                "No enabled one-class workflows configured.",
            );
        }
        let ready =
            !self.loading.saving && !self.loading.image && self.pending_transition.is_none();
        for workflow in workflows {
            let selected = self.selected_task_id.as_ref() == Some(&workflow.task_id);
            let frame = theme::selected_card_frame(selected);
            frame.show(ui, |ui| {
                if ui
                    .add_enabled(
                        ready,
                        egui::Button::selectable(
                            selected,
                            RichText::new(workflow.label()).strong(),
                        ),
                    )
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
                theme::badge(
                    ui,
                    annotation_type_label(&workflow.annotation_type),
                    theme::Intent::Info,
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
            theme::empty_state(
                ui,
                "No active assignment",
                "Claim work to see its reservation and queue status.",
                None,
            );
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
        theme::compact_metric(ui, "Active annotations", active_count.to_string());
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
                        "Position: {:.0}% from left, {:.0}% from top\nSize: {:.0}% wide by {:.0}% high",
                        bbox.x * 100.0,
                        bbox.y * 100.0,
                        bbox.width * 100.0,
                        bbox.height * 100.0
                    ),
                    AnnotationGeometry::Skeleton(skeleton) => format!(
                        "Keypoints placed: {} of {}",
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
                    index + 1,
                    class_name,
                    geometry,
                )
            })
            .collect::<Vec<_>>();
        if objects.is_empty() {
            theme::empty_state(
                ui,
                "No objects yet",
                "Draw or accept an object to inspect it.",
                None,
            );
            return;
        }

        if self.selected_annotation.is_some()
            && !objects
                .iter()
                .any(|(annotation_id, ..)| Some(annotation_id) == self.selected_annotation.as_ref())
        {
            self.selected_annotation = None;
        }

        ui.separator();
        ui.label(RichText::new("Objects").strong());
        for (annotation_id, number, class_name, geometry) in objects {
            let selected = self.selected_annotation.as_ref() == Some(&annotation_id);
            theme::selected_card_frame(selected).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                let label = format!(
                    "Object {number} | {class_name}{}",
                    if selected { " | Selected" } else { "" }
                );
                if ui
                    .add_sized(
                        [ui.available_width(), 44.0],
                        egui::Button::selectable(selected, &label).truncate(),
                    )
                    .on_hover_text(format!(
                        "{label}\nPrevious: {} | Next: {}",
                        self.shortcut_text(
                            ui.ctx(),
                            labello_domain::UserAction::SelectPreviousObject,
                        ),
                        self.shortcut_text(ui.ctx(), labello_domain::UserAction::SelectNextObject,)
                    ))
                    .clicked()
                {
                    self.selected_annotation = Some(annotation_id.clone());
                }

                egui::CollapsingHeader::new(format!("Geometry details for Object {number}"))
                    .id_salt(annotation_id.as_str())
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(&geometry)
                                .monospace()
                                .color(theme::TEXT_MUTED),
                        );
                    });
            });
        }
        if self.selected_annotation.is_some()
            && theme::danger_button(
                ui,
                true,
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
            theme::compact_metric(
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

    fn review_phase(&self) -> (&'static str, String, &'static str) {
        let total = self
            .annotations
            .iter()
            .filter(|annotation| {
                !annotation.deleted && self.annotation_matches_selected_workflow(annotation)
            })
            .count();
        if self.review_index < total {
            (
                "Object review",
                format!("{} of {total}", self.review_index + 1),
                "The active object is highlighted on the canvas.",
            )
        } else {
            (
                "Final check",
                "Full image".to_string(),
                "Check for missed objects before completing this review.",
            )
        }
    }

    fn review_actions(&mut self, ui: &mut egui::Ui, show_primary_actions: bool) {
        let ready = self.assignment.is_some() && !self.loading.saving;
        if self.correction_draft.is_some() {
            self.correction_actions(ui, ready);
            return;
        }
        let (phase, value, explanation) = self.review_phase();
        theme::compact_metric(ui, phase, value);
        ui.label(explanation);
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
        if theme::primary_button(ui, ready, egui::Button::new(approve)).clicked() {
            self.request_review(ReviewDecision::Approved);
        }
        if theme::danger_button(ui, ready, egui::Button::new(reject)).clicked() {
            self.request_review(ReviewDecision::Rejected);
        }
    }

    fn correction_actions(&mut self, ui: &mut egui::Ui, ready: bool) {
        ui.separator();
        ui.heading("Correction mode");
        ui.label("Only the highlighted existing object can be edited.");

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
        ui.add_space(theme::SPACE_2);
        ui.label(RichText::new("Object").strong().color(theme::TEXT_MUTED));
        if let Some(keypoints) = skeleton_keypoints {
            ui.label("Edit only the highlighted skeleton on the canvas.");
            ui.add_space(theme::SPACE_2);
            ui.label(RichText::new("Keypoints").strong().color(theme::TEXT_MUTED));
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

        ui.add_space(theme::SPACE_2);
        ui.label(RichText::new("Reason").strong().color(theme::TEXT_MUTED));
        if let Some(draft) = self.correction_draft.as_mut() {
            let label = ui.label("Reason (optional)");
            ui.add_enabled_ui(ready, |ui| {
                theme::resizable_multiline_text_edit(
                    ui,
                    ui.make_persistent_id("correction-reason"),
                    &mut draft.reason,
                    2,
                    Some("What was corrected?"),
                )
                .labelled_by(label.id);
            });
        }

        let (can_undo, geometry_changed) = self
            .correction_draft
            .as_ref()
            .map(|draft| (!draft.geometry_history.is_empty(), draft.geometry_changed()))
            .unwrap_or_default();
        ui.add_space(theme::SPACE_2);
        ui.label(RichText::new("Actions").strong().color(theme::TEXT_MUTED));
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(ready && can_undo, egui::Button::new("Undo correction"))
                .clicked()
            {
                self.undo_correction();
            }
            if theme::danger_button(ui, ready, egui::Button::new("Discard correction")).clicked() {
                self.discard_correction();
            }
            if theme::primary_button(
                ui,
                ready && geometry_changed,
                egui::Button::new("Correct & finalize"),
            )
            .on_disabled_hover_text("Move, resize, or change a keypoint before finalizing.")
            .clicked()
            {
                self.request_correction();
            }
        });
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
        theme::compact_metric(ui, "Candidate annotations", candidates.to_string());
        if candidates == 0 {
            theme::inline_message(
                ui,
                theme::Intent::Warning,
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
        if theme::primary_button(ui, ready && has_candidates, egui::Button::new(accept)).clicked() {
            self.request_adjudication(AdjudicationDecision::AcceptAnnotation);
        }
        if theme::danger_button(ui, ready, egui::Button::new(correct)).clicked() {
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
                centered_scroll(ui, 1100.0, |ui| self.admin_view(ui, layout));
                return;
            }
            AppView::Stats => {
                centered_scroll(ui, 1100.0, |ui| self.stats_view(ui, layout));
                return;
            }
            AppView::Annotate | AppView::Review | AppView::Adjudicate => {}
        }
        if let Some(current) = self.current.clone() {
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
            let annotation_color = self
                .selected_class_id()
                .and_then(|class_id| {
                    self.classes
                        .iter()
                        .find(|class| &class.class_id == class_id)
                })
                .and_then(|class| parse_class_color(&class.color))
                .unwrap_or(theme::ANNOTATION);
            let action = show_canvas_styled(
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
                annotation_color,
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
            ui.add_space(((ui.available_height() - 160.0) * 0.5).max(0.0));
            let width = ui.available_width().min(520.0);
            let inset = ((ui.available_width() - width) * 0.5).max(0.0);
            ui.horizontal(|ui| {
                ui.add_space(inset);
                ui.vertical(|ui| {
                    ui.set_width(width);
                    if self.loading.dataset {
                        theme::inset_frame().show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Opening dataset").strong());
                            });
                            ui.label(
                                RichText::new("Loading workflows and dataset metadata.")
                                    .color(theme::TEXT_MUTED),
                            );
                        });
                    } else if self.loading.image {
                        theme::inset_frame().show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Loading assignment image").strong());
                            });
                            ui.label(
                                RichText::new("Decoding the image preview for the canvas.")
                                    .color(theme::TEXT_MUTED),
                            );
                        });
                    } else if let Some(error) = self.runtime.error.clone() {
                        let claimed = self.assignment.is_some();
                        let (title, retry) = if claimed {
                            ("Assignment image unavailable", "Retry image load")
                        } else {
                            ("Assignment unavailable", "Retry assignment")
                        };
                        let shortcut = self
                            .shortcut_text(ui.ctx(), labello_domain::UserAction::RetryImageLoad);
                        if theme::empty_state(
                            ui,
                            title,
                            &error,
                            Some(egui::Button::new(retry).shortcut_text(shortcut)),
                        ) {
                            self.retry_assignment_load();
                        }
                    } else {
                        let title = match self.view {
                            AppView::Annotate => "No annotation assignments",
                            AppView::Review => "No review assignments",
                            AppView::Adjudicate => "No adjudication assignments",
                            _ => "No assignments",
                        };
                        let shortcut = self
                            .shortcut_text(ui.ctx(), labello_domain::UserAction::RetryImageLoad);
                        if theme::empty_state(
                            ui,
                            title,
                            "No work is available right now. Retry to check again.",
                            Some(egui::Button::new("Retry image load").shortcut_text(shortcut)),
                        ) {
                            self.retry_assignment_load();
                        }
                    }
                });
            });
        }
    }

    pub(crate) fn overlays(&mut self, ctx: &egui::Context, layout: LayoutMode) {
        if self.runtime.persistence.recovery.is_some() {
            self.draft_recovery_modal(ctx);
            return;
        }
        if self.pending_transition.is_some() {
            self.transition_modal(ctx);
            return;
        }
        if self.admin_tools.confirm_discard {
            self.admin_discard_modal(ctx);
            return;
        }
        if self.show_settings {
            self.settings_modal(ctx);
            return;
        }
        if layout != LayoutMode::Wide && self.work_view() {
            let screen = ctx.content_rect();
            let compact = layout == LayoutMode::Compact;
            let width = if compact {
                (screen.width() - 48.0).max(240.0)
            } else {
                308.0_f32.min(screen.width() - 48.0)
            };
            let max_height = if compact {
                (screen.height() * 0.7)
                    .clamp(180.0, 560.0)
                    .min(screen.height() - 48.0)
            } else {
                (screen.height() - 48.0).max(180.0)
            };
            if let Some(drawer) = self.drawer {
                let (title, align, offset) = match (drawer, compact) {
                    (Drawer::Workflow, true) => (
                        "Workflow",
                        egui::Align2::CENTER_BOTTOM,
                        egui::vec2(0.0, -12.0),
                    ),
                    (Drawer::Workflow, false) => {
                        ("Workflow", egui::Align2::LEFT_CENTER, egui::vec2(12.0, 0.0))
                    }
                    (Drawer::Inspector, true) => (
                        "Inspector",
                        egui::Align2::CENTER_BOTTOM,
                        egui::vec2(0.0, -12.0),
                    ),
                    (Drawer::Inspector, false) => (
                        "Inspector",
                        egui::Align2::RIGHT_CENTER,
                        egui::vec2(-12.0, 0.0),
                    ),
                };
                let id = egui::Id::new("workspace-drawer");
                let area = egui::Modal::default_area(id)
                    .anchor(align, offset)
                    .default_width(width)
                    .constrain_to(screen);
                let mut close = false;
                let response = theme::modal(ctx, id).area(area).show(ctx, |ui| {
                    ui.set_width(width);
                    ui.set_max_height(max_height);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let button =
                                ui.add(egui::Button::new("Close").min_size(egui::vec2(64.0, 44.0)));
                            button.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    true,
                                    format!("Close {title}"),
                                )
                            });
                            close = button.clicked();
                        });
                    });
                    egui::ScrollArea::vertical()
                        .max_height((max_height - 54.0).max(80.0))
                        .show(ui, |ui| match drawer {
                            Drawer::Workflow => self.task_panel(ui),
                            Drawer::Inspector => self.right_panel(ui, false),
                        });
                });
                response.response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Window, true, title)
                });
                if close || response.should_close() {
                    self.drawer = None;
                }
            }
        }
        self.tutorial_overlay(ctx);
    }

    pub(crate) fn workspace_context_bar(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
        let current = self.current.clone();
        let workflow = self.selected_workflow().map(|workflow| workflow.label());
        let view = self.view;
        let loading_image = self.loading.image;
        let has_assignment = self.assignment.is_some();
        let short = Self::short_viewport(ui.ctx().content_rect().size());
        let review_phase = if view == AppView::Review && current.is_some() {
            if self.correction_draft.is_some() {
                Some("Correction mode".to_string())
            } else {
                let (phase, value, _) = self.review_phase();
                Some(if phase == "Final check" {
                    phase.to_string()
                } else {
                    format!("Object {value}")
                })
            }
        } else {
            None
        };
        let add_summary = |ui: &mut egui::Ui, filename_width: f32| {
            ui.label(
                RichText::new(view_label(view))
                    .strong()
                    .color(theme::ACCENT),
            );
            if let Some(current) = current.as_ref() {
                if filename_width > 0.0 {
                    ui.separator();
                    ui.add_sized(
                        [filename_width, 44.0],
                        egui::Label::new(RichText::new(&current.image.file_name).strong())
                            .truncate(),
                    )
                    .on_hover_text(&current.image.file_name);
                }
                if layout == LayoutMode::Wide {
                    ui.add_sized(
                        [82.0, 44.0],
                        egui::Label::new(
                            RichText::new(format!(
                                "{} x {}",
                                current.image.width, current.image.height
                            ))
                            .color(theme::MUTED),
                        ),
                    );
                }
            } else if loading_image {
                ui.spinner();
                ui.label("Loading assignment...");
            } else if has_assignment {
                ui.label(RichText::new("Preview unavailable").color(theme::WARNING));
            } else {
                ui.label(RichText::new("No active assignment").color(theme::TEXT_MUTED));
            }
        };
        let stack_controls = !short
            && (layout == LayoutMode::Compact
                || (layout == LayoutMode::Medium
                    && view != AppView::Annotate
                    && current.is_some()));
        let response = if short && current.is_some() {
            ui.horizontal(|ui| self.canvas_controls(ui))
        } else if stack_controls {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.horizontal(|ui| {
                    ui.set_min_height(44.0);
                    add_summary(ui, 50.0);
                    if current.is_some() {
                        if let Some(phase) = review_phase.as_ref() {
                            theme::bounded_badge(ui, phase, theme::Intent::Info, 110.0);
                        } else if let Some(workflow) = workflow.as_ref() {
                            theme::bounded_badge(ui, workflow, theme::Intent::Accent, 90.0);
                        }
                    }
                });
                if current.is_some() {
                    self.canvas_controls(ui);
                }
            })
        } else {
            ui.horizontal(|ui| {
                add_summary(
                    ui,
                    if short {
                        0.0
                    } else if layout == LayoutMode::Wide {
                        160.0
                    } else {
                        60.0
                    },
                );
                if current.is_some()
                    && let Some(workflow) = workflow.as_ref()
                {
                    theme::bounded_badge(
                        ui,
                        workflow,
                        theme::Intent::Accent,
                        if layout == LayoutMode::Wide {
                            120.0
                        } else {
                            70.0
                        },
                    );
                }
                if let Some(phase) = review_phase.as_ref() {
                    theme::bounded_badge(
                        ui,
                        phase,
                        theme::Intent::Info,
                        if layout == LayoutMode::Wide {
                            120.0
                        } else {
                            100.0
                        },
                    );
                }
                if current.is_some() {
                    self.canvas_controls(ui);
                }
                if layout == LayoutMode::Wide {
                    ui.separator();
                    self.workspace_actions(ui, layout);
                }
            })
        };
        response.response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Workspace context bar")
        });
    }

    fn canvas_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let pan_shortcut =
                self.shortcut_text(ui.ctx(), labello_domain::UserAction::TogglePanMode);
            let pan = egui::Button::new("Pan")
                .selected(self.canvas.pan_mode())
                .min_size(egui::vec2(52.0, 44.0));
            if ui
                .add_enabled(self.canvas.can_pan(), pan)
                .on_disabled_hover_text("Zoom in before enabling Pan mode.")
                .on_hover_text(format!("Pan ({pan_shortcut}). Space or middle-drag."))
                .clicked()
            {
                self.trigger_user_action(labello_domain::UserAction::TogglePanMode);
            }

            let zoom_out_shortcut =
                self.shortcut_text(ui.ctx(), labello_domain::UserAction::ZoomOut);
            let can_zoom_out = self.canvas.can_zoom_out();
            let zoom_out = egui::Button::new("−").min_size(egui::vec2(44.0, 44.0));
            let zoom_out_response = ui
                .add_enabled(can_zoom_out, zoom_out)
                .on_disabled_hover_text("The image is already fitted.")
                .on_hover_text(format!("Zoom out ({zoom_out_shortcut}). Scroll or pinch."));
            zoom_out_response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, can_zoom_out, "Zoom out")
            });
            if zoom_out_response.clicked() {
                self.trigger_user_action(labello_domain::UserAction::ZoomOut);
            }

            ui.add_sized(
                [48.0, 44.0],
                egui::Label::new(format!("{:.0}%", self.canvas.current_zoom() * 100.0)),
            );

            let zoom_in_shortcut = self.shortcut_text(ui.ctx(), labello_domain::UserAction::ZoomIn);
            let can_zoom_in = self.canvas.can_zoom_in();
            let zoom_in = egui::Button::new("+").min_size(egui::vec2(44.0, 44.0));
            let zoom_in_response = ui
                .add_enabled(can_zoom_in, zoom_in)
                .on_disabled_hover_text("Maximum zoom reached.")
                .on_hover_text(format!("Zoom in ({zoom_in_shortcut}). Scroll or pinch."));
            zoom_in_response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, can_zoom_in, "Zoom in")
            });
            if zoom_in_response.clicked() {
                self.trigger_user_action(labello_domain::UserAction::ZoomIn);
            }

            let fit_shortcut = self.shortcut_text(ui.ctx(), labello_domain::UserAction::FitImage);
            let fit = egui::Button::new("Fit").min_size(egui::vec2(44.0, 44.0));
            if ui
                .add(fit)
                .on_hover_text(format!("Fit ({fit_shortcut}). Or double-click canvas."))
                .clicked()
            {
                self.trigger_user_action(labello_domain::UserAction::FitImage);
            }
        });
    }

    fn tutorial_overlay(&mut self, ctx: &egui::Context) {
        if !self.show_tutorial || !self.work_view() {
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
        let layout = LayoutMode::for_width(screen.width());
        let shell_height = 56.0 + self.workspace_context_height(layout, screen.size());
        let action_height = if layout == LayoutMode::Wide {
            0.0
        } else {
            self.workspace_actions_height(layout, screen.size())
        };
        let workspace = egui::Rect::from_min_max(
            egui::pos2(screen.left(), screen.top() + shell_height),
            egui::pos2(screen.right(), screen.bottom() - action_height),
        );
        let mut open = true;
        egui::Window::new("Tutorial")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(
                egui::Align2::RIGHT_TOP,
                egui::vec2(-12.0, shell_height + 12.0),
            )
            .max_width((workspace.width() - 24.0).clamp(240.0, 420.0))
            .max_height((workspace.height() - 24.0).clamp(80.0, 560.0))
            .constrain_to(workspace)
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
        let response = theme::modal(ctx, egui::Id::new("draft-recovery-modal")).show(ctx, |ui| {
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
                            if theme::primary_button(
                                ui,
                                true,
                                egui::Button::new("Recover draft"),
                            )
                            .clicked()
                            {
                                self.recover_browser_draft();
                            }
                            if theme::danger_button(
                                ui,
                                true,
                                egui::Button::new("Discard draft"),
                            )
                            .clicked()
                            {
                                self.discard_browser_draft();
                            }
                        });
                    }
                    crate::persistence::DraftValidation::Expired(message)
                    | crate::persistence::DraftValidation::Conflict(message) => {
                        theme::inline_message(ui, theme::Intent::Warning, message);
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
        response.response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Window, true, "Draft recovery dialog")
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
        let discards_edits = matches!(
            pending,
            PendingTransition::NextAssignment | PendingTransition::PreviousAssignment(_)
        ) && self.view == AppView::Annotate
            && matches!(self.save_status, SaveStatus::Dirty | SaveStatus::Retry);
        if pending == PendingTransition::NextAssignment && !discards_edits {
            return;
        }
        let modal_title = if discards_edits {
            "Unsaved annotation changes"
        } else {
            "Switch active assignment?"
        };
        let response =
            theme::modal(ctx, egui::Id::new("assignment-transition-modal")).show(ctx, |ui| {
                ui.set_max_width((ctx.content_rect().width() - 48.0).clamp(240.0, 560.0));
                ui.heading(modal_title);
                ui.label(format!("Current workflow: {current}"));
                ui.label(format!("Pending destination: {destination}"));
                if discards_edits {
                    theme::inline_message(
                        ui,
                        theme::Intent::Warning,
                        "Skipping now will discard annotation changes that have not been saved.",
                    );
                }
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    if self.view == AppView::Annotate
                        && theme::primary_button(
                            ui,
                            !self.loading.saving,
                            egui::Button::new("Submit and switch"),
                        )
                        .clicked()
                    {
                        self.submit_pending_transition();
                    }
                    if theme::danger_button(
                        ui,
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
                    if theme::quiet_button(ui, !self.loading.saving, egui::Button::new("Cancel"))
                        .clicked()
                    {
                        self.cancel_pending_transition();
                    }
                });
            });
        response.response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Window,
                true,
                "Assignment transition dialog",
            )
        });
        if response.should_close() {
            self.cancel_pending_transition();
        }
    }

    fn admin_discard_modal(&mut self, ctx: &egui::Context) {
        let mut discard = false;
        let response = theme::modal(ctx, egui::Id::new("discard-admin-changes")).show(ctx, |ui| {
            ui.set_max_width((ctx.content_rect().width() - 48.0).clamp(240.0, 480.0));
            ui.heading("Discard staged Admin changes?");
            ui.label("All unsaved configuration and permission edits will be lost.");
            ui.horizontal_wrapped(|ui| {
                if theme::danger_button(ui, true, egui::Button::new("Discard changes")).clicked() {
                    discard = true;
                }
                if theme::quiet_button(ui, true, egui::Button::new("Keep editing")).clicked() {
                    self.admin_tools.confirm_discard = false;
                }
            });
        });
        response.response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Window,
                true,
                "Discard staged Admin changes",
            )
        });
        if discard {
            self.datasets.admin_config = self.datasets.admin_baseline.clone();
            self.datasets.users = self.datasets.users_baseline.clone();
            self.clear_admin_draft();
            self.admin_tools.confirm_discard = false;
            self.runtime.notice = Some("Staged admin changes discarded".to_string());
        } else if response.should_close() {
            self.admin_tools.confirm_discard = false;
        }
    }

    fn transition_label(&self, transition: &PendingTransition) -> String {
        match transition {
            PendingTransition::NextAssignment => "Next assignment".to_string(),
            PendingTransition::PreviousAssignment(_) => "Previous assignment".to_string(),
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
        if self.shortcut_settings.confirm_discard {
            self.shortcut_discard_modal(ctx);
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
            let captured = ctx.input_mut(|input| {
                let index = input.events.iter().rposition(|event| {
                    matches!(
                        event,
                        egui::Event::Key {
                            pressed: true,
                            repeat: false,
                            ..
                        }
                    )
                })?;
                match input.events.remove(index) {
                    egui::Event::Key { key, modifiers, .. } => Some((key, modifiers)),
                    _ => None,
                }
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
        let screen = ctx.content_rect();
        let short = Self::short_viewport(screen.size());
        let max_height = (screen.height() - 48.0).max(180.0);
        let width = (screen.width() - 48.0).clamp(240.0, 720.0);
        let mut record = None;
        let mut reset_action = None;
        let mut save = false;
        let mut cancel = false;
        let mut reset_all = false;
        let response = theme::modal(ctx, egui::Id::new("settings-modal")).show(ctx, |ui| {
            ui.set_width(width);
            ui.set_max_height(max_height);
            let mut contents = |ui: &mut egui::Ui| {
                ui.heading("Keyboard shortcuts");
                ui.label(
                    RichText::new("Choose an action, then press its new key combination.")
                        .color(theme::MUTED),
                );
                if let Some(error) = &self.shortcut_settings.error {
                    theme::inline_message(
                        ui,
                        theme::Intent::Error,
                        format!("Could not save shortcuts: {error}"),
                    );
                }
                ui.add_space(6.0);
                let search_label = ui.label("Search actions");
                ui.add_sized(
                    [ui.available_width(), theme::COMPACT_TEXT_FIELD_HEIGHT],
                    theme::singleline_text_edit(&mut self.shortcut_settings.search)
                        .hint_text("Search by action or category"),
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
                let compact_footer = ui.available_width() < 420.0;
                let scroll_height = if compact_footer {
                    (screen.height() - 500.0).clamp(64.0, 520.0)
                } else if screen.height() < 700.0 {
                    (screen.height() - 380.0).clamp(120.0, 520.0)
                } else {
                    (screen.height() - 300.0).clamp(180.0, 520.0)
                };
                let mut action_list = |ui: &mut egui::Ui| {
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
                                                !self.loading.keybindings,
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
                                            egui::WidgetInfo::selected(
                                                egui::WidgetType::Button,
                                                !self.loading.keybindings,
                                                recording,
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
                                ui.label(
                                    RichText::new("Conflicts in this context").color(theme::DANGER),
                                );
                            }
                        });
                        ui.add_space(4.0);
                    }
                };
                if short {
                    action_list(ui);
                } else {
                    egui::ScrollArea::vertical()
                        .max_height(scroll_height)
                        .show(ui, |ui| action_list(ui));
                }
                if !conflicts.is_empty() {
                    theme::inline_message(
                        ui,
                        theme::Intent::Error,
                        format!(
                            "Resolve {} shortcut conflict(s) before saving.",
                            conflicts.len()
                        ),
                    );
                }
                let dirty = self.shortcut_settings.draft != self.shortcut_settings.baseline;
                let mut restore_defaults = |ui: &mut egui::Ui| {
                    if ui
                        .add_enabled(
                            !self.loading.keybindings,
                            egui::Button::new("Restore all defaults"),
                        )
                        .clicked()
                    {
                        reset_all = true;
                    }
                };
                let mut decision_actions = |ui: &mut egui::Ui| {
                    if theme::primary_button(
                        ui,
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
                    if theme::quiet_button(
                        ui,
                        !self.loading.keybindings,
                        egui::Button::new("Cancel"),
                    )
                    .clicked()
                    {
                        cancel = true;
                    }
                };
                if compact_footer {
                    ui.vertical(|ui| {
                        restore_defaults(ui);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            decision_actions(ui);
                        });
                    });
                } else {
                    ui.horizontal_wrapped(|ui| {
                        restore_defaults(ui);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            decision_actions(ui);
                        });
                    });
                }
                ui.horizontal_wrapped(|ui| {
                    if dirty && conflicts.is_empty() {
                        ui.label(RichText::new("Unsaved changes").color(theme::AMBER));
                    }
                });
            };
            if short {
                egui::ScrollArea::vertical()
                    .id_salt("settings-modal-scroll")
                    .max_height(max_height)
                    .show(ui, |ui| contents(ui));
            } else {
                contents(ui);
            }
        });
        response
            .response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Window, true, "Settings"));
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
        if cancel || (!self.loading.keybindings && response.should_close()) {
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
    }

    fn shortcut_discard_modal(&mut self, ctx: &egui::Context) {
        let response =
            theme::modal(ctx, egui::Id::new("discard-shortcut-settings")).show(ctx, |ui| {
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
        response.response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Window, true, "Discard shortcut changes")
        });
        if response.should_close() {
            self.shortcut_settings.confirm_discard = false;
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
            theme::empty_state(
                ui,
                "No prelabel suggestions",
                "This image has no suggestions for the selected workflow.",
                None,
            );
        }
        for suggestion in &prelabels {
            let selected = self.selected_prelabel.as_ref() == Some(&suggestion.suggestion_id);
            let frame = theme::selected_card_frame(selected);
            frame.show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .selectable_label(selected, suggestion.class_id.to_string())
                        .clicked()
                    {
                        self.selected_prelabel = Some(suggestion.suggestion_id.clone());
                    }
                    theme::badge(
                        ui,
                        &format!("{:.0}%", suggestion.confidence * 100.0),
                        theme::Intent::Accent,
                    );
                    if theme::primary_button(ui, !self.loading.saving, egui::Button::new("Accept"))
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
                    if theme::danger_button(ui, !self.loading.saving, egui::Button::new("Discard"))
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
    let available_width = (ui.available_width() - ui.spacing().scroll.allocated_width()).max(0.0);
    egui::ScrollArea::vertical().show(ui, |ui| {
        let width = available_width.min(max_width);
        let inset = ((available_width - width) * 0.5).max(0.0);
        ui.horizontal(|ui| {
            ui.add_space(inset);
            ui.vertical(|ui| {
                ui.set_width(width);
                add_contents(ui);
            });
        });
    });
}

fn status_message(ui: &mut egui::Ui, message: &str, intent: theme::Intent) {
    ui.add_sized(
        [ui.available_width().min(520.0), 24.0],
        egui::Label::new(RichText::new(message).color(intent.color())).truncate(),
    );
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
        UserAction::PreviousImage => "Previous assignment",
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
        | UserAction::SkipAssignment
        | UserAction::PreviousImage => "Assignment",
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
        UserAction::SelectBoundingBoxTool
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
        UserAction::PreviousImage => "Return to the last skipped or submitted assignment.",
        UserAction::SelectBoundingBoxTool
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

fn parse_class_color(value: &str) -> Option<egui::Color32> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    Some(egui::Color32::from_rgb(
        (value >> 16) as u8,
        (value >> 8) as u8,
        value as u8,
    ))
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

fn compact_status_text(status: SaveStatus) -> &'static str {
    match status {
        SaveStatus::Idle => "Idle",
        SaveStatus::Dirty => "Edit",
        SaveStatus::Saved => "Done",
        SaveStatus::Saving => "Wait",
        SaveStatus::Retry => "Retry",
    }
}

fn status_intent(status: SaveStatus) -> theme::Intent {
    match status {
        SaveStatus::Idle => theme::Intent::Neutral,
        SaveStatus::Dirty => theme::Intent::Warning,
        SaveStatus::Saved => theme::Intent::Success,
        SaveStatus::Saving => theme::Intent::Info,
        SaveStatus::Retry => theme::Intent::Error,
    }
}

fn keypoint_state_label(state: &KeypointState) -> &'static str {
    match state {
        KeypointState::Visible => "visible",
        KeypointState::Hidden => "hidden",
        KeypointState::Absent => "absent",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_colors_parse_with_a_safe_fallback_boundary() {
        assert_eq!(
            parse_class_color("#5eead4"),
            Some(egui::Color32::from_rgb(94, 234, 212))
        );
        assert_eq!(parse_class_color("5eead4"), None);
        assert_eq!(parse_class_color("#invalid"), None);
    }
}
