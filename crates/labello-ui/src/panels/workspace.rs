impl LabelloApp {
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
        if self.work.pending_transition.is_some() {
            self.transition_modal(ctx);
            return;
        }
        if self.admin.confirm_discard {
            self.admin_discard_modal(ctx);
            return;
        }
        if self.work.show_settings {
            self.settings_modal(ctx);
            return;
        }
        if layout != LayoutMode::Wide && self.work_view() {
            let screen = ctx.content_rect();
            let compact = layout == LayoutMode::Compact;
            let width = if self.work.drawer == Some(Drawer::Workflow) {
                self.workflow_panel_width(ctx)
                    .min((screen.width() - 48.0).max(240.0))
            } else if compact {
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
            if let Some(drawer) = self.work.drawer {
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
                    self.work.drawer = None;
                }
            }
        }
        self.tutorial_overlay(ctx);
    }

    pub(crate) fn workspace_context_bar(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
        let current = self.work.current.clone();
        let workflow = self.selected_workflow().map(|workflow| workflow.label());
        let view = self.view;
        let loading_image = self.loading.image;
        let has_assignment = self.work.assignment.is_some();
        let short = Self::short_viewport(ui.ctx().content_rect().size());
        let review_phase = if view == AppView::Review && current.is_some() {
            if self.work.correction_draft.is_some() {
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
            ui.horizontal(|ui| self.canvas_controls(ui, layout))
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
                    self.canvas_controls(ui, layout);
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
                    self.canvas_controls(ui, layout);
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

    fn canvas_controls(&mut self, ui: &mut egui::Ui, layout: LayoutMode) {
        ui.horizontal(|ui| {
            let pan_shortcut =
                self.shortcut_text(ui.ctx(), labello_domain::UserAction::TogglePanMode);
            let pan = egui::Button::new("Pan")
                .selected(self.work.canvas.pan_mode())
                .min_size(egui::vec2(52.0, 44.0));
            if ui
                .add_enabled(self.work.canvas.can_pan(), pan)
                .on_disabled_hover_text("Zoom in before enabling Pan mode.")
                .on_hover_text(format!("Pan ({pan_shortcut}). Space or middle-drag."))
                .clicked()
            {
                self.trigger_user_action(labello_domain::UserAction::TogglePanMode);
            }

            let zoom_out_shortcut =
                self.shortcut_text(ui.ctx(), labello_domain::UserAction::ZoomOut);
            let can_zoom_out = self.work.canvas.can_zoom_out();
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
                egui::Label::new(format!("{:.0}%", self.work.canvas.current_zoom() * 100.0)),
            );

            let zoom_in_shortcut = self.shortcut_text(ui.ctx(), labello_domain::UserAction::ZoomIn);
            let can_zoom_in = self.work.canvas.can_zoom_in();
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
            if layout == LayoutMode::Wide {
                self.workflow_panel_toggle(ui);
            }
        });
    }

}
