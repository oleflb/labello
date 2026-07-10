use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use image::{ImageBuffer, Rgba};
use labello_domain::ImageId;
use serde_json::json;
use std::io::Cursor;
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
                .header("x-user-id", "admin")
                .header("x-user-role", "data_admin")
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
                .header("x-user-id", "admin")
                .header("x-user-role", "data_admin")
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
                .header("x-user-id", "admin")
                .header("x-user-role", "data_admin")
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
                .header("x-user-id", "../escape")
                .header("x-user-role", "annotator")
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
                .header("x-user-id", "admin")
                .header("x-user-role", "data_admin")
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
                .uri("/datasets/ds/images/img_1/reviews")
                .header("x-user-id", "admin")
                .header("x-user-role", "reviewer")
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
                .uri("/datasets/ds/images/img_1/adjudications")
                .header("x-user-id", "admin")
                .header("x-user-role", "adjudicator")
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
        "schemaVersion": 1,
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
                .header("x-user-id", "someone_else")
                .header("x-user-role", "annotator")
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
                .header("x-user-id", "admin")
                .header("x-user-role", "annotator")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "schemaVersion": 1,
                        "datasetId": "ds",
                        "userId": "admin",
                        "fragments": [{
                            "imageId": "img_1",
                            "baseSequence": 0,
                            "events": [{
                                "schemaVersion": 1,
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
                .header("x-user-id", "admin")
                .header("x-user-role", "annotator")
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
                    .header("x-user-id", "admin")
                    .header("x-user-role", "data_admin")
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
                .header("x-user-id", "admin")
                .header("x-user-role", "data_admin")
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
                    .header("x-user-id", "admin")
                    .header("x-user-role", "data_admin")
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
                .header("x-user-id", "admin")
                .header("x-user-role", "data_admin")
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
                .header("x-user-id", "admin")
                .header("x-user-role", "data_admin")
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
                .header("x-user-id", "admin")
                .header("x-user-role", "data_admin")
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
                .header("x-user-id", "admin")
                .header("x-user-role", "data_admin")
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

    for review_id in ["review_first", "review_duplicate"] {
        let approval = post_test_review(
            &app,
            &image_id,
            "admin",
            review_id,
            json!({ "targetType": "task", "task_id": "bounding_box:pixel" }),
            "approved",
        )
        .await;
        assert_eq!(approval.status(), StatusCode::OK);
    }
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
    let disabled = post_test_review(
        &app,
        &image_id,
        "admin",
        "review_disabled",
        json!({ "targetType": "task", "task_id": "bounding_box:pixel" }),
        "approved",
    )
    .await;
    assert_eq!(disabled.status(), StatusCode::BAD_REQUEST);
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
async fn assign_next_uses_camel_case_query() {
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
                .header("x-user-id", "admin")
                .header("x-user-role", "data_admin")
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
                .uri("/datasets/ds/images/next?taskId=bounding_box%3Apixel&kind=annotation")
                .header("x-user-id", "admin")
                .header("x-user-role", "data_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(assignment.status(), StatusCode::OK);
    let body = to_bytes(assignment.into_body(), usize::MAX).await.unwrap();
    let assignment: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(assignment["taskId"], "bounding_box:pixel");

    let stale_snake_case = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets/ds/images/next?task_id=bounding_box%3Apixel&kind=annotation")
                .header("x-user-id", "admin")
                .header("x-user-role", "data_admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stale_snake_case.status(), StatusCode::BAD_REQUEST);
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
                .header("x-user-id", "intruder")
                .header("x-user-role", "data_admin")
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
                .header("x-user-id", "admin")
                .header("x-user-role", "data_admin")
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
                .header("x-user-id", "admin")
                .header("x-user-role", "data_admin")
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

const TEST_BOUNDARY: &str = "LABELLOBOUNDARY";

async fn create_dataset(app: &axum::Router) {
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
                .header("x-user-id", "admin")
                .header("x-user-role", "data_admin")
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

    let update = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/datasets/ds/admin")
                .header("x-user-id", "admin")
                .header("x-user-role", "data_admin")
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

async fn upload_test_image(app: &axum::Router, file_name: &str, bytes: &[u8]) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/datasets/ds/uploads?root=uploads/test&ingest=true")
                .header("x-user-id", "admin")
                .header("x-user-role", "data_admin")
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
                .uri(format!("/datasets/ds/images/{image_id}/events"))
                .header("x-user-id", "admin")
                .header("x-user-role", "annotator")
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
    let timestamp = labello_domain::now().to_rfc3339();
    append_test_event(
        app,
        image_id,
        json!({
            "kind": "task_state_changed",
            "task_state": {
                "taskId": "bounding_box:pixel",
                "status": "submitted",
                "assignedTo": "admin",
                "completedBy": "admin",
                "completedAt": timestamp,
                "updatedAt": timestamp
            }
        }),
    )
    .await;
}

async fn post_test_review(
    app: &axum::Router,
    image_id: &ImageId,
    reviewer: &str,
    review_id: &str,
    target: serde_json::Value,
    decision: &str,
) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/datasets/ds/images/{image_id}/reviews"))
                .header("x-user-id", reviewer)
                .header("x-user-role", "reviewer")
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

async fn load_test_image_state(app: &axum::Router, image_id: &ImageId) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/datasets/ds/images/{image_id}"))
                .header("x-user-id", "admin")
                .header("x-user-role", "annotator")
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
