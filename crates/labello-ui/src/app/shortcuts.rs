impl LabelloApp {
    pub(crate) fn autosave_if_due(&mut self) {
        if self.work.save_status == SaveStatus::Dirty
            && !self.loading.saving
            && self.work.pending_transition.is_none()
            && !self.work.canvas.is_dragging()
            && self
                .work
                .last_edit_at
                .is_some_and(|edited| edited.elapsed() >= Duration::from_millis(750))
        {
            self.autosave();
        }
    }

    pub(crate) fn open_shortcut_settings(&mut self) {
        if self.work.show_settings {
            return;
        }
        let mut draft = self.work.keybindings.clone();
        draft.normalize();
        self.work.shortcut_settings.baseline = Some(draft.clone());
        self.work.shortcut_settings.draft = Some(draft);
        self.work.shortcut_settings.error = None;
        self.work.shortcut_settings.recording = None;
        self.work.shortcut_settings.confirm_discard = false;
        self.work.drawer = None;
        self.work.show_tutorial = false;
        self.work.show_settings = true;
    }

    pub(crate) fn shortcut_text(
        &self,
        ctx: &egui::Context,
        action: labello_domain::UserAction,
    ) -> String {
        self.work
            .keybindings
            .bindings
            .get(&action)
            .and_then(keyboard_shortcut)
            .map(|shortcut| ctx.format_shortcut(&shortcut))
            .unwrap_or_default()
    }

    fn cycle_workflow(&mut self, direction: isize) {
        let choices = self.workflow_choices();
        if choices.len() < 2 {
            return;
        }
        let current = choices
            .iter()
            .position(|choice| Some(&choice.task_id) == self.work.selected_task_id.as_ref())
            .unwrap_or(0);
        for offset in 1..choices.len() {
            let next = (current as isize + direction * offset as isize)
                .rem_euclid(choices.len() as isize) as usize;
            if self.displayed_workflow_availability(&choices[next].task_id) != Some(false) {
                self.request_transition(PendingTransition::Workflow(choices[next].task_id.clone()));
                return;
            }
        }
    }

    fn cycle_object(&mut self, direction: isize) {
        let objects = self
            .work
            .annotations
            .iter()
            .filter(|annotation| {
                !annotation.deleted && self.annotation_matches_selected_workflow(annotation)
            })
            .map(|annotation| annotation.annotation_id.clone())
            .collect::<Vec<_>>();
        if objects.is_empty() {
            self.work.selected_annotation = None;
            return;
        }
        let current = self
            .work
            .selected_annotation
            .as_ref()
            .and_then(|selected| objects.iter().position(|id| id == selected));
        let next = current.map_or_else(
            || if direction < 0 { objects.len() - 1 } else { 0 },
            |current| (current as isize + direction).rem_euclid(objects.len() as isize) as usize,
        );
        self.work.selected_annotation = Some(objects[next].clone());
    }

    fn cycle_prelabel(&mut self, direction: isize) {
        let prelabels = self.visible_prelabels();
        if prelabels.is_empty() {
            self.work.selected_prelabel = None;
            return;
        }
        let current = self.work.selected_prelabel.as_ref().and_then(|selected| {
            prelabels
                .iter()
                .position(|suggestion| &suggestion.suggestion_id == selected)
        });
        let next = current.map_or_else(
            || {
                if direction < 0 {
                    prelabels.len() - 1
                } else {
                    0
                }
            },
            |current| (current as isize + direction).rem_euclid(prelabels.len() as isize) as usize,
        );
        self.work.selected_prelabel = Some(prelabels[next].suggestion_id.clone());
    }

    fn active_prelabel(&self) -> Option<labello_domain::PrelabelSuggestion> {
        let prelabels = self.visible_prelabels();
        self.work
            .selected_prelabel
            .as_ref()
            .and_then(|selected| {
                prelabels
                    .iter()
                    .find(|suggestion| &suggestion.suggestion_id == selected)
            })
            .cloned()
            .or_else(|| prelabels.into_iter().next())
    }

    pub(crate) fn discard_prelabel(&mut self, suggestion_id: String) {
        if !self.work.accepted_prelabels.contains(&suggestion_id) {
            self.work.accepted_prelabels.push(suggestion_id);
        }
        self.work.selected_prelabel = self
            .visible_prelabels()
            .first()
            .map(|suggestion| suggestion.suggestion_id.clone());
    }

    pub(crate) fn trigger_user_action(&mut self, action: labello_domain::UserAction) {
        use labello_domain::UserAction;
        if self.manual_migration_active() {
            match action {
                UserAction::NextImage => {
                    self.trigger_migration_primary_action();
                    return;
                }
                UserAction::AcceptReviewObject if self.view == AppView::Review => {
                    self.trigger_migration_review_action(labello_domain::ReviewDecision::Approved);
                    return;
                }
                UserAction::RejectReviewObject if self.view == AppView::Review => {
                    self.trigger_migration_review_action(labello_domain::ReviewDecision::Rejected);
                    return;
                }
                UserAction::ToggleKeypointHidden => {
                    if self.work.migration.inspected_group_id.is_none()
                        && self
                        .selected_task()
                        .and_then(|task| task.skeleton.as_ref())
                        .is_some_and(|spec| spec.allow_hidden)
                    {
                        self.work.migration.next_hidden = !self.work.migration.next_hidden;
                    }
                    return;
                }
                UserAction::MarkKeypointAbsent => {
                    if self.work.migration.inspected_group_id.is_none() {
                        self.skip_migration_keypoint();
                    }
                    return;
                }
                UserAction::AddMissingObject => {
                    self.trigger_missing_migration_object_action();
                    return;
                }
                UserAction::UndoEdit | UserAction::DeleteAnnotation => {
                    if self.work.migration.inspected_group_id.is_none() {
                        self.remove_last_migration_keypoint();
                    }
                    return;
                }
                UserAction::SelectPreviousObject => {
                    self.edit_previous_migration_object();
                    return;
                }
                UserAction::SelectNextObject => {
                    self.inspect_migration_object(1);
                    return;
                }
                UserAction::RedoEdit
                | UserAction::SaveAnnotations
                | UserAction::SelectPreviousPrelabel
                | UserAction::SelectNextPrelabel
                | UserAction::AcceptPrelabel
                | UserAction::DiscardPrelabel => return,
                _ => {}
            }
        }
        let ready = (self.work.assignment.is_some() || self.runtime.api.is_none())
            && !self.loading.saving
            && !self.loading.image
            && self.work.pending_transition.is_none()
            && !self.work.canvas.is_dragging();
        let previous_ready = self.view == AppView::Annotate
            && self.work.previous_annotation_assignment.is_some()
            && self.runtime.api.is_some()
            && !self.loading.saving
            && !self.loading.image
            && self.work.pending_transition.is_none()
            && !self.work.canvas.is_dragging();
        match action {
            UserAction::NextImage if self.view == AppView::Annotate && ready => {
                self.submit_and_advance()
            }
            UserAction::PreviousImage if previous_ready => self.return_to_previous_assignment(),
            UserAction::UndoEdit if self.view == AppView::Annotate && ready => self.undo(),
            UserAction::RedoEdit if self.view == AppView::Annotate && ready => self.redo(),
            UserAction::SaveAnnotations if self.view == AppView::Annotate && ready => {
                self.autosave()
            }
            UserAction::SkipAssignment if self.work_view() && ready => self.skip_assignment(),
            UserAction::DeleteAnnotation if self.view == AppView::Annotate && ready => {
                self.delete_selected()
            }
            UserAction::OpenTutorial => {
                self.work.drawer = None;
                self.work.show_tutorial = !self.work.show_tutorial;
            }
            UserAction::ToggleWorkflowPanel => {
                self.work.show_tutorial = false;
                let layout = self
                    .runtime
                    .repaint_ctx
                    .as_ref()
                    .map(|ctx| LayoutMode::for_width(ctx.content_rect().width()))
                    .unwrap_or(LayoutMode::Medium);
                if layout == LayoutMode::Wide {
                    self.work.drawer = None;
                    self.work.workflow_panel_collapsed =
                        !self.work.workflow_panel_collapsed;
                } else {
                    self.work.drawer =
                        (self.work.drawer != Some(Drawer::Workflow)).then_some(Drawer::Workflow);
                }
            }
            UserAction::ToggleInspectorPanel => {
                self.work.show_tutorial = false;
                let layout = self
                    .runtime
                    .repaint_ctx
                    .as_ref()
                    .map(|ctx| LayoutMode::for_width(ctx.content_rect().width()))
                    .unwrap_or(LayoutMode::Medium);
                if layout == LayoutMode::Wide {
                    self.work.drawer = None;
                    self.work.inspector_panel_collapsed =
                        !self.work.inspector_panel_collapsed;
                } else {
                    self.work.drawer =
                        (self.work.drawer != Some(Drawer::Inspector)).then_some(Drawer::Inspector);
                }
            }
            UserAction::OpenSettings => self.open_shortcut_settings(),
            UserAction::SelectPreviousWorkflow if self.view == AppView::Annotate && ready => {
                self.cycle_workflow(-1)
            }
            UserAction::SelectNextWorkflow if self.view == AppView::Annotate && ready => {
                self.cycle_workflow(1)
            }
            UserAction::SelectPreviousObject if self.view == AppView::Annotate && ready => {
                self.cycle_object(-1)
            }
            UserAction::SelectNextObject if self.view == AppView::Annotate && ready => {
                self.cycle_object(1)
            }
            UserAction::SelectPreviousPrelabel if self.view == AppView::Annotate && ready => {
                self.cycle_prelabel(-1)
            }
            UserAction::SelectNextPrelabel if self.view == AppView::Annotate && ready => {
                self.cycle_prelabel(1)
            }
            UserAction::AcceptPrelabel if self.view == AppView::Annotate && ready => {
                if let Some(suggestion) = self.active_prelabel() {
                    self.accept_prelabel(&suggestion);
                    self.work.selected_prelabel = self
                        .visible_prelabels()
                        .first()
                        .map(|suggestion| suggestion.suggestion_id.clone());
                }
            }
            UserAction::DiscardPrelabel if self.view == AppView::Annotate && ready => {
                if let Some(suggestion) = self.active_prelabel() {
                    self.discard_prelabel(suggestion.suggestion_id);
                }
            }
            UserAction::ToggleKeypointHidden if self.view == AppView::Annotate && ready => {
                if self
                    .selected_task()
                    .and_then(|task| task.skeleton.as_ref())
                    .is_some_and(|spec| spec.allow_hidden)
                {
                    self.work.next_keypoint_hidden = !self.work.next_keypoint_hidden;
                }
            }
            UserAction::MarkKeypointAbsent if self.view == AppView::Annotate && ready => {
                if self.work.active_skeleton.is_some()
                    && self
                        .selected_task()
                        .and_then(|task| task.skeleton.as_ref())
                        .is_some_and(|spec| spec.allow_absent)
                {
                    self.skip_keypoint();
                }
            }
            UserAction::RetryImageLoad
                if self.view == AppView::Annotate
                    && self.work.current.is_none()
                    && !self.loading.image =>
            {
                self.retry_assignment_load()
            }
            UserAction::TogglePanMode if self.work_view() && self.work.current.is_some() => {
                self.work.canvas.toggle_pan_mode()
            }
            UserAction::ZoomIn if self.work_view() && self.work.current.is_some() => {
                self.work.canvas.zoom_in()
            }
            UserAction::ZoomOut if self.work_view() && self.work.current.is_some() => {
                self.work.canvas.zoom_out()
            }
            UserAction::FitImage if self.work_view() && self.work.current.is_some() => {
                self.work.canvas.fit_view()
            }
            UserAction::RefocusObject
                if self.work.current.is_some()
                    && (self.view == AppView::Review || self.manual_migration_active()) =>
            {
                self.refocus_active_object();
            }
            UserAction::AcceptReviewObject if self.view == AppView::Review => {
                if self.work.correction_draft.is_none() {
                    self.request_review(labello_domain::ReviewDecision::Approved);
                }
            }
            UserAction::RejectReviewObject if self.view == AppView::Review => {
                if self.work.correction_draft.is_none() {
                    self.request_review(labello_domain::ReviewDecision::Rejected);
                }
            }
            UserAction::SelectBoundingBoxTool
            | UserAction::SelectKeypointTool
            | UserAction::AddMissingObject
            | UserAction::ToggleOfflineMode => {}
            _ => {}
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if !self.work_view()
            || ctx.text_edit_focused()
            || self.work.pending_transition.is_some()
            || self.work.show_settings
            || self.runtime.persistence.recovery.is_some()
            || self.navigation.drawer_open
            || egui::Popup::is_any_open(ctx)
        {
            return;
        }
        if let Some(drawer) = self.work.drawer {
            let action = match drawer {
                Drawer::Workflow => labello_domain::UserAction::ToggleWorkflowPanel,
                Drawer::Inspector => labello_domain::UserAction::ToggleInspectorPanel,
            };
            let chord = self.work.keybindings.bindings.get(&action).cloned();
            if chord
                .as_ref()
                .is_some_and(|chord| consume_keyboard_shortcut(ctx, chord))
            {
                self.trigger_user_action(action);
            }
            return;
        }
        if self.loading.saving || self.loading.image {
            return;
        }
        if self.work.canvas.pan_mode() && ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.work.canvas.exit_pan_mode();
        }
        if self.view == AppView::Review
            && self.work.correction_draft.is_some()
            && ctx.input_mut(|input| {
                input.consume_shortcut(&egui::KeyboardShortcut::new(
                    egui::Modifiers::COMMAND,
                    egui::Key::Z,
                )) || input.consume_shortcut(&egui::KeyboardShortcut::new(
                    egui::Modifiers::CTRL,
                    egui::Key::Z,
                ))
            })
        {
            self.undo_correction();
        }
        let mut bindings = self
            .work
            .keybindings
            .bindings
            .iter()
            .filter(|(action, _)| match action.context() {
                labello_domain::ActionContext::WorkWorkspace => self.work_view(),
                labello_domain::ActionContext::WorkImage => {
                    self.work_view() && self.work.current.is_some()
                }
                labello_domain::ActionContext::AnnotateWorkspace => self.view == AppView::Annotate,
                labello_domain::ActionContext::AnnotateImage => {
                    self.view == AppView::Annotate && self.work.current.is_some()
                }
                labello_domain::ActionContext::AnnotateNoImage => {
                    self.view == AppView::Annotate && self.work.current.is_none()
                }
                labello_domain::ActionContext::Review => self.view == AppView::Review,
                labello_domain::ActionContext::Legacy => false,
            })
            .map(|(action, chord)| (*action, chord.clone()))
            .collect::<Vec<_>>();
        bindings.sort_by_key(|(_, chord)| {
            std::cmp::Reverse(
                chord.shift as u8 + chord.alt as u8 + (chord.ctrl || chord.command) as u8,
            )
        });
        for (action, chord) in bindings {
            if consume_keyboard_shortcut(ctx, &chord) {
                self.trigger_user_action(action);
            }
        }
    }
}
