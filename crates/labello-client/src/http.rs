use labello_domain::{
    AdjudicationRecord, Assignment, DatasetId, DatasetMetadata, DatasetSnapshot, DatasetStats,
    EventLogEntry, ImageExplorerPage, ImageId, ImageRecord, ImageState, ImportId, KeybindingSet,
    OfflineBundle, OfflineSyncRequest, OfflineSyncResult, PrelabelConfig, PrelabelSuggestion,
    ReviewRecord, TaskDefinition, UserAccount, UserId,
};
use reqwest::{Method, RequestBuilder, Response, header::CONTENT_TYPE};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use url::Url;

const STATS_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
const REQUEST_ID_HEADER: &str = "x-request-id";
const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";
const UPLOAD_OFFSET_HEADER: &str = "upload-offset";
const UPLOAD_LENGTH_HEADER: &str = "upload-length";
const DIGEST_HEADER: &str = "digest";

#[derive(Deserialize)]
struct ErrorBody {
    error: String,
}

use crate::{
    AdjudicationApi, AnnotationApi, AnnotationBatchRequest, AppendEventRequest, AssignNextRequest,
    AssignmentActionRequest, AuthApi, AuthOptions, ClientError, ClientResult, CorrectionRequest,
    CreateDatasetRequest, DatasetApi, DatasetSummary, DatasetUser, ImageApi, ImageExplorerQuery,
    ImageFile, ImagePreview, ImportApi, IngestJob, IngestReport, KeybindingApi,
    OAuthCallbackRequest, OAuthLoginRequest, OfflineApi, OfflineBundleRequest, PrelabelApi,
    PrelabelSuggestionRequest, ReviewApi, SessionInfo, SetDatasetRolesRequest, StatsApi, TaskApi,
    UpdateDatasetConfigRequest, UserApi,
};

#[derive(Clone)]
pub struct HttpLabelloApi {
    base_url: Url,
    client: reqwest::Client,
    csrf_token: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    request_origin: Option<String>,
}

impl HttpLabelloApi {
    pub fn new(base_url: impl AsRef<str>) -> ClientResult<Self> {
        #[cfg(not(target_arch = "wasm32"))]
        let client = reqwest::Client::builder().cookie_store(true).build()?;
        #[cfg(target_arch = "wasm32")]
        let client = reqwest::Client::new();

        Ok(Self {
            base_url: Url::parse(base_url.as_ref())?,
            client,
            csrf_token: Default::default(),
            request_origin: None,
        })
    }

    pub fn with_origin(mut self, origin: impl AsRef<str>) -> ClientResult<Self> {
        let origin = Url::parse(origin.as_ref())?;
        if !matches!(origin.scheme(), "http" | "https") || origin.host_str().is_none() {
            return Err(ClientError::Api {
                status: 0,
                message: "request origin must be an http(s) URL with a host".to_string(),
            });
        }
        self.request_origin = Some(origin.origin().ascii_serialization());
        Ok(self)
    }

    fn endpoint(&self, path: &str) -> ClientResult<Url> {
        Ok(self.base_url.join(path.trim_start_matches('/'))?)
    }

    fn request(&self, method: Method, path: &str) -> ClientResult<RequestBuilder> {
        let unsafe_method = !matches!(method.as_str(), "GET" | "HEAD" | "OPTIONS");
        let mut request = self.client.request(method, self.endpoint(path)?);
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(origin) = &self.request_origin {
            request = request.header(reqwest::header::ORIGIN, origin);
        }
        if unsafe_method && let Some(token) = self.csrf_token() {
            request = request.header("x-csrf-token", token);
        }
        #[cfg(target_arch = "wasm32")]
        let request = request.fetch_credentials_include();
        Ok(request)
    }

    fn remember_session(&self, session: &SessionInfo) {
        *self
            .csrf_token
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(session.csrf_token.clone());
    }

    fn clear_csrf_token(&self) {
        *self
            .csrf_token
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    async fn session(&self, response: Response) -> ClientResult<SessionInfo> {
        let session = Self::json(response).await?;
        self.remember_session(&session);
        Ok(session)
    }

    async fn ensure_success(response: Response) -> ClientResult<Response> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let request_id = response_request_id(&response);
        let body = response.text().await.unwrap_or_default();
        let message = serde_json::from_str::<ErrorBody>(&body)
            .ok()
            .map(|body| body.error)
            .filter(|message| !message.trim().is_empty())
            .unwrap_or_else(|| {
                if body.trim().is_empty() {
                    status.to_string()
                } else {
                    body
                }
            });
        let message = match &request_id {
            Some(request_id) => format!("{message} (request ID: {request_id})"),
            None => message,
        };
        if status.is_server_error() {
            tracing::error!(
                event = "http.client.failed",
                outcome = "api_error",
                status = status.as_u16(),
                request_id = request_id.as_deref().unwrap_or("<missing>"),
                "API request failed"
            );
        } else if status == reqwest::StatusCode::UNAUTHORIZED {
            tracing::debug!(
                event = "http.client.rejected",
                outcome = "api_error",
                status = status.as_u16(),
                request_id = request_id.as_deref().unwrap_or("<missing>"),
                "API request requires authentication"
            );
        } else {
            tracing::warn!(
                event = "http.client.rejected",
                outcome = "api_error",
                status = status.as_u16(),
                request_id = request_id.as_deref().unwrap_or("<missing>"),
                "API request was rejected"
            );
        }
        Err(ClientError::Api {
            status: status.as_u16(),
            message,
        })
    }

    async fn json<T: DeserializeOwned>(response: Response) -> ClientResult<T> {
        Ok(Self::ensure_success(response).await?.json().await?)
    }

    async fn versioned_json<T: labello_domain::VersionedArtifact>(
        response: Response,
    ) -> ClientResult<T> {
        let bytes = Self::ensure_success(response).await?.bytes().await?;
        labello_domain::deserialize_current_artifact(&bytes)
            .map_err(|error| ClientError::SchemaArtifact(error.to_string()))
    }

    async fn send_json<T: DeserializeOwned, B: Serialize + ?Sized>(
        request: RequestBuilder,
        body: &B,
    ) -> ClientResult<T> {
        Self::json(request.json(body).send().await?).await
    }

    fn idempotent(request: RequestBuilder, idempotency_key: &str) -> RequestBuilder {
        request.header(IDEMPOTENCY_KEY_HEADER, idempotency_key)
    }
}

impl DatasetApi for HttpLabelloApi {
    fn list_datasets<'a>(&'a self) -> crate::ApiFuture<'a, Vec<DatasetSummary>> {
        Box::pin(
            async move { Self::json(self.request(Method::GET, "/datasets")?.send().await?).await },
        )
    }

    fn create_dataset<'a>(
        &'a self,
        request: CreateDatasetRequest,
    ) -> crate::ApiFuture<'a, DatasetMetadata> {
        Box::pin(async move {
            Self::send_json(self.request(Method::POST, "/datasets")?, &request).await
        })
    }

    fn get_dataset<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, DatasetMetadata> {
        Box::pin(async move {
            Self::json(
                self.request(Method::GET, &format!("/datasets/{dataset_id}"))?
                    .send()
                    .await?,
            )
            .await
        })
    }

    fn get_admin_dataset<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, DatasetMetadata> {
        Box::pin(async move {
            Self::json(
                self.request(Method::GET, &format!("/datasets/{dataset_id}/admin"))?
                    .send()
                    .await?,
            )
            .await
        })
    }

    fn update_dataset_config<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: UpdateDatasetConfigRequest,
    ) -> crate::ApiFuture<'a, DatasetMetadata> {
        Box::pin(async move {
            Self::send_json(
                self.request(Method::PUT, &format!("/datasets/{dataset_id}/admin"))?,
                &request,
            )
            .await
        })
    }

    fn ingest_dataset<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, IngestReport> {
        Box::pin(async move {
            Self::json(
                self.request(Method::POST, &format!("/datasets/{dataset_id}/ingest"))?
                    .send()
                    .await?,
            )
            .await
        })
    }

    fn start_ingest_job<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, IngestJob> {
        Box::pin(async move {
            Self::json(
                self.request(Method::POST, &format!("/datasets/{dataset_id}/ingest-jobs"))?
                    .send()
                    .await?,
            )
            .await
        })
    }

    fn get_ingest_job<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        job_id: &'a str,
    ) -> crate::ApiFuture<'a, IngestJob> {
        Box::pin(async move {
            Self::json(
                self.request(
                    Method::GET,
                    &format!("/datasets/{dataset_id}/ingest-jobs/{job_id}"),
                )?
                .send()
                .await?,
            )
            .await
        })
    }

    fn create_snapshot<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, DatasetSnapshot> {
        Box::pin(async move {
            Self::json(
                self.request(Method::POST, &format!("/datasets/{dataset_id}/snapshots"))?
                    .send()
                    .await?,
            )
            .await
        })
    }

    fn list_snapshots<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, Vec<DatasetSnapshot>> {
        Box::pin(async move {
            Self::json(
                self.request(Method::GET, &format!("/datasets/{dataset_id}/snapshots"))?
                    .send()
                    .await?,
            )
            .await
        })
    }

    fn get_snapshot_file<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        snapshot_id: &'a str,
        path: &'a str,
    ) -> crate::ApiFuture<'a, crate::SnapshotFile> {
        Box::pin(async move {
            let encoded_path = path
                .split('/')
                .map(|part| urlencoding::encode(part))
                .collect::<Vec<_>>()
                .join("/");
            let response = self
                .request(
                    Method::GET,
                    &format!("/datasets/{dataset_id}/snapshots/{snapshot_id}/files/{encoded_path}"),
                )?
                .send()
                .await?;
            let response = Self::ensure_success(response).await?;
            let media_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("application/octet-stream")
                .to_string();
            Ok(crate::SnapshotFile {
                file_name: path.to_string(),
                media_type,
                bytes: response.bytes().await?.to_vec(),
            })
        })
    }
}

impl ImportApi for HttpLabelloApi {
    fn import_capabilities<'a>(&'a self) -> crate::ApiFuture<'a, crate::ImportCapabilities> {
        Box::pin(async move {
            Self::json(
                self.request(Method::GET, "/import-capabilities")?
                    .send()
                    .await?,
            )
            .await
        })
    }

    fn browse_server_import_root<'a>(
        &'a self,
        root_id: &'a str,
        request: crate::BrowseServerImportRootRequest,
    ) -> crate::ApiFuture<'a, crate::ImportBrowsePage> {
        Box::pin(async move {
            let root_id = urlencoding::encode(root_id);
            Self::send_json(
                self.request(Method::POST, &format!("/import-roots/{root_id}/browse"))?,
                &request,
            )
            .await
        })
    }

    fn create_import<'a>(
        &'a self,
        request: crate::CreateImportRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ImportJob> {
        Box::pin(async move {
            Self::send_json(
                Self::idempotent(self.request(Method::POST, "/imports")?, idempotency_key),
                &request,
            )
            .await
        })
    }

    fn get_import<'a>(&'a self, import_id: &'a ImportId) -> crate::ApiFuture<'a, crate::ImportJob> {
        Box::pin(async move {
            Self::json(
                self.request(Method::GET, &format!("/imports/{import_id}"))?
                    .send()
                    .await?,
            )
            .await
        })
    }

    fn register_import_files<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: crate::RegisterImportFilesRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::RegisterImportFilesResult> {
        Box::pin(async move {
            Self::send_json(
                Self::idempotent(
                    self.request(
                        Method::POST,
                        &format!("/imports/{import_id}/files/register"),
                    )?,
                    idempotency_key,
                ),
                &request,
            )
            .await
        })
    }

    fn upload_import_chunk<'a>(
        &'a self,
        import_id: &'a ImportId,
        file_id: &'a str,
        upload: crate::ImportChunkUpload,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ImportChunkResult> {
        Box::pin(async move {
            let file_id = urlencoding::encode(file_id);
            let request = self.request(
                Method::POST,
                &format!("/imports/{import_id}/files/{file_id}/chunks"),
            )?;
            let response = Self::idempotent(request, idempotency_key)
                .header(CONTENT_TYPE, "application/octet-stream")
                .header(UPLOAD_OFFSET_HEADER, upload.offset)
                .header(UPLOAD_LENGTH_HEADER, upload.length)
                .header(DIGEST_HEADER, upload.digest)
                .body(upload.bytes)
                .send()
                .await?;
            Self::json(response).await
        })
    }

    fn browse_import_source<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: crate::BrowseImportSourceRequest,
    ) -> crate::ApiFuture<'a, crate::ImportBrowsePage> {
        Box::pin(async move {
            Self::send_json(
                self.request(Method::POST, &format!("/imports/{import_id}/source/browse"))?,
                &request,
            )
            .await
        })
    }

    fn inspect_yolo_descriptor<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: crate::InspectYoloDescriptorRequest,
    ) -> crate::ApiFuture<'a, crate::YoloDescriptorInspection> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!("/imports/{import_id}/yolo-descriptor/inspect"),
                )?,
                &request,
            )
            .await
        })
    }

    fn seal_import<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: crate::SealImportRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::SealImportResult> {
        Box::pin(async move {
            Self::send_json(
                Self::idempotent(
                    self.request(Method::POST, &format!("/imports/{import_id}/seal"))?,
                    idempotency_key,
                ),
                &request,
            )
            .await
        })
    }

    fn preflight_import<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: crate::StartImportPreflightRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ImportJob> {
        Box::pin(async move {
            Self::send_json(
                Self::idempotent(
                    self.request(Method::POST, &format!("/imports/{import_id}/preflight"))?,
                    idempotency_key,
                ),
                &request,
            )
            .await
        })
    }

    fn update_import_plan<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: crate::UpdateImportPlanRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ImportPlan> {
        Box::pin(async move {
            Self::send_json(
                Self::idempotent(
                    self.request(Method::PUT, &format!("/imports/{import_id}/plan"))?,
                    idempotency_key,
                ),
                &request,
            )
            .await
        })
    }

    fn import_diagnostics<'a>(
        &'a self,
        import_id: &'a ImportId,
        query: crate::ImportDiagnosticsQuery,
    ) -> crate::ApiFuture<'a, crate::ImportDiagnosticsPage> {
        Box::pin(async move {
            let response = self
                .request(Method::GET, &format!("/imports/{import_id}/diagnostics"))?
                .query(&query)
                .send()
                .await?;
            Self::json(response).await
        })
    }

    fn commit_import<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: crate::CommitImportRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::CommitImportResult> {
        Box::pin(async move {
            Self::send_json(
                Self::idempotent(
                    self.request(Method::POST, &format!("/imports/{import_id}/commit"))?,
                    idempotency_key,
                ),
                &request,
            )
            .await
        })
    }

    fn cancel_import<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: crate::CancelImportRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::CancelImportResult> {
        Box::pin(async move {
            Self::send_json(
                Self::idempotent(
                    self.request(Method::POST, &format!("/imports/{import_id}/cancel"))?,
                    idempotency_key,
                ),
                &request,
            )
            .await
        })
    }

    fn save_migration_skeleton<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: crate::SaveMigrationSkeletonRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult> {
        self.migration_json(dataset_id, image_id, "skeleton", request, idempotency_key)
    }

    fn exclude_migration_target<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: crate::ExcludeMigrationTargetRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult> {
        self.migration_json(dataset_id, image_id, "exclude", request, idempotency_key)
    }

    fn reopen_migration_target<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: crate::ReopenMigrationTargetRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult> {
        self.migration_json(dataset_id, image_id, "reopen", request, idempotency_key)
    }

    fn start_migration_pass<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: crate::StartMigrationPassRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult> {
        self.migration_json(dataset_id, image_id, "passes", request, idempotency_key)
    }

    fn keep_migration_target<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: crate::KeepMigrationTargetRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult> {
        self.migration_json(dataset_id, image_id, "keep", request, idempotency_key)
    }

    fn confirm_migration<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: crate::ConfirmMigrationRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult> {
        self.migration_json(dataset_id, image_id, "confirm", request, idempotency_key)
    }

    fn review_migration<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: crate::ReviewMigrationRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult> {
        self.migration_json(dataset_id, image_id, "review", request, idempotency_key)
    }
}

impl HttpLabelloApi {
    fn migration_json<'a, B>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        command: &'a str,
        body: B,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult>
    where
        B: Serialize + 'a,
    {
        Box::pin(async move {
            Self::send_json(
                Self::idempotent(
                    self.request(
                        Method::POST,
                        &format!("/datasets/{dataset_id}/images/{image_id}/migration/{command}"),
                    )?,
                    idempotency_key,
                ),
                &body,
            )
            .await
        })
    }
}

impl TaskApi for HttpLabelloApi {
    fn list_tasks<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, Vec<TaskDefinition>> {
        Box::pin(async move {
            Self::json(
                self.request(Method::GET, &format!("/datasets/{dataset_id}/tasks"))?
                    .send()
                    .await?,
            )
            .await
        })
    }

    fn add_task<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        task: TaskDefinition,
    ) -> crate::ApiFuture<'a, TaskDefinition> {
        Box::pin(async move {
            Self::send_json(
                self.request(Method::POST, &format!("/datasets/{dataset_id}/tasks"))?,
                &task,
            )
            .await
        })
    }
}

impl ImageApi for HttpLabelloApi {
    fn assignment_availability<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: crate::AssignmentAvailabilityRequest,
    ) -> crate::ApiFuture<'a, crate::AssignmentAvailability> {
        Box::pin(async move {
            let response = self
                .request(
                    Method::GET,
                    &format!("/datasets/{dataset_id}/assignments/availability"),
                )?
                .query(&request)
                .send()
                .await?;
            Self::json(response).await
        })
    }

    fn list_images<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        query: ImageExplorerQuery,
    ) -> crate::ApiFuture<'a, ImageExplorerPage> {
        Box::pin(async move {
            let response = self
                .request(Method::GET, &format!("/datasets/{dataset_id}/images"))?
                .query(&query)
                .send()
                .await?;
            Self::json(response).await
        })
    }

    fn assign_next_image<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: AssignNextRequest,
    ) -> crate::ApiFuture<'a, Option<Assignment>> {
        Box::pin(async move {
            Self::send_json(
                self.request(Method::POST, &format!("/datasets/{dataset_id}/images/next"))?,
                &request,
            )
            .await
        })
    }

    fn release_assignment<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: AssignmentActionRequest,
    ) -> crate::ApiFuture<'a, Assignment> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!("/datasets/{dataset_id}/assignments/release"),
                )?,
                &request,
            )
            .await
        })
    }

    fn complete_assignment<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: AssignmentActionRequest,
    ) -> crate::ApiFuture<'a, Assignment> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!("/datasets/{dataset_id}/assignments/complete"),
                )?,
                &request,
            )
            .await
        })
    }

    fn reopen_assignment<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: AssignmentActionRequest,
    ) -> crate::ApiFuture<'a, Assignment> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!("/datasets/{dataset_id}/assignments/reopen"),
                )?,
                &request,
            )
            .await
        })
    }

    fn get_image_state<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> crate::ApiFuture<'a, ImageState> {
        Box::pin(async move {
            Self::json(
                self.request(
                    Method::GET,
                    &format!("/datasets/{dataset_id}/images/{image_id}"),
                )?
                .send()
                .await?,
            )
            .await
        })
    }

    fn get_image_record<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> crate::ApiFuture<'a, ImageRecord> {
        Box::pin(async move {
            Self::json(
                self.request(
                    Method::GET,
                    &format!("/datasets/{dataset_id}/images/{image_id}/record"),
                )?
                .send()
                .await?,
            )
            .await
        })
    }

    fn get_image_file<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> crate::ApiFuture<'a, ImageFile> {
        Box::pin(async move {
            let response = self
                .request(
                    Method::GET,
                    &format!("/datasets/{dataset_id}/images/{image_id}/file"),
                )?
                .send()
                .await?;
            let response = Self::ensure_success(response).await?;
            let media_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("application/octet-stream")
                .to_string();
            Ok(ImageFile {
                image_id: image_id.clone(),
                media_type,
                bytes: response.bytes().await?.to_vec(),
            })
        })
    }

    fn get_image_preview<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        max_dimension: u32,
    ) -> crate::ApiFuture<'a, ImagePreview> {
        Box::pin(async move {
            let response = self
                .request(
                    Method::GET,
                    &format!(
                        "/datasets/{dataset_id}/images/{image_id}/preview?max={max_dimension}"
                    ),
                )?
                .send()
                .await?;
            let response = Self::ensure_success(response).await?;
            let width = preview_dimension(response.headers(), "x-image-width")?;
            let height = preview_dimension(response.headers(), "x-image-height")?;
            Ok(ImagePreview {
                image_id: image_id.clone(),
                width,
                height,
                rgba: response.bytes().await?.to_vec(),
            })
        })
    }

    fn rebuild_image<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> crate::ApiFuture<'a, ImageState> {
        Box::pin(async move {
            Self::json(
                self.request(
                    Method::POST,
                    &format!("/datasets/{dataset_id}/images/{image_id}/rebuild"),
                )?
                .send()
                .await?,
            )
            .await
        })
    }
}

fn preview_dimension(headers: &reqwest::header::HeaderMap, name: &str) -> ClientResult<u32> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| ClientError::Demo(format!("missing preview header {name}")))
}

impl AnnotationApi for HttpLabelloApi {
    fn append_event<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: AppendEventRequest,
    ) -> crate::ApiFuture<'a, EventLogEntry> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!("/datasets/{dataset_id}/images/{image_id}/events"),
                )?,
                &request,
            )
            .await
        })
    }

    fn append_assigned_event<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        request: AppendEventRequest,
    ) -> crate::ApiFuture<'a, EventLogEntry> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!(
                        "/datasets/{dataset_id}/images/{}/events",
                        assignment.image_id
                    ),
                )?
                .query(&assignment),
                &request,
            )
            .await
        })
    }

    fn apply_annotation_batch<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        request: AnnotationBatchRequest,
    ) -> crate::ApiFuture<'a, ImageState> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!(
                        "/datasets/{dataset_id}/images/{}/annotation-batch",
                        assignment.image_id
                    ),
                )?
                .query(&assignment),
                &request,
            )
            .await
        })
    }
}

impl ReviewApi for HttpLabelloApi {
    fn record_review<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        review: ReviewRecord,
    ) -> crate::ApiFuture<'a, ImageState> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!("/datasets/{dataset_id}/images/{image_id}/reviews"),
                )?,
                &review,
            )
            .await
        })
    }

    fn record_assigned_review<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        review: ReviewRecord,
    ) -> crate::ApiFuture<'a, ImageState> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!(
                        "/datasets/{dataset_id}/images/{}/reviews",
                        assignment.image_id
                    ),
                )?
                .query(&assignment),
                &review,
            )
            .await
        })
    }

    fn record_correction<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: CorrectionRequest,
    ) -> crate::ApiFuture<'a, EventLogEntry> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!("/datasets/{dataset_id}/images/{image_id}/corrections"),
                )?,
                &request,
            )
            .await
        })
    }

    fn record_assigned_correction<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        request: CorrectionRequest,
    ) -> crate::ApiFuture<'a, EventLogEntry> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!(
                        "/datasets/{dataset_id}/images/{}/corrections",
                        assignment.image_id
                    ),
                )?
                .query(&assignment),
                &request,
            )
            .await
        })
    }
}

impl AdjudicationApi for HttpLabelloApi {
    fn record_adjudication<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        adjudication: AdjudicationRecord,
    ) -> crate::ApiFuture<'a, EventLogEntry> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!("/datasets/{dataset_id}/images/{image_id}/adjudications"),
                )?,
                &adjudication,
            )
            .await
        })
    }

    fn record_assigned_adjudication<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        adjudication: AdjudicationRecord,
    ) -> crate::ApiFuture<'a, EventLogEntry> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!(
                        "/datasets/{dataset_id}/images/{}/adjudications",
                        assignment.image_id
                    ),
                )?
                .query(&assignment),
                &adjudication,
            )
            .await
        })
    }
}

impl OfflineApi for HttpLabelloApi {
    fn offline_bundle<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: OfflineBundleRequest,
    ) -> crate::ApiFuture<'a, OfflineBundle> {
        Box::pin(async move {
            let response = self
                .request(
                    Method::GET,
                    &format!("/datasets/{dataset_id}/offline-bundle"),
                )?
                .query(&request)
                .send()
                .await?;
            Self::versioned_json(response).await
        })
    }

    fn sync_offline_events<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: OfflineSyncRequest,
    ) -> crate::ApiFuture<'a, OfflineSyncResult> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!("/datasets/{dataset_id}/offline-sync"),
                )?,
                &request,
            )
            .await
        })
    }
}

impl StatsApi for HttpLabelloApi {
    fn dataset_stats<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, DatasetStats> {
        Box::pin(async move {
            Self::json(
                self.request(Method::GET, &format!("/datasets/{dataset_id}/stats"))?
                    .timeout(STATS_REQUEST_TIMEOUT)
                    .send()
                    .await?,
            )
            .await
        })
    }
}

impl KeybindingApi for HttpLabelloApi {
    fn get_keybindings<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        _user_id: &'a UserId,
    ) -> crate::ApiFuture<'a, KeybindingSet> {
        Box::pin(async move {
            Self::json(
                self.request(Method::GET, &format!("/datasets/{dataset_id}/keybindings"))?
                    .send()
                    .await?,
            )
            .await
        })
    }

    fn save_keybindings<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        keybindings: KeybindingSet,
    ) -> crate::ApiFuture<'a, KeybindingSet> {
        Box::pin(async move {
            Self::send_json(
                self.request(Method::PUT, &format!("/datasets/{dataset_id}/keybindings"))?,
                &keybindings,
            )
            .await
        })
    }
}

impl PrelabelApi for HttpLabelloApi {
    fn list_prelabel_configs<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, Vec<PrelabelConfig>> {
        Box::pin(async move {
            Self::json(
                self.request(Method::GET, &format!("/datasets/{dataset_id}/prelabels"))?
                    .send()
                    .await?,
            )
            .await
        })
    }

    fn add_prelabel_config<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        config: PrelabelConfig,
    ) -> crate::ApiFuture<'a, PrelabelConfig> {
        Box::pin(async move {
            Self::send_json(
                self.request(Method::POST, &format!("/datasets/{dataset_id}/prelabels"))?,
                &config,
            )
            .await
        })
    }

    fn prelabel_suggestions<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: PrelabelSuggestionRequest,
    ) -> crate::ApiFuture<'a, Vec<PrelabelSuggestion>> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!("/datasets/{dataset_id}/prelabel-suggestions"),
                )?,
                &request,
            )
            .await
        })
    }
}

impl AuthApi for HttpLabelloApi {
    fn csrf_token(&self) -> Option<String> {
        self.csrf_token
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn auth_options<'a>(&'a self) -> crate::ApiFuture<'a, AuthOptions> {
        Box::pin(async move {
            Self::json(self.request(Method::GET, "/auth/options")?.send().await?).await
        })
    }

    fn local_admin_login<'a>(&'a self) -> crate::ApiFuture<'a, SessionInfo> {
        Box::pin(async move {
            self.session(
                self.request(Method::POST, "/auth/local-admin")?
                    .send()
                    .await?,
            )
            .await
        })
    }

    fn github_login_url<'a>(&'a self, request: OAuthLoginRequest) -> crate::ApiFuture<'a, String> {
        Box::pin(async move {
            let mut url = self.endpoint("/auth/github/login")?;
            if let Some(return_to) = request.return_to {
                url.query_pairs_mut().append_pair("returnTo", &return_to);
            }
            Ok(url.to_string())
        })
    }

    fn github_callback<'a>(
        &'a self,
        request: OAuthCallbackRequest,
    ) -> crate::ApiFuture<'a, UserAccount> {
        Box::pin(async move {
            Self::json(
                self.request(
                    Method::GET,
                    &format!(
                        "/auth/github/callback?code={}&state={}",
                        urlencoding::encode(&request.code),
                        urlencoding::encode(&request.state)
                    ),
                )?
                .send()
                .await?,
            )
            .await
        })
    }

    fn me<'a>(&'a self) -> crate::ApiFuture<'a, SessionInfo> {
        Box::pin(async move {
            self.session(self.request(Method::GET, "/me")?.send().await?)
                .await
        })
    }

    fn logout<'a>(&'a self) -> crate::ApiFuture<'a, ()> {
        Box::pin(async move {
            let response = self.request(Method::POST, "/logout")?.send().await?;
            Self::ensure_success(response).await?;
            self.clear_csrf_token();
            Ok(())
        })
    }
}

fn response_request_id(response: &Response) -> Option<String> {
    response
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

impl UserApi for HttpLabelloApi {
    fn list_dataset_users<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, Vec<DatasetUser>> {
        Box::pin(async move {
            Self::json(
                self.request(Method::GET, &format!("/datasets/{dataset_id}/users"))?
                    .send()
                    .await?,
            )
            .await
        })
    }

    fn set_dataset_roles<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: SetDatasetRolesRequest,
    ) -> crate::ApiFuture<'a, DatasetUser> {
        Box::pin(async move {
            Self::send_json(
                self.request(Method::PUT, &format!("/datasets/{dataset_id}/roles"))?,
                &request,
            )
            .await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_arch = "wasm32"))]
    use std::io::{BufRead, BufReader, Write};
    #[cfg(not(target_arch = "wasm32"))]
    use std::net::TcpListener;

    #[cfg(not(target_arch = "wasm32"))]
    fn read_request(reader: &mut BufReader<std::net::TcpStream>) -> (String, Vec<u8>) {
        let mut request = String::new();
        let mut content_length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line.to_ascii_lowercase().starts_with("content-length:") {
                content_length = line.split_once(':').unwrap().1.trim().parse().unwrap();
            }
            request.push_str(&line);
            if line == "\r\n" {
                break;
            }
        }
        let mut body = vec![0; content_length];
        std::io::Read::read_exact(reader, &mut body).unwrap();
        (request, body)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn write_json_response(stream: &mut std::net::TcpStream, body: &str) {
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[tokio::test]
    async fn auth_requests_use_session_mode_and_retain_native_cookie() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let account = UserAccount {
            user_id: UserId::from("local_admin"),
            display_name: "Local Administrator".to_string(),
            github_user_id: None,
            github_login: None,
            created_at: labello_domain::now(),
            updated_at: labello_domain::now(),
        };
        let session = SessionInfo {
            account: account.clone(),
            can_create_datasets: true,
            csrf_token: "test-csrf-token".to_string(),
        };
        assert!(!format!("{session:?}").contains("test-csrf-token"));
        let session_json = serde_json::to_string(&session).unwrap();
        let server = std::thread::spawn(move || {
            let responses = [
                (
                    r#"{"githubOauth":true,"localAdminLogin":true}"#.to_string(),
                    "",
                ),
                (
                    session_json.clone(),
                    "Set-Cookie: labello_session=test-session; Path=/; HttpOnly\r\n",
                ),
                (session_json, ""),
                (String::new(), ""),
            ];
            let mut requests = Vec::new();
            for (body, extra_headers) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    request.push_str(&line);
                    if line == "\r\n" {
                        break;
                    }
                }
                requests.push(request);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });

        let api = HttpLabelloApi::new(format!("http://{address}"))
            .unwrap()
            .with_origin("http://127.0.0.1:8081")
            .unwrap();
        assert_eq!(
            api.auth_options().await.unwrap(),
            AuthOptions {
                github_oauth: true,
                local_admin_login: true,
            }
        );
        assert_eq!(api.local_admin_login().await.unwrap(), session);
        assert_eq!(api.me().await.unwrap(), session);
        api.logout().await.unwrap();
        assert_eq!(api.csrf_token(), None);

        let requests = server.join().unwrap();
        for request in &requests {
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("origin: http://127.0.0.1:8081\r\n")
            );
        }
        assert!(requests[0].starts_with("GET /auth/options HTTP/1.1\r\n"));
        assert!(requests[1].starts_with("POST /auth/local-admin HTTP/1.1\r\n"));
        assert!(
            requests[2]
                .to_ascii_lowercase()
                .contains("cookie: labello_session=test-session\r\n")
        );
        assert!(requests[3].starts_with("POST /logout HTTP/1.1\r\n"));
        assert!(
            requests[3]
                .to_ascii_lowercase()
                .contains("x-csrf-token: test-csrf-token\r\n")
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[tokio::test]
    async fn offline_bundle_upcasts_v2_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
            }
            let body = r#"{"schemaVersion":2,"datasetId":"ds_1","userId":"user_1","createdAt":"2026-01-02T03:04:05Z","expiresAt":null,"roles":["annotator"],"tasks":[],"images":[]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });

        let api = HttpLabelloApi::new(format!("http://{address}")).unwrap();
        let bundle = api
            .offline_bundle(&DatasetId::from("ds_1"), OfflineBundleRequest::default())
            .await
            .unwrap();

        assert_eq!(bundle.schema_version, labello_domain::SCHEMA_VERSION);
        assert!(bundle.import_manifests.is_empty());
        server.join().unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[tokio::test]
    async fn import_control_routes_send_csrf_idempotency_and_upload_headers() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let timestamp = "2026-01-02T03:04:05Z";
        let session = serde_json::json!({
            "account": {
                "userId": "admin",
                "displayName": "Administrator",
                "githubUserId": null,
                "githubLogin": null,
                "createdAt": timestamp,
                "updatedAt": timestamp
            },
            "canCreateDatasets": true,
            "csrfToken": "csrf-import"
        });
        let job = serde_json::json!({
            "importId": "imp_1",
            "ownerUserId": "admin",
            "destinationDatasetId": "animals",
            "destinationName": "Animals",
            "profile": "coco_instances_gt_v1",
            "transport": "browser_folder",
            "lifecycle": "registering",
            "createdAt": timestamp,
            "updatedAt": timestamp
        });
        let migration_result = serde_json::to_string(&crate::ManualMigrationCommandResult {
            image_state: ImageState::new(ImageId::from("img_1")),
            cursor: None,
            progress: Default::default(),
            active_pass: None,
            confirmation: None,
            assignment: None,
            annotation_id: None,
        })
        .unwrap();
        let responses = vec![
            session.to_string(),
            r#"{"available":true}"#.to_string(),
            r#"{"relativePath":"","entries":[]}"#.to_string(),
            job.to_string(),
            job.to_string(),
            r#"{"relativePath":"","entries":[]}"#.to_string(),
            r#"{"files":[]}"#.to_string(),
            r#"{"fileId":"opaque/file","acceptedOffset":4,"complete":true}"#.to_string(),
            r#"{"splits":[{"name":"train","usable":true}]}"#.to_string(),
            r#"{"importId":"imp_1","sourceFingerprint":"source"}"#.to_string(),
            job.to_string(),
            r#"{"importId":"imp_1","sourceFingerprint":"source","planHash":"plan","report":{}}"#
                .to_string(),
            r#"{"diagnostics":[],"total":0}"#.to_string(),
            r#"{"importId":"imp_1","datasetId":"animals","planHash":"plan"}"#.to_string(),
            r#"{"importId":"imp_1","lifecycle":"cancelled"}"#.to_string(),
            migration_result,
        ];
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for body in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                requests.push(read_request(&mut reader));
                write_json_response(&mut stream, &body);
            }
            requests
        });

        let api = HttpLabelloApi::new(format!("http://{address}")).unwrap();
        api.local_admin_login().await.unwrap();
        api.import_capabilities().await.unwrap();
        api.browse_server_import_root(
            "staging",
            crate::BrowseServerImportRootRequest {
                relative_path: String::new(),
                offset: 0,
            },
        )
        .await
        .unwrap();
        let import_id = ImportId::from("imp_1");
        let create = crate::CreateImportRequest {
            destination_dataset_id: DatasetId::from("animals"),
            destination_name: "Animals".to_string(),
            profile: crate::ImportProfile::CocoInstancesGtV1,
            source: crate::ImportSourceSelection::BrowserFolder,
            attestations: crate::ImportAttestations {
                ground_truth: true,
                exhaustive: true,
                coverage_scope: vec![],
                provenance: "release".to_string(),
            },
        };
        api.create_import(create, "create-key").await.unwrap();
        api.get_import(&import_id).await.unwrap();
        api.browse_import_source(
            &import_id,
            crate::BrowseImportSourceRequest {
                relative_path: String::new(),
                offset: 0,
                mode: crate::ImportSourceBrowseMode::Descriptors,
            },
        )
        .await
        .unwrap();
        api.register_import_files(
            &import_id,
            crate::RegisterImportFilesRequest { files: vec![] },
            "register-key",
        )
        .await
        .unwrap();
        api.upload_import_chunk(
            &import_id,
            "opaque/file",
            crate::ImportChunkUpload {
                offset: 0,
                length: 4,
                digest: "blake3=test".to_string(),
                bytes: b"DATA".to_vec(),
            },
            "chunk-key",
        )
        .await
        .unwrap();
        api.inspect_yolo_descriptor(
            &import_id,
            crate::InspectYoloDescriptorRequest {
                descriptor_file_id: "opaque/file".to_string(),
            },
        )
        .await
        .unwrap();
        api.seal_import(
            &import_id,
            crate::SealImportRequest {
                source: crate::ImportSourceConfiguration {
                    source_namespace: "release".to_string(),
                    descriptors: vec![],
                    selected_splits: vec!["train".to_string()],
                    selected_category_keys: vec![],
                },
                attestations: crate::ImportAttestations {
                    ground_truth: true,
                    exhaustive: true,
                    coverage_scope: vec![],
                    provenance: "release".to_string(),
                },
            },
            "seal-key",
        )
        .await
        .unwrap();
        api.preflight_import(
            &import_id,
            crate::StartImportPreflightRequest::default(),
            "preflight-key",
        )
        .await
        .unwrap();
        api.update_import_plan(
            &import_id,
            crate::UpdateImportPlanRequest {
                category_mappings: vec![],
                geometry_mappings: vec![],
                task_mappings: vec![],
                skeleton_mappings: vec![],
                compatibility: Default::default(),
                acknowledgements: vec![],
            },
            "plan-key",
        )
        .await
        .unwrap();
        api.import_diagnostics(&import_id, crate::ImportDiagnosticsQuery::default())
            .await
            .unwrap();
        api.commit_import(
            &import_id,
            crate::CommitImportRequest {
                plan_hash: "plan".to_string(),
            },
            "commit-key",
        )
        .await
        .unwrap();
        api.cancel_import(
            &import_id,
            crate::CancelImportRequest { reason: None },
            "cancel-key",
        )
        .await
        .unwrap();
        api.save_migration_skeleton(
            &DatasetId::from("animals"),
            &ImageId::from("img_1"),
            crate::SaveMigrationSkeletonRequest {
                assignment_id: labello_domain::AssignmentId::from("asg_1"),
                pass_id: None,
                target: crate::MigrationTargetExpectation {
                    object_group_id: labello_domain::ObjectGroupId::from("group_1"),
                    expected_guide_annotation_version: 1,
                    expected_guide_deleted: false,
                    expected_disposition_version: 1,
                    expected_skeleton_version: None,
                },
                skeleton: labello_domain::SkeletonGeometry { keypoints: vec![] },
            },
            "migration-key",
        )
        .await
        .unwrap();

        let requests = server.join().unwrap();
        let starts = [
            "POST /auth/local-admin ",
            "GET /import-capabilities ",
            "POST /import-roots/staging/browse ",
            "POST /imports ",
            "GET /imports/imp_1 ",
            "POST /imports/imp_1/source/browse ",
            "POST /imports/imp_1/files/register ",
            "POST /imports/imp_1/files/opaque%2Ffile/chunks ",
            "POST /imports/imp_1/yolo-descriptor/inspect ",
            "POST /imports/imp_1/seal ",
            "POST /imports/imp_1/preflight ",
            "PUT /imports/imp_1/plan ",
            "GET /imports/imp_1/diagnostics?limit=100 ",
            "POST /imports/imp_1/commit ",
            "POST /imports/imp_1/cancel ",
            "POST /datasets/animals/images/img_1/migration/skeleton ",
        ];
        for ((headers, _), start) in requests.iter().zip(starts) {
            assert!(headers.starts_with(start), "unexpected request: {headers}");
        }
        for (index, key) in [
            (3, "create-key"),
            (6, "register-key"),
            (7, "chunk-key"),
            (9, "seal-key"),
            (10, "preflight-key"),
            (11, "plan-key"),
            (13, "commit-key"),
            (14, "cancel-key"),
            (15, "migration-key"),
        ] {
            let headers = requests[index].0.to_ascii_lowercase();
            assert!(headers.contains("x-csrf-token: csrf-import\r\n"));
            assert!(headers.contains(&format!("idempotency-key: {key}\r\n")));
        }
        let chunk_headers = requests[7].0.to_ascii_lowercase();
        assert!(chunk_headers.contains("content-type: application/octet-stream\r\n"));
        assert!(chunk_headers.contains("upload-offset: 0\r\n"));
        assert!(chunk_headers.contains("upload-length: 4\r\n"));
        assert!(chunk_headers.contains("digest: blake3=test\r\n"));
        assert_eq!(requests[7].1, b"DATA");
        let inspection_headers = requests[8].0.to_ascii_lowercase();
        assert!(inspection_headers.contains("x-csrf-token: csrf-import\r\n"));
        assert!(!inspection_headers.contains("idempotency-key:"));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&requests[8].1).unwrap(),
            serde_json::json!({ "descriptorFileId": "opaque/file" })
        );
        for index in [2, 5] {
            let headers = requests[index].0.to_ascii_lowercase();
            assert!(headers.contains("x-csrf-token: csrf-import\r\n"));
            assert!(!headers.contains("idempotency-key:"));
        }
    }
}
