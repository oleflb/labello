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
            &dataset_name,
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
        let review_actions_in_overflow =
            layout != LayoutMode::Wide && self.view == AppView::Review;

        let destinations = self.primary_navigation_destinations();
        let navigation_width = |label: &str| 30.0 + label.chars().count() as f32 * 7.5;
        let total_navigation_width = destinations
            .iter()
            .map(|(_, label)| navigation_width(label))
            .sum::<f32>()
            + spacing * destinations.len().saturating_sub(1) as f32;
        let mut overflow_needed = !hidden_actions.is_empty()
            || hidden_account
            || review_actions_in_overflow
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
                            review_actions_in_overflow,
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
        include_review_actions: bool,
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
        if include_review_actions
            && ui
                .add_enabled(
                    self.work.assignment.is_some()
                        && !self.loading.saving
                        && !self.loading.image
                        && self.work.pending_transition.is_none(),
                    egui::Button::new("Skip assignment")
                        .shortcut_text(self.shortcut_text(
                            ui.ctx(),
                            labello_domain::UserAction::SkipAssignment,
                        ))
                        .min_size(egui::vec2(theme::MENU_WIDTH, 44.0)),
                )
                .clicked()
        {
            self.trigger_user_action(labello_domain::UserAction::SkipAssignment);
            ui.close();
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

    fn app_bar_icon_button(&mut self, ui: &mut egui::Ui, action: AppBarAction) {
        let enabled = action != AppBarAction::SignOut || !self.loading.logout;
        let selected = match action {
            AppBarAction::Setup => self.view == AppView::Setup,
            AppBarAction::Tutorial => self.work.show_tutorial,
            AppBarAction::Settings => self.work.show_settings,
            AppBarAction::SignOut => false,
        };
        let response = ui
            .add_enabled_ui(enabled, |ui| {
                ui.add_sized(
                    [44.0, 44.0],
                    egui::Button::new("").selected(selected),
                )
            })
            .inner
            .on_hover_text(action.tooltip());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, action.accessible_label())
        });
        Self::paint_app_bar_action_icon(
            ui,
            response.rect,
            action,
            ui.style().interact(&response).fg_stroke.color,
        );
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

    fn paint_app_bar_action_icon(
        ui: &egui::Ui,
        rect: egui::Rect,
        action: AppBarAction,
        color: egui::Color32,
    ) {
        let center = rect.center();
        let painter = ui.painter();
        let stroke = egui::Stroke::new(1.7, color);

        match action {
            AppBarAction::Setup => {
                let tooth_angle = std::f32::consts::TAU / 8.0;
                let mut outline = Vec::with_capacity(32);
                for index in 0..8 {
                    let angle = index as f32 * tooth_angle;
                    for (offset, radius) in [
                        (-0.34, 6.1),
                        (-0.18, 8.3),
                        (0.18, 8.3),
                        (0.34, 6.1),
                    ] {
                        let point_angle = angle + offset * tooth_angle;
                        outline.push(
                            center
                                + egui::vec2(point_angle.cos(), point_angle.sin()) * radius,
                        );
                    }
                }
                painter.add(egui::Shape::closed_line(outline, stroke));
                painter.circle_stroke(center, 2.6, stroke);
            }
            AppBarAction::Tutorial => {
                painter.text(
                    center,
                    egui::Align2::CENTER_CENTER,
                    "?",
                    egui::FontId::proportional(18.0),
                    color,
                );
            }
            AppBarAction::Settings => {
                let keycap =
                    egui::Rect::from_center_size(center, egui::vec2(17.0, 15.0));
                painter.rect_stroke(
                    keycap,
                    egui::CornerRadius::same(3),
                    stroke,
                    egui::StrokeKind::Inside,
                );
                painter.text(
                    center - egui::vec2(0.0, 1.0),
                    egui::Align2::CENTER_CENTER,
                    "K",
                    egui::FontId::proportional(9.5),
                    color,
                );
                painter.line_segment(
                    [
                        egui::pos2(keycap.left() + 3.0, keycap.bottom() - 2.0),
                        egui::pos2(keycap.right() - 3.0, keycap.bottom() - 2.0),
                    ],
                    egui::Stroke::new(1.2, color),
                );
            }
            AppBarAction::SignOut => {
                let door = egui::Rect::from_center_size(
                    center - egui::vec2(3.5, 0.0),
                    egui::vec2(8.0, 15.0),
                );
                painter.line_segment([door.right_top(), door.left_top()], stroke);
                painter.line_segment([door.left_top(), door.left_bottom()], stroke);
                painter.line_segment([door.left_bottom(), door.right_bottom()], stroke);
                painter.line_segment(
                    [
                        center - egui::vec2(1.0, 0.0),
                        center + egui::vec2(8.0, 0.0),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        center + egui::vec2(4.5, -3.5),
                        center + egui::vec2(8.0, 0.0),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        center + egui::vec2(4.5, 3.5),
                        center + egui::vec2(8.0, 0.0),
                    ],
                    stroke,
                );
            }
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
            let full = status_text(self.work.save_status);
            let text = if layout == LayoutMode::Compact {
                compact_status_text(self.work.save_status)
            } else {
                full
            };
            let mut detail = format!("Annotation status: {full}");
            let mut accessible_label = format!("Status: {full}");
            let mut intent = status_intent(self.work.save_status);
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

}
