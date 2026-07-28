impl LabelloApp {
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
            !self.loading.saving && !self.loading.image && self.work.pending_transition.is_none();
        for workflow in workflows {
            let selected = self.work.selected_task_id.as_ref() == Some(&workflow.task_id);
            let unavailable = self.workflow_availability(&workflow.task_id) == Some(false);
            let icon_id = ui.id().with(("workflow-type", &workflow.task_id));
            let label = if unavailable {
                format!("{}\nNo assignments available", workflow.label())
            } else {
                workflow.label()
            };
            let button = egui::Button::new((
                egui::Atom::custom(icon_id, egui::vec2(28.0, 28.0)),
                RichText::new(label).strong(),
            ))
            .selected(selected)
            .frame(true)
            .frame_when_inactive(true)
            .corner_radius(theme::SURFACE_RADIUS)
            .min_size(egui::vec2(ui.available_width(), 52.0))
            .gap(theme::SPACE_2)
            .truncate();
            let choice = ui
                .add_enabled_ui(ready && !unavailable, |ui| button.atom_ui(ui))
                .inner;
            if let Some(icon_rect) = choice.rect(icon_id) {
                workflow_type_icon(ui, icon_id, icon_rect, &workflow.annotation_type);
            }
            let mut response = choice.response.on_hover_text(format!(
                "{} workflow\nPrevious: {} · Next: {}",
                annotation_type_label(&workflow.annotation_type),
                self.shortcut_text(ui.ctx(), labello_domain::UserAction::SelectPreviousWorkflow,),
                self.shortcut_text(ui.ctx(), labello_domain::UserAction::SelectNextWorkflow,)
            ));
            if unavailable {
                response = response.on_disabled_hover_text(
                    "No assignment is currently available for this workflow. Availability is advisory and can be retried below.",
                );
            }
            if response.clicked() && !selected {
                self.request_transition(PendingTransition::Workflow(workflow.task_id.clone()));
            }
        }
        if self.work.availability.loading && self.work.availability.tasks.is_empty() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.small("Checking assignment availability…");
            });
        } else if let Some(error) = self.work.availability.error.clone() {
            theme::inline_message(
                ui,
                theme::Intent::Warning,
                "Assignment availability could not be checked. Workflows remain selectable.",
            );
            ui.small(error);
            if ui.button("Retry availability").clicked() {
                self.work.availability.last_attempt = None;
                self.request_assignment_availability();
            }
        }
        ui.separator();
        ui.label(RichText::new("Assignment").strong());
        if self.runtime.api.is_none() {
            ui.label("Demo image");
            ui.small("Local demo state; changes are not persisted.");
        } else if self.work.assignment.is_some() {
            ui.label("Active assignment");
            ui.small("Reserved for you until you submit or skip it.");
            if self.view == AppView::Annotate {
                let status = if self.work.queue.failed() {
                    "Prepared queue refill failed; retrying".to_string()
                } else if self.work.queue.is_loading() {
                    format!(
                        "Prepared queue: {} of {} ready, loading next",
                        self.work.queue.len(),
                        self.work.queue.queue_size()
                    )
                } else {
                    format!(
                        "Prepared queue: {} of {} ready",
                        self.work.queue.len(),
                        self.work.queue.queue_size()
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

}
