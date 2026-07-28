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
