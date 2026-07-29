impl LabelloApp {
    pub(crate) fn workspace_actions(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
        if !self.work_view() {
            return;
        }
        if self.manual_migration_active() {
            if self.view == AppView::Review && layout != LayoutMode::Wide {
                self.responsive_migration_review_actions(ui);
            } else {
                self.migration_workspace_actions(ui, false);
            }
            return;
        }
        let ready = (self.work.assignment.is_some() || self.runtime.api.is_none())
            && !self.loading.saving
            && !self.loading.image
            && self.work.pending_transition.is_none();
        if self.view == AppView::Annotate {
            let show_previous = self.work.previous_annotation_assignment.is_some()
                && !matches!(self.work.save_status, SaveStatus::Dirty | SaveStatus::Retry);
            if show_previous {
                if ui
                    .add_enabled(
                        self.runtime.api.is_some()
                            && !self.loading.saving
                            && !self.loading.image
                            && self.work.pending_transition.is_none(),
                        egui::Button::new("Previous"),
                    )
                    .on_hover_text("Return to the last skipped or submitted assignment.")
                    .clicked()
                {
                    self.trigger_user_action(labello_domain::UserAction::PreviousImage);
                }
            } else if ui
                .add_enabled(
                    ready && matches!(self.work.save_status, SaveStatus::Dirty | SaveStatus::Retry),
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
            && self.work.correction_draft.is_none()
        {
            let review_layout = self.compact_review_row_layout(ui);
            let add_contents = |ui: &mut egui::Ui| {
                self.review_decision_buttons(ui, review_layout.shortcut_decisions, true);
            };
            if review_layout.allow_wrap {
                ui.horizontal_wrapped(add_contents);
            } else {
                ui.horizontal(add_contents);
            }
            return;
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
                if self.work.previous_annotation_assignment.is_some()
                    && matches!(self.work.save_status, SaveStatus::Dirty | SaveStatus::Retry)
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
                        ready && !self.work.undo_stack.is_empty(),
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
                        ready && !self.work.redo_stack.is_empty(),
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
            if self.view == AppView::Review {
                self.responsive_migration_review_actions(ui);
                return;
            }
            ui.horizontal_wrapped(|ui| {
                self.migration_workspace_actions(ui, true);
            });
            return;
        }
        if self.view == AppView::Review && self.work.correction_draft.is_none() {
            let review_layout = self.compact_review_row_layout(ui);
            let add_contents = |ui: &mut egui::Ui| {
                self.review_decision_buttons(ui, review_layout.shortcut_decisions, true);
            };
            if review_layout.allow_wrap {
                ui.horizontal_wrapped(add_contents);
            } else {
                ui.horizontal(add_contents);
            }
            return;
        }
        let ready = (self.work.assignment.is_some() || self.runtime.api.is_none())
            && !self.loading.saving
            && !self.loading.image
            && self.work.pending_transition.is_none();
        let add_contents = |ui: &mut egui::Ui| {
            if self.view == AppView::Annotate
                && theme::primary_button(ui, ready, egui::Button::new("Submit & next")).clicked()
            {
                self.trigger_user_action(labello_domain::UserAction::NextImage);
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
                                self.work.previous_annotation_assignment.is_some()
                                    && self.runtime.api.is_some()
                                    && !self.loading.saving
                                    && !self.loading.image
                                    && self.work.pending_transition.is_none(),
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
                                ready && !self.work.undo_stack.is_empty(),
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
                                ready && !self.work.redo_stack.is_empty(),
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
                                        self.work.save_status,
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
        };
        ui.horizontal(add_contents);
    }

    fn responsive_migration_review_actions(&mut self, ui: &mut egui::Ui) {
        let layout = self.compact_review_row_layout(ui);
        let target = self.current_migration_review_target();
        let add_contents = |ui: &mut egui::Ui| {
            if let Some((task_id, target)) = target {
                self.migration_review_buttons(
                    ui,
                    task_id,
                    target,
                    true,
                    layout.shortcut_decisions,
                );
            }
        };
        if layout.allow_wrap {
            ui.horizontal_wrapped(add_contents);
        } else {
            ui.horizontal(add_contents);
        }
    }

    fn compact_review_row_layout(&self, ui: &egui::Ui) -> CompactReviewRowLayout {
        let full_decisions = [
            text_button_width(ui, "Accept"),
            text_button_width(ui, "Reject"),
        ];
        let shortcut_decisions = [
            text_button_width(
                ui,
                &shortcut_button_label(
                    &self.shortcut_text(
                        ui.ctx(),
                        labello_domain::UserAction::AcceptReviewObject,
                    ),
                    "Accept",
                ),
            ),
            text_button_width(
                ui,
                &shortcut_button_label(
                    &self.shortcut_text(
                        ui.ctx(),
                        labello_domain::UserAction::RejectReviewObject,
                    ),
                    "Reject",
                ),
            ),
        ];
        if review_row_fits(ui, &full_decisions) {
            CompactReviewRowLayout::default()
        } else {
            CompactReviewRowLayout {
                shortcut_decisions: true,
                allow_wrap: !review_row_fits(ui, &shortcut_decisions),
            }
        }
    }

    fn drawer_panel_buttons(&mut self, ui: &mut egui::Ui, icon_only: bool) {
        self.drawer_panel_button(ui, Drawer::Workflow, "Workflow", false, icon_only);
        self.drawer_panel_button(ui, Drawer::Inspector, "Inspector", true, icon_only);
    }

    fn drawer_panel_button(
        &mut self,
        ui: &mut egui::Ui,
        drawer: Drawer,
        label: &'static str,
        panel_on_right: bool,
        icon_only: bool,
    ) {
        let selected = self.work.drawer == Some(drawer);
        let action = match drawer {
            Drawer::Workflow => labello_domain::UserAction::ToggleWorkflowPanel,
            Drawer::Inspector => labello_domain::UserAction::ToggleInspectorPanel,
        };
        let shortcut = self.shortcut_text(ui.ctx(), action);
        let icon_id = ui.id().with(("drawer-panel-icon", label));
        let icon_width = if icon_only { 20.0 } else { 25.0 };
        let icon = egui::Atom::custom(icon_id, egui::vec2(icon_width, 16.0));
        let content = if icon_only {
            egui::Atoms::new(icon)
        } else {
            egui::Atoms::new((icon, RichText::new(label)))
        };
        let choice = egui::Button::new(content)
            .selected(selected)
            .min_size(egui::vec2(if icon_only { 44.0 } else { 0.0 }, 44.0))
            .gap(theme::SPACE_2)
            .atom_ui(ui);
        let icon_rect = choice.rect(icon_id);
        let response = choice
            .response
            .on_hover_text(format!("{label} ({shortcut})"));
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Button,
                true,
                selected,
                label,
            )
        });
        if let Some(icon_rect) = icon_rect {
            paint_side_panel_toggle_icon(
                ui,
                icon_rect,
                !selected,
                panel_on_right,
                ui.style().interact(&response).fg_stroke.color,
            );
        }
        if response.clicked() {
            self.trigger_user_action(action);
        }
    }
}

#[derive(Clone, Copy, Default)]
struct CompactReviewRowLayout {
    shortcut_decisions: bool,
    allow_wrap: bool,
}

fn drawer_panel_labels_fit(ui: &egui::Ui) -> bool {
    let spacing = ui.spacing().item_spacing.x;
    panel_label_button_width(ui, "Workflow")
        + panel_label_button_width(ui, "Inspector")
        + spacing
        <= ui.available_width() + 0.5
}

fn panel_label_button_width(ui: &egui::Ui, label: &str) -> f32 {
    25.0 + theme::SPACE_2 + text_button_width(ui, label)
}

fn review_row_fits(ui: &egui::Ui, decisions: &[f32; 2]) -> bool {
    decisions.iter().sum::<f32>() + ui.spacing().item_spacing.x <= ui.available_width() + 0.5
}

fn text_button_width(ui: &egui::Ui, label: &str) -> f32 {
    let font_id = egui::TextStyle::Button.resolve(ui.style());
    let text_width = ui.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap(label.to_owned(), font_id, theme::TEXT)
            .size()
            .x
    });
    (text_width + 2.0 * ui.spacing().button_padding.x).max(ui.spacing().interact_size.x)
}
