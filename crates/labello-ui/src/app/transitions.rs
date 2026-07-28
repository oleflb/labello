impl LabelloApp {
    pub(crate) fn request_transition(&mut self, transition: PendingTransition) {
        if self.loading.saving || self.loading.image || self.transition_is_current(&transition) {
            return;
        }
        if self.work.assignment.is_some() {
            self.work.pending_transition = Some(transition);
            return;
        }
        self.execute_transition(transition);
    }

    pub(crate) fn execute_transition(&mut self, transition: PendingTransition) {
        match transition {
            PendingTransition::NextAssignment => {
                if self.runtime.api.is_some() {
                    self.clear_current_image();
                    self.request_next_image();
                } else {
                    self.advance_current_image();
                }
            }
            PendingTransition::PreviousAssignment(assignment) => {
                self.work.previous_annotation_assignment = Some(assignment.clone());
                self.clear_current_image();
                self.request_reopen_assignment(assignment);
            }
            PendingTransition::Workflow(task_id) => {
                if self.select_workflow(&task_id) {
                    self.clear_previous_annotation_assignment();
                    self.begin_workspace_epoch();
                    self.clear_current_image();
                    self.request_next_image();
                }
            }
            PendingTransition::View(view) => {
                self.work.show_tutorial = false;
                self.work.drawer = None;
                self.begin_workspace_epoch();
                self.clear_previous_annotation_assignment();
                if view == AppView::Admin {
                    self.clear_current_image();
                    self.request_admin_dataset();
                    return;
                }
                self.clear_current_image();
                self.view = view;
                if matches!(
                    view,
                    AppView::Annotate | AppView::Review | AppView::Adjudicate
                ) {
                    self.request_next_image();
                } else if view == AppView::Stats {
                    self.request_stats();
                }
            }
        }
    }

    fn transition_is_current(&self, transition: &PendingTransition) -> bool {
        match transition {
            PendingTransition::NextAssignment => false,
            PendingTransition::PreviousAssignment(_) => false,
            PendingTransition::Workflow(task_id) => {
                self.work.selected_task_id.as_ref() == Some(task_id)
            }
            PendingTransition::View(view) => self.view == *view,
        }
    }

    pub(crate) fn submit_pending_transition(&mut self) {
        if self.view != AppView::Annotate || self.work.pending_transition.is_none() {
            return;
        }
        if let Some(issue) = self.submission_issue() {
            self.runtime.error = Some(issue);
            return;
        }
        self.request_save(true);
    }

    pub(crate) fn release_pending_transition(&mut self) {
        if self.work.pending_transition.is_some() {
            self.request_release();
        }
    }

    pub(crate) fn cancel_pending_transition(&mut self) {
        if !self.loading.saving {
            self.work.pending_transition = None;
        }
    }

    pub(crate) fn submit_and_advance(&mut self) {
        if self.view != AppView::Annotate
            || self.loading.saving
            || (self.work.assignment.is_none() && self.runtime.api.is_some())
        {
            return;
        }
        if let Some(issue) = self.submission_issue() {
            self.runtime.error = Some(issue);
            return;
        }
        if self.runtime.api.is_none() {
            self.execute_transition(PendingTransition::NextAssignment);
            return;
        }
        self.work.pending_transition = Some(PendingTransition::NextAssignment);
        self.request_save(true);
    }

    pub(crate) fn skip_assignment(&mut self) {
        if self.loading.saving || (self.work.assignment.is_none() && self.runtime.api.is_some()) {
            return;
        }
        if self.view == AppView::Annotate
            && self.runtime.api.is_some()
            && (matches!(self.work.save_status, SaveStatus::Dirty | SaveStatus::Retry)
                || (self.manual_migration_active() && self.migration_has_unsaved_input()))
        {
            self.work.pending_transition = Some(PendingTransition::NextAssignment);
            return;
        }
        if self.runtime.api.is_none() {
            self.execute_transition(PendingTransition::NextAssignment);
            return;
        }
        self.work.pending_transition = Some(PendingTransition::NextAssignment);
        self.request_release();
    }

    pub(crate) fn return_to_previous_assignment(&mut self) {
        if self.view != AppView::Annotate
            || self.loading.saving
            || self.loading.image
            || self.work.pending_transition.is_some()
            || self.runtime.api.is_none()
        {
            return;
        }
        let Some(previous) = self.work.previous_annotation_assignment.clone() else {
            return;
        };
        if previous.status == labello_domain::AssignmentStatus::Active
            && previous
                .expires_at
                .is_some_and(|expires_at| expires_at <= labello_domain::now())
        {
            self.clear_previous_annotation_assignment();
            self.runtime.error = Some(
                "The previous assignment lease expired and can no longer be returned to."
                    .to_string(),
            );
            return;
        }
        if self.work.assignment.is_some()
            && (matches!(self.work.save_status, SaveStatus::Dirty | SaveStatus::Retry)
                || (self.manual_migration_active() && self.migration_has_unsaved_input()))
        {
            self.work.pending_transition = Some(PendingTransition::PreviousAssignment(previous));
            return;
        }
        self.request_reopen_assignment(previous);
    }

    fn submission_issue(&self) -> Option<String> {
        let task = self.selected_task()?;
        let spec = task.skeleton.as_ref()?;
        if self.work.active_skeleton.is_some() {
            return Some(
                "Finish the active skeleton or mark its remaining optional keypoints absent before submitting."
                    .to_string(),
            );
        }
        for annotation in self.work.annotations.iter().filter(|annotation| {
            !annotation.deleted && self.annotation_matches_selected_workflow(annotation)
        }) {
            let AnnotationGeometry::Skeleton(skeleton) = &annotation.geometry else {
                continue;
            };
            if let Some(required) = spec.keypoints.iter().find(|required| {
                required.required
                    && skeleton.keypoints.iter().any(|keypoint| {
                        keypoint.name == required.name && keypoint.state == KeypointState::Absent
                    })
            }) {
                return Some(format!(
                    "Required keypoint '{}' is absent. Place it before submitting.",
                    required.name
                ));
            }
        }
        None
    }

    pub(crate) fn advance_current_image(&mut self) {
        self.work.assignment = None;
        self.work.current_texture = None;
        self.work.current_state = None;
        self.work.current = self.work.queue.pop_next();
        self.work.annotations.clear();
        self.work.persisted_annotations.clear();
        self.work.modified_annotations.clear();
        self.work.accepted_prelabels.clear();
        self.work.selected_prelabel = None;
        self.work.selected_annotation = None;
        if self.runtime.api.is_some() {
            self.request_next_image();
        } else {
            self.replenish_demo_queue();
        }
    }

    pub(crate) fn autosave(&mut self) {
        if self.view == AppView::Annotate
            && matches!(self.work.save_status, SaveStatus::Dirty | SaveStatus::Retry)
        {
            if self.runtime.api.is_some() {
                self.request_save(false);
                return;
            }
            self.work.save_status = if self.work.offline {
                SaveStatus::Saving
            } else {
                SaveStatus::Saved
            };
        }
    }

    pub(crate) fn can_correct_review_object(&self) -> bool {
        self.view == AppView::Review
            && self
                .work
                .assignment
                .as_ref()
                .is_some_and(|assignment| assignment.kind == AssignmentKind::Review)
            && self.selected_task().is_some_and(|task| {
                task.review.workflow == labello_domain::ReviewWorkflow::Approval
                    && task.review.allow_reviewer_corrections
            })
            && self.current_review_annotation().is_some_and(|annotation| {
                self.work.selected_annotation.as_ref() == Some(&annotation.annotation_id)
            })
    }

    pub(crate) fn current_review_annotation(&self) -> Option<&labello_domain::AnnotationVersion> {
        (self.view == AppView::Review).then_some(()).and_then(|()| {
            self.work
                .annotations
                .iter()
                .filter(|annotation| {
                    !annotation.deleted && self.annotation_matches_selected_workflow(annotation)
                })
                .nth(self.work.review_index)
        })
    }

    pub(crate) fn start_correction(&mut self) {
        if self.work.correction_draft.is_some() || !self.can_correct_review_object() {
            return;
        }
        let Some(annotation) = self.current_review_annotation().cloned() else {
            return;
        };
        let annotation_id = annotation.annotation_id.clone();
        self.work.correction_draft = Some(CorrectionDraft {
            correction_id: labello_domain::CorrectionId::generate(),
            annotation_id,
            expected_version: annotation.version,
            original_geometry: annotation.geometry.clone(),
            edited_geometry: annotation.geometry,
            reason: String::new(),
            geometry_history: Vec::new(),
            selected_keypoint: None,
        });
        self.runtime.error = None;
    }

    pub(crate) fn discard_correction(&mut self) {
        self.work.correction_draft = None;
    }

    pub(crate) fn undo_correction(&mut self) {
        let Some(draft) = self.work.correction_draft.as_mut() else {
            return;
        };
        if let Some(geometry) = draft.geometry_history.pop() {
            draft.edited_geometry = geometry;
        }
    }

    fn update_correction_geometry(&mut self, geometry: AnnotationGeometry) {
        let Some(draft) = self.work.correction_draft.as_mut() else {
            return;
        };
        if draft.edited_geometry == geometry {
            return;
        }
        draft.geometry_history.push(draft.edited_geometry.clone());
        draft.edited_geometry = geometry;
        self.runtime.error = None;
    }

    pub(crate) fn edit_correction_bbox(&mut self, edit: crate::canvas::BoundingBoxEdit) {
        let Some(draft) = self.work.correction_draft.as_ref() else {
            return;
        };
        if draft.annotation_id != edit.annotation_id
            || !matches!(&draft.edited_geometry, AnnotationGeometry::BoundingBox(_))
        {
            return;
        }
        self.update_correction_geometry(AnnotationGeometry::BoundingBox(edit.bounding_box));
    }

    pub(crate) fn select_correction_keypoint(&mut self, index: usize) {
        let Some(draft) = self.work.correction_draft.as_mut() else {
            return;
        };
        let AnnotationGeometry::Skeleton(skeleton) = &draft.edited_geometry else {
            return;
        };
        if index < skeleton.keypoints.len() {
            draft.selected_keypoint = Some(index);
        }
    }

    pub(crate) fn edit_correction_keypoint(&mut self, edit: crate::canvas::KeypointEdit) {
        let Some(draft) = self.work.correction_draft.as_ref() else {
            return;
        };
        if draft.annotation_id != edit.annotation_id {
            return;
        }
        let mut geometry = draft.edited_geometry.clone();
        let AnnotationGeometry::Skeleton(skeleton) = &mut geometry else {
            return;
        };
        let Some(keypoint) = skeleton.keypoints.get_mut(edit.keypoint_index) else {
            return;
        };
        keypoint.point = Some(edit.point);
        self.update_correction_geometry(geometry);
        self.select_correction_keypoint(edit.keypoint_index);
    }

    pub(crate) fn set_correction_keypoint_state(&mut self, state: KeypointState) {
        let Some(draft) = self.work.correction_draft.as_ref() else {
            return;
        };
        let Some(index) = draft.selected_keypoint else {
            return;
        };
        let mut geometry = draft.edited_geometry.clone();
        let AnnotationGeometry::Skeleton(skeleton) = &mut geometry else {
            return;
        };
        let Some(keypoint) = skeleton.keypoints.get_mut(index) else {
            return;
        };
        if keypoint.state == state {
            return;
        }
        if state == KeypointState::Absent {
            keypoint.point = None;
        } else if keypoint.point.is_none() {
            return;
        }
        keypoint.state = state;
        self.update_correction_geometry(geometry);
    }
}
