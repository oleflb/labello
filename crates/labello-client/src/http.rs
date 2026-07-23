use labello_domain::{
    AdjudicationRecord, Assignment, DatasetId, DatasetMetadata, DatasetSnapshot, DatasetStats,
    EventLogEntry, ImageExplorerPage, ImageId, ImageRecord, ImageState, KeybindingSet,
    OfflineBundle, OfflineSyncRequest, OfflineSyncResult, PrelabelConfig, PrelabelSuggestion,
    ReviewRecord, TaskDefinition, UserAccount, UserId,
};
use reqwest::{Method, RequestBuilder, Response, header::CONTENT_TYPE};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use url::Url;

const STATS_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
const REQUEST_ID_HEADER: &str = "x-request-id";

#[derive(Deserialize)]
struct ErrorBody {
    error: String,
}

use crate::{
    AdjudicationApi, AnnotationApi, AnnotationBatchRequest, AppendEventRequest, AssignNextRequest,
    AssignmentActionRequest, AuthApi, AuthOptions, ClientError, ClientResult, CorrectionRequest,
    CreateDatasetRequest, DatasetApi, DatasetSummary, DatasetUser, ImageApi, ImageExplorerQuery,
    ImageFile, ImagePreview, IngestJob, IngestReport, KeybindingApi, OAuthCallbackRequest,
    OAuthLoginRequest, OfflineApi, OfflineBundleRequest, PrelabelApi, PrelabelSuggestionRequest,
    ReviewApi, SetDatasetRolesRequest, StatsApi, TaskApi, UpdateDatasetConfigRequest, UserApi,
};

#[derive(Clone)]
pub struct HttpLabelloApi {
    base_url: Url,
    client: reqwest::Client,
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
        })
    }

    fn endpoint(&self, path: &str) -> ClientResult<Url> {
        Ok(self.base_url.join(path.trim_start_matches('/'))?)
    }

    fn request(&self, method: Method, path: &str) -> ClientResult<RequestBuilder> {
        let request = self.client.request(method, self.endpoint(path)?);
        #[cfg(target_arch = "wasm32")]
        let request = request.fetch_credentials_include();
        Ok(request)
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

    async fn send_json<T: DeserializeOwned, B: Serialize + ?Sized>(
        request: RequestBuilder,
        body: &B,
    ) -> ClientResult<T> {
        Self::json(request.json(body).send().await?).await
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
            Self::json(response).await
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
    fn auth_options<'a>(&'a self) -> crate::ApiFuture<'a, AuthOptions> {
        Box::pin(async move {
            Self::json(self.request(Method::GET, "/auth/options")?.send().await?).await
        })
    }

    fn local_admin_login<'a>(&'a self) -> crate::ApiFuture<'a, UserAccount> {
        Box::pin(async move {
            Self::json(
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

    fn me<'a>(&'a self) -> crate::ApiFuture<'a, UserAccount> {
        Box::pin(async move { Self::json(self.request(Method::GET, "/me")?.send().await?).await })
    }

    fn logout<'a>(&'a self) -> crate::ApiFuture<'a, ()> {
        Box::pin(async move {
            let response = self.request(Method::POST, "/logout")?.send().await?;
            Self::ensure_success(response).await.map(|_| ())
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
        let account_json = serde_json::to_string(&account).unwrap();
        let server = std::thread::spawn(move || {
            let responses = [
                (
                    r#"{"githubOauth":true,"localAdminLogin":true}"#.to_string(),
                    "",
                ),
                (
                    account_json.clone(),
                    "Set-Cookie: labello_session=test-session; Path=/; HttpOnly\r\n",
                ),
                (account_json, ""),
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

        let api = HttpLabelloApi::new(format!("http://{address}")).unwrap();
        assert_eq!(
            api.auth_options().await.unwrap(),
            AuthOptions {
                github_oauth: true,
                local_admin_login: true,
            }
        );
        assert_eq!(api.local_admin_login().await.unwrap(), account);
        assert_eq!(api.me().await.unwrap(), account);

        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("GET /auth/options HTTP/1.1\r\n"));
        assert!(requests[1].starts_with("POST /auth/local-admin HTTP/1.1\r\n"));
        assert!(
            requests[2]
                .to_ascii_lowercase()
                .contains("cookie: labello_session=test-session\r\n")
        );
    }
}
