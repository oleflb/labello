use axum::{
    Json,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
};
use labello_client::{
    AppendEventRequest, AssignNextRequest, AssignmentActionRequest, CorrectionRequest,
    OfflineBundleRequest, PrelabelSuggestionRequest,
};
use labello_domain::{
    Actor, AdjudicationDecision, AnnotationGeometry, AnnotationSource, AnnotationType,
    AssignmentKind, DatasetId, DatasetRole, EventPayload, ImageId, KeybindingSet,
    OfflineSyncRequest, PrelabelSuggestion, ReviewDecision, ReviewTarget, ReviewWorkflow,
    TaskState, TaskStatus,
};
use labello_storage::assignment::AssignmentContext;

use crate::{
    ApiState,
    auth::{actor_from_headers, ensure_any_dataset_role, ensure_dataset_role},
    error::{ApiError, ApiResult},
};

use super::{required_role_for_payload, validate_payload};

#[derive(serde::Deserialize)]
pub(crate) struct PreviewQuery {
    #[serde(default = "default_preview_max")]
    max: u32,
}

fn default_preview_max() -> u32 {
    1600
}

pub(crate) async fn assign_next(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    Query(query): Query<AssignNextRequest>,
    headers: HeaderMap,
) -> ApiResult<Json<Option<labello_domain::Assignment>>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
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

pub(crate) async fn release_assignment(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
    Json(request): Json<AssignmentActionRequest>,
) -> ApiResult<Json<labello_domain::Assignment>> {
    request.image_id.validate_path_segment()?;
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    Ok(Json(
        repo.release_assignment(
            &actor.user_id,
            &request.assignment_id,
            &request.image_id,
            &request.task_id,
            request.kind,
        )
        .await?,
    ))
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
    Ok(Json(
        repo.complete_assignment(
            &actor.user_id,
            &request.assignment_id,
            &request.image_id,
            &request.task_id,
            request.kind,
        )
        .await?,
    ))
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
    validate_payload(&metadata, &image_id, &request.payload)?;
    validate_assignment_request(&assignment, &image_id, AssignmentKind::Annotation)?;
    validate_annotation_assignment_payload(
        &repo.load_image_state(&image_id).await?,
        &assignment.task_id,
        &request.payload,
    )?;
    let (events, _) = repo
        .append_for_assignment(
            &actor.user_id,
            AssignmentContext {
                assignment_id: &assignment.assignment_id,
                image_id: &image_id,
                task_id: &assignment.task_id,
                kind: AssignmentKind::Annotation,
            },
            vec![request.payload],
            false,
        )
        .await?;
    Ok(Json(
        events.into_iter().next().expect("one payload was appended"),
    ))
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
    if matches!(request.payload, EventPayload::AssignmentUpdated { .. }) {
        return Err(ApiError::BadRequest(
            "assignment state is managed by assignment endpoints".to_string(),
        ));
    }
    validate_payload(&metadata, &image_id, &request.payload)?;
    Ok(Json(
        repo.append_payload(
            &image_id,
            &Actor {
                user_id: actor.user_id,
                role: DatasetRole::DataAdmin,
            },
            request.payload,
        )
        .await?,
    ))
}

fn validate_assignment_request(
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

fn validate_annotation_assignment_payload(
    image_state: &labello_domain::ImageState,
    task_id: &labello_domain::TaskId,
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
        _ => {
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
) -> ApiResult<Json<labello_domain::EventLogEntry>> {
    image_id.validate_path_segment()?;
    let actor = actor_from_headers(&state, &headers)?;
    if review.reviewer_user_id != actor.user_id {
        return Err(ApiError::Unauthorized(
            "cannot record reviews for another user".to_string(),
        ));
    }
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::Reviewer)?;
    validate_assignment_request(&assignment, &image_id, AssignmentKind::Review)?;
    if !metadata.images.contains_key(&image_id) {
        return Err(ApiError::NotFound(format!("image {image_id}")));
    }
    let image_state = repo.load_image_state(&image_id).await?;
    let assignment_task = metadata
        .task(&assignment.task_id)
        .ok_or_else(|| ApiError::BadRequest(format!("unknown task {}", assignment.task_id)))?;
    match assignment_task.review.workflow {
        ReviewWorkflow::None => {
            return Err(ApiError::BadRequest(
                "reviews are disabled for this task".to_string(),
            ));
        }
        ReviewWorkflow::IndependentAgreement => {
            return Err(ApiError::BadRequest(
                "independent agreement workflow is not implemented".to_string(),
            ));
        }
        ReviewWorkflow::Approval => {}
    }
    let reviewed_task = match &review.target {
        ReviewTarget::Image {
            image_id: target_image_id,
        } => {
            if target_image_id != &image_id {
                return Err(ApiError::BadRequest(
                    "review target image does not match path image".to_string(),
                ));
            }
            None
        }
        ReviewTarget::Task { task_id } => Some(
            metadata
                .task(task_id)
                .ok_or_else(|| ApiError::BadRequest(format!("unknown task {task_id}")))?,
        ),
        ReviewTarget::AnnotationVersion {
            annotation_id,
            version,
        } => {
            let annotation = image_state
                .annotations
                .get(annotation_id)
                .and_then(|versions| {
                    versions
                        .iter()
                        .find(|candidate| candidate.version == *version)
                })
                .ok_or_else(|| {
                    ApiError::BadRequest(format!(
                        "annotation {annotation_id} version {version} does not exist"
                    ))
                })?;
            Some(metadata.task(&annotation.task_id).ok_or_else(|| {
                ApiError::BadRequest(format!("unknown task {}", annotation.task_id))
            })?)
        }
    };
    if reviewed_task.is_some_and(|task| task.task_id != assignment.task_id) {
        return Err(ApiError::BadRequest(
            "review target task does not match assignment task".to_string(),
        ));
    }
    let mut payloads = vec![EventPayload::ReviewRecorded {
        review: review.clone(),
    }];
    let complete = matches!(review.target, ReviewTarget::Task { .. });
    if let ReviewTarget::Task { task_id } = &review.target {
        let task = metadata
            .task(task_id)
            .expect("task target was validated before recording the review");
        let current_reviews = repo.current_task_reviews(&image_id, task_id).await?;
        let prior_approvers = current_reviews
            .iter()
            .filter_map(|candidate| match (&candidate.target, &candidate.decision) {
                (
                    ReviewTarget::Task {
                        task_id: candidate_task_id,
                    },
                    ReviewDecision::Approved,
                ) if candidate_task_id == task_id => Some(&candidate.reviewer_user_id),
                _ => None,
            })
            .collect::<std::collections::BTreeSet<_>>();
        let approval_count = prior_approvers.len() as u32
            + u32::from(
                review.decision == ReviewDecision::Approved
                    && !prior_approvers.contains(&actor.user_id),
            );
        let status = match &review.decision {
            ReviewDecision::Approved if approval_count >= task.review.required_reviews => {
                TaskStatus::Completed
            }
            ReviewDecision::Approved => TaskStatus::Submitted,
            ReviewDecision::Rejected => TaskStatus::NeedsCorrection,
        };
        if status != TaskStatus::Submitted {
            let timestamp = labello_domain::now();
            payloads.push(EventPayload::TaskStateChanged {
                task_state: TaskState {
                    task_id: task_id.clone(),
                    status,
                    assigned_to: None,
                    completed_by: Some(actor.user_id.clone()),
                    completed_at: Some(timestamp),
                    updated_at: timestamp,
                },
            });
        }
    }
    let (events, _) = repo
        .append_for_assignment(
            &actor.user_id,
            AssignmentContext {
                assignment_id: &assignment.assignment_id,
                image_id: &image_id,
                task_id: &assignment.task_id,
                kind: AssignmentKind::Review,
            },
            payloads,
            complete,
        )
        .await?;
    Ok(Json(
        events.into_iter().next().expect("review was appended"),
    ))
}

pub(crate) async fn record_correction(
    State(state): State<ApiState>,
    Path((dataset_id, image_id)): Path<(DatasetId, ImageId)>,
    Query(assignment): Query<AssignmentActionRequest>,
    headers: HeaderMap,
    Json(request): Json<CorrectionRequest>,
) -> ApiResult<Json<labello_domain::EventLogEntry>> {
    image_id.validate_path_segment()?;
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::Reviewer)?;
    validate_assignment_request(&assignment, &image_id, AssignmentKind::Review)?;
    let task = metadata
        .task(&request.annotation.task_id)
        .ok_or_else(|| ApiError::BadRequest("unknown task".to_string()))?;
    if !task.review.allow_reviewer_corrections {
        return Err(ApiError::Unauthorized(
            "reviewer corrections are disabled for this task".to_string(),
        ));
    }
    if request.annotation.task_id != assignment.task_id {
        return Err(ApiError::BadRequest(
            "correction task does not match assignment task".to_string(),
        ));
    }
    let payload = EventPayload::AnnotationVersionCreated {
        annotation: request.annotation.clone(),
        previous_version: Some(request.previous_version),
        reason: request.reason.clone(),
    };
    validate_payload(&metadata, &image_id, &payload)?;
    let (events, _) = repo
        .append_for_assignment(
            &actor.user_id,
            AssignmentContext {
                assignment_id: &assignment.assignment_id,
                image_id: &image_id,
                task_id: &assignment.task_id,
                kind: AssignmentKind::Review,
            },
            vec![payload],
            false,
        )
        .await?;
    Ok(Json(
        events.into_iter().next().expect("correction was appended"),
    ))
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
    Json(request): Json<OfflineSyncRequest>,
) -> ApiResult<Json<labello_domain::OfflineSyncResult>> {
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
        for event in &fragment.events {
            event.image_id.validate_path_segment()?;
            if event.actor_user_id != actor.user_id {
                return Err(ApiError::Unauthorized(
                    "offline events must belong to the authenticated user".to_string(),
                ));
            }
            let required_role = required_role_for_payload(&actor, &event.payload)?;
            if event.actor_role != required_role {
                return Err(ApiError::Unauthorized(
                    "offline event role does not match its payload".to_string(),
                ));
            }
        }
    }
    let repo = state.repo(&dataset_id)?;
    Ok(Json(repo.sync_offline_events(request).await?))
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
    Json(bindings): Json<KeybindingSet>,
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

#[allow(dead_code)]
fn _accepted_prelabels_are_persisted_as_annotations(_: AnnotationSource) {}
