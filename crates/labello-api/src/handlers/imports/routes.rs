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
            let job = require_service(&state)?
                .find_matching_job(&actor.user_id, &storage_request)
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
        control.files = source_files(&state, &job.import_id, &actor.user_id).await?;
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
            reconcile_registered_files(&state, &import_id, &actor.user_id, &request)
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
    let job = require_owned_job(&state, &import_id, &actor.user_id).await?;
    let control =
        reconcile_job_control(&state, job, load_job_control(&state, &import_id).await?).await?;
    ensure_plan_update_settled(&control)?;
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

fn ensure_plan_update_settled(control: &JobControl) -> ApiResult<()> {
    if control.pending_plan_request.is_some() {
        return Err(ApiError::Conflict(
            "an import plan update is still pending; retry the mapping update before committing"
                .to_string(),
        ));
    }
    Ok(())
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
