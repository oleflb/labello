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

async fn source_files(
    state: &ApiState,
    import_id: &ImportId,
    owner: &UserId,
) -> ApiResult<BTreeMap<String, FileControl>> {
    Ok(require_service(state)?
        .registered_files(import_id, owner)
        .await
        .map_err(map_storage)?
        .into_iter()
        .map(|file| {
            (
                file.file_id,
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
    owner: &UserId,
    request: &client::RegisterImportFilesRequest,
) -> ApiResult<Option<Vec<storage::RegisteredFile>>> {
    let files = require_service(state)?
        .registered_files(import_id, owner)
        .await
        .map_err(map_storage)?;
    let mut result = Vec::with_capacity(request.files.len());
    for requested in &request.files {
        let Some(expected_digest) = requested.blake3.as_deref() else {
            return Ok(None);
        };
        let expected_digest = parse_digest(expected_digest)?;
        let Some(file) = files.iter().find(|file| {
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

async fn save_job_control(state: &ApiState, control: &JobControl) -> ApiResult<()> {
    require_service(state)?
        .control_store()
        .save_job(&control.import_id, control)
        .await
        .map_err(ApiError::Storage)
}

async fn load_job_control(state: &ApiState, import_id: &ImportId) -> ApiResult<JobControl> {
    require_service(state)?
        .control_store()
        .load_job(import_id)
        .await
        .map_err(ApiError::Storage)
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
    require_service(state)?
        .control_store()
        .list_jobs()
        .await
        .map_err(ApiError::Storage)
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
    let hash = request_hash(operation, request)?;
    if let Some(record) = require_service(state)?
        .control_store()
        .load_request::<IdempotencyRecord>(owner, key)
        .await
        .map_err(ApiError::Storage)?
    {
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
    require_service(state)?
        .control_store()
        .save_request(
            owner,
            key,
            &IdempotencyRecord::Pending {
                operation: operation.to_string(),
                request_hash: hash,
            },
        )
        .await
        .map_err(ApiError::Storage)?;
    Ok(Idempotency::Execute)
}

async fn complete_idempotency<R: Serialize>(
    state: &ApiState,
    owner: &UserId,
    key: &str,
    operation: &str,
    response: &R,
) -> ApiResult<()> {
    let record = require_service(state)?
        .control_store()
        .load_request::<IdempotencyRecord>(owner, key)
        .await
        .map_err(ApiError::Storage)?
        .ok_or_else(|| ApiError::NotFound("import idempotency record".to_string()))?;
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
    require_service(state)?
        .control_store()
        .save_request(
            owner,
            key,
            &IdempotencyRecord::Complete {
                operation: operation.to_string(),
                request_hash,
                response: serde_json::to_value(response)?,
            },
        )
        .await
        .map_err(ApiError::Storage)
}
