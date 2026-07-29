use labello_domain::{
    AnnotationGeometry, AnnotationOrigin, AnnotationType, AnnotationVersion, KeypointAnnotation,
    KeypointState, MigrationCursor, MigrationDispositionStatus, MigrationExclusionReason,
    MigrationPassId, NormalizedPoint, ObjectGroupId, RevisionSource, SkeletonGeometry,
};

use eframe::egui::{self, RichText};

use crate::{
    app::{AppView, LabelloApp, MigrationAction, UiCommand},
    canvas::{CanvasAction, CanvasAnnotationStyle, CanvasInteraction, show_canvas_colored},
    panels::shortcut_button_label,
    theme,
};

const MAX_EXCLUSION_NOTE_BYTES: usize = 2_000;

#[derive(Clone, Debug)]
enum MigrationPrimaryAction {
    SaveSkeleton(ObjectGroupId),
    AddSkeleton,
    KeepDisposition(ObjectGroupId),
    Confirm { no_guides: bool },
}

impl MigrationPrimaryAction {
    fn label(&self, compact: bool) -> &'static str {
        match (self, compact) {
            (Self::SaveSkeleton(_), true) => "Save & next",
            (Self::SaveSkeleton(_), false) => "Save skeleton & advance",
            (Self::AddSkeleton, true) => "Save object",
            (Self::AddSkeleton, false) => "Save missing object",
            (Self::KeepDisposition(_), true) => "Keep & next",
            (Self::KeepDisposition(_), false) => "Keep current & advance",
            (Self::Confirm { no_guides: true }, true) => "Confirm & finish",
            (Self::Confirm { no_guides: true }, false) => "Confirm no guides & finish",
            (Self::Confirm { no_guides: false }, true) => "Confirm & finish",
            (Self::Confirm { no_guides: false }, false) => "Confirm all guides & finish",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ManualMigrationState {
    pub cursor: Option<MigrationCursor>,
    pub inspected_group_id: Option<ObjectGroupId>,
    pub pending_revisit_target: Option<ObjectGroupId>,
    pub pending_activate_target: Option<ObjectGroupId>,
    pub active_pass_id: Option<MigrationPassId>,
    pub adding_missing_object: bool,
    pub draft: Option<SkeletonGeometry>,
    pub draft_group: Option<ObjectGroupId>,
    pub draft_dirty: bool,
    pub keypoint_index: usize,
    pub next_hidden: bool,
    pub exclusion_reason: MigrationExclusionReason,
    pub exclusion_note: String,
    pub exclusion_dirty: bool,
    pub review_index: usize,
    pub progress: Option<labello_client::ManualMigrationProgress>,
    pub busy: bool,
    pub error: Option<String>,
}

impl Default for ManualMigrationState {
    fn default() -> Self {
        Self {
            cursor: None,
            inspected_group_id: None,
            pending_revisit_target: None,
            pending_activate_target: None,
            active_pass_id: None,
            adding_missing_object: false,
            draft: None,
            draft_group: None,
            draft_dirty: false,
            keypoint_index: 0,
            next_hidden: false,
            exclusion_reason: MigrationExclusionReason::NoValidSkeleton,
            exclusion_note: String::new(),
            exclusion_dirty: false,
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
                .work
                .current_state
                .as_ref()
                .is_some_and(|state| state.migration_target_sets.contains_key(&task.task_id))
    }

    pub(crate) fn sync_manual_migration(&mut self) {
        if !self.manual_migration_active() || self.work.migration.busy {
            return;
        }
        let Some(task_id) = self.work.selected_task_id.clone() else {
            return;
        };
        if self.work.migration.progress.is_some() {
            if self.view == AppView::Review {
                self.work.migration.review_index = self.canonical_migration_review_index();
            }
            return;
        }
        if self.work.migration.active_pass_id.is_none() {
            self.work.migration.active_pass_id =
                self.work.current_state.as_ref().and_then(|state| {
                    let assignment_id = &self.work.assignment.as_ref()?.assignment_id;
                    state
                        .migration_passes
                        .values()
                        .filter(|pass| {
                            pass.task_id == task_id && &pass.assignment_id == assignment_id
                        })
                        .max_by_key(|pass| pass.started_at)
                        .map(|pass| pass.pass_id.clone())
                });
        }
        let active_pass = self.work.migration.active_pass_id.as_ref();
        let cursor = self
            .work
            .current_state
            .as_ref()
            .and_then(|state| state.migration_cursor(&task_id, active_pass).ok());
        if self.work.migration.cursor != cursor {
            self.work.migration.cursor = cursor;
            self.work.migration.inspected_group_id = None;
            self.work.migration.pending_revisit_target = None;
            self.work.migration.pending_activate_target = None;
            self.work.migration.adding_missing_object = false;
            self.work.migration.draft = None;
            self.work.migration.draft_group = None;
            self.work.migration.draft_dirty = false;
            self.work.migration.keypoint_index = 0;
            self.work.migration.next_hidden = false;
            self.work.migration.exclusion_note.clear();
            self.work.migration.exclusion_dirty = false;
        }
        if self.view == AppView::Review {
            self.work.migration.review_index = self.canonical_migration_review_index();
        }
    }

    pub(crate) fn migration_workspace_canvas(&mut self, ui: &mut egui::Ui) {
        let Some(current) = self.work.current.clone() else {
            return;
        };
        let texture = self.work.current_texture.clone();
        let Some(task) = self.selected_task().cloned() else {
            return;
        };
        let Some(state) = self.work.current_state.clone() else {
            return;
        };
        let active = self.migration_active_target();
        let guide = active.as_ref().and_then(|(_, target)| {
            state
                .current_annotation(&target.guide_annotation_id)
                .cloned()
        });
        self.work.canvas.set_review_focus(guide.as_ref());
        let task_id = task.task_id.clone();
        let mut annotations = Vec::new();
        let mut annotation_styles = std::collections::BTreeMap::new();
        let mut selectable_guides = std::collections::BTreeSet::new();
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
                let guide_style = migration_guide_style(current, status);
                if let Some(guide) = state.current_annotation(&target.guide_annotation_id) {
                    let mut rendered = guide.clone();
                    if rendered.deleted {
                        rendered.deleted = false;
                        annotation_styles.insert(
                            rendered.annotation_id.clone(),
                            if current {
                                CanvasAnnotationStyle::solid(theme::DANGER)
                            } else {
                                CanvasAnnotationStyle::dashed(theme::DANGER)
                            },
                        );
                    } else {
                        annotation_styles.insert(rendered.annotation_id.clone(), guide_style);
                    }
                    if !current
                        && self.view == AppView::Annotate
                        && (migration_target_is_unmigrated(status)
                            || matches!(status, Some(MigrationDispositionStatus::Excluded { .. })))
                    {
                        selectable_guides.insert(rendered.annotation_id.clone());
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
                    annotation_styles.insert(
                        skeleton.annotation_id.clone(),
                        CanvasAnnotationStyle::solid(if current {
                            theme::ACCENT_HOVER
                        } else {
                            theme::SUCCESS
                        }),
                    );
                }
            }
        }
        for skeleton in state.active_annotations().filter(|annotation| {
            annotation.task_id == task_id
                && annotation.object_group_id.is_none()
                && annotation.annotation_type == AnnotationType::Skeleton
        }) {
            annotations.push(skeleton.clone());
            annotation_styles.insert(
                skeleton.annotation_id.clone(),
                CanvasAnnotationStyle::solid(theme::SUCCESS),
            );
        }
        let mut selected = None;
        if let Some((group_id, target)) = active.as_ref()
            && self.work.migration.draft_group.as_ref() == Some(group_id)
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
            annotation_styles.insert(
                selected.clone().unwrap(),
                CanvasAnnotationStyle::solid(theme::ACCENT_HOVER),
            );
        }
        if self.work.migration.adding_missing_object
            && let Some(draft) = self.work.migration.draft.clone()
        {
            let annotation_id =
                labello_domain::AnnotationId::from("ann_migration_discovered_draft");
            selected = Some(annotation_id.clone());
            annotations.push(AnnotationVersion {
                annotation_id: annotation_id.clone(),
                version: 1,
                object_group_id: None,
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
            annotation_styles.insert(
                annotation_id,
                CanvasAnnotationStyle::solid(theme::ACCENT_HOVER),
            );
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
        let skeleton_editable = (guide.as_ref().is_some_and(|guide| !guide.deleted)
            && matches!(
                self.work.migration.cursor,
                Some(MigrationCursor::Object { .. })
            ))
            || (self.work.migration.adding_missing_object
                && matches!(self.work.migration.cursor, Some(MigrationCursor::FullImage)));
        let interaction = CanvasInteraction {
            editable: !self.work.migration.busy
                && self.work.migration.inspected_group_id.is_none()
                && (skeleton_editable || !selectable_guides.is_empty()),
            allow_create: skeleton_editable,
            allow_selection: true,
            edit_keypoints: false,
            selected_keypoint: None,
        };
        let action = show_canvas_colored(
            ui,
            &mut self.work.canvas,
            texture.as_ref(),
            &annotations,
            [current.image.width, current.image.height],
            false,
            selected.as_ref(),
            interaction,
            &edges,
            &[],
            theme::ANNOTATION,
            &annotation_styles,
            Some(&selectable_guides),
        );
        match action {
            Some(CanvasAction::PlaceKeypoint(point)) => self.place_migration_keypoint(point),
            Some(CanvasAction::Select(annotation_id)) => {
                self.activate_migration_from_inactive_guide(&annotation_id);
            }
            _ => {}
        }
    }

    pub(crate) fn manual_migration_actions(
        &mut self,
        ui: &mut egui::Ui,
        show_primary_actions: bool,
    ) {
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
            self.migration_review_actions(ui, show_primary_actions);
            return;
        }
        let (expected, annotated, excluded, pending) = self.migration_counts();
        if expected == 0 {
            ui.label(RichText::new("No canonical guides").color(theme::TEXT_MUTED));
        } else {
            let resolved = annotated + excluded;
            ui.label(
                RichText::new(format!(
                    "{resolved} of {expected} resolved · {pending} remaining"
                ))
                .strong(),
            );
            if resolved > 0 {
                ui.small(format!("{annotated} skeletons · {excluded} excluded"));
            }
        }
        if let Some(group_id) = self.work.migration.inspected_group_id.clone() {
            self.migration_inspection_actions(ui, group_id, expected, show_primary_actions);
            return;
        }
        match self.work.migration.cursor.clone() {
            Some(MigrationCursor::Object {
                object_group_id,
                sequence_index,
            }) => self.migration_object_actions(
                ui,
                object_group_id,
                sequence_index,
                expected,
                show_primary_actions,
            ),
            Some(MigrationCursor::FullImage) => {
                self.migration_full_image_actions(ui, expected, show_primary_actions)
            }
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
        show_primary_action: bool,
    ) {
        ui.separator();
        let status = self.migration_disposition(&group_id);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(format!("Object {} of {expected}", sequence_index + 1)).strong(),
            );
            theme::badge(
                ui,
                disposition_label(status.as_ref()),
                disposition_intent(status.as_ref()),
            );
        });
        ui.small("Canonical bounding-box guide · read only");
        let guide = self.work.current_state.as_ref().and_then(|state| {
            let target = state
                .migration_target_sets
                .get(self.work.selected_task_id.as_ref()?)?
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
            && ui.small_button("Refocus box").clicked()
        {
            self.work.canvas.focus_annotation(guide);
        }
        let dependency = self
            .work
            .current_state
            .as_ref()
            .and_then(|state| {
                state
                    .migration_dependencies
                    .get(self.work.selected_task_id.as_ref()?)
                    .and_then(|markers| markers.get(&group_id))
            })
            .map(|marker| marker.kind);
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
        } else if dependency
            .is_some_and(|kind| kind != labello_domain::MigrationDependencyKind::ManualSelection)
        {
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
            && matches!(status, Some(MigrationDispositionStatus::Excluded { .. }))
            && ui
                .add_enabled(
                    !self.work.migration.busy && guide_valid,
                    egui::Button::new("Reopen excluded target"),
                )
                .clicked()
        {
            self.request_reopen_migration_target(group_id.clone());
        }
        ui.separator();
        ui.label(RichText::new("Skeleton").strong());
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
            self.work.migration.draft_dirty = true;
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
        if let Some(name) = next_name.as_ref() {
            ui.label(RichText::new(format!("Place {name}")).strong());
            ui.small(format!(
                "{placed} of {total} placed · click its position inside the box"
            ));
        } else {
            ui.label(RichText::new("Skeleton ready").strong());
            ui.small(format!("{placed} of {total} keypoints placed"));
        }
        if self.work.migration.keypoint_index > 0
            && ui
                .add_enabled(
                    guide_valid && !self.work.migration.busy,
                    egui::Button::new("Undo last keypoint").shortcut_text(
                        self.shortcut_text(ui.ctx(), labello_domain::UserAction::UndoEdit),
                    ),
                )
                .clicked()
        {
            self.remove_last_migration_keypoint();
        }
        if let Some(name) = next_name {
            let (allow_hidden, can_absent) = self
                .selected_task()
                .and_then(|task| task.skeleton.as_ref())
                .map(|spec| {
                    (
                        spec.allow_hidden,
                        spec.allow_absent
                            && spec
                                .keypoints
                                .get(self.work.migration.keypoint_index)
                                .is_some_and(|keypoint| !keypoint.required),
                    )
                })
                .unwrap_or_default();
            ui.horizontal_wrapped(|ui| {
                if allow_hidden {
                    ui.checkbox(
                        &mut self.work.migration.next_hidden,
                        format!("Place {name} as hidden"),
                    );
                }
                if can_absent
                    && ui
                        .add(
                            egui::Button::new(format!("Mark {name} absent")).shortcut_text(
                                self.shortcut_text(
                                    ui.ctx(),
                                    labello_domain::UserAction::MarkKeypointAbsent,
                                ),
                            ),
                        )
                        .clicked()
                {
                    self.skip_migration_keypoint();
                }
            });
        }
        if show_primary_action {
            self.migration_primary_button(ui, false);
            self.migration_object_navigation_button(ui);
            self.migration_assignment_section(ui);
        }
        ui.separator();
        let exclusion = ui
            .scope(|ui| {
                ui.style_mut().animation_time = 0.0;
                egui::CollapsingHeader::new("Can't annotate this object")
                    .id_salt(("migration-exclusion", group_id.as_str()))
                    .show(ui, |ui| {
                        self.migration_exclusion_actions(ui, &group_id, target_available);
                    })
            })
            .inner;
        if exclusion.header_response.clicked()
            && let Some(body) = exclusion.body_response
        {
            body.scroll_to_me(Some(egui::Align::Center));
        }
    }

    fn migration_inspection_actions(
        &mut self,
        ui: &mut egui::Ui,
        group_id: ObjectGroupId,
        expected: u64,
        show_workspace_actions: bool,
    ) {
        let Some(target) = self.migration_target(&group_id) else {
            self.work.migration.inspected_group_id = None;
            return;
        };
        let status = self.migration_disposition(&group_id);
        ui.separator();
        ui.label(
            RichText::new(format!(
                "Reviewing object {} of {expected} · Read only",
                target.sequence_index + 1
            ))
            .strong(),
        );
        theme::badge(
            ui,
            disposition_label(status.as_ref()),
            disposition_intent(status.as_ref()),
        );
        ui.small("Browsing does not change migration progress.");
        if let Some(MigrationDispositionStatus::Excluded { exclusion }) = status.as_ref() {
            ui.label(format!(
                "Exclusion reason: {}",
                exclusion_label(exclusion.reason)
            ));
            if let Some(note) = exclusion.note.as_deref() {
                ui.label(format!("Exclusion note: {note}"));
            }
        } else {
            ui.label("Review the saved skeleton against the read-only canonical guide.");
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    self.can_edit_previous_migration_object(),
                    egui::Button::new("Previous object").shortcut_text(
                        self.shortcut_text(
                            ui.ctx(),
                            labello_domain::UserAction::SelectPreviousObject,
                        ),
                    ),
                )
                .clicked()
            {
                self.edit_previous_migration_object();
            }
            let returns_to_current = self.inspection_next_returns_to_current();
            if ui
                .add(
                    egui::Button::new(if returns_to_current {
                        "Return to current object"
                    } else {
                        "Next object"
                    })
                    .shortcut_text(
                        self.shortcut_text(ui.ctx(), labello_domain::UserAction::SelectNextObject),
                    ),
                )
                .clicked()
            {
                self.inspect_migration_object(1);
            }
        });
        if theme::primary_button(
            ui,
            !self.work.migration.busy && self.migration_expectation(&group_id).is_some(),
            egui::Button::new("Edit this object"),
        )
        .on_hover_text("Make this resolved object the audited canonical correction target.")
        .clicked()
        {
            self.begin_revisit_migration_target(group_id);
        }
        if show_workspace_actions {
            self.migration_assignment_section(ui);
        }
    }

    fn migration_exclusion_actions(
        &mut self,
        ui: &mut egui::Ui,
        group_id: &ObjectGroupId,
        target_available: bool,
    ) {
        ui.label(
            RichText::new("Use only when a valid skeleton cannot be placed.")
                .color(theme::TEXT_MUTED),
        );
        let reason_label = ui.label("Reason");
        let previous_reason = self.work.migration.exclusion_reason;
        egui::ComboBox::from_id_salt("migration-exclusion-reason")
            .width(ui.available_width())
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
            })
            .response
            .labelled_by(reason_label.id);
        if self.work.migration.exclusion_reason != previous_reason {
            self.work.migration.exclusion_dirty = true;
        }
        let note_label = ui.label(
            if self.work.migration.exclusion_reason == MigrationExclusionReason::Other {
                "Note (required)"
            } else {
                "Note (optional)"
            },
        );
        if theme::resizable_multiline_text_edit(
            ui,
            ui.make_persistent_id("migration-exclusion-note"),
            &mut self.work.migration.exclusion_note,
            2,
            Some("Context for reviewers"),
        )
        .labelled_by(note_label.id)
        .changed()
        {
            self.work.migration.exclusion_dirty = true;
        }
        let note_bytes = self.work.migration.exclusion_note.trim().len();
        let note_valid = note_bytes <= MAX_EXCLUSION_NOTE_BYTES
            && (self.work.migration.exclusion_reason != MigrationExclusionReason::Other
                || !self.work.migration.exclusion_note.trim().is_empty());
        if note_bytes > 0 || !note_valid {
            ui.small(format!("{note_bytes} of {MAX_EXCLUSION_NOTE_BYTES} bytes"));
        }
        if !note_valid {
            theme::inline_message(
                ui,
                theme::Intent::Error,
                "Use at most 2000 UTF-8 bytes; the Other reason also requires a note.",
            );
        }
        if theme::danger_button(
            ui,
            !self.work.migration.busy && note_valid && target_available,
            egui::Button::new("Exclude & advance"),
        )
        .clicked()
        {
            self.request_exclude_migration_target(group_id.clone());
        }
    }

    fn migration_full_image_actions(
        &mut self,
        ui: &mut egui::Ui,
        expected: u64,
        show_primary_action: bool,
    ) {
        ui.separator();
        ui.label(RichText::new("Full-image confirmation").strong());
        if self.work.migration.adding_missing_object {
            self.migration_missing_object_actions(ui, show_primary_action);
            return;
        }
        if expected > 0 {
            egui::CollapsingHeader::new(format!("Review {expected} resolved objects"))
                .id_salt("migration-resolved-objects")
                .show(ui, |ui| self.migration_status_list(ui));
        }
        let discovered = self.discovered_migration_skeleton_count();
        if discovered > 0 {
            ui.small(format!(
                "{discovered} additional {} added during full-image review",
                if discovered == 1 { "object" } else { "objects" }
            ));
        }
        let confirmation = if expected == 0 {
            "Confirm that this image has no canonical guides and needs no skeletons."
        } else {
            "Confirm that every imported guide is resolved and that any objects missing from the import were added."
        };
        ui.label(confirmation);
        if show_primary_action {
            self.migration_primary_button(ui, false);
            self.migration_object_navigation_button(ui);
            self.migration_assignment_section(ui);
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

    fn migration_missing_object_actions(&mut self, ui: &mut egui::Ui, show_primary_action: bool) {
        ui.label(RichText::new("Adding an object missing from the import").strong());
        ui.small("Place its keypoints on the full image. No imported box is required.");
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
        if let Some(name) = next_name.as_ref() {
            ui.label(RichText::new(format!("Place {name}")).strong());
            ui.small(format!(
                "{placed} of {total} placed · click its position on the image"
            ));
        } else {
            ui.label(RichText::new("Skeleton ready").strong());
            ui.small(format!("{placed} of {total} keypoints placed"));
        }
        if self.work.migration.keypoint_index > 0
            && ui
                .add_enabled(
                    !self.work.migration.busy,
                    egui::Button::new("Undo last keypoint").shortcut_text(
                        self.shortcut_text(ui.ctx(), labello_domain::UserAction::UndoEdit),
                    ),
                )
                .clicked()
        {
            self.remove_last_migration_keypoint();
        }
        if let Some(name) = next_name {
            let (allow_hidden, can_absent) = self
                .selected_task()
                .and_then(|task| task.skeleton.as_ref())
                .map(|spec| {
                    (
                        spec.allow_hidden,
                        spec.allow_absent
                            && spec
                                .keypoints
                                .get(self.work.migration.keypoint_index)
                                .is_some_and(|keypoint| !keypoint.required),
                    )
                })
                .unwrap_or_default();
            ui.horizontal_wrapped(|ui| {
                if allow_hidden {
                    ui.checkbox(
                        &mut self.work.migration.next_hidden,
                        format!("Place {name} as hidden"),
                    );
                }
                if can_absent && ui.button(format!("Mark {name} absent")).clicked() {
                    self.skip_migration_keypoint();
                }
            });
        }
        if show_primary_action {
            self.migration_primary_button(ui, false);
            self.migration_assignment_section(ui);
        }
    }

    fn migration_review_actions(&mut self, ui: &mut egui::Ui, show_primary_actions: bool) {
        let Some((task_id, targets, confirmation)) =
            self.work.current_state.as_ref().and_then(|state| {
                let task_id = self.work.selected_task_id.clone()?;
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
            let review_target = self.work.current_state.as_ref().and_then(|state| {
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
            match review_target {
                Some(review_target) if show_primary_actions => {
                    self.migration_review_buttons(ui, task_id, review_target, false, false);
                }
                Some(_) => {}
                None => {
                    theme::inline_message(
                        ui,
                        theme::Intent::Error,
                        "This migration target is pending or unavailable and cannot be reviewed.",
                    );
                }
            }
        } else if let Some(confirmation) = confirmation {
            theme::compact_metric(ui, "Review target", "Full-image confirmation");
            egui::CollapsingHeader::new("Resolved objects")
                .id_salt("migration-review-resolved-objects")
                .show(ui, |ui| self.migration_status_list(ui));
            if show_primary_actions {
                self.migration_review_buttons(
                    ui,
                    task_id,
                    labello_client::MigrationReviewTarget::Confirmation {
                        confirmation_hash: confirmation.confirmation_hash,
                    },
                    false,
                    false,
                );
            }
        } else {
            theme::inline_message(
                ui,
                theme::Intent::Warning,
                "The migration has not received full-image confirmation.",
            );
        }
    }

    pub(crate) fn migration_review_buttons(
        &mut self,
        ui: &mut egui::Ui,
        task_id: labello_domain::TaskId,
        target: labello_client::MigrationReviewTarget,
        compact: bool,
        shortcut_only: bool,
    ) {
        let approve_shortcut =
            self.shortcut_text(ui.ctx(), labello_domain::UserAction::AcceptReviewObject);
        let reject_shortcut =
            self.shortcut_text(ui.ctx(), labello_domain::UserAction::RejectReviewObject);
        let (approve, reject) = if shortcut_only {
            (
                shortcut_button_label(&approve_shortcut, "Accept"),
                shortcut_button_label(&reject_shortcut, "Reject"),
            )
        } else if compact {
            ("Accept".to_string(), "Reject".to_string())
        } else {
            (
                "Approve migration item".to_string(),
                "Reject migration item".to_string(),
            )
        };
        let button_width =
            compact.then(|| ((ui.available_width() - ui.spacing().item_spacing.x) / 2.0).max(44.0));
        let approve_button = egui::Button::new(approve).min_size(egui::vec2(
            button_width.unwrap_or_default(),
            if compact { 44.0 } else { 0.0 },
        ));
        let reject_button = egui::Button::new(reject).min_size(egui::vec2(
            button_width.unwrap_or_default(),
            if compact { 44.0 } else { 0.0 },
        ));
        if theme::primary_button(ui, !self.work.migration.busy, approve_button)
            .on_hover_text(format!("Accept migration item ({approve_shortcut})"))
            .clicked()
        {
            self.request_review_migration(
                task_id.clone(),
                target.clone(),
                labello_domain::ReviewDecision::Approved,
            );
        }
        if theme::danger_button(ui, !self.work.migration.busy, reject_button)
            .on_hover_text(format!("Reject migration item ({reject_shortcut})"))
            .clicked()
        {
            self.request_review_migration(
                task_id,
                target,
                labello_domain::ReviewDecision::Rejected,
            );
        }
    }

    pub(crate) fn migration_workspace_actions(&mut self, ui: &mut egui::Ui, compact: bool) {
        if self.view == AppView::Review {
            if let Some((task_id, target)) = self.current_migration_review_target() {
                self.migration_review_buttons(ui, task_id, target, false, false);
            }
            return;
        }
        if let Some(group_id) = self.work.migration.inspected_group_id.clone() {
            if theme::primary_button(
                ui,
                !self.work.migration.busy && self.migration_expectation(&group_id).is_some(),
                egui::Button::new(if compact {
                    "Edit object"
                } else {
                    "Edit this object"
                }),
            )
            .clicked()
            {
                self.begin_revisit_migration_target(group_id);
            }
            self.migration_object_navigation_button(ui);
            if !compact
                && ui
                    .add(
                        egui::Button::new(if self.inspection_next_returns_to_current() {
                            "Return current"
                        } else {
                            "Next object"
                        })
                        .shortcut_text(
                            self.shortcut_text(
                                ui.ctx(),
                                labello_domain::UserAction::SelectNextObject,
                            ),
                        ),
                    )
                    .clicked()
            {
                self.inspect_migration_object(1);
            }
        } else {
            let adding_missing_object = self.work.migration.adding_missing_object;
            if (adding_missing_object || self.migration_can_add_missing_object())
                && ui
                    .add_enabled(
                        !self.work.migration.busy,
                        egui::Button::new(if adding_missing_object {
                            "Cancel adding object"
                        } else {
                            "Add missing object"
                        })
                        .shortcut_text(
                            self.shortcut_text(
                                ui.ctx(),
                                labello_domain::UserAction::AddMissingObject,
                            ),
                        ),
                    )
                    .on_hover_text(if adding_missing_object {
                        "Discard this unsaved missing-object skeleton."
                    } else {
                        "Add a skeleton for an object that had no imported guide."
                    })
                    .clicked()
            {
                self.trigger_missing_migration_object_action();
            }
            self.migration_primary_button(ui, compact);
            if self.migration_keypoint_undo_available() {
                let response = ui.add_enabled(
                    self.migration_keypoint_undo_enabled(),
                    egui::Button::new(if compact {
                        "Undo"
                    } else {
                        "Undo last keypoint"
                    }),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Undo last keypoint")
                });
                if response.clicked() {
                    self.remove_last_migration_keypoint();
                }
            }
            self.migration_object_navigation_button(ui);
        }
        if compact {
            self.migration_more_actions(ui);
        } else {
            self.migration_assignment_buttons(ui);
        }
    }

    fn migration_can_add_missing_object(&self) -> bool {
        self.view == AppView::Annotate
            && self.work.migration.inspected_group_id.is_none()
            && !self.work.migration.adding_missing_object
            && matches!(self.work.migration.cursor, Some(MigrationCursor::FullImage))
    }

    pub(crate) fn trigger_missing_migration_object_action(&mut self) {
        if self.work.migration.busy {
            return;
        }
        if self.work.migration.adding_missing_object {
            self.cancel_missing_migration_object();
        } else if self.migration_can_add_missing_object() {
            self.begin_missing_migration_object();
        }
    }

    pub(crate) fn migration_keypoint_undo_available(&self) -> bool {
        self.view == AppView::Annotate
            && self.work.migration.inspected_group_id.is_none()
            && self.work.migration.keypoint_index > 0
            && (matches!(
                self.work.migration.cursor,
                Some(MigrationCursor::Object { .. })
            ) || (self.work.migration.adding_missing_object
                && matches!(self.work.migration.cursor, Some(MigrationCursor::FullImage))))
    }

    fn migration_keypoint_undo_enabled(&self) -> bool {
        if self.work.migration.adding_missing_object {
            return !self.work.migration.busy
                && matches!(self.work.migration.cursor, Some(MigrationCursor::FullImage));
        }
        let Some(MigrationCursor::Object {
            object_group_id, ..
        }) = self.work.migration.cursor.as_ref()
        else {
            return false;
        };
        !self.work.migration.busy && self.migration_guide_valid(object_group_id)
    }

    fn migration_object_navigation_button(&mut self, ui: &mut egui::Ui) {
        if !self.can_edit_previous_migration_object() {
            return;
        }
        if ui
            .add(egui::Button::new("Previous object").shortcut_text(
                self.shortcut_text(ui.ctx(), labello_domain::UserAction::SelectPreviousObject),
            ))
            .clicked()
        {
            self.edit_previous_migration_object();
        }
    }

    fn migration_assignment_buttons(&mut self, ui: &mut egui::Ui) {
        let ready = self.work.assignment.is_some()
            && self.runtime.api.is_some()
            && !self.loading.saving
            && !self.loading.image
            && !self.work.migration.busy
            && self.work.pending_transition.is_none();
        if self.work.previous_annotation_assignment.is_some()
            && ui
                .add_enabled(
                    ready,
                    egui::Button::new("Previous assignment").shortcut_text(
                        self.shortcut_text(ui.ctx(), labello_domain::UserAction::PreviousImage),
                    ),
                )
                .clicked()
        {
            self.trigger_user_action(labello_domain::UserAction::PreviousImage);
        }
        if ui
            .add_enabled(
                ready,
                egui::Button::new("Skip").shortcut_text(
                    self.shortcut_text(ui.ctx(), labello_domain::UserAction::SkipAssignment),
                ),
            )
            .clicked()
        {
            self.trigger_user_action(labello_domain::UserAction::SkipAssignment);
        }
    }

    fn migration_assignment_section(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.label(RichText::new("Assignment").strong());
        self.migration_assignment_buttons(ui);
    }

    fn migration_more_actions(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("More", |ui| {
            if self.work.migration.inspected_group_id.is_some()
                && ui
                    .button(if self.inspection_next_returns_to_current() {
                        "Return to current object"
                    } else {
                        "Next object"
                    })
                    .clicked()
            {
                self.inspect_migration_object(1);
                ui.close();
            }
            let ready = self.work.assignment.is_some()
                && self.runtime.api.is_some()
                && !self.loading.saving
                && !self.loading.image
                && !self.work.migration.busy
                && self.work.pending_transition.is_none();
            if self.work.previous_annotation_assignment.is_some()
                && ui
                    .add_enabled(ready, egui::Button::new("Previous assignment"))
                    .clicked()
            {
                self.trigger_user_action(labello_domain::UserAction::PreviousImage);
                ui.close();
            }
            if ui.add_enabled(ready, egui::Button::new("Skip")).clicked() {
                self.trigger_user_action(labello_domain::UserAction::SkipAssignment);
                ui.close();
            }
        });
    }

    fn migration_primary_button(&mut self, ui: &mut egui::Ui, compact: bool) {
        let Some((action, enabled)) = self.migration_primary_action() else {
            return;
        };
        let label = action.label(compact);
        if theme::primary_button(
            ui,
            enabled,
            egui::Button::new(label)
                .shortcut_text(self.shortcut_text(ui.ctx(), labello_domain::UserAction::NextImage)),
        )
        .clicked()
        {
            self.perform_migration_primary_action(action);
        }
    }

    fn migration_primary_action(&self) -> Option<(MigrationPrimaryAction, bool)> {
        if self.view != AppView::Annotate || self.work.migration.inspected_group_id.is_some() {
            return None;
        }
        match self.work.migration.cursor.as_ref()? {
            MigrationCursor::Object {
                object_group_id, ..
            } => {
                let status = self.migration_disposition(object_group_id)?;
                let guide_valid = self.migration_guide_valid(object_group_id);
                let target_available = self.migration_expectation(object_group_id).is_some();
                let (action, action_ready) =
                    if !matches!(status, MigrationDispositionStatus::Pending)
                        && self.work.migration.active_pass_id.is_some()
                        && !self.work.migration.draft_dirty
                    {
                        (
                            MigrationPrimaryAction::KeepDisposition(object_group_id.clone()),
                            !self.migration_dependency_changed(object_group_id),
                        )
                    } else {
                        (
                            MigrationPrimaryAction::SaveSkeleton(object_group_id.clone()),
                            self.migration_draft_valid(),
                        )
                    };
                Some((
                    action,
                    !self.work.migration.busy && guide_valid && target_available && action_ready,
                ))
            }
            MigrationCursor::FullImage => {
                if self.work.migration.adding_missing_object {
                    return Some((
                        MigrationPrimaryAction::AddSkeleton,
                        !self.work.migration.busy && self.migration_draft_valid(),
                    ));
                }
                let (expected, ..) = self.migration_counts();
                Some((
                    MigrationPrimaryAction::Confirm {
                        no_guides: expected == 0,
                    },
                    !self.work.migration.busy,
                ))
            }
        }
    }

    fn perform_migration_primary_action(&mut self, action: MigrationPrimaryAction) {
        match action {
            MigrationPrimaryAction::SaveSkeleton(group_id) => {
                self.request_save_migration_skeleton(group_id)
            }
            MigrationPrimaryAction::AddSkeleton => self.request_add_migration_skeleton(),
            MigrationPrimaryAction::KeepDisposition(group_id) => {
                self.request_keep_migration_target(group_id)
            }
            MigrationPrimaryAction::Confirm { .. } => self.request_confirm_migration(),
        }
    }

    pub(crate) fn trigger_migration_primary_action(&mut self) {
        if let Some((action, true)) = self.migration_primary_action() {
            self.perform_migration_primary_action(action);
        }
    }

    pub(crate) fn trigger_migration_review_action(
        &mut self,
        decision: labello_domain::ReviewDecision,
    ) {
        if self.work.migration.busy {
            return;
        }
        let Some((task_id, target)) = self.current_migration_review_target() else {
            return;
        };
        self.request_review_migration(task_id, target, decision);
    }

    pub(crate) fn current_migration_review_target(
        &self,
    ) -> Option<(
        labello_domain::TaskId,
        labello_client::MigrationReviewTarget,
    )> {
        let state = self.work.current_state.as_ref()?;
        let task_id = self.work.selected_task_id.clone()?;
        let mut targets = state.migration_target_sets.get(&task_id)?.targets.clone();
        targets.sort_by_key(|target| target.sequence_index);
        if let Some(target) = targets.get(self.work.migration.review_index) {
            let disposition = state
                .migration_dispositions
                .get(&task_id)?
                .get(&target.object_group_id)?;
            if matches!(disposition.status, MigrationDispositionStatus::Pending) {
                return None;
            }
            return Some((
                task_id,
                labello_client::MigrationReviewTarget::Disposition {
                    object_group_id: target.object_group_id.clone(),
                    disposition_version: disposition.disposition_version,
                },
            ));
        }
        let confirmation = state.migration_confirmations.get(&task_id)?;
        Some((
            task_id,
            labello_client::MigrationReviewTarget::Confirmation {
                confirmation_hash: confirmation.confirmation_hash.clone(),
            },
        ))
    }

    fn migration_guide_valid(&self, group_id: &ObjectGroupId) -> bool {
        let Some(state) = self.work.current_state.as_ref() else {
            return false;
        };
        let Some(target) = self.work.selected_task_id.as_ref().and_then(|task_id| {
            state
                .migration_target_sets
                .get(task_id)?
                .targets
                .iter()
                .find(|target| &target.object_group_id == group_id)
        }) else {
            return false;
        };
        state
            .current_annotation(&target.guide_annotation_id)
            .is_some_and(|guide| !guide.deleted)
    }

    fn migration_dependency_changed(&self, group_id: &ObjectGroupId) -> bool {
        self.work
            .current_state
            .as_ref()
            .and_then(|state| {
                state
                    .migration_dependencies
                    .get(self.work.selected_task_id.as_ref()?)
                    .and_then(|markers| markers.get(group_id))
            })
            .is_some()
    }

    fn migration_status_list(&self, ui: &mut egui::Ui) {
        let Some((task_id, set, state)) = self.work.current_state.as_ref().and_then(|state| {
            let task_id = self.work.selected_task_id.as_ref()?;
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
        let Some((task_id, set, state)) = self.work.current_state.as_ref().and_then(|state| {
            let task_id = self.work.selected_task_id.as_ref()?;
            Some((task_id, state.migration_target_sets.get(task_id)?, state))
        }) else {
            return (0, 0, 0, 0);
        };
        let mut annotated = 0;
        let mut excluded = 0;
        for target in &set.targets {
            match state
                .migration_dispositions
                .get(task_id)
                .and_then(|dispositions| dispositions.get(&target.object_group_id))
                .map(|disposition| &disposition.status)
            {
                Some(MigrationDispositionStatus::Annotated { .. }) => annotated += 1,
                Some(MigrationDispositionStatus::Excluded { .. }) => excluded += 1,
                Some(MigrationDispositionStatus::Pending) | None => {}
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

    fn discovered_migration_skeleton_count(&self) -> usize {
        let Some(task_id) = self.work.selected_task_id.as_ref() else {
            return 0;
        };
        self.work
            .current_state
            .as_ref()
            .map(|state| {
                state
                    .active_annotations()
                    .filter(|annotation| {
                        annotation.task_id == *task_id
                            && annotation.object_group_id.is_none()
                            && annotation.annotation_type == AnnotationType::Skeleton
                    })
                    .count()
            })
            .unwrap_or(0)
    }

    pub(crate) fn canonical_migration_review_index(&self) -> usize {
        let Some((task_id, set, state)) = self.work.current_state.as_ref().and_then(|state| {
            let task_id = self.work.selected_task_id.as_ref()?;
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

    fn migration_targets(&self) -> Vec<labello_domain::MigrationTarget> {
        let mut targets = self
            .work
            .current_state
            .as_ref()
            .and_then(|state| {
                state
                    .migration_target_sets
                    .get(self.work.selected_task_id.as_ref()?)
            })
            .map(|set| set.targets.clone())
            .unwrap_or_default();
        targets.sort_by_key(|target| target.sequence_index);
        targets
    }

    fn migration_target(
        &self,
        group_id: &ObjectGroupId,
    ) -> Option<labello_domain::MigrationTarget> {
        self.migration_targets()
            .into_iter()
            .find(|target| &target.object_group_id == group_id)
    }

    fn migration_browse_boundary(&self, targets: &[labello_domain::MigrationTarget]) -> usize {
        match self.work.migration.cursor.as_ref() {
            Some(MigrationCursor::Object {
                object_group_id, ..
            }) => targets
                .iter()
                .position(|target| &target.object_group_id == object_group_id)
                .unwrap_or(0),
            Some(MigrationCursor::FullImage) => targets.len(),
            None => 0,
        }
    }

    pub(crate) fn can_edit_previous_migration_object(&self) -> bool {
        if self.view != AppView::Annotate
            || self.work.migration.busy
            || self.work.migration.adding_missing_object
        {
            return false;
        }
        let targets = self.migration_targets();
        let boundary = self.migration_browse_boundary(&targets);
        let position = self
            .work
            .migration
            .inspected_group_id
            .as_ref()
            .and_then(|group_id| {
                targets
                    .iter()
                    .position(|target| &target.object_group_id == group_id)
            })
            .unwrap_or(boundary);
        position > 0
    }

    pub(crate) fn edit_previous_migration_object(&mut self) {
        if !self.can_edit_previous_migration_object() {
            return;
        }
        let targets = self.migration_targets();
        let boundary = self.migration_browse_boundary(&targets);
        let position = self
            .work
            .migration
            .inspected_group_id
            .as_ref()
            .and_then(|group_id| {
                targets
                    .iter()
                    .position(|target| &target.object_group_id == group_id)
            })
            .unwrap_or(boundary);
        if let Some(previous) = position.checked_sub(1).and_then(|index| targets.get(index)) {
            self.begin_revisit_migration_target(previous.object_group_id.clone());
        }
    }

    fn inspection_next_returns_to_current(&self) -> bool {
        let targets = self.migration_targets();
        let boundary = self.migration_browse_boundary(&targets);
        self.work
            .migration
            .inspected_group_id
            .as_ref()
            .and_then(|group_id| {
                targets
                    .iter()
                    .position(|target| &target.object_group_id == group_id)
            })
            .is_none_or(|position| position + 1 >= boundary)
    }

    pub(crate) fn inspect_migration_object(&mut self, direction: isize) {
        if self.view != AppView::Annotate
            || self.work.migration.busy
            || self.work.migration.adding_missing_object
        {
            return;
        }
        let targets = self.migration_targets();
        let boundary = self.migration_browse_boundary(&targets);
        let position = self
            .work
            .migration
            .inspected_group_id
            .as_ref()
            .and_then(|group_id| {
                targets
                    .iter()
                    .position(|target| &target.object_group_id == group_id)
            })
            .unwrap_or(boundary);
        if direction < 0 {
            if position > 0 {
                self.work.migration.inspected_group_id =
                    Some(targets[position - 1].object_group_id.clone());
            }
        } else if self.work.migration.inspected_group_id.is_some() {
            if position + 1 < boundary {
                self.work.migration.inspected_group_id =
                    Some(targets[position + 1].object_group_id.clone());
            } else {
                self.work.migration.inspected_group_id = None;
            }
        }
    }

    pub(crate) fn migration_has_unsaved_input(&self) -> bool {
        self.work.migration.draft_dirty || self.work.migration.exclusion_dirty
    }

    fn begin_missing_migration_object(&mut self) {
        if self.work.migration.busy
            || !matches!(self.work.migration.cursor, Some(MigrationCursor::FullImage))
        {
            return;
        }
        let names = self
            .selected_task()
            .and_then(|task| task.skeleton.as_ref())
            .map(|spec| {
                spec.keypoints
                    .iter()
                    .map(|point| point.name.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        self.work.migration.adding_missing_object = true;
        self.work.migration.draft = Some(ManualMigrationState::empty_skeleton(names));
        self.work.migration.draft_group = None;
        self.work.migration.draft_dirty = false;
        self.work.migration.keypoint_index = 0;
        self.work.migration.next_hidden = false;
    }

    fn cancel_missing_migration_object(&mut self) {
        self.work.migration.adding_missing_object = false;
        self.work.migration.draft = None;
        self.work.migration.draft_dirty = false;
        self.work.migration.keypoint_index = 0;
        self.work.migration.next_hidden = false;
    }

    fn begin_revisit_migration_target(&mut self, group_id: ObjectGroupId) {
        if self.migration_has_unsaved_input() {
            self.work.migration.pending_revisit_target = Some(group_id);
        } else {
            self.request_revisit_migration_target(group_id);
        }
    }

    pub(crate) fn confirm_pending_migration_revisit(&mut self) {
        let Some(group_id) = self.work.migration.pending_revisit_target.take() else {
            return;
        };
        self.work.migration.draft = None;
        self.work.migration.draft_group = None;
        self.work.migration.draft_dirty = false;
        self.work.migration.exclusion_note.clear();
        self.work.migration.exclusion_dirty = false;
        self.request_revisit_migration_target(group_id);
    }

    pub(crate) fn cancel_pending_migration_revisit(&mut self) {
        self.work.migration.pending_revisit_target = None;
    }

    fn activate_migration_from_inactive_guide(&mut self, guide_id: &labello_domain::AnnotationId) {
        if self.view != AppView::Annotate || self.work.migration.busy {
            return;
        }
        let Some(task_id) = self.work.selected_task_id.as_ref() else {
            return;
        };
        let clicked = self.work.current_state.as_ref().and_then(|state| {
            let target = state
                .migration_target_sets
                .get(task_id)?
                .targets
                .iter()
                .find(|target| &target.guide_annotation_id == guide_id)?;
            let status = state
                .migration_dispositions
                .get(task_id)
                .and_then(|dispositions| dispositions.get(&target.object_group_id))
                .map(|disposition| disposition.status.clone());
            Some((target.object_group_id.clone(), status))
        });
        let Some((clicked_group_id, status)) = clicked else {
            return;
        };
        if matches!(status, Some(MigrationDispositionStatus::Excluded { .. })) {
            if self
                .migration_active_target()
                .is_none_or(|(active_group_id, _)| active_group_id != clicked_group_id)
            {
                self.begin_revisit_migration_target(clicked_group_id);
            }
            return;
        }
        let Some((active_group_id, _)) = self.migration_active_target() else {
            return;
        };
        if clicked_group_id == active_group_id || !migration_target_is_unmigrated(status.as_ref()) {
            return;
        }
        if let Some((action, true)) = self.migration_primary_action() {
            self.work.migration.pending_activate_target = Some(clicked_group_id);
            self.perform_migration_primary_action(action);
        } else if self.migration_draft_has_no_keypoint_input() {
            self.work.migration.pending_activate_target = Some(clicked_group_id);
            self.request_skip_migration_target(active_group_id);
        }
    }

    fn migration_active_target(&self) -> Option<(ObjectGroupId, labello_domain::MigrationTarget)> {
        if self.view == AppView::Review {
            let task_id = self.work.selected_task_id.as_ref()?;
            let target = self
                .work
                .current_state
                .as_ref()?
                .migration_target_sets
                .get(task_id)?
                .targets
                .get(self.work.migration.review_index)?
                .clone();
            return Some((target.object_group_id.clone(), target));
        }
        if let Some(group_id) = self.work.migration.inspected_group_id.as_ref() {
            let target = self.migration_target(group_id)?;
            return Some((group_id.clone(), target));
        }
        let MigrationCursor::Object {
            object_group_id, ..
        } = self.work.migration.cursor.as_ref()?
        else {
            return None;
        };
        let task_id = self.work.selected_task_id.as_ref()?;
        let target = self
            .work
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
        let task_id = self.work.selected_task_id.as_ref()?;
        self.work
            .current_state
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
        let existing = self.work.selected_task_id.as_ref().and_then(|task_id| {
            let state = self.work.current_state.as_ref()?;
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
        self.work.migration.draft_dirty = false;
    }

    fn place_migration_keypoint(&mut self, point: NormalizedPoint) {
        if !self.work.migration.adding_missing_object {
            let Some((group_id, _)) = self.migration_active_target() else {
                return;
            };
            self.ensure_migration_draft(&group_id);
        }
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
        self.work.migration.draft_dirty = true;
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
            self.work.migration.draft_dirty = true;
            self.work.migration.next_hidden = false;
        }
    }

    pub(crate) fn remove_last_migration_keypoint(&mut self) {
        if self.work.migration.busy || !self.migration_draft_editable() {
            return;
        }
        let Some(index) = self.work.migration.keypoint_index.checked_sub(1) else {
            return;
        };
        let Some(keypoint) = self
            .work
            .migration
            .draft
            .as_mut()
            .and_then(|draft| draft.keypoints.get_mut(index))
        else {
            return;
        };
        keypoint.point = None;
        keypoint.state = KeypointState::Absent;
        self.work.migration.keypoint_index = index;
        self.work.migration.draft_dirty = true;
        self.work.migration.next_hidden = false;
    }

    fn migration_draft_editable(&self) -> bool {
        if self.view != AppView::Annotate {
            return false;
        }
        if self.work.migration.adding_missing_object {
            return matches!(self.work.migration.cursor, Some(MigrationCursor::FullImage));
        }
        let Some((_, target)) = self.migration_active_target() else {
            return false;
        };
        let Some(state) = self.work.current_state.as_ref() else {
            return false;
        };
        state
            .current_annotation(&target.guide_annotation_id)
            .is_some_and(|guide| !guide.deleted)
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

    fn migration_draft_has_no_keypoint_input(&self) -> bool {
        self.work.migration.keypoint_index == 0
            && self.work.migration.draft.as_ref().is_none_or(|draft| {
                draft
                    .keypoints
                    .iter()
                    .all(|keypoint| keypoint.point.is_none())
            })
    }

    fn migration_expectation(
        &self,
        group_id: &ObjectGroupId,
    ) -> Option<labello_client::MigrationTargetExpectation> {
        let task_id = self.work.selected_task_id.as_ref()?;
        let state = self.work.current_state.as_ref()?;
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
            .work
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

    fn request_add_migration_skeleton(&mut self) {
        let Some((assignment, task_id, skeleton)) = self
            .work
            .assignment
            .as_ref()
            .zip(self.work.selected_task_id.clone())
            .zip(self.work.migration.draft.clone())
            .map(|((assignment, task_id), skeleton)| (assignment, task_id, skeleton))
        else {
            return;
        };
        self.queue_migration_action(MigrationAction::AddSkeleton(
            labello_client::AddMigrationSkeletonRequest {
                assignment_id: assignment.assignment_id.clone(),
                pass_id: self.work.migration.active_pass_id.clone(),
                task_id,
                skeleton,
            },
        ));
    }

    pub(crate) fn request_exclude_migration_target(&mut self, group_id: ObjectGroupId) {
        let note = (!self.work.migration.exclusion_note.trim().is_empty())
            .then(|| self.work.migration.exclusion_note.trim().to_string());
        self.request_migration_exclusion(group_id, self.work.migration.exclusion_reason, note);
    }

    fn request_skip_migration_target(&mut self, group_id: ObjectGroupId) {
        self.request_migration_exclusion(group_id, MigrationExclusionReason::NoValidSkeleton, None);
    }

    fn request_migration_exclusion(
        &mut self,
        group_id: ObjectGroupId,
        reason: MigrationExclusionReason,
        note: Option<String>,
    ) {
        let Some((assignment, target)) = self
            .work
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
                reason,
                note,
            },
        ));
    }

    fn request_reopen_migration_target(&mut self, group_id: ObjectGroupId) {
        let Some((assignment, target)) = self
            .work
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

    pub(crate) fn request_revisit_migration_target(&mut self, group_id: ObjectGroupId) {
        let Some((assignment, target)) = self
            .work
            .assignment
            .clone()
            .zip(self.migration_expectation(&group_id))
        else {
            return;
        };
        self.queue_migration_action(MigrationAction::Revisit(
            labello_client::RevisitMigrationTargetRequest {
                assignment_id: assignment.assignment_id,
                pass_id: self.work.migration.active_pass_id.clone(),
                target,
            },
        ));
    }

    fn request_keep_migration_target(&mut self, group_id: ObjectGroupId) {
        let Some((assignment, pass_id, target)) = self
            .work
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
            self.work.assignment.clone().and_then(|assignment| {
                let task_id = self.work.selected_task_id.clone()?;
                let state = self.work.current_state.as_ref()?;
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
            self.work.assignment.clone().and_then(|assignment| {
                let task_id = self.work.selected_task_id.clone()?;
                let state = self.work.current_state.as_ref()?;
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
        let Some(assignment) = self.work.assignment.clone() else {
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
            .work
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

fn migration_guide_style(
    current: bool,
    status: Option<&MigrationDispositionStatus>,
) -> CanvasAnnotationStyle {
    if matches!(status, Some(MigrationDispositionStatus::Excluded { .. })) {
        return if current {
            CanvasAnnotationStyle::solid(theme::DANGER)
        } else {
            CanvasAnnotationStyle::dashed(theme::DANGER)
        };
    }
    if current {
        return CanvasAnnotationStyle::solid(theme::ACCENT_HOVER);
    }
    CanvasAnnotationStyle::dashed(match status {
        Some(MigrationDispositionStatus::Annotated { .. }) => theme::SUCCESS,
        Some(MigrationDispositionStatus::Excluded { .. }) => theme::DANGER,
        Some(MigrationDispositionStatus::Pending) | None => theme::TEXT_MUTED,
    })
}

fn migration_target_is_unmigrated(status: Option<&MigrationDispositionStatus>) -> bool {
    matches!(status, Some(MigrationDispositionStatus::Pending) | None)
}

fn disposition_label(status: Option<&MigrationDispositionStatus>) -> &'static str {
    match status {
        Some(MigrationDispositionStatus::Pending) => "Pending",
        Some(MigrationDispositionStatus::Annotated { .. }) => "Skeleton annotated",
        Some(MigrationDispositionStatus::Excluded { .. }) => "Excluded",
        None => "Unavailable",
    }
}

fn disposition_intent(status: Option<&MigrationDispositionStatus>) -> theme::Intent {
    match status {
        Some(MigrationDispositionStatus::Annotated { .. }) => theme::Intent::Success,
        Some(MigrationDispositionStatus::Excluded { .. }) => theme::Intent::Error,
        Some(MigrationDispositionStatus::Pending) | None => theme::Intent::Neutral,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_guide_styles_keep_the_active_exclusion_red_for_review() {
        assert_eq!(
            migration_guide_style(true, Some(&MigrationDispositionStatus::Pending)),
            CanvasAnnotationStyle::solid(theme::ACCENT_HOVER)
        );
        assert_eq!(
            migration_guide_style(
                false,
                Some(&MigrationDispositionStatus::Annotated {
                    skeleton_annotation_id: labello_domain::AnnotationId::from("skeleton"),
                    skeleton_version: 1,
                }),
            ),
            CanvasAnnotationStyle::dashed(theme::SUCCESS)
        );
        assert_eq!(
            migration_guide_style(false, Some(&MigrationDispositionStatus::Pending)),
            CanvasAnnotationStyle::dashed(theme::TEXT_MUTED)
        );
        let excluded = MigrationDispositionStatus::Excluded {
            exclusion: labello_domain::MigrationExclusion {
                reason: MigrationExclusionReason::ObjectNotPresent,
                event_id: labello_domain::EventId::from("evt-excluded"),
                actor_user_id: labello_domain::UserId::from("annotator"),
                timestamp: labello_domain::now(),
                note: None,
            },
        };
        assert_eq!(
            migration_guide_style(true, Some(&excluded)),
            CanvasAnnotationStyle::solid(theme::DANGER),
            "the current Review target must keep its rejected/excluded color"
        );
        assert_eq!(
            migration_guide_style(false, Some(&excluded)),
            CanvasAnnotationStyle::dashed(theme::DANGER)
        );
    }
}
