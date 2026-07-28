impl LabelloApp {
    pub(crate) fn workspace_actions(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
        if !self.work_view() {
            return;
        }
        if self.manual_migration_active() {
            let icon_only = ui.available_width() < 432.0;
            if layout != LayoutMode::Wide {
                self.migration_workspace_actions(ui, false);
                self.drawer_panel_buttons(ui, icon_only);
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
            let available_width = ui.available_width();
            let icon_only = available_width < 432.0;
            ui.horizontal_wrapped(|ui| {
                self.migration_workspace_actions(ui, true);
                self.drawer_panel_buttons(ui, icon_only);
            });
            return;
        }
        let ready = (self.work.assignment.is_some() || self.runtime.api.is_none())
            && !self.loading.saving
            && !self.loading.image
            && self.work.pending_transition.is_none();
        ui.horizontal_wrapped(|ui| {
            if self.view == AppView::Annotate
                && theme::primary_button(ui, ready, egui::Button::new("Submit & next")).clicked()
            {
                self.trigger_user_action(labello_domain::UserAction::NextImage);
            }
            if self.view == AppView::Review && self.work.correction_draft.is_none() {
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
        });
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
        let action = match drawer {
            Drawer::Workflow => labello_domain::UserAction::ToggleWorkflowPanel,
            Drawer::Inspector => labello_domain::UserAction::ToggleInspectorPanel,
        };
        let response = choice.response.on_hover_text(format!(
            "{label} ({})",
            self.shortcut_text(ui.ctx(), action)
        ));
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
