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
const ASSIGNMENT_AVAILABILITY_REQUEST_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(30);
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

include!("http/datasets.rs");
include!("http/imports.rs");
include!("http/workflow.rs");
include!("http/administration.rs");
include!("http/auth.rs");

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
            migration_result.clone(),
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
        api.revisit_migration_target(
            &DatasetId::from("animals"),
            &ImageId::from("img_1"),
            crate::RevisitMigrationTargetRequest {
                assignment_id: labello_domain::AssignmentId::from("asg_1"),
                pass_id: None,
                target: crate::MigrationTargetExpectation {
                    object_group_id: labello_domain::ObjectGroupId::from("group_1"),
                    expected_guide_annotation_version: 1,
                    expected_guide_deleted: false,
                    expected_disposition_version: 1,
                    expected_skeleton_version: None,
                },
            },
            "revisit-key",
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
            "POST /datasets/animals/images/img_1/migration/revisit ",
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
            (16, "revisit-key"),
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
