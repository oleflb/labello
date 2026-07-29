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
async fn config_rejects_invalid_imbalance_ratio() {
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
    let metadata: serde_json::Value = serde_json::from_slice(&body).unwrap();

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
                        "imbalance": {
                            "maxRatio": 0.5,
                            "enforce": true
                        },
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
    assert!(
        String::from_utf8_lossy(&body)
            .contains("imbalance maxRatio must be finite and at least 1.0")
    );
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
    let related_kinds = stale_available["related"]
        .as_array()
        .unwrap()
        .iter()
        .map(|availability| availability["kind"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        related_kinds,
        BTreeSet::from(["review", "adjudication"]),
        "one scan should return the other authorized work-view caches"
    );
    let adjudication = get_assignment_availability(&app, "admin", "adjudication").await;
    assert_eq!(adjudication["kind"], "adjudication");
    assert_eq!(
        adjudication["tasks"]["bounding_box:pixel"],
        serde_json::Value::Bool(false)
    );
    assert!(
        claim_assignment(&app, "admin", "adjudication")
            .await
            .is_null()
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

    let revisit = labello_client::RevisitMigrationTargetRequest {
        assignment_id: assignment.assignment_id.clone(),
        pass_id: None,
        target: migration_expectation(&saved.image_state, &fixture.task_id, &fixture.targets[0]),
    };
    let revisited = successful_migration(
        migration_request(
            &fixture,
            "annotator",
            "revisit",
            Some("revisit-first"),
            &revisit,
        )
        .await,
    );
    assert!(matches!(
        revisited.cursor,
        Some(labello_domain::MigrationCursor::Object { ref object_group_id, .. })
            if object_group_id == &fixture.targets[0].object_group_id
    ));
    let sequence = revisited.image_state.current_sequence;
    let retry = successful_migration(
        migration_request(
            &fixture,
            "annotator",
            "revisit",
            Some("revisit-first"),
            &revisit,
        )
        .await,
    );
    assert_eq!(retry.image_state.current_sequence, sequence);
    let resumed = successful_migration(
        migration_request(
            &fixture,
            "annotator",
            "skeleton",
            Some("correct-revisited-first"),
            &labello_client::SaveMigrationSkeletonRequest {
                assignment_id: assignment.assignment_id.clone(),
                pass_id: None,
                target: migration_expectation(
                    &revisited.image_state,
                    &fixture.task_id,
                    &fixture.targets[0],
                ),
                skeleton: migration_skeleton(0.25),
            },
        )
        .await,
    );
    assert!(matches!(
        resumed.cursor,
        Some(labello_domain::MigrationCursor::Object { ref object_group_id, .. })
            if object_group_id == &fixture.targets[1].object_group_id
    ));

    let second =
        migration_expectation(&resumed.image_state, &fixture.task_id, &fixture.targets[1]);
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
        pass_id: Some(pass_id.clone()),
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

    let add_missing = labello_client::AddMigrationSkeletonRequest {
        assignment_id: assignment.assignment_id.clone(),
        pass_id: Some(pass_id),
        task_id: fixture.task_id.clone(),
        skeleton: migration_skeleton(0.8),
    };
    let discovered = successful_migration(
        migration_request(
            &fixture,
            "annotator",
            "skeletons",
            Some("add-missing"),
            &add_missing,
        )
        .await,
    );
    assert_eq!(
        discovered.cursor,
        Some(labello_domain::MigrationCursor::FullImage)
    );
    let discovered_id = discovered.annotation_id.clone().unwrap();
    assert!(
        discovered
            .image_state
            .current_annotation(&discovered_id)
            .unwrap()
            .object_group_id
            .is_none()
    );
    let sequence = discovered.image_state.current_sequence;
    let retry = successful_migration(
        migration_request(
            &fixture,
            "annotator",
            "skeletons",
            Some("add-missing"),
            &add_missing,
        )
        .await,
    );
    assert_eq!(retry.image_state.current_sequence, sequence);

    let target_hash = discovered.image_state.migration_target_sets[&fixture.task_id]
        .target_set_hash
        .clone();
    let state_hash = discovered
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
async fn migration_command_rejects_an_assignment_for_an_unindexed_image() {
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
    let sequence = initial.current_sequence;
    let event_count = fixture
        .repository
        .load_events(&fixture.image_id)
        .await
        .unwrap()
        .len();
    fixture
        .repository
        .save_images_index(&ImagesIndex::default())
        .await
        .unwrap();

    let response = migration_request(
        &fixture,
        "annotator",
        "skeleton",
        Some("unindexed-image"),
        &labello_client::SaveMigrationSkeletonRequest {
            assignment_id: assignment.assignment_id,
            pass_id: None,
            target: migration_expectation(&initial, &fixture.task_id, &fixture.targets[0]),
            skeleton: migration_skeleton(0.4),
        },
    )
    .await;
    assert_eq!(response.0, StatusCode::NOT_FOUND);
    assert_eq!(
        fixture
            .repository
            .load_image_state(&fixture.image_id)
            .await
            .unwrap()
            .current_sequence,
        sequence
    );
    assert_eq!(
        fixture
            .repository
            .load_events(&fixture.image_id)
            .await
            .unwrap()
            .len(),
        event_count
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
