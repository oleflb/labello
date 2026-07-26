use labello_domain::{
    AnnotationGeometry, AnnotationOrigin, AnnotationType, AnnotationVersion, KeypointAnnotation,
    KeypointState, MigrationCursor, MigrationDispositionStatus, MigrationExclusionReason,
    MigrationPassId, NormalizedPoint, ObjectGroupId, RevisionSource, SkeletonGeometry,
};

use eframe::egui::{self, RichText};

use crate::{
    app::{AppView, LabelloApp, MigrationAction, UiCommand},
    canvas::{CanvasAction, CanvasInteraction, show_canvas_colored},
    theme,
};

const MAX_EXCLUSION_NOTE_BYTES: usize = 2_000;

#[derive(Clone, Debug)]
pub(crate) struct ManualMigrationState {
    pub cursor: Option<MigrationCursor>,
    pub active_pass_id: Option<MigrationPassId>,
    pub draft: Option<SkeletonGeometry>,
    pub draft_group: Option<ObjectGroupId>,
    pub keypoint_index: usize,
    pub next_hidden: bool,
    pub exclusion_reason: MigrationExclusionReason,
    pub exclusion_note: String,
    pub review_index: usize,
    pub progress: Option<labello_client::ManualMigrationProgress>,
    pub busy: bool,
    pub error: Option<String>,
}

impl Default for ManualMigrationState {
    fn default() -> Self {
        Self {
            cursor: None,
            active_pass_id: None,
            draft: None,
            draft_group: None,
            keypoint_index: 0,
            next_hidden: false,
            exclusion_reason: MigrationExclusionReason::NoValidSkeleton,
            exclusion_note: String::new(),
            review_index: 0,
            progress: None,
            busy: false,
            error: None,
        }
    }
}

impl ManualMigrationState {
    pub(crate) fn empty_skeleton(names: impl IntoIterator<Item = String>) -> SkeletonGeometry {
        SkeletonGeometry {
            keypoints: names
                .into_iter()
                .map(|name| KeypointAnnotation {
                    name,
                    state: KeypointState::Absent,
                    point: None,
                })
                .collect(),
        }
    }
}

impl LabelloApp {
    pub(crate) fn manual_migration_active(&self) -> bool {
        let Some(task) = self.selected_task() else {
            return false;
        };
        task.manual_box_guide_migration.is_some()
            && self
                .current_state
                .as_ref()
                .is_some_and(|state| state.migration_target_sets.contains_key(&task.task_id))
    }

    pub(crate) fn sync_manual_migration(&mut self) {
        if !self.manual_migration_active() || self.work.migration.busy {
            return;
        }
        let Some(task_id) = self.selected_task_id.clone() else {
            return;
        };
        if self.work.migration.progress.is_some() {
            if self.view == AppView::Review {
                self.work.migration.review_index = self.canonical_migration_review_index();
            }
            return;
        }
        if self.work.migration.active_pass_id.is_none() {
            self.work.migration.active_pass_id = self.current_state.as_ref().and_then(|state| {
                let assignment_id = &self.assignment.as_ref()?.assignment_id;
                state
                    .migration_passes
                    .values()
                    .filter(|pass| pass.task_id == task_id && &pass.assignment_id == assignment_id)
                    .max_by_key(|pass| pass.started_at)
                    .map(|pass| pass.pass_id.clone())
            });
        }
        let active_pass = self.work.migration.active_pass_id.as_ref();
        let cursor = self
            .current_state
            .as_ref()
            .and_then(|state| state.migration_cursor(&task_id, active_pass).ok());
        if self.work.migration.cursor != cursor {
            self.work.migration.cursor = cursor;
            self.work.migration.draft = None;
            self.work.migration.draft_group = None;
            self.work.migration.keypoint_index = 0;
            self.work.migration.next_hidden = false;
            self.work.migration.exclusion_note.clear();
        }
        if self.view == AppView::Review {
            self.work.migration.review_index = self.canonical_migration_review_index();
        }
    }

    pub(crate) fn migration_workspace_canvas(&mut self, ui: &mut egui::Ui) {
        let Some(current) = self.current.clone() else {
            return;
        };
        let texture = self.current_texture.clone();
        let Some(task) = self.selected_task().cloned() else {
            return;
        };
        let Some(state) = self.current_state.clone() else {
            return;
        };
        let active = self.migration_active_target();
        let guide = active.as_ref().and_then(|(_, target)| {
            state
                .current_annotation(&target.guide_annotation_id)
                .cloned()
        });
        if let Some(guide) = guide.as_ref().filter(|guide| !guide.deleted) {
            self.canvas.set_review_focus(Some(guide));
        } else {
            self.canvas.set_review_focus(None);
        }
        let task_id = task.task_id.clone();
        let mut annotations = Vec::new();
        let mut annotation_colors = std::collections::BTreeMap::new();
        if let Some(set) = state.migration_target_sets.get(&task_id) {
            for target in &set.targets {
                let status = state
                    .migration_dispositions
                    .get(&task_id)
                    .and_then(|values| values.get(&target.object_group_id))
                    .map(|disposition| &disposition.status);
                let current = active
                    .as_ref()
                    .is_some_and(|(group, _)| group == &target.object_group_id);
                let color = if current {
                    theme::ACCENT_HOVER
                } else {
                    match status {
                        Some(MigrationDispositionStatus::Annotated { .. }) => theme::SUCCESS,
                        Some(MigrationDispositionStatus::Excluded { .. }) => theme::DANGER,
                        _ => theme::TEXT_MUTED,
                    }
                };
                if let Some(guide) = state.current_annotation(&target.guide_annotation_id) {
                    let mut rendered = guide.clone();
                    if rendered.deleted {
                        rendered.deleted = false;
                        annotation_colors.insert(rendered.annotation_id.clone(), theme::DANGER);
                    } else {
                        annotation_colors.insert(rendered.annotation_id.clone(), color);
                    }
                    annotations.push(rendered);
                }
                if let Some(MigrationDispositionStatus::Annotated {
                    skeleton_annotation_id,
                    ..
                }) = status
                    && let Some(skeleton) = state
                        .current_annotation(skeleton_annotation_id)
                        .filter(|skeleton| !skeleton.deleted && skeleton.task_id == task_id)
                {
                    annotations.push(skeleton.clone());
                    annotation_colors.insert(skeleton.annotation_id.clone(), color);
                }
            }
        }
        let mut selected = None;
        if let Some((group_id, target)) = active.as_ref()
            && let Some(draft) = self.work.migration.draft.clone()
        {
            annotations.retain(|annotation| {
                annotation.annotation_id != target.reserved_skeleton_annotation_id
            });
            selected = Some(target.reserved_skeleton_annotation_id.clone());
            annotations.push(AnnotationVersion {
                annotation_id: target.reserved_skeleton_annotation_id.clone(),
                version: 1,
                object_group_id: Some(group_id.clone()),
                origin: AnnotationOrigin::native(),
                task_id: task.task_id.clone(),
                class_id: task.class_ids[0].clone(),
                annotation_type: AnnotationType::Skeleton,
                revision_source: RevisionSource::Human {
                    action: labello_domain::HumanRevisionKind::Authored,
                },
                geometry: AnnotationGeometry::Skeleton(draft),
                author_user_id: self.config.user_id.clone(),
                created_at: labello_domain::now(),
                updated_at: labello_domain::now(),
                deleted: false,
            });
            annotation_colors.insert(selected.clone().unwrap(), theme::ACCENT_HOVER);
        }
        let edges = task
            .skeleton
            .as_ref()
            .map(|skeleton| {
                skeleton
                    .edges
                    .iter()
                    .map(|edge| (edge.from.clone(), edge.to.clone()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if let Some((group_id, _)) = active.as_ref() {
            let guide_state = if guide.as_ref().is_some_and(|guide| guide.deleted) {
                "Deleted guide tombstone"
            } else if guide.is_some() {
                "Canonical guide"
            } else {
                "Guide unavailable"
            };
            let disposition = self.migration_disposition(group_id);
            ui.label(
                RichText::new(format!(
                    "{guide_state} | Status: {}",
                    disposition_label(disposition.as_ref())
                ))
                .strong(),
            );
        }
        let interaction = CanvasInteraction {
            editable: !self.work.migration.busy
                && guide.as_ref().is_some_and(|guide| !guide.deleted)
                && matches!(
                    self.work.migration.cursor,
                    Some(MigrationCursor::Object { .. })
                ),
            allow_create: true,
            allow_selection: false,
            edit_keypoints: false,
            selected_keypoint: None,
        };
        let action = show_canvas_colored(
            ui,
            &mut self.canvas,
            texture.as_ref(),
            &annotations,
            [current.image.width, current.image.height],
            false,
            selected.as_ref(),
            interaction,
            &edges,
            &[],
            theme::ANNOTATION,
            &annotation_colors,
        );
        if let Some(CanvasAction::PlaceKeypoint(point)) = action {
            self.place_migration_keypoint(point);
        }
    }

    pub(crate) fn manual_migration_actions(&mut self, ui: &mut egui::Ui) {
        if let Some(error) = self.work.migration.error.clone() {
            theme::inline_message(ui, theme::Intent::Error, error);
            if ui
                .add_enabled(
                    !self.loading.image && !self.work.migration.busy,
                    egui::Button::new("Reload assignment state"),
                )
                .clicked()
            {
                self.retry_assignment_load();
            }
        }
        if self.view == AppView::Review {
            self.migration_review_actions(ui);
            return;
        }
        let (expected, annotated, excluded, pending) = self.migration_counts();
        ui.label(RichText::new("Migration status").strong());
        ui.horizontal_wrapped(|ui| {
            theme::badge(ui, &format!("Total {expected}"), theme::Intent::Info);
            theme::badge(
                ui,
                &format!("Annotated {annotated}"),
                theme::Intent::Success,
            );
            theme::badge(ui, &format!("Excluded {excluded}"), theme::Intent::Error);
            theme::badge(ui, &format!("Pending {pending}"), theme::Intent::Neutral);
        });
        match self.work.migration.cursor.clone() {
            Some(MigrationCursor::Object {
                object_group_id,
                sequence_index,
            }) => self.migration_object_actions(ui, object_group_id, sequence_index, expected),
            Some(MigrationCursor::FullImage) => self.migration_full_image_actions(ui, expected),
            None => {
                theme::inline_message(
                    ui,
                    theme::Intent::Warning,
                    "Migration cursor is unavailable for this assignment.",
                );
                if ui
                    .add_enabled(
                        !self.loading.image && !self.work.migration.busy,
                        egui::Button::new("Reload assignment state"),
                    )
                    .clicked()
                {
                    self.retry_assignment_load();
                }
            }
        }
    }

    fn migration_object_actions(
        &mut self,
        ui: &mut egui::Ui,
        group_id: ObjectGroupId,
        sequence_index: u64,
        expected: u64,
    ) {
        ui.separator();
        ui.label(RichText::new("Canonical guide").strong());
        ui.label(format!(
            "Object {} of {expected} | Group {group_id} | Read-only guide",
            sequence_index + 1
        ));
        let status = self.migration_disposition(&group_id);
        ui.label(format!(
            "Current status: {}",
            disposition_label(status.as_ref())
        ));
        let guide = self.current_state.as_ref().and_then(|state| {
            let target = state
                .migration_target_sets
                .get(self.selected_task_id.as_ref()?)?
                .targets
                .iter()
                .find(|target| target.object_group_id == group_id)?;
            state
                .current_annotation(&target.guide_annotation_id)
                .cloned()
        });
        let target_available = self.migration_expectation(&group_id).is_some();
        let guide_valid = guide.as_ref().is_some_and(|guide| !guide.deleted);
        if let Some(guide) = guide.as_ref().filter(|guide| !guide.deleted)
            && ui
                .button(format!("Focus current box (guide v{})", guide.version))
                .clicked()
        {
            self.canvas.focus_annotation(guide);
        }
        let dependency = self
            .current_state
            .as_ref()
            .and_then(|state| {
                state
                    .migration_dependencies
                    .get(self.selected_task_id.as_ref()?)
                    .and_then(|markers| markers.get(&group_id))
            })
            .is_some();
        if !guide_valid {
            theme::inline_message(
                ui,
                theme::Intent::Error,
                "The canonical guide is deleted or unavailable. Skeleton editing and keep/reopen actions are disabled; record an exclusion or reload after the guide is repaired.",
            );
            if ui
                .add_enabled(
                    !self.loading.image && !self.work.migration.busy,
                    egui::Button::new("Reload assignment state"),
                )
                .clicked()
            {
                self.retry_assignment_load();
            }
        } else if dependency {
            theme::inline_message(
                ui,
                theme::Intent::Warning,
                "Correction required: the canonical guide changed after the prior disposition.",
            );
        }
        if self.work.migration.active_pass_id.is_some()
            && status
                .as_ref()
                .is_some_and(|status| !matches!(status, MigrationDispositionStatus::Pending))
        {
            if ui
                .add_enabled(
                    !self.work.migration.busy && guide_valid && !dependency,
                    egui::Button::new("Keep current disposition & advance"),
                )
                .clicked()
            {
                self.request_keep_migration_target(group_id.clone());
            }
            if matches!(status, Some(MigrationDispositionStatus::Excluded { .. })) {
                if ui
                    .add_enabled(
                        !self.work.migration.busy && guide_valid,
                        egui::Button::new("Reopen excluded target"),
                    )
                    .clicked()
                {
                    self.request_reopen_migration_target(group_id.clone());
                }
            } else {
                ui.small("Edit the annotated skeleton directly below; reopening is not required.");
            }
        }
        ui.separator();
        ui.label(RichText::new("Skeleton draft").strong());
        self.ensure_migration_draft(&group_id);
        if matches!(status, Some(MigrationDispositionStatus::Annotated { .. }))
            && ui
                .add_enabled(guide_valid, egui::Button::new("Redraw annotated skeleton"))
                .clicked()
        {
            let names: Vec<String> = self
                .selected_task()
                .and_then(|task| task.skeleton.as_ref())
                .map(|spec| {
                    spec.keypoints
                        .iter()
                        .map(|point| point.name.clone())
                        .collect()
                })
                .unwrap_or_default();
            self.work.migration.draft = Some(ManualMigrationState::empty_skeleton(names));
            self.work.migration.keypoint_index = 0;
        }
        let (placed, total, next_name) = self
            .work
            .migration
            .draft
            .as_ref()
            .map(|draft| {
                (
                    draft
                        .keypoints
                        .iter()
                        .filter(|keypoint| keypoint.point.is_some())
                        .count(),
                    draft.keypoints.len(),
                    draft
                        .keypoints
                        .get(self.work.migration.keypoint_index)
                        .map(|keypoint| keypoint.name.clone()),
                )
            })
            .unwrap_or_default();
        ui.label(format!(
            "Draft status: {placed} of {total} keypoints placed"
        ));
        if let Some(name) = next_name {
            ui.label(format!("Next keypoint: {name}"));
            ui.checkbox(&mut self.work.migration.next_hidden, "Place as hidden");
            let can_absent = self
                .selected_task()
                .and_then(|task| task.skeleton.as_ref())
                .is_some_and(|spec| {
                    spec.allow_absent
                        && spec
                            .keypoints
                            .get(self.work.migration.keypoint_index)
                            .is_some_and(|keypoint| !keypoint.required)
                });
            if ui
                .add_enabled(can_absent, egui::Button::new("Mark keypoint absent"))
                .clicked()
            {
                self.skip_migration_keypoint();
            }
        }
        if theme::primary_button(
            ui,
            !self.work.migration.busy && guide_valid && self.migration_draft_valid(),
            egui::Button::new("Save skeleton & advance"),
        )
        .clicked()
        {
            self.request_save_migration_skeleton(group_id.clone());
        }
        ui.separator();
        ui.label(RichText::new("Exclude target").strong());
        egui::ComboBox::from_label("Exclusion reason")
            .selected_text(exclusion_label(self.work.migration.exclusion_reason))
            .show_ui(ui, |ui| {
                for reason in [
                    MigrationExclusionReason::NoValidSkeleton,
                    MigrationExclusionReason::InsufficientVisibleFeatures,
                    MigrationExclusionReason::InvalidSourceBox,
                    MigrationExclusionReason::DuplicateSourceObject,
                    MigrationExclusionReason::ObjectNotPresent,
                    MigrationExclusionReason::Other,
                ] {
                    ui.selectable_value(
                        &mut self.work.migration.exclusion_reason,
                        reason,
                        exclusion_label(reason),
                    );
                }
            });
        let note_label = ui.label("Exclusion note");
        theme::resizable_multiline_text_edit(
            ui,
            ui.make_persistent_id("migration-exclusion-note"),
            &mut self.work.migration.exclusion_note,
            2,
            Some("Optional context for reviewers"),
        )
        .labelled_by(note_label.id);
        let note_bytes = self.work.migration.exclusion_note.trim().len();
        let note_valid = note_bytes <= MAX_EXCLUSION_NOTE_BYTES
            && (self.work.migration.exclusion_reason != MigrationExclusionReason::Other
                || !self.work.migration.exclusion_note.trim().is_empty());
        ui.small(format!(
            "{note_bytes} of {MAX_EXCLUSION_NOTE_BYTES} bytes{}",
            if self.work.migration.exclusion_reason == MigrationExclusionReason::Other {
                "; required for Other"
            } else {
                ""
            }
        ));
        if !note_valid {
            theme::inline_message(
                ui,
                theme::Intent::Error,
                "Use at most 2000 UTF-8 bytes; the Other reason also requires a note.",
            );
        }
        if ui
            .add_enabled(
                !self.work.migration.busy && note_valid && target_available,
                egui::Button::new("Exclude target & advance"),
            )
            .clicked()
        {
            self.request_exclude_migration_target(group_id);
        }
    }

    fn migration_full_image_actions(&mut self, ui: &mut egui::Ui, expected: u64) {
        ui.separator();
        ui.label(RichText::new("Full-image confirmation").strong());
        ui.label(
            "Object selection is fixed by canonical replay order; canvas clicks do not change it.",
        );
        self.migration_status_list(ui);
        let (confirmation, button_label) = if expected == 0 {
            (
                "Confirm that this image has no canonical guides and needs no skeletons.",
                "Confirm no guides & finish",
            )
        } else {
            (
                "Confirm that every canonical guide is resolved and the full image was checked.",
                "Confirm all guides & finish",
            )
        };
        ui.label(confirmation);
        if theme::primary_button(
            ui,
            !self.work.migration.busy,
            egui::Button::new(button_label),
        )
        .clicked()
        {
            self.request_confirm_migration();
        }
        if self.work.migration.active_pass_id.is_none()
            && expected > 0
            && ui
                .add_enabled(
                    !self.work.migration.busy,
                    egui::Button::new("Start correction pass"),
                )
                .clicked()
        {
            self.request_start_migration_pass();
        }
    }

    fn migration_review_actions(&mut self, ui: &mut egui::Ui) {
        let Some((task_id, targets, confirmation)) =
            self.current_state.as_ref().and_then(|state| {
                let task_id = self.selected_task_id.clone()?;
                let mut targets = state.migration_target_sets.get(&task_id)?.targets.clone();
                targets.sort_by_key(|target| target.sequence_index);
                Some((
                    task_id.clone(),
                    targets,
                    state.migration_confirmations.get(&task_id).cloned(),
                ))
            })
        else {
            return;
        };
        ui.label(RichText::new("Migration review").strong());
        if let Some(target) = targets.get(self.work.migration.review_index) {
            let status = self.migration_disposition(&target.object_group_id);
            theme::compact_metric(
                ui,
                "Review target",
                format!(
                    "{} of {} | {}",
                    self.work.migration.review_index + 1,
                    targets.len(),
                    disposition_label(status.as_ref())
                ),
            );
            ui.label(format!("Object group: {}", target.object_group_id));
            if let Some(MigrationDispositionStatus::Excluded { exclusion }) = status.as_ref() {
                ui.label(format!(
                    "Exclusion reason: {}",
                    exclusion_label(exclusion.reason)
                ));
                if let Some(note) = &exclusion.note {
                    ui.label(format!("Exclusion note: {note}"));
                }
            } else {
                ui.label("Review the skeleton against the read-only canonical guide.");
            }
            let review_target = self.current_state.as_ref().and_then(|state| {
                let disposition = state
                    .migration_dispositions
                    .get(&task_id)?
                    .get(&target.object_group_id)?;
                (!matches!(disposition.status, MigrationDispositionStatus::Pending)).then(|| {
                    labello_client::MigrationReviewTarget::Disposition {
                        object_group_id: target.object_group_id.clone(),
                        disposition_version: disposition.disposition_version,
                    }
                })
            });
            if let Some(review_target) = review_target {
                self.migration_review_buttons(ui, task_id, review_target);
            } else {
                theme::inline_message(
                    ui,
                    theme::Intent::Error,
                    "This migration target is pending or unavailable and cannot be reviewed.",
                );
            }
        } else if let Some(confirmation) = confirmation {
            theme::compact_metric(ui, "Review target", "Full-image confirmation");
            self.migration_status_list(ui);
            self.migration_review_buttons(
                ui,
                task_id,
                labello_client::MigrationReviewTarget::Confirmation {
                    confirmation_hash: confirmation.confirmation_hash,
                },
            );
        } else {
            theme::inline_message(
                ui,
                theme::Intent::Warning,
                "The migration has not received full-image confirmation.",
            );
        }
    }

    fn migration_review_buttons(
        &mut self,
        ui: &mut egui::Ui,
        task_id: labello_domain::TaskId,
        target: labello_client::MigrationReviewTarget,
    ) {
        ui.horizontal_wrapped(|ui| {
            if theme::primary_button(
                ui,
                !self.work.migration.busy,
                egui::Button::new("Approve migration item"),
            )
            .clicked()
            {
                self.request_review_migration(
                    task_id.clone(),
                    target.clone(),
                    labello_domain::ReviewDecision::Approved,
                );
            }
            if theme::danger_button(
                ui,
                !self.work.migration.busy,
                egui::Button::new("Reject migration item"),
            )
            .clicked()
            {
                self.request_review_migration(
                    task_id,
                    target,
                    labello_domain::ReviewDecision::Rejected,
                );
            }
        });
    }

    fn migration_status_list(&self, ui: &mut egui::Ui) {
        let Some((task_id, set, state)) = self.current_state.as_ref().and_then(|state| {
            let task_id = self.selected_task_id.as_ref()?;
            Some((task_id, state.migration_target_sets.get(task_id)?, state))
        }) else {
            return;
        };
        for target in &set.targets {
            let status = state
                .migration_dispositions
                .get(task_id)
                .and_then(|values| values.get(&target.object_group_id))
                .map(|value| &value.status);
            let current = matches!(
                self.work.migration.cursor.as_ref(),
                Some(MigrationCursor::Object { object_group_id, .. })
                    if object_group_id == &target.object_group_id
            );
            let guide_state = state
                .current_annotation(&target.guide_annotation_id)
                .map_or("Guide unavailable", |guide| {
                    if guide.deleted {
                        "Deleted tombstone"
                    } else {
                        "Guide present"
                    }
                });
            let color = if current {
                theme::ACCENT_HOVER
            } else {
                match status {
                    Some(MigrationDispositionStatus::Annotated { .. }) => theme::SUCCESS,
                    Some(MigrationDispositionStatus::Excluded { .. }) => theme::DANGER,
                    _ => theme::TEXT_MUTED,
                }
            };
            ui.label(
                RichText::new(format!(
                    "Guide {}: {}{}; {guide_state}",
                    target.sequence_index + 1,
                    if current { "Current, " } else { "" },
                    disposition_label(status)
                ))
                .strong()
                .color(color),
            );
        }
    }

    fn migration_counts(&self) -> (u64, u64, u64, u64) {
        if let Some(progress) = &self.work.migration.progress {
            return (
                progress.expected,
                progress.annotated,
                progress.excluded,
                progress.pending,
            );
        }
        let Some((task_id, set, state)) = self.current_state.as_ref().and_then(|state| {
            let task_id = self.selected_task_id.as_ref()?;
            Some((task_id, state.migration_target_sets.get(task_id)?, state))
        }) else {
            return (0, 0, 0, 0);
        };
        let mut annotated = 0;
        let mut excluded = 0;
        for target in &set.targets {
            match state.migration_dispositions[task_id][&target.object_group_id].status {
                MigrationDispositionStatus::Annotated { .. } => annotated += 1,
                MigrationDispositionStatus::Excluded { .. } => excluded += 1,
                MigrationDispositionStatus::Pending => {}
            }
        }
        let expected = set.targets.len() as u64;
        (
            expected,
            annotated,
            excluded,
            expected - annotated - excluded,
        )
    }

    pub(crate) fn canonical_migration_review_index(&self) -> usize {
        let Some((task_id, set, state)) = self.current_state.as_ref().and_then(|state| {
            let task_id = self.selected_task_id.as_ref()?;
            Some((task_id, state.migration_target_sets.get(task_id)?, state))
        }) else {
            return 0;
        };
        let mut targets = set.targets.iter().collect::<Vec<_>>();
        targets.sort_by_key(|target| target.sequence_index);
        targets
            .iter()
            .position(|target| {
                let Some(disposition) = state
                    .migration_dispositions
                    .get(task_id)
                    .and_then(|values| values.get(&target.object_group_id))
                else {
                    return true;
                };
                !state.reviews.iter().any(|review| {
                    review.reviewer_user_id == self.config.user_id
                        && review.decision == labello_domain::ReviewDecision::Approved
                        && match (&review.target, &disposition.status) {
                            (
                                labello_domain::ReviewTarget::AnnotationVersion {
                                    annotation_id,
                                    version,
                                },
                                MigrationDispositionStatus::Annotated {
                                    skeleton_annotation_id,
                                    skeleton_version,
                                },
                            ) => {
                                annotation_id == skeleton_annotation_id
                                    && version == skeleton_version
                            }
                            (
                                labello_domain::ReviewTarget::MigrationDisposition {
                                    task_id: reviewed_task,
                                    object_group_id,
                                    disposition_version,
                                },
                                MigrationDispositionStatus::Excluded { .. },
                            ) => {
                                reviewed_task == task_id
                                    && object_group_id == &target.object_group_id
                                    && *disposition_version == disposition.disposition_version
                            }
                            _ => false,
                        }
                })
            })
            .unwrap_or(targets.len())
    }

    fn migration_active_target(&self) -> Option<(ObjectGroupId, labello_domain::MigrationTarget)> {
        if self.view == AppView::Review {
            let task_id = self.selected_task_id.as_ref()?;
            let target = self
                .current_state
                .as_ref()?
                .migration_target_sets
                .get(task_id)?
                .targets
                .get(self.work.migration.review_index)?
                .clone();
            return Some((target.object_group_id.clone(), target));
        }
        let MigrationCursor::Object {
            object_group_id, ..
        } = self.work.migration.cursor.as_ref()?
        else {
            return None;
        };
        let task_id = self.selected_task_id.as_ref()?;
        let target = self
            .current_state
            .as_ref()?
            .migration_target_sets
            .get(task_id)?
            .targets
            .iter()
            .find(|target| &target.object_group_id == object_group_id)?
            .clone();
        Some((object_group_id.clone(), target))
    }

    fn migration_disposition(
        &self,
        group_id: &ObjectGroupId,
    ) -> Option<MigrationDispositionStatus> {
        let task_id = self.selected_task_id.as_ref()?;
        self.current_state
            .as_ref()?
            .migration_dispositions
            .get(task_id)?
            .get(group_id)
            .map(|value| value.status.clone())
    }

    fn ensure_migration_draft(&mut self, group_id: &ObjectGroupId) {
        if self.work.migration.draft_group.as_ref() == Some(group_id) {
            return;
        }
        let existing = self.selected_task_id.as_ref().and_then(|task_id| {
            let state = self.current_state.as_ref()?;
            let disposition = state.migration_dispositions.get(task_id)?.get(group_id)?;
            let MigrationDispositionStatus::Annotated {
                skeleton_annotation_id,
                ..
            } = &disposition.status
            else {
                return None;
            };
            let annotation = state.current_annotation(skeleton_annotation_id)?;
            match &annotation.geometry {
                AnnotationGeometry::Skeleton(skeleton) if !annotation.deleted => {
                    Some(skeleton.clone())
                }
                _ => None,
            }
        });
        let names: Vec<String> = self
            .selected_task()
            .and_then(|task| task.skeleton.as_ref())
            .map(|spec| {
                spec.keypoints
                    .iter()
                    .map(|point| point.name.clone())
                    .collect()
            })
            .unwrap_or_default();
        self.work.migration.keypoint_index = existing
            .as_ref()
            .map(|skeleton| skeleton.keypoints.len())
            .unwrap_or(0);
        self.work.migration.draft =
            Some(existing.unwrap_or_else(|| ManualMigrationState::empty_skeleton(names)));
        self.work.migration.draft_group = Some(group_id.clone());
    }

    fn place_migration_keypoint(&mut self, point: NormalizedPoint) {
        let Some((group_id, _)) = self.migration_active_target() else {
            return;
        };
        self.ensure_migration_draft(&group_id);
        let Some(keypoint) = self
            .work
            .migration
            .draft
            .as_mut()
            .and_then(|draft| draft.keypoints.get_mut(self.work.migration.keypoint_index))
        else {
            return;
        };
        keypoint.point = Some(point);
        keypoint.state = if self.work.migration.next_hidden {
            KeypointState::Hidden
        } else {
            KeypointState::Visible
        };
        self.work.migration.keypoint_index += 1;
        self.work.migration.next_hidden = false;
    }

    pub(crate) fn skip_migration_keypoint(&mut self) {
        let index = self.work.migration.keypoint_index;
        let allowed = self
            .selected_task()
            .and_then(|task| task.skeleton.as_ref())
            .is_some_and(|spec| {
                spec.allow_absent
                    && spec
                        .keypoints
                        .get(index)
                        .is_some_and(|keypoint| !keypoint.required)
            });
        if allowed {
            self.work.migration.keypoint_index += 1;
            self.work.migration.next_hidden = false;
        }
    }

    fn migration_draft_valid(&self) -> bool {
        let Some((draft, spec)) = self
            .work
            .migration
            .draft
            .as_ref()
            .zip(self.selected_task().and_then(|task| task.skeleton.as_ref()))
        else {
            return false;
        };
        self.work.migration.keypoint_index >= draft.keypoints.len()
            && spec.keypoints.iter().all(|required| {
                !required.required
                    || draft
                        .keypoints
                        .iter()
                        .any(|point| point.name == required.name && point.point.is_some())
            })
    }

    fn migration_expectation(
        &self,
        group_id: &ObjectGroupId,
    ) -> Option<labello_client::MigrationTargetExpectation> {
        let task_id = self.selected_task_id.as_ref()?;
        let state = self.current_state.as_ref()?;
        let target = state
            .migration_target_sets
            .get(task_id)?
            .targets
            .iter()
            .find(|target| &target.object_group_id == group_id)?;
        let guide = state.current_annotation(&target.guide_annotation_id)?;
        let disposition = state.migration_dispositions.get(task_id)?.get(group_id)?;
        let skeleton_version = match disposition.status {
            MigrationDispositionStatus::Annotated {
                skeleton_version, ..
            } => Some(skeleton_version),
            _ => None,
        };
        Some(labello_client::MigrationTargetExpectation {
            object_group_id: group_id.clone(),
            expected_guide_annotation_version: guide.version,
            expected_guide_deleted: guide.deleted,
            expected_disposition_version: disposition.disposition_version,
            expected_skeleton_version: skeleton_version,
        })
    }

    fn request_save_migration_skeleton(&mut self, group_id: ObjectGroupId) {
        let Some((assignment, target, skeleton)) = self
            .assignment
            .clone()
            .zip(self.migration_expectation(&group_id))
            .zip(self.work.migration.draft.clone())
            .map(|((assignment, target), skeleton)| (assignment, target, skeleton))
        else {
            return;
        };
        self.queue_migration_action(MigrationAction::SaveSkeleton(
            labello_client::SaveMigrationSkeletonRequest {
                assignment_id: assignment.assignment_id,
                pass_id: self.work.migration.active_pass_id.clone(),
                target,
                skeleton,
            },
        ));
    }

    pub(crate) fn request_exclude_migration_target(&mut self, group_id: ObjectGroupId) {
        let Some((assignment, target)) = self
            .assignment
            .clone()
            .zip(self.migration_expectation(&group_id))
        else {
            return;
        };
        self.queue_migration_action(MigrationAction::Exclude(
            labello_client::ExcludeMigrationTargetRequest {
                assignment_id: assignment.assignment_id,
                pass_id: self.work.migration.active_pass_id.clone(),
                target,
                reason: self.work.migration.exclusion_reason,
                note: (!self.work.migration.exclusion_note.trim().is_empty())
                    .then(|| self.work.migration.exclusion_note.trim().to_string()),
            },
        ));
    }

    fn request_reopen_migration_target(&mut self, group_id: ObjectGroupId) {
        let Some((assignment, target)) = self
            .assignment
            .clone()
            .zip(self.migration_expectation(&group_id))
        else {
            return;
        };
        self.queue_migration_action(MigrationAction::Reopen(
            labello_client::ReopenMigrationTargetRequest {
                assignment_id: assignment.assignment_id,
                pass_id: self.work.migration.active_pass_id.clone(),
                target,
            },
        ));
    }

    fn request_keep_migration_target(&mut self, group_id: ObjectGroupId) {
        let Some((assignment, pass_id, target)) = self
            .assignment
            .clone()
            .zip(self.work.migration.active_pass_id.clone())
            .zip(self.migration_expectation(&group_id))
            .map(|((assignment, pass_id), target)| (assignment, pass_id, target))
        else {
            return;
        };
        self.queue_migration_action(MigrationAction::Keep(
            labello_client::KeepMigrationTargetRequest {
                assignment_id: assignment.assignment_id,
                pass_id,
                target,
            },
        ));
    }

    fn request_start_migration_pass(&mut self) {
        let Some((assignment, task_id, target_hash, state_hash)) =
            self.assignment.clone().and_then(|assignment| {
                let task_id = self.selected_task_id.clone()?;
                let state = self.current_state.as_ref()?;
                Some((
                    assignment,
                    task_id.clone(),
                    state
                        .migration_target_sets
                        .get(&task_id)?
                        .target_set_hash
                        .clone(),
                    state.current_migration_state_hash(&task_id).ok()?,
                ))
            })
        else {
            return;
        };
        self.queue_migration_action(MigrationAction::StartPass(
            labello_client::StartMigrationPassRequest {
                assignment_id: assignment.assignment_id,
                task_id,
                expected_target_set_hash: target_hash,
                expected_state_hash: state_hash,
            },
        ));
    }

    fn request_confirm_migration(&mut self) {
        let Some((assignment, task_id, target_hash, state_hash)) =
            self.assignment.clone().and_then(|assignment| {
                let task_id = self.selected_task_id.clone()?;
                let state = self.current_state.as_ref()?;
                Some((
                    assignment,
                    task_id.clone(),
                    state
                        .migration_target_sets
                        .get(&task_id)?
                        .target_set_hash
                        .clone(),
                    state.current_migration_state_hash(&task_id).ok()?,
                ))
            })
        else {
            return;
        };
        let Ok(confirmation_hash) =
            labello_domain::migration_confirmation_hash(&target_hash, &state_hash)
        else {
            return;
        };
        self.queue_migration_action(MigrationAction::Confirm(
            labello_client::ConfirmMigrationRequest {
                assignment_id: assignment.assignment_id,
                task_id,
                target_set_hash: target_hash,
                state_hash,
                confirmation_hash,
            },
        ));
    }

    fn request_review_migration(
        &mut self,
        task_id: labello_domain::TaskId,
        target: labello_client::MigrationReviewTarget,
        decision: labello_domain::ReviewDecision,
    ) {
        let Some(assignment) = self.assignment.clone() else {
            return;
        };
        self.queue_migration_action(MigrationAction::Review(
            labello_client::ReviewMigrationRequest {
                assignment_id: assignment.assignment_id,
                task_id,
                target,
                decision,
                comment: None,
            },
        ));
    }

    fn queue_migration_action(&mut self, action: MigrationAction) {
        let Some(image_id) = self
            .current
            .as_ref()
            .map(|current| current.image.image_id.clone())
        else {
            return;
        };
        let operation_id = self.next_operation();
        let request = self.operation_identity(operation_id, self.config.dataset_id.clone());
        self.work.migration.busy = true;
        self.work.migration.error = None;
        self.queue_command(UiCommand::Migration {
            request,
            dataset_id: self.config.dataset_id.clone(),
            image_id,
            action,
            idempotency_key: format!("ui-migration-{operation_id}"),
        });
    }
}

fn disposition_label(status: Option<&MigrationDispositionStatus>) -> &'static str {
    match status {
        Some(MigrationDispositionStatus::Pending) => "Pending",
        Some(MigrationDispositionStatus::Annotated { .. }) => "Skeleton annotated",
        Some(MigrationDispositionStatus::Excluded { .. }) => "Excluded",
        None => "Unavailable",
    }
}

fn exclusion_label(reason: MigrationExclusionReason) -> &'static str {
    match reason {
        MigrationExclusionReason::NoValidSkeleton => "No valid skeleton",
        MigrationExclusionReason::InsufficientVisibleFeatures => "Insufficient visible features",
        MigrationExclusionReason::InvalidSourceBox => "Invalid source box",
        MigrationExclusionReason::DuplicateSourceObject => "Duplicate source object",
        MigrationExclusionReason::ObjectNotPresent => "Object not present",
        MigrationExclusionReason::Other => "Other",
    }
}
