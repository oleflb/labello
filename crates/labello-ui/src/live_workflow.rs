use std::{collections::BTreeSet, rc::Rc};

use eframe::egui;
use labello_client::{
    AppendEventRequest, AssignNextRequest, LabelloApi, PrelabelSuggestionRequest,
};
use labello_domain::{
    AdjudicationDecision, AdjudicationId, AdjudicationRecord, AnnotationId, AssignmentKind,
    EventPayload, ImageId, PrelabelConfigId, ReviewDecision, ReviewId, ReviewRecord, ReviewTarget,
    TaskState, TaskStatus,
};

use crate::{
    app::{LabelloApp, LoadedImage, QueueMode, SaveStatus, UiCommand, UiMessage},
    queue::QueuedImage,
};

impl QueueMode {
    pub(crate) fn assignment_kind(self) -> AssignmentKind {
        match self {
            Self::Annotate => AssignmentKind::Annotation,
            Self::Review => AssignmentKind::Review,
            Self::Adjudicate => AssignmentKind::Adjudication,
        }
    }
}

impl LabelloApp {
    pub(crate) fn set_queue_mode(&mut self, mode: QueueMode) {
        if self.queue_mode == mode {
            return;
        }
        if self.queue_mode == QueueMode::Annotate {
            self.autosave();
        }
        self.queue_mode = mode;
        self.clear_current_image();
        self.request_next_image();
    }

    pub(crate) fn clear_current_image(&mut self) {
        self.current = None;
        self.current_state = None;
        self.current_texture = None;
        self.annotations.clear();
        self.persisted_annotations.clear();
        self.accepted_prelabels.clear();
        self.selected_annotation = None;
        self.save_status = SaveStatus::Idle;
    }

    pub(crate) fn start_workflow_command(&self, api: Rc<dyn LabelloApi>, command: UiCommand) {
        match command {
            UiCommand::NextImage {
                dataset_id,
                task_id,
                prelabel_config_ids,
                kind,
            } => self.spawn_message(async move {
                let result =
                    load_next_image(api, dataset_id, task_id, prelabel_config_ids, kind).await;
                UiMessage::ImageLoaded(result.map_err(|error| error.to_string()))
            }),
            UiCommand::SaveAnnotations {
                dataset_id,
                image_id,
                user_id,
                task_id,
                annotations,
                persisted,
                submit,
            } => self.spawn_message(async move {
                let result = save_annotations(SaveAnnotationsJob {
                    api,
                    dataset_id,
                    image_id,
                    user_id,
                    task_id,
                    annotations,
                    persisted,
                    submit,
                })
                .await
                .map_err(|error| error.to_string());
                UiMessage::SaveFinished(result)
            }),
            UiCommand::Review {
                dataset_id,
                image_id,
                review,
            } => self.spawn_message(async move {
                UiMessage::ReviewFinished(
                    api.record_review(&dataset_id, &image_id, review)
                        .await
                        .map(|_| ())
                        .map_err(|error| error.to_string()),
                )
            }),
            UiCommand::Adjudication {
                dataset_id,
                image_id,
                adjudication,
            } => self.spawn_message(async move {
                UiMessage::AdjudicationFinished(
                    api.record_adjudication(&dataset_id, &image_id, adjudication)
                        .await
                        .map(|_| ())
                        .map_err(|error| error.to_string()),
                )
            }),
            _ => {}
        }
    }

    pub(crate) fn request_next_image(&mut self) {
        if !self.ensure_valid_task_selection() {
            self.queue.set_loading(false);
            self.loading.image = false;
            self.runtime.error = Some(
                "No enabled workflow is configured. Open Admin and create a single-class workflow."
                    .to_string(),
            );
            return;
        }
        let Some(task) = self.selected_task().cloned() else {
            return;
        };
        if self.loading.image || self.runtime.api.is_none() {
            return;
        }
        self.loading.image = true;
        self.queue.set_loading(true);
        self.queue_command(UiCommand::NextImage {
            dataset_id: self.config.dataset_id.clone(),
            task_id: task.task_id,
            prelabel_config_ids: task.prelabel_config_ids,
            kind: self.queue_mode.assignment_kind(),
        });
    }

    pub(crate) fn request_save(&mut self, submit: bool) {
        let Some(current) = self.current.clone() else {
            return;
        };
        if self.loading.saving || self.runtime.api.is_none() {
            return;
        }
        self.loading.saving = true;
        self.save_status = SaveStatus::Syncing;
        self.queue_command(UiCommand::SaveAnnotations {
            dataset_id: self.config.dataset_id.clone(),
            image_id: current.image.image_id,
            user_id: self.config.user_id.clone(),
            task_id: self.selected_task().map(|task| task.task_id.clone()),
            annotations: self.annotations.clone(),
            persisted: self.persisted_annotations.clone(),
            submit,
        });
    }

    pub(crate) fn request_review(&mut self, decision: ReviewDecision) {
        let (Some(current), Some(task)) = (self.current.clone(), self.selected_task().cloned())
        else {
            return;
        };
        if self.loading.saving || self.runtime.api.is_none() {
            return;
        }
        self.loading.saving = true;
        self.queue_command(UiCommand::Review {
            dataset_id: self.config.dataset_id.clone(),
            image_id: current.image.image_id,
            review: ReviewRecord {
                review_id: ReviewId::generate(),
                target: ReviewTarget::Task {
                    task_id: task.task_id,
                },
                reviewer_user_id: self.config.user_id.clone(),
                decision,
                timestamp: labello_domain::now(),
                comment: None,
            },
        });
    }

    pub(crate) fn request_adjudication(&mut self, decision: AdjudicationDecision) {
        let (Some(current), Some(task)) = (self.current.clone(), self.selected_task().cloned())
        else {
            return;
        };
        if self.loading.saving || self.runtime.api.is_none() {
            return;
        }
        self.loading.saving = true;
        self.queue_command(UiCommand::Adjudication {
            dataset_id: self.config.dataset_id.clone(),
            image_id: current.image.image_id,
            adjudication: AdjudicationRecord {
                adjudication_id: AdjudicationId::generate(),
                task_id: task.task_id,
                annotation_ids: self
                    .annotations
                    .iter()
                    .filter(|annotation| !annotation.deleted)
                    .map(|annotation| annotation.annotation_id.clone())
                    .collect(),
                adjudicator_user_id: self.config.user_id.clone(),
                decision,
                resolution: "Resolved in Labello UI".to_string(),
                timestamp: labello_domain::now(),
            },
        });
    }
}

struct SaveAnnotationsJob {
    api: Rc<dyn LabelloApi>,
    dataset_id: labello_domain::DatasetId,
    image_id: ImageId,
    user_id: labello_domain::UserId,
    task_id: Option<labello_domain::TaskId>,
    annotations: Vec<labello_domain::AnnotationVersion>,
    persisted: BTreeSet<AnnotationId>,
    submit: bool,
}

async fn save_annotations(
    job: SaveAnnotationsJob,
) -> labello_client::ClientResult<labello_domain::ImageState> {
    for annotation in job.annotations {
        if job.persisted.contains(&annotation.annotation_id) && annotation.deleted {
            job.api
                .append_event(
                    &job.dataset_id,
                    &job.image_id,
                    AppendEventRequest {
                        payload: EventPayload::AnnotationDeleted {
                            annotation_id: annotation.annotation_id,
                            version: annotation.version,
                            reason: None,
                        },
                    },
                )
                .await?;
        } else if !job.persisted.contains(&annotation.annotation_id) && !annotation.deleted {
            job.api
                .append_event(
                    &job.dataset_id,
                    &job.image_id,
                    AppendEventRequest {
                        payload: EventPayload::AnnotationVersionCreated {
                            annotation,
                            previous_version: None,
                            reason: None,
                        },
                    },
                )
                .await?;
        }
    }
    if job.submit
        && let Some(task_id) = job.task_id
    {
        let timestamp = labello_domain::now();
        job.api
            .append_event(
                &job.dataset_id,
                &job.image_id,
                AppendEventRequest {
                    payload: EventPayload::TaskStateChanged {
                        task_state: TaskState {
                            task_id,
                            status: TaskStatus::Submitted,
                            assigned_to: Some(job.user_id.clone()),
                            completed_by: Some(job.user_id),
                            completed_at: Some(timestamp),
                            updated_at: timestamp,
                        },
                    },
                },
            )
            .await?;
    }
    job.api.rebuild_image(&job.dataset_id, &job.image_id).await
}

async fn load_next_image(
    api: Rc<dyn LabelloApi>,
    dataset_id: labello_domain::DatasetId,
    task_id: labello_domain::TaskId,
    prelabel_config_ids: Vec<PrelabelConfigId>,
    kind: AssignmentKind,
) -> labello_client::ClientResult<LoadedImage> {
    let assignment = api
        .assign_next_image(
            &dataset_id,
            AssignNextRequest {
                task_id: task_id.clone(),
                kind: Some(kind),
            },
        )
        .await?
        .ok_or_else(|| labello_client::ClientError::Demo("no images available".to_string()))?;
    load_image(
        api,
        dataset_id,
        assignment.image_id,
        task_id,
        prelabel_config_ids,
    )
    .await
}

async fn load_image(
    api: Rc<dyn LabelloApi>,
    dataset_id: labello_domain::DatasetId,
    image_id: ImageId,
    task_id: labello_domain::TaskId,
    prelabel_config_ids: Vec<PrelabelConfigId>,
) -> labello_client::ClientResult<LoadedImage> {
    let image = api.get_image_record(&dataset_id, &image_id).await?;
    let state = api.get_image_state(&dataset_id, &image_id).await?;
    let preview = api.get_image_preview(&dataset_id, &image_id, 1600).await?;
    let color_image = Some(egui::ColorImage::from_rgba_unmultiplied(
        [preview.width as usize, preview.height as usize],
        &preview.rgba,
    ));
    let mut prelabels = Vec::new();
    for config_id in prelabel_config_ids {
        let mut suggestions = api
            .prelabel_suggestions(
                &dataset_id,
                PrelabelSuggestionRequest {
                    config_id,
                    task_id: task_id.clone(),
                },
            )
            .await?;
        prelabels.append(&mut suggestions);
    }
    let annotations = state.active_annotations().cloned().collect();
    Ok(LoadedImage {
        queued: QueuedImage { image, prelabels },
        annotations,
        state,
        color_image,
    })
}
