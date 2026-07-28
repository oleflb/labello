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

async fn api_migration_fixture() -> ApiMigrationFixture {
    let temp = tempfile::tempdir().unwrap();
    let state = ApiState::new(temp.path());
    let repository = state.repo(&DatasetId::from("ds")).unwrap().as_ref().clone();
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
    let app = router(state);
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

// Keep feature suites in this module so every test exercises the same assembled harness.
include!("tests/auth_security.rs");
include!("tests/datasets_admin.rs");
include!("tests/ingest.rs");
include!("tests/imports.rs");
include!("tests/snapshots.rs");
include!("tests/workflow.rs");
include!("tests/logging_redaction.rs");
