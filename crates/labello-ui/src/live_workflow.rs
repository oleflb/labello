use std::rc::Rc;

use eframe::egui;
use labello_client::{
    AppendEventRequest, AssignNextRequest, LabelloApi, PrelabelSuggestionRequest,
};
use labello_domain::{
    AdjudicationDecision, AdjudicationId, AdjudicationRecord, AssignmentKind, EventPayload,
    ImageId, ReviewDecision, ReviewId, ReviewRecord, ReviewTarget, TaskState, TaskStatus,
};

use crate::{
    app::{LabelloApp, LoadedImage, QueueMode, SaveStatus, UiMessage},
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
    pub(crate) fn request_next_image(&mut self) {
        let (Some(api), Some(task)) = (self.runtime.api.clone(), self.selected_task().cloned())
        else {
            return;
        };
        let dataset_id = self.config.dataset_id.clone();
        let kind = self.queue_mode.assignment_kind();
        self.loading.image = true;
        self.queue.set_loading(true);
        self.spawn_message(async move {
            let result = load_next_image(api, dataset_id, task.task_id, kind).await;
            UiMessage::ImageLoaded(result.map_err(|error| error.to_string()))
        });
    }

    pub(crate) fn request_save(&mut self, submit: bool) {
        let (Some(api), Some(current)) = (self.runtime.api.clone(), self.current.clone()) else {
            return;
        };
        if self.loading.saving {
            return;
        }
        let dataset_id = self.config.dataset_id.clone();
        let user_id = self.config.user_id.clone();
        let task_id = self.selected_task().map(|task| task.task_id.clone());
        let annotations = self.annotations.clone();
        let persisted = self.persisted_annotations.clone();
        self.loading.saving = true;
        self.save_status = SaveStatus::Syncing;
        self.spawn_message(async move {
            let result = async {
                for annotation in annotations {
                    if persisted.contains(&annotation.annotation_id) && annotation.deleted {
                        api.append_event(
                            &dataset_id,
                            &current.image.image_id,
                            AppendEventRequest {
                                payload: EventPayload::AnnotationDeleted {
                                    annotation_id: annotation.annotation_id,
                                    version: annotation.version,
                                    reason: None,
                                },
                            },
                        )
                        .await?;
                    } else if !persisted.contains(&annotation.annotation_id) && !annotation.deleted
                    {
                        api.append_event(
                            &dataset_id,
                            &current.image.image_id,
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
                if submit && let Some(task_id) = task_id {
                    let timestamp = labello_domain::now();
                    api.append_event(
                        &dataset_id,
                        &current.image.image_id,
                        AppendEventRequest {
                            payload: EventPayload::TaskStateChanged {
                                task_state: TaskState {
                                    task_id,
                                    status: TaskStatus::Submitted,
                                    assigned_to: Some(user_id.clone()),
                                    completed_by: Some(user_id),
                                    completed_at: Some(timestamp),
                                    updated_at: timestamp,
                                },
                            },
                        },
                    )
                    .await?;
                }
                api.rebuild_image(&dataset_id, &current.image.image_id)
                    .await
            }
            .await
            .map_err(|error| error.to_string());
            UiMessage::SaveFinished(result)
        });
    }

    pub(crate) fn request_review(&mut self, decision: ReviewDecision) {
        let (Some(api), Some(current), Some(task)) = (
            self.runtime.api.clone(),
            self.current.clone(),
            self.selected_task().cloned(),
        ) else {
            return;
        };
        let dataset_id = self.config.dataset_id.clone();
        let review = ReviewRecord {
            review_id: ReviewId::generate(),
            target: ReviewTarget::Task {
                task_id: task.task_id,
            },
            reviewer_user_id: self.config.user_id.clone(),
            decision,
            timestamp: labello_domain::now(),
            comment: None,
        };
        self.loading.saving = true;
        self.spawn_message(async move {
            UiMessage::ReviewFinished(
                api.record_review(&dataset_id, &current.image.image_id, review)
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string()),
            )
        });
    }

    pub(crate) fn request_adjudication(&mut self, decision: AdjudicationDecision) {
        let (Some(api), Some(current), Some(task)) = (
            self.runtime.api.clone(),
            self.current.clone(),
            self.selected_task().cloned(),
        ) else {
            return;
        };
        let dataset_id = self.config.dataset_id.clone();
        let annotation_ids = self
            .annotations
            .iter()
            .filter(|annotation| !annotation.deleted)
            .map(|annotation| annotation.annotation_id.clone())
            .collect();
        let adjudication = AdjudicationRecord {
            adjudication_id: AdjudicationId::generate(),
            task_id: task.task_id,
            annotation_ids,
            adjudicator_user_id: self.config.user_id.clone(),
            decision,
            resolution: "Resolved in Labello UI".to_string(),
            timestamp: labello_domain::now(),
        };
        self.loading.saving = true;
        self.spawn_message(async move {
            UiMessage::AdjudicationFinished(
                api.record_adjudication(&dataset_id, &current.image.image_id, adjudication)
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string()),
            )
        });
    }
}

async fn load_next_image(
    api: Rc<dyn LabelloApi>,
    dataset_id: labello_domain::DatasetId,
    task_id: labello_domain::TaskId,
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
    load_image(api, dataset_id, assignment.image_id, task_id).await
}

async fn load_image(
    api: Rc<dyn LabelloApi>,
    dataset_id: labello_domain::DatasetId,
    image_id: ImageId,
    task_id: labello_domain::TaskId,
) -> labello_client::ClientResult<LoadedImage> {
    let metadata = api.get_dataset(&dataset_id).await?;
    let image =
        metadata.images.get(&image_id).cloned().ok_or_else(|| {
            labello_client::ClientError::Demo(format!("image {image_id} not found"))
        })?;
    let state = api.get_image_state(&dataset_id, &image_id).await?;
    let file = api.get_image_file(&dataset_id, &image_id).await?;
    let color_image = decode_image(&file.bytes).ok();
    let mut prelabels = Vec::new();
    if let Some(task) = metadata.task(&task_id) {
        for config in metadata
            .prelabel_configs
            .iter()
            .filter(|config| config.available_to_annotators)
            .filter(|config| task.prelabel_config_ids.contains(&config.config_id))
        {
            let mut suggestions = api
                .prelabel_suggestions(
                    &dataset_id,
                    PrelabelSuggestionRequest {
                        config_id: config.config_id.clone(),
                        task_id: task_id.clone(),
                    },
                )
                .await?;
            prelabels.append(&mut suggestions);
        }
    }
    let annotations = state.active_annotations().cloned().collect();
    Ok(LoadedImage {
        queued: QueuedImage { image, prelabels },
        annotations,
        state,
        color_image,
    })
}

fn decode_image(bytes: &[u8]) -> Result<egui::ColorImage, image::ImageError> {
    let rgba = image::load_from_memory(bytes)?.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        size,
        rgba.as_raw(),
    ))
}
