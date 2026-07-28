impl LabelloApp {
    pub(crate) fn visible_prelabels(&self) -> Vec<labello_domain::PrelabelSuggestion> {
        if self.view != AppView::Annotate {
            return Vec::new();
        }
        self.work
            .current
            .as_ref()
            .map(|current| {
                current
                    .prelabels
                    .iter()
                    .filter(|suggestion| {
                        !self
                            .work
                            .accepted_prelabels
                            .contains(&suggestion.suggestion_id)
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
        if self.work.selected_prelabel.as_ref().is_none_or(|selected| {
            !prelabels
                .iter()
                .any(|suggestion| &suggestion.suggestion_id == selected)
        }) {
            self.work.selected_prelabel = prelabels
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
            let selected = self.work.selected_prelabel.as_ref() == Some(&suggestion.suggestion_id);
            let frame = theme::selected_card_frame(selected);
            frame.show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .selectable_label(selected, suggestion.class_id.to_string())
                        .clicked()
                    {
                        self.work.selected_prelabel = Some(suggestion.suggestion_id.clone());
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
                        self.work.selected_prelabel = self
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
