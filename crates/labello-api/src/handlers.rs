use std::collections::BTreeSet;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    routing::{get, post},
};
use labello_client::CreateDatasetRequest;
use labello_domain::{
    AnnotationSource, DatasetId, DatasetMetadata, DatasetRole, DatasetRoleAssignment, EventPayload,
    ImageId, PrelabelConfig, TaskDefinition,
};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::{
    ApiState,
    auth::{actor_from_headers, ensure_any_dataset_role, ensure_dataset_role},
    error::{ApiError, ApiResult},
};

mod oauth_routes;
mod workflow;

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/auth/github/login", get(oauth_routes::github_login))
        .route("/auth/github/callback", get(oauth_routes::github_callback))
        .route("/datasets", post(create_dataset))
        .route("/datasets/{dataset_id}", get(get_dataset))
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
    Json(request): Json<CreateDatasetRequest>,
) -> ApiResult<Json<DatasetMetadata>> {
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

async fn get_dataset(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
) -> ApiResult<Json<DatasetMetadata>> {
    let actor = actor_from_headers(&headers)?;
    let repo = state.repo(&dataset_id);
    let metadata = repo.load_dataset().await?;
    ensure_any_dataset_role(&metadata, &actor)?;
    Ok(Json(metadata))
}

async fn ingest_dataset(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
) -> ApiResult<Json<labello_client::IngestReport>> {
    let actor = actor_from_headers(&headers)?;
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
    let actor = actor_from_headers(&headers)?;
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
    let actor = actor_from_headers(&headers)?;
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
    let actor = actor_from_headers(&headers)?;
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
    let actor = actor_from_headers(&headers)?;
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
    if let EventPayload::AnnotationVersionCreated { annotation, .. } = payload {
        let record = metadata
            .images
            .get(image_id)
            .ok_or_else(|| ApiError::NotFound(format!("image {image_id}")))?;
        let task = metadata
            .task(&annotation.task_id)
            .ok_or_else(|| ApiError::BadRequest(format!("unknown task {}", annotation.task_id)))?;
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
    Ok(())
}
