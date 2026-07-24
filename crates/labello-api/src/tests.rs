use axum::{
    body::{Body, to_bytes},
    http::{HeaderValue, Request, StatusCode, header},
    middleware::Next,
};
use image::{ImageBuffer, Rgba};
use labello_domain::{ImageId, UserAccount, UserId, now};
use serde_json::json;
use std::io::Cursor;
use tower::ServiceExt;

use crate::{ApiState, router as production_router};

fn router(state: ApiState) -> axum::Router {
    let session_state = state.clone();
    production_router(state).layer(axum::middleware::from_fn(
        move |mut request: Request<Body>, next: Next| {
            let state = session_state.clone();
            async move {
                let test_user = request
                    .headers_mut()
                    .remove("x-test-user-id")
                    .and_then(|value| value.to_str().ok().map(str::to_string));
                if request.headers().get(header::COOKIE).is_none()
                    && let Some(user_id) = test_user.map(UserId::from)
                {
                    let timestamp = now();
                    state
                        .server_store
                        .upsert_user(UserAccount {
                            user_id: user_id.clone(),
                            display_name: user_id.to_string(),
                            github_user_id: None,
                            github_login: None,
                            created_at: timestamp,
                            updated_at: timestamp,
                        })
                        .unwrap();
                    let token = state.create_session(user_id).unwrap();
                    request.headers_mut().insert(
                        header::COOKIE,
                        HeaderValue::from_str(&format!("labello_session={token}")).unwrap(),
                    );
                }
                next.run(request).await
            }
        },
    ))
}

#[tokio::test]
async fn responses_receive_and_propagate_request_ids() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let first_id = first.headers()["x-request-id"].to_str().unwrap();
    uuid::Uuid::parse_str(first_id).unwrap();

    let second = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/missing")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let second_id = second.headers()["x-request-id"].to_str().unwrap();
    uuid::Uuid::parse_str(second_id).unwrap();
    assert_ne!(first_id, second_id);

    let supplied = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .header("x-request-id", "test-request-id")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(supplied.headers()["x-request-id"], "test-request-id");
}

#[tokio::test]
async fn internal_errors_are_sanitized_and_correlated() {
    let temp = tempfile::tempdir().unwrap();
    let auth_dir = temp.path().join(".labello-server");
    std::fs::create_dir_all(&auth_dir).unwrap();
    std::fs::write(
        auth_dir.join("auth.json"),
        r#"{"private":"do-not-expose-this-sentinel""#,
    )
    .unwrap();
    let response = router(ApiState::new(temp.path()))
        .oneshot(
            Request::builder()
                .uri("/me")
                .header(header::COOKIE, "labello_session=invalid")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let request_id = response.headers()["x-request-id"].to_str().unwrap();
    uuid::Uuid::parse_str(request_id).unwrap();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body, json!({ "error": "internal server error" }));
    let body = body.to_string();
    assert!(!body.contains("do-not-expose-this-sentinel"));
    assert!(!body.contains(temp.path().to_string_lossy().as_ref()));
}

#[tokio::test]
async fn auth_options_are_public_and_only_advertise_capabilities() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(
        ApiState::new(temp.path())
            .with_local_admin_login(Some(labello_domain::UserId::from("local_admin")))
            .with_github_oauth(crate::GithubOAuthConfig {
                client_id: "client-id".to_string(),
                client_secret: "oauth-secret".to_string(),
                redirect_uri: "https://api.example.com/auth/github/callback".to_string(),
            }),
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/auth/options")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let options: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        options,
        json!({ "githubOauth": true, "localAdminLogin": true })
    );
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(!body.contains("oauth-secret"));
    assert!(!body.contains("local_admin"));
}

#[tokio::test]
async fn local_admin_login_is_not_found_when_disabled() {
    let temp = tempfile::tempdir().unwrap();
    let response = router(ApiState::new(temp.path()))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/local-admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn local_admin_login_creates_session_and_requires_configured_browser_origin() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(
        ApiState::new(temp.path())
            .with_browser_origins(vec!["https://app.example.com".to_string()])
            .unwrap()
            .with_session_cookie_secure(false)
            .with_bootstrap_admins([labello_domain::UserId::from("bootstrap_admin")])
            .with_local_admin_login(Some(labello_domain::UserId::from("bootstrap_admin"))),
    );

    let rejected = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/local-admin")
                .header(header::ORIGIN, "https://other.example.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);

    let login = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/local-admin")
                .header(header::ORIGIN, "https://app.example.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);
    let cookie = login.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .to_string();
    assert!(cookie.starts_with("labello_session="));
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Lax"));
    assert!(!cookie.contains("; Secure"));
    let body = to_bytes(login.into_body(), usize::MAX).await.unwrap();
    let session: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(session["account"]["userId"], "bootstrap_admin");
    assert_eq!(session["account"]["displayName"], "bootstrap_admin");
    assert!(session["account"]["githubUserId"].is_null());
    assert!(session["account"]["githubLogin"].is_null());
    assert_eq!(session["canCreateDatasets"], true);

    let me = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/me")
                .header(header::COOKIE, cookie.split(';').next().unwrap())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(me.status(), StatusCode::OK);
    let body = to_bytes(me.into_body(), usize::MAX).await.unwrap();
    let session: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(session["account"]["userId"], "bootstrap_admin");
    assert_eq!(session["canCreateDatasets"], true);

    let native_login = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/local-admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(native_login.status(), StatusCode::OK);
}

#[tokio::test]
async fn creates_dataset_and_requires_authentication() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets")
                .header("x-test-user-id", "admin")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "datasetId": "ds",
                        "name": "Dataset",
                        "adminUserId": "admin"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().contains_key("x-request-id"));

    let create_without_identity = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "datasetId": "ds2",
                        "name": "Dataset 2",
                        "adminUserId": "admin"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_without_identity.status(), StatusCode::UNAUTHORIZED);

    let unauthorized = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/datasets/ds")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let authorized = app
        .oneshot(
            Request::builder()
                .uri("/datasets/ds")
                .header("x-test-user-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(authorized.status(), StatusCode::OK);
    let body = to_bytes(authorized.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["datasetId"], "ds");
    assert!(value["roleAssignments"].is_array());
}

#[tokio::test]
async fn rejects_unsafe_dataset_ids_and_existing_datasets() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;

    let duplicate = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets")
                .header("x-test-user-id", "admin")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "datasetId": "ds",
                        "name": "Replacement",
                        "adminUserId": "admin"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);

    let unsafe_id = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets")
                .header("x-test-user-id", "admin")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "datasetId": "../escape",
                        "name": "Escape",
                        "adminUserId": "admin"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unsafe_id.status(), StatusCode::BAD_REQUEST);
    assert!(!temp.path().parent().unwrap().join("escape").exists());

    let unsafe_image_id = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/datasets/ds/images/bad%5Cid/record")
                .header("x-test-user-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unsafe_image_id.status(), StatusCode::BAD_REQUEST);

    let unsafe_user_id = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/datasets/ds/keybindings")
                .header("x-test-user-id", "../escape")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unsafe_user_id.status(), StatusCode::BAD_REQUEST);

    let existing = app
        .oneshot(
            Request::builder()
                .uri("/datasets/ds/admin")
                .header("x-test-user-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(existing.into_body(), usize::MAX).await.unwrap();
    let metadata: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(metadata["name"], "Dataset");
}

#[tokio::test]
async fn protects_admin_dataset_config() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets")
                .header("x-test-user-id", "admin")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "datasetId": "ds",
                        "name": "Dataset",
                        "adminUserId": "admin"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::OK);

    let non_admin = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/datasets/ds/admin")
                .header("x-test-user-id", "intruder")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(non_admin.status(), StatusCode::UNAUTHORIZED);

    let admin = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/datasets/ds/admin")
                .header("x-test-user-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(admin.status(), StatusCode::OK);
    let body = to_bytes(admin.into_body(), usize::MAX).await.unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["imageRoots"], json!(["images"]));

    value["name"] = json!("Updated Dataset");
    value["imageRoots"] = json!(["images", "imports/batch-1"]);
    let update = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/datasets/ds/admin")
                .header("x-test-user-id", "admin")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "name": value["name"],
                        "imageRoots": value["imageRoots"],
                        "labelClasses": value["labelClasses"],
                        "tasks": value["tasks"],
                        "roleAssignments": value["roleAssignments"],
                        "imbalance": value["imbalance"],
                        "prelabelConfigs": value["prelabelConfigs"]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(update.status(), StatusCode::OK);

    let mut duplicate_roles = value["roleAssignments"].as_array().unwrap().clone();
    duplicate_roles.push(duplicate_roles[0].clone());
    let duplicate = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/datasets/ds/admin")
                .header("x-test-user-id", "admin")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "name": value["name"],
                        "imageRoots": value["imageRoots"],
                        "labelClasses": value["labelClasses"],
                        "tasks": value["tasks"],
                        "roleAssignments": duplicate_roles,
                        "imbalance": value["imbalance"],
                        "prelabelConfigs": value["prelabelConfigs"]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(duplicate.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn development_headers_do_not_authenticate() {
    let temp = tempfile::tempdir().unwrap();
    let response = production_router(ApiState::new(temp.path()))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets")
                .header(axum::http::HeaderName::from_static("x-user-id"), "admin")
                .header("x-dev-token", "secret")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "datasetId": "ds",
                        "name": "Dataset",
                        "adminUserId": "admin"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn review_and_adjudication_actor_ids_must_match_caller() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    let timestamp = labello_domain::now().to_rfc3339();

    let review = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets/ds/images/img_1/reviews?assignmentId=asg_1&imageId=img_1&taskId=task_1&kind=review")
                .header("x-test-user-id", "admin")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "reviewId": "rev_1",
                        "target": {
                            "targetType": "task",
                            "task_id": "task_1"
                        },
                        "reviewerUserId": "someone_else",
                        "decision": "approved",
                        "timestamp": timestamp.clone(),
                        "comment": null
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let review_status = review.status();
    let review_body = to_bytes(review.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        review_status,
        StatusCode::UNAUTHORIZED,
        "{}",
        String::from_utf8_lossy(&review_body)
    );

    let adjudication = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets/ds/images/img_1/adjudications?assignmentId=asg_1&imageId=img_1&taskId=task_1&kind=adjudication")
                .header("x-test-user-id", "admin")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "adjudicationId": "adj_1",
                        "taskId": "task_1",
                        "annotationIds": [],
                        "adjudicatorUserId": "someone_else",
                        "decision": "accept_annotation",
                        "resolution": "accepted",
                        "timestamp": timestamp
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(adjudication.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn offline_sync_is_authenticated_and_bound_to_caller() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    let body = json!({
        "schemaVersion": 2,
        "datasetId": "ds",
        "userId": "admin",
        "fragments": []
    })
    .to_string();

    let unauthenticated = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets/ds/offline-sync")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

    let wrong_user = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets/ds/offline-sync")
                .header("x-test-user-id", "someone_else")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wrong_user.status(), StatusCode::UNAUTHORIZED);

    let timestamp = labello_domain::now().to_rfc3339();
    let spoofed_record = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets/ds/offline-sync")
                .header("x-test-user-id", "admin")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "schemaVersion": 2,
                        "datasetId": "ds",
                        "userId": "admin",
                        "fragments": [{
                            "imageId": "img_1",
                            "baseSequence": 0,
                            "events": [{
                                "schemaVersion": 2,
                                "eventSequence": 1,
                                "eventId": "evt_1",
                                "imageId": "img_1",
                                "type": "review_recorded",
                                "actorUserId": "admin",
                                "actorRole": "reviewer",
                                "timestamp": timestamp.clone(),
                                "payload": {
                                    "kind": "review_recorded",
                                    "review": {
                                        "reviewId": "rev_1",
                                        "target": {
                                            "targetType": "task",
                                            "task_id": "task_1"
                                        },
                                        "reviewerUserId": "someone_else",
                                        "decision": "approved",
                                        "timestamp": timestamp,
                                        "comment": null
                                    }
                                }
                            }]
                        }]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(spoofed_record.status(), StatusCode::UNAUTHORIZED);

    let authenticated = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets/ds/offline-sync")
                .header("x-test-user-id", "admin")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(authenticated.status(), StatusCode::OK);
}

#[tokio::test]
async fn config_endpoints_do_not_parse_image_index() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    tokio::fs::write(
        temp.path().join("ds").join("images-index.json"),
        b"not json",
    )
    .await
    .unwrap();

    for uri in [
        "/datasets/ds",
        "/datasets/ds/admin",
        "/datasets/ds/keybindings",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header("x-test-user-id", "admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
    }
}

#[tokio::test]
async fn starts_and_polls_ingest_job() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;

    let started = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets/ds/ingest-jobs")
                .header("x-test-user-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(started.status(), StatusCode::OK);
    let body = to_bytes(started.into_body(), usize::MAX).await.unwrap();
    let job: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let job_id = job["jobId"].as_str().unwrap();
    assert_eq!(job["status"], "running");

    let mut status = serde_json::Value::Null;
    for _ in 0..16 {
        let polled = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/datasets/ds/ingest-jobs/{job_id}"))
                    .header("x-test-user-id", "admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(polled.status(), StatusCode::OK);
        let body = to_bytes(polled.into_body(), usize::MAX).await.unwrap();
        status = serde_json::from_slice(&body).unwrap();
        if status["status"] == "completed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(status["status"], "completed");
}

#[tokio::test]
async fn uploads_images_and_serves_record_and_preview() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    let png = png_bytes(2, 3);
    let image_id = ImageId::from_blake3_hex(blake3::hash(&png).to_hex().as_ref());

    let upload = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets/ds/uploads?root=uploads/test&ingest=true")
                .header("x-test-user-id", "admin")
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={}", TEST_BOUNDARY),
                )
                .body(Body::from(multipart_body("nested/pixel.png", &png)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(upload.status(), StatusCode::OK);
    let body = to_bytes(upload.into_body(), usize::MAX).await.unwrap();
    let report: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(report["discoveredFiles"], 1);
    assert_eq!(report["newImages"], 1);

    let admin = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/datasets/ds/admin")
                .header("x-test-user-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(admin.status(), StatusCode::OK);
    let body = to_bytes(admin.into_body(), usize::MAX).await.unwrap();
    let metadata: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(metadata["imageRoots"], json!(["images", "uploads/test"]));
    assert_eq!(metadata["images"].as_object().unwrap().len(), 0);

    let record = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/datasets/ds/images/{image_id}/record"))
                .header("x-test-user-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(record.status(), StatusCode::OK);
    let body = to_bytes(record.into_body(), usize::MAX).await.unwrap();
    let record: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(record["canonicalPath"], "uploads/test/nested/pixel.png");
    assert_eq!(record["width"], 2);
    assert_eq!(record["height"], 3);

    let preview = app
        .oneshot(
            Request::builder()
                .uri(format!("/datasets/ds/images/{image_id}/preview?max=256"))
                .header("x-test-user-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(preview.status(), StatusCode::OK);
    assert_eq!(preview.headers()["x-image-width"].to_str().unwrap(), "2");
    assert_eq!(preview.headers()["x-image-height"].to_str().unwrap(), "3");
    let body = to_bytes(preview.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.len(), 2 * 3 * 4);
}

#[tokio::test]
async fn validates_review_targets_and_counts_distinct_task_approvals() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    configure_pixel_task_review(&app, 2, "approval").await;
    let png = png_bytes(2, 2);
    let image_id = ImageId::from_blake3_hex(blake3::hash(&png).to_hex().as_ref());
    upload_test_image(&app, "review.png", &png).await;
    let timestamp = labello_domain::now().to_rfc3339();

    append_test_event(
        &app,
        &image_id,
        json!({
            "kind": "annotation_version_created",
            "annotation": {
                "annotationId": "ann_1",
                "version": 1,
                "taskId": "bounding_box:pixel",
                "classId": "pixel",
                "type": "bounding_box",
                "source": { "source": "human" },
                "geometry": {
                    "type": "bounding_box",
                    "geometry": { "x": 0.1, "y": 0.1, "width": 0.5, "height": 0.5 }
                },
                "authorUserId": "admin",
                "createdAt": timestamp,
                "updatedAt": timestamp,
                "deleted": false
            },
            "previous_version": null,
            "reason": null
        }),
    )
    .await;
    submit_test_task(&app, &image_id).await;

    let wrong_image = post_test_review(
        &app,
        &image_id,
        "admin",
        "review_wrong_image",
        json!({ "targetType": "image", "image_id": "another_image" }),
        "approved",
    )
    .await;
    assert_eq!(wrong_image.status(), StatusCode::BAD_REQUEST);
    let unknown_task = post_test_review(
        &app,
        &image_id,
        "admin",
        "review_unknown_task",
        json!({ "targetType": "task", "task_id": "unknown" }),
        "approved",
    )
    .await;
    assert_eq!(unknown_task.status(), StatusCode::BAD_REQUEST);
    let missing_version = post_test_review(
        &app,
        &image_id,
        "admin",
        "review_missing_version",
        json!({
            "targetType": "annotation_version",
            "annotation_id": "ann_1",
            "version": 2
        }),
        "approved",
    )
    .await;
    assert_eq!(missing_version.status(), StatusCode::BAD_REQUEST);

    let object_review = post_test_review(
        &app,
        &image_id,
        "admin",
        "review_object",
        json!({
            "targetType": "annotation_version",
            "annotation_id": "ann_1",
            "version": 1
        }),
        "approved",
    )
    .await;
    assert_eq!(object_review.status(), StatusCode::OK);
    assert_eq!(
        load_test_image_state(&app, &image_id).await["taskStates"]["bounding_box:pixel"]["status"],
        "submitted"
    );

    let approval = post_test_review(
        &app,
        &image_id,
        "admin",
        "review_first",
        json!({ "targetType": "task", "task_id": "bounding_box:pixel" }),
        "approved",
    )
    .await;
    assert_eq!(approval.status(), StatusCode::OK);
    assert_eq!(
        load_test_image_state(&app, &image_id).await["taskStates"]["bounding_box:pixel"]["status"],
        "submitted"
    );

    let second_approval = post_test_review(
        &app,
        &image_id,
        "reviewer_2",
        "review_second",
        json!({ "targetType": "task", "task_id": "bounding_box:pixel" }),
        "approved",
    )
    .await;
    assert_eq!(second_approval.status(), StatusCode::OK);
    assert_eq!(
        load_test_image_state(&app, &image_id).await["taskStates"]["bounding_box:pixel"]["status"],
        "completed"
    );

    configure_pixel_task_review(&app, 2, "none").await;
    let disabled = claim_assignment(&app, "admin", "review").await;
    assert!(disabled.is_null());
}

#[tokio::test]
async fn task_review_rejection_immediately_needs_correction() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    configure_pixel_task_review(&app, 3, "approval").await;
    let png = png_bytes(3, 2);
    let image_id = ImageId::from_blake3_hex(blake3::hash(&png).to_hex().as_ref());
    upload_test_image(&app, "rejection.png", &png).await;
    submit_test_task(&app, &image_id).await;

    let rejection = post_test_review(
        &app,
        &image_id,
        "admin",
        "review_rejected",
        json!({ "targetType": "task", "task_id": "bounding_box:pixel" }),
        "rejected",
    )
    .await;
    assert_eq!(rejection.status(), StatusCode::OK);
    assert_eq!(
        load_test_image_state(&app, &image_id).await["taskStates"]["bounding_box:pixel"]["status"],
        "needs_correction"
    );
}

#[tokio::test]
async fn annotation_completion_without_review_completes_task() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    configure_pixel_task_review(&app, 0, "none").await;
    let png = png_bytes(2, 4);
    let image_id = ImageId::from_blake3_hex(blake3::hash(&png).to_hex().as_ref());
    upload_test_image(&app, "no-review.png", &png).await;

    submit_test_task(&app, &image_id).await;

    assert_eq!(
        load_test_image_state(&app, &image_id).await["taskStates"]["bounding_box:pixel"]["status"],
        "completed"
    );
    assert!(claim_assignment(&app, "admin", "review").await.is_null());
}

#[tokio::test]
async fn correction_starts_a_new_review_round() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    configure_pixel_task_review(&app, 2, "approval").await;
    let png = png_bytes(4, 2);
    let image_id = ImageId::from_blake3_hex(blake3::hash(&png).to_hex().as_ref());
    upload_test_image(&app, "review-round.png", &png).await;
    submit_test_task(&app, &image_id).await;

    let first_approval = post_test_review(
        &app,
        &image_id,
        "admin",
        "round_1_approval",
        json!({ "targetType": "task", "task_id": "bounding_box:pixel" }),
        "approved",
    )
    .await;
    assert_eq!(first_approval.status(), StatusCode::OK);
    let rejection = post_test_review(
        &app,
        &image_id,
        "reviewer_2",
        "round_1_rejection",
        json!({ "targetType": "task", "task_id": "bounding_box:pixel" }),
        "rejected",
    )
    .await;
    assert_eq!(rejection.status(), StatusCode::OK);

    submit_test_task(&app, &image_id).await;
    let new_round_approval = post_test_review(
        &app,
        &image_id,
        "admin",
        "round_2_approval",
        json!({ "targetType": "task", "task_id": "bounding_box:pixel" }),
        "approved",
    )
    .await;
    assert_eq!(new_round_approval.status(), StatusCode::OK);
    assert_eq!(
        load_test_image_state(&app, &image_id).await["taskStates"]["bounding_box:pixel"]["status"],
        "submitted"
    );
    let final_approval = post_test_review(
        &app,
        &image_id,
        "reviewer_2",
        "round_2_approval_2",
        json!({ "targetType": "task", "task_id": "bounding_box:pixel" }),
        "approved",
    )
    .await;
    assert_eq!(final_approval.status(), StatusCode::OK);
    let body = to_bytes(final_approval.into_body(), usize::MAX)
        .await
        .unwrap();
    let review_state: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        review_state["taskStates"]["bounding_box:pixel"]["status"],
        "completed"
    );
    assert_eq!(
        load_test_image_state(&app, &image_id).await["taskStates"]["bounding_box:pixel"]["status"],
        "completed"
    );
}

#[tokio::test]
async fn reviewer_bbox_correction_is_terminal_rejected_idempotent_and_cancels_competitors() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    let (image_id, task_id) = prepare_correction_task(&app, false, true, "correct-box.png").await;
    let assignment = claim_assignment_for_task(&app, "admin", "review", &task_id).await;
    let competing = claim_assignment_for_task(&app, "reviewer_2", "review", &task_id).await;
    let request = json!({
        "correctionId": "cor_api_bbox",
        "annotationId": "ann_1",
        "expectedVersion": 1,
        "geometry": {
            "type": "bounding_box",
            "geometry": { "x": 0.2, "y": 0.2, "width": 0.3, "height": 0.4 }
        },
        "reason": "box was too loose"
    });

    let mut stale_request = request.clone();
    stale_request["expectedVersion"] = json!(0);
    let stale = post_test_correction(&app, &image_id, "admin", &assignment, stale_request).await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    let corrected =
        post_test_correction(&app, &image_id, "admin", &assignment, request.clone()).await;
    assert_eq!(corrected.status(), StatusCode::OK);
    let body = to_bytes(corrected.into_body(), usize::MAX).await.unwrap();
    let event: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(event["payload"]["kind"], "reviewer_correction_recorded");
    let retry = post_test_correction(&app, &image_id, "admin", &assignment, request).await;
    assert_eq!(retry.status(), StatusCode::OK);
    let body = to_bytes(retry.into_body(), usize::MAX).await.unwrap();
    let retry_event: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(retry_event["eventId"], event["eventId"]);

    let state = load_test_image_state(&app, &image_id).await;
    assert_eq!(state["annotations"]["ann_1"].as_array().unwrap().len(), 2);
    assert_eq!(state["annotations"]["ann_1"][1]["authorUserId"], "admin");
    assert_eq!(
        state["annotations"]["ann_1"][1]["source"]["source"],
        "reviewer_correction"
    );
    assert_eq!(state["reviews"][0]["decision"], "rejected");
    assert_eq!(state["taskStates"][&task_id]["status"], "completed");
    assert_eq!(
        state["taskStates"][&task_id]["outcome"],
        "reviewer_corrected"
    );
    assert_eq!(
        assignment_status_json(&state, assignment["assignmentId"].as_str().unwrap()),
        "completed"
    );
    assert_eq!(
        assignment_status_json(&state, competing["assignmentId"].as_str().unwrap()),
        "cancelled"
    );
    assert!(
        claim_assignment_for_task(&app, "other_annotator", "annotation", &task_id)
            .await
            .is_null()
    );

    let stats = get_test_stats(&app).await;
    assert_eq!(stats["reviewedTasks"], 0);
    assert_eq!(stats["approvedTasks"], 0);
    assert_eq!(stats["rejectedTasks"], 1);
    assert_eq!(stats["reviewerCorrectedTasks"], 1);
    assert_eq!(stats["finalizedTasks"], 1);
}

#[tokio::test]
async fn reviewer_keypoint_correction_uses_server_provenance_and_respects_config() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    let (image_id, task_id) = prepare_correction_task(&app, true, true, "correct-pose.png").await;
    let assignment = claim_assignment_for_task(&app, "admin", "review", &task_id).await;
    let response = post_test_correction(
        &app,
        &image_id,
        "admin",
        &assignment,
        json!({
            "correctionId": "cor_api_pose",
            "annotationId": "ann_1",
            "expectedVersion": 1,
            "geometry": {
                "type": "skeleton",
                "geometry": {
                    "keypoints": [{
                        "name": "nose",
                        "state": "visible",
                        "point": { "x": 0.7, "y": 0.4 }
                    }]
                }
            },
            "reason": null
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let state = load_test_image_state(&app, &image_id).await;
    assert_eq!(
        state["annotations"]["ann_1"][1]["geometry"]["geometry"]["keypoints"][0]["point"]["x"],
        0.7
    );
    assert_eq!(
        state["taskStates"][&task_id]["outcome"],
        "reviewer_corrected"
    );

    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    let (image_id, task_id) = prepare_correction_task(&app, false, false, "disabled.png").await;
    let assignment = claim_assignment_for_task(&app, "admin", "review", &task_id).await;
    let response = post_test_correction(
        &app,
        &image_id,
        "admin",
        &assignment,
        json!({
            "correctionId": "cor_api_disabled",
            "annotationId": "ann_1",
            "expectedVersion": 1,
            "geometry": {
                "type": "bounding_box",
                "geometry": { "x": 0.2, "y": 0.2, "width": 0.3, "height": 0.3 }
            },
            "reason": null
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn config_rejects_enabling_independent_agreement() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    configure_pixel_task(&app).await;
    let admin = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/datasets/ds/admin")
                .header("x-test-user-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(admin.into_body(), usize::MAX).await.unwrap();
    let mut metadata: serde_json::Value = serde_json::from_slice(&body).unwrap();
    metadata["tasks"][0]["review"]["workflow"] = json!("independent_agreement");

    let update = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/datasets/ds/admin")
                .header("x-test-user-id", "admin")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "name": metadata["name"],
                        "imageRoots": metadata["imageRoots"],
                        "labelClasses": metadata["labelClasses"],
                        "tasks": metadata["tasks"],
                        "roleAssignments": metadata["roleAssignments"],
                        "imbalance": metadata["imbalance"],
                        "prelabelConfigs": metadata["prelabelConfigs"]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(update.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(update.into_body(), usize::MAX).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("not implemented"));
}

#[tokio::test]
async fn assign_next_uses_camel_case_json_body() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    configure_pixel_task(&app).await;
    let png = png_bytes(2, 2);

    let upload = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets/ds/uploads?root=uploads/test&ingest=true")
                .header("x-test-user-id", "admin")
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={}", TEST_BOUNDARY),
                )
                .body(Body::from(multipart_body("pixel.png", &png)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(upload.status(), StatusCode::OK);

    let assignment = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets/ds/images/next")
                .header("x-test-user-id", "admin")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "taskId": "bounding_box:pixel",
                        "kind": "annotation",
                        "excludedImageIds": []
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(assignment.status(), StatusCode::OK);
    let body = to_bytes(assignment.into_body(), usize::MAX).await.unwrap();
    let assignment: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(assignment["taskId"], "bounding_box:pixel");
    assert!(assignment["expiresAt"].is_string());

    let stale_query = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets/ds/images/next?taskId=bounding_box%3Apixel&kind=annotation")
                .header("x-test-user-id", "admin")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stale_query.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn assign_next_honors_exact_reclaim_then_exclusions() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    configure_pixel_task(&app).await;
    upload_test_image(&app, "first.png", &png_bytes(2, 2)).await;
    upload_test_image(&app, "second.png", &png_bytes(3, 2)).await;

    let first = claim_assignment(&app, "admin", "annotation").await;
    let exact = claim_assignment_with_body(
        &app,
        "admin",
        json!({
            "taskId": "bounding_box:pixel",
            "kind": "annotation",
            "assignmentId": first["assignmentId"],
            "excludedImageIds": [first["imageId"]]
        }),
    )
    .await;
    assert_eq!(exact.status(), StatusCode::OK);
    let exact: serde_json::Value =
        serde_json::from_slice(&to_bytes(exact.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(exact["assignmentId"], first["assignmentId"]);

    assert_eq!(
        post_assignment_action(&app, "admin", "release", &first)
            .await
            .status(),
        StatusCode::OK
    );
    let different = claim_assignment_with_body(
        &app,
        "admin",
        json!({
            "taskId": "bounding_box:pixel",
            "kind": "annotation",
            "excludedImageIds": [first["imageId"]]
        }),
    )
    .await;
    assert_eq!(different.status(), StatusCode::OK);
    let different: serde_json::Value =
        serde_json::from_slice(&to_bytes(different.into_body(), usize::MAX).await.unwrap())
            .unwrap();
    assert_ne!(different["imageId"], first["imageId"]);
}

#[tokio::test]
async fn assign_next_rejects_invalid_ids_and_too_many_exclusions() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    configure_pixel_task(&app).await;

    for request in [
        json!({
            "taskId": "bounding_box:pixel",
            "kind": "annotation",
            "excludedImageIds": ["img_1", "img_2", "img_3", "img_4"]
        }),
        json!({
            "taskId": "bounding_box:pixel",
            "kind": "annotation",
            "excludedImageIds": ["../image"]
        }),
        json!({
            "taskId": "../task",
            "kind": "annotation"
        }),
        json!({
            "taskId": "bounding_box:pixel",
            "kind": "annotation",
            "assignmentId": "../assignment"
        }),
    ] {
        assert_eq!(
            claim_assignment_with_body(&app, "admin", request)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
}

#[tokio::test]
async fn assignment_lifecycle_is_exact_owned_and_resumable() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    configure_pixel_task(&app).await;
    upload_test_image(&app, "first.png", &png_bytes(2, 2)).await;
    upload_test_image(&app, "second.png", &png_bytes(3, 2)).await;

    let first = claim_assignment(&app, "admin", "annotation").await;
    let retry = claim_assignment(&app, "admin", "annotation").await;
    assert_eq!(retry["assignmentId"], first["assignmentId"]);

    let timestamp = labello_domain::now().to_rfc3339();
    let query = format!(
        "assignmentId={}&imageId={}&taskId={}&kind=annotation",
        first["assignmentId"].as_str().unwrap(),
        first["imageId"].as_str().unwrap(),
        urlencoding::encode(first["taskId"].as_str().unwrap())
    );
    let wrong_user = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/datasets/ds/images/{}/events?{query}",
                    first["imageId"].as_str().unwrap()
                ))
                .header("x-test-user-id", "other_annotator")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "payload": {
                            "kind": "annotation_version_created",
                            "annotation": {
                                "annotationId": "ann_wrong_user",
                                "version": 1,
                                "taskId": "bounding_box:pixel",
                                "classId": "pixel",
                                "type": "bounding_box",
                                "source": { "source": "human" },
                                "geometry": {
                                    "type": "bounding_box",
                                    "geometry": { "x": 0.1, "y": 0.1, "width": 0.5, "height": 0.5 }
                                },
                                "authorUserId": "other_annotator",
                                "createdAt": timestamp,
                                "updatedAt": timestamp,
                                "deleted": false
                            },
                            "previous_version": null,
                            "reason": null
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wrong_user.status(), StatusCode::UNAUTHORIZED);

    let completed = post_assignment_action(&app, "admin", "complete", &first).await;
    assert_eq!(completed.status(), StatusCode::OK);
    let first_state =
        load_test_image_state(&app, &ImageId::from(first["imageId"].as_str().unwrap())).await;
    assert_eq!(
        first_state["taskStates"]["bounding_box:pixel"]["status"],
        "submitted"
    );

    let review = claim_assignment(&app, "admin", "review").await;
    let bypass_review = post_assignment_action(&app, "admin", "complete", &review).await;
    assert_eq!(bypass_review.status(), StatusCode::BAD_REQUEST);
    let released_review = post_assignment_action(&app, "admin", "release", &review).await;
    assert_eq!(released_review.status(), StatusCode::OK);

    let next = claim_assignment(&app, "admin", "annotation").await;
    assert_ne!(next["imageId"], first["imageId"]);
    assert_ne!(next["assignmentId"], first["assignmentId"]);
    let released = post_assignment_action(&app, "admin", "release", &next).await;
    assert_eq!(released.status(), StatusCode::OK);
    let reclaimed = claim_assignment(&app, "admin", "annotation").await;
    assert_eq!(reclaimed["imageId"], next["imageId"]);
    assert_ne!(reclaimed["assignmentId"], next["assignmentId"]);
}

#[tokio::test]
async fn annotation_batch_validates_atomically_and_returns_resulting_state() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    configure_pixel_task(&app).await;
    upload_test_image(&app, "batch.png", &png_bytes(2, 2)).await;
    let assignment = claim_assignment(&app, "admin", "annotation").await;
    let image_id = assignment["imageId"].as_str().unwrap();
    let query = format!(
        "assignmentId={}&imageId={}&taskId={}&kind=annotation",
        assignment["assignmentId"].as_str().unwrap(),
        image_id,
        urlencoding::encode(assignment["taskId"].as_str().unwrap())
    );
    let annotation = |id: &str| {
        json!({
            "kind": "annotation_version_created",
            "annotation": {
                "annotationId": id,
                "version": 1,
                "taskId": "bounding_box:pixel",
                "classId": "pixel",
                "type": "bounding_box",
                "source": { "source": "human" },
                "geometry": {
                    "type": "bounding_box",
                    "geometry": { "x": 0.1, "y": 0.1, "width": 0.5, "height": 0.5 }
                },
                "authorUserId": "admin",
                "createdAt": labello_domain::now().to_rfc3339(),
                "updatedAt": labello_domain::now().to_rfc3339(),
                "deleted": false
            },
            "previous_version": null,
            "reason": null
        })
    };
    let post = |body: serde_json::Value| {
        app.clone().oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/datasets/ds/images/{image_id}/annotation-batch?{query}"
                ))
                .header("x-test-user-id", "admin")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
    };

    let rejected = post(json!({
        "payloads": [
            annotation("ann_1"),
            {
                "kind": "annotation_deleted",
                "annotation_id": "missing",
                "version": 1,
                "reason": null
            }
        ],
        "complete": false
    }))
    .await
    .unwrap();
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
    let state = load_test_image_state(&app, &ImageId::from(image_id)).await;
    assert!(state["annotations"].as_object().unwrap().is_empty());

    let request = json!({
        "payloads": [annotation("ann_1"), annotation("ann_2")],
        "complete": true
    });
    let saved = post(request.clone()).await.unwrap();
    assert_eq!(saved.status(), StatusCode::OK);
    let body = to_bytes(saved.into_body(), usize::MAX).await.unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(saved["annotations"].as_object().unwrap().len(), 2);
    let sequence = saved["currentSequence"].as_u64().unwrap();

    let retried = post(request).await.unwrap();
    assert_eq!(retried.status(), StatusCode::OK);
    let body = to_bytes(retried.into_body(), usize::MAX).await.unwrap();
    let retried: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(retried["currentSequence"], sequence);
}

#[tokio::test]
async fn concurrent_api_claims_do_not_share_annotation_work() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    configure_pixel_task(&app).await;
    upload_test_image(&app, "only.png", &png_bytes(2, 2)).await;

    let (first, second) = tokio::join!(
        claim_assignment(&app, "admin", "annotation"),
        claim_assignment(&app, "other_annotator", "annotation")
    );
    assert_eq!(
        usize::from(!first.is_null()) + usize::from(!second.is_null()),
        1
    );
}

#[tokio::test]
async fn upload_requires_admin_and_rejects_unsafe_paths() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    let png = png_bytes(1, 1);

    let non_admin = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets/ds/uploads?root=uploads/test&ingest=true")
                .header("x-test-user-id", "intruder")
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={}", TEST_BOUNDARY),
                )
                .body(Body::from(multipart_body("pixel.png", &png)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(non_admin.status(), StatusCode::UNAUTHORIZED);

    let unsafe_root = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets/ds/uploads?root=../escape&ingest=true")
                .header("x-test-user-id", "admin")
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={}", TEST_BOUNDARY),
                )
                .body(Body::from(multipart_body("pixel.png", &png)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unsafe_root.status(), StatusCode::BAD_REQUEST);

    let unsafe_file_name = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets/ds/uploads?root=uploads/test&ingest=true")
                .header("x-test-user-id", "admin")
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={}", TEST_BOUNDARY),
                )
                .body(Body::from(multipart_body("../escape.png", &png)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unsafe_file_name.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn session_survives_state_recreation_and_logout_invalidates_it() {
    let temp = tempfile::tempdir().unwrap();
    let state = ApiState::new(temp.path()).with_session_cookie_secure(false);
    let timestamp = labello_domain::now();
    state
        .server_store
        .upsert_user(labello_domain::UserAccount {
            user_id: labello_domain::UserId::from("session_user"),
            display_name: "Session User".to_string(),
            github_user_id: Some("42".to_string()),
            github_login: Some("session-user".to_string()),
            created_at: timestamp,
            updated_at: timestamp,
        })
        .unwrap();
    let token = state
        .create_session(labello_domain::UserId::from("session_user"))
        .unwrap();

    let app = production_router(ApiState::new(temp.path()).with_session_cookie_secure(false));
    let me = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/me")
                .header(header::COOKIE, format!("labello_session={token}"))
                .header(axum::http::HeaderName::from_static("x-user-id"), "spoofed")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(me.status(), StatusCode::OK);
    let body = to_bytes(me.into_body(), usize::MAX).await.unwrap();
    let session: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(session["account"]["userId"], "session_user");
    assert_eq!(session["canCreateDatasets"], false);

    let logout = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/logout")
                .header(header::COOKIE, format!("labello_session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);
    let cleared = logout.headers()[header::SET_COOKIE].to_str().unwrap();
    assert!(cleared.contains("HttpOnly"));
    assert!(cleared.contains("SameSite=Lax"));
    assert!(!cleared.contains("Secure"));
    assert!(cleared.contains("Max-Age=0"));

    let expired = app
        .oneshot(
            Request::builder()
                .uri("/me")
                .header(header::COOKIE, format!("labello_session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(expired.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn oauth_flow_binds_state_to_browser_and_redirects_once_to_valid_return_target() {
    let temp = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock_address = listener.local_addr().unwrap();
    let mock_github = axum::Router::new()
        .route(
            "/token",
            axum::routing::post(|| async { axum::Json(json!({ "access_token": "github-token" })) }),
        )
        .route(
            "/user",
            axum::routing::get(|| async {
                axum::Json(json!({ "id": 42, "login": "octocat", "name": "Octo Cat" }))
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock_github).await.unwrap() });

    let state = ApiState::new(temp.path())
        .with_browser_origins(vec!["https://app.example.com".to_string()])
        .unwrap()
        .with_session_cookie_secure(true)
        .with_github_oauth(crate::GithubOAuthConfig {
            client_id: "client".to_string(),
            client_secret: "secret".to_string(),
            redirect_uri: "https://api.example.com/auth/github/callback".to_string(),
        })
        .with_github_oauth_endpoints(crate::oauth::GithubOAuthEndpoints {
            token_url: format!("http://{mock_address}/token"),
            user_url: format!("http://{mock_address}/user"),
        });
    let app = router(state.clone());
    let return_to = "https://app.example.com/datasets/ds?tab=review";
    let login = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/auth/github/login?returnTo={}",
                    urlencoding::encode(return_to)
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::TEMPORARY_REDIRECT);
    let location = login.headers()[header::LOCATION].to_str().unwrap();
    let url = url::Url::parse(location).unwrap();
    let generated = url
        .query_pairs()
        .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
        .unwrap();
    let browser_a_cookie = login.headers()[header::SET_COOKIE].to_str().unwrap();
    assert!(browser_a_cookie.starts_with("labello_oauth_flow="));
    assert!(browser_a_cookie.contains("Path=/auth/github"));
    assert!(browser_a_cookie.contains("HttpOnly"));
    assert!(browser_a_cookie.contains("SameSite=None"));
    assert!(browser_a_cookie.contains("Secure"));
    let browser_a_cookie = browser_a_cookie.split(';').next().unwrap().to_string();

    let invalid_return = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/auth/github/login?returnTo=https%3A%2F%2Fevil.example%2Fsteal")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid_return.status(), StatusCode::BAD_REQUEST);

    let missing_cookie = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/auth/github/callback?code=unused&state={generated}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_cookie.status(), StatusCode::UNAUTHORIZED);

    let browser_b_login = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/auth/github/login?returnTo=https%3A%2F%2Fapp.example.com%2Fother")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let browser_b_cookie = browser_b_login.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    let wrong_browser = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/auth/github/callback?code=unused&state={generated}"
                ))
                .header(header::COOKIE, browser_b_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wrong_browser.status(), StatusCode::UNAUTHORIZED);

    let callback = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/auth/github/callback?code=valid-code&state={generated}"
                ))
                .header(header::COOKIE, &browser_a_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(callback.status(), StatusCode::SEE_OTHER);
    assert_eq!(callback.headers()[header::LOCATION], return_to);
    let set_cookies: Vec<_> = callback
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|value| value.to_str().unwrap())
        .collect();
    assert_eq!(set_cookies.len(), 2);
    assert!(set_cookies.iter().any(|cookie| {
        cookie.starts_with("labello_session=")
            && cookie.contains("SameSite=None")
            && cookie.contains("Secure")
    }));
    assert!(set_cookies.iter().any(|cookie| {
        cookie.starts_with("labello_oauth_flow=;") && cookie.contains("Max-Age=0")
    }));

    let replay = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/auth/github/callback?code=valid-code&state={generated}"
                ))
                .header(header::COOKIE, browser_a_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn data_admin_lists_discovered_users_and_assigns_roles() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    let discover = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/me")
                .header("x-test-user-id", "worker")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(discover.status(), StatusCode::OK);

    let assigned = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/datasets/ds/roles")
                .header("x-test-user-id", "admin")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "userId": "worker", "roles": ["annotator", "reviewer"] }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(assigned.status(), StatusCode::OK);

    let users = app
        .oneshot(
            Request::builder()
                .uri("/datasets/ds/users")
                .header("x-test-user-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(users.status(), StatusCode::OK);
    let body = to_bytes(users.into_body(), usize::MAX).await.unwrap();
    let users: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    let worker = users
        .iter()
        .find(|user| user["account"]["userId"] == "worker")
        .unwrap();
    assert_eq!(worker["roles"], json!(["annotator", "reviewer"]));
}

#[tokio::test]
async fn credentialed_cors_only_allows_configured_origins() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(
        ApiState::new(temp.path())
            .with_browser_origins(vec!["https://app.remote.example".to_string()])
            .unwrap(),
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/me")
                .header(header::ORIGIN, "https://app.remote.example")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "content-type")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        "https://app.remote.example"
    );
    assert_eq!(
        response.headers()[header::ACCESS_CONTROL_ALLOW_CREDENTIALS],
        "true"
    );
    let allowed_headers = response.headers()[header::ACCESS_CONTROL_ALLOW_HEADERS]
        .to_str()
        .unwrap();
    assert_eq!(allowed_headers, "content-type");

    let actual = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .header(header::ORIGIN, "https://app.remote.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        actual.headers()[header::ACCESS_CONTROL_EXPOSE_HEADERS],
        "x-image-width,x-image-height,x-request-id"
    );

    let denied = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .header(header::ORIGIN, "https://evil.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        denied
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none()
    );
}

#[test]
fn browser_origin_configuration_rejects_empty_and_non_origin_urls() {
    let temp = tempfile::tempdir().unwrap();
    assert!(
        ApiState::new(temp.path())
            .with_browser_origins(Vec::new())
            .is_err()
    );
    for invalid in [
        "https://app.example/path",
        "https://app.example?query=yes",
        "file:///tmp/app",
        "not a URL",
    ] {
        assert!(
            ApiState::new(temp.path())
                .with_browser_origins(vec![invalid.to_string()])
                .is_err(),
            "accepted {invalid}"
        );
    }
}

#[tokio::test]
async fn data_admin_explores_images_with_bounded_pagination_and_filters() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    configure_pixel_task(&app).await;
    upload_test_image(&app, "alpha.png", &png_bytes(2, 2)).await;
    upload_test_image(&app, "beta.png", &png_bytes(3, 2)).await;

    let unauthorized = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/datasets/ds/images")
                .header("x-test-user-id", "intruder")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let first_page = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/datasets/ds/images?page=1&pageSize=1&status=pending&taskId=bounding_box%3Apixel")
                .header("x-test-user-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first_page.status(), StatusCode::OK);
    let body = to_bytes(first_page.into_body(), usize::MAX).await.unwrap();
    let page: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert_eq!(page["totalItems"], 2);
    assert_eq!(page["totalPages"], 2);
    assert_eq!(
        page["items"][0]["taskStatuses"]["bounding_box:pixel"],
        "pending"
    );

    let searched = app
        .oneshot(
            Request::builder()
                .uri("/datasets/ds/images?pageSize=500&search=BETA")
                .header("x-test-user-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(searched.status(), StatusCode::OK);
    let body = to_bytes(searched.into_body(), usize::MAX).await.unwrap();
    let page: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(page["pageSize"], 100);
    assert_eq!(page["totalItems"], 1);
    assert_eq!(page["items"][0]["image"]["fileName"], "beta.png");
}

#[tokio::test]
async fn data_admin_creates_lists_and_downloads_native_snapshot_files() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    upload_test_image(&app, "snapshot.png", &png_bytes(2, 3)).await;

    let unauthorized = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets/ds/snapshots")
                .header("x-test-user-id", "intruder")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets/ds/snapshots")
                .header("x-test-user-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::OK);
    let body = to_bytes(created.into_body(), usize::MAX).await.unwrap();
    let snapshot: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(snapshot["includesImageBytes"], false);
    let snapshot_id = snapshot["snapshotId"].as_str().unwrap();
    let files = snapshot["files"].as_array().unwrap();
    assert!(
        files
            .iter()
            .any(|file| file["path"] == "labello.dataset.toml")
    );
    assert!(files.iter().any(|file| file["path"] == "images-index.json"));
    assert!(
        files
            .iter()
            .any(|file| file["path"].as_str().unwrap().ends_with("/events.jsonl"))
    );
    assert!(
        files
            .iter()
            .any(|file| file["path"].as_str().unwrap().ends_with("/state.json"))
    );

    let manifest = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/datasets/ds/snapshots/{snapshot_id}/files/manifest.json"
                ))
                .header("x-test-user-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(manifest.status(), StatusCode::OK);
    let manifest_body = to_bytes(manifest.into_body(), usize::MAX).await.unwrap();
    let downloaded_manifest: serde_json::Value = serde_json::from_slice(&manifest_body).unwrap();
    assert_eq!(downloaded_manifest["snapshotId"], snapshot_id);

    let index_entry = files
        .iter()
        .find(|file| file["path"] == "images-index.json")
        .unwrap();
    let index = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/datasets/ds/snapshots/{snapshot_id}/files/images-index.json"
                ))
                .header("x-test-user-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(index.status(), StatusCode::OK);
    let index_body = to_bytes(index.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        blake3::hash(&index_body).to_hex().as_str(),
        index_entry["blake3"].as_str().unwrap()
    );

    let listed = app
        .oneshot(
            Request::builder()
                .uri("/datasets/ds/snapshots")
                .header("x-test-user-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let body = to_bytes(listed.into_body(), usize::MAX).await.unwrap();
    let snapshots: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0]["snapshotId"], snapshot_id);
}

const TEST_BOUNDARY: &str = "LABELLOBOUNDARY";

async fn create_dataset(app: &axum::Router) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets")
                .header("x-test-user-id", "admin")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "datasetId": "ds",
                        "name": "Dataset",
                        "adminUserId": "admin"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

async fn configure_pixel_task(app: &axum::Router) {
    configure_pixel_task_review(app, 1, "approval").await;
}

async fn configure_pixel_task_review(app: &axum::Router, required_reviews: u32, workflow: &str) {
    let admin = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/datasets/ds/admin")
                .header("x-test-user-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(admin.status(), StatusCode::OK);
    let body = to_bytes(admin.into_body(), usize::MAX).await.unwrap();
    let mut metadata: serde_json::Value = serde_json::from_slice(&body).unwrap();
    metadata["labelClasses"] = json!([{
        "classId": "pixel",
        "name": "Pixel",
        "color": "#5eead4",
        "description": null
    }]);
    metadata["tasks"] = json!([{
        "taskId": "bounding_box:pixel",
        "name": "Pixel bounding boxes",
        "annotationType": "bounding_box",
        "classIds": ["pixel"],
        "instructions": {
            "title": "Label pixels",
            "exampleText": "Draw boxes around pixels.",
            "exampleImages": []
        },
        "skeleton": null,
        "review": {
            "requiredReviews": required_reviews,
            "workflow": workflow,
            "allowReviewerCorrections": false,
            "agreementThreshold": null
        },
        "prelabelConfigIds": [],
        "enabled": true
    }]);
    if !metadata["roleAssignments"]
        .as_array()
        .unwrap()
        .iter()
        .any(|assignment| assignment["userId"] == "reviewer_2")
    {
        metadata["roleAssignments"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "datasetId": "ds",
                "userId": "reviewer_2",
                "roles": ["reviewer"],
                "assignedAt": labello_domain::now().to_rfc3339(),
                "assignedBy": "admin"
            }));
    }
    if !metadata["roleAssignments"]
        .as_array()
        .unwrap()
        .iter()
        .any(|assignment| assignment["userId"] == "other_annotator")
    {
        metadata["roleAssignments"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "datasetId": "ds",
                "userId": "other_annotator",
                "roles": ["annotator"],
                "assignedAt": labello_domain::now().to_rfc3339(),
                "assignedBy": "admin"
            }));
    }

    let update = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/datasets/ds/admin")
                .header("x-test-user-id", "admin")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "name": metadata["name"],
                        "imageRoots": metadata["imageRoots"],
                        "labelClasses": metadata["labelClasses"],
                        "tasks": metadata["tasks"],
                        "roleAssignments": metadata["roleAssignments"],
                        "imbalance": metadata["imbalance"],
                        "prelabelConfigs": metadata["prelabelConfigs"]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(update.status(), StatusCode::OK);
}

async fn prepare_correction_task(
    app: &axum::Router,
    skeleton: bool,
    allow_reviewer_corrections: bool,
    file_name: &str,
) -> (ImageId, String) {
    configure_pixel_task_review(app, 1, "approval").await;
    let admin = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/datasets/ds/admin")
                .header("x-test-user-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(admin.into_body(), usize::MAX).await.unwrap();
    let mut metadata: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let task_id = if skeleton {
        metadata["tasks"][0]["taskId"] = json!("skeleton:pixel");
        metadata["tasks"][0]["annotationType"] = json!("skeleton");
        metadata["tasks"][0]["skeleton"] = json!({
            "keypoints": [{ "name": "nose", "required": true }],
            "edges": [],
            "allowHidden": true,
            "allowAbsent": false
        });
        "skeleton:pixel"
    } else {
        "bounding_box:pixel"
    };
    metadata["tasks"][0]["review"]["allowReviewerCorrections"] = json!(allow_reviewer_corrections);
    let update = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/datasets/ds/admin")
                .header("x-test-user-id", "admin")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "name": metadata["name"],
                        "imageRoots": metadata["imageRoots"],
                        "labelClasses": metadata["labelClasses"],
                        "tasks": metadata["tasks"],
                        "roleAssignments": metadata["roleAssignments"],
                        "imbalance": metadata["imbalance"],
                        "prelabelConfigs": metadata["prelabelConfigs"]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(update.status(), StatusCode::OK);

    let png = png_bytes(5, 5);
    let image_id = ImageId::from_blake3_hex(blake3::hash(&png).to_hex().as_ref());
    upload_test_image(app, file_name, &png).await;
    let timestamp = labello_domain::now().to_rfc3339();
    let (annotation_type, geometry) = if skeleton {
        (
            "skeleton",
            json!({
                "type": "skeleton",
                "geometry": {
                    "keypoints": [{
                        "name": "nose",
                        "state": "visible",
                        "point": { "x": 0.5, "y": 0.5 }
                    }]
                }
            }),
        )
    } else {
        (
            "bounding_box",
            json!({
                "type": "bounding_box",
                "geometry": { "x": 0.1, "y": 0.1, "width": 0.3, "height": 0.3 }
            }),
        )
    };
    append_test_event(
        app,
        &image_id,
        json!({
            "kind": "annotation_version_created",
            "annotation": {
                "annotationId": "ann_1",
                "version": 1,
                "taskId": task_id,
                "classId": "pixel",
                "type": annotation_type,
                "source": { "source": "human" },
                "geometry": geometry,
                "authorUserId": "admin",
                "createdAt": timestamp,
                "updatedAt": timestamp,
                "deleted": false
            },
            "previous_version": null,
            "reason": null
        }),
    )
    .await;
    submit_test_task_for_task(app, &image_id, task_id).await;
    (image_id, task_id.to_string())
}

async fn upload_test_image(app: &axum::Router, file_name: &str, bytes: &[u8]) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets/ds/uploads?root=uploads/test&ingest=true")
                .header("x-test-user-id", "admin")
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={TEST_BOUNDARY}"),
                )
                .body(Body::from(multipart_body(file_name, bytes)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

async fn append_test_event(app: &axum::Router, image_id: &ImageId, payload: serde_json::Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/datasets/ds/images/{image_id}/admin/events"))
                .header("x-test-user-id", "admin")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "payload": payload }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
}

async fn submit_test_task(app: &axum::Router, image_id: &ImageId) {
    submit_test_task_for_task(app, image_id, "bounding_box:pixel").await;
}

async fn submit_test_task_for_task(app: &axum::Router, image_id: &ImageId, task_id: &str) {
    let assignment = claim_assignment_for_task(app, "admin", "annotation", task_id).await;
    assert_eq!(assignment["imageId"], image_id.as_str());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets/ds/assignments/complete")
                .header("x-test-user-id", "admin")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "assignmentId": assignment["assignmentId"],
                        "imageId": assignment["imageId"],
                        "taskId": assignment["taskId"],
                        "kind": "annotation"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

async fn post_test_review(
    app: &axum::Router,
    image_id: &ImageId,
    reviewer: &str,
    review_id: &str,
    target: serde_json::Value,
    decision: &str,
) -> axum::response::Response {
    let assignment = claim_assignment(app, reviewer, "review").await;
    assert!(!assignment.is_null(), "expected a review assignment");
    let query = format!(
        "assignmentId={}&imageId={}&taskId={}&kind=review",
        assignment["assignmentId"].as_str().unwrap(),
        assignment["imageId"].as_str().unwrap(),
        urlencoding::encode(assignment["taskId"].as_str().unwrap())
    );
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/datasets/ds/images/{image_id}/reviews?{query}"))
                .header("x-test-user-id", reviewer)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "reviewId": review_id,
                        "target": target,
                        "reviewerUserId": reviewer,
                        "decision": decision,
                        "timestamp": labello_domain::now().to_rfc3339(),
                        "comment": null
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn claim_assignment(app: &axum::Router, user_id: &str, kind: &str) -> serde_json::Value {
    claim_assignment_for_task(app, user_id, kind, "bounding_box:pixel").await
}

async fn claim_assignment_for_task(
    app: &axum::Router,
    user_id: &str,
    kind: &str,
    task_id: &str,
) -> serde_json::Value {
    let response = claim_assignment_with_body(
        app,
        user_id,
        json!({
            "taskId": task_id,
            "kind": kind,
            "excludedImageIds": []
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

async fn claim_assignment_with_body(
    app: &axum::Router,
    user_id: &str,
    request: serde_json::Value,
) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets/ds/images/next")
                .header("x-test-user-id", user_id)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(request.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn post_test_correction(
    app: &axum::Router,
    image_id: &ImageId,
    reviewer: &str,
    assignment: &serde_json::Value,
    request: serde_json::Value,
) -> axum::response::Response {
    let query = format!(
        "assignmentId={}&imageId={}&taskId={}&kind=review",
        assignment["assignmentId"].as_str().unwrap(),
        assignment["imageId"].as_str().unwrap(),
        urlencoding::encode(assignment["taskId"].as_str().unwrap())
    );
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/datasets/ds/images/{image_id}/corrections?{query}"
                ))
                .header("x-test-user-id", reviewer)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(request.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

fn assignment_status_json<'a>(state: &'a serde_json::Value, assignment_id: &str) -> &'a str {
    state["assignments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|assignment| assignment["assignmentId"] == assignment_id)
        .unwrap()["status"]
        .as_str()
        .unwrap()
}

async fn get_test_stats(app: &axum::Router) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/datasets/ds/stats")
                .header("x-test-user-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

async fn post_assignment_action(
    app: &axum::Router,
    user_id: &str,
    action: &str,
    assignment: &serde_json::Value,
) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/datasets/ds/assignments/{action}"))
                .header("x-test-user-id", user_id)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "assignmentId": assignment["assignmentId"],
                        "imageId": assignment["imageId"],
                        "taskId": assignment["taskId"],
                        "kind": assignment["kind"]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn load_test_image_state(app: &axum::Router, image_id: &ImageId) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/datasets/ds/images/{image_id}"))
                .header("x-test-user-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn png_bytes(width: u32, height: u32) -> Vec<u8> {
    let image = ImageBuffer::from_pixel(width, height, Rgba([12_u8, 34, 56, 255]));
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

fn multipart_body(file_name: &str, bytes: &[u8]) -> Vec<u8> {
    let mut body = format!(
        "--{TEST_BOUNDARY}\r\nContent-Disposition: form-data; name=\"files\"; filename=\"{file_name}\"\r\nContent-Type: image/png\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{TEST_BOUNDARY}--\r\n").as_bytes());
    body
}
