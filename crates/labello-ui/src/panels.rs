use eframe::egui::{self, RichText};
use labello_domain::{AdjudicationDecision, AnnotationGeometry, KeypointState, ReviewDecision};

use crate::{
    app::{
        AppView, Drawer, LabelloApp, LayoutMode, PendingTransition, SaveStatus, Tool,
        annotation_type_label,
    },
    theme,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppBarAction {
    Setup,
    Tutorial,
    Settings,
    SignOut,
}

impl AppBarAction {
    fn label(self) -> &'static str {
        match self {
            Self::Setup => "Setup",
            Self::Tutorial => "Tutorial",
            Self::Settings => "Settings",
            Self::SignOut => "Sign out",
        }
    }

    fn accessible_label(self) -> &'static str {
        match self {
            Self::Setup => "Open setup",
            Self::Tutorial => "Open tutorial",
            Self::Settings => "Open settings",
            Self::SignOut => "Sign out",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Setup => "#",
            Self::Tutorial => "?",
            Self::Settings => "⚙",
            Self::SignOut => "↪",
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            Self::Setup => "Open dataset setup.",
            Self::Tutorial => "Show or hide workflow instructions.",
            Self::Settings => "Open keyboard shortcut settings.",
            Self::SignOut => "Sign out of Labello.",
        }
    }
}

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
        let runtime_status = if let Some(error) = &self.runtime.storage_error {
            Some(("Error", error.clone(), theme::Intent::Error))
        } else if let Some(error) = &self.runtime.error {
            Some(("Error", error.clone(), theme::Intent::Error))
        } else {
            self.runtime
                .notice
                .clone()
                .map(|notice| ("Update", notice, theme::Intent::Success))
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

        let right_rect = egui::Rect::from_min_max(
            egui::pos2(dataset_rect.right() + side_gap, bar_rect.top()),
            bar_rect.max,
        );
        let mut right_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(right_rect)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );

        let mut actions = vec![
            AppBarAction::Setup,
            AppBarAction::Settings,
            AppBarAction::SignOut,
        ];
        if self.work_view() && self.selected_task().is_some() {
            actions.insert(1, AppBarAction::Tutorial);
        }
        if account.is_none() {
            actions.retain(|action| *action != AppBarAction::SignOut);
        }

        let status_width = if layout == LayoutMode::Compact {
            64.0
        } else {
            76.0
        };
        let spacing = ui.spacing().item_spacing.x;
        let mut right_remaining = (right_rect.width() - status_width).max(0.0);
        let mut visible_action_count = 0;
        for _ in &actions {
            let required = 44.0 + spacing;
            if right_remaining + 0.5 < required {
                break;
            }
            right_remaining -= required;
            visible_action_count += 1;
        }
        let show_account = account.is_some()
            && visible_action_count == actions.len()
            && right_remaining >= 96.0 + spacing;
        let hidden_account = account.is_some() && !show_account;
        let hidden_actions = actions[visible_action_count..].to_vec();
        let panel_actions_in_overflow = layout != LayoutMode::Wide && self.work_view();

        let destinations = self.primary_navigation_destinations();
        let navigation_width = |label: &str| 30.0 + label.chars().count() as f32 * 7.5;
        let total_navigation_width = destinations
            .iter()
            .map(|(_, label)| navigation_width(label))
            .sum::<f32>()
            + spacing * destinations.len().saturating_sub(1) as f32;
        let mut overflow_needed = !hidden_actions.is_empty()
            || hidden_account
            || panel_actions_in_overflow
            || total_navigation_width > left_rect.width();
        let available_navigation_width =
            (left_rect.width() - if overflow_needed { 44.0 + spacing } else { 0.0 }).max(0.0);
        let mut direct_count = 0;
        let mut used_width = 0.0;
        for (_, label) in &destinations {
            let width = navigation_width(label);
            let required = width + if direct_count == 0 { 0.0 } else { spacing };
            if used_width + required > available_navigation_width + 0.5 {
                break;
            }
            used_width += required;
            direct_count += 1;
        }
        if direct_count < destinations.len() {
            overflow_needed = true;
        }
        let hidden_destinations = destinations[direct_count..].to_vec();

        if overflow_needed {
            let overflow_button =
                egui::Button::new(RichText::new("...").size(18.0)).min_size(egui::vec2(44.0, 44.0));
            let overflow_response = left_ui.add(overflow_button);
            let menu_height = (left_ui.ctx().content_rect().height() - 80.0).max(132.0);
            egui::Popup::menu(&overflow_response).show(|ui| {
                egui::ScrollArea::vertical()
                    .id_salt("application-overflow-scroll")
                    .max_height(menu_height)
                    .show(ui, |ui| {
                        self.application_overflow_contents(
                            ui,
                            &hidden_destinations,
                            &hidden_actions,
                            panel_actions_in_overflow,
                            hidden_account.then_some(account.as_deref()).flatten(),
                        );
                    });
            });
            let overflow_response =
                overflow_response.on_hover_text("Open additional application actions.");
            overflow_response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    true,
                    "More application actions",
                )
            });
        }
        for (view, label) in destinations.into_iter().take(direct_count) {
            if left_ui
                .add_sized(
                    [navigation_width(label), 44.0],
                    egui::Button::selectable(self.view == view, label),
                )
                .clicked()
            {
                self.open_view(view);
            }
        }

        for action in actions.iter().take(visible_action_count).rev() {
            self.app_bar_icon_button(&mut right_ui, *action);
        }
        if show_account && let Some(account) = account.as_ref() {
            right_ui
                .add_sized([96.0, 44.0], egui::Label::new(account).truncate())
                .on_hover_text(account);
        }
        self.status_pill(&mut right_ui, runtime_status, status_width, layout);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Application bar")
        });
    }

    fn application_overflow_contents(
        &mut self,
        ui: &mut egui::Ui,
        hidden_destinations: &[(AppView, &'static str)],
        hidden_actions: &[AppBarAction],
        include_panel_actions: bool,
        hidden_account: Option<&str>,
    ) {
        ui.set_min_width(theme::MENU_WIDTH);
        for (view, label) in hidden_destinations {
            if ui
                .add(
                    egui::Button::selectable(self.view == *view, *label)
                        .min_size(egui::vec2(theme::MENU_WIDTH, 44.0)),
                )
                .clicked()
            {
                self.open_view(*view);
                ui.close();
            }
        }
        if include_panel_actions {
            self.workspace_overflow_actions(ui);
        }
        for action in hidden_actions {
            if ui
                .add_enabled(
                    *action != AppBarAction::SignOut || !self.loading.logout,
                    egui::Button::new(action.label()).min_size(egui::vec2(theme::MENU_WIDTH, 44.0)),
                )
                .clicked()
            {
                self.perform_app_bar_action(*action);
                ui.close();
            }
        }
        if let Some(account) = hidden_account {
            ui.separator();
            ui.add_sized(
                [theme::MENU_WIDTH, 44.0],
                egui::Label::new(RichText::new(account).strong()).truncate(),
            );
        }
    }

    fn workspace_overflow_actions(&mut self, ui: &mut egui::Ui) {
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
        for (label, action, selected) in panel_actions {
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
    }

    fn app_bar_icon_button(&mut self, ui: &mut egui::Ui, action: AppBarAction) {
        let enabled = action != AppBarAction::SignOut || !self.loading.logout;
        let selected = match action {
            AppBarAction::Setup => self.view == AppView::Setup,
            AppBarAction::Tutorial => self.show_tutorial,
            AppBarAction::Settings => self.show_settings,
            AppBarAction::SignOut => false,
        };
        let response = ui
            .add_enabled_ui(enabled, |ui| {
                ui.add_sized(
                    [44.0, 44.0],
                    egui::Button::new(RichText::new(action.icon()).size(18.0)).selected(selected),
                )
            })
            .inner
            .on_hover_text(action.tooltip());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, action.accessible_label())
        });
        if response.clicked() {
            self.perform_app_bar_action(action);
        }
    }

    fn perform_app_bar_action(&mut self, action: AppBarAction) {
        match action {
            AppBarAction::Setup => self.open_view(AppView::Setup),
            AppBarAction::Tutorial => {
                self.trigger_user_action(labello_domain::UserAction::OpenTutorial)
            }
            AppBarAction::Settings => self.open_shortcut_settings(),
            AppBarAction::SignOut => self.request_logout(),
        }
    }

    fn status_pill(
        &mut self,
        ui: &mut egui::Ui,
        runtime_status: Option<(&'static str, String, theme::Intent)>,
        width: f32,
        layout: LayoutMode,
    ) {
        let (text, detail, intent, accessible_label) = if self.work_view() {
            let full = status_text(self.save_status);
            let text = if layout == LayoutMode::Compact {
                compact_status_text(self.save_status)
            } else {
                full
            };
            let mut detail = format!("Annotation status: {full}");
            let mut accessible_label = format!("Status: {full}");
            let mut intent = status_intent(self.save_status);
            if let Some((_, runtime_detail, runtime_intent)) = runtime_status {
                let prefix = if matches!(runtime_intent, theme::Intent::Error) {
                    "Error"
                } else {
                    "Update"
                };
                detail.push_str(&format!("\n{prefix}: {runtime_detail}"));
                accessible_label.push_str(&format!(". {prefix}: {runtime_detail}"));
                if matches!(runtime_intent, theme::Intent::Error) {
                    intent = theme::Intent::Error;
                }
            }
            (text, detail, intent, accessible_label)
        } else if let Some((text, detail, intent)) = runtime_status {
            let prefix = if matches!(intent, theme::Intent::Error) {
                "Status error"
            } else {
                "Status update"
            };
            (text, detail.clone(), intent, format!("{prefix}: {detail}"))
        } else {
            (
                "Ready",
                "Labello is ready.".to_string(),
                theme::Intent::Neutral,
                "Status: Ready".to_string(),
            )
        };
        let color = intent.color();
        let response = ui
            .add_sized(
                [width, 44.0],
                egui::Button::new(RichText::new(text).color(color).strong())
                    .fill(egui::Color32::from_rgba_unmultiplied(
                        color.r(),
                        color.g(),
                        color.b(),
                        36,
                    ))
                    .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.55)))
                    .corner_radius(12.0),
            )
            .on_hover_text(&detail);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, accessible_label.clone())
        });
        egui::Popup::menu(&response).show(|ui| {
            ui.set_max_width(320.0);
            ui.label(detail);
        });
    }

    pub(crate) fn workspace_actions(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
        if !self.work_view() {
            return;
        }
        if self.manual_migration_active() {
            if layout != LayoutMode::Wide && ui.button("Migration controls").clicked() {
                self.drawer = Some(Drawer::Inspector);
            }
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
        if self.manual_migration_active() {
            if ui.button("Migration controls").clicked() {
                self.drawer = Some(Drawer::Inspector);
            }
            return;
        }
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
        if self.manual_migration_active() {
            self.manual_migration_actions(ui);
            return;
        }
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
                centered_scroll(ui, 1100.0, |ui| self.setup_view(ui, layout));
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
        self.workspace_canvas(ui);
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
                (screen.width() - 96.0).max(240.0)
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
