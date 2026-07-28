#[cfg(not(target_arch = "wasm32"))]
#[test]
fn yolo_descriptor_inspection_checks_every_usable_split_by_default() {
    let api = Rc::new(SpyApi::new());
    let mut app = base_live_app(api.clone());
    let scheduled = Rc::new(RefCell::new(None));
    let scheduled_for_spawner = scheduled.clone();
    app.set_native_task_spawner(move |future| {
        *scheduled_for_spawner.borrow_mut() = Some(future);
    });
    app.import_flow.profile = labello_client::ImportProfile::UltralyticsYoloDetectV1;
    app.import_flow.transport = labello_client::ImportTransport::ServerDirectory;
    app.import_flow.job = Some(test_import_job(
        DatasetId::from("yolo-inspection"),
        "YOLO inspection".to_string(),
        app.import_flow.profile,
        app.import_flow.transport,
    ));
    app.import_flow.descriptors = vec![crate::import_flow::ImportDescriptorDraft {
        descriptor_file_id: "dataset.yaml".to_string(),
        kind: labello_client::ImportDescriptorKind::YoloDataset,
        ..Default::default()
    }];

    app.request_yolo_descriptor_inspection();
    assert!(app.import_flow.yolo_inspection_loading);
    app.start_next_command();
    poll_ready_task(
        scheduled
            .borrow_mut()
            .take()
            .expect("inspection task was not scheduled"),
    );
    app.process_messages(&egui::Context::default());

    assert_eq!(api.counts().inspect_yolo_descriptor, 1);
    assert_eq!(
        app.import_flow
            .yolo_splits
            .iter()
            .map(|split| (split.name.as_str(), split.selected))
            .collect::<Vec<_>>(),
        vec![("train", true), ("val", true)]
    );
    assert_eq!(
        app.import_flow.yolo_inspected_descriptor_file_id.as_deref(),
        Some("dataset.yaml")
    );

    let import_id = app.import_flow.job.as_ref().unwrap().import_id.clone();
    let stale_request = app.import_request_identity(Some(import_id));
    app.runtime.active_requests.insert(stale_request.request_id);
    app.import_flow.pending_yolo_inspection_request_id = Some(stale_request.request_id + 1);
    app.import_flow.yolo_inspection_loading = true;
    app.runtime
        .tx
        .send(UiMessage::YoloDescriptorInspected {
            request: stale_request,
            descriptor_file_id: "dataset.yaml".to_string(),
            result: Ok(labello_client::YoloDescriptorInspection {
                splits: vec![labello_client::YoloSplitInspection {
                    name: "test".to_string(),
                    usable: true,
                    issue: None,
                }],
            }),
        })
        .unwrap();
    app.process_messages(&egui::Context::default());
    assert!(app.import_flow.yolo_inspection_loading);
    assert_eq!(
        app.import_flow
            .yolo_splits
            .iter()
            .map(|split| split.name.as_str())
            .collect::<Vec<_>>(),
        vec!["train", "val"]
    );

    let failed_request = app.import_request_identity(Some(
        app.import_flow.job.as_ref().unwrap().import_id.clone(),
    ));
    app.runtime
        .active_requests
        .insert(failed_request.request_id);
    app.import_flow.pending_yolo_inspection_request_id = Some(failed_request.request_id);
    app.runtime
        .tx
        .send(UiMessage::YoloDescriptorInspected {
            request: failed_request,
            descriptor_file_id: "dataset.yaml".to_string(),
            result: Err("The YAML is malformed.".to_string()),
        })
        .unwrap();
    app.process_messages(&egui::Context::default());
    assert!(!app.import_flow.yolo_inspection_loading);
    assert!(app.import_flow.yolo_splits.is_empty());
    assert_eq!(
        app.import_flow.yolo_inspection_error.as_deref(),
        Some("The YAML is malformed.")
    );

    app.request_yolo_descriptor_inspection();
    assert!(app.import_flow.yolo_inspection_loading);
    assert!(app.import_flow.yolo_inspection_error.is_none());
}

#[test]
fn setup_import_blocks_mapping_when_real_category_contract_is_absent() {
    let api = Rc::new(SpyApi::new());
    let mut harness = live_harness(api.clone());
    harness.set_size(egui::vec2(1180.0, 2600.0));
    step_until(&mut harness, 12, |app| !app.datasets.summaries.is_empty());

    select_setup_section(&mut harness, "Import");
    step_until(&mut harness, 12, |app| {
        app.import_flow.capabilities.is_some()
    });
    {
        let flow = &mut harness.state_mut().import_flow;
        flow.open = true;
        flow.destination_id = "imported".to_string();
        flow.destination_name = "Imported dataset".to_string();
        flow.transport = labello_client::ImportTransport::ServerDirectory;
        flow.server_root_id = "staging".to_string();
        flow.server_relative_path = "release-1".to_string();
        flow.ground_truth = true;
        flow.exhaustive = true;
        flow.coverage_scope = "person".to_string();
        flow.provenance = "curated benchmark".to_string();
    }
    harness.step();
    click(&mut harness, "Register import");
    step_until(&mut harness, 8, |app| app.import_flow.job.is_some());
    assert_eq!(api.counts().create_import, 1);
    assert!(harness.query_by_label("Source configuration").is_some());

    harness.state_mut().import_flow.descriptors[0].descriptor_file_id =
        "annotations/descriptor.json".to_string();
    harness.state_mut().import_flow.descriptors[0].image_root_file_id =
        "images/example.jpg".to_string();
    harness.step();
    click(&mut harness, "Seal source and run preflight");
    step_until(&mut harness, 12, |app| {
        app.import_flow
            .job
            .as_ref()
            .is_some_and(|job| job.preflight_report.is_some())
            && !app.import_flow.busy
    });
    assert_eq!(api.counts().seal_import, 1);
    assert_eq!(api.counts().preflight_import, 1);
    assert!(harness.query_by_label("Preflight summary").is_some());
    assert!(
        harness
            .query_by_label("Category and task mapping")
            .is_some()
    );

    assert!(
        harness
            .query_by_label("This API contract reports only a category count, not the discovered category keys, IDs, names, or skeleton schemas required for a valid plan. Mapping and commit are disabled; Labello will not guess sparse source IDs.")
            .is_some()
    );
    assert!(
        harness
            .get_by_role_and_label(
                egui::accesskit::Role::Button,
                "Save mappings and re-run preflight"
            )
            .accesskit_node()
            .is_disabled()
    );
    assert_eq!(api.counts().update_import_plan, 0);
    assert_eq!(api.counts().commit_import, 0);
}

#[test]
fn import_capability_is_bootstrap_admin_gated_and_stale_epochs_are_ignored() {
    let api = Rc::new(SpyApi::new());
    let mut harness = live_harness(api);
    step_until(&mut harness, 12, |app| !app.datasets.summaries.is_empty());
    select_setup_section(&mut harness, "Import");
    step_until(&mut harness, 12, |app| {
        app.import_flow.capabilities.is_some()
    });
    harness.state_mut().auth.can_create_datasets = false;
    harness.step();
    assert!(harness.query_by_label("Import").is_none());

    let app = harness.state_mut();
    app.auth.can_create_datasets = true;
    let request = app.import_request_identity(None);
    app.runtime.active_requests.insert(request.request_id);
    app.begin_import_epoch();
    app.runtime
        .tx
        .send(UiMessage::ImportCapabilitiesLoaded {
            request,
            result: Err("stale import response".to_string()),
        })
        .unwrap();
    app.process_messages(&egui::Context::default());
    assert!(app.import_flow.capabilities_error.is_none());
}

#[test]
fn terminal_import_result_invalidates_an_inflight_status_poll() {
    let mut app = LabelloApp::default();
    let mut job = test_import_job(
        DatasetId::from("imported"),
        "Imported".to_string(),
        labello_client::ImportProfile::CocoInstancesGtV1,
        labello_client::ImportTransport::ServerDirectory,
    );
    job.lifecycle = labello_client::ImportLifecycle::Building;
    let import_id = job.import_id.clone();
    app.import_flow.job = Some(job.clone());
    app.import_flow.screen = crate::import_flow::ImportScreen::Running;

    let commit = app.import_request_identity(Some(import_id.clone()));
    let poll = app.import_request_identity(Some(import_id.clone()));
    app.runtime.active_requests.insert(commit.request_id);
    app.runtime.active_requests.insert(poll.request_id);
    app.import_flow
        .active_operations
        .insert(commit.request_id, crate::app::ImportActivity::Commit);
    app.import_flow
        .active_operations
        .insert(poll.request_id, crate::app::ImportActivity::LoadStatus);

    app.runtime
        .tx
        .send(UiMessage::ImportCommitted {
            request: commit,
            result: Ok(labello_client::CommitImportResult {
                import_id: import_id.clone(),
                dataset_id: DatasetId::from("imported"),
                plan_hash: "plan-hash".to_string(),
                recovered: false,
            }),
        })
        .unwrap();
    app.runtime
        .tx
        .send(UiMessage::ImportJobLoaded {
            request: poll,
            result: Box::new(Ok(job)),
        })
        .unwrap();

    app.process_messages(&egui::Context::default());

    assert_eq!(
        app.import_flow.screen,
        crate::import_flow::ImportScreen::Success
    );
    assert_eq!(
        app.import_flow.job.as_ref().unwrap().lifecycle,
        labello_client::ImportLifecycle::Succeeded
    );
    assert!(app.import_flow.active_operations.is_empty());
}

#[test]
fn endpoint_and_session_identity_changes_clear_import_state() {
    let mut endpoint_app = LabelloApp::default();
    endpoint_app.import_flow.open = true;
    endpoint_app.import_flow.destination_id = "stale-import".to_string();
    endpoint_app.config.api_base_url = "http://127.0.0.1:8089".to_string();

    endpoint_app.rebuild_http_api();

    assert!(!endpoint_app.import_flow.open);
    assert!(endpoint_app.import_flow.destination_id.is_empty());

    let mut session_app = LabelloApp::default();
    session_app.auth.account = Some(UserAccount {
        user_id: UserId::from("old-user"),
        display_name: "Old user".to_string(),
        github_user_id: None,
        github_login: None,
        created_at: now(),
        updated_at: now(),
    });
    session_app.import_flow.open = true;
    session_app.import_flow.destination_id = "stale-import".to_string();
    let request = test_request(&session_app, 81_001, None);
    session_app.auth.active_session_request_id = Some(request.request_id);
    session_app
        .runtime
        .active_requests
        .insert(request.request_id);
    session_app
        .runtime
        .tx
        .send(UiMessage::SessionLoaded {
            request,
            result: Ok(SessionInfo {
                account: UserAccount {
                    user_id: UserId::from("new-user"),
                    display_name: "New user".to_string(),
                    github_user_id: None,
                    github_login: None,
                    created_at: now(),
                    updated_at: now(),
                },
                can_create_datasets: true,
                csrf_token: "test-token".to_string(),
            }),
        })
        .unwrap();

    session_app.process_messages(&egui::Context::default());

    assert!(!session_app.import_flow.open);
    assert!(session_app.import_flow.destination_id.is_empty());
}

#[test]
fn import_recovery_hydrates_persisted_source_plan_and_job_owned_state() {
    let api = Rc::new(SpyApi::new());
    let mut recovered = test_import_job(
        DatasetId::from("recovered"),
        "Recovered dataset".to_string(),
        labello_client::ImportProfile::CocoInstancesGtV1,
        labello_client::ImportTransport::ServerDirectory,
    );
    recovered.import_id = ImportId::from("imp_recovered");
    recovered.lifecycle = labello_client::ImportLifecycle::AwaitingDecision;
    recovered.source_fingerprint = Some("source-recovered".to_string());
    recovered.plan_hash = Some("plan-recovered".to_string());
    recovered.preflight_report = Some(test_import_report());
    let recovered_plan = contract_import_plan(recovered.import_id.clone());
    recovered.recovery = Some(labello_client::ImportRecoveryState {
        attestations: labello_client::ImportAttestations {
            ground_truth: true,
            exhaustive: true,
            coverage_scope: vec!["person".to_string()],
            provenance: "curated release".to_string(),
        },
        server_root_id: Some("staging".to_string()),
        source: Some(labello_client::ImportSourceConfiguration {
            source_namespace: "release".to_string(),
            descriptors: vec![
                labello_client::ImportDescriptorSelection {
                    descriptor_file_id: "file-instances".to_string(),
                    kind: labello_client::ImportDescriptorKind::CocoInstances,
                    release: "v1".to_string(),
                    split: "train".to_string(),
                    image_root_file_id: Some("file-image".to_string()),
                    pairing_group: Some("people".to_string()),
                },
                labello_client::ImportDescriptorSelection {
                    descriptor_file_id: "file-keypoints".to_string(),
                    kind: labello_client::ImportDescriptorKind::CocoKeypoints,
                    release: "v1".to_string(),
                    split: "train".to_string(),
                    image_root_file_id: Some("file-image".to_string()),
                    pairing_group: Some("people".to_string()),
                },
            ],
            selected_splits: vec!["train".to_string()],
            selected_category_keys: Vec::new(),
        }),
        registered_files: vec![labello_client::RegisteredImportFile {
            client_file_id: "client-instances".to_string(),
            file_id: "file-instances".to_string(),
            byte_size: 100,
            accepted_bytes: 100,
            complete: true,
        }],
        accepted_plan: Some(recovered_plan.clone()),
    });
    api.set_import_job(recovered.clone());
    let mut harness = live_harness(api);
    step_until(&mut harness, 8, |app| !app.datasets.summaries.is_empty());
    select_setup_section(&mut harness, "Import");
    step_until(&mut harness, 8, |app| {
        app.import_flow.capabilities.is_some()
    });
    {
        let flow = &mut harness.state_mut().import_flow;
        flow.open = true;
        flow.job = Some(test_import_job(
            DatasetId::from("old"),
            "Old dataset".to_string(),
            labello_client::ImportProfile::CocoInstancesGtV1,
            labello_client::ImportTransport::BrowserFolder,
        ));
        flow.plan = Some(labello_client::ImportPlan {
            import_id: ImportId::from("imp_test"),
            source_fingerprint: "old-source".to_string(),
            plan_hash: "old-plan".to_string(),
            commit_ready: true,
            blocking_diagnostic_codes: Vec::new(),
            required_acknowledgement_codes: Vec::new(),
            report: test_import_report(),
            source_categories: Vec::new(),
            accepted_request: None,
        });
        flow.registered_paths
            .push(crate::import_flow::RegisteredImportPath {
                client_file_id: "old-client".to_string(),
                file_id: "old-file".to_string(),
                relative_path: "old/descriptor.json".to_string(),
            });
        flow.diagnostics.push(Default::default());
        flow.recovery_import_id = "imp_recovered".to_string();
    }

    harness.state_mut().request_import_recovery();
    assert!(harness.state().import_flow.job.is_none());
    assert!(harness.state().import_flow.plan.is_none());
    assert!(harness.state().import_flow.registered_paths.is_empty());
    assert!(harness.state().import_flow.diagnostics.is_empty());
    harness.step();
    step_until(&mut harness, 8, |app| {
        app.import_flow
            .job
            .as_ref()
            .is_some_and(|job| job.import_id == recovered.import_id)
    });
    let flow = &harness.state().import_flow;
    assert_eq!(flow.plan.as_ref(), Some(&recovered_plan));
    assert!(!flow.recovery_contract_gap);
    assert_eq!(flow.profile, recovered.profile);
    assert_eq!(flow.transport, recovered.transport);
    assert_eq!(
        flow.destination_id,
        recovered.destination_dataset_id.as_str()
    );
    assert_eq!(flow.descriptors.len(), 2);
    assert_eq!(flow.categories.len(), 1);
    assert_eq!(flow.categories[0].source_category_id, "17");
    assert_eq!(
        flow.categories[0]
            .source_skeleton
            .as_ref()
            .unwrap()
            .keypoints[0]
            .name,
        "nose"
    );
    assert_eq!(
        flow.accepted_plan_request.as_ref(),
        recovered_plan.accepted_request.as_ref()
    );
    assert!(harness.query_by_label("Restart import setup").is_none());

    let mut yolo_recovered = test_import_job(
        DatasetId::from("recovered-yolo"),
        "Recovered YOLO".to_string(),
        labello_client::ImportProfile::UltralyticsYoloDetectV1,
        labello_client::ImportTransport::ServerDirectory,
    );
    yolo_recovered.recovery = Some(labello_client::ImportRecoveryState {
        attestations: labello_client::ImportAttestations {
            ground_truth: true,
            exhaustive: true,
            coverage_scope: Vec::new(),
            provenance: "curated release".to_string(),
        },
        server_root_id: Some("staging".to_string()),
        source: Some(labello_client::ImportSourceConfiguration {
            source_namespace: "release".to_string(),
            descriptors: vec![labello_client::ImportDescriptorSelection {
                descriptor_file_id: "dataset.yaml".to_string(),
                kind: labello_client::ImportDescriptorKind::YoloDataset,
                release: "v1".to_string(),
                split: "train".to_string(),
                image_root_file_id: None,
                pairing_group: None,
            }],
            selected_splits: vec!["train".to_string(), "val".to_string()],
            selected_category_keys: Vec::new(),
        }),
        registered_files: Vec::new(),
        accepted_plan: None,
    });
    harness
        .state_mut()
        .import_flow
        .hydrate_job_contract(&yolo_recovered);
    assert_eq!(
        harness
            .state()
            .import_flow
            .yolo_splits
            .iter()
            .map(|split| (split.name.as_str(), split.selected))
            .collect::<Vec<_>>(),
        vec![("train", true), ("val", true)]
    );

    let mut uploading_yolo = test_import_job(
        DatasetId::from("uploading-yolo"),
        "Uploading YOLO".to_string(),
        labello_client::ImportProfile::UltralyticsYoloDetectV1,
        labello_client::ImportTransport::BrowserFolder,
    );
    uploading_yolo.recovery = Some(labello_client::ImportRecoveryState {
        attestations: labello_client::ImportAttestations {
            ground_truth: true,
            exhaustive: true,
            coverage_scope: Vec::new(),
            provenance: "curated release".to_string(),
        },
        server_root_id: None,
        source: None,
        registered_files: Vec::new(),
        accepted_plan: None,
    });
    harness.state_mut().import_flow.descriptors = vec![Default::default()];
    harness
        .state_mut()
        .import_flow
        .hydrate_job_contract(&uploading_yolo);
    assert_eq!(
        harness.state().import_flow.descriptors[0].kind,
        labello_client::ImportDescriptorKind::YoloDataset
    );
}

#[test]
fn mapping_edits_and_failed_plan_responses_keep_commit_disabled() {
    let api = Rc::new(SpyApi::new());
    let mut harness = live_harness(api.clone());
    harness.set_size(egui::vec2(1180.0, 2600.0));
    step_until(&mut harness, 12, |app| !app.datasets.summaries.is_empty());
    select_setup_section(&mut harness, "Import");
    step_until(&mut harness, 12, |app| {
        app.import_flow.capabilities.is_some()
    });
    let job = test_import_job(
        DatasetId::from("imported"),
        "Imported".to_string(),
        labello_client::ImportProfile::CocoInstancesGtV1,
        labello_client::ImportTransport::ServerDirectory,
    );
    api.set_import_job(job.clone());
    {
        let flow = &mut harness.state_mut().import_flow;
        flow.open = true;
        flow.job = Some(job);
        flow.screen = crate::import_flow::ImportScreen::Preflight;
        flow.job.as_mut().unwrap().preflight_report = Some(test_import_report());
        flow.categories = vec![contract_import_category()];
        flow.exhaustive = true;
    }
    harness.step();
    harness.state_mut().request_update_import_plan();
    harness.state_mut().import_flow.categories[0].class_name = "Changed while saving".to_string();
    harness.step();
    step_until(&mut harness, 8, |app| !app.import_flow.busy);
    assert!(
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Commit import")
            .accesskit_node()
            .is_disabled()
    );
    assert_eq!(
        api.last_import_plan_request().unwrap().category_mappings[0].class_name,
        "Person"
    );

    api.fail_next_import_plan();
    harness.state_mut().request_update_import_plan();
    harness.step();
    step_until(&mut harness, 8, |app| !app.import_flow.busy);
    assert!(harness.state().import_flow.plan.is_none());
    assert!(
        harness
            .state()
            .import_flow
            .error
            .as_deref()
            .is_some_and(|error| error.contains("import plan failed"))
    );
}

#[test]
fn mutable_import_spy_accepts_multiple_manual_approval_categories() {
    let api = Rc::new(SpyApi::new());
    let mut harness = live_harness(api.clone());
    step_until(&mut harness, 12, |app| !app.datasets.summaries.is_empty());
    select_setup_section(&mut harness, "Import");
    step_until(&mut harness, 12, |app| {
        app.import_flow.capabilities.is_some()
    });
    let mut job = test_import_job(
        DatasetId::from("imported"),
        "Imported".to_string(),
        labello_client::ImportProfile::CocoInstancesGtV1,
        labello_client::ImportTransport::ServerDirectory,
    );
    let mut report = test_import_report();
    report.source.categories = 2;
    job.preflight_report = Some(report);
    api.set_import_job(job.clone());
    {
        let flow = &mut harness.state_mut().import_flow;
        flow.open = true;
        flow.job = Some(job);
        flow.screen = crate::import_flow::ImportScreen::Preflight;
        flow.exhaustive = true;
        let person = contract_import_category();
        let mut vehicle = contract_import_category();
        vehicle.source_category_key = "release:vehicle:18".to_string();
        vehicle.source_category_id = "18".to_string();
        vehicle.source_name = "Vehicle".to_string();
        vehicle.class_id = "vehicle".to_string();
        vehicle.class_name = "Vehicle".to_string();
        vehicle.bounding_box_task_id = "bounding_box:vehicle".to_string();
        vehicle.bounding_box_task_name = "Vehicle bounding boxes".to_string();
        vehicle.skeleton_task_id = "skeleton:vehicle".to_string();
        vehicle.skeleton_task_name = "Vehicle skeletons".to_string();
        flow.categories = vec![person, vehicle];
        for category in &mut flow.categories {
            category.source_skeleton = None;
            category.direct_geometry = vec![labello_client::ImportGeometryKind::BoundingBox];
            category.target_keypoint_names = "nose,left_eye".to_string();
            category.workflow_intent = labello_client::ImportWorkflowIntent::RequireApproval;
            category.geometry_mappings = vec![
                labello_client::ImportGeometryMappingRequest {
                    source_category_key: category.source_category_key.clone(),
                    source_geometry: labello_client::ImportGeometryKind::BoundingBox,
                    target_geometry: labello_client::ImportGeometryKind::BoundingBox,
                    policy: labello_client::ImportGeometryPolicy::Direct,
                    parameters: Vec::new(),
                },
                labello_client::ImportGeometryMappingRequest {
                    source_category_key: category.source_category_key.clone(),
                    source_geometry: labello_client::ImportGeometryKind::BoundingBox,
                    target_geometry: labello_client::ImportGeometryKind::Skeleton,
                    policy: labello_client::ImportGeometryPolicy::ManualBoxGuideV1,
                    parameters: Vec::new(),
                },
            ];
        }
    }

    harness.state_mut().request_update_import_plan();
    harness.step();
    step_until(&mut harness, 8, |app| {
        app.import_flow.plan.is_some() && !app.import_flow.busy
    });

    let request = api.last_import_plan_request().unwrap();
    assert_eq!(request.task_mappings.len(), 4);
    assert!(request.task_mappings.iter().all(|mapping| {
        mapping.task.review.workflow == labello_domain::ReviewWorkflow::Approval
            && mapping.task.review.required_reviews == 1
    }));
    assert_eq!(request.skeleton_mappings.len(), 2);
    assert!(
        request
            .skeleton_mappings
            .iter()
            .all(|mapping| mapping.source_keypoint_names.is_empty())
    );
    assert!(harness.state().import_flow.error.is_none());
}

#[cfg(feature = "inspector-presets")]
#[test]
fn import_progress_overview_exposes_stage_and_activity_status() {
    use crate::inspector_presets::{self, InspectorPreset};

    for (preset, expected_stage) in [
        (
            InspectorPreset::ImportSource,
            "Step 1 of 5: Source, current",
        ),
        (
            InspectorPreset::ImportMultipleDescriptors,
            "Step 2 of 5: Configure, current",
        ),
        (
            InspectorPreset::ImportPreflight,
            "Step 3 of 5: Preflight, current",
        ),
        (InspectorPreset::ImportReady, "Step 4 of 5: Ready, current"),
        (
            InspectorPreset::ImportRunning,
            "Step 5 of 5: Import, current",
        ),
        (
            InspectorPreset::ImportFailure,
            "Step 3 of 5: Preflight, failed",
        ),
        (
            InspectorPreset::ImportSuccess,
            "Step 5 of 5: Import, complete",
        ),
    ] {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(390.0, 667.0))
            .build_eframe(|ctx| inspector_presets::build(preset, &ctx.egui_ctx));
        harness.step();

        assert!(harness.query_by_label("Import progress").is_some());
        assert!(
            harness.query_by_label(expected_stage).is_some(),
            "missing stage status {expected_stage:?} for {preset:?}"
        );
        assert!(
            harness
                .query_by_label("Step 1 of 5: Source, complete")
                .is_some()
                || preset == InspectorPreset::ImportSource
        );
        assert_visible_controls_clamped(&harness, 390.0, 667.0);
    }

    let mut running = Harness::builder()
        .with_size(egui::vec2(1288.0, 820.0))
        .build_eframe(|ctx| {
            inspector_presets::build(InspectorPreset::ImportRunning, &ctx.egui_ctx)
        });
    running.step();
    assert!(
        running
            .query_by_label("Building and publishing dataset")
            .is_some()
    );
    assert!(
        running
            .query_all_by_role_and_label(
                egui::accesskit::Role::ProgressIndicator,
                "Building dataset: 482 of 1020 records processed",
            )
            .next()
            .is_some()
    );
}

#[cfg(feature = "inspector-presets")]
#[test]
fn import_mapping_feedback_is_immediate_and_ready_tracks_the_exact_draft() {
    use crate::inspector_presets::{self, InspectorPreset};

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1180.0, 1200.0))
        .build_eframe(|ctx| inspector_presets::build(InspectorPreset::ImportReady, &ctx.egui_ctx));
    harness.step();

    assert!(
        !harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Commit import")
            .accesskit_node()
            .is_disabled()
    );
    assert!(harness.query_by_label("COCO crowd objects").is_some());
    assert!(harness.query_by_label("YOLO missing labels").is_none());

    harness.state_mut().import_flow.categories[0].class_id = "bad/class".to_string();
    harness.step();

    assert!(
        harness
            .query_by_label("Class ID must be a non-empty safe path segment of at most 255 bytes.")
            .is_some()
    );
    assert!(
        harness
            .query_by_label("Step 3 of 5: Preflight, current")
            .is_some()
    );
    assert!(
        harness
            .query_by_label(
                "Last accepted preflight — current edits are not included. Save the corrected mappings to refresh diagnostics and readiness."
            )
            .is_some()
    );
    assert!(
        harness
            .get_by_role_and_label(
                egui::accesskit::Role::Button,
                "Save mappings and re-run preflight"
            )
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Commit import")
            .accesskit_node()
            .is_disabled()
    );

    harness.state_mut().import_flow.categories[0].class_id = "person".to_string();
    harness.step();

    assert!(
        harness
            .query_by_label("Step 4 of 5: Ready, current")
            .is_some()
    );
    assert!(
        harness
            .query_by_label("Class ID must be a non-empty safe path segment of at most 255 bytes.")
            .is_none()
    );
    assert!(
        !harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Commit import")
            .accesskit_node()
            .is_disabled()
    );

    harness.state_mut().import_flow.geometry_bounds = labello_client::GeometryBoundsPolicy::Clip;
    harness.step();

    assert!(
        harness
            .query_by_label(
                "Out-of-bounds geometry will be clipped as derived pending data and requires acknowledgement."
            )
            .is_some()
    );
    assert!(
        harness
            .query_by_label("Step 3 of 5: Preflight, current")
            .is_some()
    );
    assert!(
        !harness
            .get_by_role_and_label(
                egui::accesskit::Role::Button,
                "Save mappings and re-run preflight"
            )
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Commit import")
            .accesskit_node()
            .is_disabled()
    );
}

#[cfg(feature = "inspector-presets")]
#[test]
fn server_source_pickers_commit_folder_and_opaque_file_selections() {
    use crate::inspector_presets::{self, InspectorPreset};

    let mut folder = Harness::builder()
        .with_size(egui::vec2(900.0, 800.0))
        .build_eframe(|ctx| {
            inspector_presets::build(InspectorPreset::ImportServerFolderPicker, &ctx.egui_ctx)
        });
    folder.step();
    assert!(folder.query_by_label("Relative source path").is_none());
    click_accesskit_button(&mut folder, "Select folder release-2026");
    folder.step();
    assert_eq!(
        folder.state().import_flow.server_relative_path,
        "release-2026"
    );
    assert!(folder.state().import_flow.source_picker.target.is_none());

    let mut failed_folder = Harness::builder()
        .with_size(egui::vec2(390.0, 844.0))
        .build_eframe(|ctx| {
            inspector_presets::build(InspectorPreset::ImportServerFolderPicker, &ctx.egui_ctx)
        });
    {
        let picker = &mut failed_folder.state_mut().import_flow.source_picker;
        picker.relative_path = "release-2026/nested".to_string();
        picker.page = None;
        picker.error = Some("This folder could not be listed.".to_string());
    }
    failed_folder.step();
    click_accesskit_button(&mut failed_folder, "Select this folder");
    failed_folder.step();
    assert_eq!(
        failed_folder.state().import_flow.server_relative_path,
        "release-2026/nested"
    );

    let mut descriptor = Harness::builder()
        .with_size(egui::vec2(900.0, 800.0))
        .build_eframe(|ctx| {
            inspector_presets::build(InspectorPreset::ImportServerDescriptorPicker, &ctx.egui_ctx)
        });
    descriptor.step();
    click_accesskit_button(&mut descriptor, "Select dataset.yaml");
    descriptor.step();
    assert_eq!(
        descriptor.state().import_flow.descriptors[0].descriptor_file_id,
        "file-yaml"
    );
    assert!(
        descriptor
            .state()
            .import_flow
            .registered_paths
            .iter()
            .any(|path| path.file_id == "file-yaml"
                && path.relative_path == "release-2026/dataset.yaml")
    );
    assert!(
        descriptor
            .state()
            .import_flow
            .source_picker
            .target
            .is_none()
    );
}
