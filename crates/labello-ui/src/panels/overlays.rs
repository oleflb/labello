impl LabelloApp {
    fn tutorial_overlay(&mut self, ctx: &egui::Context) {
        if !self.work.show_tutorial || !self.work_view() {
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
            self.work.show_tutorial = false;
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
        let Some(pending) = self.work.pending_transition.clone() else {
            return;
        };
        let current = self
            .selected_workflow()
            .map(|workflow| workflow.label())
            .unwrap_or_else(|| "No workflow".to_string());
        let destination = self.transition_label(&pending);
        let discards_migration_draft =
            self.manual_migration_active() && self.migration_has_unsaved_input();
        let discards_edits = matches!(
            pending,
            PendingTransition::NextAssignment | PendingTransition::PreviousAssignment(_)
        ) && self.view == AppView::Annotate
            && (matches!(self.work.save_status, SaveStatus::Dirty | SaveStatus::Retry)
                || discards_migration_draft);
        if pending == PendingTransition::NextAssignment && !discards_edits {
            return;
        }
        let modal_title = if discards_migration_draft {
            "Unsaved migration draft"
        } else if discards_edits {
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
                    theme::inline_message(ui, theme::Intent::Warning, if discards_migration_draft {
                        "Continuing will discard migration keypoints or exclusion input that has not been saved."
                    } else {
                        "Skipping now will discard annotation changes that have not been saved."
                    });
                }
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    if self.view == AppView::Annotate
                        && !discards_migration_draft
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
                            if discards_migration_draft {
                                "Discard draft and switch"
                            } else {
                                "Discard edits and skip"
                            }
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

    fn migration_revisit_discard_modal(&mut self, ctx: &egui::Context) {
        let response =
            theme::modal(ctx, egui::Id::new("migration-revisit-discard-modal")).show(ctx, |ui| {
                ui.set_max_width((ctx.content_rect().width() - 48.0).clamp(240.0, 560.0));
                ui.heading("Discard current migration draft?");
                theme::inline_message(
                    ui,
                    theme::Intent::Warning,
                    "Editing the previous object will discard migration keypoints or exclusion input that has not been saved.",
                );
                ui.horizontal_wrapped(|ui| {
                    if theme::danger_button(
                        ui,
                        !self.work.migration.busy,
                        egui::Button::new("Discard draft and edit object"),
                    )
                    .clicked()
                    {
                        self.confirm_pending_migration_revisit();
                    }
                    if theme::quiet_button(
                        ui,
                        !self.work.migration.busy,
                        egui::Button::new("Cancel"),
                    )
                    .clicked()
                    {
                        self.cancel_pending_migration_revisit();
                    }
                });
            });
        response.response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Window,
                true,
                "Discard current migration draft",
            )
        });
        if response.should_close() {
            self.cancel_pending_migration_revisit();
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
                    self.admin.confirm_discard = false;
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
            self.admin.confirm_discard = false;
            self.runtime.notice = Some("Staged admin changes discarded".to_string());
        } else if response.should_close() {
            self.admin.confirm_discard = false;
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
        if !self.work.show_settings {
            return;
        }
        if self.work.shortcut_settings.confirm_discard {
            self.shortcut_discard_modal(ctx);
            return;
        }
        if self.work.shortcut_settings.draft.is_none() {
            let mut draft = self.work.keybindings.clone();
            draft.normalize();
            self.work.shortcut_settings.baseline = Some(draft.clone());
            self.work.shortcut_settings.draft = Some(draft);
        }
        if !self.loading.keybindings
            && let Some(action) = self.work.shortcut_settings.recording
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
                    self.work.shortcut_settings.recording = None;
                } else if let Some(draft) = self.work.shortcut_settings.draft.as_mut() {
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
                    self.work.shortcut_settings.recording = None;
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
                if let Some(error) = &self.work.shortcut_settings.error {
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
                    theme::singleline_text_edit(&mut self.work.shortcut_settings.search)
                        .hint_text("Search by action or category"),
                )
                .labelled_by(search_label.id);
                ui.add_space(8.0);
                let conflicts = self
                    .work
                    .shortcut_settings
                    .draft
                    .as_ref()
                    .map(|draft| draft.conflicts())
                    .unwrap_or_default();
                let conflicting_actions = conflicts
                    .iter()
                    .flat_map(|(_, actions)| actions.iter().copied())
                    .collect::<std::collections::BTreeSet<_>>();
                let query = self
                    .work
                    .shortcut_settings
                    .search
                    .trim()
                    .to_ascii_lowercase();
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
                            .work
                            .shortcut_settings
                            .draft
                            .as_ref()
                            .and_then(|draft| draft.bindings.get(&action))
                        else {
                            continue;
                        };
                        let recording = self.work.shortcut_settings.recording == Some(action);
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
                let dirty =
                    self.work.shortcut_settings.draft != self.work.shortcut_settings.baseline;
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
            self.work.shortcut_settings.recording = Some(action);
        }
        if let Some(action) = reset_action {
            let default = labello_domain::KeybindingSet::defaults_for(self.config.user_id.clone())
                .bindings
                .get(&action)
                .cloned();
            if let (Some(draft), Some(default)) =
                (self.work.shortcut_settings.draft.as_mut(), default)
            {
                draft.bindings.insert(action, default);
            }
        }
        if reset_all {
            self.work.shortcut_settings.draft = Some(labello_domain::KeybindingSet::defaults_for(
                self.config.user_id.clone(),
            ));
            self.work.shortcut_settings.recording = None;
        }
        if save {
            self.request_keybindings_save();
        }
        let dirty = self.work.shortcut_settings.draft != self.work.shortcut_settings.baseline;
        if cancel || (!self.loading.keybindings && response.should_close()) {
            if dirty {
                self.work.shortcut_settings.confirm_discard = true;
                self.work.show_settings = true;
            } else {
                self.work.show_settings = false;
                self.work.shortcut_settings.draft = None;
                self.work.shortcut_settings.baseline = None;
                self.work.shortcut_settings.error = None;
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
                        self.work.shortcut_settings.confirm_discard = false;
                    }
                    if ui.button("Discard changes").clicked() {
                        self.work.shortcut_settings.confirm_discard = false;
                        self.work.shortcut_settings.draft = None;
                        self.work.shortcut_settings.baseline = None;
                        self.work.shortcut_settings.error = None;
                        self.work.show_settings = false;
                    }
                });
            });
        response.response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Window, true, "Discard shortcut changes")
        });
        if response.should_close() {
            self.work.shortcut_settings.confirm_discard = false;
        }
    }

}
