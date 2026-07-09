use labello_domain::{
    AdjudicationRecord, Assignment, DatasetId, DatasetMetadata, DatasetStats, EventLogEntry,
    ImageId, ImageRecord, ImageState, KeybindingSet, OfflineBundle, OfflineSyncRequest,
    OfflineSyncResult, PrelabelConfig, PrelabelSuggestion, ReviewRecord, TaskDefinition,
    UserAccount, UserId,
};
use reqwest::{Method, RequestBuilder, Response, header::CONTENT_TYPE};
use serde::{Serialize, de::DeserializeOwned};
use url::Url;

use crate::{
    AdjudicationApi, AnnotationApi, AppendEventRequest, AssignNextRequest, AuthApi, ClientError,
    ClientResult, CorrectionRequest, CreateDatasetRequest, DatasetApi, DatasetSummary, ImageApi,
    ImageFile, ImagePreview, IngestReport, KeybindingApi, OAuthCallbackRequest, OAuthLoginRequest,
    OfflineApi, OfflineBundleRequest, PrelabelApi, PrelabelSuggestionRequest, ReviewApi, StatsApi,
    TaskApi, UpdateDatasetConfigRequest,
};

#[derive(Clone, Debug, Default)]
pub struct AuthHeaders {
    pub user_id: Option<UserId>,
    pub role: Option<labello_domain::DatasetRole>,
    pub dev_token: Option<String>,
}

#[derive(Clone)]
pub struct HttpLabelloApi {
    base_url: Url,
    client: reqwest::Client,
    auth: AuthHeaders,
}

impl HttpLabelloApi {
    pub fn new(base_url: impl AsRef<str>) -> ClientResult<Self> {
        Ok(Self {
            base_url: Url::parse(base_url.as_ref())?,
            client: reqwest::Client::new(),
            auth: AuthHeaders::default(),
        })
    }

    pub fn with_auth(mut self, auth: AuthHeaders) -> Self {
        self.auth = auth;
        self
    }

    fn endpoint(&self, path: &str) -> ClientResult<Url> {
        Ok(self.base_url.join(path.trim_start_matches('/'))?)
    }

    fn request(&self, method: Method, path: &str) -> ClientResult<RequestBuilder> {
        let mut request = self.client.request(method, self.endpoint(path)?);
        if let Some(user_id) = &self.auth.user_id {
            request = request.header("x-user-id", user_id.as_str());
        }
        if let Some(role) = &self.auth.role {
            request = request.header("x-user-role", role.to_string());
        }
        if let Some(token) = &self.auth.dev_token {
            request = request.header("x-dev-token", token);
        }
        Ok(request)
    }

    async fn json<T: DeserializeOwned>(response: Response) -> ClientResult<T> {
        let status = response.status();
        if status.is_success() {
            Ok(response.json().await?)
        } else {
            let message = response.text().await.unwrap_or_else(|_| status.to_string());
            Err(ClientError::Api {
                status: status.as_u16(),
                message,
            })
        }
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
    fn assign_next_image<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: AssignNextRequest,
    ) -> crate::ApiFuture<'a, Option<Assignment>> {
        Box::pin(async move {
            let mut path = format!(
                "/datasets/{dataset_id}/images/next?task_id={}",
                urlencoding::encode(request.task_id.as_str())
            );
            if let Some(kind) = request.kind {
                let kind = serde_json::to_value(kind)?
                    .as_str()
                    .unwrap_or("annotation")
                    .to_string();
                path.push_str("&kind=");
                path.push_str(&urlencoding::encode(&kind));
            }
            let response = self.request(Method::POST, &path)?.send().await?;
            Self::json(response).await
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
            let status = response.status();
            if !status.is_success() {
                let message = response.text().await.unwrap_or_else(|_| status.to_string());
                return Err(ClientError::Api {
                    status: status.as_u16(),
                    message,
                });
            }
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
            let status = response.status();
            if !status.is_success() {
                let message = response.text().await.unwrap_or_else(|_| status.to_string());
                return Err(ClientError::Api {
                    status: status.as_u16(),
                    message,
                });
            }
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
}

impl ReviewApi for HttpLabelloApi {
    fn record_review<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        review: ReviewRecord,
    ) -> crate::ApiFuture<'a, EventLogEntry> {
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
}

impl OfflineApi for HttpLabelloApi {
    fn offline_bundle<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: OfflineBundleRequest,
    ) -> crate::ApiFuture<'a, OfflineBundle> {
        Box::pin(async move {
            let path = format!(
                "/datasets/{dataset_id}/offline-bundle?limit={}&include_image_bytes={}",
                request.limit, request.include_image_bytes
            );
            Self::json(self.request(Method::GET, &path)?.send().await?).await
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
    fn github_login_url<'a>(&'a self, request: OAuthLoginRequest) -> crate::ApiFuture<'a, String> {
        Box::pin(async move {
            let state = request.state.unwrap_or_else(|| "labello".to_string());
            let response = self
                .request(
                    Method::GET,
                    &format!("/auth/github/login?state={}", urlencoding::encode(&state)),
                )?
                .send()
                .await?;
            Ok(response.url().to_string())
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
                        "/auth/github/callback?code={}",
                        urlencoding::encode(&request.code)
                    ),
                )?
                .send()
                .await?,
            )
            .await
        })
    }
}
