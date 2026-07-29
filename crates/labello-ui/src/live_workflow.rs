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
    app::{
        AppView, IMAGE_QUEUE_SIZE, LabelloApp, LoadedImage, ReviewPhase, SaveStatus, UiCommand,
        UiMessage,
    },
    queue::QueuedImage,
};

impl LabelloApp {
    pub(crate) fn clear_current_image(&mut self) {
        self.release_prepared_assignments();
        if let Some(request_id) = self.work.active_load_id {
            self.runtime.active_requests.remove(&request_id);
        }
        if let Some(request_id) = self.work.active_operation_id {
            self.runtime.active_requests.remove(&request_id);
        }
        if let Some(request_id) = self.work.active_prefetch_id {
            self.runtime.active_requests.remove(&request_id);
        }
        self.work.assignment = None;
        self.work.current = None;
        self.work.current_state = None;
        self.work.current_texture = None;
        self.work.annotations.clear();
        self.work.persisted_annotations.clear();
        self.work.modified_annotations.clear();
        self.work.accepted_prelabels.clear();
        self.work.selected_prelabel = None;
        self.work.selected_annotation = None;
        self.work.active_skeleton = None;
        self.work.skeleton_keypoint_index = 0;
        self.work.next_keypoint_hidden = false;
        self.work.save_status = SaveStatus::Idle;
        self.work.edit_generation = 0;
        self.work.review_index = 0;
        self.work.review_rejected = false;
        self.work.correction_draft = None;
        self.work.migration = Default::default();
        self.work.canvas.fit_view();
        self.work.active_load_id = None;
        self.work.active_prefetch_id = None;
        self.work.active_operation_id = None;
        self.runtime.persistence.work_ready = None;
        self.reset_work_draft_tracking();
        self.loading.image = false;
        self.loading.saving = false;
        self.work.queue.set_loading(false);
        self.work.queue.clear();
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
                excluded_image_ids,
            } => self.spawn_message(request.clone(), async move {
                let assignment = match api
                    .assign_next_image(
                        &dataset_id,
                        AssignNextRequest {
                            task_id,
                            kind: Some(kind.clone()),
                            assignment_id: reclaim_assignment_id,
                            excluded_image_ids,
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
            UiCommand::PrefetchAssignment {
                request,
                operation_id,
                dataset_id,
                task_id,
                prelabel_config_ids,
                excluded_image_ids,
            } => self.spawn_message(request.clone(), async move {
                let assignment = match api
                    .assign_next_image(
                        &dataset_id,
                        AssignNextRequest {
                            task_id,
                            kind: Some(AssignmentKind::Annotation),
                            assignment_id: None,
                            excluded_image_ids,
                        },
                    )
                    .await
                {
                    Ok(assignment) => assignment,
                    Err(error) => {
                        return UiMessage::PrefetchLoaded {
                            request,
                            operation_id,
                            result: Box::new(Err(error.to_string())),
                        };
                    }
                };
                let Some(assignment) = assignment else {
                    return UiMessage::PrefetchLoaded {
                        request,
                        operation_id,
                        result: Box::new(Ok(None)),
                    };
                };
                let result = load_image(
                    api.clone(),
                    dataset_id.clone(),
                    assignment.clone(),
                    prelabel_config_ids,
                    true,
                )
                .await
                .map(Some)
                .map_err(|error| error.to_string());
                if result.is_err() {
                    let _ = api
                        .release_assignment(&dataset_id, assignment_action(&assignment))
                        .await;
                }
                UiMessage::PrefetchLoaded {
                    request,
                    operation_id,
                    result: Box::new(result),
                }
            }),
            UiCommand::ReleaseReservation {
                request,
                dataset_id,
                assignment,
            } => self.spawn_message(request.clone(), async move {
                let result = api
                    .release_assignment(&dataset_id, assignment_action(&assignment))
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string());
                UiMessage::ReservationReleased { request, result }
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
            UiCommand::ReopenAssignment {
                request,
                operation_id,
                dataset_id,
                assignment,
                prelabel_config_ids,
            } => self.spawn_message(request.clone(), async move {
                let assignment = if assignment.status == labello_domain::AssignmentStatus::Active {
                    if assignment
                        .expires_at
                        .is_some_and(|expires_at| expires_at <= labello_domain::now())
                    {
                        return UiMessage::PreviousAssignmentLoaded {
                            request,
                            operation_id,
                            assignment: Some(assignment),
                            result: Box::new(Err(
                                "the previous assignment lease expired".to_string()
                            )),
                        };
                    }
                    assignment
                } else {
                    match api
                        .reopen_assignment(&dataset_id, assignment_action(&assignment))
                        .await
                    {
                        Ok(assignment) => assignment,
                        Err(error) => {
                            return UiMessage::PreviousAssignmentLoaded {
                                request,
                                operation_id,
                                assignment: None,
                                result: Box::new(Err(error.to_string())),
                            };
                        }
                    }
                };
                let result = load_image(
                    api,
                    dataset_id,
                    assignment.clone(),
                    prelabel_config_ids,
                    true,
                )
                .await
                .map_err(|error| error.to_string());
                UiMessage::PreviousAssignmentLoaded {
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
        if self.loading.image || self.work.assignment.is_some() || self.runtime.api.is_none() {
            return;
        }
        let mut availability_matches = self.work.availability.dataset_id.as_ref()
            == Some(&self.config.dataset_id)
            && self.work.availability.kind.as_ref() == Some(&kind);
        if (!availability_matches || !self.work.availability.resolved)
            && self.restore_cached_assignment_availability()
        {
            availability_matches = true;
        }
        if !availability_matches || !self.work.availability.resolved {
            self.work.availability.load_after_resolution = true;
            if !self.work.availability.loading {
                self.request_assignment_availability();
            }
            return;
        }
        let task = if self.workflow_availability(&task.task_id) == Some(true) {
            task
        } else {
            let choices = self.workflow_choices();
            let current = choices
                .iter()
                .position(|choice| choice.task_id == task.task_id)
                .unwrap_or(0);
            let next = (1..=choices.len())
                .map(|offset| &choices[(current + offset) % choices.len()])
                .find(|choice| self.workflow_availability(&choice.task_id) == Some(true))
                .cloned();
            let Some(next) = next else {
                self.work.availability.load_after_resolution = true;
                self.runtime.persistence.expected_assignment = None;
                self.runtime.notice = Some(
                    match self.view {
                        AppView::Annotate => "No annotation work is currently available.",
                        AppView::Review => "No reviews are currently waiting.",
                        AppView::Adjudicate => "No adjudications are currently waiting.",
                        _ => "No work is currently available.",
                    }
                    .to_string(),
                );
                return;
            };
            self.select_workflow(&next.task_id);
            self.runtime.persistence.expected_assignment = None;
            self.selected_task()
                .cloned()
                .expect("selected available workflow must remain configured")
        };
        self.work.availability.load_after_resolution = false;
        let operation_id = self.begin_load();
        let request = self.operation_identity(operation_id, self.config.dataset_id.clone());
        let excluded_image_ids = self.assignment_exclusions();
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
            excluded_image_ids,
        });
    }

    pub(crate) fn request_prefetch(&mut self) {
        if self.view != AppView::Annotate
            || self.work.assignment.is_none()
            || self.work.queue.is_loading()
            || self.work.queue.len() >= self.work.queue.queue_size()
            || self.runtime.api.is_none()
        {
            return;
        }
        let Some(task) = self.selected_task().cloned() else {
            return;
        };
        let operation_id = self.next_operation();
        self.work.active_prefetch_id = Some(operation_id);
        self.work.queue.set_loading(true);
        let request = self.operation_identity(operation_id, self.config.dataset_id.clone());
        let excluded_image_ids = self.assignment_exclusions();
        self.queue_command(UiCommand::PrefetchAssignment {
            request,
            operation_id,
            dataset_id: self.config.dataset_id.clone(),
            task_id: task.task_id,
            prelabel_config_ids: task.prelabel_config_ids,
            excluded_image_ids,
        });
    }

    fn assignment_exclusions(&self) -> Vec<labello_domain::ImageId> {
        let mut excluded = self
            .work
            .assignment
            .as_ref()
            .map(|assignment| vec![assignment.image_id.clone()])
            .unwrap_or_default();
        for image_id in self.work.queue.prepared_image_ids() {
            if !excluded.contains(&image_id) {
                excluded.push(image_id);
            }
        }
        if let Some(image_id) = self.work.one_shot_excluded_image_id.clone()
            && !excluded.contains(&image_id)
        {
            excluded.push(image_id);
        }
        excluded.truncate(IMAGE_QUEUE_SIZE + 1);
        excluded
    }

    pub(crate) fn release_prepared_assignments(&mut self) {
        if self.runtime.api.is_none() {
            self.work.queue.clear();
            return;
        }
        for assignment in self.work.queue.drain_prepared_assignments() {
            self.release_reservation(self.config.dataset_id.clone(), assignment);
        }
    }

    pub(crate) fn release_reservation(
        &mut self,
        dataset_id: labello_domain::DatasetId,
        assignment: Assignment,
    ) {
        let Some(api) = self.runtime.api.clone() else {
            return;
        };
        let operation_id = self.next_operation();
        let request = self.operation_identity(operation_id, dataset_id.clone());
        self.runtime.active_requests.insert(request.request_id);
        self.start_workflow_command(
            api,
            UiCommand::ReleaseReservation {
                request,
                dataset_id,
                assignment,
            },
        );
    }

    pub(crate) fn clear_previous_annotation_assignment(&mut self) {
        let Some(previous) = self.work.previous_annotation_assignment.take() else {
            return;
        };
        if previous.status == labello_domain::AssignmentStatus::Active
            && self
                .work
                .assignment
                .as_ref()
                .is_none_or(|current| current.assignment_id != previous.assignment_id)
        {
            self.release_reservation(self.config.dataset_id.clone(), previous);
        }
    }

    pub(crate) fn remember_previous_annotation_assignment(&mut self, assignment: Assignment) {
        self.clear_previous_annotation_assignment();
        self.work.previous_annotation_assignment = Some(assignment);
    }

    pub(crate) fn retry_prefetch_if_due(&mut self, ctx: &egui::Context) {
        if self.work.queue.retry_due() {
            self.work.queue.clear_failure();
            self.request_prefetch();
        }
        if let Some(delay) = self.work.queue.retry_after() {
            ctx.request_repaint_after(delay);
        }
    }

    pub(crate) fn retry_assignment_load(&mut self) {
        if self.loading.image || self.runtime.api.is_none() {
            return;
        }
        let Some(assignment) = self.work.assignment.clone() else {
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
        let Some(assignment) = self.work.assignment.clone() else {
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
        let edit_generation = self.work.edit_generation;
        self.work.save_status = SaveStatus::Saving;
        self.queue_command(UiCommand::SaveAnnotations {
            request,
            operation_id,
            edit_generation,
            dataset_id: self.config.dataset_id.clone(),
            assignment,
            annotations: self.work.annotations.clone(),
            persisted: self.work.persisted_annotations.clone(),
            modified: self.work.modified_annotations.clone(),
            submit,
        });
    }

    pub(crate) fn request_release(&mut self) {
        let Some(assignment) = self.work.assignment.clone() else {
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

    pub(crate) fn request_reopen_assignment(&mut self, assignment: Assignment) {
        if self.loading.image || self.loading.saving || self.runtime.api.is_none() {
            return;
        }
        let prelabel_config_ids = self
            .work
            .tasks
            .iter()
            .find(|task| task.task_id == assignment.task_id)
            .map(|task| task.prelabel_config_ids.clone())
            .unwrap_or_default();
        let operation_id = self.begin_load();
        let request = self.operation_identity(operation_id, self.config.dataset_id.clone());
        self.queue_command(UiCommand::ReopenAssignment {
            request,
            operation_id,
            dataset_id: self.config.dataset_id.clone(),
            assignment,
            prelabel_config_ids,
        });
    }

    pub(crate) fn request_review(&mut self, decision: ReviewDecision) {
        if decision == ReviewDecision::Approved && self.work.correction_draft.is_some() {
            self.runtime.error =
                Some("Discard correction mode before approving this object.".to_string());
            return;
        }
        let (Some(assignment), Some(task)) =
            (self.work.assignment.clone(), self.selected_task().cloned())
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
        let (Some(assignment), Some(draft)) = (
            self.work.assignment.clone(),
            self.work.correction_draft.clone(),
        ) else {
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
            (self.work.assignment.clone(), self.selected_task().cloned())
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
            (self.work.assignment.clone(), self.selected_task().cloned())
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
                    .work
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
        self.work.active_load_id = Some(operation_id);
        self.loading.image = true;
        operation_id
    }

    fn begin_operation(&mut self) -> u64 {
        let operation_id = self.next_operation();
        self.work.active_operation_id = Some(operation_id);
        self.loading.saving = true;
        operation_id
    }

    pub(crate) fn next_operation(&mut self) -> u64 {
        self.work.next_operation_id = self.work.next_operation_id.wrapping_add(1);
        self.work.next_operation_id
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
    let (image, state, preview) = futures::try_join!(
        api.get_image_record(&dataset_id, &assignment.image_id),
        api.get_image_state(&dataset_id, &assignment.image_id),
        api.get_image_preview(&dataset_id, &assignment.image_id, 1600),
    )?;
    let color_image = Some(egui::ColorImage::from_rgba_unmultiplied(
        [preview.width as usize, preview.height as usize],
        &preview.rgba,
    ));
    let mut prelabels = Vec::new();
    if fetch_prelabels {
        let requests = prelabel_config_ids.into_iter().map(|config_id| {
            api.prelabel_suggestions(
                &dataset_id,
                PrelabelSuggestionRequest {
                    config_id,
                    task_id: assignment.task_id.clone(),
                },
            )
        });
        for mut suggestions in futures::future::try_join_all(requests).await? {
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
