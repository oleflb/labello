use std::{collections::BTreeSet, rc::Rc};

use eframe::egui;
use labello_client::{
    AnnotationBatchRequest, AssignNextRequest, AssignmentActionRequest, CorrectionRequest,
    LabelloApi, PrelabelSuggestionRequest,
};
use labello_domain::{
    AdjudicationDecision, AdjudicationId, AdjudicationRecord, AnnotationId, Assignment,
    AssignmentKind, EventPayload, PrelabelConfigId, ReviewDecision, ReviewId, ReviewRecord,
    ReviewTarget,
};

use crate::{
    app::{AppView, LabelloApp, LoadedImage, ReviewPhase, SaveStatus, UiCommand, UiMessage},
    queue::QueuedImage,
};

impl LabelloApp {
    pub(crate) fn clear_current_image(&mut self) {
        if let Some(request_id) = self.active_load_id {
            self.runtime.active_requests.remove(&request_id);
        }
        if let Some(request_id) = self.active_operation_id {
            self.runtime.active_requests.remove(&request_id);
        }
        self.assignment = None;
        self.current = None;
        self.current_state = None;
        self.current_texture = None;
        self.annotations.clear();
        self.persisted_annotations.clear();
        self.modified_annotations.clear();
        self.accepted_prelabels.clear();
        self.selected_annotation = None;
        self.active_skeleton = None;
        self.skeleton_keypoint_index = 0;
        self.next_keypoint_hidden = false;
        self.save_status = SaveStatus::Idle;
        self.edit_generation = 0;
        self.review_index = 0;
        self.review_rejected = false;
        self.correction_draft = None;
        self.canvas.fit_view();
        self.active_load_id = None;
        self.active_operation_id = None;
        self.runtime.persistence.work_ready = None;
        self.reset_work_draft_tracking();
        self.loading.image = false;
        self.loading.saving = false;
        self.queue.set_loading(false);
    }

    pub(crate) fn start_workflow_command(&self, api: Rc<dyn LabelloApi>, command: UiCommand) {
        match command {
            UiCommand::ClaimAssignment {
                request,
                operation_id,
                dataset_id,
                task_id,
                prelabel_config_ids,
                kind,
                reclaim_assignment_id,
            } => self.spawn_message(request.clone(), async move {
                let assignment = match api
                    .assign_next_image(
                        &dataset_id,
                        AssignNextRequest {
                            task_id,
                            kind: Some(kind.clone()),
                            assignment_id: reclaim_assignment_id,
                        },
                    )
                    .await
                {
                    Ok(assignment) => assignment,
                    Err(error) => {
                        return UiMessage::ImageLoaded {
                            request,
                            operation_id,
                            assignment: None,
                            result: Box::new(Err(error.to_string())),
                        };
                    }
                };
                let Some(assignment) = assignment else {
                    return UiMessage::ImageLoaded {
                        request,
                        operation_id,
                        assignment: None,
                        result: Box::new(Ok(None)),
                    };
                };
                let result = load_image(
                    api,
                    dataset_id,
                    assignment.clone(),
                    prelabel_config_ids,
                    kind == AssignmentKind::Annotation,
                )
                .await
                .map(Some)
                .map_err(|error| error.to_string());
                UiMessage::ImageLoaded {
                    request,
                    operation_id,
                    assignment: Some(assignment),
                    result: Box::new(result),
                }
            }),
            UiCommand::ReloadAssignment {
                request,
                operation_id,
                dataset_id,
                assignment,
                prelabel_config_ids,
                fetch_prelabels,
            } => self.spawn_message(request.clone(), async move {
                let result = load_image(
                    api,
                    dataset_id,
                    assignment.clone(),
                    prelabel_config_ids,
                    fetch_prelabels,
                )
                .await
                .map(Some)
                .map_err(|error| error.to_string());
                UiMessage::ImageLoaded {
                    request,
                    operation_id,
                    assignment: Some(assignment),
                    result: Box::new(result),
                }
            }),
            UiCommand::SaveAnnotations {
                request,
                operation_id,
                edit_generation,
                dataset_id,
                assignment,
                annotations,
                persisted,
                modified,
                submit,
            } => self.spawn_message(request.clone(), async move {
                let assignment_id = assignment.assignment_id.clone();
                let result = save_annotations(SaveAnnotationsJob {
                    api,
                    dataset_id,
                    assignment,
                    annotations,
                    persisted,
                    modified,
                    submit,
                })
                .await
                .map_err(|error| error.to_string());
                UiMessage::SaveFinished {
                    request,
                    operation_id,
                    assignment_id,
                    edit_generation,
                    completed: submit,
                    result: Box::new(result),
                }
            }),
            UiCommand::ReleaseAssignment {
                request,
                operation_id,
                dataset_id,
                assignment,
            } => self.spawn_message(request.clone(), async move {
                let assignment_id = assignment.assignment_id.clone();
                let result = api
                    .release_assignment(&dataset_id, assignment_action(&assignment))
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string());
                UiMessage::ReleaseFinished {
                    request,
                    operation_id,
                    assignment_id,
                    result,
                }
            }),
            UiCommand::Review {
                request,
                operation_id,
                dataset_id,
                assignment,
                review,
                phase,
            } => self.spawn_message(request.clone(), async move {
                let assignment_id = assignment.assignment_id.clone();
                let decision = review.decision.clone();
                let result = api
                    .record_assigned_review(&dataset_id, assignment_action(&assignment), review)
                    .await
                    .map_err(|error| error.to_string());
                UiMessage::ReviewFinished {
                    request,
                    operation_id,
                    assignment_id,
                    phase,
                    decision,
                    result: Box::new(result),
                }
            }),
            UiCommand::Correction {
                request,
                operation_id,
                dataset_id,
                assignment,
                correction,
            } => self.spawn_message(request.clone(), async move {
                let assignment_id = assignment.assignment_id.clone();
                let result = api
                    .record_assigned_correction(
                        &dataset_id,
                        assignment_action(&assignment),
                        correction,
                    )
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string());
                UiMessage::CorrectionFinished {
                    request,
                    operation_id,
                    assignment_id,
                    result,
                }
            }),
            UiCommand::Adjudication {
                request,
                operation_id,
                dataset_id,
                assignment,
                adjudication,
            } => self.spawn_message(request.clone(), async move {
                let assignment_id = assignment.assignment_id.clone();
                let result = api
                    .record_assigned_adjudication(
                        &dataset_id,
                        assignment_action(&assignment),
                        adjudication,
                    )
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string());
                UiMessage::AdjudicationFinished {
                    request,
                    operation_id,
                    assignment_id,
                    result,
                }
            }),
            _ => {}
        }
    }

    pub(crate) fn request_next_image(&mut self) {
        let Some(kind) = self.assignment_kind() else {
            return;
        };
        if !self.ensure_valid_task_selection() {
            self.runtime.error = Some(
                "No enabled one-class workflow is configured. Ask a data admin to enable one."
                    .to_string(),
            );
            return;
        }
        let Some(task) = self.selected_task().cloned() else {
            return;
        };
        if self.loading.image || self.assignment.is_some() || self.runtime.api.is_none() {
            return;
        }
        let operation_id = self.begin_load();
        let request = self.operation_identity(operation_id, self.config.dataset_id.clone());
        self.queue_command(UiCommand::ClaimAssignment {
            request,
            operation_id,
            dataset_id: self.config.dataset_id.clone(),
            task_id: task.task_id,
            prelabel_config_ids: if kind == AssignmentKind::Annotation {
                task.prelabel_config_ids
            } else {
                Vec::new()
            },
            kind,
            reclaim_assignment_id: self.runtime.persistence.expected_assignment.clone(),
        });
    }

    pub(crate) fn retry_assignment_load(&mut self) {
        if self.loading.image || self.runtime.api.is_none() {
            return;
        }
        let Some(assignment) = self.assignment.clone() else {
            self.request_next_image();
            return;
        };
        let prelabel_config_ids = if self.view == AppView::Annotate {
            self.selected_task()
                .map(|task| task.prelabel_config_ids.clone())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let operation_id = self.begin_load();
        let request = self.operation_identity(operation_id, self.config.dataset_id.clone());
        self.queue_command(UiCommand::ReloadAssignment {
            request,
            operation_id,
            dataset_id: self.config.dataset_id.clone(),
            assignment,
            prelabel_config_ids,
            fetch_prelabels: self.view == AppView::Annotate,
        });
    }

    pub(crate) fn request_save(&mut self, submit: bool) {
        let Some(assignment) = self.assignment.clone() else {
            return;
        };
        if assignment.kind != AssignmentKind::Annotation
            || self.loading.saving
            || self.runtime.api.is_none()
        {
            return;
        }
        let operation_id = self.begin_operation();
        let request = self.operation_identity(operation_id, self.config.dataset_id.clone());
        let edit_generation = self.edit_generation;
        self.save_status = SaveStatus::Saving;
        self.queue_command(UiCommand::SaveAnnotations {
            request,
            operation_id,
            edit_generation,
            dataset_id: self.config.dataset_id.clone(),
            assignment,
            annotations: self.annotations.clone(),
            persisted: self.persisted_annotations.clone(),
            modified: self.modified_annotations.clone(),
            submit,
        });
    }

    pub(crate) fn request_release(&mut self) {
        let Some(assignment) = self.assignment.clone() else {
            return;
        };
        if self.loading.saving || self.runtime.api.is_none() {
            return;
        }
        let operation_id = self.begin_operation();
        let request = self.operation_identity(operation_id, self.config.dataset_id.clone());
        self.queue_command(UiCommand::ReleaseAssignment {
            request,
            operation_id,
            dataset_id: self.config.dataset_id.clone(),
            assignment,
        });
    }

    pub(crate) fn request_review(&mut self, decision: ReviewDecision) {
        if decision == ReviewDecision::Approved && self.correction_draft.is_some() {
            self.runtime.error =
                Some("Discard correction mode before approving this object.".to_string());
            return;
        }
        let (Some(assignment), Some(task)) =
            (self.assignment.clone(), self.selected_task().cloned())
        else {
            return;
        };
        if assignment.kind != AssignmentKind::Review
            || self.loading.saving
            || self.runtime.api.is_none()
        {
            return;
        }
        self.sync_review_selection();
        let (target, phase) = if let Some(annotation) = self.current_review_annotation() {
            (
                ReviewTarget::AnnotationVersion {
                    annotation_id: annotation.annotation_id.clone(),
                    version: annotation.version,
                },
                ReviewPhase::Object,
            )
        } else {
            (
                ReviewTarget::Task {
                    task_id: task.task_id,
                },
                ReviewPhase::FullImage,
            )
        };
        let discard_correction = decision == ReviewDecision::Rejected;
        if self.queue_review(assignment, target, decision, phase) && discard_correction {
            self.discard_correction();
        }
    }

    pub(crate) fn request_correction(&mut self) {
        let (Some(assignment), Some(draft)) =
            (self.assignment.clone(), self.correction_draft.clone())
        else {
            return;
        };
        if assignment.kind != AssignmentKind::Review
            || self.loading.saving
            || self.runtime.api.is_none()
            || !self.can_correct_review_object()
            || !draft.geometry_changed()
        {
            return;
        }
        let operation_id = self.begin_operation();
        let request = self.operation_identity(operation_id, self.config.dataset_id.clone());
        self.queue_command(UiCommand::Correction {
            request,
            operation_id,
            dataset_id: self.config.dataset_id.clone(),
            assignment,
            correction: CorrectionRequest {
                correction_id: draft.correction_id,
                annotation_id: draft.annotation_id,
                expected_version: draft.expected_version,
                geometry: draft.edited_geometry,
                reason: (!draft.reason.trim().is_empty()).then(|| draft.reason.trim().to_string()),
            },
        });
    }

    pub(crate) fn request_full_image_review(&mut self, decision: ReviewDecision) {
        let (Some(assignment), Some(task)) =
            (self.assignment.clone(), self.selected_task().cloned())
        else {
            return;
        };
        self.queue_review(
            assignment,
            ReviewTarget::Task {
                task_id: task.task_id,
            },
            decision,
            ReviewPhase::FullImage,
        );
    }

    fn queue_review(
        &mut self,
        assignment: Assignment,
        target: ReviewTarget,
        decision: ReviewDecision,
        phase: ReviewPhase,
    ) -> bool {
        if self.loading.saving || self.runtime.api.is_none() {
            return false;
        }
        let operation_id = self.begin_operation();
        let request = self.operation_identity(operation_id, self.config.dataset_id.clone());
        self.queue_command(UiCommand::Review {
            request,
            operation_id,
            dataset_id: self.config.dataset_id.clone(),
            assignment,
            review: ReviewRecord {
                review_id: ReviewId::generate(),
                target,
                reviewer_user_id: self.config.user_id.clone(),
                decision,
                timestamp: labello_domain::now(),
                comment: None,
            },
            phase,
        })
    }

    pub(crate) fn request_adjudication(&mut self, decision: AdjudicationDecision) {
        let (Some(assignment), Some(task)) =
            (self.assignment.clone(), self.selected_task().cloned())
        else {
            return;
        };
        if assignment.kind != AssignmentKind::Adjudication
            || self.loading.saving
            || self.runtime.api.is_none()
        {
            return;
        }
        let operation_id = self.begin_operation();
        let request = self.operation_identity(operation_id, self.config.dataset_id.clone());
        self.queue_command(UiCommand::Adjudication {
            request,
            operation_id,
            dataset_id: self.config.dataset_id.clone(),
            assignment,
            adjudication: AdjudicationRecord {
                adjudication_id: AdjudicationId::generate(),
                task_id: task.task_id,
                annotation_ids: self
                    .annotations
                    .iter()
                    .filter(|annotation| {
                        !annotation.deleted && self.annotation_matches_selected_workflow(annotation)
                    })
                    .map(|annotation| annotation.annotation_id.clone())
                    .collect(),
                adjudicator_user_id: self.config.user_id.clone(),
                decision,
                resolution: "Resolved in Labello UI".to_string(),
                timestamp: labello_domain::now(),
            },
        });
    }

    fn begin_load(&mut self) -> u64 {
        let operation_id = self.next_operation();
        self.active_load_id = Some(operation_id);
        self.loading.image = true;
        self.queue.set_loading(true);
        operation_id
    }

    fn begin_operation(&mut self) -> u64 {
        let operation_id = self.next_operation();
        self.active_operation_id = Some(operation_id);
        self.loading.saving = true;
        operation_id
    }

    pub(crate) fn next_operation(&mut self) -> u64 {
        self.next_operation_id = self.next_operation_id.wrapping_add(1);
        self.next_operation_id
    }
}

struct SaveAnnotationsJob {
    api: Rc<dyn LabelloApi>,
    dataset_id: labello_domain::DatasetId,
    assignment: Assignment,
    annotations: Vec<labello_domain::AnnotationVersion>,
    persisted: BTreeSet<AnnotationId>,
    modified: BTreeSet<AnnotationId>,
    submit: bool,
}

async fn save_annotations(
    job: SaveAnnotationsJob,
) -> labello_client::ClientResult<labello_domain::ImageState> {
    let action = assignment_action(&job.assignment);
    let mut payloads = Vec::new();
    for annotation in job.annotations {
        let payload = if job.persisted.contains(&annotation.annotation_id) && annotation.deleted {
            EventPayload::AnnotationDeleted {
                annotation_id: annotation.annotation_id,
                version: annotation.version,
                reason: None,
            }
        } else if !job.persisted.contains(&annotation.annotation_id) && !annotation.deleted {
            EventPayload::AnnotationVersionCreated {
                annotation,
                previous_version: None,
                reason: None,
            }
        } else if job.modified.contains(&annotation.annotation_id) && !annotation.deleted {
            EventPayload::AnnotationVersionCreated {
                previous_version: annotation.version.checked_sub(1),
                annotation,
                reason: Some("annotator_edit".to_string()),
            }
        } else {
            continue;
        };
        payloads.push(payload);
    }
    job.api
        .apply_annotation_batch(
            &job.dataset_id,
            action,
            AnnotationBatchRequest {
                payloads,
                complete: job.submit,
            },
        )
        .await
}

async fn load_image(
    api: Rc<dyn LabelloApi>,
    dataset_id: labello_domain::DatasetId,
    assignment: Assignment,
    prelabel_config_ids: Vec<PrelabelConfigId>,
    fetch_prelabels: bool,
) -> labello_client::ClientResult<LoadedImage> {
    let image = api
        .get_image_record(&dataset_id, &assignment.image_id)
        .await?;
    let state = api
        .get_image_state(&dataset_id, &assignment.image_id)
        .await?;
    let preview = api
        .get_image_preview(&dataset_id, &assignment.image_id, 1600)
        .await?;
    let color_image = Some(egui::ColorImage::from_rgba_unmultiplied(
        [preview.width as usize, preview.height as usize],
        &preview.rgba,
    ));
    let mut prelabels = Vec::new();
    if fetch_prelabels {
        for config_id in prelabel_config_ids {
            let mut suggestions = api
                .prelabel_suggestions(
                    &dataset_id,
                    PrelabelSuggestionRequest {
                        config_id,
                        task_id: assignment.task_id.clone(),
                    },
                )
                .await?;
            prelabels.append(&mut suggestions);
        }
    }
    let annotations = state.active_annotations().cloned().collect();
    Ok(LoadedImage {
        assignment,
        queued: QueuedImage { image, prelabels },
        annotations,
        state,
        color_image,
    })
}

fn assignment_action(assignment: &Assignment) -> AssignmentActionRequest {
    AssignmentActionRequest {
        assignment_id: assignment.assignment_id.clone(),
        image_id: assignment.image_id.clone(),
        task_id: assignment.task_id.clone(),
        kind: assignment.kind.clone(),
    }
}
