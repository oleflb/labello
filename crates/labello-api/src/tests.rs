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
}
