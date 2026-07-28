impl LabelloApp {
    const AVAILABILITY_LOADING_TEXT: &'static str = "Checking assignment availability…";
    const WORKFLOW_ICON_SIZE: f32 = 28.0;
    const WORKFLOW_PILL_HEIGHT: f32 = 52.0;

    pub(crate) fn workflow_panel_width(&self, ctx: &egui::Context) -> f32 {
        let workflows = self.workflow_choices();
        let style = ctx.style_of(ctx.theme());
        let label_font = egui::TextStyle::Button.resolve(&style);
        let small_font = egui::TextStyle::Small.resolve(&style);
        let show_availability_loading =
            self.work.availability.loading && self.work.availability.tasks.is_empty();
        let widest_content = ctx.fonts_mut(|fonts| {
            let widest_pill = workflows
                .iter()
                .map(|workflow| {
                    let label_width = fonts
                        .layout_no_wrap(workflow.label(), label_font.clone(), theme::TEXT)
                        .size()
                        .x;
                    Self::WORKFLOW_ICON_SIZE
                        + theme::SPACE_2
                        + label_width
                        + 2.0 * theme::SPACE_3
                })
                .fold(0.0, f32::max);
            let availability_row = if show_availability_loading {
                style.spacing.interact_size.y
                    + style.spacing.item_spacing.x
                    + fonts
                        .layout_no_wrap(
                            Self::AVAILABILITY_LOADING_TEXT.to_string(),
                            small_font,
                            theme::MUTED,
                        )
                        .size()
                        .x
            } else {
                0.0
            };
            widest_pill.max(availability_row)
        });

        let measured_width = widest_content + 2.0 * theme::SPACE_4 + 2.0;
        if workflows.is_empty() {
            measured_width.max(LayoutMode::TASK_PANEL_WIDTH)
        } else {
            measured_width
        }
    }

    fn workflow_queue_status(&self) -> Option<String> {
        (self.view == AppView::Annotate && self.work.assignment.is_some()).then(|| {
            if self.work.queue.failed() {
                "Loaded assignment queue refill failed; retrying".to_string()
            } else if self.work.queue.is_loading() {
                format!(
                    "Loaded assignment queue: {} of {}; loading",
                    self.work.queue.len(),
                    self.work.queue.queue_size()
                )
            } else {
                format!(
                    "Loaded assignment queue: {} of {}",
                    self.work.queue.len(),
                    self.work.queue.queue_size()
                )
            }
        })
    }

    pub(crate) fn workflow_panel_toggle(&mut self, ui: &mut egui::Ui) {
        let (label, hover) = if self.work.workflow_panel_collapsed {
            ("Expand workflow panel", "Expand workflow panel")
        } else {
            ("Collapse workflow panel", "Collapse workflow panel")
        };
        let response = ui
            .add(egui::Button::new("").min_size(egui::vec2(44.0, 44.0)))
            .on_hover_text(hover);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label)
        });
        paint_workflow_panel_toggle_icon(
            ui,
            response.rect,
            self.work.workflow_panel_collapsed,
            ui.style().interact(&response).fg_stroke.color,
        );
        if response.clicked() {
            self.trigger_user_action(labello_domain::UserAction::ToggleWorkflowPanel);
            ui.ctx()
                .request_discard("workflow panel visibility changed");
        }
    }

    pub(crate) fn task_panel(&mut self, ui: &mut egui::Ui) {
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
            let button = egui::Button::new((
                egui::Atom::custom(
                    icon_id,
                    egui::vec2(Self::WORKFLOW_ICON_SIZE, Self::WORKFLOW_ICON_SIZE),
                ),
                RichText::new(workflow.label()).strong(),
            ))
            .selected(selected)
            .frame(true)
            .frame_when_inactive(true)
            .corner_radius(theme::SURFACE_RADIUS)
            .min_size(egui::vec2(
                ui.available_width(),
                Self::WORKFLOW_PILL_HEIGHT,
            ))
            .gap(theme::SPACE_2)
            .truncate();
            let choice = ui
                .add_enabled_ui(ready && !unavailable, |ui| button.atom_ui(ui))
                .inner;
            choice.response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Button,
                    ready && !unavailable,
                    selected,
                    workflow.label(),
                )
            });
            let response_id = choice.response.id;
            let queue_status = selected
                .then(|| self.workflow_queue_status())
                .flatten();
            let accessibility_description = if unavailable {
                Some(match queue_status.as_ref() {
                    Some(queue_status) => {
                        format!("No assignments available. {queue_status}")
                    }
                    None => "No assignments available".to_string(),
                })
            } else {
                queue_status.clone()
            };
            if let Some(description) = accessibility_description {
                ui.ctx().accesskit_node_builder(response_id, |node| {
                    node.set_description(description);
                });
            }
            if let Some(icon_rect) = choice.rect(icon_id) {
                workflow_type_icon(ui, icon_id, icon_rect, &workflow.annotation_type);
            }
            let mut hover_text = format!(
                "{} workflow\nPrevious: {} · Next: {}",
                annotation_type_label(&workflow.annotation_type),
                self.shortcut_text(ui.ctx(), labello_domain::UserAction::SelectPreviousWorkflow,),
                self.shortcut_text(ui.ctx(), labello_domain::UserAction::SelectNextWorkflow,)
            );
            if let Some(queue_status) = queue_status.as_ref() {
                hover_text.push('\n');
                hover_text.push_str(queue_status);
            }
            let mut response = choice.response.on_hover_text(hover_text);
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
                let spinner = ui.spinner();
                spinner.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::ProgressIndicator,
                        true,
                        "Loading workflow assignment availability",
                    )
                });
                ui.small(Self::AVAILABILITY_LOADING_TEXT);
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
    }

}

fn paint_workflow_panel_toggle_icon(
    ui: &egui::Ui,
    rect: egui::Rect,
    expanding: bool,
    color: egui::Color32,
) {
    let icon = egui::Rect::from_center_size(rect.center(), egui::vec2(25.0, 16.0));
    let (panel, chevron_x) = if expanding {
        (
            egui::Rect::from_min_size(icon.min, egui::vec2(16.0, icon.height())),
            icon.right() - 3.0,
        )
    } else {
        (
            egui::Rect::from_min_size(
                egui::pos2(icon.left() + 9.0, icon.top()),
                egui::vec2(16.0, icon.height()),
            ),
            icon.left() + 3.0,
        )
    };
    let painter = ui.painter();
    painter.rect_stroke(
        panel,
        egui::CornerRadius::same(2),
        egui::Stroke::new(1.5, color),
        egui::StrokeKind::Inside,
    );
    painter.rect_filled(
        egui::Rect::from_min_max(
            panel.min + egui::vec2(2.5, 2.5),
            egui::pos2(panel.left() + 6.0, panel.bottom() - 2.5),
        ),
        egui::CornerRadius::same(1),
        color,
    );
    let direction = if expanding { 1.0 } else { -1.0 };
    for y in [-4.0, 4.0] {
        painter.line_segment(
            [
                egui::pos2(chevron_x - direction * 3.0, icon.center().y + y),
                egui::pos2(chevron_x, icon.center().y),
            ],
            egui::Stroke::new(1.5, color),
        );
    }
}
