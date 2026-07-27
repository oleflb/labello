use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path as AxumPath, Query, State},
    http::HeaderMap,
    routing::{get, post},
};
use labello_client as client;
use labello_domain::{ImportId, UserId};
use labello_storage as storage;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    ApiState,
    auth::actor_from_headers,
    error::{ApiError, ApiResult},
};

const IDEMPOTENCY_HEADER: &str = "idempotency-key";
const API_STATE_DIR: &str = "api";
const API_JOBS_DIR: &str = "jobs";
const API_REQUESTS_DIR: &str = "requests";
const MAX_JSON_BODY: usize = 1024 * 1024;
const MAX_REGISTRATION_BODY: usize = 8 * 1024 * 1024;

pub(super) fn routes(state: &ApiState) -> Router<ApiState> {
    let chunk_limit = state
        .import_service()
        .map(|service| service.capabilities().limits.upload_chunk_bytes)
        .unwrap_or_else(|| storage::ImportLimits::default().upload_chunk_bytes);
    let control = Router::new()
        .route("/import-capabilities", get(capabilities))
        .route("/import-roots/{root_id}/browse", post(browse_import_root))
        .route("/imports", get(list_imports).post(create_import))
        .route("/imports/{import_id}", get(get_import))
        .route(
            "/imports/{import_id}/source/browse",
            post(browse_import_source),
        )
        .route(
            "/imports/{import_id}/yolo-descriptor/inspect",
            post(inspect_yolo_descriptor),
        )
        .route("/imports/{import_id}/seal", post(seal_import))
        .route("/imports/{import_id}/preflight", post(preflight_import))
        .route(
            "/imports/{import_id}/plan",
            get(get_import_plan).put(update_import_plan),
        )
        .route("/imports/{import_id}/diagnostics", get(import_diagnostics))
        .route("/imports/{import_id}/commit", post(commit_import))
        .route("/imports/{import_id}/cancel", post(cancel_import))
        .layer(DefaultBodyLimit::max(MAX_JSON_BODY));
    let registration = Router::new()
        .route("/imports/{import_id}/files/register", post(register_files))
        .layer(DefaultBodyLimit::max(MAX_REGISTRATION_BODY));
    let chunks = Router::new()
        .route(
            "/imports/{import_id}/files/{file_id}/chunks",
            post(upload_chunk),
        )
        .layer(DefaultBodyLimit::max(chunk_limit));
    control.merge(registration).merge(chunks)
}

async fn capabilities(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> ApiResult<Json<client::ImportCapabilities>> {
    let actor = import_actor(&state, &headers)?;
    ensure_import_admin(&state, &actor.user_id)?;
    Ok(Json(convert_capabilities(&state, &actor.user_id)))
}

async fn browse_import_root(
    State(state): State<ApiState>,
    AxumPath(root_id): AxumPath<String>,
    headers: HeaderMap,
    Json(request): Json<client::BrowseServerImportRootRequest>,
) -> ApiResult<Json<client::ImportBrowsePage>> {
    let actor = import_actor(&state, &headers)?;
    ensure_import_admin(&state, &actor.user_id)?;
    let page = require_service(&state)?
        .browse_server_root(
            &root_id,
            &actor.user_id,
            &request.relative_path,
            request.offset as usize,
        )
        .await
        .map_err(map_storage)?;
    Ok(Json(convert_browse_page(page)))
}

async fn list_imports(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<client::ImportJob>>> {
    let actor = import_actor(&state, &headers)?;
    ensure_import_admin(&state, &actor.user_id)?;
    let service = require_service(&state)?;
    let mut jobs = Vec::new();
    for control in list_job_controls(&state).await? {
        if control.owner_user_id != actor.user_id {
            continue;
        }
        match service.job(&control.import_id, &actor.user_id).await {
            Ok(job) => {
                let control = reconcile_job_control(&state, job.clone(), control).await?;
                jobs.push(convert_job(job, Some(&control)));
            }
            Err(storage::StorageError::NotFound(_)) => {}
            Err(error) => return Err(map_storage(error)),
        }
    }
    jobs.sort_by_key(|job| std::cmp::Reverse(job.created_at));
    Ok(Json(jobs))
}

async fn get_import(
    State(state): State<ApiState>,
    AxumPath(import_id): AxumPath<ImportId>,
    headers: HeaderMap,
) -> ApiResult<Json<client::ImportJob>> {
    let actor = import_actor(&state, &headers)?;
    ensure_import_admin(&state, &actor.user_id)?;
    let job = require_service(&state)?
        .job(&import_id, &actor.user_id)
        .await
        .map_err(map_storage)?;
    let control = match load_job_control(&state, &import_id).await {
        Ok(control) => Some(control),
        Err(ApiError::Storage(storage::StorageError::NotFound(_))) => None,
        Err(error) => return Err(error),
    };
    let control = match control {
        Some(control) => Some(reconcile_job_control(&state, job.clone(), control).await?),
        None => None,
    };
    Ok(Json(convert_job(job, control.as_ref())))
}

async fn create_import(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(request): Json<client::CreateImportRequest>,
) -> ApiResult<Json<client::ImportJob>> {
    let actor = import_actor(&state, &headers)?;
    ensure_import_admin(&state, &actor.user_id)?;
    let key = idempotency_key(&headers)?;
    let _command = state.lock_import_commands().await;
    if let Idempotency::Replay(response) =
        begin_idempotency(&state, &actor.user_id, key, "create", &request).await?
    {
        return Ok(Json(response));
    }

    let profile = storage_profile(request.profile)?;
    let (transport, server_selection) = match &request.source {
        client::ImportSourceSelection::BrowserFolder => (storage::ImportTransport::Browser, None),
        client::ImportSourceSelection::ServerDirectory {
            import_root_id,
            relative_path,
        } => (
            storage::ImportTransport::ServerDirectory,
            Some(storage::ServerDirectorySelection {
                root_id: import_root_id.clone(),
                relative_directory: relative_path.clone(),
            }),
        ),
    };
    let storage_request = storage::CreateImportRequest {
        destination_dataset_id: request.destination_dataset_id.clone(),
        destination_name: request.destination_name.clone(),
        profile,
        transport,
    };
    let service = require_service(&state)?;
    let _mutation = state.lock_datasets_root_mutation().await;
    let mut recovered_create = false;
    let mut job = match service
        .create_job(actor.user_id.clone(), storage_request.clone())
        .await
    {
        Ok(job) => job,
        Err(storage::StorageError::Import { code, message })
            if matches!(code.as_str(), "destination_reserved" | "destination_exists") =>
        {
            let job = find_matching_job(&state, &actor.user_id, &storage_request)
                .await?
                .ok_or_else(|| map_storage(storage::StorageError::Import { code, message }))?;
            recovered_create = true;
            job
        }
        Err(error) => return Err(map_storage(error)),
    };
    if recovered_create
        && server_selection.is_some()
        && job.phase == storage::ImportJobPhase::Registering
    {
        service
            .cancel(&job.import_id, &actor.user_id)
            .await
            .map_err(map_storage)?;
        job = service
            .create_job(actor.user_id.clone(), storage_request)
            .await
            .map_err(map_storage)?;
    }
    drop(_mutation);
    if let Some(selection) = server_selection
        && job.phase == storage::ImportJobPhase::Registering
    {
        job = match service
            .copy_server_directory(&job.import_id, &actor.user_id, selection)
            .await
        {
            Ok(job) => job,
            Err(error) => {
                if let Err(cancel_error) = service.cancel(&job.import_id, &actor.user_id).await {
                    tracing::error!(
                        event = "import.create.cleanup_failed",
                        import_id = %job.import_id,
                        error_kind = cancel_error.kind(),
                        diagnostic = cancel_error.safe_diagnostic().as_deref().unwrap_or("redacted"),
                        "failed to clean up server-directory import"
                    );
                }
                return Err(map_storage(error));
            }
        };
    }
    let mut control = JobControl {
        import_id: job.import_id.clone(),
        owner_user_id: actor.user_id.clone(),
        create_request: request,
        seal_request: None,
        files: BTreeMap::new(),
        plan: None,
        accepted_plan_request: None,
        pending_plan_request: None,
    };
    if transport == storage::ImportTransport::ServerDirectory {
        control.files = source_files(&state, &job.import_id).await?;
    }
    save_job_control(&state, &control).await?;
    let response = convert_job(job, Some(&control));
    complete_idempotency(&state, &actor.user_id, key, "create", &response).await?;
    tracing::info!(
        event = "import.created",
        import_id = %response.import_id,
        owner_user_id = %actor.user_id,
        profile = ?response.profile,
        transport = ?response.transport,
        "import job created"
    );
    Ok(Json(response))
}

async fn register_files(
    State(state): State<ApiState>,
    AxumPath(import_id): AxumPath<ImportId>,
    headers: HeaderMap,
    Json(request): Json<client::RegisterImportFilesRequest>,
) -> ApiResult<Json<client::RegisterImportFilesResult>> {
    let actor = import_actor(&state, &headers)?;
    ensure_import_admin(&state, &actor.user_id)?;
    let key = idempotency_key(&headers)?;
    let _command = state.lock_import_commands().await;
    if let Idempotency::Replay(response) = begin_idempotency(
        &state,
        &actor.user_id,
        key,
        &format!("register:{import_id}"),
        &request,
    )
    .await?
    {
        return Ok(Json(response));
    }
    require_owned_job(&state, &import_id, &actor.user_id).await?;
    let registrations = request
        .files
        .iter()
        .map(|file| {
            let digest = file.blake3.as_deref().ok_or_else(|| {
                ApiError::Unprocessable(
                    "a full BLAKE3 digest is required for every file".to_string(),
                )
            })?;
            Ok(storage::BrowserFileRegistration {
                relative_path: file.relative_path.clone(),
                byte_size: file.byte_size,
                blake3: parse_digest(digest)?.to_string(),
            })
        })
        .collect::<ApiResult<Vec<_>>>()?;
    let registered = match require_service(&state)?
        .register_browser_files(&import_id, &actor.user_id, registrations)
        .await
    {
        Ok(registered) => registered,
        Err(storage::StorageError::Import { ref code, .. }) if code == "source_path_collision" => {
            reconcile_registered_files(&state, &import_id, &request)
                .await?
                .ok_or_else(|| {
                    ApiError::Conflict("registered source paths already exist".to_string())
                })?
        }
        Err(error) => return Err(map_storage(error)),
    };
    let mut control = load_job_control(&state, &import_id).await?;
    let files = request
        .files
        .iter()
        .zip(&registered)
        .map(|(requested, stored)| {
            control.files.insert(
                stored.file_id.clone(),
                FileControl {
                    client_file_id: Some(requested.client_file_id.clone()),
                    relative_path: stored.relative_path.clone(),
                    byte_size: stored.byte_size,
                    blake3: stored.blake3.clone(),
                    accepted_bytes: stored.accepted_bytes,
                    complete: stored.complete,
                },
            );
            client::RegisteredImportFile {
                client_file_id: requested.client_file_id.clone(),
                file_id: stored.file_id.clone(),
                byte_size: stored.byte_size,
                accepted_bytes: stored.accepted_bytes,
                complete: stored.complete,
            }
        })
        .collect::<Vec<_>>();
    save_job_control(&state, &control).await?;
    let response = client::RegisterImportFilesResult {
        registered_files: files.len() as u64,
        registered_bytes: files.iter().map(|file| file.byte_size).sum(),
        files,
    };
    complete_idempotency(
        &state,
        &actor.user_id,
        key,
        &format!("register:{import_id}"),
        &response,
    )
    .await?;
    Ok(Json(response))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChunkIdentity<'a> {
    file_id: &'a str,
    offset: u64,
    length: u64,
    digest: &'a str,
    body_blake3: String,
}

async fn upload_chunk(
    State(state): State<ApiState>,
    AxumPath((import_id, file_id)): AxumPath<(ImportId, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<client::ImportChunkResult>> {
    let actor = import_actor(&state, &headers)?;
    ensure_import_admin(&state, &actor.user_id)?;
    let key = idempotency_key(&headers)?;
    let offset = required_u64_header(&headers, "upload-offset")?;
    let length = required_u64_header(&headers, "upload-length")?;
    if length != body.len() as u64 {
        return Err(ApiError::BadRequest(
            "upload-length does not match the request body".to_string(),
        ));
    }
    let digest_header = required_header(&headers, "digest")?;
    let digest = parse_digest(digest_header)?;
    let identity = ChunkIdentity {
        file_id: &file_id,
        offset,
        length,
        digest,
        body_blake3: blake3::hash(&body).to_hex().to_string(),
    };
    let operation = format!("chunk:{import_id}:{file_id}");
    let _command = state.lock_import_commands().await;
    if let Idempotency::Replay(response) =
        begin_idempotency(&state, &actor.user_id, key, &operation, &identity).await?
    {
        return Ok(Json(response));
    }
    require_owned_job(&state, &import_id, &actor.user_id).await?;
    let file = require_service(&state)?
        .upload_chunk(&import_id, &actor.user_id, &file_id, offset, &body, digest)
        .await
        .map_err(map_storage)?;
    let mut control = load_job_control(&state, &import_id).await?;
    if let Some(stored) = control.files.get_mut(&file_id) {
        stored.accepted_bytes = file.accepted_bytes;
        stored.complete = file.complete;
    }
    save_job_control(&state, &control).await?;
    let response = client::ImportChunkResult {
        file_id: file.file_id,
        accepted_offset: file.accepted_bytes,
        complete: file.complete,
        file_blake3: file.complete.then_some(file.blake3),
    };
    complete_idempotency(&state, &actor.user_id, key, &operation, &response).await?;
    Ok(Json(response))
}

async fn inspect_yolo_descriptor(
    State(state): State<ApiState>,
    AxumPath(import_id): AxumPath<ImportId>,
    headers: HeaderMap,
    Json(request): Json<client::InspectYoloDescriptorRequest>,
) -> ApiResult<Json<client::YoloDescriptorInspection>> {
    let actor = import_actor(&state, &headers)?;
    ensure_import_admin(&state, &actor.user_id)?;
    require_owned_job(&state, &import_id, &actor.user_id).await?;
    let control = load_job_control(&state, &import_id).await?;
    let descriptor_path =
        resolve_source_reference(&control, &request.descriptor_file_id)?.to_string();
    let inspection = require_service(&state)?
        .inspect_yolo_descriptor(&import_id, &actor.user_id, &descriptor_path)
        .await
        .map_err(map_storage)?;
    Ok(Json(client::YoloDescriptorInspection {
        splits: inspection
            .splits
            .into_iter()
            .map(|split| client::YoloSplitInspection {
                name: split.name,
                usable: split.usable,
                issue: split.issue,
            })
            .collect(),
    }))
}

async fn browse_import_source(
    State(state): State<ApiState>,
    AxumPath(import_id): AxumPath<ImportId>,
    headers: HeaderMap,
    Json(request): Json<client::BrowseImportSourceRequest>,
) -> ApiResult<Json<client::ImportBrowsePage>> {
    let actor = import_actor(&state, &headers)?;
    ensure_import_admin(&state, &actor.user_id)?;
    let mode = match request.mode {
        client::ImportSourceBrowseMode::Descriptors => storage::ImportBrowseMode::Descriptors,
        client::ImportSourceBrowseMode::Images => storage::ImportBrowseMode::Images,
    };
    let page = require_service(&state)?
        .browse_staged_source(
            &import_id,
            &actor.user_id,
            &request.relative_path,
            request.offset as usize,
            mode,
        )
        .await
        .map_err(map_storage)?;
    Ok(Json(convert_browse_page(page)))
}

fn convert_browse_page(page: storage::ImportBrowsePage) -> client::ImportBrowsePage {
    client::ImportBrowsePage {
        relative_path: page.relative_path,
        entries: page
            .entries
            .into_iter()
            .map(|entry| client::ImportBrowseEntry {
                name: entry.name,
                relative_path: entry.relative_path,
                kind: match entry.kind {
                    storage::ImportBrowseEntryKind::Directory => {
                        client::ImportBrowseEntryKind::Directory
                    }
                    storage::ImportBrowseEntryKind::File => client::ImportBrowseEntryKind::File,
                },
                file_id: entry.file_id,
            })
            .collect(),
        next_offset: page
            .next_offset
            .and_then(|offset| u32::try_from(offset).ok()),
    }
}

async fn seal_import(
    State(state): State<ApiState>,
    AxumPath(import_id): AxumPath<ImportId>,
    headers: HeaderMap,
    Json(request): Json<client::SealImportRequest>,
) -> ApiResult<Json<client::SealImportResult>> {
    let actor = import_actor(&state, &headers)?;
    ensure_import_admin(&state, &actor.user_id)?;
    let key = idempotency_key(&headers)?;
    let operation = format!("seal:{import_id}");
    let _command = state.lock_import_commands().await;
    if let Idempotency::Replay(response) =
        begin_idempotency(&state, &actor.user_id, key, &operation, &request).await?
    {
        return Ok(Json(response));
    }
    let job = require_owned_job(&state, &import_id, &actor.user_id).await?;
    if request.source.descriptors.is_empty() || request.source.source_namespace.trim().is_empty() {
        return Err(ApiError::Unprocessable(
            "a source namespace and at least one descriptor are required".to_string(),
        ));
    }
    let control = load_job_control(&state, &import_id).await?;
    convert_preflight(&job, &control, &request)?;
    let job = require_service(&state)?
        .seal(&import_id, &actor.user_id)
        .await
        .map_err(map_storage)?;
    let mut control = load_job_control(&state, &import_id).await?;
    control.seal_request = Some(request);
    save_job_control(&state, &control).await?;
    let response = client::SealImportResult {
        import_id: import_id.clone(),
        source_fingerprint: job.source_fingerprint.unwrap_or_default(),
        files: job.accepted_files as u64,
        bytes: job.accepted_bytes,
    };
    complete_idempotency(&state, &actor.user_id, key, &operation, &response).await?;
    tracing::info!(
        event = "import.sealed",
        import_id = %import_id,
        owner_user_id = %actor.user_id,
        file_count = response.files,
        byte_count = response.bytes,
        "import source sealed"
    );
    Ok(Json(response))
}

async fn preflight_import(
    State(state): State<ApiState>,
    AxumPath(import_id): AxumPath<ImportId>,
    headers: HeaderMap,
    Json(request): Json<client::StartImportPreflightRequest>,
) -> ApiResult<Json<client::ImportJob>> {
    let actor = import_actor(&state, &headers)?;
    ensure_import_admin(&state, &actor.user_id)?;
    let key = idempotency_key(&headers)?;
    let operation = format!("preflight:{import_id}");
    let _command = state.lock_import_commands().await;
    if let Idempotency::Replay(response) =
        begin_idempotency(&state, &actor.user_id, key, &operation, &request).await?
    {
        return Ok(Json(response));
    }
    let job = require_owned_job(&state, &import_id, &actor.user_id).await?;
    let mut control = load_job_control(&state, &import_id).await?;
    let seal = control.seal_request.as_ref().ok_or_else(|| {
        ApiError::Conflict("the import source must be sealed before preflight".to_string())
    })?;
    let preflight_request = convert_preflight(&job, &control, seal)?;
    let plan = require_service(&state)?
        .preflight(&import_id, &actor.user_id, preflight_request)
        .await
        .map_err(map_storage)?;
    control.plan = Some(plan.clone());
    save_job_control(&state, &control).await?;
    let current = require_owned_job(&state, &import_id, &actor.user_id).await?;
    let response = convert_job(current, Some(&control));
    complete_idempotency(&state, &actor.user_id, key, &operation, &response).await?;
    tracing::info!(
        event = "import.preflight.completed",
        import_id = %import_id,
        owner_user_id = %actor.user_id,
        diagnostic_count = plan.diagnostics.len(),
        image_count = plan.totals.images,
        annotation_count = plan.totals.output_annotations,
        committable = plan.committable(),
        "import preflight completed"
    );
    Ok(Json(response))
}

async fn get_import_plan(
    State(state): State<ApiState>,
    AxumPath(import_id): AxumPath<ImportId>,
    headers: HeaderMap,
) -> ApiResult<Json<client::ImportPlan>> {
    let actor = import_actor(&state, &headers)?;
    ensure_import_admin(&state, &actor.user_id)?;
    let plan = require_service(&state)?
        .plan(&import_id, &actor.user_id)
        .await
        .map_err(map_storage)?;
    let job = require_owned_job(&state, &import_id, &actor.user_id).await?;
    let control =
        reconcile_job_control(&state, job, load_job_control(&state, &import_id).await?).await?;
    Ok(Json(convert_plan(
        &plan,
        control.accepted_plan_request.as_ref(),
    )))
}

async fn update_import_plan(
    State(state): State<ApiState>,
    AxumPath(import_id): AxumPath<ImportId>,
    headers: HeaderMap,
    Json(request): Json<client::UpdateImportPlanRequest>,
) -> ApiResult<Json<client::ImportPlan>> {
    let actor = import_actor(&state, &headers)?;
    ensure_import_admin(&state, &actor.user_id)?;
    let key = idempotency_key(&headers)?;
    let operation = format!("plan:{import_id}");
    let _command = state.lock_import_commands().await;
    if let Idempotency::Replay(response) =
        begin_idempotency(&state, &actor.user_id, key, &operation, &request).await?
    {
        return Ok(Json(response));
    }
    let service = require_service(&state)?;
    let current = service
        .plan(&import_id, &actor.user_id)
        .await
        .map_err(map_storage)?;
    validate_plan_update_against_current(&current, &request)?;
    let preflight = convert_plan_update(current.request, request.clone())?;
    let mut control = load_job_control(&state, &import_id).await?;
    if control.owner_user_id != actor.user_id {
        return Err(ApiError::NotFound("import job".to_string()));
    }
    control.pending_plan_request = Some(request.clone());
    save_job_control(&state, &control).await?;
    let plan = service
        .update_plan(&import_id, &actor.user_id, preflight)
        .await
        .map_err(map_storage)?;
    control.plan = Some(plan.clone());
    control.accepted_plan_request = Some(request.clone());
    control.pending_plan_request = None;
    save_job_control(&state, &control).await?;
    let response = convert_plan(&plan, Some(&request));
    complete_idempotency(&state, &actor.user_id, key, &operation, &response).await?;
    Ok(Json(response))
}

async fn import_diagnostics(
    State(state): State<ApiState>,
    AxumPath(import_id): AxumPath<ImportId>,
    Query(query): Query<client::ImportDiagnosticsQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<client::ImportDiagnosticsPage>> {
    let actor = import_actor(&state, &headers)?;
    ensure_import_admin(&state, &actor.user_id)?;
    require_owned_job(&state, &import_id, &actor.user_id).await?;
    let max = convert_capabilities(&state, &actor.user_id)
        .limits
        .max_diagnostic_page_size;
    if query.limit == 0 || query.limit > max {
        return Err(ApiError::BadRequest(format!(
            "diagnostic limit must be between 1 and {max}"
        )));
    }
    let offset = query
        .cursor
        .as_deref()
        .map(|value| value.parse::<u64>())
        .transpose()
        .map_err(|_| ApiError::BadRequest("invalid diagnostic cursor".to_string()))?
        .unwrap_or(0);
    let plan = require_service(&state)?
        .plan(&import_id, &actor.user_id)
        .await
        .map_err(map_storage)?;
    Ok(Json(diagnostic_page(&plan, &query, offset)))
}

fn diagnostic_page(
    plan: &storage::ImportPlan,
    query: &client::ImportDiagnosticsQuery,
    offset: u64,
) -> client::ImportDiagnosticsPage {
    let diagnostics = plan
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            query
                .code
                .as_ref()
                .is_none_or(|code| &diagnostic.code == code)
                && query
                    .severity
                    .is_none_or(|severity| client_severity(diagnostic.severity) == severity)
        })
        .collect::<Vec<_>>();
    let total = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.count)
        .sum::<u64>();
    let end = offset.saturating_add(u64::from(query.limit)).min(total);
    let mut page = Vec::with_capacity(query.limit as usize);
    let mut position = 0_u64;
    for diagnostic in diagnostics {
        let next = position.saturating_add(diagnostic.count);
        if offset < next && end > position {
            let start_in_diagnostic = offset.saturating_sub(position);
            let end_in_diagnostic = end.saturating_sub(position).min(diagnostic.count);
            for occurrence in start_in_diagnostic..end_in_diagnostic {
                page.push(convert_diagnostic(
                    diagnostic,
                    position + occurrence,
                    occurrence,
                ));
            }
        }
        position = next;
        if position >= end {
            break;
        }
    }
    client::ImportDiagnosticsPage {
        diagnostics: page,
        next_cursor: (end < total).then(|| end.to_string()),
        total,
    }
}

async fn commit_import(
    State(state): State<ApiState>,
    AxumPath(import_id): AxumPath<ImportId>,
    headers: HeaderMap,
    Json(request): Json<client::CommitImportRequest>,
) -> ApiResult<Json<client::CommitImportResult>> {
    let actor = import_actor(&state, &headers)?;
    // Reauthorization deliberately occurs immediately before every commit attempt and retry.
    ensure_import_admin(&state, &actor.user_id)?;
    let key = idempotency_key(&headers)?;
    let operation = format!("commit:{import_id}");
    let _command = state.lock_import_commands().await;
    if let Idempotency::Replay(response) =
        begin_idempotency(&state, &actor.user_id, key, &operation, &request).await?
    {
        return Ok(Json(response));
    }
    require_owned_job(&state, &import_id, &actor.user_id).await?;
    let _mutation = state.lock_datasets_root_mutation().await;
    let result = require_service(&state)?
        .commit(&import_id, &actor.user_id, &request.plan_hash)
        .await
        .map_err(map_storage)?;
    let response = client::CommitImportResult {
        import_id: result.import_id,
        dataset_id: result.dataset_id,
        plan_hash: request.plan_hash,
        recovered: result.recovered,
    };
    complete_idempotency(&state, &actor.user_id, key, &operation, &response).await?;
    tracing::info!(
        event = "import.committed",
        import_id = %response.import_id,
        dataset_id = %response.dataset_id,
        owner_user_id = %actor.user_id,
        recovered = response.recovered,
        "import dataset published"
    );
    Ok(Json(response))
}

async fn cancel_import(
    State(state): State<ApiState>,
    AxumPath(import_id): AxumPath<ImportId>,
    headers: HeaderMap,
    Json(request): Json<client::CancelImportRequest>,
) -> ApiResult<Json<client::CancelImportResult>> {
    let actor = import_actor(&state, &headers)?;
    ensure_import_admin(&state, &actor.user_id)?;
    let key = idempotency_key(&headers)?;
    let operation = format!("cancel:{import_id}");
    let _command = state.lock_import_commands().await;
    if let Idempotency::Replay(response) =
        begin_idempotency(&state, &actor.user_id, key, &operation, &request).await?
    {
        return Ok(Json(response));
    }
    require_owned_job(&state, &import_id, &actor.user_id).await?;
    let job = require_service(&state)?
        .cancel(&import_id, &actor.user_id)
        .await
        .map_err(map_storage)?;
    let response = client::CancelImportResult {
        import_id,
        lifecycle: client_phase(job.phase),
    };
    complete_idempotency(&state, &actor.user_id, key, &operation, &response).await?;
    Ok(Json(response))
}

fn import_actor(state: &ApiState, headers: &HeaderMap) -> ApiResult<labello_domain::Actor> {
    actor_from_headers(state, headers)
}

fn ensure_import_admin(state: &ApiState, user_id: &UserId) -> ApiResult<()> {
    if state.is_bootstrap_admin(user_id) {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "bootstrap administrator access required".to_string(),
        ))
    }
}

fn require_service(state: &ApiState) -> ApiResult<&storage::ImportService> {
    state
        .import_service()
        .map(AsRef::as_ref)
        .ok_or_else(|| ApiError::Conflict("dataset import is unavailable".to_string()))
}

async fn require_owned_job(
    state: &ApiState,
    import_id: &ImportId,
    owner: &UserId,
) -> ApiResult<storage::ImportJob> {
    require_service(state)?
        .job(import_id, owner)
        .await
        .map_err(map_storage)
}

fn idempotency_key(headers: &HeaderMap) -> ApiResult<&str> {
    let key = required_header(headers, IDEMPOTENCY_HEADER)?;
    if key.len() > 200
        || key.is_empty()
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

fn required_header<'a>(headers: &'a HeaderMap, name: &str) -> ApiResult<&'a str> {
    let values = headers.get_all(name);
    let mut values = values.iter();
    let value = values
        .next()
        .filter(|_| values.next().is_none())
        .ok_or_else(|| ApiError::BadRequest(format!("exactly one {name} header is required")))?;
    value
        .to_str()
        .map_err(|_| ApiError::BadRequest(format!("{name} header is invalid")))
}

fn required_u64_header(headers: &HeaderMap, name: &str) -> ApiResult<u64> {
    required_header(headers, name)?
        .parse()
        .map_err(|_| ApiError::BadRequest(format!("{name} must be an unsigned integer")))
}

fn parse_digest(value: &str) -> ApiResult<&str> {
    let value = value.strip_prefix("blake3=").unwrap_or(value);
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError::BadRequest(
            "digest must contain a full hexadecimal BLAKE3 digest".to_string(),
        ));
    }
    Ok(value)
}

fn storage_profile(profile: client::ImportProfile) -> ApiResult<storage::ImportProfile> {
    match profile {
        client::ImportProfile::UltralyticsYoloDetectV1 => {
            Ok(storage::ImportProfile::UltralyticsYoloDetectV1)
        }
        client::ImportProfile::UltralyticsYoloPoseV1 => {
            Ok(storage::ImportProfile::UltralyticsYoloPoseV1)
        }
        client::ImportProfile::CocoInstancesGtV1 => Ok(storage::ImportProfile::CocoInstancesGtV1),
        client::ImportProfile::CocoKeypointsGtV1 => Ok(storage::ImportProfile::CocoKeypointsGtV1),
        client::ImportProfile::Unknown => Err(ApiError::Unprocessable(
            "unsupported import profile".to_string(),
        )),
    }
}

fn client_profile(profile: storage::ImportProfile) -> client::ImportProfile {
    match profile {
        storage::ImportProfile::UltralyticsYoloDetectV1 => {
            client::ImportProfile::UltralyticsYoloDetectV1
        }
        storage::ImportProfile::UltralyticsYoloPoseV1 => {
            client::ImportProfile::UltralyticsYoloPoseV1
        }
        storage::ImportProfile::CocoInstancesGtV1 => client::ImportProfile::CocoInstancesGtV1,
        storage::ImportProfile::CocoKeypointsGtV1 => client::ImportProfile::CocoKeypointsGtV1,
    }
}

fn client_phase(phase: storage::ImportJobPhase) -> client::ImportLifecycle {
    match phase {
        storage::ImportJobPhase::Registering => client::ImportLifecycle::Registering,
        storage::ImportJobPhase::Uploading => client::ImportLifecycle::Uploading,
        storage::ImportJobPhase::Sealed => client::ImportLifecycle::Sealed,
        storage::ImportJobPhase::Preflighting => client::ImportLifecycle::Preflighting,
        storage::ImportJobPhase::AwaitingDecision => client::ImportLifecycle::AwaitingDecision,
        storage::ImportJobPhase::Building => client::ImportLifecycle::Building,
        storage::ImportJobPhase::Verifying => client::ImportLifecycle::Verifying,
        storage::ImportJobPhase::Committing => client::ImportLifecycle::Committing,
        storage::ImportJobPhase::Succeeded => client::ImportLifecycle::Succeeded,
        storage::ImportJobPhase::Failed => client::ImportLifecycle::Failed,
        storage::ImportJobPhase::Cancelled => client::ImportLifecycle::Cancelled,
        storage::ImportJobPhase::Expired => client::ImportLifecycle::Expired,
    }
}

fn progress_phase(phase: storage::ImportJobPhase) -> client::ImportProgressPhase {
    match phase {
        storage::ImportJobPhase::Registering => client::ImportProgressPhase::Registration,
        storage::ImportJobPhase::Uploading => client::ImportProgressPhase::Upload,
        storage::ImportJobPhase::Sealed => client::ImportProgressPhase::Sealing,
        storage::ImportJobPhase::Preflighting | storage::ImportJobPhase::AwaitingDecision => {
            client::ImportProgressPhase::Preflight
        }
        storage::ImportJobPhase::Building => client::ImportProgressPhase::Build,
        storage::ImportJobPhase::Verifying => client::ImportProgressPhase::Verification,
        storage::ImportJobPhase::Committing | storage::ImportJobPhase::Succeeded => {
            client::ImportProgressPhase::Commit
        }
        storage::ImportJobPhase::Failed
        | storage::ImportJobPhase::Cancelled
        | storage::ImportJobPhase::Expired => client::ImportProgressPhase::Cleanup,
    }
}

fn convert_job(job: storage::ImportJob, control: Option<&JobControl>) -> client::ImportJob {
    let plan = control.and_then(|control| control.plan.as_ref());
    let phase = job.phase.clone();
    let report = plan.map(convert_report);
    client::ImportJob {
        import_id: job.import_id,
        owner_user_id: job.owner_user_id,
        destination_dataset_id: job.destination_dataset_id,
        destination_name: job.destination_name,
        profile: client_profile(job.profile),
        transport: match job.transport {
            storage::ImportTransport::Browser => client::ImportTransport::BrowserFolder,
            storage::ImportTransport::ServerDirectory => client::ImportTransport::ServerDirectory,
        },
        lifecycle: client_phase(phase.clone()),
        progress: client::ImportProgress {
            phase: progress_phase(phase.clone()),
            registered_files: job.accepted_files as u64,
            uploaded_files: job.accepted_files as u64,
            total_files: job.accepted_files as u64,
            accepted_bytes: job.accepted_bytes,
            total_bytes: job.accepted_bytes,
            processed_images: plan.map_or(0, |plan| plan.totals.images as u64),
            total_images: plan.map_or(0, |plan| plan.totals.images as u64),
            processed_objects: plan.map_or(0, |plan| plan.totals.source_objects as u64),
            total_objects: plan.map_or(0, |plan| plan.totals.source_objects as u64),
        },
        failure: job.failure_code.map(|code| client::ImportFailure {
            safe_summary: format!("import failed ({code})"),
            code,
            phase: progress_phase(phase.clone()),
            retryable: false,
        }),
        source_fingerprint: job.source_fingerprint,
        plan_hash: job.plan_hash,
        preflight_report: report,
        can_cancel: !matches!(
            phase,
            storage::ImportJobPhase::Committing
                | storage::ImportJobPhase::Succeeded
                | storage::ImportJobPhase::Cancelled
                | storage::ImportJobPhase::Expired
        ),
        created_at: job.created_at,
        updated_at: job.updated_at,
        expires_at: None,
        recovery: control.map(|control| client::ImportRecoveryState {
            attestations: control.create_request.attestations.clone(),
            server_root_id: match &control.create_request.source {
                client::ImportSourceSelection::ServerDirectory { import_root_id, .. } => {
                    Some(import_root_id.clone())
                }
                client::ImportSourceSelection::BrowserFolder => None,
            },
            source: control
                .seal_request
                .as_ref()
                .map(|seal| safe_source_configuration(control, &seal.source)),
            registered_files: control
                .files
                .iter()
                .map(|(file_id, file)| client::RegisteredImportFile {
                    client_file_id: file.client_file_id.clone().unwrap_or_default(),
                    file_id: file_id.clone(),
                    byte_size: file.byte_size,
                    accepted_bytes: file.accepted_bytes,
                    complete: file.complete,
                })
                .collect(),
            accepted_plan: plan
                .map(|plan| convert_plan(plan, control.accepted_plan_request.as_ref())),
        }),
    }
}

fn safe_source_configuration(
    control: &JobControl,
    source: &client::ImportSourceConfiguration,
) -> client::ImportSourceConfiguration {
    let opaque_id = |reference: &str| {
        control
            .files
            .iter()
            .find(|(file_id, file)| {
                file_id.as_str() == reference
                    || file.client_file_id.as_deref() == Some(reference)
                    || file.relative_path == reference
            })
            .map(|(file_id, _)| file_id.clone())
            .unwrap_or_else(|| reference.to_string())
    };
    client::ImportSourceConfiguration {
        source_namespace: source.source_namespace.clone(),
        descriptors: source
            .descriptors
            .iter()
            .map(|descriptor| client::ImportDescriptorSelection {
                descriptor_file_id: opaque_id(&descriptor.descriptor_file_id),
                kind: descriptor.kind,
                release: descriptor.release.clone(),
                split: descriptor.split.clone(),
                image_root_file_id: descriptor.image_root_file_id.as_deref().map(&opaque_id),
                pairing_group: descriptor.pairing_group.clone(),
            })
            .collect(),
        selected_splits: source.selected_splits.clone(),
        selected_category_keys: source.selected_category_keys.clone(),
    }
}

fn convert_capabilities(state: &ApiState, actor: &UserId) -> client::ImportCapabilities {
    let Some(service) = state.import_service() else {
        return client::ImportCapabilities {
            available: false,
            unavailable_reason: Some("dataset import is not configured".to_string()),
            ..Default::default()
        };
    };
    let capabilities = service.capabilities();
    let available = capabilities.available
        && capabilities.atomic_publication
        && capabilities.secure_server_open
        && capabilities.browser_upload
        && !capabilities.profiles.is_empty();
    let unavailable_reason = if available {
        None
    } else {
        capabilities.unavailable_reason.clone()
    };
    let visible_roots = state.visible_import_roots(actor);
    let server_roots = capabilities
        .server_directory_roots
        .iter()
        .filter(|root_id| visible_roots.contains(root_id.as_str()))
        .map(|root_id| client::ServerImportRoot {
            root_id: root_id.clone(),
            display_name: root_id.clone(),
        })
        .collect::<Vec<_>>();
    client::ImportCapabilities {
        available,
        unavailable_reason,
        profiles: storage::ImportProfile::ALL
            .into_iter()
            .map(|profile| client::ImportProfileCapability {
                profile: client_profile(profile),
                enabled: available && capabilities.profiles.contains(&profile),
                display_name: match profile {
                    storage::ImportProfile::UltralyticsYoloDetectV1 => "Ultralytics YOLO detection",
                    storage::ImportProfile::UltralyticsYoloPoseV1 => "Ultralytics YOLO pose",
                    storage::ImportProfile::CocoInstancesGtV1 => "COCO instances ground truth",
                    storage::ImportProfile::CocoKeypointsGtV1 => "COCO keypoints ground truth",
                }
                .to_string(),
                profile_version: 1,
            })
            .collect(),
        transports: vec![
            client::ImportTransportCapability {
                transport: client::ImportTransport::BrowserFolder,
                enabled: available && capabilities.browser_upload,
                resumable: true,
            },
            client::ImportTransportCapability {
                transport: client::ImportTransport::ServerDirectory,
                enabled: available && !server_roots.is_empty(),
                resumable: false,
            },
        ],
        server_roots,
        limits: client::ImportLimits {
            max_browser_files: capabilities.limits.browser_source_files as u64,
            max_browser_bytes: capabilities.limits.browser_source_bytes,
            max_server_files: capabilities.limits.server_source_files as u64,
            max_source_bytes: capabilities.limits.total_source_bytes,
            max_selected_images: capabilities.limits.selected_images as u64,
            max_single_file_bytes: capabilities.limits.single_source_file_bytes,
            upload_chunk_bytes: capabilities.limits.upload_chunk_bytes as u64,
            max_selected_categories: capabilities.limits.selected_categories as u32,
            max_generated_tasks: capabilities.limits.selected_tasks as u32,
            max_annotations: capabilities.limits.annotations_total as u64,
            max_annotations_per_image: capabilities.limits.annotations_per_image as u32,
            max_keypoints_per_skeleton: capabilities.limits.keypoints_per_skeleton as u32,
            max_diagnostic_page_size: 100,
        },
        schema_version: capabilities.schema_version,
        parser_version: capabilities.parser_version.clone(),
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        manual_box_guide_migration: available,
    }
}

fn convert_preflight(
    job: &storage::ImportJob,
    control: &JobControl,
    seal: &client::SealImportRequest,
) -> ApiResult<storage::PreflightRequest> {
    if !seal.source.selected_category_keys.is_empty() {
        return Err(ApiError::Unprocessable(
            "selectedCategoryKeys is not supported; select categories in the import plan"
                .to_string(),
        ));
    }
    if seal.attestations != control.create_request.attestations {
        return Err(ApiError::Unprocessable(
            "seal attestations must match the import creation attestations".to_string(),
        ));
    }
    validate_identity_component(&seal.source.source_namespace, "source namespace")?;
    if seal.source.selected_splits.is_empty()
        || seal
            .source
            .selected_splits
            .iter()
            .any(|split| validate_identity_component(split, "selected split").is_err())
        || seal
            .source
            .selected_splits
            .iter()
            .collect::<BTreeSet<_>>()
            .len()
            != seal.source.selected_splits.len()
    {
        return Err(ApiError::Unprocessable(
            "selected splits must be unique nonempty identity components".to_string(),
        ));
    }
    let mut descriptor_paths = Vec::new();
    let mut coco_descriptors = Vec::new();
    let mut descriptor_identities = BTreeSet::new();
    for descriptor in &seal.source.descriptors {
        let path = resolve_source_reference(control, &descriptor.descriptor_file_id)?;
        validate_identity_component(&descriptor.release, "descriptor release")?;
        validate_identity_component(&descriptor.split, "descriptor split")?;
        if !seal.source.selected_splits.contains(&descriptor.split) {
            return Err(ApiError::Unprocessable(
                "every descriptor split must be selected".to_string(),
            ));
        }
        if descriptor
            .pairing_group
            .as_deref()
            .is_some_and(|value| validate_identity_component(value, "pairing group").is_err())
        {
            return Err(ApiError::Unprocessable(
                "pairing group must be a nonempty identity component".to_string(),
            ));
        }
        if !descriptor_identities.insert((
            path.clone(),
            descriptor_kind_name(descriptor.kind),
            descriptor.release.clone(),
            descriptor.split.clone(),
            descriptor.pairing_group.clone(),
        )) {
            return Err(ApiError::Unprocessable(
                "descriptor identities must be unique".to_string(),
            ));
        }
        let kind_allowed = match job.profile {
            storage::ImportProfile::UltralyticsYoloDetectV1
            | storage::ImportProfile::UltralyticsYoloPoseV1 => {
                descriptor.kind == client::ImportDescriptorKind::YoloDataset
            }
            storage::ImportProfile::CocoInstancesGtV1 => {
                descriptor.kind == client::ImportDescriptorKind::CocoInstances
            }
            storage::ImportProfile::CocoKeypointsGtV1 => matches!(
                descriptor.kind,
                client::ImportDescriptorKind::CocoInstances
                    | client::ImportDescriptorKind::CocoKeypoints
            ),
        };
        if !kind_allowed {
            return Err(ApiError::Unprocessable(
                "descriptor kind does not match the selected import profile".to_string(),
            ));
        }
        match descriptor.kind {
            client::ImportDescriptorKind::YoloDataset => {
                if descriptor.image_root_file_id.is_some() || descriptor.pairing_group.is_some() {
                    return Err(ApiError::Unprocessable(
                        "YOLO descriptors do not support image-root or pairing inputs".to_string(),
                    ));
                }
                descriptor_paths.push(path);
            }
            client::ImportDescriptorKind::CocoInstances
            | client::ImportDescriptorKind::CocoKeypoints => {
                let image_reference =
                    descriptor.image_root_file_id.as_deref().ok_or_else(|| {
                        ApiError::Unprocessable(
                            "COCO descriptors require an explicit registered image-root reference"
                                .to_string(),
                        )
                    })?;
                let image_path = resolve_source_reference(control, image_reference)?;
                let image_root = Path::new(&image_path)
                    .parent()
                    .and_then(Path::to_str)
                    .filter(|parent| !parent.is_empty())
                    .unwrap_or(&image_path)
                    .replace('\\', "/");
                coco_descriptors.push(storage::CocoDescriptorSelection {
                    kind: match descriptor.kind {
                        client::ImportDescriptorKind::CocoInstances => {
                            labello_domain::ImportDescriptorKind::CocoInstances
                        }
                        client::ImportDescriptorKind::CocoKeypoints => {
                            labello_domain::ImportDescriptorKind::CocoKeypoints
                        }
                        client::ImportDescriptorKind::YoloDataset => unreachable!(),
                    },
                    descriptor_path: path,
                    image_root,
                    split: descriptor.split.clone(),
                    source_namespace: seal.source.source_namespace.clone(),
                    release: descriptor.release.clone(),
                    pairing_group: descriptor.pairing_group.clone(),
                });
            }
        }
    }
    if matches!(
        job.profile,
        storage::ImportProfile::UltralyticsYoloDetectV1
            | storage::ImportProfile::UltralyticsYoloPoseV1
    ) && descriptor_paths.len() != 1
    {
        return Err(ApiError::Unprocessable(
            "YOLO imports require exactly one descriptor".to_string(),
        ));
    }
    let source_release = seal
        .source
        .descriptors
        .first()
        .map(|descriptor| descriptor.release.clone())
        .unwrap_or_default();
    Ok(storage::PreflightRequest {
        descriptor_paths,
        selected_splits: seal.source.selected_splits.clone(),
        coco_descriptors,
        ground_truth_attested: seal.attestations.ground_truth,
        exhaustive_attested: seal.attestations.exhaustive,
        source_namespace: seal.source.source_namespace.clone(),
        source_release,
        coverage_scope: seal.attestations.coverage_scope.clone(),
        attestation_provenance: seal.attestations.provenance.clone(),
        intent: if seal.attestations.exhaustive {
            storage::ImportIntent::AuthoritativeGroundTruth
        } else {
            storage::ImportIntent::RequireApproval
        },
        policies: storage::CompatibilityPolicies::default(),
        output: storage::OutputPolicy::defaults_for(job.profile),
        acknowledged_warning_codes: Vec::new(),
        category_mappings: Vec::new(),
        task_mappings: Vec::new(),
        geometry_mappings: Vec::new(),
    })
}

fn convert_plan_update(
    mut current: storage::PreflightRequest,
    request: client::UpdateImportPlanRequest,
) -> ApiResult<storage::PreflightRequest> {
    if request.category_mappings.is_empty() || request.task_mappings.is_empty() {
        return Err(ApiError::Unprocessable(
            "at least one category and task mapping is required".to_string(),
        ));
    }
    let category_keys = request
        .category_mappings
        .iter()
        .map(|mapping| mapping.source_category_key.clone())
        .collect::<BTreeSet<_>>();
    let selected = request
        .category_mappings
        .iter()
        .filter(|mapping| mapping.selected)
        .map(|mapping| mapping.source_category_key.clone())
        .collect::<BTreeSet<_>>();
    if selected.is_empty() || category_keys.len() != request.category_mappings.len() {
        return Err(ApiError::Unprocessable(
            "source category keys must be unique and at least one must be selected".to_string(),
        ));
    }
    let mut class_ids = BTreeSet::new();
    for mapping in &request.category_mappings {
        if mapping.source_category_key.trim().is_empty()
            || mapping.source_category_id.trim().is_empty()
            || mapping.class_name.trim().is_empty()
            || !valid_color(&mapping.color)
            || mapping.class_id.validate_path_segment().is_err()
            || (mapping.selected && !class_ids.insert(mapping.class_id.clone()))
        {
            return Err(ApiError::Unprocessable(
                "category mappings require valid unique source keys and selected class IDs"
                    .to_string(),
            ));
        }
    }

    let mut skeletons = BTreeMap::new();
    let mut skeleton_categories = BTreeSet::new();
    for mapping in &request.skeleton_mappings {
        if !selected.contains(&mapping.source_category_key)
            || !mapping.names_confirmed
            || !skeleton_categories.insert(mapping.source_category_key.clone())
            || skeletons
                .insert(mapping.target_task_id.clone(), mapping)
                .is_some()
        {
            return Err(ApiError::Unprocessable(
                "skeleton mappings must uniquely target selected categories and tasks".to_string(),
            ));
        }
        validate_skeleton(&mapping.skeleton)?;
        let source_names = mapping
            .source_keypoint_names
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if source_names.len() != mapping.source_keypoint_names.len()
            || source_names.iter().any(|name| name.trim().is_empty())
        {
            return Err(ApiError::Unprocessable(
                "source keypoint names must be unique and nonempty".to_string(),
            ));
        }
    }
    let mut first_intent = None;
    let mut task_ids = BTreeSet::new();
    let mut category_task_types = BTreeSet::new();
    let mut task_mappings = Vec::new();
    for mapping in &request.task_mappings {
        if !selected.contains(&mapping.source_category_key)
            || !task_ids.insert(mapping.task.task_id.clone())
            || !category_task_types.insert((
                mapping.source_category_key.clone(),
                geometry_kind(mapping.task.annotation_type.clone()),
            ))
        {
            return Err(ApiError::Unprocessable(
                "task mappings must have unique IDs and types for selected source categories"
                    .to_string(),
            ));
        }
        let category = request
            .category_mappings
            .iter()
            .find(|category| category.source_category_key == mapping.source_category_key)
            .expect("selected category was validated");
        if mapping.task.task_id.validate_path_segment().is_err()
            || mapping.task.name.trim().is_empty()
            || mapping.task.instructions.title.trim().is_empty()
            || mapping.task.instructions.example_text.trim().is_empty()
            || mapping.task.class_ids != [category.class_id.clone()]
            || !mapping.task.enabled
            || !mapping.task.prelabel_config_ids.is_empty()
        {
            return Err(ApiError::Unprocessable(
                "mapped tasks must be enabled, use exactly the mapped class, and have no prelabels"
                    .to_string(),
            ));
        }
        let task = mapping.task.clone();
        if let Some(skeleton) = skeletons.get(&task.task_id) {
            if task.annotation_type != labello_domain::AnnotationType::Skeleton
                || skeleton.source_category_key != mapping.source_category_key
                || task.skeleton.as_ref() != Some(&skeleton.skeleton)
            {
                return Err(ApiError::Unprocessable(
                    "skeleton mappings must exactly match their skeleton task and category"
                        .to_string(),
                ));
            }
        } else if task.annotation_type == labello_domain::AnnotationType::Skeleton {
            return Err(ApiError::Unprocessable(
                "every skeleton task requires an explicit confirmed skeleton mapping".to_string(),
            ));
        } else if task.skeleton.is_some() || task.manual_box_guide_migration.is_some() {
            return Err(ApiError::Unprocessable(
                "bounding-box tasks cannot contain skeleton or manual-guide configuration"
                    .to_string(),
            ));
        }
        validate_workflow(&task, mapping.workflow_intent)?;
        let intent = match mapping.workflow_intent {
            client::ImportWorkflowIntent::AuthoritativeGroundTruth => {
                storage::ImportIntent::AuthoritativeGroundTruth
            }
            client::ImportWorkflowIntent::RequireApproval => storage::ImportIntent::RequireApproval,
            client::ImportWorkflowIntent::SeedFutureAnnotation => {
                storage::ImportIntent::SeedFutureAnnotation
            }
        };
        first_intent.get_or_insert(intent);
        task_mappings.push(storage::ImportTaskMapping {
            source_category_key: mapping.source_category_key.clone(),
            task,
            intent,
        });
    }
    if skeletons.len()
        != task_mappings
            .iter()
            .filter(|mapping| {
                mapping.task.annotation_type == labello_domain::AnnotationType::Skeleton
            })
            .count()
    {
        return Err(ApiError::Unprocessable(
            "orphan skeleton mappings are not supported".to_string(),
        ));
    }
    current.intent = first_intent.unwrap_or(current.intent);
    current.category_mappings = request
        .category_mappings
        .iter()
        .map(|mapping| storage::ImportCategoryMapping {
            source_category_key: mapping.source_category_key.clone(),
            source_category_id: mapping.source_category_id.clone(),
            class_id: mapping.class_id.clone(),
            class_name: mapping.class_name.clone(),
            color: mapping.color.clone(),
            selected: mapping.selected,
        })
        .collect();
    current.task_mappings = task_mappings.clone();

    let mut bounding_boxes = false;
    let mut skeleton_output = false;
    let mut manual_schemas = Vec::new();
    let mut geometry_targets = BTreeSet::new();
    let mut manual_categories = Vec::new();
    let mut geometry_mappings = Vec::new();
    for mapping in &request.geometry_mappings {
        if !selected.contains(&mapping.source_category_key) {
            return Err(ApiError::Unprocessable(
                "geometry mappings may only reference selected categories".to_string(),
            ));
        }
        if !geometry_targets.insert((mapping.source_category_key.clone(), mapping.target_geometry))
        {
            return Err(ApiError::Unprocessable(
                "geometry mapping targets must be unique".to_string(),
            ));
        }
        let matching_task = task_mappings.iter().find(|task| {
            task.source_category_key == mapping.source_category_key
                && geometry_kind(task.task.annotation_type.clone()) == mapping.target_geometry
        });
        let policy = match mapping.policy {
            client::ImportGeometryPolicy::Direct => {
                if !mapping.parameters.is_empty()
                    || mapping.source_geometry != mapping.target_geometry
                    || matching_task.is_none()
                {
                    return Err(ApiError::Unprocessable(
                        "direct geometry must keep its type and target a matching task".to_string(),
                    ));
                }
                match mapping.target_geometry {
                    client::ImportGeometryKind::BoundingBox => bounding_boxes = true,
                    client::ImportGeometryKind::Skeleton => {
                        let skeleton = request
                            .skeleton_mappings
                            .iter()
                            .find(|skeleton| {
                                skeleton.source_category_key == mapping.source_category_key
                            })
                            .expect("skeleton task mapping was validated");
                        let target_names = skeleton
                            .skeleton
                            .keypoints
                            .iter()
                            .map(|point| &point.name)
                            .collect::<Vec<_>>();
                        if skeleton.source_keypoint_names.iter().collect::<Vec<_>>() != target_names
                        {
                            return Err(ApiError::Unprocessable(
                                "direct skeleton mappings cannot rename or reorder source keypoints"
                                    .to_string(),
                            ));
                        }
                        skeleton_output = true;
                    }
                }
                labello_domain::ImportGeometryPolicy::Direct
            }
            client::ImportGeometryPolicy::KeypointEnvelopeV1 => {
                if mapping.source_geometry != client::ImportGeometryKind::Skeleton
                    || mapping.target_geometry != client::ImportGeometryKind::BoundingBox
                    || matching_task.is_none()
                {
                    return Err(ApiError::Unprocessable(
                        "keypoint-envelope geometry requires a skeleton source and bounding-box task"
                            .to_string(),
                    ));
                }
                let (padding_ratio, minimum_pixels, include_hidden) =
                    envelope_parameters(&mapping.parameters)?;
                bounding_boxes = true;
                labello_domain::ImportGeometryPolicy::KeypointEnvelopeV1 {
                    padding_ratio,
                    minimum_pixels,
                    include_hidden,
                }
            }
            client::ImportGeometryPolicy::BoxRelativeTemplateV1 => {
                if mapping.source_geometry != client::ImportGeometryKind::BoundingBox
                    || mapping.target_geometry != client::ImportGeometryKind::Skeleton
                    || matching_task.is_none()
                {
                    return Err(ApiError::Unprocessable(
                        "box-relative templates require a bounding-box source and skeleton task"
                            .to_string(),
                    ));
                }
                let skeleton = request
                    .skeleton_mappings
                    .iter()
                    .find(|skeleton| skeleton.source_category_key == mapping.source_category_key)
                    .ok_or_else(|| {
                        ApiError::Unprocessable(
                            "box-relative templates require a confirmed skeleton mapping"
                                .to_string(),
                        )
                    })?;
                if !skeleton.source_keypoint_names.is_empty() {
                    return Err(ApiError::Unprocessable(
                        "box-relative templates cannot declare source keypoint names".to_string(),
                    ));
                }
                skeleton_output = true;
                labello_domain::ImportGeometryPolicy::BoxRelativeTemplateV1 {
                    keypoints: template_parameters(&mapping.parameters, &skeleton.skeleton)?,
                }
            }
            client::ImportGeometryPolicy::ManualBoxGuideV1 => {
                if !mapping.parameters.is_empty()
                    || mapping.source_geometry != client::ImportGeometryKind::BoundingBox
                    || mapping.target_geometry != client::ImportGeometryKind::Skeleton
                    || matching_task.is_none()
                {
                    return Err(ApiError::Unprocessable(
                        "manual box-guide requires a box-to-skeleton category and task".to_string(),
                    ));
                }
                manual_categories.push(mapping.source_category_key.clone());
                bounding_boxes = true;
                skeleton_output = true;
                let skeleton = request
                    .skeleton_mappings
                    .iter()
                    .find(|skeleton| skeleton.source_category_key == mapping.source_category_key)
                    .ok_or_else(|| {
                        ApiError::Unprocessable(
                            "manual box-guide mapping requires a skeleton mapping".to_string(),
                        )
                    })?;
                if !skeleton.source_keypoint_names.is_empty() {
                    return Err(ApiError::Unprocessable(
                        "manual box-guide mappings cannot declare source keypoint names"
                            .to_string(),
                    ));
                }
                manual_schemas.push(storage::BoxToSkeletonPolicy::ManualBoxGuide {
                    keypoint_names: skeleton
                        .skeleton
                        .keypoints
                        .iter()
                        .map(|point| point.name.clone())
                        .collect(),
                    edges: skeleton
                        .skeleton
                        .edges
                        .iter()
                        .map(|edge| (edge.from.clone(), edge.to.clone()))
                        .collect(),
                });
                labello_domain::ImportGeometryPolicy::ManualBoxGuideV1
            }
            client::ImportGeometryPolicy::Omit => {
                if !mapping.parameters.is_empty() || matching_task.is_some() {
                    return Err(ApiError::Unprocessable(
                        "omitted geometry cannot have a matching task mapping".to_string(),
                    ));
                }
                labello_domain::ImportGeometryPolicy::Omit
            }
        };
        geometry_mappings.push(labello_domain::ImportGeometryMapping {
            source_category_key: mapping.source_category_key.clone(),
            source_geometry: domain_geometry_kind(mapping.source_geometry),
            target_geometry: domain_geometry_kind(mapping.target_geometry),
            policy,
        });
    }
    for task in &task_mappings {
        if !geometry_targets.contains(&(
            task.source_category_key.clone(),
            geometry_kind(task.task.annotation_type.clone()),
        )) {
            return Err(ApiError::Unprocessable(
                "every mapped task requires one matching geometry mapping".to_string(),
            ));
        }
    }
    for category_key in manual_categories {
        let skeleton_task = task_mappings
            .iter()
            .find(|mapping| {
                mapping.source_category_key == category_key
                    && mapping.task.annotation_type == labello_domain::AnnotationType::Skeleton
            })
            .expect("manual geometry task was validated");
        let guide = task_mappings
            .iter()
            .find(|mapping| {
                mapping.source_category_key == category_key
                    && mapping.task.annotation_type == labello_domain::AnnotationType::BoundingBox
            })
            .ok_or_else(|| {
                ApiError::Unprocessable(
                    "manual box-guide migration requires a direct bounding-box guide task"
                        .to_string(),
                )
            })?;
        skeleton_task
            .task
            .validate_manual_migration(&guide.task)
            .map_err(|_| {
                ApiError::Unprocessable(
                    "manual box-guide task and guide configuration are inconsistent".to_string(),
                )
            })?;
    }
    // The legacy output summary can represent only one schema. Explicit geometry and
    // task mappings remain authoritative when categories use different schemas.
    let box_to_skeleton = manual_schemas
        .first()
        .filter(|schema| manual_schemas.iter().all(|candidate| candidate == *schema))
        .cloned()
        .unwrap_or(storage::BoxToSkeletonPolicy::None);
    current.output = storage::OutputPolicy {
        bounding_boxes,
        skeletons: skeleton_output,
        box_to_skeleton,
    };
    current.geometry_mappings = geometry_mappings;
    current.policies = storage::CompatibilityPolicies {
        yolo_missing_labels: match request.compatibility.yolo_missing_labels {
            client::YoloMissingLabelPolicy::Block => storage::YoloMissingLabelPolicy::Block,
            client::YoloMissingLabelPolicy::Incomplete => {
                storage::YoloMissingLabelPolicy::RetainIncomplete
            }
            client::YoloMissingLabelPolicy::MissingIsBackground => {
                storage::YoloMissingLabelPolicy::MissingIsBackground
            }
        },
        yolo_duplicate_rows: match request.compatibility.yolo_duplicate_rows {
            client::YoloDuplicateRowPolicy::Block => storage::DuplicateRowPolicy::Block,
            client::YoloDuplicateRowPolicy::Deduplicate => storage::DuplicateRowPolicy::Deduplicate,
        },
        coco_crowds: match request.compatibility.coco_crowds {
            client::CocoCrowdPolicy::Block => storage::CocoCrowdPolicy::Block,
            client::CocoCrowdPolicy::Incomplete => storage::CocoCrowdPolicy::Incomplete,
            client::CocoCrowdPolicy::ExcludeImageTask => storage::CocoCrowdPolicy::ExcludeImageTask,
        },
        coco_bbox_only: request.compatibility.coco_structure
            == client::CocoStructurePolicy::BboxCompatibility,
        geometry_bounds: match request.compatibility.geometry_bounds {
            client::GeometryBoundsPolicy::Reject => storage::GeometryBoundsPolicy::Block,
            client::GeometryBoundsPolicy::Clip => storage::GeometryBoundsPolicy::ClipDerived,
        },
        cross_split_duplicates: match request.compatibility.cross_split_duplicates {
            client::CrossSplitDuplicatePolicy::Block => storage::CrossSplitDuplicatePolicy::Block,
            client::CrossSplitDuplicatePolicy::MergeMemberships => {
                storage::CrossSplitDuplicatePolicy::MultipleMemberships
            }
        },
        yolo_keypoint_names: match request.compatibility.missing_keypoint_names {
            client::MissingKeypointNamesPolicy::Block => {
                storage::YoloKeypointNamePolicy::RequireSourceNames
            }
            client::MissingKeypointNamesPolicy::GenerateIndexed => {
                storage::YoloKeypointNamePolicy::GenerateIndexed
            }
        },
    };
    current.acknowledged_warning_codes = request
        .acknowledgements
        .into_iter()
        .filter(|acknowledgement| acknowledgement.acknowledged)
        .map(|acknowledgement| acknowledgement.diagnostic_code)
        .collect();
    current.acknowledged_warning_codes.sort();
    current.acknowledged_warning_codes.dedup();
    Ok(current)
}

fn validate_plan_update_against_current(
    current: &storage::ImportPlan,
    request: &client::UpdateImportPlanRequest,
) -> ApiResult<()> {
    let known_categories = if current.request.category_mappings.is_empty() {
        current.class_ids.keys().collect::<BTreeSet<_>>()
    } else {
        current
            .request
            .category_mappings
            .iter()
            .map(|mapping| &mapping.source_category_key)
            .collect()
    };
    if request.category_mappings.iter().any(|mapping| {
        !known_categories.contains(&mapping.source_category_key)
            || current
                .source_categories
                .get(&mapping.source_category_key)
                .is_none_or(|source| source.source_category_id != mapping.source_category_id)
    }) {
        return Err(ApiError::Unprocessable(
            "category mappings must preserve discovered source category keys and IDs".to_string(),
        ));
    }

    let mut acknowledged = BTreeSet::new();
    for acknowledgement in request
        .acknowledgements
        .iter()
        .filter(|acknowledgement| acknowledgement.acknowledged)
    {
        let diagnostic = current
            .diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.code == acknowledgement.diagnostic_code
                    && diagnostic.requires_acknowledgement
            })
            .ok_or_else(|| {
                ApiError::Unprocessable(
                    "acknowledgements must reference a current acknowledgement-required diagnostic"
                        .to_string(),
                )
            })?;
        if acknowledgement.policy.trim().is_empty()
            || acknowledgement.affected_count != diagnostic.count
            || !acknowledged.insert(&acknowledgement.diagnostic_code)
        {
            return Err(ApiError::Unprocessable(
                "acknowledgement policy, count, and diagnostic code must match the current plan"
                    .to_string(),
            ));
        }
    }
    Ok(())
}

fn valid_color(color: &str) -> bool {
    color.len() == 7
        && color.starts_with('#')
        && color[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn domain_geometry_kind(kind: client::ImportGeometryKind) -> labello_domain::ImportGeometryKind {
    match kind {
        client::ImportGeometryKind::BoundingBox => labello_domain::ImportGeometryKind::BoundingBox,
        client::ImportGeometryKind::Skeleton => labello_domain::ImportGeometryKind::Skeleton,
    }
}

fn envelope_parameters(
    parameters: &[client::ImportMappingParameter],
) -> ApiResult<(f64, u32, bool)> {
    let mut padding_ratio = None;
    let mut minimum_pixels = None;
    let mut include_hidden = None;
    for parameter in parameters {
        match parameter {
            client::ImportMappingParameter::Scalar { name, value }
                if matches!(name.as_str(), "padding" | "padding_ratio" | "paddingRatio")
                    && padding_ratio.replace(*value).is_none() => {}
            client::ImportMappingParameter::Scalar { name, value }
                if matches!(
                    name.as_str(),
                    "minimum_pixels" | "minimumPixels" | "min_pixels" | "minPixels"
                ) && minimum_pixels.replace(*value).is_none() => {}
            client::ImportMappingParameter::Boolean { name, value }
                if matches!(name.as_str(), "include_hidden" | "includeHidden" | "hidden")
                    && include_hidden.replace(*value).is_none() => {}
            _ => {
                return Err(ApiError::Unprocessable(
                    "keypoint-envelope parameters must contain one padding, minimum-pixels, and hidden value"
                        .to_string(),
                ));
            }
        }
    }
    let padding_ratio =
        padding_ratio.filter(|value| value.is_finite() && (0.0..=1.0).contains(value));
    let minimum_pixels = minimum_pixels.and_then(|value| {
        (value.is_finite() && value.fract() == 0.0 && value >= 1.0 && value <= f64::from(u32::MAX))
            .then_some(value as u32)
    });
    match (padding_ratio, minimum_pixels, include_hidden) {
        (Some(padding_ratio), Some(minimum_pixels), Some(include_hidden)) => {
            Ok((padding_ratio, minimum_pixels, include_hidden))
        }
        _ => Err(ApiError::Unprocessable(
            "keypoint-envelope parameters are missing or invalid".to_string(),
        )),
    }
}

fn template_parameters(
    parameters: &[client::ImportMappingParameter],
    skeleton: &labello_domain::SkeletonSpec,
) -> ApiResult<Vec<labello_domain::ImportTemplateKeypoint>> {
    if parameters.len() != skeleton.keypoints.len() || parameters.is_empty() {
        return Err(ApiError::Unprocessable(
            "box-relative templates must define every target skeleton keypoint exactly once"
                .to_string(),
        ));
    }
    let mut keypoints = Vec::with_capacity(parameters.len());
    for (parameter, spec) in parameters.iter().zip(&skeleton.keypoints) {
        let client::ImportMappingParameter::Point { name, x, y, state } = parameter else {
            return Err(ApiError::Unprocessable(
                "box-relative template parameters must be named points".to_string(),
            ));
        };
        if name != &spec.name
            || !x.is_finite()
            || !y.is_finite()
            || !(0.0..=1.0).contains(x)
            || !(0.0..=1.0).contains(y)
            || match state {
                labello_domain::KeypointState::Visible => false,
                labello_domain::KeypointState::Hidden => !skeleton.allow_hidden,
                labello_domain::KeypointState::Absent => !skeleton.allow_absent || spec.required,
            }
        {
            return Err(ApiError::Unprocessable(
                "box-relative template points must exactly match the schema and visibility policy"
                    .to_string(),
            ));
        }
        keypoints.push(labello_domain::ImportTemplateKeypoint {
            name: name.clone(),
            x: *x,
            y: *y,
            state: state.clone(),
        });
    }
    if keypoints
        .iter()
        .all(|point| point.state == labello_domain::KeypointState::Absent)
    {
        return Err(ApiError::Unprocessable(
            "box-relative templates cannot contain only absent keypoints".to_string(),
        ));
    }
    Ok(keypoints)
}

fn validate_identity_component(value: &str, name: &str) -> ApiResult<()> {
    if value.is_empty()
        || value.len() > 255
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(ApiError::Unprocessable(format!(
            "{name} must contain only ASCII letters, digits, '.', '_', or '-'"
        )));
    }
    Ok(())
}

fn descriptor_kind_name(kind: client::ImportDescriptorKind) -> &'static str {
    match kind {
        client::ImportDescriptorKind::YoloDataset => "yolo_dataset",
        client::ImportDescriptorKind::CocoInstances => "coco_instances",
        client::ImportDescriptorKind::CocoKeypoints => "coco_keypoints",
    }
}

fn geometry_kind(annotation_type: labello_domain::AnnotationType) -> client::ImportGeometryKind {
    match annotation_type {
        labello_domain::AnnotationType::BoundingBox => client::ImportGeometryKind::BoundingBox,
        labello_domain::AnnotationType::Skeleton => client::ImportGeometryKind::Skeleton,
    }
}

fn validate_skeleton(skeleton: &labello_domain::SkeletonSpec) -> ApiResult<()> {
    let names = skeleton
        .keypoints
        .iter()
        .map(|point| point.name.as_str())
        .collect::<BTreeSet<_>>();
    if names.is_empty()
        || names.len() != skeleton.keypoints.len()
        || names.iter().any(|name| name.trim().is_empty())
        || skeleton.edges.iter().any(|edge| {
            edge.from == edge.to
                || !names.contains(edge.from.as_str())
                || !names.contains(edge.to.as_str())
        })
    {
        return Err(ApiError::Unprocessable(
            "skeleton keypoints and edges must form a valid unique schema".to_string(),
        ));
    }
    Ok(())
}

fn validate_workflow(
    task: &labello_domain::TaskDefinition,
    intent: client::ImportWorkflowIntent,
) -> ApiResult<()> {
    use labello_domain::ReviewWorkflow;

    let review = &task.review;
    let structurally_valid = match &review.workflow {
        ReviewWorkflow::None => {
            review.required_reviews == 0
                && !review.allow_reviewer_corrections
                && review.agreement_threshold.is_none()
        }
        ReviewWorkflow::Approval => {
            review.required_reviews >= 1
                && !review.allow_reviewer_corrections
                && review.agreement_threshold.is_none()
        }
        ReviewWorkflow::IndependentAgreement => {
            review.required_reviews >= 2
                && !review.allow_reviewer_corrections
                && review
                    .agreement_threshold
                    .as_ref()
                    .is_some_and(|threshold| {
                        threshold.threshold.is_finite()
                            && (0.0..=1.0).contains(&threshold.threshold)
                    })
        }
    };
    let intent_valid = match intent {
        client::ImportWorkflowIntent::AuthoritativeGroundTruth => {
            review.workflow == ReviewWorkflow::None
        }
        client::ImportWorkflowIntent::RequireApproval => {
            review.workflow == ReviewWorkflow::Approval
        }
        client::ImportWorkflowIntent::SeedFutureAnnotation => {
            review.workflow != ReviewWorkflow::None
        }
    };
    if !structurally_valid || !intent_valid {
        return Err(ApiError::Unprocessable(
            "task review workflow is inconsistent with its import workflow intent".to_string(),
        ));
    }
    Ok(())
}

fn resolve_source_reference(control: &JobControl, reference: &str) -> ApiResult<String> {
    if let Some(file) = control.files.get(reference) {
        return Ok(file.relative_path.clone());
    }
    if let Some(file) = control
        .files
        .values()
        .find(|file| file.client_file_id.as_deref() == Some(reference))
    {
        return Ok(file.relative_path.clone());
    }
    if Path::new(reference).is_absolute()
        || reference.is_empty()
        || reference
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(ApiError::Unprocessable(
            "descriptor source reference is invalid".to_string(),
        ));
    }
    control
        .files
        .values()
        .find(|file| file.relative_path == reference)
        .map(|file| file.relative_path.clone())
        .ok_or_else(|| {
            ApiError::Unprocessable("descriptor source reference was not registered".to_string())
        })
}

fn client_severity(severity: storage::DiagnosticSeverity) -> client::ImportDiagnosticSeverity {
    match severity {
        storage::DiagnosticSeverity::Error => client::ImportDiagnosticSeverity::Error,
        storage::DiagnosticSeverity::WarningRequiresAck => {
            client::ImportDiagnosticSeverity::WarningRequiresAck
        }
        storage::DiagnosticSeverity::Warning => client::ImportDiagnosticSeverity::Warning,
        storage::DiagnosticSeverity::Info => client::ImportDiagnosticSeverity::Info,
    }
}

fn convert_report(plan: &storage::ImportPlan) -> client::ImportPreflightReport {
    let diagnostics = plan
        .diagnostics
        .iter()
        .map(|diagnostic| client::ImportDiagnosticSummary {
            code: diagnostic.code.clone(),
            severity: client_severity(diagnostic.severity),
            source_profile: client_profile(diagnostic.profile),
            count: diagnostic.count,
            safe_summary: diagnostic.summary.clone(),
            impact: client::ImportDiagnosticImpact {
                blocks_commit: diagnostic.blocks_commit,
                requires_acknowledgement: diagnostic.requires_acknowledgement,
                changes_coverage: diagnostic.changes_coverage,
                discards_metadata: false,
            },
            examples: diagnostic
                .examples
                .iter()
                .map(|example| client::ImportDiagnosticExample {
                    source: Some(convert_source_reference(example)),
                    safe_summary: diagnostic.summary.clone(),
                })
                .collect(),
        })
        .collect();
    client::ImportPreflightReport {
        source_fingerprint: plan.source_fingerprint.clone(),
        plan_hash: Some(plan.plan_hash.clone()),
        source: client::ImportSourceCounts {
            files: plan.totals.source_files as u64,
            bytes: plan.totals.source_bytes,
            descriptors: plan.totals.descriptors as u64,
            splits: plan.request.selected_splits.len() as u64,
            images: plan.totals.images as u64,
            categories: plan.totals.categories as u64,
            objects: plan.totals.source_objects as u64,
            keypoints: plan.totals.keypoints as u64,
        },
        geometry: {
            let source_direct = (plan.totals.direct_boxes + plan.totals.direct_skeletons) as u64;
            client::ImportGeometryCounts {
                direct: source_direct.saturating_sub(plan.totals.clipped_geometry as u64),
                clipped: plan.totals.clipped_geometry as u64,
                template_derived: plan.totals.template_derived as u64,
                envelope_derived: plan.totals.envelope_derived as u64,
                ..Default::default()
            }
        },
        coverage: {
            let boxes = &plan.coverage.bounding_boxes;
            let skeletons = &plan.coverage.skeletons;
            client::ImportCoverageCounts {
                complete: (boxes.complete + skeletons.complete) as u64,
                verified_empty: (boxes.verified_empty + skeletons.verified_empty) as u64,
                incomplete: (boxes.incomplete + skeletons.incomplete) as u64,
                excluded: (boxes.excluded + skeletons.excluded) as u64,
            }
        },
        coverage_by_geometry: client::ImportCoverageByGeometry {
            bounding_boxes: client_coverage_counts(&plan.coverage.bounding_boxes),
            skeletons: client_coverage_counts(&plan.coverage.skeletons),
        },
        output: client::ImportOutputEstimate {
            classes: plan.class_ids.len() as u64,
            tasks: plan.totals.output_tasks as u64,
            annotations: plan.totals.output_annotations as u64,
            events: plan.totals.images as u64,
            output_bytes: plan.totals.estimated_output_bytes,
            temporary_bytes: plan.totals.estimated_output_bytes,
            required_free_bytes: plan
                .totals
                .estimated_output_bytes
                .saturating_add(plan.totals.estimated_output_bytes / 10)
                .saturating_add(64 * 1024 * 1024),
        },
        blocking_diagnostics: plan
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.blocks_commit)
            .map(|diagnostic| diagnostic.count)
            .sum(),
        required_acknowledgements: plan
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.requires_acknowledgement)
            .map(|diagnostic| diagnostic.count)
            .sum(),
        diagnostics,
    }
}

fn client_coverage_counts(
    counts: &labello_domain::ImportCoverageCounts,
) -> client::ImportCoverageCounts {
    client::ImportCoverageCounts {
        complete: counts.complete as u64,
        verified_empty: counts.verified_empty as u64,
        incomplete: counts.incomplete as u64,
        excluded: counts.excluded as u64,
    }
}

fn convert_plan(
    plan: &storage::ImportPlan,
    accepted_request: Option<&client::UpdateImportPlanRequest>,
) -> client::ImportPlan {
    let generated_request = generated_plan_request(plan);
    let current_request = accepted_request
        .cloned()
        .unwrap_or_else(|| current_plan_request(plan, &generated_request));
    let source_categories = plan
        .source_categories
        .iter()
        .map(|(key, source)| {
            let generated_category_mapping = generated_request
                .category_mappings
                .iter()
                .find(|mapping| mapping.source_category_key == *key)
                .expect("generated category mapping")
                .clone();
            let current_category_mapping = current_request
                .category_mappings
                .iter()
                .find(|mapping| mapping.source_category_key == *key)
                .cloned()
                .unwrap_or_else(|| generated_category_mapping.clone());
            client::ImportSourceCategory {
                source_category_key: key.clone(),
                source_category_id: source.source_category_id.clone(),
                source_name: source.source_name.clone(),
                source_supercategory: source.source_supercategory.clone(),
                source_namespace: source.source_namespace.clone(),
                direct_geometry: [
                    source
                        .direct_bounding_boxes
                        .then_some(client::ImportGeometryKind::BoundingBox),
                    source
                        .direct_skeletons
                        .then_some(client::ImportGeometryKind::Skeleton),
                ]
                .into_iter()
                .flatten()
                .collect(),
                keypoint_schema: source_skeleton(source),
                generated_category_mapping,
                generated_task_mappings: generated_request
                    .task_mappings
                    .iter()
                    .filter(|mapping| mapping.source_category_key == *key)
                    .cloned()
                    .collect(),
                current_category_mapping,
                current_geometry_mappings: current_request
                    .geometry_mappings
                    .iter()
                    .filter(|mapping| mapping.source_category_key == *key)
                    .cloned()
                    .collect(),
                current_task_mappings: current_request
                    .task_mappings
                    .iter()
                    .filter(|mapping| mapping.source_category_key == *key)
                    .cloned()
                    .collect(),
                current_skeleton_mappings: current_request
                    .skeleton_mappings
                    .iter()
                    .filter(|mapping| mapping.source_category_key == *key)
                    .cloned()
                    .collect(),
            }
        })
        .collect();
    client::ImportPlan {
        import_id: plan.import_id.clone(),
        source_fingerprint: plan.source_fingerprint.clone(),
        plan_hash: plan.plan_hash.clone(),
        commit_ready: plan.committable(),
        blocking_diagnostic_codes: plan
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.blocks_commit)
            .map(|diagnostic| diagnostic.code.clone())
            .collect(),
        required_acknowledgement_codes: plan
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.requires_acknowledgement
                    && !plan
                        .request
                        .acknowledged_warning_codes
                        .contains(&diagnostic.code)
            })
            .map(|diagnostic| diagnostic.code.clone())
            .collect(),
        report: convert_report(plan),
        source_categories,
        accepted_request: Some(current_request),
    }
}

fn generated_plan_request(plan: &storage::ImportPlan) -> client::UpdateImportPlanRequest {
    let category_mappings = plan
        .source_categories
        .iter()
        .map(|(key, source)| client::ImportCategoryMappingRequest {
            source_category_key: key.clone(),
            source_category_id: source.source_category_id.clone(),
            class_id: labello_domain::ClassId::from(plan.class_ids[key].clone()),
            class_name: source.source_name.clone(),
            color: generated_color(&plan.class_ids[key]),
            selected: plan.class_ids.contains_key(key),
        })
        .collect::<Vec<_>>();
    let task_mappings = plan
        .task_ids
        .iter()
        .flat_map(|(key, task_ids)| {
            task_ids
                .iter()
                .map(move |task_id| client::ImportTaskMappingRequest {
                    source_category_key: key.clone(),
                    task: generated_task(plan, key, task_id),
                    workflow_intent: client_intent(plan.request.intent),
                })
        })
        .collect::<Vec<_>>();
    let geometry_mappings = task_mappings
        .iter()
        .map(|mapping| {
            let kind = geometry_kind(mapping.task.annotation_type.clone());
            client::ImportGeometryMappingRequest {
                source_category_key: mapping.source_category_key.clone(),
                source_geometry: kind,
                target_geometry: kind,
                policy: client::ImportGeometryPolicy::Direct,
                parameters: Vec::new(),
            }
        })
        .collect();
    let skeleton_mappings = task_mappings
        .iter()
        .filter_map(|mapping| {
            let skeleton = mapping.task.skeleton.clone()?;
            Some(client::ImportSkeletonMappingRequest {
                source_category_key: mapping.source_category_key.clone(),
                target_task_id: mapping.task.task_id.clone(),
                source_keypoint_names: plan.source_categories[&mapping.source_category_key]
                    .keypoint_names
                    .clone(),
                skeleton,
                names_confirmed: true,
            })
        })
        .collect();
    client::UpdateImportPlanRequest {
        category_mappings,
        geometry_mappings,
        task_mappings,
        skeleton_mappings,
        compatibility: client_compatibility(&plan.request.policies),
        acknowledgements: Vec::new(),
    }
}

fn current_plan_request(
    plan: &storage::ImportPlan,
    generated: &client::UpdateImportPlanRequest,
) -> client::UpdateImportPlanRequest {
    if plan.request.category_mappings.is_empty() {
        return generated.clone();
    }
    let mut request = generated.clone();
    request.category_mappings = plan
        .request
        .category_mappings
        .iter()
        .map(|mapping| client::ImportCategoryMappingRequest {
            source_category_key: mapping.source_category_key.clone(),
            source_category_id: mapping.source_category_id.clone(),
            class_id: mapping.class_id.clone(),
            class_name: mapping.class_name.clone(),
            color: mapping.color.clone(),
            selected: mapping.selected,
        })
        .collect();
    request.task_mappings = plan
        .request
        .task_mappings
        .iter()
        .map(|mapping| client::ImportTaskMappingRequest {
            source_category_key: mapping.source_category_key.clone(),
            task: mapping.task.clone(),
            workflow_intent: client_intent(mapping.intent),
        })
        .collect();
    request.geometry_mappings = plan
        .request
        .geometry_mappings
        .iter()
        .map(client_geometry_mapping)
        .collect();
    request.skeleton_mappings = request
        .task_mappings
        .iter()
        .filter_map(|mapping| {
            let skeleton = mapping.task.skeleton.clone()?;
            let source_names = if request.geometry_mappings.iter().any(|geometry| {
                geometry.source_category_key == mapping.source_category_key
                    && geometry.policy == client::ImportGeometryPolicy::Direct
                    && geometry.target_geometry == client::ImportGeometryKind::Skeleton
            }) {
                plan.source_categories[&mapping.source_category_key]
                    .keypoint_names
                    .clone()
            } else {
                Vec::new()
            };
            Some(client::ImportSkeletonMappingRequest {
                source_category_key: mapping.source_category_key.clone(),
                target_task_id: mapping.task.task_id.clone(),
                skeleton,
                source_keypoint_names: source_names,
                names_confirmed: true,
            })
        })
        .collect();
    request.compatibility = client_compatibility(&plan.request.policies);
    request.acknowledgements = plan
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            plan.request
                .acknowledged_warning_codes
                .contains(&diagnostic.code)
        })
        .map(|diagnostic| client::ImportAcknowledgementRequest {
            diagnostic_code: diagnostic.code.clone(),
            policy: "accepted".to_string(),
            affected_count: diagnostic.count,
            acknowledged: true,
        })
        .collect();
    request
}

fn generated_task(
    plan: &storage::ImportPlan,
    key: &str,
    task_id: &str,
) -> labello_domain::TaskDefinition {
    let source = &plan.source_categories[key];
    let annotation_type = if task_id.starts_with("bounding_box:") {
        labello_domain::AnnotationType::BoundingBox
    } else {
        labello_domain::AnnotationType::Skeleton
    };
    let skeleton = (annotation_type == labello_domain::AnnotationType::Skeleton)
        .then(|| source_skeleton(source))
        .flatten();
    labello_domain::TaskDefinition {
        task_id: labello_domain::TaskId::from(task_id),
        name: format!(
            "{} {}",
            source.source_name,
            if annotation_type == labello_domain::AnnotationType::BoundingBox {
                "boxes"
            } else {
                "skeletons"
            }
        ),
        annotation_type,
        class_ids: vec![labello_domain::ClassId::from(plan.class_ids[key].clone())],
        instructions: labello_domain::TutorialContent {
            title: format!("Annotate {}", source.source_name),
            example_text:
                "Imported source geometry and coverage are recorded in the audit history."
                    .to_string(),
            example_images: Vec::new(),
        },
        skeleton,
        review: review_for_intent(plan.request.intent),
        prelabel_config_ids: Vec::new(),
        manual_box_guide_migration: None,
        enabled: true,
    }
}

fn source_skeleton(source: &storage::ImportSourceCategory) -> Option<labello_domain::SkeletonSpec> {
    (!source.keypoint_names.is_empty()).then(|| labello_domain::SkeletonSpec {
        keypoints: source
            .keypoint_names
            .iter()
            .map(|name| labello_domain::KeypointSpec {
                name: name.clone(),
                required: false,
            })
            .collect(),
        edges: source
            .edges
            .iter()
            .map(|(from, to)| labello_domain::SkeletonEdge {
                from: from.clone(),
                to: to.clone(),
            })
            .collect(),
        allow_hidden: source.allow_hidden,
        allow_absent: true,
    })
}

fn generated_color(class_id: &str) -> String {
    let digest = blake3::hash(class_id.as_bytes()).to_hex().to_string();
    format!("#{}", &digest[..6])
}

fn client_intent(intent: storage::ImportIntent) -> client::ImportWorkflowIntent {
    match intent {
        storage::ImportIntent::AuthoritativeGroundTruth => {
            client::ImportWorkflowIntent::AuthoritativeGroundTruth
        }
        storage::ImportIntent::RequireApproval => client::ImportWorkflowIntent::RequireApproval,
        storage::ImportIntent::SeedFutureAnnotation => {
            client::ImportWorkflowIntent::SeedFutureAnnotation
        }
    }
}

fn review_for_intent(intent: storage::ImportIntent) -> labello_domain::ReviewConfig {
    match intent {
        storage::ImportIntent::AuthoritativeGroundTruth => labello_domain::ReviewConfig {
            required_reviews: 0,
            workflow: labello_domain::ReviewWorkflow::None,
            allow_reviewer_corrections: false,
            agreement_threshold: None,
        },
        storage::ImportIntent::RequireApproval | storage::ImportIntent::SeedFutureAnnotation => {
            labello_domain::ReviewConfig {
                required_reviews: 1,
                workflow: labello_domain::ReviewWorkflow::Approval,
                allow_reviewer_corrections: false,
                agreement_threshold: None,
            }
        }
    }
}

fn client_compatibility(
    policies: &storage::CompatibilityPolicies,
) -> client::ImportCompatibilityPolicies {
    client::ImportCompatibilityPolicies {
        yolo_missing_labels: match policies.yolo_missing_labels {
            storage::YoloMissingLabelPolicy::Block => client::YoloMissingLabelPolicy::Block,
            storage::YoloMissingLabelPolicy::MissingIsBackground => {
                client::YoloMissingLabelPolicy::MissingIsBackground
            }
            storage::YoloMissingLabelPolicy::RetainIncomplete => {
                client::YoloMissingLabelPolicy::Incomplete
            }
        },
        yolo_duplicate_rows: match policies.yolo_duplicate_rows {
            storage::DuplicateRowPolicy::Block => client::YoloDuplicateRowPolicy::Block,
            storage::DuplicateRowPolicy::Deduplicate => client::YoloDuplicateRowPolicy::Deduplicate,
        },
        coco_crowds: match policies.coco_crowds {
            storage::CocoCrowdPolicy::Block => client::CocoCrowdPolicy::Block,
            storage::CocoCrowdPolicy::Incomplete => client::CocoCrowdPolicy::Incomplete,
            storage::CocoCrowdPolicy::ExcludeImageTask => client::CocoCrowdPolicy::ExcludeImageTask,
        },
        coco_structure: if policies.coco_bbox_only {
            client::CocoStructurePolicy::BboxCompatibility
        } else {
            client::CocoStructurePolicy::Canonical
        },
        geometry_bounds: match policies.geometry_bounds {
            storage::GeometryBoundsPolicy::Block => client::GeometryBoundsPolicy::Reject,
            storage::GeometryBoundsPolicy::ClipDerived => client::GeometryBoundsPolicy::Clip,
        },
        cross_split_duplicates: match policies.cross_split_duplicates {
            storage::CrossSplitDuplicatePolicy::Block => client::CrossSplitDuplicatePolicy::Block,
            storage::CrossSplitDuplicatePolicy::MultipleMemberships => {
                client::CrossSplitDuplicatePolicy::MergeMemberships
            }
        },
        missing_keypoint_names: match policies.yolo_keypoint_names {
            storage::YoloKeypointNamePolicy::RequireSourceNames => {
                client::MissingKeypointNamesPolicy::Block
            }
            storage::YoloKeypointNamePolicy::GenerateIndexed => {
                client::MissingKeypointNamesPolicy::GenerateIndexed
            }
        },
    }
}

fn client_geometry_mapping(
    mapping: &labello_domain::ImportGeometryMapping,
) -> client::ImportGeometryMappingRequest {
    let (policy, parameters) = match &mapping.policy {
        labello_domain::ImportGeometryPolicy::Direct => {
            (client::ImportGeometryPolicy::Direct, Vec::new())
        }
        labello_domain::ImportGeometryPolicy::KeypointEnvelopeV1 {
            padding_ratio,
            minimum_pixels,
            include_hidden,
        } => (
            client::ImportGeometryPolicy::KeypointEnvelopeV1,
            vec![
                client::ImportMappingParameter::Scalar {
                    name: "paddingRatio".to_string(),
                    value: *padding_ratio,
                },
                client::ImportMappingParameter::Scalar {
                    name: "minimumPixels".to_string(),
                    value: f64::from(*minimum_pixels),
                },
                client::ImportMappingParameter::Boolean {
                    name: "includeHidden".to_string(),
                    value: *include_hidden,
                },
            ],
        ),
        labello_domain::ImportGeometryPolicy::ManualBoxGuideV1 => {
            (client::ImportGeometryPolicy::ManualBoxGuideV1, Vec::new())
        }
        labello_domain::ImportGeometryPolicy::BoxRelativeTemplateV1 { keypoints } => (
            client::ImportGeometryPolicy::BoxRelativeTemplateV1,
            keypoints
                .iter()
                .map(|point| client::ImportMappingParameter::Point {
                    name: point.name.clone(),
                    x: point.x,
                    y: point.y,
                    state: point.state.clone(),
                })
                .collect(),
        ),
        labello_domain::ImportGeometryPolicy::Omit => {
            (client::ImportGeometryPolicy::Omit, Vec::new())
        }
    };
    client::ImportGeometryMappingRequest {
        source_category_key: mapping.source_category_key.clone(),
        source_geometry: match mapping.source_geometry {
            labello_domain::ImportGeometryKind::BoundingBox => {
                client::ImportGeometryKind::BoundingBox
            }
            labello_domain::ImportGeometryKind::Skeleton => client::ImportGeometryKind::Skeleton,
        },
        target_geometry: match mapping.target_geometry {
            labello_domain::ImportGeometryKind::BoundingBox => {
                client::ImportGeometryKind::BoundingBox
            }
            labello_domain::ImportGeometryKind::Skeleton => client::ImportGeometryKind::Skeleton,
        },
        policy,
        parameters,
    }
}

fn convert_source_reference(
    example: &storage::DiagnosticExample,
) -> client::ImportDiagnosticSourceReference {
    client::ImportDiagnosticSourceReference {
        relative_path: example.source_path.clone(),
        source_image_id: example.source_image_key.clone(),
        category_id: None,
        annotation_id: example.source_object_key.clone(),
        line: example.line,
    }
}

fn convert_diagnostic(
    diagnostic: &storage::ImportDiagnostic,
    index: u64,
    occurrence: u64,
) -> client::ImportDiagnostic {
    client::ImportDiagnostic {
        diagnostic_id: format!("{}:{index}", diagnostic.code),
        code: diagnostic.code.clone(),
        severity: client_severity(diagnostic.severity),
        source_profile: client_profile(diagnostic.profile),
        safe_summary: diagnostic.summary.clone(),
        impact: client::ImportDiagnosticImpact {
            blocks_commit: diagnostic.blocks_commit,
            requires_acknowledgement: diagnostic.requires_acknowledgement,
            changes_coverage: diagnostic.changes_coverage,
            discards_metadata: false,
        },
        source: usize::try_from(occurrence)
            .ok()
            .and_then(|occurrence| diagnostic.examples.get(occurrence))
            .map(convert_source_reference),
    }
}

fn map_storage(error: storage::StorageError) -> ApiError {
    match error {
        storage::StorageError::NotFound(_) => ApiError::NotFound("import job".to_string()),
        storage::StorageError::Import { code, message } => match code.as_str() {
            "import_owner_mismatch" => ApiError::NotFound("import job".to_string()),
            "import_root_forbidden" => ApiError::Forbidden("import root access denied".to_string()),
            "import_id_invalid" | "destination_id_invalid" | "destination_id_reserved" => {
                ApiError::BadRequest(message)
            }
            "source_file_limit"
            | "source_byte_limit"
            | "source_file_too_large"
            | "server_source_browse_limit"
            | "upload_chunk_limit"
            | "selected_image_limit"
            | "annotation_limit"
            | "keypoint_limit" => ApiError::PayloadTooLarge(message),
            "destination_exists"
            | "destination_reserved"
            | "source_path_collision"
            | "job_phase_invalid"
            | "source_sealed"
            | "upload_chunk_not_sequential"
            | "upload_chunk_retry_mismatch"
            | "source_changed"
            | "plan_stale"
            | "job_not_cancellable"
            | "reservation_limit"
            | "upload_concurrency_limit"
            | "build_concurrency_limit"
            | "descriptor_inspection_busy"
            | "import_unavailable" => ApiError::Conflict(message),
            "profile_disabled"
            | "destination_name_invalid"
            | "source_incomplete"
            | "ground_truth_attestation_required"
            | "plan_not_committable"
            | "parser_time_limit"
            | "source_file_missing"
            | "import_root_missing"
            | "upload_chunk_digest_mismatch"
            | "source_file_digest_mismatch" => ApiError::Unprocessable(message),
            _ if code.starts_with("yolo_")
                || code.starts_with("coco_")
                || code.starts_with("image_")
                || code.starts_with("descriptor_")
                || code.starts_with("server_source_")
                || code.starts_with("source_path_")
                || code.starts_with("source_file_")
                || code.starts_with("geometry_")
                || code.starts_with("category_") =>
            {
                ApiError::Unprocessable(message)
            }
            _ => ApiError::Storage(storage::StorageError::Import { code, message }),
        },
        error => ApiError::Storage(error),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobControl {
    import_id: ImportId,
    owner_user_id: UserId,
    create_request: client::CreateImportRequest,
    seal_request: Option<client::SealImportRequest>,
    files: BTreeMap<String, FileControl>,
    plan: Option<storage::ImportPlan>,
    #[serde(default)]
    accepted_plan_request: Option<client::UpdateImportPlanRequest>,
    #[serde(default)]
    pending_plan_request: Option<client::UpdateImportPlanRequest>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileControl {
    client_file_id: Option<String>,
    relative_path: String,
    byte_size: u64,
    blake3: String,
    #[serde(default)]
    accepted_bytes: u64,
    #[serde(default)]
    complete: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceIndexSnapshot {
    files: BTreeMap<String, storage::RegisteredFile>,
}

async fn source_files(
    state: &ApiState,
    import_id: &ImportId,
) -> ApiResult<BTreeMap<String, FileControl>> {
    let path = state
        .datasets_root()
        .join(".labello-server/imports/jobs")
        .join(import_id.as_str())
        .join("source-index.json");
    let index: SourceIndexSnapshot = read_json(&path).await?;
    Ok(index
        .files
        .into_iter()
        .map(|(file_id, file)| {
            (
                file_id,
                FileControl {
                    client_file_id: None,
                    relative_path: file.relative_path,
                    byte_size: file.byte_size,
                    blake3: file.blake3,
                    accepted_bytes: file.accepted_bytes,
                    complete: file.complete,
                },
            )
        })
        .collect())
}

async fn reconcile_registered_files(
    state: &ApiState,
    import_id: &ImportId,
    request: &client::RegisterImportFilesRequest,
) -> ApiResult<Option<Vec<storage::RegisteredFile>>> {
    let path = state
        .datasets_root()
        .join(".labello-server/imports/jobs")
        .join(import_id.as_str())
        .join("source-index.json");
    let index: SourceIndexSnapshot = read_json(&path).await?;
    let mut result = Vec::with_capacity(request.files.len());
    for requested in &request.files {
        let Some(expected_digest) = requested.blake3.as_deref() else {
            return Ok(None);
        };
        let expected_digest = parse_digest(expected_digest)?;
        let Some(file) = index.files.values().find(|file| {
            file.relative_path == requested.relative_path
                && file.byte_size == requested.byte_size
                && file.blake3 == expected_digest
        }) else {
            return Ok(None);
        };
        result.push(file.clone());
    }
    Ok(Some(result))
}

fn api_root(state: &ApiState) -> std::path::PathBuf {
    state
        .datasets_root()
        .join(".labello-server/imports")
        .join(API_STATE_DIR)
}

fn job_control_path(state: &ApiState, import_id: &ImportId) -> std::path::PathBuf {
    api_root(state)
        .join(API_JOBS_DIR)
        .join(format!("{}.json", import_id.as_str()))
}

async fn save_job_control(state: &ApiState, control: &JobControl) -> ApiResult<()> {
    write_json_atomic(&job_control_path(state, &control.import_id), control).await
}

async fn load_job_control(state: &ApiState, import_id: &ImportId) -> ApiResult<JobControl> {
    import_id.validate_path_segment()?;
    read_json(&job_control_path(state, import_id)).await
}

async fn reconcile_job_control(
    state: &ApiState,
    job: storage::ImportJob,
    mut control: JobControl,
) -> ApiResult<JobControl> {
    if job.phase != storage::ImportJobPhase::AwaitingDecision {
        return Ok(control);
    }
    let plan = require_service(state)?
        .plan(&job.import_id, &job.owner_user_id)
        .await
        .map_err(map_storage)?;
    let plan_changed = control
        .plan
        .as_ref()
        .is_none_or(|stored| stored.plan_hash != plan.plan_hash);
    if plan_changed {
        control.plan = Some(plan.clone());
        if let Some(pending) = control.pending_plan_request.clone()
            && convert_plan_update(plan.request.clone(), pending.clone())? == plan.request
        {
            control.accepted_plan_request = Some(pending);
            control.pending_plan_request = None;
        } else {
            control.accepted_plan_request = None;
        }
        save_job_control(state, &control).await?;
    }
    Ok(control)
}

async fn list_job_controls(state: &ApiState) -> ApiResult<Vec<JobControl>> {
    let directory = api_root(state).join(API_JOBS_DIR);
    if !tokio::fs::try_exists(&directory)
        .await
        .map_err(|source| io_error(&directory, source))?
    {
        return Ok(Vec::new());
    }
    let mut entries = tokio::fs::read_dir(&directory)
        .await
        .map_err(|source| io_error(&directory, source))?;
    let mut controls = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|source| io_error(&directory, source))?
    {
        if entry
            .file_type()
            .await
            .map_err(|source| io_error(entry.path(), source))?
            .is_file()
        {
            controls.push(read_json(&entry.path()).await?);
        }
    }
    Ok(controls)
}

async fn find_matching_job(
    state: &ApiState,
    owner: &UserId,
    request: &storage::CreateImportRequest,
) -> ApiResult<Option<storage::ImportJob>> {
    let directory = state.datasets_root().join(".labello-server/imports/jobs");
    let mut entries = tokio::fs::read_dir(&directory)
        .await
        .map_err(|source| io_error(&directory, source))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|source| io_error(&directory, source))?
    {
        let path = entry.path().join("job.json");
        if !tokio::fs::try_exists(&path)
            .await
            .map_err(|source| io_error(&path, source))?
        {
            continue;
        }
        let job: storage::ImportJob = read_json(&path).await?;
        if &job.owner_user_id == owner
            && job.destination_dataset_id == request.destination_dataset_id
            && job.destination_name == request.destination_name
            && job.profile == request.profile
            && job.transport == request.transport
            && !matches!(
                job.phase,
                storage::ImportJobPhase::Failed
                    | storage::ImportJobPhase::Cancelled
                    | storage::ImportJobPhase::Expired
            )
        {
            return Ok(Some(job));
        }
    }
    Ok(None)
}

enum Idempotency<T> {
    Replay(T),
    Execute,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "status")]
enum IdempotencyRecord {
    Pending {
        operation: String,
        request_hash: String,
    },
    Complete {
        operation: String,
        request_hash: String,
        response: serde_json::Value,
    },
}

fn request_path(state: &ApiState, owner: &UserId, key: &str) -> std::path::PathBuf {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"labello:import-api-idempotency:v1\0");
    hasher.update(owner.as_str().as_bytes());
    hasher.update(b"\0");
    hasher.update(key.as_bytes());
    api_root(state)
        .join(API_REQUESTS_DIR)
        .join(format!("{}.json", hasher.finalize().to_hex()))
}

fn request_hash<T: Serialize>(operation: &str, request: &T) -> ApiResult<String> {
    let bytes = serde_json::to_vec(request)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"labello:import-api-request:v1\0");
    hasher.update(operation.as_bytes());
    hasher.update(b"\0");
    hasher.update(&bytes);
    Ok(hasher.finalize().to_hex().to_string())
}

async fn begin_idempotency<T, R>(
    state: &ApiState,
    owner: &UserId,
    key: &str,
    operation: &str,
    request: &T,
) -> ApiResult<Idempotency<R>>
where
    T: Serialize,
    R: DeserializeOwned,
{
    let path = request_path(state, owner, key);
    let hash = request_hash(operation, request)?;
    if tokio::fs::try_exists(&path)
        .await
        .map_err(|source| io_error(&path, source))?
    {
        let record: IdempotencyRecord = read_json(&path).await?;
        return match record {
            IdempotencyRecord::Pending {
                operation: stored_operation,
                request_hash,
            } if stored_operation == operation && request_hash == hash => Ok(Idempotency::Execute),
            IdempotencyRecord::Complete {
                operation: stored_operation,
                request_hash,
                response,
            } if stored_operation == operation && request_hash == hash => {
                Ok(Idempotency::Replay(serde_json::from_value(response)?))
            }
            _ => Err(ApiError::Conflict(
                "idempotency key was already used for a different request".to_string(),
            )),
        };
    }
    write_json_atomic(
        &path,
        &IdempotencyRecord::Pending {
            operation: operation.to_string(),
            request_hash: hash,
        },
    )
    .await?;
    Ok(Idempotency::Execute)
}

async fn complete_idempotency<R: Serialize>(
    state: &ApiState,
    owner: &UserId,
    key: &str,
    operation: &str,
    response: &R,
) -> ApiResult<()> {
    let path = request_path(state, owner, key);
    let record: IdempotencyRecord = read_json(&path).await?;
    let request_hash = match record {
        IdempotencyRecord::Pending {
            operation: stored_operation,
            request_hash,
        } if stored_operation == operation => request_hash,
        IdempotencyRecord::Complete { .. } => return Ok(()),
        _ => {
            return Err(ApiError::Conflict(
                "idempotency key changed while processing".to_string(),
            ));
        }
    };
    write_json_atomic(
        &path,
        &IdempotencyRecord::Complete {
            operation: operation.to_string(),
            request_hash,
            response: serde_json::to_value(response)?,
        },
    )
    .await
}

async fn read_json<T: DeserializeOwned>(path: &Path) -> ApiResult<T> {
    labello_storage::fsjson::read_json(path)
        .await
        .map_err(ApiError::Storage)
}

async fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> ApiResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| ApiError::Internal("import control path has no parent".to_string()))?;
    let api_directory = parent.parent().unwrap_or(parent);
    if let Some(imports_directory) = api_directory.parent() {
        tokio::fs::create_dir_all(imports_directory)
            .await
            .map_err(|source| io_error(imports_directory, source))?;
    }
    create_private_directory(api_directory).await?;
    create_private_directory(parent).await?;

    let temporary = path.with_extension(format!("tmp-{}", uuid::Uuid::now_v7().simple()));
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut options = tokio::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .await
        .map_err(|source| io_error(&temporary, source))?;
    use tokio::io::AsyncWriteExt;
    file.write_all(&bytes)
        .await
        .map_err(|source| io_error(&temporary, source))?;
    file.write_all(b"\n")
        .await
        .map_err(|source| io_error(&temporary, source))?;
    file.sync_all()
        .await
        .map_err(|source| io_error(&temporary, source))?;
    drop(file);
    tokio::fs::rename(&temporary, path)
        .await
        .map_err(|source| io_error(path, source))?;
    tokio::fs::File::open(parent)
        .await
        .map_err(|source| io_error(parent, source))?
        .sync_all()
        .await
        .map_err(|source| io_error(parent, source))?;
    Ok(())
}

async fn create_private_directory(path: &Path) -> ApiResult<()> {
    let mut builder = tokio::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        builder.mode(0o700);
    }
    match builder.create(path).await {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
                    .await
                    .map_err(|source| io_error(path, source))?;
            }
            Ok(())
        }
        Err(source) => Err(io_error(path, source)),
    }
}

fn io_error(path: impl Into<std::path::PathBuf>, source: std::io::Error) -> ApiError {
    ApiError::Storage(storage::StorageError::Io {
        path: path.into(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use labello_domain::{DatasetId, SCHEMA_VERSION, now};
    use serde_json::json;

    #[test]
    fn parser_time_limit_is_an_actionable_client_error() {
        let error = map_storage(storage::StorageError::Import {
            code: "parser_time_limit".to_string(),
            message: "import parsing exceeded the parser time budget".to_string(),
        });

        assert!(matches!(error, ApiError::Unprocessable(_)));
    }

    fn attestations() -> client::ImportAttestations {
        client::ImportAttestations {
            ground_truth: true,
            exhaustive: true,
            coverage_scope: Vec::new(),
            provenance: "fixture".to_string(),
        }
    }

    fn job(profile: storage::ImportProfile) -> storage::ImportJob {
        let timestamp = now();
        storage::ImportJob {
            schema_version: SCHEMA_VERSION,
            import_id: ImportId::from("imp_test"),
            owner_user_id: UserId::from("admin"),
            destination_dataset_id: DatasetId::from("imported"),
            destination_name: "Imported".to_string(),
            profile,
            transport: storage::ImportTransport::Browser,
            phase: storage::ImportJobPhase::Uploading,
            source_fingerprint: None,
            plan_hash: None,
            preflight_generation: None,
            accepted_files: 2,
            accepted_bytes: 10,
            created_at: timestamp,
            updated_at: timestamp,
            failure_code: None,
        }
    }

    fn control(profile: client::ImportProfile) -> JobControl {
        JobControl {
            import_id: ImportId::from("imp_test"),
            owner_user_id: UserId::from("admin"),
            create_request: client::CreateImportRequest {
                destination_dataset_id: DatasetId::from("imported"),
                destination_name: "Imported".to_string(),
                profile,
                source: client::ImportSourceSelection::BrowserFolder,
                attestations: attestations(),
            },
            seal_request: None,
            files: BTreeMap::from([
                (
                    "descriptor".to_string(),
                    FileControl {
                        client_file_id: None,
                        relative_path: "annotations/keypoints.json".to_string(),
                        byte_size: 5,
                        blake3: "a".repeat(64),
                        accepted_bytes: 5,
                        complete: true,
                    },
                ),
                (
                    "instances".to_string(),
                    FileControl {
                        client_file_id: None,
                        relative_path: "annotations/instances.json".to_string(),
                        byte_size: 5,
                        blake3: "c".repeat(64),
                        accepted_bytes: 5,
                        complete: true,
                    },
                ),
                (
                    "image".to_string(),
                    FileControl {
                        client_file_id: None,
                        relative_path: "images/a.png".to_string(),
                        byte_size: 5,
                        blake3: "b".repeat(64),
                        accepted_bytes: 5,
                        complete: true,
                    },
                ),
            ]),
            plan: None,
            accepted_plan_request: None,
            pending_plan_request: None,
        }
    }

    #[test]
    fn coco_selection_preserves_identity_and_rejects_unsupported_inputs() {
        let job = job(storage::ImportProfile::CocoKeypointsGtV1);
        let control = control(client::ImportProfile::CocoKeypointsGtV1);
        let mut seal: client::SealImportRequest = serde_json::from_value(json!({
            "source": {
                "sourceNamespace": "release_set",
                "descriptors": [
                    {
                        "descriptorFileId": "instances",
                        "kind": "coco_instances",
                        "release": "v2",
                        "split": "train",
                        "imageRootFileId": "image",
                        "pairingGroup": "people"
                    },
                    {
                        "descriptorFileId": "descriptor",
                        "kind": "coco_keypoints",
                        "release": "v2",
                        "split": "train",
                        "imageRootFileId": "image",
                        "pairingGroup": "people"
                    }
                ],
                "selectedSplits": ["train"],
                "selectedCategoryKeys": []
            },
            "attestations": {
                "groundTruth": true,
                "exhaustive": true,
                "coverageScope": [],
                "provenance": "fixture"
            }
        }))
        .unwrap();

        let converted = convert_preflight(&job, &control, &seal).unwrap();
        assert_eq!(
            converted.intent,
            storage::ImportIntent::AuthoritativeGroundTruth
        );
        assert_eq!(converted.coco_descriptors.len(), 2);
        let descriptor = &converted.coco_descriptors[1];
        assert_eq!(descriptor.descriptor_path, "annotations/keypoints.json");
        assert_eq!(descriptor.image_root, "images");
        assert_eq!(descriptor.split, "train");
        assert_eq!(descriptor.source_namespace, "release_set");
        assert_eq!(descriptor.release, "v2");
        assert_eq!(
            descriptor.kind,
            labello_domain::ImportDescriptorKind::CocoKeypoints
        );
        assert_eq!(descriptor.pairing_group.as_deref(), Some("people"));

        let mut non_exhaustive_control = control.clone();
        non_exhaustive_control
            .create_request
            .attestations
            .exhaustive = false;
        let mut non_exhaustive_seal = seal.clone();
        non_exhaustive_seal.attestations.exhaustive = false;
        let converted =
            convert_preflight(&job, &non_exhaustive_control, &non_exhaustive_seal).unwrap();
        assert_eq!(converted.intent, storage::ImportIntent::RequireApproval);

        seal.source.descriptors[0].kind = client::ImportDescriptorKind::YoloDataset;
        assert!(convert_preflight(&job, &control, &seal).is_err());
        seal.source.descriptors[0].kind = client::ImportDescriptorKind::CocoInstances;
        seal.source.selected_category_keys = vec!["release_set:v2:7".to_string()];
        assert!(convert_preflight(&job, &control, &seal).is_err());
    }

    fn current_preflight() -> storage::PreflightRequest {
        storage::PreflightRequest {
            descriptor_paths: vec!["dataset.yaml".to_string()],
            selected_splits: vec!["train".to_string()],
            coco_descriptors: Vec::new(),
            ground_truth_attested: true,
            exhaustive_attested: true,
            source_namespace: "fixture".to_string(),
            source_release: "v1".to_string(),
            coverage_scope: vec!["person".to_string()],
            attestation_provenance: "fixture".to_string(),
            intent: storage::ImportIntent::AuthoritativeGroundTruth,
            policies: storage::CompatibilityPolicies::default(),
            output: storage::OutputPolicy::defaults_for(
                storage::ImportProfile::UltralyticsYoloDetectV1,
            ),
            acknowledged_warning_codes: Vec::new(),
            category_mappings: Vec::new(),
            task_mappings: Vec::new(),
            geometry_mappings: Vec::new(),
        }
    }

    fn valid_mapping_json() -> serde_json::Value {
        json!({
            "categoryMappings": [{
                "sourceCategoryKey": "0", "sourceCategoryId": "0",
                "classId": "person", "className": "Person", "color": "#123456",
                "selected": true
            }],
            "geometryMappings": [{
                "sourceCategoryKey": "0", "sourceGeometry": "bounding_box",
                "targetGeometry": "bounding_box", "policy": "direct", "parameters": []
            }],
            "taskMappings": [{
                "sourceCategoryKey": "0",
                "task": {
                    "taskId": "person-box", "name": "Person boxes",
                    "annotationType": "bounding_box", "classIds": ["person"],
                    "instructions": {"title": "Boxes", "exampleText": "Draw", "exampleImages": []},
                    "skeleton": null,
                    "review": {"requiredReviews": 0, "workflow": "none", "allowReviewerCorrections": false, "agreementThreshold": null},
                    "prelabelConfigIds": [], "manualBoxGuideMigration": null, "enabled": true
                },
                "workflowIntent": "authoritative_ground_truth"
            }],
            "skeletonMappings": [], "compatibility": {}, "acknowledgements": []
        })
    }

    #[test]
    fn plan_mapping_validation_rejects_malicious_or_unrepresentable_shapes() {
        let valid: client::UpdateImportPlanRequest =
            serde_json::from_value(valid_mapping_json()).unwrap();
        assert!(convert_plan_update(current_preflight(), valid).is_ok());

        let mut wrong_class = valid_mapping_json();
        wrong_class["taskMappings"][0]["task"]["classIds"] = json!(["other"]);
        let request = serde_json::from_value(wrong_class).unwrap();
        assert!(convert_plan_update(current_preflight(), request).is_err());

        let mut duplicate_category = valid_mapping_json();
        let duplicate = duplicate_category["categoryMappings"][0].clone();
        duplicate_category["categoryMappings"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        let request = serde_json::from_value(duplicate_category).unwrap();
        assert!(convert_plan_update(current_preflight(), request).is_err());

        let mut wrong_workflow = valid_mapping_json();
        wrong_workflow["taskMappings"][0]["task"]["review"]["workflow"] = json!("approval");
        wrong_workflow["taskMappings"][0]["task"]["review"]["requiredReviews"] = json!(1);
        let request = serde_json::from_value(wrong_workflow).unwrap();
        assert!(convert_plan_update(current_preflight(), request).is_err());

        let mut envelope = valid_mapping_json();
        envelope["geometryMappings"][0]["sourceGeometry"] = json!("skeleton");
        envelope["geometryMappings"][0]["policy"] = json!("keypoint_envelope_v1");
        let request = serde_json::from_value(envelope).unwrap();
        assert!(convert_plan_update(current_preflight(), request).is_err());

        let mut template = valid_mapping_json();
        template["geometryMappings"][0]["targetGeometry"] = json!("skeleton");
        template["geometryMappings"][0]["policy"] = json!("box_relative_template_v1");
        let request = serde_json::from_value(template).unwrap();
        assert!(convert_plan_update(current_preflight(), request).is_err());

        let mut manual_without_guide = valid_mapping_json();
        manual_without_guide["geometryMappings"][0]["targetGeometry"] = json!("skeleton");
        manual_without_guide["geometryMappings"][0]["policy"] = json!("manual_box_guide_v1");
        let request = serde_json::from_value(manual_without_guide).unwrap();
        assert!(convert_plan_update(current_preflight(), request).is_err());
    }

    #[test]
    fn plan_mapping_converts_envelope_and_exact_named_template_parameters() {
        let mut envelope = valid_mapping_json();
        envelope["geometryMappings"][0]["sourceGeometry"] = json!("skeleton");
        envelope["geometryMappings"][0]["policy"] = json!("keypoint_envelope_v1");
        envelope["geometryMappings"][0]["parameters"] = json!([
            {"name": "padding", "value": 0.05},
            {"name": "minimumPixels", "value": 1.0},
            {"name": "includeHidden", "value": true}
        ]);
        let converted = convert_plan_update(
            current_preflight(),
            serde_json::from_value(envelope).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            converted.geometry_mappings[0].policy,
            labello_domain::ImportGeometryPolicy::KeypointEnvelopeV1 {
                padding_ratio: 0.05,
                minimum_pixels: 1,
                include_hidden: true
            }
        ));

        let mut template = valid_mapping_json();
        template["geometryMappings"][0]["targetGeometry"] = json!("skeleton");
        template["geometryMappings"][0]["policy"] = json!("box_relative_template_v1");
        template["geometryMappings"][0]["parameters"] = json!([
            {"name": "nose", "x": 0.5, "y": 0.25, "state": "visible"},
            {"name": "tail", "x": 0.5, "y": 0.75, "state": "hidden"}
        ]);
        template["taskMappings"][0]["task"]["taskId"] = json!("person-skeleton");
        template["taskMappings"][0]["task"]["name"] = json!("Person skeleton");
        template["taskMappings"][0]["task"]["annotationType"] = json!("skeleton");
        template["taskMappings"][0]["task"]["skeleton"] = json!({
            "keypoints": [
                {"name": "nose", "required": false},
                {"name": "tail", "required": false}
            ],
            "edges": [{"from": "nose", "to": "tail"}],
            "allowHidden": true,
            "allowAbsent": true
        });
        template["skeletonMappings"] = json!([{
            "sourceCategoryKey": "0",
            "targetTaskId": "person-skeleton",
            "skeleton": template["taskMappings"][0]["task"]["skeleton"].clone(),
            "sourceKeypointNames": [],
            "namesConfirmed": true
        }]);
        let converted = convert_plan_update(
            current_preflight(),
            serde_json::from_value(template).unwrap(),
        )
        .unwrap();
        let labello_domain::ImportGeometryPolicy::BoxRelativeTemplateV1 { keypoints } =
            &converted.geometry_mappings[0].policy
        else {
            panic!("expected template policy");
        };
        assert_eq!(
            keypoints
                .iter()
                .map(|point| point.name.as_str())
                .collect::<Vec<_>>(),
            ["nose", "tail"]
        );

        let mut invalid = valid_mapping_json();
        invalid["geometryMappings"][0]["sourceGeometry"] = json!("skeleton");
        invalid["geometryMappings"][0]["policy"] = json!("keypoint_envelope_v1");
        invalid["geometryMappings"][0]["parameters"] = json!([
            {"name": "padding", "value": "NaN"},
            {"name": "minimumPixels", "value": 0.5},
            {"name": "includeHidden", "value": true}
        ]);
        assert!(
            serde_json::from_value::<client::UpdateImportPlanRequest>(invalid).is_err(),
            "non-numeric public parameters must be rejected during decoding"
        );
    }

    #[test]
    fn plan_mapping_allows_independent_manual_categories() {
        let mut request = valid_mapping_json();
        request["categoryMappings"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "sourceCategoryKey": "1", "sourceCategoryId": "1",
                "classId": "car", "className": "Car", "color": "#654321",
                "selected": true
            }));
        request["geometryMappings"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "sourceCategoryKey": "0", "sourceGeometry": "bounding_box",
                "targetGeometry": "skeleton", "policy": "manual_box_guide_v1",
                "parameters": []
            }));
        request["geometryMappings"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "sourceCategoryKey": "1", "sourceGeometry": "bounding_box",
                "targetGeometry": "bounding_box", "policy": "direct",
                "parameters": []
            }));
        request["taskMappings"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "sourceCategoryKey": "0",
                "task": {
                    "taskId": "person-skeleton", "name": "Person skeleton",
                    "annotationType": "skeleton", "classIds": ["person"],
                    "instructions": {"title": "Skeleton", "exampleText": "Draw", "exampleImages": []},
                    "skeleton": {
                        "keypoints": [{"name": "center", "required": false}],
                        "edges": [], "allowHidden": false, "allowAbsent": true
                    },
                    "review": {"requiredReviews": 0, "workflow": "none", "allowReviewerCorrections": false, "agreementThreshold": null},
                    "prelabelConfigIds": [],
                    "manualBoxGuideMigration": {
                        "guideTaskId": "person-box", "cardinality": "exactly_one",
                        "allowExclusion": true, "sequence": "imported_spatial_order_v1"
                    },
                    "enabled": true
                },
                "workflowIntent": "authoritative_ground_truth"
            }));
        request["taskMappings"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "sourceCategoryKey": "1",
                "task": {
                    "taskId": "car-box", "name": "Car boxes",
                    "annotationType": "bounding_box", "classIds": ["car"],
                    "instructions": {"title": "Boxes", "exampleText": "Draw", "exampleImages": []},
                    "skeleton": null,
                    "review": {"requiredReviews": 0, "workflow": "none", "allowReviewerCorrections": false, "agreementThreshold": null},
                    "prelabelConfigIds": [], "manualBoxGuideMigration": null, "enabled": true
                },
                "workflowIntent": "authoritative_ground_truth"
            }));
        request["skeletonMappings"] = json!([{
            "sourceCategoryKey": "0", "targetTaskId": "person-skeleton",
            "sourceKeypointNames": [], "namesConfirmed": true,
            "skeleton": {
                "keypoints": [{"name": "center", "required": false}],
                "edges": [], "allowHidden": false, "allowAbsent": true
            }
        }]);
        let converted = convert_plan_update(
            current_preflight(),
            serde_json::from_value(request.clone()).unwrap(),
        )
        .unwrap();
        assert_eq!(converted.geometry_mappings.len(), 3);
        assert!(converted.geometry_mappings.iter().any(|mapping| {
            mapping.source_category_key == "0"
                && matches!(
                    mapping.policy,
                    labello_domain::ImportGeometryPolicy::ManualBoxGuideV1
                )
        }));
        assert!(converted.geometry_mappings.iter().any(|mapping| {
            mapping.source_category_key == "1"
                && matches!(mapping.policy, labello_domain::ImportGeometryPolicy::Direct)
        }));

        let mut second_manual = request;
        second_manual["geometryMappings"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "sourceCategoryKey": "1", "sourceGeometry": "bounding_box",
                "targetGeometry": "skeleton", "policy": "manual_box_guide_v1",
                "parameters": []
            }));
        let mut skeleton_task = second_manual["taskMappings"][1].clone();
        skeleton_task["sourceCategoryKey"] = json!("1");
        skeleton_task["task"]["taskId"] = json!("car-skeleton");
        skeleton_task["task"]["name"] = json!("Car skeleton");
        skeleton_task["task"]["classIds"] = json!(["car"]);
        skeleton_task["task"]["manualBoxGuideMigration"]["guideTaskId"] = json!("car-box");
        second_manual["taskMappings"]
            .as_array_mut()
            .unwrap()
            .push(skeleton_task);
        let mut skeleton_mapping = second_manual["skeletonMappings"][0].clone();
        skeleton_mapping["sourceCategoryKey"] = json!("1");
        skeleton_mapping["targetTaskId"] = json!("car-skeleton");
        second_manual["skeletonMappings"]
            .as_array_mut()
            .unwrap()
            .push(skeleton_mapping);
        let shared_schema = convert_plan_update(
            current_preflight(),
            serde_json::from_value(second_manual.clone()).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            shared_schema.output.box_to_skeleton,
            storage::BoxToSkeletonPolicy::ManualBoxGuide { .. }
        ));

        second_manual["taskMappings"][3]["task"]["skeleton"]["keypoints"] =
            json!([{"name": "wheel", "required": false}]);
        second_manual["skeletonMappings"][1]["skeleton"]["keypoints"] =
            json!([{"name": "wheel", "required": false}]);
        let converted = convert_plan_update(
            current_preflight(),
            serde_json::from_value(second_manual).unwrap(),
        )
        .unwrap();
        assert_eq!(converted.geometry_mappings.len(), 4);
        assert_eq!(converted.task_mappings.len(), 4);
        assert!(matches!(
            converted.output.box_to_skeleton,
            storage::BoxToSkeletonPolicy::None
        ));
        for (category, guide, target) in [
            ("0", "person-box", "person-skeleton"),
            ("1", "car-box", "car-skeleton"),
        ] {
            let target = converted
                .task_mappings
                .iter()
                .find(|mapping| {
                    mapping.source_category_key == category
                        && mapping.task.task_id == labello_domain::TaskId::from(target)
                })
                .unwrap();
            assert_eq!(
                target
                    .task
                    .manual_box_guide_migration
                    .as_ref()
                    .unwrap()
                    .guide_task_id,
                labello_domain::TaskId::from(guide)
            );
        }
    }

    fn report_plan() -> storage::ImportPlan {
        storage::ImportPlan {
            schema_version: SCHEMA_VERSION,
            import_id: ImportId::from("imp_report"),
            destination_dataset_id: DatasetId::from("report"),
            source_fingerprint: "source".to_string(),
            plan_hash: "plan".to_string(),
            request: current_preflight(),
            totals: storage::ImportTotals {
                source_files: 3,
                source_bytes: 30,
                descriptors: 1,
                images: 2,
                categories: 1,
                source_objects: 7,
                keypoints: 0,
                direct_boxes: 7,
                direct_skeletons: 0,
                derived_geometry: 0,
                clipped_geometry: 0,
                envelope_derived: 0,
                template_derived: 0,
                output_tasks: 1,
                output_annotations: 7,
                estimated_output_bytes: 1_000,
            },
            coverage: labello_domain::ImportCoverageTotals {
                bounding_boxes: labello_domain::ImportCoverageCounts {
                    complete: 1,
                    verified_empty: 1,
                    incomplete: 2,
                    excluded: 3,
                },
                skeletons: Default::default(),
            },
            diagnostics: vec![storage::ImportDiagnostic {
                code: "bad_rows".to_string(),
                severity: storage::DiagnosticSeverity::Error,
                profile: storage::ImportProfile::UltralyticsYoloDetectV1,
                count: 7,
                summary: "bad rows".to_string(),
                blocks_commit: true,
                requires_acknowledgement: true,
                changes_coverage: false,
                examples: Vec::new(),
            }],
            source_categories: BTreeMap::from([(
                "0".to_string(),
                storage::ImportSourceCategory {
                    source_namespace: "fixture".to_string(),
                    source_category_id: "0".to_string(),
                    source_name: "Person".to_string(),
                    source_supercategory: None,
                    direct_bounding_boxes: true,
                    direct_skeletons: false,
                    keypoint_names: Vec::new(),
                    edges: Vec::new(),
                    allow_hidden: false,
                },
            )]),
            class_ids: BTreeMap::from([("0".to_string(), "person".to_string())]),
            task_ids: BTreeMap::from([("0".to_string(), vec!["person-box".to_string()])]),
        }
    }

    #[test]
    fn reports_and_diagnostic_pages_use_storage_occurrence_counts() {
        let plan = report_plan();
        let report = convert_report(&plan);
        assert_eq!(report.blocking_diagnostics, 7);
        assert_eq!(report.required_acknowledgements, 7);
        assert_eq!(report.output.events, 2);
        assert_eq!(report.output.temporary_bytes, 1_000);
        assert_eq!(report.source.objects, 7);
        assert_eq!(report.coverage.complete, 1);
        assert_eq!(report.coverage.verified_empty, 1);
        assert_eq!(report.coverage.incomplete, 2);
        assert_eq!(report.coverage.excluded, 3);
        assert_eq!(report.coverage_by_geometry.bounding_boxes, report.coverage);
        assert_eq!(
            report.coverage_by_geometry.skeletons,
            client::ImportCoverageCounts::default()
        );
        assert_eq!(report.output.required_free_bytes, 64 * 1024 * 1024 + 1_100);

        let query = client::ImportDiagnosticsQuery {
            cursor: Some("2".to_string()),
            limit: 3,
            code: None,
            severity: None,
        };
        let page = diagnostic_page(&plan, &query, 2);
        assert_eq!(page.total, 7);
        assert_eq!(page.diagnostics.len(), 3);
        assert_eq!(page.next_cursor.as_deref(), Some("5"));
        assert_eq!(page.diagnostics[0].diagnostic_id, "bad_rows:2");

        let mut bad_source_id = valid_mapping_json();
        bad_source_id["categoryMappings"][0]["sourceCategoryId"] = json!("forged");
        let request = serde_json::from_value(bad_source_id).unwrap();
        assert!(validate_plan_update_against_current(&plan, &request).is_err());

        let mut bad_acknowledgement = valid_mapping_json();
        bad_acknowledgement["acknowledgements"] = json!([{
            "diagnosticCode": "bad_rows",
            "policy": "accept",
            "affectedCount": 6,
            "acknowledged": true
        }]);
        let request = serde_json::from_value(bad_acknowledgement).unwrap();
        assert!(validate_plan_update_against_current(&plan, &request).is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn api_import_control_state_is_private() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let path = temp
            .path()
            .join(".labello-server/imports/api/jobs/imp_test.json");
        write_json_atomic(&path, &json!({"private": true}))
            .await
            .unwrap();

        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(path.parent().unwrap().parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[tokio::test]
    async fn capabilities_filter_server_roots_for_each_actor() {
        let datasets = tempfile::tempdir().unwrap();
        let admin_root = tempfile::tempdir().unwrap();
        let other_root = tempfile::tempdir().unwrap();
        let public_root = tempfile::tempdir().unwrap();
        let service = storage::ImportService::new(
            datasets.path(),
            storage::ImportConfig {
                enabled: true,
                import_roots: vec![
                    storage::ImportRoot {
                        root_id: "admin".to_string(),
                        path: admin_root.path().to_path_buf(),
                        allowed_owners: vec![UserId::from("admin")],
                    },
                    storage::ImportRoot {
                        root_id: "other".to_string(),
                        path: other_root.path().to_path_buf(),
                        allowed_owners: vec![UserId::from("other")],
                    },
                    storage::ImportRoot {
                        root_id: "public".to_string(),
                        path: public_root.path().to_path_buf(),
                        allowed_owners: Vec::new(),
                    },
                ],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let fail_closed = ApiState::new(datasets.path()).with_import_service(service.clone());
        assert!(
            convert_capabilities(&fail_closed, &UserId::from("admin"))
                .server_roots
                .is_empty()
        );
        let state = ApiState::new(datasets.path())
            .with_import_service(service)
            .with_import_root_owners([
                ("admin".to_string(), BTreeSet::from([UserId::from("admin")])),
                ("other".to_string(), BTreeSet::from([UserId::from("other")])),
                ("public".to_string(), BTreeSet::new()),
            ]);

        let admin = convert_capabilities(&state, &UserId::from("admin"));
        assert_eq!(
            admin
                .server_roots
                .iter()
                .map(|root| root.root_id.as_str())
                .collect::<Vec<_>>(),
            ["admin", "public"]
        );
        let other = convert_capabilities(&state, &UserId::from("other"));
        assert_eq!(
            other
                .server_roots
                .iter()
                .map(|root| root.root_id.as_str())
                .collect::<Vec<_>>(),
            ["other", "public"]
        );
    }
}
