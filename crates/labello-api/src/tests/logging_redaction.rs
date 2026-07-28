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
