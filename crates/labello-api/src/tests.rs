use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use serde_json::json;
use tower::ServiceExt;

use crate::{ApiState, router};

#[tokio::test]
async fn creates_dataset_and_enforces_dataset_headers() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets")
                .header("x-user-id", "admin")
                .header("x-user-role", "data_admin")
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
                .header("x-user-id", "admin")
                .header("x-user-role", "data_admin")
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
async fn protects_admin_dataset_config() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()));
    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets")
                .header("x-user-id", "admin")
                .header("x-user-role", "data_admin")
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
                .header("x-user-id", "intruder")
                .header("x-user-role", "data_admin")
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
                .header("x-user-id", "admin")
                .header("x-user-role", "data_admin")
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
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/datasets/ds/admin")
                .header("x-user-id", "admin")
                .header("x-user-role", "data_admin")
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
}

#[tokio::test]
async fn rejects_dev_header_identity_without_configured_token() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(ApiState::new(temp.path()).with_dev_auth_token(Some("secret".to_string())));

    let missing_token = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets")
                .header("x-user-id", "admin")
                .header("x-user-role", "data_admin")
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
    assert_eq!(missing_token.status(), StatusCode::UNAUTHORIZED);

    let valid_token = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets")
                .header("x-user-id", "admin")
                .header("x-user-role", "data_admin")
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
    assert_eq!(valid_token.status(), StatusCode::OK);
}
