use axum::{
    Json,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
};
use labello_client::{
    AddMigrationSkeletonRequest, AppendEventRequest, AssignNextRequest, AssignmentActionRequest,
    AssignmentAvailability, AssignmentAvailabilityEntry, AssignmentAvailabilityRequest,
    AssignmentRevalidation, ConfirmMigrationRequest, CorrectionRequest,
    DeleteMigrationSkeletonRequest, EditMigrationSkeletonRequest, ExcludeMigrationTargetRequest,
    KeepMigrationTargetRequest, ManualMigrationCommandResult, OfflineBundleRequest,
    PrelabelSuggestionRequest, ReopenMigrationTargetRequest, ReviewMigrationRequest,
    RevisitMigrationTargetRequest, SaveMigrationSkeletonRequest, StartMigrationPassRequest,
};
use labello_domain::{
    Actor, AdjudicationDecision, AnnotationGeometry, AnnotationType, Assignment, AssignmentKind,
    DatasetId, DatasetRole, EventPayload, ImageId, KeybindingSet, OfflineSyncRequest,
    PrelabelSuggestion, TaskOutcome, TaskState, TaskStatus,
};
use labello_storage::assignment::AssignmentContext;

use crate::{
    ApiState,
    auth::{actor_from_headers, ensure_any_dataset_role, ensure_dataset_role},
    error::{ApiError, ApiResult},
};

mod event_policy;

use event_policy::{
    construct_annotation_mutation, required_role_for_payload, validate_admin_repair_payload,
    validate_annotation_assignment_payload, validate_assignment_request, validate_payload,
};

#[derive(serde::Deserialize)]
pub(crate) struct PreviewQuery {
    #[serde(default = "default_preview_max")]
    max: u32,
}

fn default_preview_max() -> u32 {
    1600
}

pub(crate) async fn assignment_availability(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
    Query(request): Query<AssignmentAvailabilityRequest>,
) -> ApiResult<Json<AssignmentAvailability>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let mut availabilities = repo
        .assignment_availabilities(&actor.user_id, request.kind.clone())
        .await?;
    let requested = availabilities
        .iter()
        .position(|(kind, _)| kind == &request.kind)
        .expect("the authorized requested kind must be included");
    let (_, tasks) = availabilities.remove(requested);
    Ok(Json(AssignmentAvailability {
        kind: request.kind,
        tasks,
        related: availabilities
            .into_iter()
            .map(|(kind, tasks)| AssignmentAvailabilityEntry { kind, tasks })
            .collect(),
    }))
}

pub(crate) async fn assign_next(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
    Json(mut request): Json<AssignNextRequest>,
) -> ApiResult<Json<Option<labello_domain::Assignment>>> {
    request.task_id.validate_path_segment()?;
    if let Some(assignment_id) = &request.assignment_id {
        assignment_id.validate_path_segment()?;
    }
    for image_id in &request.excluded_image_ids {
        image_id.validate_path_segment()?;
    }
    request.excluded_image_ids.sort();
    request.excluded_image_ids.dedup();
    if request.excluded_image_ids.len() > 3 {
        return Err(ApiError::BadRequest(
            "at most 3 image IDs may be excluded".to_string(),
        ));
    }
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let kind = request
        .kind
        .unwrap_or(labello_domain::AssignmentKind::Annotation);
    if let Some(assignment_id) = request.assignment_id
        && let Some(assignment) = repo
            .reclaim_assignment(
                &actor.user_id,
                &assignment_id,
                &request.task_id,
                kind.clone(),
            )
            .await?
    {
        tracing::debug!(
            event = "assignment.reclaimed",
            dataset_id = %dataset_id,
            user_id = %actor.user_id,
            assignment_id = %assignment.assignment_id,
            "assignment reclaimed"
        );
        return Ok(Json(Some(assignment)));
    }
    let assignment = repo
        .assign_next_image_excluding(
            &actor.user_id,
            &request.task_id,
            kind,
            &request.excluded_image_ids,
        )
        .await?;
    if let Some(assignment) = &assignment {
        tracing::debug!(
            event = "assignment.claimed",
            dataset_id = %dataset_id,
            user_id = %actor.user_id,
            assignment_id = %assignment.assignment_id,
            "assignment claimed"
        );
    } else {
        tracing::debug!(
            event = "assignment.unavailable",
            dataset_id = %dataset_id,
            user_id = %actor.user_id,
            "no assignment available"
        );
    }
    Ok(Json(assignment))
}

pub(crate) async fn release_assignment(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
    Json(request): Json<AssignmentActionRequest>,
) -> ApiResult<Json<labello_domain::Assignment>> {
    request.image_id.validate_path_segment()?;
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let assignment = repo
        .release_assignment(
            &actor.user_id,
            &request.assignment_id,
            &request.image_id,
            &request.task_id,
            request.kind,
        )
        .await?;
    tracing::debug!(
        event = "assignment.released",
        dataset_id = %dataset_id,
        user_id = %actor.user_id,
        assignment_id = %assignment.assignment_id,
        "assignment released"
    );
    Ok(Json(assignment))
}

pub(crate) async fn revalidate_assignment(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    headers: HeaderMap,
    Json(request): Json<AssignmentActionRequest>,
) -> ApiResult<Json<Option<AssignmentRevalidation>>> {
    image_id.validate_path_segment()?;
    request.assignment_id.validate_path_segment()?;
    request.image_id.validate_path_segment()?;
    request.task_id.validate_path_segment()?;
    if request.image_id != image_id {
        return Err(ApiError::BadRequest(
            "assignment image does not match the route image".to_string(),
        ));
    }
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let revalidated = repo
        .revalidate_assignment_on_image(
            &actor.user_id,
            &request.assignment_id,
            &image_id,
            &request.task_id,
            request.kind,
        )
        .await?
        .map(|(assignment, state)| AssignmentRevalidation { assignment, state });
    Ok(Json(revalidated))
}

pub(crate) async fn complete_assignment(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
    Json(request): Json<AssignmentActionRequest>,
) -> ApiResult<Json<labello_domain::Assignment>> {
    request.image_id.validate_path_segment()?;
    if request.kind != AssignmentKind::Annotation {
        return Err(ApiError::BadRequest(
            "review and adjudication assignments complete with their final decision".to_string(),
        ));
    }
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let assignment = repo
        .complete_assignment(
            &actor.user_id,
            &request.assignment_id,
            &request.image_id,
            &request.task_id,
            request.kind,
        )
        .await?;
    tracing::debug!(
        event = "assignment.completed",
        dataset_id = %dataset_id,
        user_id = %actor.user_id,
        assignment_id = %assignment.assignment_id,
        "assignment completed"
    );
    Ok(Json(assignment))
}

pub(crate) async fn reopen_assignment(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
    Json(request): Json<AssignmentActionRequest>,
) -> ApiResult<Json<labello_domain::Assignment>> {
    request.assignment_id.validate_path_segment()?;
    request.image_id.validate_path_segment()?;
    request.task_id.validate_path_segment()?;
    if request.kind != AssignmentKind::Annotation {
        return Err(ApiError::BadRequest(
            "only annotation assignments can be reopened".to_string(),
        ));
    }
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let assignment = repo
        .reopen_annotation_assignment(
            &actor.user_id,
            &request.assignment_id,
            &request.image_id,
            &request.task_id,
            request.kind,
        )
        .await?;
    tracing::debug!(
        event = "assignment.reopened",
        dataset_id = %dataset_id,
        user_id = %actor.user_id,
        assignment_id = %assignment.assignment_id,
        "assignment reopened"
    );
    Ok(Json(assignment))
}

pub(crate) async fn get_image_state(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    headers: HeaderMap,
) -> ApiResult<Json<labello_domain::ImageState>> {
    image_id.validate_path_segment()?;
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset_config().await?;
    ensure_any_dataset_role(&metadata, &actor)?;
    repo.load_image_record(&image_id).await?;
    Ok(Json(repo.load_image_state(&image_id).await?))
}

pub(crate) async fn get_image_record(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    headers: HeaderMap,
) -> ApiResult<Json<labello_domain::ImageRecord>> {
    image_id.validate_path_segment()?;
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset_config().await?;
    ensure_any_dataset_role(&metadata, &actor)?;
    let record = repo.load_image_record(&image_id).await?;
    Ok(Json(record))
}

pub(crate) async fn get_image_file(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    image_id.validate_path_segment()?;
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset_config().await?;
    ensure_any_dataset_role(&metadata, &actor)?;
    let record = repo.load_image_record(&image_id).await?;
    let path = repo.image_path(&record.canonical_path)?;
    let bytes =
        tokio::fs::read(&path)
            .await
            .map_err(|source| labello_storage::StorageError::Io {
                path: path.clone(),
                source,
            })?;
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, record.media_type.clone())],
        Bytes::from(bytes),
    ))
}

pub(crate) async fn get_image_preview(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    Query(query): Query<PreviewQuery>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    image_id.validate_path_segment()?;
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset_config().await?;
    ensure_any_dataset_role(&metadata, &actor)?;
    let record = repo.load_image_record(&image_id).await?;
    let path = repo.image_path(&record.canonical_path)?;
    let max = query.max.clamp(256, 4096);
    let (width, height, rgba) = tokio::task::spawn_blocking(move || preview_rgba(path, max))
        .await
        .map_err(|error| ApiError::BadRequest(error.to_string()))??;
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (
                header::HeaderName::from_static("x-image-width"),
                width.to_string(),
            ),
            (
                header::HeaderName::from_static("x-image-height"),
                height.to_string(),
            ),
        ],
        Bytes::from(rgba),
    ))
}

fn preview_rgba(path: std::path::PathBuf, max: u32) -> Result<(u32, u32, Vec<u8>), ApiError> {
    let image = image::open(&path).map_err(|source| labello_storage::StorageError::Image {
        path: path.clone(),
        source,
    })?;
    let image = if image.width().max(image.height()) > max {
        image.resize(max, max, image::imageops::FilterType::Triangle)
    } else {
        image
    };
    let rgba = image.to_rgba8();
    Ok((rgba.width(), rgba.height(), rgba.into_raw()))
}

pub(crate) async fn append_event(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    Query(assignment): Query<AssignmentActionRequest>,
    headers: HeaderMap,
    Json(request): Json<AppendEventRequest>,
) -> ApiResult<Json<labello_domain::EventLogEntry>> {
    image_id.validate_path_segment()?;
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset().await?;
    let required_role = required_role_for_payload(&actor, &request.payload)?;
    ensure_dataset_role(&metadata, &actor, required_role.clone())?;
    validate_assignment_request(&assignment, &image_id, AssignmentKind::Annotation)?;
    let image_state = repo.load_image_state(&image_id).await?;
    validate_annotation_assignment_payload(&image_state, &assignment.task_id, &request.payload)?;
    let payload = construct_annotation_mutation(
        &actor,
        &image_state,
        labello_domain::now(),
        request.payload,
    )?;
    validate_payload(&metadata, &image_id, &payload)?;
    let (events, _) = repo
        .append_for_assignment(
            &actor.user_id,
            AssignmentContext {
                assignment_id: &assignment.assignment_id,
                image_id: &image_id,
                task_id: &assignment.task_id,
                kind: AssignmentKind::Annotation,
            },
            vec![payload],
            false,
        )
        .await?;
    Ok(Json(
        events.into_iter().next().expect("one payload was appended"),
    ))
}

pub(crate) async fn apply_annotation_batch(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    Query(assignment): Query<AssignmentActionRequest>,
    headers: HeaderMap,
    Json(request): Json<labello_client::AnnotationBatchRequest>,
) -> ApiResult<Json<labello_domain::ImageState>> {
    const MAX_BATCH_SIZE: usize = 10_000;

    image_id.validate_path_segment()?;
    if request.payloads.len() > MAX_BATCH_SIZE {
        return Err(ApiError::BadRequest(format!(
            "annotation batch exceeds {MAX_BATCH_SIZE} mutations"
        )));
    }
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::Annotator)?;
    validate_assignment_request(&assignment, &image_id, AssignmentKind::Annotation)?;
    let mut image_state = repo.load_image_state(&image_id).await?;
    let timestamp = labello_domain::now();
    let mut payloads = Vec::with_capacity(request.payloads.len());
    for payload in request.payloads {
        if required_role_for_payload(&actor, &payload)? != DatasetRole::Annotator {
            return Err(ApiError::BadRequest(
                "annotation batches only accept annotation mutations".to_string(),
            ));
        }
        validate_annotation_assignment_payload(&image_state, &assignment.task_id, &payload)?;
        let payload = construct_annotation_mutation(&actor, &image_state, timestamp, payload)?;
        validate_payload(&metadata, &image_id, &payload)?;
        let already_reflected = match &payload {
            EventPayload::AnnotationVersionCreated {
                annotation,
                previous_version: None,
                ..
            } => image_state.current_annotation(&annotation.annotation_id) == Some(annotation),
            EventPayload::AnnotationDeleted {
                annotation_id,
                version,
                ..
            } => image_state
                .current_annotation(annotation_id)
                .is_some_and(|annotation| annotation.version == *version && annotation.deleted),
            _ => false,
        };
        if !already_reflected {
            let event = labello_domain::EventLogEntry::new(
                image_state.current_sequence + 1,
                image_id.clone(),
                actor.user_id.clone(),
                DatasetRole::Annotator,
                timestamp,
                payload.clone(),
            );
            image_state
                .apply_event(&event)
                .map_err(|error| ApiError::BadRequest(error.to_string()))?;
        }
        payloads.push(payload);
    }
    let mutation_count = payloads.len();
    let complete = request.complete;
    let image_state = repo
        .apply_annotation_batch(
            &actor.user_id,
            AssignmentContext {
                assignment_id: &assignment.assignment_id,
                image_id: &image_id,
                task_id: &assignment.task_id,
                kind: AssignmentKind::Annotation,
            },
            payloads,
            request.complete,
        )
        .await?;
    tracing::debug!(
        event = "annotation.batch.saved",
        dataset_id = %dataset_id,
        user_id = %actor.user_id,
        mutation_count,
        complete,
        "annotation batch saved"
    );
    Ok(Json(image_state))
}

pub(crate) async fn save_migration_skeleton(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    headers: HeaderMap,
    Json(request): Json<SaveMigrationSkeletonRequest>,
) -> ApiResult<Json<ManualMigrationCommandResult>> {
    let key = migration_idempotency_key(&headers)?;
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let assignment = migration_assignment(
        &repo,
        &image_id,
        &request.assignment_id,
        &actor,
        DatasetRole::Annotator,
    )
    .await?;
    let expected = storage_expectation(&request.target);
    let result = repo
        .save_migration_skeleton(
            &actor.user_id,
            migration_context(&assignment, &image_id),
            request.pass_id.as_ref(),
            &expected,
            request.skeleton,
            key,
        )
        .await?;
    Ok(Json(client_migration_result(result)))
}

pub(crate) async fn add_migration_skeleton(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    headers: HeaderMap,
    Json(request): Json<AddMigrationSkeletonRequest>,
) -> ApiResult<Json<ManualMigrationCommandResult>> {
    let key = migration_idempotency_key(&headers)?;
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let assignment = migration_assignment(
        &repo,
        &image_id,
        &request.assignment_id,
        &actor,
        DatasetRole::Annotator,
    )
    .await?;
    let result = repo
        .add_migration_skeleton(
            &actor.user_id,
            AssignmentContext {
                assignment_id: &assignment.assignment_id,
                image_id: &image_id,
                task_id: &request.task_id,
                kind: AssignmentKind::Annotation,
            },
            request.pass_id.as_ref(),
            request.skeleton,
            key,
        )
        .await?;
    Ok(Json(client_migration_result(result)))
}

pub(crate) async fn edit_migration_skeleton(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    headers: HeaderMap,
    Json(request): Json<EditMigrationSkeletonRequest>,
) -> ApiResult<Json<ManualMigrationCommandResult>> {
    let key = migration_idempotency_key(&headers)?;
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let assignment = migration_assignment(
        &repo,
        &image_id,
        &request.assignment_id,
        &actor,
        DatasetRole::Annotator,
    )
    .await?;
    let result = repo
        .edit_migration_skeleton(
            &actor.user_id,
            AssignmentContext {
                assignment_id: &assignment.assignment_id,
                image_id: &image_id,
                task_id: &request.task_id,
                kind: AssignmentKind::Annotation,
            },
            request.pass_id.as_ref(),
            &request.annotation_id,
            request.expected_version,
            request.skeleton,
            key,
        )
        .await?;
    Ok(Json(client_migration_result(result)))
}

pub(crate) async fn delete_migration_skeleton(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    headers: HeaderMap,
    Json(request): Json<DeleteMigrationSkeletonRequest>,
) -> ApiResult<Json<ManualMigrationCommandResult>> {
    let key = migration_idempotency_key(&headers)?;
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let assignment = migration_assignment(
        &repo,
        &image_id,
        &request.assignment_id,
        &actor,
        DatasetRole::Annotator,
    )
    .await?;
    let result = repo
        .delete_migration_skeleton(
            &actor.user_id,
            AssignmentContext {
                assignment_id: &assignment.assignment_id,
                image_id: &image_id,
                task_id: &request.task_id,
                kind: AssignmentKind::Annotation,
            },
            request.pass_id.as_ref(),
            &request.annotation_id,
            request.expected_version,
            key,
        )
        .await?;
    Ok(Json(client_migration_result(result)))
}

pub(crate) async fn exclude_migration_target(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    headers: HeaderMap,
    Json(request): Json<ExcludeMigrationTargetRequest>,
) -> ApiResult<Json<ManualMigrationCommandResult>> {
    let key = migration_idempotency_key(&headers)?;
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let assignment = migration_assignment(
        &repo,
        &image_id,
        &request.assignment_id,
        &actor,
        DatasetRole::Annotator,
    )
    .await?;
    let result = repo
        .exclude_migration_target(
            &actor.user_id,
            migration_context(&assignment, &image_id),
            request.pass_id.as_ref(),
            &storage_expectation(&request.target),
            request.reason,
            request.note,
            key,
        )
        .await?;
    Ok(Json(client_migration_result(result)))
}

pub(crate) async fn reopen_migration_target(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    headers: HeaderMap,
    Json(request): Json<ReopenMigrationTargetRequest>,
) -> ApiResult<Json<ManualMigrationCommandResult>> {
    let key = migration_idempotency_key(&headers)?;
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let assignment = migration_assignment(
        &repo,
        &image_id,
        &request.assignment_id,
        &actor,
        DatasetRole::Annotator,
    )
    .await?;
    let result = repo
        .reopen_migration_target(
            &actor.user_id,
            migration_context(&assignment, &image_id),
            request.pass_id.as_ref(),
            &storage_expectation(&request.target),
            key,
        )
        .await?;
    Ok(Json(client_migration_result(result)))
}

pub(crate) async fn revisit_migration_target(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    headers: HeaderMap,
    Json(request): Json<RevisitMigrationTargetRequest>,
) -> ApiResult<Json<ManualMigrationCommandResult>> {
    let key = migration_idempotency_key(&headers)?;
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let assignment = migration_assignment(
        &repo,
        &image_id,
        &request.assignment_id,
        &actor,
        DatasetRole::Annotator,
    )
    .await?;
    let result = repo
        .revisit_migration_target(
            &actor.user_id,
            migration_context(&assignment, &image_id),
            request.pass_id.as_ref(),
            &storage_expectation(&request.target),
            key,
        )
        .await?;
    Ok(Json(client_migration_result(result)))
}

pub(crate) async fn start_migration_pass(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    headers: HeaderMap,
    Json(request): Json<StartMigrationPassRequest>,
) -> ApiResult<Json<ManualMigrationCommandResult>> {
    let key = migration_idempotency_key(&headers)?;
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let assignment = migration_assignment(
        &repo,
        &image_id,
        &request.assignment_id,
        &actor,
        DatasetRole::Annotator,
    )
    .await?;
    let result = repo
        .start_migration_pass(
            &actor.user_id,
            AssignmentContext {
                assignment_id: &assignment.assignment_id,
                image_id: &image_id,
                task_id: &request.task_id,
                kind: AssignmentKind::Annotation,
            },
            &request.expected_target_set_hash,
            &request.expected_state_hash,
            key,
        )
        .await?;
    Ok(Json(client_migration_result(result)))
}

pub(crate) async fn keep_migration_target(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    headers: HeaderMap,
    Json(request): Json<KeepMigrationTargetRequest>,
) -> ApiResult<Json<ManualMigrationCommandResult>> {
    let key = migration_idempotency_key(&headers)?;
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let assignment = migration_assignment(
        &repo,
        &image_id,
        &request.assignment_id,
        &actor,
        DatasetRole::Annotator,
    )
    .await?;
    let result = repo
        .keep_migration_target(
            &actor.user_id,
            migration_context(&assignment, &image_id),
            &request.pass_id,
            &storage_expectation(&request.target),
            key,
        )
        .await?;
    Ok(Json(client_migration_result(result)))
}

pub(crate) async fn confirm_migration(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    headers: HeaderMap,
    Json(request): Json<ConfirmMigrationRequest>,
) -> ApiResult<Json<ManualMigrationCommandResult>> {
    let key = migration_idempotency_key(&headers)?;
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let assignment = migration_assignment(
        &repo,
        &image_id,
        &request.assignment_id,
        &actor,
        DatasetRole::Annotator,
    )
    .await?;
    let result = repo
        .confirm_and_submit_migration(
            &actor.user_id,
            AssignmentContext {
                assignment_id: &assignment.assignment_id,
                image_id: &image_id,
                task_id: &request.task_id,
                kind: AssignmentKind::Annotation,
            },
            &request.target_set_hash,
            &request.state_hash,
            &request.confirmation_hash,
            key,
        )
        .await?;
    Ok(Json(client_migration_result(result)))
}

pub(crate) async fn review_migration(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    headers: HeaderMap,
    Json(request): Json<ReviewMigrationRequest>,
) -> ApiResult<Json<ManualMigrationCommandResult>> {
    let key = migration_idempotency_key(&headers)?;
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let assignment = migration_assignment(
        &repo,
        &image_id,
        &request.assignment_id,
        &actor,
        DatasetRole::Reviewer,
    )
    .await?;
    let target = match request.target {
        labello_client::MigrationReviewTarget::Disposition {
            object_group_id,
            disposition_version,
        } => labello_storage::assignment::MigrationReviewTarget::Disposition {
            object_group_id,
            disposition_version,
        },
        labello_client::MigrationReviewTarget::Confirmation { confirmation_hash } => {
            labello_storage::assignment::MigrationReviewTarget::Confirmation { confirmation_hash }
        }
    };
    let result = repo
        .review_migration(
            &actor.user_id,
            AssignmentContext {
                assignment_id: &assignment.assignment_id,
                image_id: &image_id,
                task_id: &request.task_id,
                kind: AssignmentKind::Review,
            },
            &target,
            request.decision,
            request.comment,
            key,
        )
        .await?;
    Ok(Json(client_migration_result(result)))
}

async fn migration_assignment(
    repo: &labello_storage::DatasetRepository,
    image_id: &ImageId,
    assignment_id: &labello_domain::AssignmentId,
    actor: &Actor,
    required_role: DatasetRole,
) -> ApiResult<Assignment> {
    image_id.validate_path_segment()?;
    assignment_id.validate_path_segment()?;
    let metadata = repo.load_dataset_config().await?;
    ensure_dataset_role(&metadata, actor, required_role)?;
    repo.load_image_state(image_id)
        .await?
        .assignments
        .into_iter()
        .find(|assignment| assignment.assignment_id == *assignment_id)
        .ok_or_else(|| ApiError::BadRequest("assignment does not exist".to_string()))
}

fn migration_context<'a>(
    assignment: &'a Assignment,
    image_id: &'a ImageId,
) -> AssignmentContext<'a> {
    AssignmentContext {
        assignment_id: &assignment.assignment_id,
        image_id,
        task_id: &assignment.task_id,
        kind: assignment.kind.clone(),
    }
}

fn storage_expectation(
    expected: &labello_client::MigrationTargetExpectation,
) -> labello_storage::assignment::MigrationTargetExpectation {
    labello_storage::assignment::MigrationTargetExpectation {
        object_group_id: expected.object_group_id.clone(),
        expected_guide_annotation_version: expected.expected_guide_annotation_version,
        expected_guide_deleted: expected.expected_guide_deleted,
        expected_disposition_version: expected.expected_disposition_version,
        expected_skeleton_version: expected.expected_skeleton_version,
    }
}

fn client_migration_result(
    result: labello_storage::assignment::ManualMigrationCommandResult,
) -> ManualMigrationCommandResult {
    ManualMigrationCommandResult {
        image_state: result.image_state,
        cursor: Some(result.cursor),
        progress: labello_client::ManualMigrationProgress {
            expected: result.progress.expected,
            annotated: result.progress.annotated,
            excluded: result.progress.excluded,
            pending: result.progress.pending,
        },
        active_pass: result.active_pass,
        confirmation: result.confirmation,
        assignment: result.assignment,
        annotation_id: result.annotation_id,
    }
}

fn migration_idempotency_key(headers: &HeaderMap) -> ApiResult<&str> {
    let values = headers.get_all("idempotency-key");
    let mut values = values.iter();
    let key = values
        .next()
        .filter(|_| values.next().is_none())
        .ok_or_else(|| {
            ApiError::BadRequest("exactly one idempotency-key header is required".to_string())
        })?
        .to_str()
        .map_err(|_| ApiError::BadRequest("idempotency-key header is invalid".to_string()))?;
    if key.is_empty()
        || key.len() > 200
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b',' && byte != b';')
    {
        return Err(ApiError::BadRequest(
            "idempotency-key must be 1-200 visible ASCII characters".to_string(),
        ));
    }
    Ok(key)
}

pub(crate) async fn append_admin_repair_event(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    headers: HeaderMap,
    Json(request): Json<AppendEventRequest>,
) -> ApiResult<Json<labello_domain::EventLogEntry>> {
    image_id.validate_path_segment()?;
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::DataAdmin)?;
    let mut image_state = repo.load_image_state(&image_id).await?;
    validate_admin_repair_payload(&metadata, &image_id, &image_state, &request.payload)?;
    let expected_sequence = image_state.current_sequence;
    let timestamp = labello_domain::now();
    let payload = construct_annotation_mutation(&actor, &image_state, timestamp, request.payload)?;
    image_state
        .apply_event(&labello_domain::EventLogEntry::new(
            image_state.current_sequence + 1,
            image_id.clone(),
            actor.user_id.clone(),
            DatasetRole::DataAdmin,
            timestamp,
            payload.clone(),
        ))
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    Ok(Json(
        repo.append_admin_repair_payload(&actor.user_id, &image_id, expected_sequence, payload)
            .await?,
    ))
}

pub(crate) async fn rebuild_image(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    headers: HeaderMap,
) -> ApiResult<Json<labello_domain::ImageState>> {
    image_id.validate_path_segment()?;
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset_config().await?;
    ensure_any_dataset_role(&metadata, &actor)?;
    repo.load_image_record(&image_id).await?;
    Ok(Json(repo.rebuild_image_state(&image_id).await?))
}

pub(crate) async fn record_review(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    Query(assignment): Query<AssignmentActionRequest>,
    headers: HeaderMap,
    Json(review): Json<labello_domain::ReviewRecord>,
) -> ApiResult<Json<labello_domain::ImageState>> {
    image_id.validate_path_segment()?;
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    validate_assignment_request(&assignment, &image_id, AssignmentKind::Review)?;
    let image_state = repo
        .record_review_for_assignment(
            &actor.user_id,
            AssignmentContext {
                assignment_id: &assignment.assignment_id,
                image_id: &image_id,
                task_id: &assignment.task_id,
                kind: AssignmentKind::Review,
            },
            review,
        )
        .await?;
    tracing::debug!(
        event = "review.recorded",
        dataset_id = %dataset_id,
        user_id = %actor.user_id,
        assignment_id = %assignment.assignment_id,
        "review recorded"
    );
    Ok(Json(image_state))
}

pub(crate) async fn record_correction(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    Query(assignment): Query<AssignmentActionRequest>,
    headers: HeaderMap,
    Json(request): Json<CorrectionRequest>,
) -> ApiResult<Json<labello_domain::EventLogEntry>> {
    image_id.validate_path_segment()?;
    request.correction_id.validate_path_segment()?;
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset_config().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::Reviewer)?;
    validate_assignment_request(&assignment, &image_id, AssignmentKind::Review)?;
    let event = repo
        .correct_review_annotation(
            &actor.user_id,
            AssignmentContext {
                assignment_id: &assignment.assignment_id,
                image_id: &image_id,
                task_id: &assignment.task_id,
                kind: AssignmentKind::Review,
            },
            &request.correction_id,
            &request.annotation_id,
            request.expected_version,
            request.geometry,
            request.reason,
        )
        .await?;
    tracing::debug!(
        event = "correction.recorded",
        dataset_id = %dataset_id,
        user_id = %actor.user_id,
        assignment_id = %assignment.assignment_id,
        "review correction recorded"
    );
    Ok(Json(event))
}

pub(crate) async fn record_adjudication(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    Query(assignment): Query<AssignmentActionRequest>,
    headers: HeaderMap,
    Json(adjudication): Json<labello_domain::AdjudicationRecord>,
) -> ApiResult<Json<labello_domain::EventLogEntry>> {
    image_id.validate_path_segment()?;
    let actor = actor_from_headers(&state, &headers)?;
    if adjudication.adjudicator_user_id != actor.user_id {
        return Err(ApiError::Unauthorized(
            "cannot record adjudications for another user".to_string(),
        ));
    }
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::Adjudicator)?;
    validate_assignment_request(&assignment, &image_id, AssignmentKind::Adjudication)?;
    if adjudication.task_id != assignment.task_id {
        return Err(ApiError::BadRequest(
            "adjudication task does not match assignment task".to_string(),
        ));
    }
    let status = match adjudication.decision {
        AdjudicationDecision::AcceptAnnotation
        | AdjudicationDecision::MergeAnnotations
        | AdjudicationDecision::RejectAnnotation => TaskStatus::Completed,
        AdjudicationDecision::NeedsCorrection => TaskStatus::NeedsCorrection,
    };
    let timestamp = labello_domain::now();
    let (events, _) = repo
        .append_for_assignment(
            &actor.user_id,
            AssignmentContext {
                assignment_id: &assignment.assignment_id,
                image_id: &image_id,
                task_id: &assignment.task_id,
                kind: AssignmentKind::Adjudication,
            },
            vec![
                EventPayload::AdjudicationRecorded {
                    adjudication: adjudication.clone(),
                },
                EventPayload::TaskStateChanged {
                    task_state: TaskState {
                        task_id: adjudication.task_id,
                        outcome: (status == TaskStatus::Completed)
                            .then_some(TaskOutcome::Adjudicated),
                        status,
                        assigned_to: None,
                        completed_by: Some(actor.user_id.clone()),
                        completed_at: Some(timestamp),
                        updated_at: timestamp,
                    },
                },
            ],
            true,
        )
        .await?;
    tracing::debug!(
        event = "adjudication.recorded",
        dataset_id = %dataset_id,
        user_id = %actor.user_id,
        assignment_id = %assignment.assignment_id,
        "adjudication recorded"
    );
    Ok(Json(
        events
            .into_iter()
            .next()
            .expect("adjudication was appended"),
    ))
}

pub(crate) async fn offline_bundle(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    Query(query): Query<OfflineBundleRequest>,
    headers: HeaderMap,
) -> ApiResult<Json<labello_domain::OfflineBundle>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    Ok(Json(
        repo.create_offline_bundle(&actor.user_id, query.limit, query.include_image_bytes)
            .await?,
    ))
}

pub(crate) async fn offline_sync(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
    Json(value): Json<serde_json::Value>,
) -> ApiResult<Json<labello_domain::OfflineSyncResult>> {
    let request: OfflineSyncRequest =
        labello_domain::deserialize_current_artifact(&serde_json::to_vec(&value)?)
            .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    let actor = actor_from_headers(&state, &headers)?;
    if request.dataset_id != dataset_id {
        return Err(ApiError::BadRequest(
            "request datasetId does not match path".to_string(),
        ));
    }
    if request.user_id != actor.user_id {
        return Err(ApiError::Unauthorized(
            "offline sync userId must match the authenticated user".to_string(),
        ));
    }
    for fragment in &request.fragments {
        fragment.image_id.validate_path_segment()?;
    }
    let repo = state.repo(&dataset_id)?;
    let result = repo.sync_offline_events(request).await?;
    tracing::info!(
        event = "offline_sync.completed",
        dataset_id = %dataset_id,
        user_id = %actor.user_id,
        merged_events = result.merged_events,
        conflict_count = result.conflicts.len(),
        "offline synchronization completed"
    );
    Ok(Json(result))
}

pub(crate) async fn stats(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
) -> ApiResult<Json<labello_domain::DatasetStats>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset_config().await?;
    ensure_any_dataset_role(&metadata, &actor)?;
    Ok(Json(repo.dataset_stats().await?))
}

pub(crate) async fn get_keybindings(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
) -> ApiResult<Json<KeybindingSet>> {
    let actor = actor_from_headers(&state, &headers)?;
    actor.user_id.validate_path_segment()?;
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset_config().await?;
    ensure_any_dataset_role(&metadata, &actor)?;
    Ok(Json(repo.load_keybindings(&actor.user_id).await?))
}

pub(crate) async fn put_keybindings(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
    Json(mut bindings): Json<KeybindingSet>,
) -> ApiResult<Json<KeybindingSet>> {
    let actor = actor_from_headers(&state, &headers)?;
    if actor.user_id != bindings.user_id {
        return Err(ApiError::Unauthorized(
            "cannot edit another user's keybindings".to_string(),
        ));
    }
    actor.user_id.validate_path_segment()?;
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset_config().await?;
    ensure_any_dataset_role(&metadata, &actor)?;
    labello_domain::validate_schema_version(bindings.schema_version)
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    let complete = labello_domain::UserAction::ACTIVE
        .into_iter()
        .all(|action| bindings.bindings.contains_key(&action));
    if complete {
        bindings
            .validate()
            .map_err(labello_storage::StorageError::from)?;
    }
    bindings.normalize();
    bindings
        .validate()
        .map_err(labello_storage::StorageError::from)?;
    repo.save_keybindings(&bindings).await?;
    Ok(Json(bindings))
}

pub(crate) async fn prelabel_suggestions(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
    Json(request): Json<PrelabelSuggestionRequest>,
) -> ApiResult<Json<Vec<PrelabelSuggestion>>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset_config().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::Annotator)?;
    let config = metadata
        .prelabel_configs
        .iter()
        .find(|config| config.config_id == request.config_id)
        .ok_or_else(|| ApiError::NotFound("prelabel config".to_string()))?;
    if !config.available_to_annotators {
        return Err(ApiError::Unauthorized(
            "prelabel config is not available to annotators".to_string(),
        ));
    }
    let task = metadata
        .task(&request.task_id)
        .ok_or_else(|| ApiError::NotFound("task".to_string()))?;
    let Some(class_id) = task.class_ids.first().cloned() else {
        return Ok(Json(Vec::new()));
    };
    let geometry = match task.annotation_type {
        AnnotationType::BoundingBox => {
            AnnotationGeometry::BoundingBox(labello_domain::BoundingBox {
                x: 0.25,
                y: 0.25,
                width: 0.5,
                height: 0.5,
            })
        }
        AnnotationType::Skeleton => {
            AnnotationGeometry::Skeleton(labello_domain::SkeletonGeometry {
                keypoints: task
                    .skeleton
                    .as_ref()
                    .map(|s| &s.keypoints)
                    .into_iter()
                    .flatten()
                    .map(|spec| labello_domain::KeypointAnnotation {
                        name: spec.name.clone(),
                        state: labello_domain::KeypointState::Hidden,
                        point: Some(labello_domain::NormalizedPoint { x: 0.5, y: 0.5 }),
                    })
                    .collect(),
            })
        }
    };
    let suggestion = PrelabelSuggestion {
        suggestion_id: format!("pre_{}_{}", request.config_id, request.task_id),
        config_id: request.config_id,
        task_id: request.task_id,
        class_id,
        confidence: 0.9,
        geometry,
    };
    Ok(Json(if suggestion.passes(&config.output_processing) {
        vec![suggestion]
    } else {
        vec![]
    }))
}
