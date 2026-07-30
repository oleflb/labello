impl LabelloApp {
    pub(crate) fn inspector_panel_toggle(&mut self, ui: &mut egui::Ui) {
        let (label, hover) = if self.work.inspector_panel_collapsed {
            ("Expand inspector panel", "Expand inspector panel")
        } else {
            ("Collapse inspector panel", "Collapse inspector panel")
        };
        let response = ui
            .add(egui::Button::new("").min_size(egui::vec2(44.0, 44.0)))
            .on_hover_text(hover);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label)
        });
        paint_side_panel_toggle_icon(
            ui,
            response.rect,
            self.work.inspector_panel_collapsed,
            true,
            ui.style().interact(&response).fg_stroke.color,
        );
        if response.clicked() {
            self.trigger_user_action(labello_domain::UserAction::ToggleInspectorPanel);
            ui.ctx()
                .request_discard("inspector panel visibility changed");
        }
    }

    pub(crate) fn right_panel(&mut self, ui: &mut egui::Ui, show_primary_actions: bool) {
        ui.heading(RichText::new("Inspector").color(theme::TEXT));
        if self.manual_migration_active() {
            // Migration commands live in the persistent workspace action bar so
            // collapsing this optional panel never hides the current action.
            self.manual_migration_actions(ui, false);
            return;
        }
        let active_count = self
            .work
            .annotations
            .iter()
            .filter(|annotation| {
                !annotation.deleted && self.annotation_matches_selected_workflow(annotation)
            })
            .count();
        theme::compact_metric(ui, "Active annotations", active_count.to_string());
        if self.view == AppView::Annotate {
            self.annotation_object_actions(ui);
        }
        if self.view == AppView::Annotate && self.work.tool == Tool::Keypoints {
            self.keypoint_actions(ui);
        }
        match self.view {
            AppView::Annotate => self.prelabel_panel(ui),
            AppView::Review => self.review_actions(ui, show_primary_actions),
            AppView::Adjudicate => self.adjudication_actions(ui, show_primary_actions),
            AppView::Setup | AppView::Admin | AppView::Stats => {}
        }
    }

    fn annotation_object_actions(&mut self, ui: &mut egui::Ui) {
        let objects = self
            .work.annotations
            .iter()
            .filter(|annotation| {
                !annotation.deleted && self.annotation_matches_selected_workflow(annotation)
            })
            .enumerate()
            .map(|(index, annotation)| {
                let class_name = self.class_name(&annotation.class_id);
                let geometry = match &annotation.geometry {
                    AnnotationGeometry::BoundingBox(bbox) => format!(
                        "Position: {:.0}% from left, {:.0}% from top\nSize: {:.0}% wide by {:.0}% high",
                        bbox.x * 100.0,
                        bbox.y * 100.0,
                        bbox.width * 100.0,
                        bbox.height * 100.0
                    ),
                    AnnotationGeometry::Skeleton(skeleton) => format!(
                        "Keypoints placed: {} of {}",
                        skeleton
                            .keypoints
                            .iter()
                            .filter(|keypoint| keypoint.point.is_some())
                            .count(),
                        skeleton.keypoints.len()
                    ),
                };
                (
                    annotation.annotation_id.clone(),
                    index + 1,
                    class_name,
                    geometry,
                )
            })
            .collect::<Vec<_>>();
        if objects.is_empty() {
            theme::empty_state(
                ui,
                "No objects yet",
                "Draw or accept an object to inspect it.",
                None,
            );
            return;
        }

        if self.work.selected_annotation.is_some()
            && !objects.iter().any(|(annotation_id, ..)| {
                Some(annotation_id) == self.work.selected_annotation.as_ref()
            })
        {
            self.work.selected_annotation = None;
        }

        ui.separator();
        ui.label(RichText::new("Objects").strong());
        for (annotation_id, number, class_name, geometry) in objects {
            let selected = self.work.selected_annotation.as_ref() == Some(&annotation_id);
            theme::selected_card_frame(selected).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                let label = format!(
                    "Object {number} | {class_name}{}",
                    if selected { " | Selected" } else { "" }
                );
                if ui
                    .add_sized(
                        [ui.available_width(), 44.0],
                        egui::Button::selectable(selected, &label).truncate(),
                    )
                    .on_hover_text(format!(
                        "{label}\nPrevious: {} | Next: {}",
                        self.shortcut_text(
                            ui.ctx(),
                            labello_domain::UserAction::SelectPreviousObject,
                        ),
                        self.shortcut_text(ui.ctx(), labello_domain::UserAction::SelectNextObject,)
                    ))
                    .clicked()
                {
                    self.work.selected_annotation = Some(annotation_id.clone());
                }

                egui::CollapsingHeader::new(format!("Geometry details for Object {number}"))
                    .id_salt(annotation_id.as_str())
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(&geometry)
                                .monospace()
                                .color(theme::TEXT_MUTED),
                        );
                    });
            });
        }
        if self.work.selected_annotation.is_some()
            && theme::danger_button(
                ui,
                true,
                egui::Button::new("Delete selected annotation").shortcut_text(
                    self.shortcut_text(ui.ctx(), labello_domain::UserAction::DeleteAnnotation),
                ),
            )
            .clicked()
        {
            self.delete_selected();
        }
    }

    fn keypoint_actions(&mut self, ui: &mut egui::Ui) {
        let spec = self.selected_task().and_then(|task| task.skeleton.clone());
        let next_keypoint = spec.as_ref().and_then(|skeleton| {
            skeleton
                .keypoints
                .get(self.work.skeleton_keypoint_index)
                .map(|keypoint| keypoint.name.clone())
        });
        if let Some(name) = next_keypoint {
            theme::compact_metric(
                ui,
                if self.work.active_skeleton.is_some() {
                    "Place keypoint"
                } else {
                    "Start skeleton"
                },
                name.as_str(),
            );
            if let Some(spec) = spec {
                let hidden_shortcut = self.shortcut_text(
                    ui.ctx(),
                    labello_domain::UserAction::ToggleKeypointHidden,
                );
                ui.add_enabled_ui(!self.loading.saving, |ui| {
                    if spec.allow_hidden {
                        keypoint_placement_mode(
                            ui,
                            &name,
                            &mut self.work.next_keypoint_hidden,
                            &hidden_shortcut,
                        );
                    }
                    ui.horizontal(|ui| {
                        if spec.allow_absent
                            && self.work.active_skeleton.is_some()
                            && spec
                                .keypoints
                                .get(self.work.skeleton_keypoint_index)
                                .is_some_and(|keypoint| !keypoint.required)
                            && ui
                                .add(
                                    egui::Button::new(format!(
                                        "Mark {name} as not present"
                                    ))
                                    .shortcut_text(self.shortcut_text(
                                        ui.ctx(),
                                        labello_domain::UserAction::MarkKeypointAbsent,
                                    )),
                                )
                                .on_hover_text(
                                    "Record this optional keypoint without a position.",
                                )
                                .clicked()
                        {
                            self.skip_keypoint();
                        }
                    });
                });
            }
        }
    }

    fn review_phase(&self) -> (&'static str, String, &'static str) {
        let total = self
            .work
            .annotations
            .iter()
            .filter(|annotation| {
                !annotation.deleted && self.annotation_matches_selected_workflow(annotation)
            })
            .count();
        if self.work.review_index < total {
            (
                "Object review",
                format!("{} of {total}", self.work.review_index + 1),
                "The active object is highlighted on the canvas.",
            )
        } else {
            (
                "Final check",
                "Full image".to_string(),
                "Check for missed objects before completing this review.",
            )
        }
    }

    fn review_actions(&mut self, ui: &mut egui::Ui, show_primary_actions: bool) {
        let ready = self.work.assignment.is_some() && !self.loading.saving;
        if self.work.correction_draft.is_some() {
            self.correction_actions(ui, ready);
            return;
        }
        let (phase, value, explanation) = self.review_phase();
        theme::compact_metric(ui, phase, value);
        ui.label(explanation);
        self.review_refocus_button(ui);
        if self.can_correct_review_object() {
            ui.add_space(8.0);
            if ui
                .add_enabled(ready, egui::Button::new("Correct object"))
                .on_hover_text("Edit this existing object without returning it to the annotator.")
                .clicked()
            {
                self.start_correction();
            }
        }
        if show_primary_actions {
            ui.horizontal_wrapped(|ui| self.review_decision_buttons(ui, false, false));
        }
    }

    fn review_refocus_button(&mut self, ui: &mut egui::Ui) {
        let Some(mut annotation) = self.current_review_annotation().cloned() else {
            return;
        };
        if let Some(draft) = self
            .work
            .correction_draft
            .as_ref()
            .filter(|draft| draft.annotation_id == annotation.annotation_id)
        {
            annotation.geometry = draft.edited_geometry.clone();
        }
        if ui
            .small_button("Refocus object")
            .on_hover_text("Center and zoom the active review object on the canvas.")
            .clicked()
        {
            self.work.canvas.focus_annotation(&annotation);
        }
    }

    fn review_decision_buttons(
        &mut self,
        ui: &mut egui::Ui,
        shortcut_only: bool,
        fill_width: bool,
    ) {
        let ready = self.work.assignment.is_some() && !self.loading.saving;
        let compact =
            LayoutMode::for_width(ui.ctx().content_rect().width()) == LayoutMode::Compact;
        let approve_shortcut = self.shortcut_text(
            ui.ctx(),
            labello_domain::UserAction::AcceptReviewObject,
        );
        let reject_shortcut = self.shortcut_text(
            ui.ctx(),
            labello_domain::UserAction::RejectReviewObject,
        );
        let (approve, reject) = if shortcut_only {
            (
                shortcut_button_label(&approve_shortcut, "Accept"),
                shortcut_button_label(&reject_shortcut, "Reject"),
            )
        } else if compact || fill_width {
            ("Accept".to_string(), "Reject".to_string())
        } else if self.current_review_annotation().is_none() {
            ("Complete review".to_string(), "Send back".to_string())
        } else {
            (
                "Approve object".to_string(),
                "Reject object & finish".to_string(),
            )
        };
        let button_width = fill_width
            .then(|| ((ui.available_width() - ui.spacing().item_spacing.x) / 2.0).max(44.0));
        let approve_button = egui::Button::new(&approve).min_size(egui::vec2(
            button_width.unwrap_or_default(),
            if fill_width { 44.0 } else { 0.0 },
        ));
        let reject_button = egui::Button::new(&reject).min_size(egui::vec2(
            button_width.unwrap_or_default(),
            if fill_width { 44.0 } else { 0.0 },
        ));
        if theme::primary_button(ui, ready, approve_button)
            .on_hover_text(format!(
                "Accept review object ({})",
                shortcut_button_label(&approve_shortcut, "Accept")
            ))
            .clicked()
        {
            self.request_review(ReviewDecision::Approved);
        }
        if theme::danger_button(ui, ready, reject_button)
            .on_hover_text(format!(
                "Reject review object ({})",
                shortcut_button_label(&reject_shortcut, "Reject")
            ))
            .clicked()
        {
            self.request_review(ReviewDecision::Rejected);
        }
    }

    fn correction_actions(&mut self, ui: &mut egui::Ui, ready: bool) {
        ui.separator();
        ui.heading("Correction mode");
        ui.label("Only the highlighted existing object can be edited.");
        self.review_refocus_button(ui);

        let skeleton_keypoints = self.work.correction_draft.as_ref().and_then(|draft| {
            let AnnotationGeometry::Skeleton(skeleton) = &draft.edited_geometry else {
                return None;
            };
            Some(
                skeleton
                    .keypoints
                    .iter()
                    .enumerate()
                    .map(|(index, keypoint)| (index, keypoint.name.clone(), keypoint.state.clone()))
                    .collect::<Vec<_>>(),
            )
        });
        ui.add_space(theme::SPACE_2);
        ui.label(RichText::new("Object").strong().color(theme::TEXT_MUTED));
        if let Some(keypoints) = skeleton_keypoints {
            ui.label("Edit only the highlighted skeleton on the canvas.");
            ui.add_space(theme::SPACE_2);
            ui.label(RichText::new("Keypoints").strong().color(theme::TEXT_MUTED));
            ui.label("Select and drag an existing keypoint:");
            for (index, name, state) in keypoints {
                let selected = self
                    .work
                    .correction_draft
                    .as_ref()
                    .is_some_and(|draft| draft.selected_keypoint == Some(index));
                if ui
                    .selectable_label(
                        selected,
                        format!("{name} ({})", keypoint_state_label(&state)),
                    )
                    .clicked()
                {
                    self.select_correction_keypoint(index);
                }
            }
            self.correction_keypoint_state(ui, ready);
        } else {
            ui.label("Drag inside the box to move it, or drag a handle to resize it.");
        }

        ui.add_space(theme::SPACE_2);
        ui.label(RichText::new("Reason").strong().color(theme::TEXT_MUTED));
        if let Some(draft) = self.work.correction_draft.as_mut() {
            let label = ui.label("Reason (optional)");
            ui.add_enabled_ui(ready, |ui| {
                theme::resizable_multiline_text_edit(
                    ui,
                    ui.make_persistent_id("correction-reason"),
                    &mut draft.reason,
                    2,
                    Some("What was corrected?"),
                )
                .labelled_by(label.id);
            });
        }

        let (can_undo, geometry_changed) = self
            .work
            .correction_draft
            .as_ref()
            .map(|draft| (!draft.geometry_history.is_empty(), draft.geometry_changed()))
            .unwrap_or_default();
        ui.add_space(theme::SPACE_2);
        ui.label(RichText::new("Actions").strong().color(theme::TEXT_MUTED));
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(ready && can_undo, egui::Button::new("Undo correction"))
                .clicked()
            {
                self.undo_correction();
            }
            if theme::danger_button(ui, ready, egui::Button::new("Discard correction")).clicked() {
                self.discard_correction();
            }
            if theme::primary_button(
                ui,
                ready && geometry_changed,
                egui::Button::new("Correct & finalize"),
            )
            .on_disabled_hover_text("Move, resize, or change a keypoint before finalizing.")
            .clicked()
            {
                self.request_correction();
            }
        });
    }

    fn correction_keypoint_state(&mut self, ui: &mut egui::Ui, ready: bool) {
        let Some((index, current, has_point, required)) =
            self.work.correction_draft.as_ref().and_then(|draft| {
                let index = draft.selected_keypoint?;
                let AnnotationGeometry::Skeleton(skeleton) = &draft.edited_geometry else {
                    return None;
                };
                let keypoint = skeleton.keypoints.get(index)?;
                let required = self
                    .selected_task()
                    .and_then(|task| task.skeleton.as_ref())
                    .and_then(|spec| spec.keypoints.get(index))
                    .is_some_and(|spec| spec.required);
                Some((
                    index,
                    keypoint.state.clone(),
                    keypoint.point.is_some(),
                    required,
                ))
            })
        else {
            return;
        };
        let (allow_hidden, allow_absent) = self
            .selected_task()
            .and_then(|task| task.skeleton.as_ref())
            .map(|spec| (spec.allow_hidden, spec.allow_absent))
            .unwrap_or_default();
        ui.label(format!("Keypoint {} visibility", index + 1));
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    ready && has_point,
                    egui::Button::selectable(current == KeypointState::Visible, "Visible"),
                )
                .clicked()
            {
                self.set_correction_keypoint_state(KeypointState::Visible);
            }
            if ui
                .add_enabled(
                    ready && allow_hidden && has_point,
                    egui::Button::selectable(current == KeypointState::Hidden, "Hidden"),
                )
                .clicked()
            {
                self.set_correction_keypoint_state(KeypointState::Hidden);
            }
            if ui
                .add_enabled(
                    ready && allow_absent && !required,
                    egui::Button::selectable(current == KeypointState::Absent, "Absent"),
                )
                .clicked()
            {
                self.set_correction_keypoint_state(KeypointState::Absent);
            }
        });
    }

    fn adjudication_actions(&mut self, ui: &mut egui::Ui, show_primary_actions: bool) {
        let candidates = self
            .work
            .annotations
            .iter()
            .filter(|annotation| {
                !annotation.deleted && self.annotation_matches_selected_workflow(annotation)
            })
            .count();
        theme::compact_metric(ui, "Candidate annotations", candidates.to_string());
        if candidates == 0 {
            theme::inline_message(
                ui,
                theme::Intent::Warning,
                "This assignment has no annotation candidates to adjudicate.",
            );
        }
        if show_primary_actions {
            ui.horizontal_wrapped(|ui| self.adjudication_decision_buttons(ui, false));
        }
    }

    fn adjudication_decision_buttons(&mut self, ui: &mut egui::Ui, compact: bool) {
        let has_candidates = self.work.annotations.iter().any(|annotation| {
            !annotation.deleted && self.annotation_matches_selected_workflow(annotation)
        });
        let ready = self.work.assignment.is_some() && !self.loading.saving;
        let (accept, correct) = if compact {
            ("Accept all", "Send back")
        } else {
            ("Accept all annotations", "Send back for correction")
        };
        if theme::primary_button(ui, ready && has_candidates, egui::Button::new(accept)).clicked() {
            self.request_adjudication(AdjudicationDecision::AcceptAnnotation);
        }
        if theme::danger_button(ui, ready, egui::Button::new(correct)).clicked() {
            self.request_adjudication(AdjudicationDecision::NeedsCorrection);
        }
    }

}

pub(crate) fn shortcut_button_label(shortcut: &str, fallback: &str) -> String {
    if shortcut.is_empty() {
        fallback.to_string()
    } else {
        shortcut.to_string()
    }
}
