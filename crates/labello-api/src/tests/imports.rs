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
