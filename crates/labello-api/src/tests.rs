use axum::{
    body::{Body, to_bytes},
    http::{HeaderValue, Request, StatusCode, header},
    middleware::Next,
};
use image::{ImageBuffer, Rgba};
use labello_domain::{
    Actor, AnnotationGeometry, AnnotationId, AnnotationOrigin, AnnotationType, AnnotationVersion,
    Assignment, BoundingBox, ClassId, DatasetId, DatasetMetadata, DatasetRole,
    DatasetRoleAssignment, EventPayload, HumanRevisionKind, ImageId, ImageRecord, ImageState,
    ImagesIndex, ImportCoverage, ImportGeometryProvenance, ImportId, ImportTaskInitialization,
    ImportedOrigin, KeypointAnnotation, KeypointSpec, KeypointState, LabelClass,
    ManualBoxGuideMigration, MigrationCardinality, MigrationHashContext, MigrationSequence,
    MigrationTarget, MigrationTargetSetInitialization, NormalizedPoint, ObjectGroupId,
    ReviewConfig, ReviewDecision, ReviewWorkflow, RevisionSource, SCHEMA_VERSION, SkeletonGeometry,
    SkeletonSpec, SourceProfile, TaskDefinition, TaskId, TaskOutcome, TaskState, TaskStatus,
    TutorialContent, UserAccount, UserId, migration_confirmation_hash, migration_target_set_hash,
    now,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Cursor,
};
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
                    let session = state.create_session(user_id).unwrap();
                    request.headers_mut().insert(
                        header::COOKIE,
                        HeaderValue::from_str(&format!("labello_session={}", session.cookie))
                            .unwrap(),
                    );
                    if request.headers().get(crate::csrf::HEADER).is_none() {
                        request.headers_mut().insert(
                            axum::http::HeaderName::from_static(crate::csrf::HEADER),
                            HeaderValue::from_str(&session.csrf).unwrap(),
                        );
                    }
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
    let csrf_token = session["csrfToken"].as_str().unwrap().to_string();
    assert_eq!(csrf_token.len(), 64);

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
    assert_eq!(me.headers()[header::CACHE_CONTROL], "no-store");
    let body = to_bytes(me.into_body(), usize::MAX).await.unwrap();
    let session: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(session["account"]["userId"], "bootstrap_admin");
    assert_eq!(session["canCreateDatasets"], true);
    assert_eq!(session["csrfToken"], csrf_token);

    let rotated = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/local-admin")
                .header(header::ORIGIN, "https://app.example.com")
                .header(header::COOKIE, cookie.split(';').next().unwrap())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rotated.status(), StatusCode::OK);
    let rotated_cookie = rotated.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .to_string();
    assert_ne!(
        rotated_cookie.split(';').next().unwrap(),
        cookie.split(';').next().unwrap()
    );
    let body = to_bytes(rotated.into_body(), usize::MAX).await.unwrap();
    let rotated_session: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_ne!(rotated_session["csrfToken"], csrf_token);

    let old_session = app
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
    assert_eq!(old_session.status(), StatusCode::UNAUTHORIZED);

    let missing_origin = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/local-admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_origin.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn unsafe_session_requests_require_csrf_and_validate_optional_origin() {
    let temp = tempfile::tempdir().unwrap();
    let state = ApiState::new(temp.path())
        .with_browser_origins(vec!["https://app.example.com".to_string()])
        .unwrap();
    let timestamp = now();
    state
        .server_store
        .upsert_user(UserAccount {
            user_id: UserId::from("admin"),
            display_name: "Admin".to_string(),
            github_user_id: None,
            github_login: None,
            created_at: timestamp,
            updated_at: timestamp,
        })
        .unwrap();
    let session = state.create_session(UserId::from("admin")).unwrap();
    let app = production_router(state);
    let request = |dataset_id: &str, csrf: Option<&str>, origin: Option<&str>| {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/datasets")
            .header(
                header::COOKIE,
                format!("labello_session={}", session.cookie),
            )
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(csrf) = csrf {
            builder = builder.header(crate::csrf::HEADER, csrf);
        }
        if let Some(origin) = origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        builder
            .body(Body::from(
                json!({
                    "datasetId": dataset_id,
                    "name": "CSRF dataset",
                    "adminUserId": "admin"
                })
                .to_string(),
            ))
            .unwrap()
    };

    let missing = app
        .clone()
        .oneshot(request("missing", None, None))
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

    let wrong = app
        .clone()
        .oneshot(request("wrong", Some("wrong-token"), None))
        .await
        .unwrap();
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

    let wrong_origin = app
        .clone()
        .oneshot(request(
            "wrong-origin",
            Some(&session.csrf),
            Some("https://other.example.com"),
        ))
        .await
        .unwrap();
    assert_eq!(wrong_origin.status(), StatusCode::UNAUTHORIZED);

    let browser = app
        .clone()
        .oneshot(request(
            "browser-origin",
            Some(&session.csrf),
            Some("https://app.example.com"),
        ))
        .await
        .unwrap();
    assert_eq!(browser.status(), StatusCode::OK);

    let native = app
        .oneshot(request("native-no-origin", Some(&session.csrf), None))
        .await
        .unwrap();
    assert_eq!(native.status(), StatusCode::OK);
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
    assert_eq!(spoofed_record.status(), StatusCode::BAD_REQUEST);

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
async fn offline_sync_rejects_authoritative_fields_and_server_owned_mutations() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    let mutation = json!({
        "kind": "annotation_upsert",
        "annotationId": "ann_1",
        "expectedVersion": null,
        "taskId": "task_1",
        "classId": "person",
        "annotationType": "bounding_box",
        "source": { "source": "human" },
        "geometry": {
            "type": "bounding_box",
            "geometry": { "x": 0.1, "y": 0.1, "width": 0.2, "height": 0.2 }
        },
        "reason": null
    });

    let forged = [
        ("actorUserId", json!("someone_else")),
        ("timestamp", json!("2026-01-02T03:04:05Z")),
        ("version", json!(99)),
        ("origin", json!({ "origin": "native", "legacyV2": false })),
        ("objectGroupId", json!("forged_group")),
    ];
    for (field, value) in forged {
        let mut mutation = mutation.clone();
        mutation[field] = value;
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/datasets/ds/offline-sync")
                    .header("x-test-user-id", "admin")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "schemaVersion": 3,
                            "datasetId": "ds",
                            "userId": "admin",
                            "fragments": [{
                                "imageId": "img_1",
                                "baseSequence": 0,
                                "mutations": [mutation]
                            }]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "accepted {field}"
        );
    }

    for kind in ["import_initialized", "migration_disposition_changed"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/datasets/ds/offline-sync")
                    .header("x-test-user-id", "admin")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "schemaVersion": 3,
                            "datasetId": "ds",
                            "userId": "admin",
                            "fragments": [{
                                "imageId": "img_1",
                                "baseSequence": 0,
                                "mutations": [{ "kind": kind }]
                            }]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "accepted {kind}"
        );
    }
}

#[tokio::test]
async fn ordinary_event_ingresses_reject_server_owned_payloads() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    configure_pixel_task(&app).await;
    let png = png_bytes(2, 2);
    let image_id = ImageId::from_blake3_hex(blake3::hash(&png).to_hex().as_ref());
    upload_test_image(&app, "server-owned.png", &png).await;
    let assignment = claim_assignment(&app, "admin", "annotation").await;
    let timestamp = labello_domain::now().to_rfc3339();
    let query = format!(
        "assignmentId={}&imageId={}&taskId={}&kind=annotation",
        assignment["assignmentId"].as_str().unwrap(),
        image_id,
        urlencoding::encode(assignment["taskId"].as_str().unwrap())
    );
    let annotation = json!({
        "annotationId": "ann_server_owned",
        "version": 2,
        "taskId": "bounding_box:pixel",
        "classId": "pixel",
        "type": "bounding_box",
        "source": {
            "source": "reviewer_correction",
            "correction_id": "cor_server_owned"
        },
        "geometry": {
            "type": "bounding_box",
            "geometry": { "x": 0.1, "y": 0.1, "width": 0.5, "height": 0.5 }
        },
        "authorUserId": "admin",
        "createdAt": timestamp,
        "updatedAt": timestamp,
        "deleted": false
    });
    let task_state = json!({
        "taskId": "bounding_box:pixel",
        "status": "pending",
        "outcome": null,
        "assignedTo": null,
        "completedBy": null,
        "completedAt": null,
        "updatedAt": timestamp
    });
    let hash = "0000000000000000000000000000000000000000000000000000000000000000";
    let payloads = vec![
        (
            "assignment_updated",
            json!({
                "kind": "assignment_updated",
                "assignment": assignment
            }),
        ),
        (
            "reviewer_correction_recorded",
            json!({
                "kind": "reviewer_correction_recorded",
                "correction": {
                    "correctionId": "cor_server_owned",
                    "assignmentId": "asg_server_owned",
                    "annotationId": "ann_server_owned",
                    "previousVersion": 1,
                    "correctedVersion": 2,
                    "taskId": "bounding_box:pixel",
                    "reviewerUserId": "admin",
                    "timestamp": timestamp,
                    "reason": null
                },
                "annotation": annotation,
                "review": {
                    "reviewId": "rev_server_owned",
                    "target": {
                        "targetType": "annotation_version",
                        "annotation_id": "ann_server_owned",
                        "version": 2
                    },
                    "reviewerUserId": "admin",
                    "decision": "rejected",
                    "timestamp": timestamp,
                    "comment": null
                },
                "task_state": {
                    "taskId": "bounding_box:pixel",
                    "status": "completed",
                    "outcome": "reviewer_corrected",
                    "assignedTo": null,
                    "completedBy": "admin",
                    "completedAt": timestamp,
                    "updatedAt": timestamp
                },
                "assignments": []
            }),
        ),
        (
            "import_initialized",
            json!({
                "kind": "import_initialized",
                "import_id": "imp_server_owned",
                "annotations": [],
                "task_initializations": [],
                "migration_target_sets": []
            }),
        ),
        (
            "imported_task_reopened",
            json!({
                "kind": "imported_task_reopened",
                "task_state": task_state,
                "reason": "server owned"
            }),
        ),
        (
            "import_coverage_included",
            json!({
                "kind": "import_coverage_included",
                "task_state": task_state,
                "reason": "server owned"
            }),
        ),
        (
            "migration_disposition_changed",
            json!({
                "kind": "migration_disposition_changed",
                "task_id": "bounding_box:pixel",
                "object_group_id": "group_1",
                "disposition": {
                    "dispositionVersion": 1,
                    "status": { "status": "pending" }
                }
            }),
        ),
        (
            "migration_disposition_reopened",
            json!({
                "kind": "migration_disposition_reopened",
                "task_id": "bounding_box:pixel",
                "object_group_id": "group_1",
                "disposition": {
                    "dispositionVersion": 2,
                    "status": { "status": "pending" }
                }
            }),
        ),
        (
            "migration_dependency_marked",
            json!({
                "kind": "migration_dependency_marked",
                "task_id": "bounding_box:pixel",
                "object_group_id": "group_1",
                "marker": {
                    "markerVersion": 1,
                    "kind": "guide_unavailable",
                    "requiredDispositionVersion": 1,
                    "eventId": "evt_dependency",
                    "timestamp": timestamp
                }
            }),
        ),
        (
            "migration_dependency_cleared",
            json!({
                "kind": "migration_dependency_cleared",
                "task_id": "bounding_box:pixel",
                "object_group_id": "group_1",
                "marker_version": 1
            }),
        ),
        (
            "migration_pass_started",
            json!({
                "kind": "migration_pass_started",
                "pass": {
                    "passId": "pass_1",
                    "assignmentId": "asg_server_owned",
                    "taskId": "bounding_box:pixel",
                    "expectedTargetSetHash": hash,
                    "startingStateHash": hash,
                    "actorUserId": "admin",
                    "startedAt": timestamp,
                    "items": []
                }
            }),
        ),
        (
            "migration_pass_item_recorded",
            json!({
                "kind": "migration_pass_item_recorded",
                "pass_id": "pass_1",
                "item": {
                    "objectGroupId": "group_1",
                    "guideAnnotationVersion": 1,
                    "guideDeleted": false,
                    "dispositionVersion": 1,
                    "action": { "action": "kept" },
                    "eventId": "evt_pass_item"
                }
            }),
        ),
        (
            "migration_full_image_confirmed",
            json!({
                "kind": "migration_full_image_confirmed",
                "confirmation": {
                    "taskId": "bounding_box:pixel",
                    "targetSetHash": hash,
                    "stateHash": hash,
                    "confirmationHash": hash,
                    "actorUserId": "admin",
                    "timestamp": timestamp
                }
            }),
        ),
    ];

    for (event_type, payload) in payloads {
        let ingresses = [
            (
                "direct append",
                format!("/datasets/ds/images/{image_id}/events?{query}"),
                json!({ "payload": payload }),
            ),
            (
                "annotation batch",
                format!("/datasets/ds/images/{image_id}/annotation-batch?{query}"),
                json!({ "payloads": [payload], "complete": false }),
            ),
            (
                "admin repair",
                format!("/datasets/ds/images/{image_id}/admin/events"),
                json!({ "payload": payload }),
            ),
            (
                "offline sync",
                "/datasets/ds/offline-sync".to_string(),
                json!({
                    "schemaVersion": 2,
                    "datasetId": "ds",
                    "userId": "admin",
                    "fragments": [{
                        "imageId": image_id,
                        "baseSequence": 0,
                        "events": [{
                            "schemaVersion": 2,
                            "eventSequence": 1,
                            "eventId": "evt_server_owned",
                            "imageId": image_id,
                            "type": event_type,
                            "actorUserId": "admin",
                            "actorRole": "data_admin",
                            "timestamp": timestamp,
                            "payload": payload
                        }]
                    }]
                }),
            ),
        ];

        for (ingress, uri, body) in ingresses {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(uri)
                        .header("x-test-user-id", "admin")
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::BAD_REQUEST,
                "{ingress} accepted {event_type}"
            );
        }
    }
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
    let stale_disposition = post_test_review(
        &app,
        &image_id,
        "admin",
        "review_stale_disposition",
        json!({
            "targetType": "migration_disposition",
            "task_id": "bounding_box:pixel",
            "object_group_id": "group_1",
            "disposition_version": 1
        }),
        "approved",
    )
    .await;
    assert_eq!(stale_disposition.status(), StatusCode::BAD_REQUEST);
    let stale_confirmation = post_test_review(
        &app,
        &image_id,
        "admin",
        "review_stale_confirmation",
        json!({
            "targetType": "migration_confirmation",
            "task_id": "bounding_box:pixel",
            "confirmation_hash": "0000000000000000000000000000000000000000000000000000000000000000"
        }),
        "approved",
    )
    .await;
    assert_eq!(stale_confirmation.status(), StatusCode::BAD_REQUEST);
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
        state["annotations"]["ann_1"][1]["revisionSource"]["source"],
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
async fn assignment_availability_is_batched_authenticated_and_advisory() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    configure_pixel_task(&app).await;
    upload_test_image(&app, "only.png", &png_bytes(2, 2)).await;

    let unauthorized = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/datasets/ds/assignments/availability?kind=annotation")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let stale_available = get_assignment_availability(&app, "admin", "annotation").await;
    assert_eq!(stale_available["kind"], "annotation");
    assert_eq!(
        stale_available["tasks"]["bounding_box:pixel"],
        serde_json::Value::Bool(true)
    );

    let competing = claim_assignment(&app, "other_annotator", "annotation").await;
    assert!(!competing.is_null());
    assert!(
        claim_assignment(&app, "admin", "annotation")
            .await
            .is_null(),
        "the claim response remains authoritative when an earlier availability result is stale"
    );
    let reserved = get_assignment_availability(&app, "admin", "annotation").await;
    assert_eq!(
        reserved["tasks"]["bounding_box:pixel"],
        serde_json::Value::Bool(false)
    );

    assert_eq!(
        post_assignment_action(&app, "other_annotator", "release", &competing)
            .await
            .status(),
        StatusCode::OK
    );
    let released = get_assignment_availability(&app, "admin", "annotation").await;
    assert_eq!(
        released["tasks"]["bounding_box:pixel"],
        serde_json::Value::Bool(true)
    );
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
async fn cancelled_and_submitted_annotation_assignments_can_be_reopened_exactly() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    create_dataset(&app).await;
    configure_pixel_task(&app).await;
    upload_test_image(&app, "first.png", &png_bytes(2, 2)).await;

    let original = claim_assignment(&app, "admin", "annotation").await;
    assert_eq!(
        post_assignment_action(&app, "admin", "release", &original)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        post_assignment_action(&app, "other_annotator", "reopen", &original)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );

    let reopened = post_assignment_action(&app, "admin", "reopen", &original).await;
    assert_eq!(reopened.status(), StatusCode::OK);
    let reopened: serde_json::Value =
        serde_json::from_slice(&to_bytes(reopened.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(reopened["imageId"], original["imageId"]);
    assert_eq!(reopened["status"], "active");
    assert_ne!(reopened["assignmentId"], original["assignmentId"]);

    let retry = post_assignment_action(&app, "admin", "reopen", &original).await;
    assert_eq!(retry.status(), StatusCode::OK);
    let retry: serde_json::Value =
        serde_json::from_slice(&to_bytes(retry.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(retry["assignmentId"], reopened["assignmentId"]);

    assert_eq!(
        post_assignment_action(&app, "admin", "complete", &reopened)
            .await
            .status(),
        StatusCode::OK
    );
    let resubmitted = post_assignment_action(&app, "admin", "reopen", &reopened).await;
    assert_eq!(resubmitted.status(), StatusCode::OK);
    let resubmitted: serde_json::Value =
        serde_json::from_slice(&to_bytes(resubmitted.into_body(), usize::MAX).await.unwrap())
            .unwrap();
    assert_eq!(resubmitted["imageId"], original["imageId"]);
    assert_ne!(resubmitted["assignmentId"], reopened["assignmentId"]);

    let invalid_kind = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets/ds/assignments/reopen")
                .header("x-test-user-id", "admin")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "assignmentId": original["assignmentId"],
                        "imageId": original["imageId"],
                        "taskId": original["taskId"],
                        "kind": "review"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid_kind.status(), StatusCode::BAD_REQUEST);
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
    assert_eq!(saved["annotations"]["ann_1"][0]["version"], 1);
    assert_eq!(
        saved["annotations"]["ann_1"][0]["origin"],
        json!({ "origin": "native", "legacyV2": false })
    );
    assert_eq!(
        saved["annotations"]["ann_1"][0]["revisionSource"],
        json!({ "source": "human", "action": "authored" })
    );
    assert_eq!(
        saved["annotations"]["ann_1"][0]["objectGroupId"],
        serde_json::Value::Null
    );
    let sequence = saved["currentSequence"].as_u64().unwrap();

    let retried = post(request).await.unwrap();
    let retried_status = retried.status();
    let body = to_bytes(retried.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        retried_status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&body)
    );
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
    let session_tokens = state
        .create_session(labello_domain::UserId::from("session_user"))
        .unwrap();
    let token = session_tokens.cookie;
    let csrf_token = session_tokens.csrf;
    let auth_store =
        std::fs::read_to_string(temp.path().join(".labello-server/auth.json")).unwrap();
    assert!(!auth_store.contains(&token));
    assert!(auth_store.contains(&csrf_token));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(temp.path().join(".labello-server"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(temp.path().join(".labello-server/auth.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

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
    assert_eq!(me.headers()[header::CACHE_CONTROL], "no-store");
    let body = to_bytes(me.into_body(), usize::MAX).await.unwrap();
    let session: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(session["account"]["userId"], "session_user");
    assert_eq!(session["canCreateDatasets"], false);
    assert_eq!(session["csrfToken"], csrf_token);

    let missing_csrf = app
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
    assert_eq!(missing_csrf.status(), StatusCode::UNAUTHORIZED);

    let wrong_csrf = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/logout")
                .header(header::COOKIE, format!("labello_session={token}"))
                .header(crate::csrf::HEADER, "wrong-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wrong_csrf.status(), StatusCode::UNAUTHORIZED);

    let still_active = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/me")
                .header(header::COOKIE, format!("labello_session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(still_active.status(), StatusCode::OK);

    let logout = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/logout")
                .header(header::COOKIE, format!("labello_session={token}"))
                .header(crate::csrf::HEADER, &csrf_token)
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
async fn legacy_sessions_receive_a_persisted_csrf_token_on_load() {
    let temp = tempfile::tempdir().unwrap();
    let state = ApiState::new(temp.path());
    let timestamp = now();
    state
        .server_store
        .upsert_user(UserAccount {
            user_id: UserId::from("legacy_user"),
            display_name: "Legacy User".to_string(),
            github_user_id: None,
            github_login: None,
            created_at: timestamp,
            updated_at: timestamp,
        })
        .unwrap();
    let session = state.create_session(UserId::from("legacy_user")).unwrap();
    let path = temp.path().join(".labello-server/auth.json");
    let mut stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for record in stored["sessions"].as_object_mut().unwrap().values_mut() {
        record.as_object_mut().unwrap().remove("csrfToken");
    }
    std::fs::write(&path, serde_json::to_vec_pretty(&stored).unwrap()).unwrap();
    drop(state);

    let app = production_router(ApiState::new(temp.path()));
    let me = app
        .oneshot(
            Request::builder()
                .uri("/me")
                .header(
                    header::COOKIE,
                    format!("labello_session={}", session.cookie),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(me.status(), StatusCode::OK);
    let body = to_bytes(me.into_body(), usize::MAX).await.unwrap();
    let migrated: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let csrf = migrated["csrfToken"].as_str().unwrap();
    assert_eq!(csrf.len(), 64);
    let persisted = std::fs::read_to_string(path).unwrap();
    assert!(persisted.contains(csrf));
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
    let session_cookie = set_cookies
        .iter()
        .find(|cookie| cookie.starts_with("labello_session="))
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    let me = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/me")
                .header(header::COOKIE, session_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(me.status(), StatusCode::OK);
    let body = to_bytes(me.into_body(), usize::MAX).await.unwrap();
    let session: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(session["csrfToken"].as_str().unwrap().len(), 64);

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
                .uri("/imports")
                .header(header::ORIGIN, "https://app.remote.example")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .header(
                    header::ACCESS_CONTROL_REQUEST_HEADERS,
                    "content-type,x-csrf-token,idempotency-key,upload-offset,upload-length,digest",
                )
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
    for allowed in [
        "content-type",
        "x-csrf-token",
        "idempotency-key",
        "upload-offset",
        "upload-length",
        "digest",
    ] {
        assert!(
            allowed_headers.split(',').any(|header| header == allowed),
            "missing CORS header {allowed} in {allowed_headers}"
        );
    }

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

async fn get_assignment_availability(
    app: &axum::Router,
    user_id: &str,
    kind: &str,
) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/datasets/ds/assignments/availability?kind={kind}"))
                .header("x-test-user-id", user_id)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
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

struct ApiMigrationFixture {
    _temp: tempfile::TempDir,
    app: axum::Router,
    repository: labello_storage::DatasetRepository,
    image_id: ImageId,
    guide_task_id: TaskId,
    task_id: TaskId,
    targets: Vec<MigrationTarget>,
}

#[tokio::test]
async fn assembled_manual_migration_routes_enforce_contract_and_replay_end_to_end() {
    let fixture = api_migration_fixture().await;
    let assignment: Assignment = serde_json::from_value(
        claim_assignment_for_task(
            &fixture.app,
            "annotator",
            "annotation",
            fixture.task_id.as_str(),
        )
        .await,
    )
    .unwrap();
    let initial = migration_state(&fixture.app, &fixture.image_id, "annotator").await;
    let first = migration_expectation(&initial, &fixture.task_id, &fixture.targets[0]);

    let forged_terminal = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/datasets/ds/images/{}/admin/events",
                    fixture.image_id
                ))
                .header("x-test-user-id", "admin")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "payload": {
                            "kind": "task_state_changed",
                            "task_state": {
                                "taskId": fixture.task_id,
                                "status": "completed",
                                "outcome": "approved",
                                "assignedTo": null,
                                "completedBy": "admin",
                                "completedAt": labello_domain::now(),
                                "updatedAt": labello_domain::now()
                            }
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(forged_terminal.status(), StatusCode::BAD_REQUEST);

    let missing_key = migration_request(
        &fixture,
        "annotator",
        "skeleton",
        None,
        &labello_client::SaveMigrationSkeletonRequest {
            assignment_id: assignment.assignment_id.clone(),
            pass_id: None,
            target: first.clone(),
            skeleton: migration_skeleton(0.2),
        },
    )
    .await;
    assert_eq!(missing_key.0, StatusCode::BAD_REQUEST);

    let wrong_owner = migration_request(
        &fixture,
        "reviewer_1",
        "skeleton",
        Some("wrong-owner"),
        &labello_client::SaveMigrationSkeletonRequest {
            assignment_id: assignment.assignment_id.clone(),
            pass_id: None,
            target: first.clone(),
            skeleton: migration_skeleton(0.2),
        },
    )
    .await;
    assert_eq!(wrong_owner.0, StatusCode::UNAUTHORIZED);

    let mut wrong_group = first.clone();
    wrong_group.object_group_id = fixture.targets[1].object_group_id.clone();
    let wrong_group = migration_request(
        &fixture,
        "annotator",
        "skeleton",
        Some("wrong-group"),
        &labello_client::SaveMigrationSkeletonRequest {
            assignment_id: assignment.assignment_id.clone(),
            pass_id: None,
            target: wrong_group,
            skeleton: migration_skeleton(0.2),
        },
    )
    .await;
    assert_eq!(wrong_group.0, StatusCode::CONFLICT);

    let mut stale = first.clone();
    stale.expected_guide_annotation_version += 1;
    let stale = migration_request(
        &fixture,
        "annotator",
        "skeleton",
        Some("stale-skeleton"),
        &labello_client::SaveMigrationSkeletonRequest {
            assignment_id: assignment.assignment_id.clone(),
            pass_id: None,
            target: stale,
            skeleton: migration_skeleton(0.2),
        },
    )
    .await;
    assert_eq!(stale.0, StatusCode::CONFLICT);

    let save = labello_client::SaveMigrationSkeletonRequest {
        assignment_id: assignment.assignment_id.clone(),
        pass_id: None,
        target: first,
        skeleton: migration_skeleton(0.2),
    };
    let saved = successful_migration(
        migration_request(&fixture, "annotator", "skeleton", Some("save-first"), &save).await,
    );
    assert!(matches!(
        saved.cursor,
        Some(labello_domain::MigrationCursor::Object { ref object_group_id, .. })
            if object_group_id == &fixture.targets[1].object_group_id
    ));
    let current_skeleton = saved
        .image_state
        .current_annotation(&fixture.targets[0].reserved_skeleton_annotation_id)
        .unwrap();
    let mut edited_skeleton = current_skeleton.clone();
    edited_skeleton.version += 1;
    edited_skeleton.geometry = AnnotationGeometry::Skeleton(migration_skeleton(0.25));
    edited_skeleton.revision_source = RevisionSource::Human {
        action: HumanRevisionKind::Edited,
    };
    edited_skeleton.author_user_id = UserId::from("admin");
    edited_skeleton.updated_at = now();
    let added_skeleton = AnnotationVersion::native(
        AnnotationId::from("admin_added_skeleton"),
        fixture.task_id.clone(),
        ClassId::from("person"),
        AnnotationType::Skeleton,
        AnnotationGeometry::Skeleton(migration_skeleton(0.3)),
        UserId::from("admin"),
        now(),
    );
    for payload in [
        EventPayload::AnnotationVersionCreated {
            annotation: edited_skeleton,
            previous_version: Some(current_skeleton.version),
            reason: Some("manual repair edit".to_string()),
        },
        EventPayload::AnnotationDeleted {
            annotation_id: current_skeleton.annotation_id.clone(),
            version: current_skeleton.version,
            reason: Some("manual repair delete".to_string()),
        },
        EventPayload::AnnotationVersionCreated {
            annotation: added_skeleton,
            previous_version: None,
            reason: Some("manual repair add".to_string()),
        },
    ] {
        let response = admin_migration_repair_request(&fixture, payload).await;
        assert_eq!(response.0, StatusCode::BAD_REQUEST, "{}", response.1);
    }
    let sequence = saved.image_state.current_sequence;
    let retry = successful_migration(
        migration_request(&fixture, "annotator", "skeleton", Some("save-first"), &save).await,
    );
    assert_eq!(retry.image_state.current_sequence, sequence);

    let second = migration_expectation(&saved.image_state, &fixture.task_id, &fixture.targets[1]);
    let exclude = labello_client::ExcludeMigrationTargetRequest {
        assignment_id: assignment.assignment_id.clone(),
        pass_id: None,
        target: second,
        reason: labello_domain::MigrationExclusionReason::ObjectNotPresent,
        note: None,
    };
    let excluded = successful_migration(
        migration_request(
            &fixture,
            "annotator",
            "exclude",
            Some("exclude-second"),
            &exclude,
        )
        .await,
    );
    assert_eq!(
        excluded.cursor,
        Some(labello_domain::MigrationCursor::FullImage)
    );
    assert_eq!(excluded.progress.annotated, 1);
    assert_eq!(excluded.progress.excluded, 1);
    let sequence = excluded.image_state.current_sequence;
    let retry = successful_migration(
        migration_request(
            &fixture,
            "annotator",
            "exclude",
            Some("exclude-second"),
            &exclude,
        )
        .await,
    );
    assert_eq!(retry.image_state.current_sequence, sequence);

    let target_hash = excluded.image_state.migration_target_sets[&fixture.task_id]
        .target_set_hash
        .clone();
    let state_hash = excluded
        .image_state
        .current_migration_state_hash(&fixture.task_id)
        .unwrap();
    let wrong_task = migration_request(
        &fixture,
        "annotator",
        "passes",
        Some("wrong-task"),
        &labello_client::StartMigrationPassRequest {
            assignment_id: assignment.assignment_id.clone(),
            task_id: fixture.guide_task_id.clone(),
            expected_target_set_hash: target_hash.clone(),
            expected_state_hash: state_hash.clone(),
        },
    )
    .await;
    assert_eq!(wrong_task.0, StatusCode::BAD_REQUEST);
    let mut stale_pass = serde_json::to_value(labello_client::StartMigrationPassRequest {
        assignment_id: assignment.assignment_id.clone(),
        task_id: fixture.task_id.clone(),
        expected_target_set_hash: target_hash.clone(),
        expected_state_hash: state_hash.clone(),
    })
    .unwrap();
    stale_pass["expectedStateHash"] = json!("0".repeat(64));
    let stale_pass = import_json_request(
        &fixture.app,
        "POST",
        &format!("/datasets/ds/images/{}/migration/passes", fixture.image_id),
        "annotator",
        Some("stale-pass"),
        stale_pass,
    )
    .await;
    assert_eq!(stale_pass.0, StatusCode::CONFLICT);

    let pass_request = labello_client::StartMigrationPassRequest {
        assignment_id: assignment.assignment_id.clone(),
        task_id: fixture.task_id.clone(),
        expected_target_set_hash: target_hash,
        expected_state_hash: state_hash,
    };
    let pass = successful_migration(
        migration_request(
            &fixture,
            "annotator",
            "passes",
            Some("start-pass"),
            &pass_request,
        )
        .await,
    );
    let pass_id = pass.active_pass.as_ref().unwrap().pass_id.clone();
    let sequence = pass.image_state.current_sequence;
    let retry = successful_migration(
        migration_request(
            &fixture,
            "annotator",
            "passes",
            Some("start-pass"),
            &pass_request,
        )
        .await,
    );
    assert_eq!(retry.image_state.current_sequence, sequence);
    assert_eq!(retry.active_pass.unwrap().pass_id, pass_id);

    let keep = labello_client::KeepMigrationTargetRequest {
        assignment_id: assignment.assignment_id.clone(),
        pass_id: pass_id.clone(),
        target: migration_expectation(&pass.image_state, &fixture.task_id, &fixture.targets[0]),
    };
    let kept = successful_migration(
        migration_request(&fixture, "annotator", "keep", Some("keep-first"), &keep).await,
    );
    assert!(matches!(
        kept.cursor,
        Some(labello_domain::MigrationCursor::Object { ref object_group_id, .. })
            if object_group_id == &fixture.targets[1].object_group_id
    ));
    let sequence = kept.image_state.current_sequence;
    let retry = successful_migration(
        migration_request(&fixture, "annotator", "keep", Some("keep-first"), &keep).await,
    );
    assert_eq!(retry.image_state.current_sequence, sequence);

    let reopen = labello_client::ReopenMigrationTargetRequest {
        assignment_id: assignment.assignment_id.clone(),
        pass_id: Some(pass_id.clone()),
        target: migration_expectation(&kept.image_state, &fixture.task_id, &fixture.targets[1]),
    };
    let reopened = successful_migration(
        migration_request(
            &fixture,
            "annotator",
            "reopen",
            Some("reopen-second"),
            &reopen,
        )
        .await,
    );
    let sequence = reopened.image_state.current_sequence;
    let retry = successful_migration(
        migration_request(
            &fixture,
            "annotator",
            "reopen",
            Some("reopen-second"),
            &reopen,
        )
        .await,
    );
    assert_eq!(retry.image_state.current_sequence, sequence);

    let correction_exclude = labello_client::ExcludeMigrationTargetRequest {
        assignment_id: assignment.assignment_id.clone(),
        pass_id: Some(pass_id),
        target: migration_expectation(&reopened.image_state, &fixture.task_id, &fixture.targets[1]),
        reason: labello_domain::MigrationExclusionReason::NoValidSkeleton,
        note: None,
    };
    let corrected = successful_migration(
        migration_request(
            &fixture,
            "annotator",
            "exclude",
            Some("correction-exclude"),
            &correction_exclude,
        )
        .await,
    );
    assert_eq!(
        corrected.cursor,
        Some(labello_domain::MigrationCursor::FullImage)
    );
    let sequence = corrected.image_state.current_sequence;
    let retry = successful_migration(
        migration_request(
            &fixture,
            "annotator",
            "exclude",
            Some("correction-exclude"),
            &correction_exclude,
        )
        .await,
    );
    assert_eq!(retry.image_state.current_sequence, sequence);

    let target_hash = corrected.image_state.migration_target_sets[&fixture.task_id]
        .target_set_hash
        .clone();
    let state_hash = corrected
        .image_state
        .current_migration_state_hash(&fixture.task_id)
        .unwrap();
    let confirmation_hash = migration_confirmation_hash(&target_hash, &state_hash).unwrap();
    let mut stale_confirm = serde_json::to_value(labello_client::ConfirmMigrationRequest {
        assignment_id: assignment.assignment_id.clone(),
        task_id: fixture.task_id.clone(),
        target_set_hash: target_hash.clone(),
        state_hash: state_hash.clone(),
        confirmation_hash: confirmation_hash.clone(),
    })
    .unwrap();
    stale_confirm["confirmationHash"] = json!("f".repeat(64));
    let stale_confirm = import_json_request(
        &fixture.app,
        "POST",
        &format!("/datasets/ds/images/{}/migration/confirm", fixture.image_id),
        "annotator",
        Some("stale-confirm"),
        stale_confirm,
    )
    .await;
    assert_eq!(stale_confirm.0, StatusCode::CONFLICT);

    let confirm = labello_client::ConfirmMigrationRequest {
        assignment_id: assignment.assignment_id.clone(),
        task_id: fixture.task_id.clone(),
        target_set_hash: target_hash,
        state_hash,
        confirmation_hash,
    };
    let submitted = successful_migration(
        migration_request(
            &fixture,
            "annotator",
            "confirm",
            Some("confirm-first"),
            &confirm,
        )
        .await,
    );
    assert_eq!(
        submitted.image_state.task_states[&fixture.task_id].status,
        TaskStatus::Submitted
    );
    let sequence = submitted.image_state.current_sequence;
    let retry = successful_migration(
        migration_request(
            &fixture,
            "annotator",
            "confirm",
            Some("confirm-first"),
            &confirm,
        )
        .await,
    );
    assert_eq!(retry.image_state.current_sequence, sequence);

    let review_assignment: Assignment = serde_json::from_value(
        claim_assignment_for_task(
            &fixture.app,
            "reviewer_1",
            "review",
            fixture.task_id.as_str(),
        )
        .await,
    )
    .unwrap();
    let first_version = submitted.image_state.migration_dispositions[&fixture.task_id]
        [&fixture.targets[0].object_group_id]
        .disposition_version;
    let second_version = submitted.image_state.migration_dispositions[&fixture.task_id]
        [&fixture.targets[1].object_group_id]
        .disposition_version;
    let wrong_review_group = migration_request(
        &fixture,
        "reviewer_1",
        "review",
        Some("wrong-review-group"),
        &labello_client::ReviewMigrationRequest {
            assignment_id: review_assignment.assignment_id.clone(),
            task_id: fixture.task_id.clone(),
            target: labello_client::MigrationReviewTarget::Disposition {
                object_group_id: fixture.targets[1].object_group_id.clone(),
                disposition_version: second_version,
            },
            decision: ReviewDecision::Approved,
            comment: None,
        },
    )
    .await;
    assert_eq!(wrong_review_group.0, StatusCode::CONFLICT);
    let wrong_review_owner = migration_request(
        &fixture,
        "annotator",
        "review",
        Some("wrong-review-owner"),
        &labello_client::ReviewMigrationRequest {
            assignment_id: review_assignment.assignment_id.clone(),
            task_id: fixture.task_id.clone(),
            target: labello_client::MigrationReviewTarget::Disposition {
                object_group_id: fixture.targets[0].object_group_id.clone(),
                disposition_version: first_version,
            },
            decision: ReviewDecision::Approved,
            comment: None,
        },
    )
    .await;
    assert_eq!(wrong_review_owner.0, StatusCode::UNAUTHORIZED);
    let wrong_review_task = migration_request(
        &fixture,
        "reviewer_1",
        "review",
        Some("wrong-review-task"),
        &labello_client::ReviewMigrationRequest {
            assignment_id: review_assignment.assignment_id.clone(),
            task_id: fixture.guide_task_id.clone(),
            target: labello_client::MigrationReviewTarget::Disposition {
                object_group_id: fixture.targets[0].object_group_id.clone(),
                disposition_version: first_version,
            },
            decision: ReviewDecision::Approved,
            comment: None,
        },
    )
    .await;
    assert_eq!(wrong_review_task.0, StatusCode::BAD_REQUEST);

    let rejection = labello_client::ReviewMigrationRequest {
        assignment_id: review_assignment.assignment_id,
        task_id: fixture.task_id.clone(),
        target: labello_client::MigrationReviewTarget::Disposition {
            object_group_id: fixture.targets[0].object_group_id.clone(),
            disposition_version: first_version,
        },
        decision: ReviewDecision::Rejected,
        comment: Some("correct the first skeleton".to_string()),
    };
    let rejected = successful_migration(
        migration_request(
            &fixture,
            "reviewer_1",
            "review",
            Some("reject-first"),
            &rejection,
        )
        .await,
    );
    assert_eq!(
        rejected.image_state.task_states[&fixture.task_id].status,
        TaskStatus::NeedsCorrection
    );
    let sequence = rejected.image_state.current_sequence;
    let retry = successful_migration(
        migration_request(
            &fixture,
            "reviewer_1",
            "review",
            Some("reject-first"),
            &rejection,
        )
        .await,
    );
    assert_eq!(retry.image_state.current_sequence, sequence);

    let reopened_assignment: Assignment = serde_json::from_value(
        claim_assignment_for_task(
            &fixture.app,
            "annotator",
            "annotation",
            fixture.task_id.as_str(),
        )
        .await,
    )
    .unwrap();
    let correction_state = migration_state(&fixture.app, &fixture.image_id, "annotator").await;
    let correction = labello_client::SaveMigrationSkeletonRequest {
        assignment_id: reopened_assignment.assignment_id.clone(),
        pass_id: None,
        target: migration_expectation(&correction_state, &fixture.task_id, &fixture.targets[0]),
        skeleton: migration_skeleton(0.35),
    };
    let corrected = successful_migration(
        migration_request(
            &fixture,
            "annotator",
            "skeleton",
            Some("review-correction"),
            &correction,
        )
        .await,
    );
    assert_eq!(
        corrected.cursor,
        Some(labello_domain::MigrationCursor::FullImage)
    );

    let target_hash = corrected.image_state.migration_target_sets[&fixture.task_id]
        .target_set_hash
        .clone();
    let state_hash = corrected
        .image_state
        .current_migration_state_hash(&fixture.task_id)
        .unwrap();
    let confirmation_hash = migration_confirmation_hash(&target_hash, &state_hash).unwrap();
    let reconfirm = labello_client::ConfirmMigrationRequest {
        assignment_id: reopened_assignment.assignment_id,
        task_id: fixture.task_id.clone(),
        target_set_hash: target_hash,
        state_hash,
        confirmation_hash: confirmation_hash.clone(),
    };
    let resubmitted = successful_migration(
        migration_request(
            &fixture,
            "annotator",
            "confirm",
            Some("confirm-correction"),
            &reconfirm,
        )
        .await,
    );

    let final_review: Assignment = serde_json::from_value(
        claim_assignment_for_task(
            &fixture.app,
            "reviewer_2",
            "review",
            fixture.task_id.as_str(),
        )
        .await,
    )
    .unwrap();
    let mut reviewed = resubmitted;
    for (index, target) in fixture.targets.iter().enumerate() {
        let request = labello_client::ReviewMigrationRequest {
            assignment_id: final_review.assignment_id.clone(),
            task_id: fixture.task_id.clone(),
            target: labello_client::MigrationReviewTarget::Disposition {
                object_group_id: target.object_group_id.clone(),
                disposition_version: reviewed.image_state.migration_dispositions[&fixture.task_id]
                    [&target.object_group_id]
                    .disposition_version,
            },
            decision: ReviewDecision::Approved,
            comment: None,
        };
        let key = format!("approve-object-{index}");
        reviewed = successful_migration(
            migration_request(&fixture, "reviewer_2", "review", Some(&key), &request).await,
        );
        let sequence = reviewed.image_state.current_sequence;
        let retry = successful_migration(
            migration_request(&fixture, "reviewer_2", "review", Some(&key), &request).await,
        );
        assert_eq!(retry.image_state.current_sequence, sequence);
    }
    let final_approval = labello_client::ReviewMigrationRequest {
        assignment_id: final_review.assignment_id,
        task_id: fixture.task_id.clone(),
        target: labello_client::MigrationReviewTarget::Confirmation { confirmation_hash },
        decision: ReviewDecision::Approved,
        comment: None,
    };
    let approved = successful_migration(
        migration_request(
            &fixture,
            "reviewer_2",
            "review",
            Some("approve-confirmation"),
            &final_approval,
        )
        .await,
    );
    assert_eq!(
        approved.image_state.task_states[&fixture.task_id].status,
        TaskStatus::Completed
    );
    let sequence = approved.image_state.current_sequence;
    let retry = successful_migration(
        migration_request(
            &fixture,
            "reviewer_2",
            "review",
            Some("approve-confirmation"),
            &final_approval,
        )
        .await,
    );
    assert_eq!(retry.image_state.current_sequence, sequence);

    tokio::fs::remove_file(fixture.repository.state_path(&fixture.image_id))
        .await
        .unwrap();
    let reloaded = migration_state(&fixture.app, &fixture.image_id, "reviewer_2").await;
    assert_eq!(reloaded.current_sequence, sequence);
    assert_eq!(
        reloaded.task_states[&fixture.task_id].status,
        TaskStatus::Completed
    );
}

#[tokio::test]
async fn api_deleted_guide_can_only_be_resolved_by_canonical_exclusion() {
    let fixture = api_migration_fixture().await;
    let assignment: Assignment = serde_json::from_value(
        claim_assignment_for_task(
            &fixture.app,
            "annotator",
            "annotation",
            fixture.task_id.as_str(),
        )
        .await,
    )
    .unwrap();
    let initial = migration_state(&fixture.app, &fixture.image_id, "annotator").await;
    let saved = successful_migration(
        migration_request(
            &fixture,
            "annotator",
            "skeleton",
            Some("deleted-api-save"),
            &labello_client::SaveMigrationSkeletonRequest {
                assignment_id: assignment.assignment_id.clone(),
                pass_id: None,
                target: migration_expectation(&initial, &fixture.task_id, &fixture.targets[0]),
                skeleton: migration_skeleton(0.4),
            },
        )
        .await,
    );
    let guide = saved
        .image_state
        .current_annotation(&fixture.targets[0].guide_annotation_id)
        .unwrap();
    let deleted = admin_migration_repair_request(
        &fixture,
        EventPayload::AnnotationDeleted {
            annotation_id: guide.annotation_id.clone(),
            version: guide.version,
            reason: Some("invalid imported guide".to_string()),
        },
    )
    .await;
    assert_eq!(deleted.0, StatusCode::OK, "{}", deleted.1);
    let deleted_state = migration_state(&fixture.app, &fixture.image_id, "annotator").await;
    let expected = migration_expectation(&deleted_state, &fixture.task_id, &fixture.targets[0]);
    assert!(expected.expected_guide_deleted);
    let annotate = migration_request(
        &fixture,
        "annotator",
        "skeleton",
        Some("deleted-api-annotate"),
        &labello_client::SaveMigrationSkeletonRequest {
            assignment_id: assignment.assignment_id.clone(),
            pass_id: None,
            target: expected.clone(),
            skeleton: migration_skeleton(0.5),
        },
    )
    .await;
    assert_eq!(annotate.0, StatusCode::CONFLICT);
    let excluded = successful_migration(
        migration_request(
            &fixture,
            "annotator",
            "exclude",
            Some("deleted-api-exclude"),
            &labello_client::ExcludeMigrationTargetRequest {
                assignment_id: assignment.assignment_id,
                pass_id: None,
                target: expected,
                reason: labello_domain::MigrationExclusionReason::InvalidSourceBox,
                note: Some("guide removed by audited repair".to_string()),
            },
        )
        .await,
    );
    assert!(
        !excluded.image_state.migration_dependencies[&fixture.task_id]
            .contains_key(&fixture.targets[0].object_group_id)
    );
    assert!(
        excluded
            .image_state
            .current_annotation(&fixture.targets[0].reserved_skeleton_annotation_id)
            .unwrap()
            .deleted
    );
    assert_eq!(
        fixture
            .repository
            .rebuild_image_state(&fixture.image_id)
            .await
            .unwrap(),
        excluded.image_state
    );
}

async fn api_migration_fixture() -> ApiMigrationFixture {
    let temp = tempfile::tempdir().unwrap();
    let repository = labello_storage::DatasetRepository::new(temp.path().join("ds"));
    let image_id = ImageId::from("img_migration");
    let guide_task_id = TaskId::from("bounding_box:person");
    let task_id = TaskId::from("skeleton:person");
    let class_id = ClassId::from("person");
    let timestamp = now();
    let tutorial = TutorialContent {
        title: "Instructions".to_string(),
        example_text: "Annotate".to_string(),
        example_images: Vec::new(),
    };
    let mut metadata = DatasetMetadata::new(DatasetId::from("ds"), "Dataset", timestamp);
    metadata.label_classes.push(LabelClass {
        class_id: class_id.clone(),
        name: "Person".to_string(),
        color: "#ffffff".to_string(),
        description: None,
    });
    metadata.tasks.push(TaskDefinition {
        task_id: guide_task_id.clone(),
        name: "Person boxes".to_string(),
        annotation_type: AnnotationType::BoundingBox,
        class_ids: vec![class_id.clone()],
        instructions: tutorial.clone(),
        skeleton: None,
        review: ReviewConfig::default(),
        prelabel_config_ids: Vec::new(),
        manual_box_guide_migration: None,
        enabled: true,
    });
    metadata.tasks.push(TaskDefinition {
        task_id: task_id.clone(),
        name: "Person skeletons".to_string(),
        annotation_type: AnnotationType::Skeleton,
        class_ids: vec![class_id.clone()],
        instructions: tutorial,
        skeleton: Some(SkeletonSpec {
            keypoints: vec![KeypointSpec {
                name: "nose".to_string(),
                required: true,
            }],
            edges: Vec::new(),
            allow_hidden: true,
            allow_absent: false,
        }),
        review: ReviewConfig {
            required_reviews: 1,
            workflow: ReviewWorkflow::Approval,
            allow_reviewer_corrections: false,
            agreement_threshold: None,
        },
        prelabel_config_ids: Vec::new(),
        manual_box_guide_migration: Some(ManualBoxGuideMigration {
            guide_task_id: guide_task_id.clone(),
            cardinality: MigrationCardinality::ExactlyOne,
            allow_exclusion: true,
            sequence: MigrationSequence::ImportedSpatialOrderV1,
        }),
        enabled: true,
    });
    for (user, roles) in [
        ("admin", BTreeSet::from([DatasetRole::DataAdmin])),
        ("annotator", BTreeSet::from([DatasetRole::Annotator])),
        ("reviewer_1", BTreeSet::from([DatasetRole::Reviewer])),
        ("reviewer_2", BTreeSet::from([DatasetRole::Reviewer])),
    ] {
        metadata.role_assignments.push(DatasetRoleAssignment {
            dataset_id: DatasetId::from("ds"),
            user_id: UserId::from(user),
            roles,
            assigned_at: timestamp,
            assigned_by: None,
        });
    }
    repository.initialize(metadata).await.unwrap();
    repository
        .save_images_index(&ImagesIndex {
            schema_version: SCHEMA_VERSION,
            image_count: 1,
            images_by_hash: BTreeMap::from([(
                "migration-hash".to_string(),
                ImageRecord {
                    image_id: image_id.clone(),
                    blake3: "migration-hash".to_string(),
                    canonical_path: "images/migration.png".to_string(),
                    known_paths: vec!["images/migration.png".to_string()],
                    duplicate_paths: Vec::new(),
                    file_name: "migration.png".to_string(),
                    byte_size: 4,
                    width: 100,
                    height: 100,
                    media_type: "image/png".to_string(),
                    source_memberships: Some(vec!["train".to_string()]),
                },
            )]),
        })
        .await
        .unwrap();

    let import_id = ImportId::from("import_migration");
    let targets = (0..2)
        .map(|index| MigrationTarget {
            object_group_id: ObjectGroupId::from(format!("group_{index}")),
            guide_annotation_id: AnnotationId::from(format!("box_{index}")),
            reserved_skeleton_annotation_id: AnnotationId::from(format!("skeleton_{index}")),
            sequence_index: index,
        })
        .collect::<Vec<_>>();
    let annotations = targets
        .iter()
        .enumerate()
        .map(|(index, target)| AnnotationVersion {
            annotation_id: target.guide_annotation_id.clone(),
            version: 1,
            object_group_id: Some(target.object_group_id.clone()),
            origin: AnnotationOrigin::Imported {
                imported: ImportedOrigin {
                    import_id: import_id.clone(),
                    source_profile: SourceProfile {
                        profile_id: "test".to_string(),
                        profile_version: 1,
                    },
                    source_namespace: "fixture".to_string(),
                    source_object_key: format!("object_{index}"),
                    geometry_provenance: ImportGeometryProvenance::Direct,
                },
            },
            task_id: guide_task_id.clone(),
            class_id: class_id.clone(),
            annotation_type: AnnotationType::BoundingBox,
            revision_source: RevisionSource::Import {
                import_id: import_id.clone(),
            },
            geometry: AnnotationGeometry::BoundingBox(BoundingBox {
                x: 0.1 + index as f32 * 0.3,
                y: 0.1,
                width: 0.2,
                height: 0.2,
            }),
            author_user_id: UserId::from("importer"),
            created_at: timestamp,
            updated_at: timestamp,
            deleted: false,
        })
        .collect::<Vec<_>>();
    let target_set_hash = migration_target_set_hash(
        &MigrationHashContext {
            dataset_id: &DatasetId::from("ds"),
            image_id: &image_id,
            guide_task_id: &guide_task_id,
            target_task_id: &task_id,
        },
        &targets,
    )
    .unwrap();
    repository
        .append_payload(
            &image_id,
            &Actor {
                user_id: UserId::from("importer"),
                role: DatasetRole::DataAdmin,
            },
            EventPayload::ImportInitialized {
                import_id,
                annotations,
                task_initializations: vec![
                    ImportTaskInitialization {
                        task_id: guide_task_id.clone(),
                        coverage: ImportCoverage::Complete,
                        initial_state: TaskState {
                            task_id: guide_task_id.clone(),
                            status: TaskStatus::Completed,
                            outcome: Some(TaskOutcome::ImportedGroundTruth),
                            assigned_to: None,
                            completed_by: Some(UserId::from("importer")),
                            completed_at: Some(timestamp),
                            updated_at: timestamp,
                        },
                    },
                    ImportTaskInitialization {
                        task_id: task_id.clone(),
                        coverage: ImportCoverage::Incomplete,
                        initial_state: TaskState::new(task_id.clone(), timestamp),
                    },
                ],
                migration_target_sets: vec![MigrationTargetSetInitialization {
                    dataset_id: DatasetId::from("ds"),
                    guide_task_id: guide_task_id.clone(),
                    target_task_id: task_id.clone(),
                    target_set_hash,
                    targets: targets.clone(),
                }],
            },
        )
        .await
        .unwrap();
    let app = router(ApiState::new(temp.path()));
    ApiMigrationFixture {
        _temp: temp,
        app,
        repository,
        image_id,
        guide_task_id,
        task_id,
        targets,
    }
}

async fn migration_request<T: serde::Serialize>(
    fixture: &ApiMigrationFixture,
    user_id: &str,
    command: &str,
    idempotency_key: Option<&str>,
    request: &T,
) -> (StatusCode, Value) {
    import_json_request(
        &fixture.app,
        "POST",
        &format!(
            "/datasets/ds/images/{}/migration/{command}",
            fixture.image_id
        ),
        user_id,
        idempotency_key,
        serde_json::to_value(request).unwrap(),
    )
    .await
}

async fn admin_migration_repair_request(
    fixture: &ApiMigrationFixture,
    payload: EventPayload,
) -> (StatusCode, Value) {
    import_json_request(
        &fixture.app,
        "POST",
        &format!("/datasets/ds/images/{}/admin/events", fixture.image_id),
        "admin",
        None,
        json!({ "payload": payload }),
    )
    .await
}

fn successful_migration(
    response: (StatusCode, Value),
) -> labello_client::ManualMigrationCommandResult {
    assert_eq!(response.0, StatusCode::OK, "{}", response.1);
    serde_json::from_value(response.1).unwrap()
}

async fn migration_state(app: &axum::Router, image_id: &ImageId, user_id: &str) -> ImageState {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/datasets/ds/images/{image_id}"))
                .header("x-test-user-id", user_id)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_value(response_json(response).await).unwrap()
}

fn migration_expectation(
    state: &ImageState,
    task_id: &TaskId,
    target: &MigrationTarget,
) -> labello_client::MigrationTargetExpectation {
    let guide = state
        .current_annotation(&target.guide_annotation_id)
        .unwrap();
    let skeleton = state
        .current_annotation(&target.reserved_skeleton_annotation_id)
        .filter(|annotation| !annotation.deleted);
    labello_client::MigrationTargetExpectation {
        object_group_id: target.object_group_id.clone(),
        expected_guide_annotation_version: guide.version,
        expected_guide_deleted: guide.deleted,
        expected_disposition_version: state.migration_dispositions[task_id]
            [&target.object_group_id]
            .disposition_version,
        expected_skeleton_version: skeleton.map(|annotation| annotation.version),
    }
}

fn migration_skeleton(x: f32) -> SkeletonGeometry {
    SkeletonGeometry {
        keypoints: vec![KeypointAnnotation {
            name: "nose".to_string(),
            state: KeypointState::Visible,
            point: Some(NormalizedPoint { x, y: 0.5 }),
        }],
    }
}

async fn restarted_import_router(root: &std::path::Path) -> axum::Router {
    let service = labello_storage::ImportService::new(
        root,
        labello_storage::ImportConfig {
            enabled: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    service.recover().await.unwrap();
    router(
        ApiState::new(root)
            .with_bootstrap_admins([UserId::from("admin"), UserId::from("other")])
            .with_import_service(service),
    )
}

#[tokio::test]
async fn imports_all_profiles_publish_atomically_and_remain_accessible_after_restart() {
    let temp = tempfile::tempdir().unwrap();
    let import_service = labello_storage::ImportService::new(
        temp.path(),
        labello_storage::ImportConfig {
            enabled: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(import_service.capabilities().available);
    let state = ApiState::new(temp.path())
        .with_bootstrap_admins([UserId::from("admin"), UserId::from("other")])
        .with_import_service(import_service);
    let mut app = router(state);

    let unauthenticated = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/import-capabilities")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
    let forbidden = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/import-capabilities")
                .header("x-test-user-id", "viewer")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
    let capabilities = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/import-capabilities")
                .header("x-test-user-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(capabilities.status(), StatusCode::OK);
    let capabilities = response_json(capabilities).await;
    assert_eq!(capabilities["available"], true);
    assert_eq!(capabilities["profiles"].as_array().unwrap().len(), 4);

    let missing_key = import_json_request(
        &app,
        "POST",
        "/imports",
        "admin",
        None,
        json!({
            "destinationDatasetId": "missing-key",
            "destinationName": "Missing key",
            "profile": "coco_instances_gt_v1",
            "source": { "transport": "browser_folder" },
            "attestations": {
                "groundTruth": true, "exhaustive": true,
                "coverageScope": [], "provenance": "fixture"
            }
        }),
    )
    .await;
    assert_eq!(missing_key.0, StatusCode::BAD_REQUEST);
    let unsupported = import_json_request(
        &app,
        "POST",
        "/imports",
        "admin",
        Some("unsupported-profile"),
        json!({
            "destinationDatasetId": "unsupported",
            "destinationName": "Unsupported",
            "profile": "future_profile_v2",
            "source": { "transport": "browser_folder" },
            "attestations": {
                "groundTruth": true, "exhaustive": true,
                "coverageScope": [], "provenance": "fixture"
            }
        }),
    )
    .await;
    assert_eq!(unsupported.0, StatusCode::UNPROCESSABLE_ENTITY);
    let oversized = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/imports")
                .header("x-test-user-id", "admin")
                .header("idempotency-key", "oversized-control")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "destinationDatasetId": "oversized",
                        "destinationName": "x".repeat(2 * 1024 * 1024),
                        "profile": "coco_instances_gt_v1",
                        "source": { "transport": "browser_folder" },
                        "attestations": {
                            "groundTruth": true, "exhaustive": true,
                            "coverageScope": [], "provenance": "fixture"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let cancellable = import_json_request(
        &app,
        "POST",
        "/imports",
        "admin",
        Some("cancel-create"),
        json!({
            "destinationDatasetId": "cancelled-import",
            "destinationName": "Cancelled import",
            "profile": "coco_instances_gt_v1",
            "source": { "transport": "browser_folder" },
            "attestations": {
                "groundTruth": true, "exhaustive": true,
                "coverageScope": [], "provenance": "fixture"
            }
        }),
    )
    .await;
    assert_eq!(cancellable.0, StatusCode::OK);
    let cancelled = import_json_request(
        &app,
        "POST",
        &format!(
            "/imports/{}/cancel",
            cancellable.1["importId"].as_str().unwrap()
        ),
        "admin",
        Some("cancel-command"),
        json!({ "reason": "integration test" }),
    )
    .await;
    assert_eq!(cancelled.0, StatusCode::OK);
    assert_eq!(cancelled.1["lifecycle"], "cancelled");

    let png = png_bytes(4, 4);
    let yolo_detect = BTreeMap::from([
        (
            "dataset.yaml".to_string(),
            b"path: .\ntrain: images/train\nval: images/val\nnames: [person]\n".to_vec(),
        ),
        ("images/train/a.png".to_string(), png.clone()),
        ("images/val/b.png".to_string(), png_bytes(5, 4)),
        (
            "labels/train/a.txt".to_string(),
            b"0 0.5 0.5 0.5 0.5\n".to_vec(),
        ),
        (
            "labels/val/b.txt".to_string(),
            b"0 0.5 0.5 0.5 0.5\n".to_vec(),
        ),
    ]);
    let yolo_pose = BTreeMap::from([
        (
            "dataset.yaml".to_string(),
            b"path: .\ntrain: images/train\nnames: [person]\nkpt_shape: [2, 3]\nkpt_names:\n  0: [nose, tail]\n".to_vec(),
        ),
        ("images/train/a.png".to_string(), png.clone()),
        (
            "labels/train/a.txt".to_string(),
            b"0 0.5 0.5 0.5 0.5 0.05 0.1 2 0.2 0.3 1\n".to_vec(),
        ),
    ]);
    let coco_files = |keypoints: bool| {
        let category = if keypoints {
            json!({"id": 7, "name": "person", "keypoints": ["nose", "tail"], "skeleton": [[1, 2]]})
        } else {
            json!({"id": 7, "name": "person"})
        };
        let mut annotation = json!({
            "id": 99, "image_id": 42, "category_id": 7,
            "bbox": [1.0, 1.0, 2.0, 2.0], "area": 4.0,
            "iscrowd": 0, "segmentation": [[0.0, 0.0, 3.0, 0.0, 3.0, 3.0]]
        });
        if keypoints {
            annotation["keypoints"] = json!([1.0, 1.0, 2, 0, 0, 0]);
            annotation["num_keypoints"] = json!(1);
        }
        let descriptor = |category: Value, annotation: Value| {
            serde_json::to_vec(&json!({
                "images": [{"id": 42, "file_name": "a.png", "width": 4, "height": 4}],
                "categories": [category], "annotations": [annotation]
            }))
            .unwrap()
        };
        let mut files = BTreeMap::from([("images/a.png".to_string(), png.clone())]);
        if keypoints {
            let mut instances_annotation = annotation.clone();
            instances_annotation
                .as_object_mut()
                .unwrap()
                .remove("keypoints");
            instances_annotation
                .as_object_mut()
                .unwrap()
                .remove("num_keypoints");
            files.insert(
                "instances.json".to_string(),
                descriptor(json!({"id": 7, "name": "person"}), instances_annotation),
            );
            files.insert(
                "keypoints.json".to_string(),
                descriptor(category, annotation),
            );
        } else {
            files.insert(
                "annotations.json".to_string(),
                descriptor(category, annotation),
            );
        }
        files
    };
    let cases = vec![
        (
            "ultralytics_yolo_detect_v1",
            "yolo-api-detect",
            yolo_detect,
            "yolo_dataset",
            "dataset.yaml",
        ),
        (
            "ultralytics_yolo_pose_v1",
            "yolo-api-pose",
            yolo_pose,
            "yolo_dataset",
            "dataset.yaml",
        ),
        (
            "coco_instances_gt_v1",
            "coco-api-instances",
            coco_files(false),
            "coco_instances",
            "annotations.json",
        ),
        (
            "coco_keypoints_gt_v1",
            "coco-api-keypoints",
            coco_files(true),
            "coco_keypoints",
            "keypoints.json",
        ),
        (
            "coco_instances_gt_v1",
            "coco-api-sparse",
            BTreeMap::from([
                ("images/a.png".to_string(), png.clone()),
                (
                    "annotations.json".to_string(),
                    serde_json::to_vec(&json!({
                        "images": [{"id": 42, "file_name": "a.png", "width": 4, "height": 4}],
                        "categories": [
                            {"id": 3, "name": "person"},
                            {"id": 17, "name": "vehicle"}
                        ],
                        "annotations": [
                            {"id": 99, "image_id": 42, "category_id": 3, "bbox": [0.0, 0.0, 2.0, 2.0], "area": 4.0, "iscrowd": 0, "segmentation": [[0.0, 0.0, 2.0, 0.0, 2.0, 2.0]]},
                            {"id": 101, "image_id": 42, "category_id": 17, "bbox": [2.0, 2.0, 2.0, 2.0], "area": 4.0, "iscrowd": 0, "segmentation": [[2.0, 2.0, 4.0, 2.0, 4.0, 4.0]]}
                        ]
                    }))
                    .unwrap(),
                ),
            ]),
            "coco_instances",
            "annotations.json",
        ),
    ];
    let mut published = Vec::new();

    for (case_index, (profile, dataset_id, files, descriptor_kind, descriptor_path)) in
        cases.into_iter().enumerate()
    {
        let create_body = json!({
            "destinationDatasetId": dataset_id,
            "destinationName": format!("Imported {dataset_id}"),
            "profile": profile,
            "source": { "transport": "browser_folder" },
            "attestations": {
                "groundTruth": true,
                "exhaustive": true,
                "coverageScope": [],
                "provenance": "API integration fixture"
            }
        });
        let created = import_json_request(
            &app,
            "POST",
            "/imports",
            "admin",
            Some(&format!("create-{case_index}")),
            create_body.clone(),
        )
        .await;
        assert_eq!(created.0, StatusCode::OK, "{}", created.1);
        let import_id = created.1["importId"].as_str().unwrap().to_string();
        assert_eq!(
            created.1["recovery"]["attestations"],
            create_body["attestations"]
        );
        assert_eq!(created.1["recovery"]["registeredFiles"], json!([]));
        app = restarted_import_router(temp.path()).await;

        let replay = import_json_request(
            &app,
            "POST",
            "/imports",
            "admin",
            Some(&format!("create-{case_index}")),
            create_body,
        )
        .await;
        assert_eq!(replay.0, StatusCode::OK);
        assert_eq!(replay.1["importId"], import_id);
        if case_index == 0 {
            let owner_jobs = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/imports")
                        .header("x-test-user-id", "admin")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(owner_jobs.status(), StatusCode::OK);
            assert!(
                response_json(owner_jobs)
                    .await
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|job| job["importId"] == import_id)
            );
            let other_jobs = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/imports")
                        .header("x-test-user-id", "other")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(other_jobs.status(), StatusCode::OK);
            assert!(
                response_json(other_jobs)
                    .await
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }

        let hidden = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/datasets")
                    .header("x-test-user-id", "admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(hidden.status(), StatusCode::OK);
        assert!(
            response_json(hidden)
                .await
                .as_array()
                .unwrap()
                .iter()
                .all(|dataset| dataset["datasetId"] != dataset_id)
        );
        let reserved_create = import_json_request(
            &app,
            "POST",
            "/datasets",
            "admin",
            None,
            json!({
                "datasetId": dataset_id,
                "name": "Must not bypass import reservation",
                "adminUserId": "admin"
            }),
        )
        .await;
        assert_eq!(reserved_create.0, StatusCode::CONFLICT);

        let registrations = files
            .iter()
            .enumerate()
            .map(|(index, (path, bytes))| {
                json!({
                    "clientFileId": format!("client-{index}"),
                    "relativePath": path,
                    "byteSize": bytes.len(),
                    "blake3": blake3::hash(bytes).to_hex().to_string()
                })
            })
            .collect::<Vec<_>>();
        let registered = import_json_request(
            &app,
            "POST",
            &format!("/imports/{import_id}/files/register"),
            "admin",
            Some(&format!("register-{case_index}")),
            json!({ "files": registrations }),
        )
        .await;
        assert_eq!(registered.0, StatusCode::OK, "{}", registered.1);
        let mut file_ids = BTreeMap::new();
        for file in registered.1["files"].as_array().unwrap() {
            let client_index = file["clientFileId"]
                .as_str()
                .unwrap()
                .trim_start_matches("client-")
                .parse::<usize>()
                .unwrap();
            let path = files.keys().nth(client_index).unwrap();
            file_ids.insert(path.clone(), file["fileId"].as_str().unwrap().to_string());
        }
        if case_index == 0 {
            let incomplete = import_json_request(
                &app,
                "POST",
                &format!("/imports/{import_id}/yolo-descriptor/inspect"),
                "admin",
                None,
                json!({ "descriptorFileId": file_ids["dataset.yaml"] }),
            )
            .await;
            assert_eq!(incomplete.0, StatusCode::UNPROCESSABLE_ENTITY);
        }
        for (file_index, (path, bytes)) in files.iter().enumerate() {
            let digest = blake3::hash(bytes).to_hex().to_string();
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(format!(
                            "/imports/{import_id}/files/{}/chunks",
                            file_ids[path]
                        ))
                        .header("x-test-user-id", "admin")
                        .header(
                            "idempotency-key",
                            format!("chunk-{case_index}-{file_index}"),
                        )
                        .header("upload-offset", "0")
                        .header("upload-length", bytes.len().to_string())
                        .header("digest", digest)
                        .header(header::CONTENT_TYPE, "application/octet-stream")
                        .body(Body::from(bytes.clone()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "upload {path}");
        }
        app = restarted_import_router(temp.path()).await;
        if descriptor_kind == "yolo_dataset" {
            let inspection = import_json_request(
                &app,
                "POST",
                &format!("/imports/{import_id}/yolo-descriptor/inspect"),
                "admin",
                None,
                json!({ "descriptorFileId": file_ids[descriptor_path] }),
            )
            .await;
            assert_eq!(inspection.0, StatusCode::OK, "{}", inspection.1);
            assert_eq!(inspection.1["splits"][0]["name"], "train");
            assert_eq!(inspection.1["splits"][0]["usable"], true);
            if case_index == 0 {
                assert_eq!(inspection.1["splits"][1]["name"], "val");
                assert_eq!(inspection.1["splits"][1]["usable"], true);
                let wrong_owner = import_json_request(
                    &app,
                    "POST",
                    &format!("/imports/{import_id}/yolo-descriptor/inspect"),
                    "other",
                    None,
                    json!({ "descriptorFileId": file_ids[descriptor_path] }),
                )
                .await;
                assert_eq!(wrong_owner.0, StatusCode::NOT_FOUND);
                let forbidden = import_json_request(
                    &app,
                    "POST",
                    &format!("/imports/{import_id}/yolo-descriptor/inspect"),
                    "viewer",
                    None,
                    json!({ "descriptorFileId": file_ids[descriptor_path] }),
                )
                .await;
                assert_eq!(forbidden.0, StatusCode::FORBIDDEN);
            }
        }
        let uploading = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/imports/{import_id}"))
                    .header("x-test-user-id", "admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(uploading.status(), StatusCode::OK);
        let uploading = response_json(uploading).await;
        assert_eq!(
            uploading["recovery"]["registeredFiles"]
                .as_array()
                .unwrap()
                .len(),
            files.len()
        );
        let uploading_text = uploading.to_string();
        assert!(!uploading_text.contains("relativePath"));
        assert!(!uploading_text.contains("blake3"));
        let image_root = files
            .keys()
            .find(|path| path.ends_with("a.png"))
            .and_then(|path| file_ids.get(path))
            .cloned();
        let descriptors = if case_index == 3 {
            json!([
                {
                    "descriptorFileId": file_ids["instances.json"],
                    "kind": "coco_instances",
                    "release": "v1", "split": "train",
                    "imageRootFileId": image_root.clone(),
                    "pairingGroup": "people"
                },
                {
                    "descriptorFileId": file_ids["keypoints.json"],
                    "kind": "coco_keypoints",
                    "release": "v1", "split": "train",
                    "imageRootFileId": image_root.clone(),
                    "pairingGroup": "people"
                }
            ])
        } else {
            json!([{
                "descriptorFileId": file_ids[descriptor_path],
                "kind": descriptor_kind,
                "release": "v1", "split": "train",
                "imageRootFileId": if descriptor_kind == "yolo_dataset" { None } else { image_root.clone() },
                "pairingGroup": null
            }])
        };
        let selected_splits = if case_index == 0 {
            json!(["train", "val"])
        } else {
            json!(["train"])
        };
        let sealed = import_json_request(
            &app,
            "POST",
            &format!("/imports/{import_id}/seal"),
            "admin",
            Some(&format!("seal-{case_index}")),
            json!({
                "source": {
                    "sourceNamespace": format!("fixture-{case_index}"),
                    "descriptors": descriptors,
                    "selectedSplits": selected_splits,
                    "selectedCategoryKeys": []
                },
                "attestations": {
                    "groundTruth": true,
                    "exhaustive": true,
                    "coverageScope": [],
                    "provenance": "API integration fixture"
                }
            }),
        )
        .await;
        assert_eq!(sealed.0, StatusCode::OK, "{}", sealed.1);
        app = restarted_import_router(temp.path()).await;
        let sealed_job = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/imports/{import_id}"))
                    .header("x-test-user-id", "admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let sealed_job = response_json(sealed_job).await;
        assert_eq!(sealed_job["lifecycle"], "sealed");
        assert_eq!(
            sealed_job["recovery"]["source"]["descriptors"][0]["descriptorFileId"]
                .as_str()
                .unwrap(),
            if case_index == 3 {
                file_ids["instances.json"].as_str()
            } else {
                file_ids[descriptor_path].as_str()
            }
        );
        assert!(!sealed_job.to_string().contains(descriptor_path));
        let preflight = import_json_request(
            &app,
            "POST",
            &format!("/imports/{import_id}/preflight"),
            "admin",
            Some(&format!("preflight-{case_index}")),
            json!({ "restart": false }),
        )
        .await;
        assert_eq!(preflight.0, StatusCode::OK, "{}", preflight.1);
        app = restarted_import_router(temp.path()).await;
        let recovered_plan = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/imports/{import_id}/plan"))
                    .header("x-test-user-id", "admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(recovered_plan.status(), StatusCode::OK);
        let recovered_plan = response_json(recovered_plan).await;
        let source_categories = recovered_plan["sourceCategories"].as_array().unwrap();
        assert_eq!(source_categories.len(), if case_index == 4 { 2 } else { 1 });
        if case_index == 4 {
            assert_eq!(
                source_categories
                    .iter()
                    .map(|category| category["sourceCategoryId"].as_str().unwrap())
                    .collect::<BTreeSet<_>>(),
                BTreeSet::from(["3", "17"])
            );
        }
        let source_category = &source_categories[0];
        if case_index != 4 {
            assert_eq!(
                source_category["sourceCategoryId"],
                if case_index < 2 { "0" } else { "7" }
            );
            assert_eq!(source_category["sourceName"], "person");
        }
        assert!(
            source_category["directGeometry"]
                .as_array()
                .unwrap()
                .contains(&json!("bounding_box"))
        );
        assert!(source_category["generatedCategoryMapping"].is_object());
        assert!(source_category["currentTaskMappings"].is_array());
        if matches!(case_index, 1 | 3) {
            assert_eq!(
                source_category["keypointSchema"]["keypoints"][0]["name"],
                "nose"
            );
            assert!(
                source_category["directGeometry"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("skeleton"))
            );
        }
        let expected_geometry_tasks = if matches!(case_index, 0 | 1 | 3 | 4) {
            2
        } else {
            1
        };
        assert_eq!(
            preflight.1["preflightReport"]["coverage"]["complete"],
            expected_geometry_tasks
        );
        assert_eq!(
            preflight.1["preflightReport"]["coverage"]["verifiedEmpty"],
            0
        );
        assert_eq!(preflight.1["preflightReport"]["coverage"]["incomplete"], 0);
        assert_eq!(
            preflight.1["preflightReport"]["coverageByGeometry"]["boundingBoxes"]["complete"],
            if matches!(case_index, 0 | 4) { 2 } else { 1 }
        );
        assert_eq!(
            preflight.1["preflightReport"]["coverageByGeometry"]["skeletons"]["complete"],
            if matches!(case_index, 1 | 3) { 1 } else { 0 }
        );
        let mut plan_hash = preflight.1["planHash"].as_str().unwrap().to_string();
        if case_index == 0 {
            let current_plan = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/imports/{import_id}/plan"))
                        .header("x-test-user-id", "admin")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(current_plan.status(), StatusCode::OK);
            assert_eq!(response_json(current_plan).await["planHash"], plan_hash);
            let update_body = json!({
                "categoryMappings": [{
                    "sourceCategoryKey": "0", "sourceCategoryId": "0",
                    "classId": "mapped-person", "className": "Mapped person",
                    "color": "#123456", "selected": true
                }],
                "geometryMappings": [{
                    "sourceCategoryKey": "0", "sourceGeometry": "bounding_box",
                    "targetGeometry": "skeleton", "policy": "box_relative_template_v1",
                    "parameters": [
                        {"name": "nose", "x": 0.5, "y": 0.25, "state": "visible"},
                        {"name": "tail", "x": 0.5, "y": 0.75, "state": "hidden"}
                    ]
                }],
                "taskMappings": [{
                    "sourceCategoryKey": "0",
                    "task": {
                        "taskId": "skeleton:mapped-person", "name": "Mapped skeletons",
                        "annotationType": "skeleton", "classIds": ["mapped-person"],
                        "instructions": {"title": "Mapped skeletons", "exampleText": "Map skeletons", "exampleImages": []},
                        "skeleton": {
                            "keypoints": [
                                {"name": "nose", "required": false},
                                {"name": "tail", "required": false}
                            ],
                            "edges": [{"from": "nose", "to": "tail"}],
                            "allowHidden": true, "allowAbsent": true
                        },
                        "review": {"requiredReviews": 0, "workflow": "none", "allowReviewerCorrections": false, "agreementThreshold": null},
                        "prelabelConfigIds": [], "manualBoxGuideMigration": null, "enabled": true
                    },
                    "workflowIntent": "authoritative_ground_truth"
                }],
                "skeletonMappings": [{
                    "sourceCategoryKey": "0", "targetTaskId": "skeleton:mapped-person",
                    "sourceKeypointNames": [], "namesConfirmed": true,
                    "skeleton": {
                        "keypoints": [
                            {"name": "nose", "required": false},
                            {"name": "tail", "required": false}
                        ],
                        "edges": [{"from": "nose", "to": "tail"}],
                        "allowHidden": true, "allowAbsent": true
                    }
                }],
                "compatibility": {},
                "acknowledgements": []
            });
            let mut invalid_parameters = update_body.clone();
            invalid_parameters["geometryMappings"][0]["parameters"][0]["x"] = json!(1.1);
            let invalid = import_json_request(
                &app,
                "PUT",
                &format!("/imports/{import_id}/plan"),
                "admin",
                Some("update-plan-invalid-geometry"),
                invalid_parameters,
            )
            .await;
            assert_eq!(invalid.0, StatusCode::UNPROCESSABLE_ENTITY, "{}", invalid.1);
            let updated = import_json_request(
                &app,
                "PUT",
                &format!("/imports/{import_id}/plan"),
                "admin",
                Some("update-plan"),
                update_body.clone(),
            )
            .await;
            assert_eq!(updated.0, StatusCode::OK, "{}", updated.1);
            assert_eq!(
                updated.1["sourceFingerprint"],
                sealed.1["sourceFingerprint"]
            );
            assert_eq!(updated.1["commitReady"], false);
            assert_ne!(updated.1["planHash"], plan_hash);
            let mut accepted_body = update_body;
            accepted_body["acknowledgements"] = json!([{
                "diagnosticCode": "template_skeleton_derived",
                "policy": "accept derived pending seed",
                "affectedCount": 1,
                "acknowledged": true
            }]);
            let accepted = import_json_request(
                &app,
                "PUT",
                &format!("/imports/{import_id}/plan"),
                "admin",
                Some("update-plan-accepted"),
                accepted_body.clone(),
            )
            .await;
            assert_eq!(accepted.0, StatusCode::OK, "{}", accepted.1);
            assert_eq!(accepted.1["commitReady"], true);
            assert_eq!(
                accepted.1["acceptedRequest"],
                serde_json::to_value(
                    serde_json::from_value::<labello_client::UpdateImportPlanRequest>(
                        accepted_body.clone(),
                    )
                    .unwrap(),
                )
                .unwrap()
            );
            plan_hash = accepted.1["planHash"].as_str().unwrap().to_string();
            let retry = import_json_request(
                &app,
                "PUT",
                &format!("/imports/{import_id}/plan"),
                "admin",
                Some("update-plan-accepted"),
                accepted_body,
            )
            .await;
            assert_eq!(retry.0, StatusCode::OK);
            assert_eq!(retry.1["planHash"], plan_hash);
            let wrong_owner = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/imports/{import_id}/plan"))
                        .header("x-test-user-id", "other")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(wrong_owner.status(), StatusCode::NOT_FOUND);
        } else if case_index == 1 {
            let mut envelope_body = json!({
                "categoryMappings": [{
                    "sourceCategoryKey": "0", "sourceCategoryId": "0",
                    "classId": "person", "className": "Person", "color": "#123456",
                    "selected": true
                }],
                "geometryMappings": [{
                    "sourceCategoryKey": "0", "sourceGeometry": "skeleton",
                    "targetGeometry": "bounding_box", "policy": "keypoint_envelope_v1",
                    "parameters": [
                        {"name": "paddingRatio", "value": 0.05},
                        {"name": "minimumPixels", "value": 1.0},
                        {"name": "includeHidden", "value": true}
                    ]
                }],
                "taskMappings": [{
                    "sourceCategoryKey": "0",
                    "task": {
                        "taskId": "bounding_box:person", "name": "Person envelopes",
                        "annotationType": "bounding_box", "classIds": ["person"],
                        "instructions": {"title": "Person envelopes", "exampleText": "Review envelopes", "exampleImages": []},
                        "skeleton": null,
                        "review": {"requiredReviews": 1, "workflow": "approval", "allowReviewerCorrections": false, "agreementThreshold": null},
                        "prelabelConfigIds": [], "manualBoxGuideMigration": null, "enabled": true
                    },
                    "workflowIntent": "require_approval"
                }],
                "skeletonMappings": [], "compatibility": {}, "acknowledgements": []
            });
            let envelope = import_json_request(
                &app,
                "PUT",
                &format!("/imports/{import_id}/plan"),
                "admin",
                Some("pose-envelope-plan"),
                envelope_body.clone(),
            )
            .await;
            assert_eq!(envelope.0, StatusCode::OK, "{}", envelope.1);
            assert_eq!(envelope.1["commitReady"], false);
            assert_eq!(envelope.1["report"]["geometry"]["envelopeDerived"], 1);
            envelope_body["acknowledgements"] = json!([{
                "diagnosticCode": "keypoint_envelope_derived", "policy": "accept envelope",
                "affectedCount": 1, "acknowledged": true
            }, {
                "diagnosticCode": "keypoint_envelope_clipped", "policy": "accept clipped envelope",
                "affectedCount": 1, "acknowledged": true
            }]);
            let envelope = import_json_request(
                &app,
                "PUT",
                &format!("/imports/{import_id}/plan"),
                "admin",
                Some("pose-envelope-accepted"),
                envelope_body,
            )
            .await;
            assert_eq!(envelope.0, StatusCode::OK, "{}", envelope.1);
            assert_eq!(envelope.1["commitReady"], true, "{}", envelope.1);
            plan_hash = envelope.1["planHash"].as_str().unwrap().to_string();
        } else if case_index == 4 {
            let source_key = |source_id: &str| {
                recovered_plan["sourceCategories"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|category| category["sourceCategoryId"] == source_id)
                    .unwrap()["sourceCategoryKey"]
                    .as_str()
                    .unwrap()
                    .to_string()
            };
            let person_key = source_key("3");
            let vehicle_key = source_key("17");
            let skeleton = json!({
                "keypoints": [{"name": "center", "required": false}],
                "edges": [], "allowHidden": true, "allowAbsent": true
            });
            let manual = import_json_request(
                &app,
                "PUT",
                &format!("/imports/{import_id}/plan"),
                "admin",
                Some("sparse-manual-and-direct"),
                json!({
                    "categoryMappings": [
                        {"sourceCategoryKey": person_key, "sourceCategoryId": "3", "classId": "person", "className": "Person", "color": "#123456", "selected": true},
                        {"sourceCategoryKey": vehicle_key, "sourceCategoryId": "17", "classId": "vehicle", "className": "Vehicle", "color": "#654321", "selected": true}
                    ],
                    "geometryMappings": [
                        {"sourceCategoryKey": person_key, "sourceGeometry": "bounding_box", "targetGeometry": "bounding_box", "policy": "direct", "parameters": []},
                        {"sourceCategoryKey": person_key, "sourceGeometry": "bounding_box", "targetGeometry": "skeleton", "policy": "manual_box_guide_v1", "parameters": []},
                        {"sourceCategoryKey": vehicle_key, "sourceGeometry": "bounding_box", "targetGeometry": "bounding_box", "policy": "direct", "parameters": []}
                    ],
                    "taskMappings": [
                        {
                            "sourceCategoryKey": person_key,
                            "task": {
                                "taskId": "bounding_box:person", "name": "Person guides", "annotationType": "bounding_box", "classIds": ["person"],
                                "instructions": {"title": "Person guides", "exampleText": "Use imported guides", "exampleImages": []},
                                "skeleton": null,
                                "review": {"requiredReviews": 1, "workflow": "approval", "allowReviewerCorrections": false, "agreementThreshold": null},
                                "prelabelConfigIds": [], "manualBoxGuideMigration": null, "enabled": true
                            },
                            "workflowIntent": "require_approval"
                        },
                        {
                            "sourceCategoryKey": person_key,
                            "task": {
                                "taskId": "skeleton:person", "name": "Person skeletons", "annotationType": "skeleton", "classIds": ["person"],
                                "instructions": {"title": "Person skeletons", "exampleText": "Migrate every guide", "exampleImages": []},
                                "skeleton": skeleton,
                                "review": {"requiredReviews": 1, "workflow": "approval", "allowReviewerCorrections": false, "agreementThreshold": null},
                                "prelabelConfigIds": [],
                                "manualBoxGuideMigration": {"guideTaskId": "bounding_box:person", "cardinality": "exactly_one", "allowExclusion": true, "sequence": "imported_spatial_order_v1"},
                                "enabled": true
                            },
                            "workflowIntent": "require_approval"
                        },
                        {
                            "sourceCategoryKey": vehicle_key,
                            "task": {
                                "taskId": "bounding_box:vehicle", "name": "Vehicle seeds", "annotationType": "bounding_box", "classIds": ["vehicle"],
                                "instructions": {"title": "Vehicle seeds", "exampleText": "Continue from imported seeds", "exampleImages": []},
                                "skeleton": null,
                                "review": {"requiredReviews": 1, "workflow": "approval", "allowReviewerCorrections": false, "agreementThreshold": null},
                                "prelabelConfigIds": [], "manualBoxGuideMigration": null, "enabled": true
                            },
                            "workflowIntent": "seed_future_annotation"
                        }
                    ],
                    "skeletonMappings": [{
                        "sourceCategoryKey": person_key, "targetTaskId": "skeleton:person",
                        "skeleton": skeleton, "sourceKeypointNames": [], "namesConfirmed": true
                    }],
                    "compatibility": {}, "acknowledgements": []
                }),
            )
            .await;
            assert_eq!(manual.0, StatusCode::OK, "{}", manual.1);
            assert_eq!(manual.1["commitReady"], true, "{}", manual.1);
            assert_eq!(
                manual.1["acceptedRequest"]["categoryMappings"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
            plan_hash = manual.1["planHash"].as_str().unwrap().to_string();
        }
        let diagnostics = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/imports/{import_id}/diagnostics?limit=1"))
                    .header("x-test-user-id", "admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(diagnostics.status(), StatusCode::OK);
        let diagnostics = response_json(diagnostics).await;
        assert!(diagnostics["diagnostics"].is_array());
        assert!(diagnostics["total"].is_number());

        let hidden_from_other_owner = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/imports/{import_id}"))
                    .header("x-test-user-id", "other")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(hidden_from_other_owner.status(), StatusCode::NOT_FOUND);

        let committed = import_json_request(
            &app,
            "POST",
            &format!("/imports/{import_id}/commit"),
            "admin",
            Some(&format!("commit-{case_index}")),
            json!({ "planHash": plan_hash }),
        )
        .await;
        assert_eq!(committed.0, StatusCode::OK, "{}", committed.1);
        assert_eq!(committed.1["datasetId"], dataset_id);
        published.push((import_id.clone(), plan_hash, case_index));

        let dataset = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/datasets/{dataset_id}"))
                    .header("x-test-user-id", "admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(dataset.status(), StatusCode::OK);

        let collision = import_json_request(
            &app,
            "POST",
            "/imports",
            "admin",
            Some(&format!("collision-{case_index}")),
            json!({
                "destinationDatasetId": dataset_id,
                "destinationName": "Collision",
                "profile": profile,
                "source": { "transport": "browser_folder" },
                "attestations": {
                    "groundTruth": true, "exhaustive": true,
                    "coverageScope": [], "provenance": "fixture"
                }
            }),
        )
        .await;
        assert_eq!(collision.0, StatusCode::CONFLICT);
    }

    drop(app);
    let restarted_service = labello_storage::ImportService::new(
        temp.path(),
        labello_storage::ImportConfig {
            enabled: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    restarted_service.recover().await.unwrap();
    let restarted = router(
        ApiState::new(temp.path())
            .with_bootstrap_admins([UserId::from("admin")])
            .with_import_service(restarted_service),
    );
    for (import_id, plan_hash, case_index) in published {
        let job = restarted
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/imports/{import_id}"))
                    .header("x-test-user-id", "admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(job.status(), StatusCode::OK);
        assert_eq!(response_json(job).await["lifecycle"], "succeeded");
        let replay = import_json_request(
            &restarted,
            "POST",
            &format!("/imports/{import_id}/commit"),
            "admin",
            Some(&format!("commit-{case_index}")),
            json!({ "planHash": plan_hash }),
        )
        .await;
        assert_eq!(replay.0, StatusCode::OK, "{}", replay.1);
    }
}

#[tokio::test]
async fn server_directory_import_copies_selected_source_and_publishes() {
    let datasets = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(source.path().join("images/train")).unwrap();
    std::fs::create_dir_all(source.path().join("labels/train")).unwrap();
    std::fs::write(
        source.path().join("dataset.yaml"),
        b"path: .\ntrain: images/train\nnames: [person]\n",
    )
    .unwrap();
    std::fs::write(source.path().join("images/train/a.png"), png_bytes(4, 4)).unwrap();
    std::fs::write(
        source.path().join("labels/train/a.txt"),
        b"0 0.5 0.5 0.5 0.5\n",
    )
    .unwrap();
    let service = labello_storage::ImportService::new(
        datasets.path(),
        labello_storage::ImportConfig {
            enabled: true,
            import_roots: vec![labello_storage::ImportRoot {
                root_id: "releases".to_string(),
                path: source.path().to_path_buf(),
                allowed_owners: vec![UserId::from("admin")],
            }],
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let app = router(ApiState::new(datasets.path()).with_import_service(service));
    let browsed_root = import_json_request(
        &app,
        "POST",
        "/import-roots/releases/browse",
        "admin",
        None,
        json!({ "relativePath": "", "offset": 0 }),
    )
    .await;
    assert_eq!(browsed_root.0, StatusCode::OK, "{}", browsed_root.1);
    assert!(
        browsed_root.1["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["kind"] == "directory")
    );
    assert!(
        !browsed_root
            .1
            .to_string()
            .contains(source.path().to_str().unwrap())
    );
    let created = import_json_request(
        &app,
        "POST",
        "/imports",
        "admin",
        Some("server-create"),
        json!({
            "destinationDatasetId": "server-import",
            "destinationName": "Server import",
            "profile": "ultralytics_yolo_detect_v1",
            "source": {
                "transport": "server_directory",
                "importRootId": "releases",
                "relativePath": ""
            },
            "attestations": {
                "groundTruth": true, "exhaustive": true,
                "coverageScope": [], "provenance": "curated release"
            }
        }),
    )
    .await;
    assert_eq!(created.0, StatusCode::OK, "{}", created.1);
    assert_eq!(created.1["lifecycle"], "uploading");
    let import_id = created.1["importId"].as_str().unwrap();
    let browsed_source = import_json_request(
        &app,
        "POST",
        &format!("/imports/{import_id}/source/browse"),
        "admin",
        None,
        json!({ "relativePath": "", "offset": 0, "mode": "descriptors" }),
    )
    .await;
    assert_eq!(browsed_source.0, StatusCode::OK, "{}", browsed_source.1);
    let descriptor_file_id = browsed_source.1["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["relativePath"] == "dataset.yaml")
        .and_then(|entry| entry["fileId"].as_str())
        .unwrap()
        .to_string();
    let inspection = import_json_request(
        &app,
        "POST",
        &format!("/imports/{import_id}/yolo-descriptor/inspect"),
        "admin",
        None,
        json!({ "descriptorFileId": descriptor_file_id }),
    )
    .await;
    assert_eq!(inspection.0, StatusCode::OK, "{}", inspection.1);
    assert_eq!(inspection.1["splits"][0]["name"], "train");
    assert_eq!(inspection.1["splits"][0]["usable"], true);
    let sealed = import_json_request(
        &app,
        "POST",
        &format!("/imports/{import_id}/seal"),
        "admin",
        Some("server-seal"),
        json!({
            "source": {
                "sourceNamespace": "server-release",
                "descriptors": [{
                    "descriptorFileId": descriptor_file_id,
                    "kind": "yolo_dataset",
                    "release": "v1",
                    "split": "train"
                }],
                "selectedSplits": ["train"],
                "selectedCategoryKeys": []
            },
            "attestations": {
                "groundTruth": true, "exhaustive": true,
                "coverageScope": [], "provenance": "curated release"
            }
        }),
    )
    .await;
    assert_eq!(sealed.0, StatusCode::OK, "{}", sealed.1);
    let preflight = import_json_request(
        &app,
        "POST",
        &format!("/imports/{import_id}/preflight"),
        "admin",
        Some("server-preflight"),
        json!({ "restart": false }),
    )
    .await;
    assert_eq!(preflight.0, StatusCode::OK, "{}", preflight.1);
    let committed = import_json_request(
        &app,
        "POST",
        &format!("/imports/{import_id}/commit"),
        "admin",
        Some("server-commit"),
        json!({ "planHash": preflight.1["planHash"] }),
    )
    .await;
    assert_eq!(committed.0, StatusCode::OK, "{}", committed.1);
    let dataset = app
        .oneshot(
            Request::builder()
                .uri("/datasets/server-import")
                .header("x-test-user-id", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(dataset.status(), StatusCode::OK);
}

#[tokio::test]
async fn import_mutations_require_session_csrf_and_allowed_browser_origin() {
    let temp = tempfile::tempdir().unwrap();
    let service = labello_storage::ImportService::new(
        temp.path(),
        labello_storage::ImportConfig {
            enabled: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let state = ApiState::new(temp.path())
        .with_browser_origins(vec!["https://app.example.com".to_string()])
        .unwrap()
        .with_import_service(service);
    let timestamp = now();
    state
        .server_store
        .upsert_user(UserAccount {
            user_id: UserId::from("admin"),
            display_name: "Admin".to_string(),
            github_user_id: None,
            github_login: None,
            created_at: timestamp,
            updated_at: timestamp,
        })
        .unwrap();
    let session = state.create_session(UserId::from("admin")).unwrap();
    let app = production_router(state);
    let body = json!({
        "destinationDatasetId": "csrf-import",
        "destinationName": "CSRF import",
        "profile": "coco_instances_gt_v1",
        "source": { "transport": "browser_folder" },
        "attestations": {
            "groundTruth": true, "exhaustive": true,
            "coverageScope": [], "provenance": "fixture"
        }
    })
    .to_string();
    let request = |csrf: Option<&str>, origin: Option<&str>| {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/imports")
            .header(
                header::COOKIE,
                format!("labello_session={}", session.cookie),
            )
            .header("idempotency-key", "csrf-create")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(csrf) = csrf {
            builder = builder.header(crate::csrf::HEADER, csrf);
        }
        if let Some(origin) = origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        builder.body(Body::from(body.clone())).unwrap()
    };
    let missing = app.clone().oneshot(request(None, None)).await.unwrap();
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
    let wrong_origin = app
        .clone()
        .oneshot(request(
            Some(&session.csrf),
            Some("https://other.example.com"),
        ))
        .await
        .unwrap();
    assert_eq!(wrong_origin.status(), StatusCode::UNAUTHORIZED);
    let accepted = app
        .oneshot(request(
            Some(&session.csrf),
            Some("https://app.example.com"),
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::OK);
}

async fn import_json_request(
    app: &axum::Router,
    method: &str,
    uri: &str,
    user_id: &str,
    idempotency_key: Option<&str>,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-test-user-id", user_id)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(key) = idempotency_key {
        request = request.header("idempotency-key", key);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    (status, response_json(response).await)
}

async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap_or_else(|error| {
        panic!(
            "response was not JSON: {error}: {}",
            String::from_utf8_lossy(&body)
        )
    })
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
