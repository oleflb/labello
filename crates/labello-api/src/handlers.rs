use std::collections::BTreeSet;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    routing::{get, post},
};
use labello_client::{CreateDatasetRequest, DatasetSummary, UpdateDatasetConfigRequest};
use labello_domain::{
    Actor, AnnotationSource, DatasetId, DatasetMetadata, DatasetRole, DatasetRoleAssignment,
    EventPayload, ImageId, PrelabelConfig, TaskDefinition,
};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::{
    ApiState,
    auth::{
        actor_from_headers, ensure_any_dataset_role, ensure_bootstrap_admin, ensure_dataset_role,
        has_dataset_role,
    },
    error::{ApiError, ApiResult},
};

mod oauth_routes;
mod workflow;

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/auth/github/login", get(oauth_routes::github_login))
        .route("/auth/github/callback", get(oauth_routes::github_callback))
        .route("/datasets", get(list_datasets).post(create_dataset))
        .route("/datasets/{dataset_id}", get(get_dataset))
        .route(
            "/datasets/{dataset_id}/admin",
            get(get_admin_dataset).put(update_dataset_config),
        )
        .route("/datasets/{dataset_id}/ingest", post(ingest_dataset))
        .route(
            "/datasets/{dataset_id}/tasks",
            get(list_tasks).post(add_task),
        )
        .route(
            "/datasets/{dataset_id}/prelabels",
            get(list_prelabel_configs).post(add_prelabel_config),
        )
        .route(
            "/datasets/{dataset_id}/images/next",
            post(workflow::assign_next),
        )
        .route(
            "/datasets/{dataset_id}/images/{image_id}",
            get(workflow::get_image_state),
        )
        .route(
            "/datasets/{dataset_id}/images/{image_id}/file",
            get(workflow::get_image_file),
        )
        .route(
            "/datasets/{dataset_id}/images/{image_id}/events",
            post(workflow::append_event),
        )
        .route(
            "/datasets/{dataset_id}/images/{image_id}/rebuild",
            post(workflow::rebuild_image),
        )
        .route(
            "/datasets/{dataset_id}/images/{image_id}/reviews",
            post(workflow::record_review),
        )
        .route(
            "/datasets/{dataset_id}/images/{image_id}/corrections",
            post(workflow::record_correction),
        )
        .route(
            "/datasets/{dataset_id}/images/{image_id}/adjudications",
            post(workflow::record_adjudication),
        )
        .route(
            "/datasets/{dataset_id}/offline-bundle",
            get(workflow::offline_bundle),
        )
        .route(
            "/datasets/{dataset_id}/offline-sync",
            post(workflow::offline_sync),
        )
        .route("/datasets/{dataset_id}/stats", get(workflow::stats))
        .route(
            "/datasets/{dataset_id}/keybindings",
            get(workflow::get_keybindings).put(workflow::put_keybindings),
        )
        .route(
            "/datasets/{dataset_id}/prelabel-suggestions",
            post(workflow::prelabel_suggestions),
        )
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "service": "labello" }))
}

async fn create_dataset(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(request): Json<CreateDatasetRequest>,
) -> ApiResult<Json<DatasetMetadata>> {
    let actor = actor_from_headers(&state, &headers)?;
    ensure_bootstrap_admin(&state, &actor, "create datasets")?;
    if actor.user_id != request.admin_user_id && !state.is_bootstrap_admin(&request.admin_user_id) {
        return Err(ApiError::Unauthorized(
            "bootstrap admins can only create datasets for themselves or another bootstrap admin"
                .to_string(),
        ));
    }
    let repo = state.repo(&request.dataset_id);
    let mut metadata = DatasetMetadata::new(
        request.dataset_id.clone(),
        request.name,
        labello_domain::now(),
    );
    metadata.role_assignments.push(DatasetRoleAssignment {
        dataset_id: request.dataset_id,
        user_id: request.admin_user_id,
        roles: BTreeSet::from([
            DatasetRole::DataAdmin,
            DatasetRole::Annotator,
            DatasetRole::Reviewer,
            DatasetRole::Adjudicator,
        ]),
        assigned_at: labello_domain::now(),
        assigned_by: None,
    });
    repo.initialize(metadata.clone()).await?;
    Ok(Json(metadata))
}

async fn list_datasets(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<DatasetSummary>>> {
    let actor = actor_from_headers(&state, &headers)?;
    let mut summaries = Vec::new();
    tokio::fs::create_dir_all(state.datasets_root())
        .await
        .map_err(|source| labello_storage::StorageError::Io {
            path: state.datasets_root().to_path_buf(),
            source,
        })?;
    let mut entries = tokio::fs::read_dir(state.datasets_root())
        .await
        .map_err(|source| labello_storage::StorageError::Io {
            path: state.datasets_root().to_path_buf(),
            source,
        })?;
    while let Some(entry) =
        entries
            .next_entry()
            .await
            .map_err(|source| labello_storage::StorageError::Io {
                path: state.datasets_root().to_path_buf(),
                source,
            })?
    {
        let Ok(file_type) = entry.file_type().await else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let dataset_id = DatasetId::from(entry.file_name().to_string_lossy().to_string());
        let repo = state.repo(&dataset_id);
        let Ok(metadata) = repo.load_dataset().await else {
            continue;
        };
        let roles = actor_roles(&metadata, &actor);
        if roles.is_empty() && !state.is_bootstrap_admin(&actor.user_id) {
            continue;
        }
        summaries.push(DatasetSummary {
            dataset_id: metadata.dataset_id,
            name: metadata.name,
            roles,
            total_images: metadata.images.len(),
        });
    }
    Ok(Json(summaries))
}

async fn get_dataset(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
) -> ApiResult<Json<DatasetMetadata>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id);
    let metadata = repo.load_dataset().await?;
    ensure_any_dataset_role(&metadata, &actor)?;
    Ok(Json(sanitize_dataset(metadata, &actor)))
}

async fn get_admin_dataset(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
) -> ApiResult<Json<DatasetMetadata>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id);
    let metadata = repo.load_dataset().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::DataAdmin)?;
    Ok(Json(metadata))
}

async fn update_dataset_config(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
    Json(request): Json<UpdateDatasetConfigRequest>,
) -> ApiResult<Json<DatasetMetadata>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id);
    let mut metadata = repo.load_dataset().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::DataAdmin)?;
    validate_config_update(&metadata, &request, &actor)?;
    for root in &request.image_roots {
        repo.safe_relative_root(root)?;
    }
    metadata.name = request.name;
    metadata.image_roots = normalize_roots(request.image_roots);
    metadata.label_classes = request.label_classes;
    metadata.tasks = request.tasks;
    metadata.role_assignments = request.role_assignments;
    metadata.imbalance = request.imbalance;
    metadata.prelabel_configs = request.prelabel_configs;
    metadata.updated_at = labello_domain::now();
    repo.save_dataset(&metadata).await?;
    Ok(Json(metadata))
}

async fn ingest_dataset(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
) -> ApiResult<Json<labello_client::IngestReport>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id);
    let metadata = repo.load_dataset().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::DataAdmin)?;
    Ok(Json(storage_ingest_to_client(repo.ingest_images().await?)))
}

fn storage_ingest_to_client(report: labello_storage::IngestReport) -> labello_client::IngestReport {
    labello_client::IngestReport {
        discovered_files: report.discovered_files,
        new_images: report.new_images,
        duplicate_files: report
            .duplicate_files
            .into_iter()
            .map(|duplicate| labello_client::DuplicateImage {
                image_id: duplicate.image_id,
                canonical_path: duplicate.canonical_path,
                duplicate_path: duplicate.duplicate_path,
                blake3: duplicate.blake3,
            })
            .collect(),
        changed_paths: report
            .changed_paths
            .into_iter()
            .map(|changed| labello_client::ChangedPath {
                relative_path: changed.relative_path,
                previous_blake3: changed.previous_blake3,
                current_blake3: changed.current_blake3,
            })
            .collect(),
        unreadable_files: report.unreadable_files,
    }
}

async fn list_tasks(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<TaskDefinition>>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id);
    let metadata = repo.load_dataset().await?;
    ensure_any_dataset_role(&metadata, &actor)?;
    Ok(Json(metadata.tasks))
}

async fn add_task(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
    Json(task): Json<TaskDefinition>,
) -> ApiResult<Json<TaskDefinition>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id);
    let mut metadata = repo.load_dataset().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::DataAdmin)?;
    metadata
        .tasks
        .retain(|existing| existing.task_id != task.task_id);
    metadata.tasks.push(task.clone());
    metadata.updated_at = labello_domain::now();
    repo.save_dataset(&metadata).await?;
    Ok(Json(task))
}

async fn list_prelabel_configs(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<PrelabelConfig>>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id);
    let metadata = repo.load_dataset().await?;
    ensure_any_dataset_role(&metadata, &actor)?;
    Ok(Json(metadata.prelabel_configs))
}

async fn add_prelabel_config(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
    Json(config): Json<PrelabelConfig>,
) -> ApiResult<Json<PrelabelConfig>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id);
    let mut metadata = repo.load_dataset().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::DataAdmin)?;
    metadata
        .prelabel_configs
        .retain(|existing| existing.config_id != config.config_id);
    metadata.prelabel_configs.push(config.clone());
    metadata.updated_at = labello_domain::now();
    repo.save_dataset(&metadata).await?;
    Ok(Json(config))
}

pub(crate) fn validate_payload(
    metadata: &DatasetMetadata,
    image_id: &ImageId,
    payload: &EventPayload,
) -> ApiResult<()> {
    match payload {
        EventPayload::AnnotationVersionCreated { annotation, .. } => {
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
        | EventPayload::AdjudicationRecorded { .. }
        | EventPayload::AssignmentUpdated { .. } => {}
    }
    Ok(())
}

pub(crate) fn required_role_for_payload(
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

fn sanitize_dataset(mut metadata: DatasetMetadata, actor: &Actor) -> DatasetMetadata {
    if !has_dataset_role(&metadata, &actor.user_id, &DatasetRole::DataAdmin) {
        metadata.role_assignments.clear();
        metadata.image_roots.clear();
        metadata
            .prelabel_configs
            .retain(|config| config.available_to_annotators);
    }
    metadata
}

fn actor_roles(metadata: &DatasetMetadata, actor: &Actor) -> Vec<DatasetRole> {
    metadata
        .role_assignments
        .iter()
        .find(|assignment| {
            assignment.dataset_id == metadata.dataset_id && assignment.user_id == actor.user_id
        })
        .map(|assignment| assignment.roles.iter().cloned().collect())
        .unwrap_or_default()
}

fn validate_config_update(
    metadata: &DatasetMetadata,
    request: &UpdateDatasetConfigRequest,
    actor: &Actor,
) -> ApiResult<()> {
    if request.name.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "dataset name cannot be empty".to_string(),
        ));
    }
    if request
        .image_roots
        .iter()
        .all(|root| root.trim().is_empty())
    {
        return Err(ApiError::BadRequest(
            "at least one image root is required".to_string(),
        ));
    }
    let class_ids: BTreeSet<_> = request
        .label_classes
        .iter()
        .map(|class| class.class_id.clone())
        .collect();
    if class_ids.len() != request.label_classes.len() {
        return Err(ApiError::BadRequest("duplicate class ids".to_string()));
    }
    let task_ids: BTreeSet<_> = request
        .tasks
        .iter()
        .map(|task| task.task_id.clone())
        .collect();
    if task_ids.len() != request.tasks.len() {
        return Err(ApiError::BadRequest("duplicate task ids".to_string()));
    }
    for task in &request.tasks {
        for class_id in &task.class_ids {
            if !class_ids.contains(class_id) {
                return Err(ApiError::BadRequest(format!(
                    "task {} references unknown class {class_id}",
                    task.task_id
                )));
            }
        }
    }
    let has_admin = request.role_assignments.iter().any(|assignment| {
        assignment.dataset_id == metadata.dataset_id
            && assignment.roles.contains(&DatasetRole::DataAdmin)
    });
    if !has_admin {
        return Err(ApiError::BadRequest(
            "at least one data_admin role assignment is required".to_string(),
        ));
    }
    let actor_still_admin = request.role_assignments.iter().any(|assignment| {
        assignment.dataset_id == metadata.dataset_id
            && assignment.user_id == actor.user_id
            && assignment.roles.contains(&DatasetRole::DataAdmin)
    });
    if !actor_still_admin {
        return Err(ApiError::BadRequest(
            "cannot remove your own data_admin role through the API".to_string(),
        ));
    }
    Ok(())
}

fn normalize_roots(roots: Vec<String>) -> Vec<String> {
    let mut normalized: Vec<_> = roots
        .into_iter()
        .map(|root| root.trim().trim_matches('/').to_string())
        .filter(|root| !root.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if normalized.is_empty() {
        normalized.push("images".to_string());
    }
    normalized
}
