use axum::{
    Json,
    extract::{Multipart, Path, Query, State},
    http::HeaderMap,
};
use labello_client::{IngestJob, IngestJobStatus};
use labello_domain::{DatasetId, DatasetRole};

use crate::{
    ApiState,
    auth::{actor_from_headers, ensure_dataset_role},
    error::{ApiError, ApiResult},
};

const MAX_INGEST_REPORT_DETAILS: usize = 100;

pub(super) async fn ingest_dataset(
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

pub(super) async fn start_ingest_job(
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

pub(super) async fn get_ingest_job(
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
pub(super) struct UploadQuery {
    root: String,
    #[serde(default = "default_upload_ingest")]
    ingest: bool,
}

fn default_upload_ingest() -> bool {
    true
}

pub(super) async fn upload_images(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_ingest_report_details() {
        let report = labello_storage::IngestReport {
            duplicate_files: (0..150)
                .map(|index| labello_storage::DuplicateImage {
                    image_id: labello_domain::ImageId::from(format!("img_{index}")),
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
