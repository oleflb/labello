impl LabelloApp {
    pub(crate) fn workspace_actions(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
        if !self.work_view() {
            return;
        }
        if self.manual_migration_active() {
            if layout != LayoutMode::Wide && ui.button("Migration controls").clicked() {
                self.work.drawer = Some(Drawer::Inspector);
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
            if ui.button("Migration controls").clicked() {
                self.work.drawer = Some(Drawer::Inspector);
            }
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

}
