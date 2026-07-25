use labello_client::AssignmentActionRequest;
use labello_domain::{
    Actor, AnnotationOrigin, AssignmentKind, DatasetMetadata, DatasetRole, EventPayload,
    HumanRevisionKind, ImageId, ImageState, RevisionSource, TaskId, Timestamp,
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
                annotation.revision_source,
                RevisionSource::ReviewerCorrection { .. } | RevisionSource::Import { .. }
            ) {
                return Err(ApiError::BadRequest(
                    "import and reviewer correction provenance is created by dedicated server endpoints only"
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
            if let RevisionSource::PrelabelSuggestion {
                config_id,
                model_id,
                confidence,
            } = &annotation.revision_source
            {
                let valid_source = confidence.is_finite()
                    && (0.0..=1.0).contains(confidence)
                    && metadata.prelabel_configs.iter().any(|config| {
                        config.available_to_annotators
                            && config.config_id == *config_id
                            && config.model.model_id == *model_id
                    });
                if !valid_source {
                    return Err(ApiError::BadRequest(
                        "annotation prelabel provenance is not an available configured model"
                            .to_string(),
                    ));
                }
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
        EventPayload::ImportInitialized { .. }
        | EventPayload::ImportedTaskReopened { .. }
        | EventPayload::ImportCoverageIncluded { .. }
        | EventPayload::MigrationDispositionChanged { .. }
        | EventPayload::MigrationDispositionReopened { .. }
        | EventPayload::MigrationDependencyMarked { .. }
        | EventPayload::MigrationDependencyCleared { .. }
        | EventPayload::MigrationPassStarted { .. }
        | EventPayload::MigrationPassItemRecorded { .. }
        | EventPayload::MigrationFullImageConfirmed { .. } => {
            return Err(server_owned_payload_error());
        }
    }
    Ok(())
}

pub(super) fn construct_annotation_mutation(
    actor: &Actor,
    image_state: &ImageState,
    timestamp: Timestamp,
    payload: EventPayload,
) -> ApiResult<EventPayload> {
    let EventPayload::AnnotationVersionCreated {
        mut annotation,
        previous_version,
        reason,
    } = payload
    else {
        return Ok(payload);
    };
    if reason.as_ref().is_some_and(|reason| reason.len() > 2_000) {
        return Err(ApiError::BadRequest(
            "annotation mutation reason exceeds 2000 bytes".to_string(),
        ));
    }

    if let Some(current) = image_state.current_annotation(&annotation.annotation_id) {
        if previous_version.is_none()
            && annotation.version == current.version
            && annotation.task_id == current.task_id
            && annotation.class_id == current.class_id
            && annotation.annotation_type == current.annotation_type
            && annotation.geometry == current.geometry
            && annotation.author_user_id == actor.user_id
            && !current.deleted
        {
            return Ok(EventPayload::AnnotationVersionCreated {
                annotation: current.clone(),
                previous_version,
                reason,
            });
        }
        if previous_version != Some(current.version) {
            return Err(ApiError::BadRequest(format!(
                "annotation {} expected version {}",
                annotation.annotation_id, current.version
            )));
        }
        if annotation.origin != current.origin
            || annotation.object_group_id != current.object_group_id
        {
            return Err(ApiError::BadRequest(
                "annotation origin and objectGroupId are immutable".to_string(),
            ));
        }
        annotation.version = current
            .version
            .checked_add(1)
            .ok_or_else(|| ApiError::BadRequest("annotation version overflow".to_string()))?;
        annotation.origin = current.origin.clone();
        annotation.object_group_id = current.object_group_id.clone();
        annotation.task_id = current.task_id.clone();
        annotation.annotation_type = current.annotation_type.clone();
        annotation.created_at = current.created_at;
        annotation.revision_source = RevisionSource::Human {
            action: if annotation.geometry == current.geometry {
                HumanRevisionKind::AcceptedUnchanged
            } else {
                HumanRevisionKind::Edited
            },
        };
    } else {
        if previous_version.is_some() {
            return Err(ApiError::BadRequest(format!(
                "annotation {} does not exist",
                annotation.annotation_id
            )));
        }
        if annotation.object_group_id.is_some()
            || !matches!(annotation.origin, AnnotationOrigin::Native { .. })
        {
            return Err(ApiError::BadRequest(
                "new ordinary annotations cannot set import origin or objectGroupId".to_string(),
            ));
        }
        annotation.version = 1;
        annotation.origin = AnnotationOrigin::native();
        annotation.object_group_id = None;
        annotation.created_at = timestamp;
        annotation.revision_source = match annotation.revision_source {
            RevisionSource::Human { .. } => RevisionSource::Human {
                action: HumanRevisionKind::Authored,
            },
            source @ RevisionSource::PrelabelSuggestion { .. } => source,
            RevisionSource::Import { .. } | RevisionSource::ReviewerCorrection { .. } => {
                return Err(ApiError::BadRequest(
                    "import and reviewer correction provenance is server-owned".to_string(),
                ));
            }
        };
    }
    annotation.author_user_id = actor.user_id.clone();
    annotation.updated_at = timestamp;
    annotation.deleted = false;

    Ok(EventPayload::AnnotationVersionCreated {
        annotation,
        previous_version,
        reason,
    })
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
        EventPayload::ImportInitialized { .. }
        | EventPayload::ImportedTaskReopened { .. }
        | EventPayload::ImportCoverageIncluded { .. }
        | EventPayload::MigrationDispositionChanged { .. }
        | EventPayload::MigrationDispositionReopened { .. }
        | EventPayload::MigrationDependencyMarked { .. }
        | EventPayload::MigrationDependencyCleared { .. }
        | EventPayload::MigrationPassStarted { .. }
        | EventPayload::MigrationPassItemRecorded { .. }
        | EventPayload::MigrationFullImageConfirmed { .. } => Err(server_owned_payload_error()),
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
        | EventPayload::AssignmentUpdated { .. }
        | EventPayload::ImportInitialized { .. }
        | EventPayload::ImportedTaskReopened { .. }
        | EventPayload::ImportCoverageIncluded { .. }
        | EventPayload::MigrationDispositionChanged { .. }
        | EventPayload::MigrationDispositionReopened { .. }
        | EventPayload::MigrationDependencyMarked { .. }
        | EventPayload::MigrationDependencyCleared { .. }
        | EventPayload::MigrationPassStarted { .. }
        | EventPayload::MigrationPassItemRecorded { .. }
        | EventPayload::MigrationFullImageConfirmed { .. } => {
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
    image_state: &ImageState,
    payload: &EventPayload,
) -> ApiResult<()> {
    let (annotation_id, task_id) = match payload {
        EventPayload::AnnotationVersionCreated { annotation, .. } => {
            (Some(&annotation.annotation_id), Some(&annotation.task_id))
        }
        EventPayload::AnnotationDeleted { annotation_id, .. } => (
            Some(annotation_id),
            image_state
                .current_annotation(annotation_id)
                .map(|annotation| &annotation.task_id),
        ),
        _ => (None, None),
    };
    let manual_task = task_id.is_some_and(|task_id| {
        metadata
            .task(task_id)
            .is_some_and(|task| task.manual_box_guide_migration.is_some())
    });
    let reserved_target = annotation_id.is_some_and(|annotation_id| {
        image_state.migration_target_sets.values().any(|set| {
            set.targets
                .iter()
                .any(|target| target.reserved_skeleton_annotation_id == *annotation_id)
        })
    });
    if manual_task || reserved_target {
        return Err(ApiError::BadRequest(
            "manual migration skeletons can be changed only by migration commands".to_string(),
        ));
    }
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
        EventPayload::ImportInitialized { .. }
        | EventPayload::ImportedTaskReopened { .. }
        | EventPayload::ImportCoverageIncluded { .. }
        | EventPayload::MigrationDispositionChanged { .. }
        | EventPayload::MigrationDispositionReopened { .. }
        | EventPayload::MigrationDependencyMarked { .. }
        | EventPayload::MigrationDependencyCleared { .. }
        | EventPayload::MigrationPassStarted { .. }
        | EventPayload::MigrationPassItemRecorded { .. }
        | EventPayload::MigrationFullImageConfirmed { .. } => Err(server_owned_payload_error()),
    }
}

fn server_owned_payload_error() -> ApiError {
    ApiError::BadRequest(
        "import and migration events are created by dedicated server endpoints only".to_string(),
    )
}
