impl eframe::App for LabelloApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if !self.theme_applied {
            self.theme_applied = theme::apply_fallback(ui.ctx());
            if !self.theme_applied {
                return;
            }
        }
        ui.painter()
            .rect_filled(ui.max_rect(), egui::CornerRadius::ZERO, theme::APP_BG);
        self.process_messages(ui.ctx());
        self.retry_prefetch_if_due(ui.ctx());
        self.sync_review_selection();
        self.sync_manual_migration();
        self.start_next_persistence_command();
        self.start_setup_load();
        self.refresh_stats_if_due();
        self.refresh_assignment_availability_if_due();
        self.refresh_ingest_if_due();
        self.refresh_import_if_due();
        self.autosave_if_due();
        self.handle_shortcuts(ui.ctx());
        let viewport = ui.available_size();
        let layout = LayoutMode::for_width(ui.available_width());
        let workflow_panel_width = self.workflow_panel_width(ui.ctx());
        let compact_action_height = (self.work_view()
            && (layout != LayoutMode::Wide || self.manual_migration_active()))
        .then(|| self.workspace_actions_height(layout, viewport));
        egui::Panel::top("app_bar")
            .exact_size(56.0)
            .frame(theme::top_bar_frame().inner_margin(egui::Margin::symmetric(14, 6)))
            .show(ui, |ui| self.app_bar(ui, layout));
        if self.work_view() {
            egui::Panel::top("workspace_context")
                .exact_size(self.workspace_context_height(layout, viewport))
                .frame(
                    theme::top_bar_frame()
                        .fill(theme::PANEL)
                        .inner_margin(egui::Margin::symmetric(14, 6)),
                )
                .show(ui, |ui| self.workspace_context_bar(ui, layout));
        }
        if let Some(action_height) = compact_action_height {
            egui::Panel::bottom("compact_primary_actions")
                .min_size(action_height)
                .frame(theme::top_bar_frame())
                .show(ui, |ui| {
                    if layout == LayoutMode::Compact {
                        self.compact_workspace_actions(ui);
                    } else {
                        ui.horizontal_wrapped(|ui| self.workspace_actions(ui, layout));
                    }
                });
        }
        let show_wide_inspector = self.work_view()
            && layout == LayoutMode::Wide
            && !self.work.inspector_panel_collapsed;
        let inspector_left = show_wide_inspector.then(|| {
            ui.ctx().content_rect().right() - LayoutMode::INSPECTOR_PANEL_WIDTH - theme::SPACE_2
        });
        if self.work_view() && layout == LayoutMode::Wide {
            if !self.work.workflow_panel_collapsed {
                egui::Panel::left("task_panel")
                    .resizable(false)
                    .exact_size(workflow_panel_width)
                    .frame(theme::side_frame())
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| self.task_panel(ui));
                    });
            }
            if show_wide_inspector {
                egui::Panel::right("review_panel")
                    .resizable(false)
                    .exact_size(LayoutMode::INSPECTOR_PANEL_WIDTH)
                    .frame(theme::side_frame().inner_margin(egui::Margin {
                        left: 24,
                        right: 16,
                        top: 16,
                        bottom: 16,
                    }))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                egui::Frame::new()
                                    .inner_margin(egui::Margin {
                                        left: 2,
                                        right: 0,
                                        top: 0,
                                        bottom: 0,
                                    })
                                    .show(ui, |ui| self.right_panel(ui, true));
                            });
                    });
            }
        }
        let central_frame = if self.work_view() {
            theme::central_frame()
                .fill(egui::Color32::TRANSPARENT)
                .inner_margin(egui::Margin::same(theme::SPACE_2 as i8))
        } else {
            theme::central_frame()
        };
        egui::CentralPanel::default()
            .frame(central_frame)
            .show(ui, |ui| {
                let mut work_rect = ui.available_rect_before_wrap();
                if let Some(action_height) = compact_action_height {
                    let action_top = ui.ctx().content_rect().bottom() - action_height;
                    work_rect.max.y = work_rect
                        .max
                        .y
                        .min(action_top - theme::SPACE_2);
                }
                if let Some(inspector_left) = inspector_left {
                    work_rect.max.x = work_rect.max.x.min(inspector_left);
                }
                if work_rect != ui.available_rect_before_wrap() {
                    let mut work_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(work_rect)
                            .layout(*ui.layout()),
                    );
                    self.central(&mut work_ui, layout);
                } else {
                    self.central(ui, layout);
                }
            });
        self.overlays(ui.ctx(), layout);
        self.queue_current_drafts();
        self.persist_workspace_preference();
        self.start_next_command();
        if self.work.save_status == SaveStatus::Dirty
            && let Some(edited) = self.work.last_edit_at
        {
            ui.ctx().request_repaint_after(
                std::time::Duration::from_millis(750).saturating_sub(edited.elapsed()),
            );
        }
        if self.loading.ingesting {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(500));
        }
        if self.view == AppView::Stats && !self.loading.stats {
            let until_refresh = self
                .datasets
                .last_stats_attempt
                .map(|attempt| std::time::Duration::from_secs(3).saturating_sub(attempt.elapsed()))
                .unwrap_or(std::time::Duration::from_secs(3));
            ui.ctx().request_repaint_after(until_refresh);
        }
        if self.work_view() && self.runtime.api.is_some() && !self.work.availability.loading {
            let until_refresh = if self.work.availability.checked_at.is_some() {
                self.assignment_availability_cache_age()
                    .map(|age| ASSIGNMENT_AVAILABILITY_CACHE_TTL.saturating_sub(age))
                    .unwrap_or_default()
            } else {
                self.work
                    .availability
                    .last_attempt
                    .map(|attempt| {
                        ASSIGNMENT_AVAILABILITY_CACHE_TTL.saturating_sub(attempt.elapsed())
                    })
                    .unwrap_or_default()
            };
            ui.ctx().request_repaint_after(until_refresh);
        }
    }
}
