use labello_client::AssignmentActionRequest;
use labello_domain::{
    Actor, AnnotationSource, AssignmentKind, DatasetMetadata, DatasetRole, EventPayload, ImageId,
    ImageState, TaskId,
};

use crate::error::{ApiError, ApiResult};

pub(super) fn validate_payload(
    metadata: &DatasetMetadata,
    image_id: &ImageId,
    payload: &EventPayload,
) -> ApiResult<()> {
    match payload {
        EventPayload::AnnotationVersionCreated { annotation, .. } => {
            if matches!(
                annotation.source,
                AnnotationSource::ReviewerCorrection { .. }
            ) {
                return Err(ApiError::BadRequest(
                    "reviewer correction provenance is created by the correction endpoint only"
                        .to_string(),
                ));
            }
            let record = metadata
                .images
                .get(image_id)
                .ok_or_else(|| ApiError::NotFound(format!("image {image_id}")))?;
            let task = metadata.task(&annotation.task_id).ok_or_else(|| {
                ApiError::BadRequest(format!("unknown task {}", annotation.task_id))
            })?;
            annotation
                .validate_for_task(task, record.dimensions())
                .map_err(|error| ApiError::BadRequest(error.to_string()))?;
            if matches!(
                annotation.source,
                AnnotationSource::PrelabelSuggestion { .. }
            ) {
                annotation
                    .geometry
                    .validate()
                    .map_err(|error| ApiError::BadRequest(error.to_string()))?;
            }
        }
        EventPayload::TaskStateChanged { task_state } => {
            if metadata.task(&task_state.task_id).is_none() {
                return Err(ApiError::BadRequest(format!(
                    "unknown task {}",
                    task_state.task_id
                )));
            }
            if !metadata.images.contains_key(image_id) {
                return Err(ApiError::NotFound(format!("image {image_id}")));
            }
        }
        EventPayload::AnnotationDeleted { .. }
        | EventPayload::ReviewRecorded { .. }
        | EventPayload::ReviewerCorrectionRecorded { .. }
        | EventPayload::AdjudicationRecorded { .. }
        | EventPayload::AssignmentUpdated { .. } => {}
    }
    Ok(())
}

pub(super) fn required_role_for_payload(
    actor: &Actor,
    payload: &EventPayload,
) -> ApiResult<DatasetRole> {
    match payload {
        EventPayload::AnnotationVersionCreated { annotation, .. } => {
            if annotation.author_user_id != actor.user_id {
                return Err(ApiError::Unauthorized(
                    "cannot create annotations for another user".to_string(),
                ));
            }
            Ok(DatasetRole::Annotator)
        }
        EventPayload::AnnotationDeleted { .. } => Ok(DatasetRole::Annotator),
        EventPayload::TaskStateChanged { task_state } => {
            if task_state
                .assigned_to
                .as_ref()
                .is_some_and(|user_id| user_id != &actor.user_id)
                || task_state
                    .completed_by
                    .as_ref()
                    .is_some_and(|user_id| user_id != &actor.user_id)
            {
                return Err(ApiError::Unauthorized(
                    "cannot submit task state for another user".to_string(),
                ));
            }
            Ok(DatasetRole::Annotator)
        }
        EventPayload::ReviewRecorded { review } => {
            if review.reviewer_user_id != actor.user_id {
                return Err(ApiError::Unauthorized(
                    "cannot record reviews for another user".to_string(),
                ));
            }
            Ok(DatasetRole::Reviewer)
        }
        EventPayload::ReviewerCorrectionRecorded { .. } => Err(ApiError::BadRequest(
            "reviewer correction events are created by the correction endpoint only".to_string(),
        )),
        EventPayload::AdjudicationRecorded { adjudication } => {
            if adjudication.adjudicator_user_id != actor.user_id {
                return Err(ApiError::Unauthorized(
                    "cannot record adjudications for another user".to_string(),
                ));
            }
            Ok(DatasetRole::Adjudicator)
        }
        EventPayload::AssignmentUpdated { .. } => Err(ApiError::BadRequest(
            "assignment events are created by assignment endpoints only".to_string(),
        )),
    }
}

pub(super) fn validate_assignment_request(
    assignment: &AssignmentActionRequest,
    image_id: &ImageId,
    kind: AssignmentKind,
) -> ApiResult<()> {
    if &assignment.image_id != image_id {
        return Err(ApiError::BadRequest(
            "assignment imageId does not match path image".to_string(),
        ));
    }
    if assignment.kind != kind {
        return Err(ApiError::BadRequest(format!(
            "assignment kind must be {kind:?}"
        )));
    }
    Ok(())
}

pub(super) fn validate_annotation_assignment_payload(
    image_state: &ImageState,
    task_id: &TaskId,
    payload: &EventPayload,
) -> ApiResult<()> {
    let payload_task_id = match payload {
        EventPayload::AnnotationVersionCreated { annotation, .. } => &annotation.task_id,
        EventPayload::AnnotationDeleted { annotation_id, .. } => {
            &image_state
                .current_annotation(annotation_id)
                .ok_or_else(|| ApiError::BadRequest(format!("unknown annotation {annotation_id}")))?
                .task_id
        }
        EventPayload::TaskStateChanged { .. } => {
            return Err(ApiError::BadRequest(
                "complete annotation assignments through the assignment completion endpoint"
                    .to_string(),
            ));
        }
        EventPayload::ReviewRecorded { .. }
        | EventPayload::ReviewerCorrectionRecorded { .. }
        | EventPayload::AdjudicationRecorded { .. }
        | EventPayload::AssignmentUpdated { .. } => {
            return Err(ApiError::BadRequest(
                "annotation assignments only accept annotation mutations".to_string(),
            ));
        }
    };
    if payload_task_id != task_id {
        return Err(ApiError::BadRequest(
            "payload task does not match assignment task".to_string(),
        ));
    }
    Ok(())
}

pub(super) fn validate_admin_repair_payload(
    metadata: &DatasetMetadata,
    image_id: &ImageId,
    payload: &EventPayload,
) -> ApiResult<()> {
    match payload {
        EventPayload::AnnotationVersionCreated { .. }
        | EventPayload::AnnotationDeleted { .. }
        | EventPayload::TaskStateChanged { .. }
        | EventPayload::ReviewRecorded { .. }
        | EventPayload::AdjudicationRecorded { .. } => {
            validate_payload(metadata, image_id, payload)
        }
        EventPayload::ReviewerCorrectionRecorded { .. }
        | EventPayload::AssignmentUpdated { .. } => Err(ApiError::BadRequest(
            "assignment and reviewer correction state is managed by workflow endpoints".to_string(),
        )),
    }
}
