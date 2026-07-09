use axum::{
    Json,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
};
use labello_client::{
    AppendEventRequest, AssignNextRequest, CorrectionRequest, OfflineBundleRequest,
    PrelabelSuggestionRequest,
};
use labello_domain::{
    Actor, AdjudicationDecision, AnnotationGeometry, AnnotationSource, AnnotationType, DatasetId,
    DatasetRole, EventPayload, ImageId, KeybindingSet, OfflineSyncRequest, PrelabelSuggestion,
    ReviewDecision, ReviewTarget, TaskState, TaskStatus,
};

use crate::{
    ApiState,
    auth::{actor_from_headers, ensure_any_dataset_role, ensure_dataset_role},
    error::{ApiError, ApiResult},
};

use super::{required_role_for_payload, validate_payload};

pub(crate) async fn assign_next(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    Query(query): Query<AssignNextRequest>,
    headers: HeaderMap,
) -> ApiResult<Json<Option<labello_domain::Assignment>>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id);
    Ok(Json(
        repo.assign_next_image(
            &actor.user_id,
            &query.task_id,
            query
                .kind
                .unwrap_or(labello_domain::AssignmentKind::Annotation),
        )
        .await?,
    ))
}

pub(crate) async fn get_image_state(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    headers: HeaderMap,
) -> ApiResult<Json<labello_domain::ImageState>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id);
    let metadata = repo.load_dataset().await?;
    ensure_any_dataset_role(&metadata, &actor)?;
    Ok(Json(repo.load_image_state(&image_id).await?))
}

pub(crate) async fn get_image_file(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id);
    let metadata = repo.load_dataset().await?;
    ensure_any_dataset_role(&metadata, &actor)?;
    let record = metadata
        .images
        .get(&image_id)
        .ok_or_else(|| ApiError::NotFound(format!("image {image_id}")))?;
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

pub(crate) async fn append_event(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    headers: HeaderMap,
    Json(request): Json<AppendEventRequest>,
) -> ApiResult<Json<labello_domain::EventLogEntry>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id);
    let metadata = repo.load_dataset().await?;
    let required_role = required_role_for_payload(&actor, &request.payload)?;
    ensure_dataset_role(&metadata, &actor, required_role.clone())?;
    validate_payload(&metadata, &image_id, &request.payload)?;
    let event_actor = Actor {
        user_id: actor.user_id,
        role: required_role,
    };
    Ok(Json(
        repo.append_payload(&image_id, &event_actor, request.payload)
            .await?,
    ))
}

pub(crate) async fn rebuild_image(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    headers: HeaderMap,
) -> ApiResult<Json<labello_domain::ImageState>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id);
    let metadata = repo.load_dataset().await?;
    ensure_any_dataset_role(&metadata, &actor)?;
    Ok(Json(repo.rebuild_image_state(&image_id).await?))
}

pub(crate) async fn record_review(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    headers: HeaderMap,
    Json(review): Json<labello_domain::ReviewRecord>,
) -> ApiResult<Json<labello_domain::EventLogEntry>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id);
    let metadata = repo.load_dataset().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::Reviewer)?;
    let event_actor = Actor {
        user_id: actor.user_id.clone(),
        role: DatasetRole::Reviewer,
    };
    let event = repo
        .append_payload(
            &image_id,
            &event_actor,
            EventPayload::ReviewRecorded {
                review: review.clone(),
            },
        )
        .await?;
    if let ReviewTarget::Task { task_id } = review.target {
        let status = match review.decision {
            ReviewDecision::Approved => TaskStatus::Completed,
            ReviewDecision::Rejected => TaskStatus::NeedsCorrection,
        };
        let timestamp = labello_domain::now();
        repo.append_payload(
            &image_id,
            &event_actor,
            EventPayload::TaskStateChanged {
                task_state: TaskState {
                    task_id,
                    status,
                    assigned_to: None,
                    completed_by: Some(event_actor.user_id.clone()),
                    completed_at: Some(timestamp),
                    updated_at: timestamp,
                },
            },
        )
        .await?;
    }
    Ok(Json(event))
}

pub(crate) async fn record_correction(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    headers: HeaderMap,
    Json(request): Json<CorrectionRequest>,
) -> ApiResult<Json<labello_domain::EventLogEntry>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id);
    let metadata = repo.load_dataset().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::Reviewer)?;
    let event_actor = Actor {
        user_id: actor.user_id.clone(),
        role: DatasetRole::Reviewer,
    };
    let task = metadata
        .task(&request.annotation.task_id)
        .ok_or_else(|| ApiError::BadRequest("unknown task".to_string()))?;
    if !task.review.allow_reviewer_corrections {
        return Err(ApiError::Unauthorized(
            "reviewer corrections are disabled for this task".to_string(),
        ));
    }
    let payload = EventPayload::AnnotationVersionCreated {
        annotation: request.annotation.clone(),
        previous_version: Some(request.previous_version),
        reason: request.reason.clone(),
    };
    validate_payload(&metadata, &image_id, &payload)?;
    Ok(Json(
        repo.append_payload(&image_id, &event_actor, payload)
            .await?,
    ))
}

pub(crate) async fn record_adjudication(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    headers: HeaderMap,
    Json(adjudication): Json<labello_domain::AdjudicationRecord>,
) -> ApiResult<Json<labello_domain::EventLogEntry>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id);
    let metadata = repo.load_dataset().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::Adjudicator)?;
    let event_actor = Actor {
        user_id: actor.user_id.clone(),
        role: DatasetRole::Adjudicator,
    };
    let event = repo
        .append_payload(
            &image_id,
            &event_actor,
            EventPayload::AdjudicationRecorded {
                adjudication: adjudication.clone(),
            },
        )
        .await?;
    let status = match adjudication.decision {
        AdjudicationDecision::AcceptAnnotation
        | AdjudicationDecision::MergeAnnotations
        | AdjudicationDecision::RejectAnnotation => TaskStatus::Completed,
        AdjudicationDecision::NeedsCorrection => TaskStatus::NeedsCorrection,
    };
    let timestamp = labello_domain::now();
    repo.append_payload(
        &image_id,
        &event_actor,
        EventPayload::TaskStateChanged {
            task_state: TaskState {
                task_id: adjudication.task_id,
                status,
                assigned_to: None,
                completed_by: Some(event_actor.user_id.clone()),
                completed_at: Some(timestamp),
                updated_at: timestamp,
            },
        },
    )
    .await?;
    Ok(Json(event))
}

pub(crate) async fn offline_bundle(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    Query(query): Query<OfflineBundleRequest>,
    headers: HeaderMap,
) -> ApiResult<Json<labello_domain::OfflineBundle>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id);
    Ok(Json(
        repo.create_offline_bundle(&actor.user_id, query.limit, query.include_image_bytes)
            .await?,
    ))
}

pub(crate) async fn offline_sync(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    Json(request): Json<OfflineSyncRequest>,
) -> ApiResult<Json<labello_domain::OfflineSyncResult>> {
    if request.dataset_id != dataset_id {
        return Err(ApiError::BadRequest(
            "request datasetId does not match path".to_string(),
        ));
    }
    let repo = state.repo(&dataset_id);
    Ok(Json(repo.sync_offline_events(request).await?))
}

pub(crate) async fn stats(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
) -> ApiResult<Json<labello_domain::DatasetStats>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id);
    let metadata = repo.load_dataset().await?;
    ensure_any_dataset_role(&metadata, &actor)?;
    Ok(Json(repo.dataset_stats().await?))
}

pub(crate) async fn get_keybindings(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
) -> ApiResult<Json<KeybindingSet>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id);
    let metadata = repo.load_dataset().await?;
    ensure_any_dataset_role(&metadata, &actor)?;
    Ok(Json(repo.load_keybindings(&actor.user_id).await?))
}

pub(crate) async fn put_keybindings(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
    Json(bindings): Json<KeybindingSet>,
) -> ApiResult<Json<KeybindingSet>> {
    let actor = actor_from_headers(&state, &headers)?;
    if actor.user_id != bindings.user_id {
        return Err(ApiError::Unauthorized(
            "cannot edit another user's keybindings".to_string(),
        ));
    }
    let repo = state.repo(&dataset_id);
    let metadata = repo.load_dataset().await?;
    ensure_any_dataset_role(&metadata, &actor)?;
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
    let repo = state.repo(&dataset_id);
    let metadata = repo.load_dataset().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::Annotator)?;
    let config = metadata
        .prelabel_configs
        .iter()
        .find(|config| config.config_id == request.config_id)
        .ok_or_else(|| ApiError::NotFound("prelabel config".to_string()))?;
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

#[allow(dead_code)]
fn _accepted_prelabels_are_persisted_as_annotations(_: AnnotationSource) {}
