use std::{collections::BTreeSet, time::Duration};

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, MatchedPath, Multipart, Path, Query, State},
    http::{HeaderMap, HeaderValue, Method, Request, Response, StatusCode, header},
    response::IntoResponse,
    routing::{get, post, put},
};
use labello_client::{
    AuthOptions, CreateDatasetRequest, DatasetSummary, DatasetUser, ImageExplorerQuery, IngestJob,
    IngestJobStatus, SessionInfo, SetDatasetRolesRequest, UpdateDatasetConfigRequest,
};
use labello_domain::{
    Actor, AnnotationSource, DatasetId, DatasetMetadata, DatasetRole, DatasetRoleAssignment,
    EventPayload, ImageExplorerItem, ImageExplorerPage, ImageId, PrelabelConfig, ReviewWorkflow,
    TaskDefinition, TaskStatus,
};
use tower::ServiceBuilder;
use tower_http::{
    cors::CorsLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, RequestId, SetRequestIdLayer},
    trace::TraceLayer,
};
use tracing::Span;

use crate::{
    ApiState,
    auth::{
        actor_from_headers, current_account, ensure_any_dataset_role, ensure_bootstrap_admin,
        ensure_dataset_role, has_dataset_role, session_token,
    },
    error::{ApiError, ApiResult},
};

mod oauth_routes;
mod workflow;

const MAX_INGEST_REPORT_DETAILS: usize = 100;

pub fn router(state: ApiState) -> Router {
    let browser_origins = state.browser_origins().to_vec();
    let cors = if browser_origins.is_empty() {
        None
    } else {
        let origins = browser_origins
            .iter()
            .map(|origin| {
                HeaderValue::from_str(origin)
                    .expect("browser origins are validated when API state is configured")
            })
            .collect::<Vec<_>>();
        Some(
            CorsLayer::new()
                .allow_credentials(true)
                .allow_origin(origins)
                .allow_methods([Method::GET, Method::POST, Method::PUT, Method::OPTIONS])
                .allow_headers([header::CONTENT_TYPE])
                .expose_headers([
                    header::HeaderName::from_static("x-image-width"),
                    header::HeaderName::from_static("x-image-height"),
                    header::HeaderName::from_static("x-request-id"),
                ]),
        )
    };
    let trace = TraceLayer::new_for_http()
        .make_span_with(|request: &Request<Body>| {
            let route = request
                .extensions()
                .get::<MatchedPath>()
                .map(MatchedPath::as_str)
                .unwrap_or("<unmatched>");
            let request_id = request
                .extensions()
                .get::<RequestId>()
                .and_then(|request_id| request_id.header_value().to_str().ok())
                .unwrap_or("<missing>");
            tracing::info_span!(
                "http.request",
                request_id,
                method = %request.method(),
                route
            )
        })
        .on_request(())
        .on_response(
            |response: &Response<Body>, latency: Duration, _span: &Span| {
                tracing::info!(
                    event = "http.request.completed",
                    status = response.status().as_u16(),
                    latency_ms = latency.as_millis() as u64,
                    "request completed"
                );
            },
        )
        .on_body_chunk(())
        .on_eos(())
        .on_failure(());
    let middleware = ServiceBuilder::new()
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .layer(trace)
        .layer(PropagateRequestIdLayer::x_request_id())
        .option_layer(cors);
    let app = Router::new()
        .route("/health", get(health))
        .route("/me", get(me))
        .route("/logout", post(logout))
        .route("/auth/options", get(auth_options))
        .route("/auth/local-admin", post(local_admin_login))
        .route("/auth/github/login", get(oauth_routes::github_login))
        .route("/auth/github/callback", get(oauth_routes::github_callback))
        .route("/datasets", get(list_datasets).post(create_dataset))
        .route("/datasets/{dataset_id}", get(get_dataset))
        .route("/datasets/{dataset_id}/users", get(list_dataset_users))
        .route("/datasets/{dataset_id}/roles", put(set_dataset_roles))
        .route(
            "/datasets/{dataset_id}/admin",
            get(get_admin_dataset).put(update_dataset_config),
        )
        .route("/datasets/{dataset_id}/ingest", post(ingest_dataset))
        .route("/datasets/{dataset_id}/ingest-jobs", post(start_ingest_job))
        .route(
            "/datasets/{dataset_id}/ingest-jobs/{job_id}",
            get(get_ingest_job),
        )
        .route("/datasets/{dataset_id}/uploads", post(upload_images))
        .route(
            "/datasets/{dataset_id}/snapshots",
            get(list_snapshots).post(create_snapshot),
        )
        .route(
            "/datasets/{dataset_id}/snapshots/{snapshot_id}/files/{*file_path}",
            get(download_snapshot_file),
        )
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
        .route("/datasets/{dataset_id}/images", get(list_images))
        .route(
            "/datasets/{dataset_id}/assignments/release",
            post(workflow::release_assignment),
        )
        .route(
            "/datasets/{dataset_id}/assignments/complete",
            post(workflow::complete_assignment),
        )
        .route(
            "/datasets/{dataset_id}/images/{image_id}",
            get(workflow::get_image_state),
        )
        .route(
            "/datasets/{dataset_id}/images/{image_id}/record",
            get(workflow::get_image_record),
        )
        .route(
            "/datasets/{dataset_id}/images/{image_id}/file",
            get(workflow::get_image_file),
        )
        .route(
            "/datasets/{dataset_id}/images/{image_id}/preview",
            get(workflow::get_image_preview),
        )
        .route(
            "/datasets/{dataset_id}/images/{image_id}/events",
            post(workflow::append_event),
        )
        .route(
            "/datasets/{dataset_id}/images/{image_id}/annotation-batch",
            post(workflow::apply_annotation_batch),
        )
        .route(
            "/datasets/{dataset_id}/images/{image_id}/admin/events",
            post(workflow::append_admin_repair_event),
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
        .layer(DefaultBodyLimit::max(128 * 1024 * 1024))
        .with_state(state);
    app.layer(middleware)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "service": "labello" }))
}

async fn auth_options(State(state): State<ApiState>) -> Json<AuthOptions> {
    Json(AuthOptions {
        github_oauth: state.github_oauth.is_some(),
        local_admin_login: state.local_admin_login_enabled(),
    })
}

async fn local_admin_login(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let user_id = state
        .local_admin_user_id()
        .cloned()
        .ok_or_else(|| ApiError::NotFound("local admin login".to_string()))?;
    require_configured_origin(&state, &headers)?;
    let timestamp = labello_domain::now();
    let account = if let Some(account) = state.server_store.user(&user_id)? {
        account
    } else {
        state
            .server_store
            .upsert_user(labello_domain::UserAccount {
                user_id: user_id.clone(),
                display_name: user_id.to_string(),
                github_user_id: None,
                github_login: None,
                created_at: timestamp,
                updated_at: timestamp,
            })?
    };
    let token = state.create_session(user_id)?;
    let cookie = crate::session::session_cookie(&token, state.session_cookie_secure());
    tracing::info!(
        event = "auth.local_admin.completed",
        user_id = %account.user_id,
        "local administrator login completed"
    );
    Ok((
        [(
            header::SET_COOKIE,
            HeaderValue::from_str(&cookie)
                .map_err(|error| ApiError::Internal(error.to_string()))?,
        )],
        Json(SessionInfo {
            can_create_datasets: state.is_bootstrap_admin(&account.user_id),
            account,
        }),
    ))
}

fn require_configured_origin(state: &ApiState, headers: &HeaderMap) -> ApiResult<()> {
    let mut origins = headers.get_all(header::ORIGIN).iter();
    let Some(origin) = origins.next() else {
        return Ok(());
    };
    if origins.next().is_some()
        || origin.to_str().map_or(true, |origin| {
            !state
                .browser_origins()
                .iter()
                .any(|allowed| allowed == origin)
        })
    {
        return Err(ApiError::Unauthorized("origin is not allowed".to_string()));
    }
    Ok(())
}

async fn me(State(state): State<ApiState>, headers: HeaderMap) -> ApiResult<Json<SessionInfo>> {
    let account = current_account(&state, &headers)?;
    tracing::info!(
        event = "auth.completed",
        auth_mode = "session",
        user_id = %account.user_id,
        "authentication completed"
    );
    Ok(Json(SessionInfo {
        can_create_datasets: state.is_bootstrap_admin(&account.user_id),
        account,
    }))
}

async fn logout(State(state): State<ApiState>, headers: HeaderMap) -> ApiResult<impl IntoResponse> {
    if let Some(token) = session_token(&headers) {
        state.server_store.delete_session(&token)?;
    }
    tracing::info!(event = "auth.session.deleted", "session deleted");
    let cookie = crate::session::expired_session_cookie(state.session_cookie_secure());
    Ok((
        [(
            header::SET_COOKIE,
            HeaderValue::from_str(&cookie)
                .map_err(|error| ApiError::Internal(error.to_string()))?,
        )],
        StatusCode::NO_CONTENT,
    ))
}

async fn list_dataset_users(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<DatasetUser>>> {
    let actor = actor_from_headers(&state, &headers)?;
    let metadata = state.repo(&dataset_id)?.load_dataset_config().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::DataAdmin)?;
    let mut users = state
        .server_store
        .users()?
        .into_iter()
        .map(|account| (account.user_id.clone(), account))
        .collect::<std::collections::BTreeMap<_, _>>();
    for assignment in &metadata.role_assignments {
        users
            .entry(assignment.user_id.clone())
            .or_insert_with(|| labello_domain::UserAccount {
                user_id: assignment.user_id.clone(),
                display_name: assignment.user_id.to_string(),
                github_user_id: None,
                github_login: None,
                created_at: assignment.assigned_at,
                updated_at: assignment.assigned_at,
            });
    }
    Ok(Json(
        users
            .into_values()
            .map(|account| {
                let roles = metadata
                    .role_assignments
                    .iter()
                    .find(|assignment| assignment.user_id == account.user_id)
                    .map(|assignment| assignment.roles.iter().cloned().collect())
                    .unwrap_or_default();
                DatasetUser { account, roles }
            })
            .collect(),
    ))
}

async fn set_dataset_roles(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
    Json(request): Json<SetDatasetRolesRequest>,
) -> ApiResult<Json<DatasetUser>> {
    let actor = actor_from_headers(&state, &headers)?;
    request.user_id.validate_path_segment()?;
    let repo = state.repo(&dataset_id)?;
    let mut metadata = repo.load_dataset_config().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::DataAdmin)?;
    let account = state
        .server_store
        .user(&request.user_id)?
        .or_else(|| {
            metadata
                .role_assignments
                .iter()
                .find(|assignment| assignment.user_id == request.user_id)
                .map(|assignment| labello_domain::UserAccount {
                    user_id: assignment.user_id.clone(),
                    display_name: assignment.user_id.to_string(),
                    github_user_id: None,
                    github_login: None,
                    created_at: assignment.assigned_at,
                    updated_at: assignment.assigned_at,
                })
        })
        .ok_or_else(|| {
            ApiError::BadRequest(format!(
                "user {} has not logged in and is not in the user directory",
                request.user_id
            ))
        })?;
    let roles: BTreeSet<_> = request.roles.iter().cloned().collect();
    if roles.len() != request.roles.len() {
        return Err(ApiError::BadRequest("duplicate dataset roles".to_string()));
    }
    if request.user_id == actor.user_id && !roles.contains(&DatasetRole::DataAdmin) {
        return Err(ApiError::BadRequest(
            "cannot remove your own data_admin role through the API".to_string(),
        ));
    }
    metadata
        .role_assignments
        .retain(|assignment| assignment.user_id != request.user_id);
    if !roles.is_empty() {
        metadata.role_assignments.push(DatasetRoleAssignment {
            dataset_id: dataset_id.clone(),
            user_id: request.user_id.clone(),
            roles: roles.clone(),
            assigned_at: labello_domain::now(),
            assigned_by: Some(actor.user_id.clone()),
        });
    }
    if !metadata
        .role_assignments
        .iter()
        .any(|assignment| assignment.roles.contains(&DatasetRole::DataAdmin))
    {
        return Err(ApiError::BadRequest(
            "at least one data_admin role assignment is required".to_string(),
        ));
    }
    metadata.updated_at = labello_domain::now();
    repo.save_dataset(&metadata).await?;
    tracing::info!(
        event = "dataset.roles.updated",
        dataset_id = %dataset_id,
        actor_user_id = %actor.user_id,
        target_user_id = %request.user_id,
        role_count = roles.len(),
        "dataset roles updated"
    );
    Ok(Json(DatasetUser {
        account,
        roles: roles.into_iter().collect(),
    }))
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
    let repo = state.repo(&request.dataset_id)?;
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
    tracing::info!(
        event = "dataset.created",
        dataset_id = %metadata.dataset_id,
        actor_user_id = %actor.user_id,
        "dataset created"
    );
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
        let file_type = match entry.file_type().await {
            Ok(file_type) => file_type,
            Err(error) => {
                tracing::warn!(
                    event = "dataset.entry.skipped",
                    error_kind = %error.kind(),
                    "could not inspect dataset entry"
                );
                continue;
            }
        };
        if !file_type.is_dir() {
            continue;
        }
        let dataset_id = DatasetId::from(entry.file_name().to_string_lossy().to_string());
        let repo = match state.repo(&dataset_id) {
            Ok(repo) => repo,
            Err(_) => {
                tracing::warn!(
                    event = "dataset.entry.skipped",
                    dataset_id = %dataset_id,
                    error_kind = "invalid_id",
                    "invalid dataset directory ignored"
                );
                continue;
            }
        };
        let metadata = match repo.load_dataset_config().await {
            Ok(metadata) => metadata,
            Err(error) => {
                tracing::warn!(
                    event = "dataset.entry.skipped",
                    dataset_id = %dataset_id,
                    error_kind = error.kind(),
                    diagnostic = error.safe_diagnostic().as_deref().unwrap_or("redacted"),
                    "unreadable dataset ignored"
                );
                continue;
            }
        };
        let roles = actor_roles(&metadata, &actor);
        if roles.is_empty() && !state.is_bootstrap_admin(&actor.user_id) {
            continue;
        }
        let total_images = match repo.image_count().await {
            Ok(count) => count,
            Err(error) => {
                tracing::warn!(
                    event = "dataset.image_count.failed",
                    dataset_id = %dataset_id,
                    error_kind = error.kind(),
                    diagnostic = error.safe_diagnostic().as_deref().unwrap_or("redacted"),
                    "could not count dataset images"
                );
                0
            }
        };
        summaries.push(DatasetSummary {
            dataset_id: metadata.dataset_id,
            name: metadata.name,
            roles,
            total_images,
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
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset_config().await?;
    ensure_any_dataset_role(&metadata, &actor)?;
    Ok(Json(sanitize_dataset(metadata, &actor)))
}

async fn get_admin_dataset(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
) -> ApiResult<Json<DatasetMetadata>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset_config().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::DataAdmin)?;
    Ok(Json(config_response(metadata)))
}

async fn update_dataset_config(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
    Json(request): Json<UpdateDatasetConfigRequest>,
) -> ApiResult<Json<DatasetMetadata>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let mut metadata = repo.load_dataset_config().await?;
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
    tracing::info!(
        event = "dataset.configuration.updated",
        dataset_id = %dataset_id,
        actor_user_id = %actor.user_id,
        task_count = metadata.tasks.len(),
        class_count = metadata.label_classes.len(),
        image_root_count = metadata.image_roots.len(),
        "dataset configuration updated"
    );
    Ok(Json(config_response(metadata)))
}

async fn ingest_dataset(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
) -> ApiResult<Json<labello_client::IngestReport>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset_config().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::DataAdmin)?;
    tracing::info!(
        event = "ingest.started",
        dataset_id = %dataset_id,
        actor_user_id = %actor.user_id,
        mode = "synchronous",
        "dataset ingest started"
    );
    let started = std::time::Instant::now();
    let report = storage_ingest_to_client(repo.ingest_images().await?);
    tracing::info!(
        event = "ingest.completed",
        dataset_id = %dataset_id,
        mode = "synchronous",
        latency_ms = started.elapsed().as_millis() as u64,
        discovered_files = report.discovered_files,
        new_images = report.new_images,
        duplicate_files = report.duplicate_files.len(),
        unreadable_files = report.unreadable_files.len(),
        "dataset ingest completed"
    );
    Ok(Json(report))
}

async fn start_ingest_job(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
) -> ApiResult<Json<IngestJob>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset_config().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::DataAdmin)?;
    let job = IngestJob {
        job_id: uuid::Uuid::new_v4().to_string(),
        dataset_id: dataset_id.clone(),
        status: IngestJobStatus::Running,
        report: None,
        error: None,
    };
    state.put_ingest_job(job.clone()).await;
    tracing::info!(
        event = "ingest.started",
        dataset_id = %dataset_id,
        job_id = %job.job_id,
        actor_user_id = %actor.user_id,
        mode = "background",
        "dataset ingest started"
    );

    let state_for_job = state.clone();
    let job_for_task = job.clone();
    let repo_for_job = repo.clone();
    let started = std::time::Instant::now();
    let ingest_task = tokio::spawn(async move { repo_for_job.ingest_images().await });
    tokio::spawn(async move {
        let mut finished = job_for_task;
        match ingest_task.await {
            Ok(Ok(report)) => {
                finished.status = IngestJobStatus::Completed;
                let report = storage_ingest_to_client(report);
                tracing::info!(
                    event = "ingest.completed",
                    dataset_id = %finished.dataset_id,
                    job_id = %finished.job_id,
                    mode = "background",
                    latency_ms = started.elapsed().as_millis() as u64,
                    discovered_files = report.discovered_files,
                    new_images = report.new_images,
                    duplicate_files = report.duplicate_files.len(),
                    unreadable_files = report.unreadable_files.len(),
                    "dataset ingest completed"
                );
                finished.report = Some(report);
            }
            Ok(Err(error)) => {
                finished.status = IngestJobStatus::Failed;
                finished.error = Some("ingest failed".to_string());
                tracing::error!(
                    event = "ingest.failed",
                    dataset_id = %finished.dataset_id,
                    job_id = %finished.job_id,
                    error_kind = error.kind(),
                    diagnostic = error.safe_diagnostic().as_deref().unwrap_or("redacted"),
                    latency_ms = started.elapsed().as_millis() as u64,
                    "dataset ingest failed"
                );
            }
            Err(error) => {
                finished.status = IngestJobStatus::Failed;
                finished.error = Some("ingest task failed".to_string());
                tracing::error!(
                    event = "ingest.failed",
                    dataset_id = %finished.dataset_id,
                    job_id = %finished.job_id,
                    error_kind = "background_task",
                    cancelled = error.is_cancelled(),
                    panic = error.is_panic(),
                    latency_ms = started.elapsed().as_millis() as u64,
                    "dataset ingest task failed"
                );
            }
        }
        state_for_job.put_ingest_job(finished).await;
    });

    Ok(Json(job))
}

async fn get_ingest_job(
    State(state): State<ApiState>,
    Path((dataset_id, job_id)): Path<(DatasetId, String)>,
    headers: HeaderMap,
) -> ApiResult<Json<IngestJob>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset_config().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::DataAdmin)?;
    let job = state
        .get_ingest_job(&job_id)
        .await
        .filter(|job| job.dataset_id == dataset_id)
        .ok_or_else(|| ApiError::NotFound(format!("ingest job {job_id}")))?;
    Ok(Json(job))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadQuery {
    root: String,
    #[serde(default = "default_upload_ingest")]
    ingest: bool,
}

fn default_upload_ingest() -> bool {
    true
}

async fn upload_images(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    Query(query): Query<UploadQuery>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> ApiResult<Json<labello_client::IngestReport>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let mut metadata = repo.load_dataset_config().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::DataAdmin)?;
    let root = normalize_upload_root(&query.root)?;
    repo.safe_relative_root(&root)?;
    let mut written_files = 0usize;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| ApiError::BadRequest(error.to_string()))?
    {
        if field.name() != Some("files") {
            continue;
        }
        let Some(file_name) = field.file_name().map(str::to_string) else {
            continue;
        };
        let relative_path = upload_relative_path(&root, &file_name)?;
        let path = repo.image_path(&relative_path)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|source| {
                labello_storage::StorageError::Io {
                    path: parent.to_path_buf(),
                    source,
                }
            })?;
        }
        let bytes = field
            .bytes()
            .await
            .map_err(|error| ApiError::BadRequest(error.to_string()))?;
        tokio::fs::write(&path, bytes).await.map_err(|source| {
            labello_storage::StorageError::Io {
                path: path.clone(),
                source,
            }
        })?;
        written_files += 1;
    }
    if written_files == 0 {
        return Err(ApiError::BadRequest(
            "upload contained no files".to_string(),
        ));
    }
    if !metadata.image_roots.contains(&root) {
        metadata.image_roots.push(root);
        metadata.updated_at = labello_domain::now();
        repo.save_dataset(&metadata).await?;
    }
    let report = if query.ingest {
        repo.ingest_images().await?
    } else {
        labello_storage::IngestReport::default()
    };
    let report = storage_ingest_to_client(report);
    tracing::info!(
        event = "upload.completed",
        dataset_id = %dataset_id,
        actor_user_id = %actor.user_id,
        written_files,
        ingest_requested = query.ingest,
        new_images = report.new_images,
        "image upload completed"
    );
    Ok(Json(report))
}

async fn create_snapshot(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
) -> ApiResult<Json<labello_domain::DatasetSnapshot>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset_config().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::DataAdmin)?;
    let snapshot = repo.create_snapshot().await?;
    tracing::info!(
        event = "snapshot.created",
        dataset_id = %dataset_id,
        actor_user_id = %actor.user_id,
        snapshot_id = %snapshot.snapshot_id,
        "dataset snapshot created"
    );
    Ok(Json(snapshot))
}

async fn list_snapshots(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<labello_domain::DatasetSnapshot>>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset_config().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::DataAdmin)?;
    Ok(Json(repo.list_snapshots().await?))
}

async fn download_snapshot_file(
    State(state): State<ApiState>,
    Path((dataset_id, snapshot_id, file_path)): Path<(DatasetId, String, String)>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset_config().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::DataAdmin)?;
    let bytes = repo.snapshot_file(&snapshot_id, &file_path).await?;
    let media_type = mime_guess::from_path(&file_path).first_or_octet_stream();
    let file_name = file_path.rsplit('/').next().unwrap_or("snapshot-file");
    let disposition = format!("attachment; filename=\"{}\"", file_name.replace('"', ""));
    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_str(media_type.as_ref())
                    .map_err(|error| ApiError::Internal(error.to_string()))?,
            ),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_str(&disposition)
                    .map_err(|error| ApiError::Internal(error.to_string()))?,
            ),
        ],
        Body::from(bytes),
    ))
}

async fn list_images(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    Query(query): Query<ImageExplorerQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<ImageExplorerPage>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset_config().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::DataAdmin)?;
    let index = repo.load_images_index().await?;
    let search = query
        .search
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_lowercase);
    let mut items = Vec::new();
    for image in index.images_by_hash.into_values() {
        if search.as_ref().is_some_and(|search| {
            !image.file_name.to_lowercase().contains(search)
                && !image.canonical_path.to_lowercase().contains(search)
        }) {
            continue;
        }
        let state = repo.load_image_state(&image.image_id).await?;
        let mut task_statuses = metadata
            .tasks
            .iter()
            .map(|task| (task.task_id.clone(), TaskStatus::Pending))
            .collect::<std::collections::BTreeMap<_, _>>();
        task_statuses.extend(
            state
                .task_states
                .iter()
                .map(|(task_id, task_state)| (task_id.clone(), task_state.status.clone())),
        );
        let class_ids = state
            .active_annotations()
            .map(|annotation| annotation.class_id.clone())
            .collect();
        let item = ImageExplorerItem {
            image,
            task_statuses,
            class_ids,
        };
        if query.status.as_ref().is_some_and(|status| {
            if let Some(task_id) = query.task_id.as_ref() {
                item.task_statuses.get(task_id) != Some(status)
            } else {
                !item
                    .task_statuses
                    .values()
                    .any(|candidate| candidate == status)
            }
        }) {
            continue;
        }
        if query
            .task_id
            .as_ref()
            .is_some_and(|task_id| !item.task_statuses.contains_key(task_id))
        {
            continue;
        }
        if query
            .class_id
            .as_ref()
            .is_some_and(|class_id| !item.class_ids.contains(class_id))
        {
            continue;
        }
        items.push(item);
    }
    items.sort_by(|left, right| {
        left.image
            .canonical_path
            .cmp(&right.image.canonical_path)
            .then_with(|| left.image.image_id.cmp(&right.image.image_id))
    });
    let total_items = items.len();
    let page = query.page.max(1);
    let page_size = query.page_size.clamp(1, 100);
    let total_pages = total_items.div_ceil(page_size);
    let start = page
        .saturating_sub(1)
        .saturating_mul(page_size)
        .min(total_items);
    let end = start.saturating_add(page_size).min(total_items);
    Ok(Json(ImageExplorerPage {
        items: items[start..end].to_vec(),
        page,
        page_size,
        total_items,
        total_pages,
    }))
}

fn storage_ingest_to_client(report: labello_storage::IngestReport) -> labello_client::IngestReport {
    labello_client::IngestReport {
        discovered_files: report.discovered_files,
        new_images: report.new_images,
        duplicate_files: report
            .duplicate_files
            .into_iter()
            .take(MAX_INGEST_REPORT_DETAILS)
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
            .take(MAX_INGEST_REPORT_DETAILS)
            .map(|changed| labello_client::ChangedPath {
                relative_path: changed.relative_path,
                previous_blake3: changed.previous_blake3,
                current_blake3: changed.current_blake3,
            })
            .collect(),
        unreadable_files: report
            .unreadable_files
            .into_iter()
            .take(MAX_INGEST_REPORT_DETAILS)
            .collect(),
    }
}

fn normalize_upload_root(root: &str) -> ApiResult<String> {
    let root = root.trim().trim_matches('/').to_string();
    if root.is_empty() {
        Err(ApiError::BadRequest(
            "upload root cannot be empty".to_string(),
        ))
    } else {
        Ok(root)
    }
}

fn upload_relative_path(root: &str, file_name: &str) -> ApiResult<String> {
    let file_name = file_name.trim().trim_matches('/').replace('\\', "/");
    if file_name.is_empty() {
        return Err(ApiError::BadRequest(
            "upload file name is empty".to_string(),
        ));
    }
    Ok(format!("{root}/{file_name}"))
}

async fn list_tasks(
    State(state): State<ApiState>,
    Path(dataset_id): Path<DatasetId>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<TaskDefinition>>> {
    let actor = actor_from_headers(&state, &headers)?;
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset_config().await?;
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
    let repo = state.repo(&dataset_id)?;
    let mut metadata = repo.load_dataset_config().await?;
    ensure_dataset_role(&metadata, &actor, DatasetRole::DataAdmin)?;
    validate_enabled_task(&task)?;
    for class_id in &task.class_ids {
        if !metadata
            .label_classes
            .iter()
            .any(|class| &class.class_id == class_id)
        {
            return Err(ApiError::BadRequest(format!(
                "task {} references unknown class {class_id}",
                task.task_id
            )));
        }
    }
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
    let repo = state.repo(&dataset_id)?;
    let metadata = repo.load_dataset_config().await?;
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
    let repo = state.repo(&dataset_id)?;
    let mut metadata = repo.load_dataset_config().await?;
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

fn sanitize_dataset(mut metadata: DatasetMetadata, actor: &Actor) -> DatasetMetadata {
    metadata.images.clear();
    if !has_dataset_role(&metadata, &actor.user_id, &DatasetRole::DataAdmin) {
        metadata.role_assignments.clear();
        metadata.image_roots.clear();
        metadata
            .prelabel_configs
            .retain(|config| config.available_to_annotators);
    }
    metadata
}

fn config_response(mut metadata: DatasetMetadata) -> DatasetMetadata {
    metadata.images.clear();
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
        validate_enabled_task(task)?;
        for class_id in &task.class_ids {
            if !class_ids.contains(class_id) {
                return Err(ApiError::BadRequest(format!(
                    "task {} references unknown class {class_id}",
                    task.task_id
                )));
            }
        }
    }
    let mut role_users = BTreeSet::new();
    for assignment in &request.role_assignments {
        assignment.user_id.validate_path_segment()?;
        if assignment.dataset_id != metadata.dataset_id {
            return Err(ApiError::BadRequest(format!(
                "role assignment for {} belongs to a different dataset",
                assignment.user_id
            )));
        }
        if assignment.roles.is_empty() {
            return Err(ApiError::BadRequest(format!(
                "role assignment for {} must contain at least one role",
                assignment.user_id
            )));
        }
        if !role_users.insert(&assignment.user_id) {
            return Err(ApiError::BadRequest(format!(
                "duplicate role assignment for user {}",
                assignment.user_id
            )));
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

fn validate_enabled_task(task: &TaskDefinition) -> ApiResult<()> {
    if task.enabled && task.review.workflow == ReviewWorkflow::IndependentAgreement {
        return Err(ApiError::BadRequest(format!(
            "independent agreement workflow is not implemented for task {}",
            task.task_id
        )));
    }
    if task.enabled && task.class_ids.len() != 1 {
        return Err(ApiError::BadRequest(format!(
            "enabled task {} must have exactly one class",
            task.task_id
        )));
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

#[cfg(test)]
mod report_tests {
    use super::*;

    #[test]
    fn enabled_tasks_require_exactly_one_class() {
        let mut task = TaskDefinition {
            task_id: labello_domain::TaskId::from("task"),
            name: "Task".to_string(),
            annotation_type: labello_domain::AnnotationType::BoundingBox,
            class_ids: Vec::new(),
            instructions: labello_domain::TutorialContent {
                title: "Instructions".to_string(),
                example_text: "Label it".to_string(),
                example_images: Vec::new(),
            },
            skeleton: None,
            review: labello_domain::ReviewConfig::default(),
            prelabel_config_ids: Vec::new(),
            enabled: true,
        };
        assert!(validate_enabled_task(&task).is_err());
        task.class_ids = vec![
            labello_domain::ClassId::from("one"),
            labello_domain::ClassId::from("two"),
        ];
        assert!(validate_enabled_task(&task).is_err());
        task.class_ids.truncate(1);
        assert!(validate_enabled_task(&task).is_ok());
        task.review.workflow = ReviewWorkflow::IndependentAgreement;
        assert!(validate_enabled_task(&task).is_err());
        task.enabled = false;
        assert!(validate_enabled_task(&task).is_ok());
    }

    #[test]
    fn caps_ingest_report_details() {
        let report = labello_storage::IngestReport {
            duplicate_files: (0..150)
                .map(|index| labello_storage::DuplicateImage {
                    image_id: ImageId::from(format!("img_{index}")),
                    canonical_path: format!("images/{index}.png"),
                    duplicate_path: format!("dupes/{index}.png"),
                    blake3: format!("hash_{index}"),
                })
                .collect(),
            changed_paths: (0..150)
                .map(|index| labello_storage::ChangedPath {
                    relative_path: format!("images/{index}.png"),
                    previous_blake3: format!("old_{index}"),
                    current_blake3: format!("new_{index}"),
                })
                .collect(),
            unreadable_files: (0..150).map(|index| format!("bad/{index}.png")).collect(),
            ..Default::default()
        };

        let report = storage_ingest_to_client(report);

        assert_eq!(report.duplicate_files.len(), MAX_INGEST_REPORT_DETAILS);
        assert_eq!(report.changed_paths.len(), MAX_INGEST_REPORT_DETAILS);
        assert_eq!(report.unreadable_files.len(), MAX_INGEST_REPORT_DETAILS);
    }
}
