use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};
#[cfg(not(target_arch = "wasm32"))]
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

use eframe::egui;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use labello_client::{
    AdjudicationApi, AnnotationApi, AnnotationBatchRequest, ApiFuture, AppendEventRequest,
    AssignNextRequest, AssignmentActionRequest, AuthApi, AuthOptions, ClientError, ClientResult,
    CorrectionRequest, CreateDatasetRequest, DatasetApi, DatasetSummary, DatasetUser, ImageApi,
    ImageExplorerQuery, ImageFile, ImagePreview, ImportApi, IngestJob, IngestJobStatus,
    IngestReport, KeybindingApi, OAuthCallbackRequest, OAuthLoginRequest, OfflineApi,
    OfflineBundleRequest, PrelabelApi, PrelabelSuggestionRequest, ReviewApi, SessionInfo,
    SetDatasetRolesRequest, SnapshotFile, StatsApi, TaskApi, UpdateDatasetConfigRequest, UserApi,
};
use labello_domain::{
    AdjudicationRecord, AnnotationGeometry, AnnotationOrigin, AnnotationType, Assignment,
    AssignmentId, AssignmentKind, AssignmentStatus, BoundingBox, BrowserAcceleration, ClassId,
    DatasetId, DatasetMetadata, DatasetRole, DatasetRoleAssignment, DatasetSnapshot, DatasetStats,
    EventId, EventLogEntry, EventPayload, HumanRevisionKind, ImageExplorerItem, ImageExplorerPage,
    ImageId, ImageRecord, ImageState, ImportId, KeybindingSet, KeypointAnnotation, KeypointSpec,
    KeypointState, LabelClass, MigrationDispositionStatus, MigrationExclusion, ModelSpec,
    NormalizedPoint, OfflineBundle, OfflineSyncRequest, OfflineSyncResult, OutputProcessing,
    PrelabelConfig, PrelabelConfigId, PrelabelExecution, PrelabelSuggestion, ReviewConfig,
    ReviewId, ReviewRecord, ReviewTarget, RevisionSource, SCHEMA_VERSION, SkeletonGeometry,
    SkeletonSpec, SnapshotFileEntry, TaskDefinition, TaskId, TaskStatus, TutorialContent,
    UserAccount, UserId,
};
use web_time::{Duration, Instant};

mod support;

use support::*;

use crate::app::{
    AdminSection, AppConfig, AppView, Drawer, FolderUploadProgress, IMAGE_QUEUE_SIZE, LabelloApp,
    LayoutMode, RequestIdentity, SaveStatus, SetupSection, UiCommand, UiMessage,
};
use crate::canvas::BoundingBoxEdit;
use crate::persistence::{StoredCanvasTransform, StoredView, WorkspacePreference};
use crate::theme;

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_task_spawner_delivers_live_messages() {
    let mut app = LabelloApp::default();
    let scheduled = Rc::new(RefCell::new(None));
    let scheduled_for_spawner = scheduled.clone();
    app.set_native_task_spawner(move |future| {
        *scheduled_for_spawner.borrow_mut() = Some(future);
    });
    let request = RequestIdentity {
        auth_epoch: 0,
        workspace_epoch: 0,
        request_id: 1,
        dataset_id: None,
    };

    app.spawn_message(request.clone(), async move {
        UiMessage::RequestFailed {
            request,
            error: "scheduled".to_string(),
        }
    });

    let task = scheduled
        .borrow_mut()
        .take()
        .expect("native task was not scheduled");
    poll_ready_task(task);
    let message = app.runtime.rx.try_recv().unwrap();
    assert!(matches!(
        message,
        UiMessage::RequestFailed { error, .. } if error == "scheduled"
    ));
}

#[cfg(not(target_arch = "wasm32"))]
fn poll_ready_task(mut future: Pin<Box<dyn Future<Output = ()> + 'static>>) {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    assert!(matches!(
        future.as_mut().poll(&mut context),
        Poll::Ready(())
    ));
}

#[test]
fn setup_create_open_and_admin_workflows_use_live_commands() {
    let api = Rc::new(SpyApi::new());
    let mut harness = live_harness(api.clone());
    step_until(&mut harness, 8, |app| app.datasets.summaries.len() == 1);

    assert!(harness.query_by_label("Choose where to work").is_some());
    assert_eq!(api.counts().me, 1);
    assert_eq!(api.counts().auth_options, 1);
    assert!(
        harness
            .query_all_by_label("Continue with Demo Dataset")
            .next()
            .is_some()
    );
    assert!(
        harness
            .query_all_by_label("Admin Demo Dataset")
            .next()
            .is_some()
    );

    harness.set_size(egui::vec2(1500.0, 1200.0));
    harness.step();
    select_setup_section(&mut harness, "Create");
    harness.state_mut().setup.create_dataset_id = "new-dataset".to_string();
    harness.state_mut().setup.create_dataset_name = "New dataset".to_string();
    harness.step();
    click(&mut harness, "Create dataset");
    step_until(&mut harness, 20, |app| app.loading.admin);
    assert!(harness.state().runtime.error.is_none());
    step_until(&mut harness, 20, |app| {
        app.view == AppView::Admin && !app.loading.admin
    });
    assert_eq!(api.counts().create_dataset, 1);
    assert_eq!(api.counts().get_admin_dataset, 1);
    assert!(harness.query_by_label("Dataset Admin").is_some());
    assert!(
        harness
            .state()
            .datasets
            .admin_config
            .as_ref()
            .is_some_and(|metadata| metadata.tasks.is_empty())
    );
}

#[test]
fn setup_sections_are_permission_gated_responsive_and_preserve_import_state() {
    let api = Rc::new(SpyApi::new());
    let mut harness = live_harness(api);
    step_until(&mut harness, 8, |app| app.datasets.summaries.len() == 1);

    harness.state_mut().auth.can_create_datasets = false;
    harness.step();
    for label in ["Create", "Import"] {
        assert!(harness.query_by_label(label).is_none());
    }

    harness.state_mut().auth.can_create_datasets = true;
    harness.step();
    for label in ["Datasets", "Connection", "Create", "Import"] {
        assert!(harness.query_by_label(label).is_some());
    }
    assert!(harness.query_by_label("Setup navigation").is_some());
    click_accesskit_button(&mut harness, "Import");
    assert_eq!(harness.state().setup.section, SetupSection::Import);
    assert!(harness.state().import_flow.open);

    harness.set_size(egui::vec2(900.0, 780.0));
    harness.step();
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::ComboBox, "Setup section")
            .is_some()
    );
    harness.state_mut().import_flow.destination_id = "active-import".to_string();
    harness
        .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Setup section")
        .click_accesskit();
    harness.step();
    click_accesskit_button(&mut harness, "Connection");
    assert_eq!(harness.state().setup.section, SetupSection::Connection);
    assert!(harness.state().import_flow.open);
    assert_eq!(harness.state().import_flow.destination_id, "active-import");

    harness.state_mut().setup.section = SetupSection::Import;
    harness.state_mut().auth.can_create_datasets = false;
    harness.step();
    assert_eq!(harness.state().setup.section, SetupSection::Datasets);
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
fn mutable_import_spy_accepts_api_valid_manual_approval_request() {
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
    job.preflight_report = Some(test_import_report());
    api.set_import_job(job.clone());
    {
        let flow = &mut harness.state_mut().import_flow;
        flow.open = true;
        flow.job = Some(job);
        flow.screen = crate::import_flow::ImportScreen::Preflight;
        flow.categories = vec![contract_import_category()];
        flow.geometry_policy = labello_client::ImportGeometryPolicy::ManualBoxGuideV1;
        flow.workflow_intent = labello_client::ImportWorkflowIntent::RequireApproval;
        flow.keypoint_names = "nose,left_eye".to_string();
    }

    harness.state_mut().request_update_import_plan();
    harness.step();
    step_until(&mut harness, 8, |app| {
        app.import_flow.plan.is_some() && !app.import_flow.busy
    });

    let request = api.last_import_plan_request().unwrap();
    assert_eq!(request.task_mappings.len(), 2);
    assert!(request.task_mappings.iter().all(|mapping| {
        mapping.task.review.workflow == labello_domain::ReviewWorkflow::Approval
            && mapping.task.review.required_reviews == 1
    }));
    assert!(
        request.skeleton_mappings[0]
            .source_keypoint_names
            .is_empty()
    );
    assert!(harness.state().import_flow.error.is_none());
}

#[cfg(feature = "inspector-presets")]
#[test]
fn import_and_migration_presets_are_accessible_at_desktop_mobile_and_short_sizes() {
    use crate::inspector_presets::{self, InspectorPreset};

    for (width, height) in [
        (1440.0, 900.0),
        (1288.0, 820.0),
        (390.0, 667.0),
        (390.0, 320.0),
    ] {
        let mut source = Harness::builder()
            .with_size(egui::vec2(width, height))
            .build_eframe(|ctx| {
                inspector_presets::build(InspectorPreset::ImportSource, &ctx.egui_ctx)
            });
        source.step();
        assert!(source.query_by_label("Import dataset").is_some());
        assert!(source.query_by_label("Import profile").is_some());
        assert_visible_controls_clamped(&source, width, height);

        let mut migration = Harness::builder()
            .with_size(egui::vec2(width, height))
            .build_eframe(|ctx| {
                let mut app =
                    inspector_presets::build(InspectorPreset::MigrationObject, &ctx.egui_ctx);
                if width < 600.0 {
                    app.drawer = Some(Drawer::Inspector);
                }
                app
            });
        migration.step();
        assert!(migration.query_by_label("Annotation canvas").is_some());
        if width >= 600.0 || height >= 667.0 {
            assert!(migration.query_by_label("Canonical guide").is_some());
            assert!(migration.query_by_label("Exclusion reason").is_some());
        }
        if width >= 600.0 {
            let canvas = migration.get_by_label("Annotation canvas").rect();
            let inspector = migration.get_by_label("Inspector").rect();
            assert!(
                canvas.right() <= inspector.left(),
                "canvas overlaps the inspector: canvas={canvas:?} inspector={inspector:?}",
            );
        }
        assert_visible_controls_clamped(&migration, width, height);
    }

    let mut full_image = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(|ctx| {
            inspector_presets::build(InspectorPreset::MigrationFullImage, &ctx.egui_ctx)
        });
    full_image.step();
    assert!(
        full_image
            .query_by_label("Full-image confirmation")
            .is_some()
    );
    assert!(
        full_image
            .query_by_label("Confirm all guides & finish")
            .is_some()
    );
    assert!(
        full_image
            .query_by_role(egui::accesskit::Role::CheckBox)
            .is_none()
    );
    assert!(full_image.query_by_label("Start correction pass").is_some());

    let mut no_guides_app = inspector_presets::build(
        InspectorPreset::MigrationFullImage,
        &egui::Context::default(),
    );
    let task_id = no_guides_app.selected_task_id.clone().unwrap();
    let state = no_guides_app.current_state.as_mut().unwrap();
    state
        .migration_target_sets
        .get_mut(&task_id)
        .unwrap()
        .targets
        .clear();
    state
        .migration_dispositions
        .get_mut(&task_id)
        .unwrap()
        .clear();
    no_guides_app.migration.cursor = Some(labello_domain::MigrationCursor::FullImage);
    no_guides_app.migration.progress = None;
    let no_guides_api = Rc::new(SpyApi::new());
    no_guides_api.set_image_state(no_guides_app.current_state.clone().unwrap());
    let mut no_guides = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_max_steps(40)
        .build_eframe(|_| no_guides_app);
    no_guides.step();
    assert!(
        no_guides
            .query_by_label("Confirm no guides & finish")
            .is_some()
    );
    assert!(
        no_guides
            .query_by_role(egui::accesskit::Role::CheckBox)
            .is_none()
    );
    assert!(
        no_guides
            .query_by_label(
                "Confirm that this image has no canonical guides and needs no skeletons."
            )
            .is_some()
    );
    no_guides.state_mut().runtime.api = Some(no_guides_api.clone());
    click_accesskit_button(&mut no_guides, "Confirm no guides & finish");
    step_until(&mut no_guides, 8, |app| !app.migration.busy);
    assert_eq!(no_guides_api.counts().migration_commands, 1);

    let mut deleted = Harness::builder()
        .with_size(egui::vec2(390.0, 667.0))
        .build_eframe(|ctx| {
            let mut app =
                inspector_presets::build(InspectorPreset::MigrationGuideDeleted, &ctx.egui_ctx);
            app.drawer = Some(Drawer::Inspector);
            app
        });
    deleted.step();
    assert!(
        deleted
            .query_by_label("Deleted guide tombstone | Status: Pending")
            .is_some()
    );
    assert!(deleted.query_by_label("Reload assignment state").is_some());
    assert_label_inside(
        &deleted,
        "The canonical guide is deleted or unavailable. Skeleton editing and keep/reopen actions are disabled; record an exclusion or reload after the guide is repaired.",
        390.0,
        667.0,
    );
    assert_visible_controls_clamped(&deleted, 390.0, 667.0);

    let mut partial = Harness::builder()
        .with_size(egui::vec2(1180.0, 2600.0))
        .build_eframe(|ctx| {
            inspector_presets::build(InspectorPreset::ImportPartialCategories, &ctx.egui_ctx)
        });
    partial.step();
    assert!(
        partial
            .get_by_role_and_label(
                egui::accesskit::Role::Button,
                "Save mappings and re-run preflight"
            )
            .accesskit_node()
            .is_disabled()
    );
    assert!(partial.query_by_label("Keypoint envelope").is_none());
    assert!(partial.query_by_label("Box-relative template").is_none());

    let mut descriptors = Harness::builder()
        .with_size(egui::vec2(1180.0, 1400.0))
        .build_eframe(|ctx| {
            inspector_presets::build(InspectorPreset::ImportMultipleDescriptors, &ctx.egui_ctx)
        });
    descriptors.step();
    assert_eq!(
        descriptors
            .query_all_by_role_and_label(egui::accesskit::Role::ComboBox, "Descriptor kind")
            .count(),
        2
    );

    let mut recovery = Harness::builder()
        .with_size(egui::vec2(800.0, 900.0))
        .build_eframe(|ctx| {
            inspector_presets::build(InspectorPreset::ImportRecoveryBlocked, &ctx.egui_ctx)
        });
    recovery.step();
    assert!(recovery.query_by_label("Restart import setup").is_some());
    assert!(
        recovery
            .query_by_label("This recovered job does not include its attestations, source descriptors, category identities/schema, or accepted mapping request in the current API contract. Unsafe continuation is disabled.")
            .is_some()
    );

    let mut annotated = Harness::builder()
        .with_size(egui::vec2(1440.0, 1000.0))
        .build_eframe(|ctx| {
            inspector_presets::build(InspectorPreset::MigrationAnnotatedEdit, &ctx.egui_ctx)
        });
    annotated.step();
    assert!(
        annotated
            .query_by_label("Redraw annotated skeleton")
            .is_some()
    );
    assert!(annotated.query_by_label("Reopen excluded target").is_none());
    assert!(
        annotated
            .query_by_label("Focus current box (guide v1)")
            .is_some()
    );
}

#[cfg(feature = "inspector-presets")]
#[test]
fn mutable_migration_spy_preserves_failure_and_durable_reload_progression() {
    use crate::inspector_presets::{self, InspectorPreset};

    let api = Rc::new(SpyApi::new());
    let mut app =
        inspector_presets::build(InspectorPreset::MigrationObject, &egui::Context::default());
    let image_id = app.current.as_ref().unwrap().image.image_id.clone();
    api.set_image_state(app.current_state.clone().unwrap());
    app.runtime.api = Some(api.clone());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 1000.0))
        .with_max_steps(40)
        .build_eframe(|_| app);
    harness.step();

    api.fail_next_migration();
    harness
        .state_mut()
        .request_exclude_migration_target(labello_domain::ObjectGroupId::from("group-left"));
    harness.step();
    step_until(&mut harness, 8, |app| !app.migration.busy);
    assert!(
        harness
            .state()
            .migration
            .error
            .as_deref()
            .is_some_and(|error| error.contains("migration command failed")),
        "counts={:?} migration_error={:?} runtime_error={:?}",
        api.counts(),
        harness.state().migration.error,
        harness.state().runtime.error,
    );
    assert!(matches!(
        harness.state().migration.cursor,
        Some(labello_domain::MigrationCursor::Object { ref object_group_id, .. })
            if object_group_id == &labello_domain::ObjectGroupId::from("group-left")
    ));

    harness
        .state_mut()
        .request_exclude_migration_target(labello_domain::ObjectGroupId::from("group-left"));
    harness.step();
    step_until(&mut harness, 8, |app| {
        matches!(
            app.migration.cursor,
            Some(labello_domain::MigrationCursor::Object { ref object_group_id, .. })
                if object_group_id == &labello_domain::ObjectGroupId::from("group-right")
        )
    });
    assert_eq!(api.counts().migration_commands, 2);

    let durable = api.image_state(&image_id);
    let mut reloaded =
        inspector_presets::build(InspectorPreset::MigrationObject, &egui::Context::default());
    reloaded.current_state = Some(durable.clone());
    reloaded.annotations = durable.active_annotations().cloned().collect();
    reloaded.migration = Default::default();
    let mut reload_harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 1000.0))
        .build_eframe(|_| reloaded);
    reload_harness.step();
    assert!(matches!(
        reload_harness.state().migration.cursor,
        Some(labello_domain::MigrationCursor::Object { ref object_group_id, .. })
            if object_group_id == &labello_domain::ObjectGroupId::from("group-right")
    ));
}

#[cfg(feature = "inspector-presets")]
#[test]
fn migration_draft_supports_undo_and_delete() {
    use crate::inspector_presets::{self, InspectorPreset};

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 1000.0))
        .build_eframe(|ctx| {
            inspector_presets::build(InspectorPreset::MigrationObject, &ctx.egui_ctx)
        });
    harness.step();

    let place_first_keypoint = |app: &mut LabelloApp| {
        let draft = app.migration.draft.as_mut().unwrap();
        draft.keypoints[0].point = Some(labello_domain::NormalizedPoint { x: 0.5, y: 0.5 });
        draft.keypoints[0].state = labello_domain::KeypointState::Visible;
        app.migration.keypoint_index = 1;
    };

    let task_id = harness.state().selected_task_id.clone().unwrap();
    let guide_id = harness
        .state()
        .current_state
        .as_ref()
        .unwrap()
        .migration_target_sets[&task_id]
        .targets[0]
        .guide_annotation_id
        .clone();
    let guide_before = harness
        .state()
        .current_state
        .as_ref()
        .unwrap()
        .current_annotation(&guide_id)
        .unwrap()
        .clone();

    place_first_keypoint(harness.state_mut());
    harness.state_mut().migration.next_hidden = true;
    harness.step();
    click_accesskit_button(&mut harness, "Undo last keypoint");
    assert_eq!(harness.state().migration.keypoint_index, 0);
    assert!(
        harness.state().migration.draft.as_ref().unwrap().keypoints[0]
            .point
            .is_none()
    );
    assert!(!harness.state().migration.next_hidden);
    assert_eq!(
        harness
            .state()
            .current_state
            .as_ref()
            .unwrap()
            .current_annotation(&guide_id),
        Some(&guide_before)
    );

    place_first_keypoint(harness.state_mut());
    harness.step();
    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::Z);
    harness.step();
    assert_eq!(harness.state().migration.keypoint_index, 0);
    assert!(
        harness.state().migration.draft.as_ref().unwrap().keypoints[0]
            .point
            .is_none()
    );

    place_first_keypoint(harness.state_mut());
    harness.step();
    harness.key_press(egui::Key::Delete);
    harness.step();
    assert_eq!(harness.state().migration.keypoint_index, 0);
    assert!(
        harness.state().migration.draft.as_ref().unwrap().keypoints[0]
            .point
            .is_none()
    );

    place_first_keypoint(harness.state_mut());
    harness
        .state_mut()
        .current_state
        .as_mut()
        .unwrap()
        .annotations
        .get_mut(&guide_id)
        .unwrap()
        .last_mut()
        .unwrap()
        .deleted = true;
    harness.step();
    assert!(
        harness
            .query_all_by_label_contains("Undo last keypoint")
            .next()
            .unwrap()
            .accesskit_node()
            .is_disabled()
    );
    harness.key_press(egui::Key::Delete);
    harness.step();
    assert_eq!(harness.state().migration.keypoint_index, 1);
    assert!(
        harness.state().migration.draft.as_ref().unwrap().keypoints[0]
            .point
            .is_some()
    );
}

#[cfg(feature = "inspector-presets")]
#[test]
fn migration_confirmation_button_dispatches_once() {
    use crate::inspector_presets::{self, InspectorPreset};

    let api = Rc::new(SpyApi::new());
    let mut app = inspector_presets::build(
        InspectorPreset::MigrationFullImage,
        &egui::Context::default(),
    );
    api.set_image_state(app.current_state.clone().unwrap());
    app.runtime.api = Some(api.clone());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_max_steps(40)
        .build_eframe(|_| app);
    harness.step();

    click_accesskit_button(&mut harness, "Confirm all guides & finish");
    step_until(&mut harness, 8, |app| !app.migration.busy);
    assert_eq!(api.counts().migration_commands, 1);
    assert!(
        harness
            .query_by_label("Confirm all guides & finish")
            .is_none()
    );
    harness.step();
    assert_eq!(api.counts().migration_commands, 1);
}

#[cfg(feature = "inspector-presets")]
#[test]
fn assignment_reload_discards_stale_manual_cursor_pass_and_local_draft() {
    use crate::app::LoadedImage;
    use crate::inspector_presets::{self, InspectorPreset};

    let mut app =
        inspector_presets::build(InspectorPreset::MigrationObject, &egui::Context::default());
    let loaded = LoadedImage {
        assignment: app.assignment.clone().unwrap(),
        queued: app.current.clone().unwrap(),
        annotations: app.annotations.clone(),
        state: app.current_state.clone().unwrap(),
        color_image: None,
    };
    app.migration.cursor = Some(labello_domain::MigrationCursor::FullImage);
    app.migration.active_pass_id = Some(labello_domain::MigrationPassId::from("stale-pass"));
    app.migration.draft =
        Some(crate::manual_migration::ManualMigrationState::empty_skeleton(["stale".to_string()]));
    app.migration.draft_group = Some(labello_domain::ObjectGroupId::from("stale-group"));
    app.migration.error = Some("stale failure".to_string());
    let operation_id = 77_001;
    let request = test_request(&app, operation_id, Some("demo"));
    app.active_load_id = Some(operation_id);
    app.runtime.active_requests.insert(operation_id);
    app.runtime
        .tx
        .send(UiMessage::ImageLoaded {
            request,
            operation_id,
            assignment: Some(loaded.assignment.clone()),
            result: Box::new(Ok(Some(loaded))),
        })
        .unwrap();

    app.process_messages(&egui::Context::default());

    assert!(app.migration.cursor.is_none());
    assert!(app.migration.active_pass_id.is_none());
    assert!(app.migration.draft.is_none());
    assert!(app.migration.draft_group.is_none());
    assert!(app.migration.error.is_none());
    app.sync_manual_migration();
    assert!(matches!(
        app.migration.cursor,
        Some(labello_domain::MigrationCursor::Object { ref object_group_id, .. })
            if object_group_id == &labello_domain::ObjectGroupId::from("group-left")
    ));
}

#[test]
fn admin_workflow_saves_ingests_and_handles_browser_only_folder_upload() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_admin_harness(api.clone());
    harness.set_size(egui::vec2(1180.0, 4000.0));
    harness.step();

    assert!(harness.query_by_label("Dataset Admin").is_some());
    select_admin_section(&mut harness, "Images");
    click(&mut harness, "Pick folder and upload");
    harness.step();
    assert!(!harness.state().loading.uploading);
    assert!(
        harness
            .state()
            .runtime
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("browser build")
    );
    assert!(
        harness
            .state()
            .admin_tools
            .upload_error
            .as_deref()
            .is_some_and(|error| error.contains("browser build"))
    );

    click_accesskit_button(&mut harness, "Add image root");
    harness.step();
    select_admin_section(&mut harness, "Schema");
    click_accesskit_button(&mut harness, "Add bounding box class workflow");
    harness.step();
    select_admin_section(&mut harness, "Automation");
    click_accesskit_button(&mut harness, "Add browser prelabel config");
    harness.step();
    let config = harness.state().datasets.admin_config.as_ref().unwrap();
    assert_eq!(config.image_roots.len(), 2);
    assert_eq!(config.label_classes.len(), 3);
    assert_eq!(config.prelabel_configs.len(), 2);
    assert_eq!(config.role_assignments.len(), 1);
    assert_eq!(config.tasks.len(), 3);
    assert!(config.tasks.iter().any(|task| {
        task.annotation_type == AnnotationType::BoundingBox
            && task.class_ids == vec![ClassId::from("object")]
    }));

    let before_save = api.counts();
    click_accesskit_button(&mut harness, "Save Admin changes");
    step_until(&mut harness, 8, |_| api.counts().update_dataset_config == 1);
    assert_eq!(api.counts().update_dataset_config, 1);
    assert_eq!(api.counts().list_datasets, before_save.list_datasets);
    assert!(api.metadata().label_classes.len() >= 3);

    click_application_menu_item(&mut harness, "Admin");
    step_until(&mut harness, 8, |app| app.view == AppView::Admin);
    let before_ingest = api.counts();
    harness.state_mut().request_ingest();
    harness.step();
    let badge = harness.get_by_label("Dataset Demo Dataset");
    assert!(badge.rect().height() < 80.0);
    step_until(&mut harness, 16, |_| api.counts().ingest_dataset >= 1);
    assert_eq!(
        api.counts().ingest_dataset,
        before_ingest.ingest_dataset + 1
    );
    for _ in 0..8 {
        harness.step();
    }
    assert_eq!(api.counts().get_dataset, before_ingest.get_dataset);
    assert_eq!(api.counts().dataset_stats, before_ingest.dataset_stats);
    assert!(
        harness
            .state()
            .runtime
            .notice
            .as_deref()
            .unwrap_or_default()
            .contains("Ingest complete")
    );

    click_application_menu_item(&mut harness, "Annotate");
    step_until(&mut harness, 12, |app| app.current.is_some());
    assert_eq!(harness.state().view, AppView::Annotate);
}

#[test]
fn admin_image_explorer_pages_and_snapshots_use_async_api_commands() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_admin_harness(api.clone());
    harness.set_size(egui::vec2(1300.0, 2400.0));
    harness.step();
    step_until(&mut harness, 12, |app| {
        app.admin_tools.images.is_some() && app.admin_tools.snapshots_loaded
    });
    assert_eq!(api.counts().list_images, 1);
    assert_eq!(api.counts().list_snapshots, 1);
    assert!(harness.query_by_label("one.png").is_none());
    select_admin_section(&mut harness, "Images");
    assert!(harness.query_by_label("one.png").is_some());
    assert!(
        harness
            .query_all_by_role_and_label(egui::accesskit::Role::TextInput, "Search images")
            .next()
            .is_some()
    );
    for label in ["Status filter", "Workflow filter", "Class filter"] {
        assert!(
            harness
                .query_all_by_role_and_label(egui::accesskit::Role::ComboBox, label)
                .next()
                .is_some(),
            "missing accessible image {label}"
        );
    }
    assert!(harness.query_by_label("Workflow").is_some());
    assert!(harness.query_all_by_label("Pending 1").next().is_some());
    assert!(harness.query_all_by_label("person").next().is_some());

    harness.state_mut().admin_tools.image_query.page_size = 1;
    harness.state_mut().admin_tools.image_search = "png".to_string();
    harness.state_mut().admin_tools.image_status = Some(TaskStatus::Pending);
    harness.state_mut().admin_tools.image_task = Some(TaskId::from("bounding_box:person"));
    harness.state_mut().admin_tools.image_class = Some(ClassId::from("person"));
    harness.state_mut().request_images();
    step_until(&mut harness, 8, |app| {
        app.admin_tools
            .images
            .as_ref()
            .is_some_and(|page| page.page_size == 1)
    });
    assert_eq!(api.counts().list_images, 2);
    let query = api.last_image_query().unwrap();
    assert_eq!(query.search.as_deref(), Some("png"));
    assert_eq!(query.status, Some(TaskStatus::Pending));
    assert_eq!(query.task_id, Some(TaskId::from("bounding_box:person")));
    assert_eq!(query.class_id, Some(ClassId::from("person")));

    click(&mut harness, "Next images");
    assert_eq!(api.counts().list_images, 3);
    step_until(&mut harness, 8, |app| {
        app.admin_tools
            .images
            .as_ref()
            .is_some_and(|page| page.page == 2)
    });
    assert_eq!(api.last_image_query().unwrap().page, 2);

    select_admin_section(&mut harness, "Backups");
    click_accesskit_button(&mut harness, "Create snapshot");
    step_until(&mut harness, 8, |app| !app.loading.creating_snapshot);
    assert_eq!(api.counts().create_snapshot, 1);
    assert!(harness.query_by_label("snapshot-test").is_some());

    click_accesskit_button(&mut harness, "Show files");
    assert!(
        harness
            .query_all_by_role_and_label(
                egui::accesskit::Role::Button,
                "Download snapshot.json from snapshot snapshot-test"
            )
            .next()
            .is_some()
    );
    click_accesskit_button(&mut harness, "Download");
    step_until(&mut harness, 8, |app| app.loading.snapshot_file.is_none());
    assert_eq!(api.counts().get_snapshot_file, 1);
    assert!(
        harness
            .state()
            .admin_tools
            .snapshot_action_error
            .as_deref()
            .is_some_and(|error| error.contains("browser build"))
    );
    assert!(harness.state().runtime.error.is_none());
}

#[test]
fn admin_classes_and_workflows_use_compact_desktop_editors() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_admin_harness(api);
    harness.set_size(egui::vec2(1300.0, 8000.0));
    harness.step();
    select_admin_section(&mut harness, "Schema");

    let class_name_fields = harness
        .query_all_by_role_and_label(egui::accesskit::Role::TextInput, "Name")
        .collect::<Vec<_>>();
    let class_id_fields = harness
        .query_all_by_role_and_label(egui::accesskit::Role::TextInput, "ID")
        .collect::<Vec<_>>();
    let class_color_fields = harness
        .query_all_by_role_and_label(egui::accesskit::Role::TextInput, "Color")
        .collect::<Vec<_>>();
    let class_description_fields = harness
        .query_all_by_role_and_label(egui::accesskit::Role::MultilineTextInput, "Description")
        .collect::<Vec<_>>();
    let classes_card = harness.get_by_label("Classes card").rect();
    assert_eq!(class_name_fields.len(), 2);
    for index in 0..class_name_fields.len() {
        let unit = class_name_fields[index].rect().width();
        assert!(
            (class_id_fields[index].rect().width() - unit).abs() <= 2.0,
            "name={:?} id={:?}",
            class_name_fields[index].rect(),
            class_id_fields[index].rect()
        );
        assert!((class_color_fields[index].rect().width() - unit).abs() <= 2.0);
        assert!(class_name_fields[index].rect().height() >= 27.0);
        assert!(class_description_fields[index].rect().width() >= 2.9 * unit);
        assert!(
            class_description_fields[index].rect().right() >= classes_card.right() - 32.0,
            "class editor does not fill its card"
        );
        assert!(
            (class_description_fields[index].rect().height()
                - class_name_fields[index].rect().height())
            .abs()
                <= 2.0,
            "description={:?} name={:?}",
            class_description_fields[index].rect(),
            class_name_fields[index].rect()
        );
    }
    let person_workflow = "Person boxes | bounding_box | Person | Enabled";
    let vehicle_workflow = "Vehicle boxes | bounding_box | Vehicle | Enabled";
    let person = harness.get_by_label(person_workflow).rect();
    let vehicle = harness.get_by_label(vehicle_workflow).rect();
    assert!(vehicle.top() - person.top() <= 70.0);
    assert!(harness.query_by_label("Annotator instructions").is_none());

    click_accesskit_button(&mut harness, person_workflow);
    assert!(harness.query_by_label("Annotator instructions").is_some());
    assert!(
        harness
            .query_all_by_role_and_label(egui::accesskit::Role::TextInput, "Task ID")
            .next()
            .is_some()
    );
}

#[test]
fn session_is_restored_before_datasets_load_and_logout_clears_it() {
    let api = Rc::new(SpyApi::new());
    let mut harness = live_harness(api.clone());
    step_until(&mut harness, 8, |app| !app.datasets.summaries.is_empty());

    assert_eq!(api.counts().me, 1);
    assert_eq!(api.counts().list_datasets, 1);
    assert_eq!(
        harness
            .state()
            .auth
            .account
            .as_ref()
            .map(|account| account.user_id.clone()),
        Some(UserId::from("admin"))
    );
    harness.state_mut().drawer = Some(Drawer::Workflow);
    harness.state_mut().show_tutorial = true;
    click(&mut harness, "Sign out");
    step_until(&mut harness, 8, |app| app.auth.account.is_none());
    assert_eq!(api.counts().logout, 1);
    assert!(harness.state().datasets.summaries.is_empty());
    assert!(harness.state().drawer.is_none());
    assert!(!harness.state().show_tutorial);
    assert_eq!(harness.state().setup.section, SetupSection::Connection);
    assert!(harness.query_by_label("Sign in with GitHub").is_some());
}

#[test]
fn signed_out_setup_offers_advertised_login_methods_without_raw_credentials() {
    let api = Rc::new(SpyApi::new());
    api.fail_me();
    let mut app = base_live_app(api.clone());
    app.auth.checked = false;
    app.auth.options_checked = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 780.0))
        .with_max_steps(20)
        .build_eframe(|_| app);
    step_until(&mut harness, 8, |app| app.auth.checked);
    assert_eq!(harness.state().setup.section, SetupSection::Connection);
    assert!(harness.query_by_label("Datasets").is_none());
    assert!(harness.query_by_label("Sign in with GitHub").is_some());
    assert!(harness.query_by_label("Continue as local admin").is_some());
    assert!(harness.query_by_label("Dev token").is_none());
    assert!(harness.query_by_label("Development user ID").is_none());

    click(&mut harness, "Continue as local admin");
    step_until(&mut harness, 8, |app| app.auth.account.is_some());
    assert_eq!(api.counts().local_admin_login, 1);
}

#[test]
fn auth_options_failure_clears_state_from_the_previous_endpoint() {
    let api = Rc::new(SpyApi::new());
    let metadata = api.metadata();
    let mut app = LabelloApp::default();
    app.runtime.api = Some(api.clone());
    app.auth.account = Some(api.state.borrow().users[0].account.clone());
    app.datasets.summaries = vec![DatasetSummary {
        dataset_id: metadata.dataset_id.clone(),
        name: metadata.name.clone(),
        roles: vec![DatasetRole::DataAdmin],
        total_images: metadata.images.len(),
    }];
    app.datasets.metadata = Some(metadata.clone());
    app.datasets.admin_config = Some(metadata.clone());
    app.datasets.admin_baseline = Some(metadata);
    app.datasets.stats = stats(3);
    app.datasets.last_stats_completion = Some(Instant::now());
    app.datasets.stats_error = Some("old statistics error".to_string());
    app.runtime.notice = Some("Signed in as Previous User".to_string());
    app.view = AppView::Admin;
    app.request_auth_options();
    let request = app.runtime.commands.back().unwrap().request().clone();
    app.runtime
        .tx
        .send(UiMessage::AuthOptionsLoaded {
            request,
            result: Err("server unavailable".to_string()),
        })
        .unwrap();

    app.process_messages(&egui::Context::default());

    assert!(app.auth.checked);
    assert!(app.auth.account.is_none());
    assert!(app.datasets.summaries.is_empty());
    assert!(app.datasets.metadata.is_none());
    assert!(app.datasets.admin_config.is_none());
    assert_eq!(app.datasets.stats, DatasetStats::default());
    assert!(app.datasets.last_stats_completion.is_none());
    assert!(app.datasets.stats_error.is_none());
    assert!(app.runtime.notice.is_none());
    assert!(app.current.is_none());
    assert_eq!(app.view, AppView::Setup);
}

#[test]
fn replacement_session_request_ignores_the_stale_result() {
    let api = Rc::new(SpyApi::new());
    let account = api.state.borrow().users[0].account.clone();
    let mut app = base_live_app(api);
    app.auth.account = None;

    app.request_session();
    let stale_request = app.runtime.commands.back().unwrap().request().clone();
    app.request_session();
    let active_request = app.runtime.commands.back().unwrap().request().clone();
    assert_ne!(stale_request, active_request);

    app.runtime
        .tx
        .send(UiMessage::SessionLoaded {
            request: stale_request,
            result: Ok(SessionInfo {
                account: account.clone(),
                can_create_datasets: true,
                csrf_token: "stale-csrf-token".to_string(),
            }),
        })
        .unwrap();
    app.process_messages(&egui::Context::default());
    assert!(app.loading.session);
    assert!(app.auth.account.is_none());
    assert_eq!(
        app.auth.active_session_request_id,
        Some(active_request.request_id)
    );

    app.runtime
        .tx
        .send(UiMessage::SessionLoaded {
            request: active_request,
            result: Ok(SessionInfo {
                account: account.clone(),
                can_create_datasets: true,
                csrf_token: "active-csrf-token".to_string(),
            }),
        })
        .unwrap();
    app.process_messages(&egui::Context::default());
    assert!(!app.loading.session);
    assert_eq!(app.auth.account, Some(account));
}

#[test]
fn github_login_uses_the_application_url_and_same_tab() {
    let api = Rc::new(SpyApi::new());
    let mut app = base_live_app(api.clone());
    app.request_github_login();
    app.start_next_command();

    let ctx = egui::Context::default();
    let output = ctx.run_ui(egui::RawInput::default(), |ui| {
        app.process_messages(ui.ctx());
    });

    assert_eq!(
        api.last_oauth_return_to().as_deref(),
        Some("https://app.example.test/label?dataset=demo")
    );
    assert_eq!(
        output.platform_output.commands,
        vec![egui::OutputCommand::OpenUrl(egui::OpenUrl::same_tab(
            "https://example.invalid/login",
        ))]
    );
}

#[test]
fn admin_people_directory_saves_roles_and_protects_the_last_admin() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_admin_harness(api.clone());
    harness.set_size(egui::vec2(1300.0, 1800.0));
    harness.step();

    assert!(harness.query_by_label("People").is_some());
    select_admin_section(&mut harness, "People");
    assert!(harness.query_by_label("Reviewer Person").is_some());
    let role_bounds = ["Annotator", "Reviewer", "Adjudicator", "Data admin"].map(|role| {
        harness
            .get_by_role_and_label(
                egui::accesskit::Role::CheckBox,
                &format!("{role} role for Reviewer Person (reviewer)"),
            )
            .rect()
    });
    assert!(role_bounds.windows(2).all(|roles| {
        (roles[0].top() - roles[1].top()).abs() <= 1.0
            && (roles[0].bottom() - roles[1].bottom()).abs() <= 1.0
    }));
    let identity_bounds = ["Reviewer Person", "@review-person", "ID: reviewer"]
        .map(|label| harness.get_by_label(label).rect())
        .into_iter()
        .reduce(egui::Rect::union)
        .unwrap();
    assert!(
        (identity_bounds.center().y - role_bounds[0].center().y).abs() <= 2.0,
        "identity={identity_bounds:?} roles={:?}",
        role_bounds[0]
    );
    assert!(
        harness
            .query_all_by_role_and_label(
                egui::accesskit::Role::CheckBox,
                "Reviewer role for Reviewer Person (reviewer)"
            )
            .next()
            .is_some()
    );
    assert!(
        harness
            .query_all_by_role_and_label(
                egui::accesskit::Role::Button,
                "Save permissions for Reviewer Person (reviewer)"
            )
            .next()
            .is_none()
    );
    let reviewer_role_id = harness
        .get_by_role_and_label(
            egui::accesskit::Role::CheckBox,
            "Reviewer role for Reviewer Person (reviewer)",
        )
        .accesskit_node()
        .locate()
        .0;
    let reviewer = harness
        .state_mut()
        .datasets
        .users
        .iter_mut()
        .find(|user| user.account.user_id == UserId::from("reviewer"))
        .unwrap();
    reviewer.roles.push(DatasetRole::Reviewer);
    harness
        .state_mut()
        .datasets
        .users
        .iter_mut()
        .find(|user| user.account.user_id == UserId::from("admin"))
        .unwrap()
        .roles
        .retain(|role| role != &DatasetRole::Annotator);
    harness.step();
    let staged_reviewer_role_id = harness
        .get_by_role_and_label(
            egui::accesskit::Role::CheckBox,
            "Reviewer role for Reviewer Person (reviewer)",
        )
        .accesskit_node()
        .locate()
        .0;
    assert_eq!(staged_reviewer_role_id, reviewer_role_id);
    assert!(
        harness
            .query_by_label("Permission changes staged")
            .is_some()
    );
    harness.state_mut().open_view(AppView::Stats);
    assert_eq!(harness.state().view, AppView::Admin);
    assert!(
        harness
            .state()
            .runtime
            .error
            .as_deref()
            .is_some_and(|error| error.contains("before leaving Admin"))
    );
    assert!(harness.query_by_label("Discard staged changes").is_some());
    let dataset_id = harness.state().config.dataset_id.clone();
    harness
        .state_mut()
        .open_dataset(DatasetId::from("other"), AppView::Stats);
    assert_eq!(harness.state().config.dataset_id, dataset_id);
    assert!(
        harness
            .state()
            .runtime
            .error
            .as_deref()
            .is_some_and(|error| error.contains("before switching datasets"))
    );
    harness.state_mut().request_logout();
    assert!(!harness.state().loading.logout);
    assert!(
        harness
            .state()
            .runtime
            .error
            .as_deref()
            .is_some_and(|error| error.contains("before signing out"))
    );
    click_accesskit_button(&mut harness, "Save Admin changes");
    step_until(&mut harness, 8, |app| {
        !app.loading.admin
            && app.loading.roles_user.is_none()
            && app.admin_tools.pending_role_saves.is_empty()
    });
    assert!(
        harness
            .query_all_by_label("Admin changes saved")
            .next()
            .is_some()
    );
    assert_eq!(api.counts().update_dataset_config, 0);
    assert_eq!(api.counts().set_dataset_roles, 2);
    assert!(
        harness
            .state()
            .datasets
            .users
            .iter()
            .find(|user| user.account.user_id == UserId::from("reviewer"))
            .unwrap()
            .roles
            .contains(&DatasetRole::Reviewer)
    );
    assert_eq!(
        harness.state().datasets.users,
        harness.state().datasets.users_baseline
    );
    assert!(
        harness
            .state()
            .datasets
            .admin_config
            .as_ref()
            .unwrap()
            .role_assignments
            .iter()
            .any(|assignment| {
                assignment.user_id == UserId::from("reviewer")
                    && assignment.roles.contains(&DatasetRole::Reviewer)
            })
    );
    assert!(
        harness
            .get_by_role_and_label(
                egui::accesskit::Role::CheckBox,
                "Data admin role for Admin User (admin)"
            )
            .accesskit_node()
            .is_disabled()
    );
    assert!(!harness.state().admin_changes_dirty());

    let admin = harness
        .state()
        .datasets
        .users
        .iter()
        .find(|user| user.account.user_id == UserId::from("admin"))
        .unwrap();
    assert!(admin.roles.contains(&DatasetRole::DataAdmin));
    assert!(!admin.roles.contains(&DatasetRole::Annotator));
}

#[test]
fn failed_global_admin_save_preserves_config_and_permission_edits() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_admin_harness(api.clone());
    harness
        .state_mut()
        .datasets
        .admin_config
        .as_mut()
        .unwrap()
        .name = "Staged dataset name".to_string();
    harness
        .state_mut()
        .datasets
        .users
        .iter_mut()
        .find(|user| user.account.user_id == UserId::from("reviewer"))
        .unwrap()
        .roles
        .push(DatasetRole::Reviewer);
    api.fail_next_admin_save();
    harness.step();

    click_accesskit_button(&mut harness, "Save Admin changes");
    step_until(&mut harness, 8, |app| !app.loading.admin);

    assert_eq!(api.counts().update_dataset_config, 1);
    assert!(harness.state().loading.roles_user.is_none());
    assert!(harness.state().admin_tools.pending_role_saves.is_empty());
    assert!(harness.state().admin_changes_dirty());
    assert_eq!(
        harness.state().datasets.admin_config.as_ref().unwrap().name,
        "Staged dataset name"
    );
    assert!(
        harness
            .state()
            .datasets
            .users
            .iter()
            .find(|user| user.account.user_id == UserId::from("reviewer"))
            .unwrap()
            .roles
            .contains(&DatasetRole::Reviewer)
    );
    assert!(
        harness
            .state()
            .runtime
            .error
            .as_deref()
            .is_some_and(|error| error.contains("admin save failed"))
    );
}

#[test]
fn global_admin_save_sequences_configuration_and_permissions() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_admin_harness(api.clone());
    harness
        .state_mut()
        .datasets
        .admin_config
        .as_mut()
        .unwrap()
        .name = "Unified Admin save".to_string();
    harness
        .state_mut()
        .datasets
        .users
        .iter_mut()
        .find(|user| user.account.user_id == UserId::from("reviewer"))
        .unwrap()
        .roles
        .push(DatasetRole::Reviewer);
    harness.step();

    click_accesskit_button(&mut harness, "Save Admin changes");
    step_until(&mut harness, 12, |app| {
        !app.loading.admin
            && app.loading.roles_user.is_none()
            && app.admin_tools.pending_role_saves.is_empty()
    });

    assert_eq!(api.counts().update_dataset_config, 1);
    assert_eq!(api.counts().set_dataset_roles, 1);
    assert_eq!(api.metadata().name, "Unified Admin save");
    assert!(
        api.dataset_users()
            .iter()
            .find(|user| user.account.user_id == UserId::from("reviewer"))
            .unwrap()
            .roles
            .contains(&DatasetRole::Reviewer)
    );
    assert!(!harness.state().admin_changes_dirty());
}

#[test]
fn failed_permission_sequence_keeps_remaining_edits_staged() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_admin_harness(api.clone());
    harness
        .state_mut()
        .datasets
        .users
        .iter_mut()
        .find(|user| user.account.user_id == UserId::from("admin"))
        .unwrap()
        .roles
        .retain(|role| role != &DatasetRole::Annotator);
    harness
        .state_mut()
        .datasets
        .users
        .iter_mut()
        .find(|user| user.account.user_id == UserId::from("reviewer"))
        .unwrap()
        .roles
        .push(DatasetRole::Reviewer);
    api.fail_role_save_at(2);
    harness.step();

    click_accesskit_button(&mut harness, "Save Admin changes");
    step_until(&mut harness, 12, |app| {
        app.loading.roles_user.is_none() && app.admin_tools.pending_role_saves.is_empty()
    });

    assert_eq!(api.counts().set_dataset_roles, 2);
    let users = &harness.state().datasets.users;
    let baseline = &harness.state().datasets.users_baseline;
    let admin = users
        .iter()
        .find(|user| user.account.user_id == UserId::from("admin"))
        .unwrap();
    let saved_admin = baseline
        .iter()
        .find(|user| user.account.user_id == UserId::from("admin"))
        .unwrap();
    assert!(!admin.roles.contains(&DatasetRole::Annotator));
    assert_eq!(admin.roles, saved_admin.roles);
    let reviewer = users
        .iter()
        .find(|user| user.account.user_id == UserId::from("reviewer"))
        .unwrap();
    let saved_reviewer = baseline
        .iter()
        .find(|user| user.account.user_id == UserId::from("reviewer"))
        .unwrap();
    assert!(reviewer.roles.contains(&DatasetRole::Reviewer));
    assert!(!saved_reviewer.roles.contains(&DatasetRole::Reviewer));
    assert!(harness.state().admin_changes_dirty());
    assert!(
        harness
            .state()
            .runtime
            .error
            .as_deref()
            .is_some_and(|error| error.contains("role save failed"))
    );
}

#[test]
fn admin_staged_changes_can_be_discarded_without_a_server_reload() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_admin_harness(api.clone());
    let original_name = harness
        .state()
        .datasets
        .admin_config
        .as_ref()
        .unwrap()
        .name
        .clone();
    harness
        .state_mut()
        .datasets
        .admin_config
        .as_mut()
        .unwrap()
        .name = "Unsaved rename".to_string();
    harness.step();

    select_admin_section(&mut harness, "Schema");
    assert_eq!(harness.state().admin_tools.section, AdminSection::Schema);
    select_admin_section(&mut harness, "Overview");
    assert_eq!(
        harness.state().datasets.admin_config.as_ref().unwrap().name,
        "Unsaved rename"
    );

    click(&mut harness, "Discard staged changes");
    assert!(
        harness
            .query_by_label("Discard staged Admin changes?")
            .is_some()
    );
    assert_eq!(
        harness.state().datasets.admin_config.as_ref().unwrap().name,
        "Unsaved rename"
    );
    click_accesskit_button(&mut harness, "Discard changes");
    assert_eq!(
        harness.state().datasets.admin_config.as_ref().unwrap().name,
        original_name
    );
    assert_eq!(api.counts().get_admin_dataset, 1);
}

#[test]
fn admin_permission_changes_can_be_discarded() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_admin_harness(api.clone());
    let baseline = harness.state().datasets.users_baseline.clone();
    harness
        .state_mut()
        .datasets
        .users
        .iter_mut()
        .find(|user| user.account.user_id == UserId::from("reviewer"))
        .unwrap()
        .roles
        .push(DatasetRole::Reviewer);
    harness.step();

    click(&mut harness, "Discard staged changes");
    harness.step();
    assert!(
        harness
            .query_by_label("All unsaved configuration and permission edits will be lost.")
            .is_some()
    );
    click_accesskit_button(&mut harness, "Discard changes");

    assert_eq!(harness.state().datasets.users, baseline);
    assert!(!harness.state().admin_changes_dirty());
    assert_eq!(api.counts().get_admin_dataset, 1);
}

#[test]
fn admin_destructive_edits_require_confirmation() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_admin_harness(api);
    harness.set_size(egui::vec2(1300.0, 4000.0));
    harness.step();
    select_admin_section(&mut harness, "Schema");
    let classes = harness
        .state()
        .datasets
        .admin_config
        .as_ref()
        .unwrap()
        .label_classes
        .len();

    click_accesskit_button(&mut harness, "Remove class");
    assert!(
        harness
            .query_all_by_label("Confirm removal")
            .next()
            .is_some()
    );
    assert!(
        harness
            .query_all_by_label_contains("Remove class 'Person'")
            .next()
            .is_some()
    );
    assert_eq!(
        harness
            .state()
            .datasets
            .admin_config
            .as_ref()
            .unwrap()
            .label_classes
            .len(),
        classes
    );
    click_accesskit_button(&mut harness, "Cancel");
    assert_eq!(
        harness
            .state()
            .datasets
            .admin_config
            .as_ref()
            .unwrap()
            .label_classes
            .len(),
        classes
    );

    click_accesskit_button(&mut harness, "Remove class");
    click_accesskit_button(&mut harness, "Confirm removal");
    assert_eq!(
        harness
            .state()
            .datasets
            .admin_config
            .as_ref()
            .unwrap()
            .label_classes
            .len(),
        classes - 1
    );
}

#[test]
fn admin_navigation_and_remote_states_are_responsive_and_explicit() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_admin_harness(api);
    step_until(&mut harness, 12, |app| {
        app.admin_tools.images.is_some() && app.admin_tools.snapshots_loaded
    });

    harness.set_size(egui::vec2(1440.0, 1000.0));
    harness.step();
    for section in [
        "Overview",
        "People",
        "Images",
        "Schema",
        "Automation",
        "Backups",
    ] {
        assert!(
            harness
                .query_all_by_role_and_label(egui::accesskit::Role::Button, section)
                .next()
                .is_some(),
            "missing wide Admin destination {section}"
        );
    }
    let title = harness.get_by_label("Dataset Admin").rect();
    let status = harness.get_by_label("Admin changes saved").rect();
    let reload = harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Reload")
        .rect();
    assert!(status.left() > title.right());
    assert!(reload.left() > title.right());
    assert!(
        status.right().max(reload.right()) >= 1250.0,
        "status={status:?}, reload={reload:?}"
    );

    harness.state_mut().admin_tools.section = AdminSection::Overview;
    harness.step();
    let unscrolled_admin_x = harness.get_by_label("Dataset Admin").rect().left();
    harness.state_mut().admin_tools.section = AdminSection::Schema;
    harness.step();
    for label in [
        "Class Workflows card",
        "Classes card",
        "Labeling Workflows card",
    ] {
        let card = harness.get_by_label(label).rect();
        assert!(card.width() >= 900.0, "{label} was only {card:?}");
        assert!(card.right() >= 1250.0, "{label} was only {card:?}");
    }
    let scrolled_admin_x = harness.get_by_label("Dataset Admin").rect().left();
    assert!((unscrolled_admin_x - scrolled_admin_x).abs() <= 0.5);
    harness.state_mut().admin_tools.section = AdminSection::Automation;
    harness.step();
    let prelabels_card = harness.get_by_label("Prelabels card").rect();
    for label in ["Prelabels card", "Assignment Balance card"] {
        let card = harness.get_by_label(label).rect();
        assert!(card.width() >= 900.0, "{label} was only {card:?}");
        assert!(card.right() >= 1250.0, "{label} was only {card:?}");
    }
    for label in ["Name", "Model name", "Location"] {
        let field = harness
            .get_by_role_and_label(egui::accesskit::Role::TextInput, label)
            .rect();
        assert!(
            field.right() >= prelabels_card.right() - 48.0,
            "{label} does not fill its Automation column: field={field:?}, card={prelabels_card:?}"
        );
    }
    harness.state_mut().admin_tools.section = AdminSection::Overview;
    harness.step();
    harness
        .state_mut()
        .datasets
        .admin_config
        .as_mut()
        .unwrap()
        .images
        .clear();
    harness
        .state_mut()
        .datasets
        .admin_baseline
        .as_mut()
        .unwrap()
        .images
        .clear();
    harness.step();
    click_accesskit_button(&mut harness, "Explore images");
    assert_eq!(harness.state().admin_tools.section, AdminSection::Images);
    harness.state_mut().admin_tools.section = AdminSection::Overview;
    harness.step();

    harness.state_mut().loading.admin = true;
    harness.step();
    assert!(
        harness
            .query_by_label("Saving or refreshing Admin changes")
            .is_some()
    );
    assert!(
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Reload")
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        harness
            .get_by_role_and_label(egui::accesskit::Role::TextInput, "Dataset name")
            .accesskit_node()
            .is_disabled()
    );
    harness.state_mut().loading.admin = false;

    harness.state_mut().admin_tools.section = AdminSection::People;
    harness.state_mut().loading.uploading = true;
    harness.step();
    assert!(
        harness
            .get_by_role_and_label(
                egui::accesskit::Role::CheckBox,
                "Annotator role for Admin User (admin)"
            )
            .accesskit_node()
            .is_disabled()
    );
    harness.state_mut().loading.uploading = false;

    harness.state_mut().admin_tools.section = AdminSection::Images;
    harness.state_mut().loading.images = true;
    harness.step();
    assert!(harness.query_by_label("Refreshing images...").is_some());
    harness.state_mut().loading.images = false;
    harness.state_mut().admin_tools.images_error = Some("offline".to_string());
    harness.step();
    assert!(
        harness
            .query_by_label("Showing saved image results. Refresh failed: offline")
            .is_some()
    );
    let mut empty_page = harness.state().admin_tools.images.clone().unwrap();
    empty_page.items.clear();
    harness.state_mut().admin_tools.images = Some(empty_page);
    harness.state_mut().admin_tools.images_error = None;
    harness.state_mut().loading.images = true;
    harness.step();
    assert!(harness.query_by_label("Refreshing images...").is_some());
    assert!(harness.query_by_label("No matching images").is_none());
    harness.state_mut().loading.images = false;

    harness.state_mut().admin_tools.section = AdminSection::Backups;
    harness.state_mut().loading.snapshots = true;
    harness.step();
    assert!(harness.query_by_label("Refreshing backups...").is_some());
    harness.state_mut().loading.snapshots = false;
    harness.state_mut().admin_tools.snapshots_error = Some("offline".to_string());
    harness.step();
    assert!(
        harness
            .query_by_label("Showing the last loaded backups. Refresh failed: offline")
            .is_some()
    );
    harness.state_mut().admin_tools.snapshots_loaded = false;
    harness.state_mut().admin_tools.snapshots = vec![test_snapshot(DatasetId::from("demo"))];
    harness.state_mut().admin_tools.snapshots_error = Some("offline".to_string());
    harness.step();
    assert!(
        harness
            .query_by_label("Showing newly created backups. Catalog refresh failed: offline")
            .is_some()
    );

    harness.state_mut().admin_tools.section = AdminSection::Overview;
    harness.set_size(egui::vec2(900.0, 780.0));
    harness.step();
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::ComboBox, "Admin section")
            .is_some()
    );
    harness.set_size(egui::vec2(320.0, 568.0));
    harness.step();
    assert!(
        harness
            .query_all_by_role_and_label(egui::accesskit::Role::ComboBox, "Admin section")
            .next()
            .is_some()
    );
    assert_eq!(
        harness
            .query_all_by_role_and_label(egui::accesskit::Role::Button, "People")
            .count(),
        0
    );
    harness.state_mut().admin_tools.section = AdminSection::People;
    harness.step();
    let people_search = harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, "Search people")
        .rect();
    assert!((people_search.height() - theme::COMPACT_TEXT_FIELD_HEIGHT).abs() <= 1.0);
    harness.set_size(egui::vec2(390.0, 844.0));
    harness.step();
    let person_card = harness.get_by_label("Person card Admin User").rect();
    assert!(person_card.left() <= 38.5 && person_card.right() >= 347.5);
    harness.set_size(egui::vec2(320.0, 568.0));
    harness.step();
    harness.state_mut().admin_tools.section = AdminSection::Images;
    harness.step();
    let root_path = harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, "Root path")
        .rect();
    assert!(root_path.height() >= 43.0);
    let image_search = harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, "Search images")
        .rect();
    assert!((image_search.height() - theme::COMPACT_TEXT_FIELD_HEIGHT).abs() <= 1.0);
    assert_visible_controls_clamped(&harness, 320.0, 568.0);
    harness.state_mut().admin_tools.section = AdminSection::Automation;
    harness.step();
    let prelabels_card = harness.get_by_label("Prelabels card").rect();
    let location = harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, "Location")
        .rect();
    assert!(location.left() <= prelabels_card.left() + 16.0);
    assert!(
        location.right() >= prelabels_card.right() - 32.0,
        "location={location:?}, card={prelabels_card:?}"
    );
    assert_visible_controls_clamped(&harness, 320.0, 568.0);
}

#[test]
fn snapshot_load_history_advances_only_after_a_successful_catalog_request() {
    let mut app = base_live_app(Rc::new(SpyApi::new()));
    for (request_id, result) in [
        (1, Err("initial failure".to_string())),
        (2, Ok(Vec::new())),
        (3, Err("refresh failure".to_string())),
    ] {
        let request = test_request(&app, request_id, Some("demo"));
        app.runtime.active_requests.insert(request_id);
        app.loading.snapshots = true;
        app.runtime
            .tx
            .send(UiMessage::SnapshotsLoaded { request, result })
            .unwrap();
        app.process_messages(&egui::Context::default());
        if request_id == 1 {
            assert!(!app.admin_tools.snapshots_loaded);
        } else {
            assert!(app.admin_tools.snapshots_loaded);
        }
    }
    assert_eq!(
        app.admin_tools.snapshots_error.as_deref(),
        Some("refresh failure")
    );
}

#[test]
fn image_load_failure_shows_retry_and_loads_image() {
    let api = Rc::new(SpyApi::new());
    api.fail_next_preview();
    let mut harness = live_harness(api.clone());
    step_until(&mut harness, 8, |app| app.datasets.summaries.len() == 1);
    click(&mut harness, "Continue with Demo Dataset");
    step_until(&mut harness, 20, |app| {
        !app.loading.image
            && app
                .runtime
                .error
                .as_deref()
                .is_some_and(|error| error.contains("preview failed"))
    });
    harness.step();

    assert!(harness.state().current.is_none());
    assert!(
        harness
            .query_by_label("Assignment image unavailable")
            .is_some()
    );
    assert!(harness.query_by_label("Skip").is_some());
    assert!(
        harness
            .query_by_label_contains("Retry image load")
            .is_some()
    );
    harness.set_size(egui::vec2(320.0, 568.0));
    harness.step();
    assert!(
        harness
            .get_by_label("Workspace context bar")
            .rect()
            .height()
            <= 44.0
    );
    assert_visible_controls_clamped(&harness, 320.0, 568.0);
    click(&mut harness, "Retry image load");
    step_until(&mut harness, 12, |app| app.current.is_some());
    assert!(api.counts().get_image_preview >= 2);
    assert_eq!(api.counts().assign_next_image, 2);
}

#[test]
fn workers_select_class_specific_workflows() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);

    assert!(harness.query_all_by_label("Person boxes").next().is_some());
    assert!(harness.query_by_label("Current").is_none());
    assert!(harness.query_all_by_label("Vehicle boxes").next().is_some());
    click(&mut harness, "Vehicle boxes");
    release_and_switch(&mut harness);
    step_until(&mut harness, 12, |app| {
        app.selected_class_id() == Some(&ClassId::from("vehicle")) && app.current.is_some()
    });
    assert!(harness.query_all_by_label("Vehicle boxes").next().is_some());

    assert_eq!(
        harness
            .state()
            .selected_task()
            .map(|task| task.task_id.clone()),
        Some(TaskId::from("bounding_box:vehicle"))
    );
    assert!(harness.query_by_label("Accept").is_none());

    let canvas = harness.get_by_label("Annotation canvas");
    let rect = canvas.rect();
    let start = rect.left_top() + rect.size() * 0.25;
    let end = rect.left_top() + rect.size() * 0.45;
    harness.drag_at(start);
    harness.step();
    harness.hover_at(end);
    harness.step();
    harness.drop_at(end);
    harness.step();

    let annotation = harness.state().annotations.last().unwrap();
    assert_eq!(annotation.task_id, TaskId::from("bounding_box:vehicle"));
    assert_eq!(annotation.class_id, ClassId::from("vehicle"));
}

#[test]
fn missing_workflow_is_actionable() {
    let api = Rc::new(SpyApi::new());
    api.clear_workflows();
    let mut harness = live_harness(api);
    step_until(&mut harness, 8, |app| app.datasets.summaries.len() == 1);
    click(&mut harness, "Continue with Demo Dataset");
    step_until(&mut harness, 20, |app| {
        app.runtime
            .error
            .as_deref()
            .is_some_and(|error| error.contains("No enabled one-class workflow"))
    });

    assert!(
        harness
            .state()
            .runtime
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("No enabled one-class workflow")
    );
}

#[test]
fn no_available_assignment_is_a_normal_empty_state() {
    let api = Rc::new(SpyApi::new());
    api.set_no_assignment(true);
    let mut harness = live_harness(api);
    step_until(&mut harness, 8, |app| app.datasets.summaries.len() == 1);
    click(&mut harness, "Continue with Demo Dataset");
    step_until(&mut harness, 12, |app| {
        !app.loading.dataset && !app.loading.image
    });

    assert!(harness.state().current.is_none());
    assert!(harness.state().runtime.error.is_none());
    assert_eq!(
        harness.state().runtime.notice.as_deref(),
        Some("No annotation work is currently available.")
    );
}

#[test]
fn invalid_dataset_ids_are_rejected_before_an_api_request() {
    let api = Rc::new(SpyApi::new());
    let mut app = base_live_app(api.clone());
    app.setup.create_dataset_id = "../outside".to_string();
    app.setup.create_dataset_name = "Unsafe".to_string();
    app.request_create_dataset();

    assert_eq!(api.counts().create_dataset, 0);
    assert!(
        app.runtime
            .error
            .as_deref()
            .is_some_and(|error| error.contains("Dataset ID"))
    );
}

#[test]
fn stale_save_responses_cannot_replace_the_current_image_state() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);
    let current_id = harness
        .state()
        .current
        .as_ref()
        .unwrap()
        .image
        .image_id
        .clone();
    harness
        .state()
        .runtime
        .tx
        .send(UiMessage::SaveFinished {
            request: test_request(harness.state(), u64::MAX, Some("demo")),
            operation_id: u64::MAX,
            assignment_id: AssignmentId::generate(),
            edit_generation: 0,
            completed: false,
            result: Box::new(Ok(ImageState::new(ImageId::from("img_stale")))),
        })
        .unwrap();
    harness.step();

    assert_eq!(
        harness.state().current.as_ref().unwrap().image.image_id,
        current_id
    );
    assert_eq!(
        harness.state().current_state.as_ref().unwrap().image_id,
        current_id
    );
}

#[test]
fn keybindings_are_editable_and_persisted() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    assert!(harness.query_by_label("Keyboard shortcuts").is_none());
    click_application_menu_item(&mut harness, "Settings");
    assert!(harness.query_by_label("Keyboard shortcuts").is_some());
    click_accesskit_button(&mut harness, "Record shortcut for Submit and next");
    assert_eq!(
        harness.state().shortcut_settings.recording,
        Some(labello_domain::UserAction::NextImage),
    );
    harness.key_press(egui::Key::Enter);
    harness.step();
    assert_eq!(harness.state().shortcut_settings.recording, None);
    assert_eq!(
        harness
            .state()
            .shortcut_settings
            .draft
            .as_ref()
            .unwrap()
            .bindings[&labello_domain::UserAction::NextImage]
            .key,
        "Enter"
    );
    let save = harness
        .query_all_by_role_and_label(egui::accesskit::Role::Button, "Save changes")
        .next()
        .unwrap();
    assert!(!save.accesskit_node().is_disabled());
    click_accesskit_button(&mut harness, "Save changes");
    step_until(&mut harness, 8, |app| !app.loading.keybindings);

    assert_eq!(api.counts().save_keybindings, 1);
    assert_eq!(
        harness.state().keybindings.bindings[&labello_domain::UserAction::NextImage].key,
        "Enter"
    );
    assert_eq!(
        harness.state().runtime.notice.as_deref(),
        Some("Keyboard shortcuts saved")
    );
    click(&mut harness, "Cancel");
    harness.key_press(egui::Key::Enter);
    step_until(&mut harness, 16, |_| api.counts().complete_assignment == 1);
    assert_eq!(api.counts().complete_assignment, 1);
}

#[test]
fn failed_shortcut_save_keeps_the_draft_and_shows_the_error_in_settings() {
    let api = Rc::new(SpyApi::new());
    let mut app = LabelloApp::default();
    app.runtime.api = Some(api);
    app.open_shortcut_settings();
    app.shortcut_settings
        .draft
        .as_mut()
        .unwrap()
        .bindings
        .get_mut(&labello_domain::UserAction::NextImage)
        .unwrap()
        .key = "Enter".to_string();
    let draft = app.shortcut_settings.draft.clone();

    app.request_keybindings_save();
    let UiCommand::SaveKeybindings { request, .. } =
        app.runtime.commands.pop_back().expect("save command")
    else {
        panic!("expected keybinding save command");
    };
    app.runtime
        .tx
        .send(UiMessage::KeybindingsSaved {
            request,
            result: Err("settings unavailable".to_string()),
        })
        .unwrap();
    app.process_messages(&egui::Context::default());

    assert_eq!(app.shortcut_settings.draft, draft);
    assert_eq!(
        app.shortcut_settings.error.as_deref(),
        Some("settings unavailable")
    );
    let harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 780.0))
        .build_eframe(move |_| app);
    assert!(
        harness
            .query_by_label("Could not save shortcuts: settings unavailable")
            .is_some()
    );
}

#[test]
fn shortcut_settings_cancel_discards_the_draft() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);
    click_application_menu_item(&mut harness, "Settings");
    click_accesskit_button(&mut harness, "Record shortcut for Submit and next");
    harness.key_press(egui::Key::Escape);
    harness.step();
    assert!(harness.state().show_settings);
    assert_eq!(harness.state().shortcut_settings.recording, None);
    assert!(!harness.state().shortcut_settings.confirm_discard);
    click_accesskit_button(&mut harness, "Record shortcut for Submit and next");
    harness.key_press(egui::Key::Enter);
    harness.step();
    click(&mut harness, "Cancel");
    harness.step();
    assert!(
        harness
            .query_by_label("Discard shortcut changes?")
            .is_some()
    );
    click_accesskit_button(&mut harness, "Discard changes");

    assert!(!harness.state().show_settings);
    assert_eq!(
        harness.state().keybindings.bindings[&labello_domain::UserAction::NextImage].key,
        "ArrowRight"
    );
}

#[test]
fn shortcut_settings_lock_editing_while_saving() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);
    click_application_menu_item(&mut harness, "Settings");
    harness.state_mut().loading.keybindings = true;
    harness.step();

    assert!(harness.query_by_label("Close window").is_none());

    for label in [
        "Record shortcut for Submit and next",
        "Reset Submit and next",
        "Restore all defaults",
        "Cancel",
    ] {
        let control = harness
            .query_all_by_label_contains(label)
            .find(|node| node.accesskit_node().role() == egui::accesskit::Role::Button)
            .unwrap_or_else(|| panic!("missing {label}"));
        assert!(control.accesskit_node().is_disabled(), "{label} is enabled");
    }
}

#[test]
fn draft_recovery_modal_blocks_background_controls() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1500.0, 780.0))
        .build_eframe(|_| LabelloApp::default());
    let menu = harness
        .query_all_by_label_contains("Open settings")
        .find(|node| node.accesskit_node().role() == egui::accesskit::Role::Button)
        .expect("settings button")
        .rect()
        .center();
    let metadata = SpyApi::new().metadata();
    let identity = crate::persistence::StorageIdentity::new(
        &harness.state().config.api_base_url,
        harness.state().config.user_id.clone(),
    )
    .unwrap();
    let draft = crate::persistence::AdminDraft::new(
        &identity,
        metadata.dataset_id.clone(),
        &metadata,
        &metadata,
    );
    harness.state_mut().runtime.persistence.recovery =
        Some(crate::persistence::DraftRecovery::Admin(
            Box::new(draft),
            crate::persistence::DraftValidation::Valid,
        ));
    harness.step();
    assert!(harness.query_by_label("Unsaved admin draft").is_some());

    click_at(&mut harness, menu);

    assert!(!harness.state().show_settings);
    assert!(harness.state().runtime.persistence.recovery.is_some());
}

#[test]
fn overlays_and_menus_block_background_shortcuts() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    let image_id = harness
        .state()
        .assignment
        .as_ref()
        .unwrap()
        .image_id
        .clone();
    harness.state_mut().drawer = Some(Drawer::Inspector);
    harness.step();

    harness.key_press(egui::Key::ArrowRight);
    harness.step();

    assert_eq!(api.counts().complete_assignment, 0);
    assert_eq!(
        harness.state().assignment.as_ref().unwrap().image_id,
        image_id
    );

    harness.state_mut().canvas.zoom_in();
    harness.state_mut().canvas.toggle_pan_mode();
    harness.key_press(egui::Key::Escape);
    harness.step();
    assert!(harness.state().canvas.pan_mode());
    harness.state_mut().drawer = None;
    harness.step();

    harness.set_size(egui::vec2(320.0, 568.0));
    harness.step();
    click(&mut harness, "More application actions");
    harness.key_press(egui::Key::ArrowRight);
    harness.step();
    assert_eq!(api.counts().complete_assignment, 0);
    assert_eq!(
        harness.state().assignment.as_ref().unwrap().image_id,
        image_id
    );
}

#[test]
fn pan_mode_shortcut_requires_zoom_and_escape_returns_to_annotation_mode() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);
    let zoom = harness.state().keybindings.bindings[&labello_domain::UserAction::ZoomIn].clone();
    harness
        .state_mut()
        .keybindings
        .bindings
        .insert(labello_domain::UserAction::RetryImageLoad, zoom);
    assert!(harness.state().keybindings.validate().is_ok());

    harness.key_press(egui::Key::P);
    harness.step();
    assert!(!harness.state().canvas.pan_mode());
    harness.key_press(egui::Key::Plus);
    harness.step();
    assert!(harness.state().canvas.current_zoom() > 1.0);
    harness.key_press(egui::Key::P);
    harness.step();
    assert!(harness.state().canvas.pan_mode());
    assert!(harness.query_by_label("Pan").is_some());

    harness.key_press(egui::Key::Escape);
    harness.step();
    assert!(!harness.state().canvas.pan_mode());
}

#[test]
fn logical_primary_and_shifted_punctuation_shortcuts_dispatch() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    harness.state_mut().create_bbox(BoundingBox {
        x: 0.1,
        y: 0.1,
        width: 0.2,
        height: 0.2,
    });
    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::S);
    harness.step();
    step_until(&mut harness, 8, |app| !app.loading.saving);
    assert_eq!(api.counts().append_event, 1);

    harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Questionmark);
    harness.step();
    assert!(harness.state().show_tutorial);
}

#[test]
fn long_status_messages_keep_their_complete_accessible_text() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);
    let message = "A deliberately long status message that exceeds the visible top bar limit but must remain available to assistive technology and pointer users.";
    harness.state_mut().runtime.error = Some(message.to_string());
    for (width, height) in [(320.0, 568.0), (600.0, 800.0), (1440.0, 900.0)] {
        harness.set_size(egui::vec2(width, height));
        harness.step();
        let dataset = harness.get_by_label("Dataset Demo Dataset").rect();
        let status_label = format!("Status: Idle. Error: {message}");
        let status = harness
            .get_by_role_and_label(egui::accesskit::Role::Button, &status_label)
            .rect();
        let left_action = harness
            .query_by_label("More application actions")
            .or_else(|| {
                harness
                    .query_all_by_role_and_label(egui::accesskit::Role::Button, "Annotate")
                    .next()
            })
            .expect("top-bar navigation control")
            .rect();
        assert!((dataset.center().x - width / 2.0).abs() <= 1.0);
        assert!(left_action.right() <= dataset.left() + 0.5);
        assert!(dataset.right() <= status.left() + 0.5);
        assert_visible_controls_clamped(&harness, width, height);
    }
    let status_label = format!("Status: Idle. Error: {message}");
    click_accesskit_button(&mut harness, &status_label);
    assert!(
        harness
            .query_all_by_label_contains(message)
            .any(|node| node.accesskit_node().role() == egui::accesskit::Role::Label)
    );

    harness.key_press(egui::Key::Escape);
    harness.state_mut().runtime.error = None;
    harness.state_mut().save_status = SaveStatus::Dirty;
    let notice = "Dataset catalog refreshed";
    harness.state_mut().runtime.notice = Some(notice.to_string());
    harness.step();
    assert!(harness.query_by_label(notice).is_none());
    click_accesskit_button(&mut harness, &format!("Status: Unsaved. Update: {notice}"));
    assert!(
        harness
            .query_all_by_label_contains(notice)
            .any(|node| node.accesskit_node().role() == egui::accesskit::Role::Label)
    );
}

#[test]
fn right_arrow_submits_and_claims_a_different_image() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    let original = harness
        .state()
        .assignment
        .as_ref()
        .unwrap()
        .image_id
        .clone();

    step_until(&mut harness, 12, |app| app.queue.len() == 2);
    let next = harness.state().queue.prepared_image_ids()[0].clone();
    let previews_before = api.counts().get_image_preview;
    harness.key_press(egui::Key::ArrowRight);
    step_until(&mut harness, 16, |app| {
        app.assignment
            .as_ref()
            .is_some_and(|assignment| assignment.image_id != original)
    });

    assert_eq!(api.counts().complete_assignment, 1);
    assert_eq!(api.counts().release_assignment, 0);
    assert_eq!(harness.state().assignment.as_ref().unwrap().image_id, next);
    assert_eq!(api.counts().get_image_preview, previews_before);
    assert!(!harness.state().loading.image);
    assert!(harness.state().current_texture.is_some());
}

#[test]
fn annotation_prefetch_fills_two_without_blocking_the_current_image() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    step_until(&mut harness, 12, |app| app.queue.len() == 2);

    assert_eq!(harness.state().queue.queue_size(), 2);
    assert!(!harness.state().loading.image);
    assert!(
        harness
            .query_by_label("Prepared queue: 2 of 2 ready")
            .is_some()
    );
    let exclusions = api.exclusions();
    assert!(exclusions.iter().all(|excluded| excluded.len() <= 3));
    assert_eq!(exclusions[0], Vec::<ImageId>::new());
    assert_eq!(exclusions[1].len(), 1);
    assert_eq!(exclusions[2].len(), 2);
}

#[test]
fn empty_prepared_queue_falls_back_to_blocking_load() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);
    step_until(&mut harness, 12, |app| app.queue.len() == 2);
    harness.state_mut().queue.clear();

    click(&mut harness, "Submit & next");
    harness.step();
    assert!(harness.state().loading.image);
    assert!(harness.state().current.is_none());
    harness.step();
    assert!(harness.state().current.is_some());
}

#[test]
fn submit_failure_preserves_current_and_prepared_queue() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    step_until(&mut harness, 12, |app| app.queue.len() == 2);
    let current = harness
        .state()
        .assignment
        .as_ref()
        .unwrap()
        .image_id
        .clone();
    let queued = harness.state().queue.prepared_image_ids();
    api.fail_next_batch();

    click(&mut harness, "Submit & next");
    step_until(&mut harness, 8, |app| !app.loading.saving);

    assert_eq!(
        harness.state().assignment.as_ref().unwrap().image_id,
        current
    );
    assert_eq!(harness.state().queue.prepared_image_ids(), queued);
}

#[test]
fn stale_prefetch_response_cannot_enter_the_queue() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    step_until(&mut harness, 12, |app| app.queue.len() == 2);
    let loaded = harness.state_mut().queue.pop_prepared().unwrap();
    harness.state_mut().queue.clear();
    let operation_id = 90_001;
    let request = test_request(harness.state(), operation_id, Some("demo"));
    harness.state_mut().active_prefetch_id = Some(operation_id);
    harness
        .state_mut()
        .runtime
        .active_requests
        .insert(operation_id);
    harness.state_mut().begin_workspace_epoch();
    harness
        .state()
        .runtime
        .tx
        .send(UiMessage::PrefetchLoaded {
            request,
            operation_id,
            result: Box::new(Ok(Some(loaded))),
        })
        .unwrap();

    harness
        .state_mut()
        .process_messages(&egui::Context::default());
    assert!(harness.state().queue.is_empty());
    step_until(&mut harness, 8, |_| api.counts().release_assignment > 0);
}

#[test]
fn stale_blocking_claim_releases_its_assignment() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    step_until(&mut harness, 12, |app| app.queue.len() == 2);
    let assignment = harness.state_mut().queue.pop_prepared().unwrap().assignment;
    harness.state_mut().queue.clear();
    let operation_id = 90_002;
    let request = test_request(harness.state(), operation_id, Some("demo"));
    harness.state_mut().active_load_id = Some(operation_id);
    harness
        .state_mut()
        .runtime
        .active_requests
        .insert(operation_id);
    harness.state_mut().begin_workspace_epoch();
    harness
        .state()
        .runtime
        .tx
        .send(UiMessage::ImageLoaded {
            request,
            operation_id,
            assignment: Some(assignment),
            result: Box::new(Err("stale load".to_string())),
        })
        .unwrap();

    harness
        .state_mut()
        .process_messages(&egui::Context::default());
    step_until(&mut harness, 8, |_| api.counts().release_assignment > 0);
}

#[test]
fn save_keeps_the_same_assignment_active() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    step_until(&mut harness, 12, |app| app.queue.len() == 2);
    let claims_before = api.counts().assign_next_image;
    click(&mut harness, "Accept");
    let assignment_id = harness
        .state()
        .assignment
        .as_ref()
        .unwrap()
        .assignment_id
        .clone();

    click(&mut harness, "Save");
    step_until(&mut harness, 10, |app| app.save_status == SaveStatus::Saved);

    assert_eq!(
        harness
            .state()
            .assignment
            .as_ref()
            .map(|assignment| &assignment.assignment_id),
        Some(&assignment_id)
    );
    assert_eq!(api.counts().complete_assignment, 0);
    assert_eq!(api.counts().assign_next_image, claims_before);
}

#[test]
fn annotation_edits_debounce_once_and_undo_redo_remain_available() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());

    click(&mut harness, "Accept");
    assert_eq!(harness.state().save_status, SaveStatus::Dirty);
    assert_eq!(api.counts().append_event, 0);
    click(&mut harness, "More actions");
    assert!(harness.query_by_label_contains("Undo").is_some());
    harness.key_press(egui::Key::Escape);
    harness.step();

    harness.state_mut().undo();
    assert!(harness.state().annotations.is_empty());
    harness.state_mut().redo();
    assert_eq!(harness.state().annotations.len(), 1);

    harness.state_mut().last_edit_at = Some(Instant::now() - Duration::from_secs(1));
    harness.state_mut().autosave_if_due();
    assert_eq!(harness.state().save_status, SaveStatus::Saving);
    step_until(&mut harness, 10, |app| app.save_status == SaveStatus::Saved);
    assert_eq!(api.counts().append_event, 1);

    harness.state_mut().autosave();
    for _ in 0..3 {
        harness.step();
    }
    assert_eq!(api.counts().append_event, 1);

    harness.state_mut().undo();
    assert!(
        harness
            .state()
            .annotations
            .iter()
            .all(|annotation| annotation.deleted)
    );
    harness.state_mut().redo();
    assert_eq!(
        harness
            .state()
            .annotations
            .iter()
            .filter(|annotation| !annotation.deleted)
            .count(),
        1
    );
}

#[test]
fn autosave_waits_for_an_active_canvas_drag() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    click(&mut harness, "Accept");
    let start = harness.get_by_label("Annotation canvas").rect().center();
    harness.drag_at(start);
    harness.step();
    assert!(harness.state().canvas.is_dragging());

    harness.state_mut().last_edit_at = Some(Instant::now() - Duration::from_secs(1));
    harness.state_mut().autosave_if_due();

    assert_eq!(harness.state().save_status, SaveStatus::Dirty);
    assert!(!harness.state().loading.saving);
    assert_eq!(api.counts().annotation_batch, 0);
}

#[test]
fn edits_made_during_save_remain_dirty_when_the_saved_generation_finishes() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    click(&mut harness, "Accept");
    harness.state_mut().request_save(false);
    harness.state_mut().create_bbox(BoundingBox {
        x: 0.55,
        y: 0.55,
        width: 0.2,
        height: 0.2,
    });

    step_until(&mut harness, 10, |app| !app.loading.saving);
    assert_eq!(harness.state().save_status, SaveStatus::Dirty);
    assert_eq!(harness.state().annotations.len(), 2);
    assert_eq!(api.counts().annotation_batch, 1);
}

#[test]
fn a_full_command_queue_cannot_strand_save_loading() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);
    click(&mut harness, "Accept");
    while harness.state().runtime.commands.len() < 64 {
        let request_id = 10_000 + harness.state().runtime.commands.len() as u64;
        let request = test_request(harness.state(), request_id, None);
        assert!(
            harness
                .state_mut()
                .queue_command(UiCommand::DatasetList { request })
        );
    }

    harness.state_mut().submit_and_advance();

    assert!(!harness.state().loading.saving);
    assert_eq!(harness.state().active_operation_id, None);
    assert_eq!(harness.state().save_status, SaveStatus::Retry);
    assert!(harness.state().pending_transition.is_none());
}

#[test]
fn queue_saturation_rolls_back_dataset_admin_and_session_owners() {
    let api = Rc::new(SpyApi::new());
    let metadata = api.metadata();
    let users = api.dataset_users();
    let mut app = base_live_app(api);
    app.auth.checked = true;
    app.datasets.metadata = Some(metadata.clone());
    app.datasets.admin_config = Some(metadata.clone());
    app.datasets.admin_baseline = Some(metadata);
    app.datasets.users = users.clone();
    app.datasets.users_baseline = users;

    saturate_command_queue(&mut app);
    app.request_dataset_list();
    assert!(!app.loading.datasets);
    assert!(app.datasets.summaries_error.is_some());

    saturate_command_queue(&mut app);
    app.setup.create_dataset_id = "queued-dataset".to_string();
    app.setup.create_dataset_name = "Queued dataset".to_string();
    app.request_create_dataset();
    assert!(!app.loading.dataset);

    saturate_command_queue(&mut app);
    app.request_admin_dataset();
    assert!(!app.loading.admin);
    assert!(app.admin_tools.load_error.is_some());

    saturate_command_queue(&mut app);
    app.request_admin_save();
    assert!(!app.loading.admin);

    app.datasets
        .users
        .iter_mut()
        .find(|user| user.account.user_id == UserId::from("reviewer"))
        .unwrap()
        .roles
        .push(DatasetRole::Reviewer);
    saturate_command_queue(&mut app);
    app.request_admin_changes_save();
    assert!(app.loading.roles_user.is_none());
    assert!(app.admin_tools.pending_role_saves.is_empty());

    saturate_command_queue(&mut app);
    app.request_images();
    assert!(!app.loading.images);
    assert!(app.admin_tools.images_error.is_some());

    saturate_command_queue(&mut app);
    app.request_snapshots();
    assert!(!app.loading.snapshots);
    assert!(app.admin_tools.snapshots_error.is_some());

    saturate_command_queue(&mut app);
    app.request_snapshot_create();
    assert!(!app.loading.creating_snapshot);
    assert!(app.admin_tools.snapshot_action_error.is_some());

    saturate_command_queue(&mut app);
    app.request_snapshot_download("snapshot".to_string(), "manifest.json".to_string());
    assert!(app.loading.snapshot_file.is_none());
    assert!(app.admin_tools.snapshot_action_error.is_some());

    saturate_command_queue(&mut app);
    app.request_ingest();
    assert!(!app.loading.ingesting);
    assert!(!app.loading.ingest_polling);

    saturate_command_queue(&mut app);
    app.loading.ingesting = true;
    app.loading.ingest_job_id = Some("job".to_string());
    app.loading.last_ingest_poll = Some(Instant::now() - Duration::from_secs(1));
    app.refresh_ingest_if_due();
    assert!(app.loading.ingesting);
    assert!(!app.loading.ingest_polling);

    saturate_command_queue(&mut app);
    app.request_keybindings_save();
    assert!(!app.loading.keybindings);
    assert!(app.shortcut_settings.error.is_some());

    app.view = AppView::Stats;
    saturate_command_queue(&mut app);
    app.request_stats();
    assert!(!app.loading.stats);
    assert!(app.datasets.active_stats_request.is_none());
    assert!(app.datasets.stats_error.is_some());

    saturate_command_queue(&mut app);
    let session_request = test_request(&app, 90_001, None);
    app.loading.session = true;
    app.auth.checked = false;
    app.auth.active_session_request_id = Some(session_request.request_id);
    assert!(!app.queue_command(UiCommand::Session {
        request: session_request
    }));
    assert!(!app.loading.session);
    assert!(app.auth.checked);
    assert!(app.auth.active_session_request_id.is_none());

    saturate_command_queue(&mut app);
    let logout_request = test_request(&app, 90_002, None);
    app.loading.logout = true;
    assert!(!app.queue_command(UiCommand::Logout {
        request: logout_request
    }));
    assert!(!app.loading.logout);
}

#[test]
fn queue_saturation_rolls_back_claim_release_review_and_adjudication() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);

    saturate_command_queue(harness.state_mut());
    harness.state_mut().skip_assignment();
    assert!(!harness.state().loading.saving);
    assert!(harness.state().active_operation_id.is_none());
    assert!(harness.state().pending_transition.is_none());

    harness.state_mut().clear_current_image();
    saturate_command_queue(harness.state_mut());
    harness.state_mut().request_next_image();
    assert!(!harness.state().loading.image);
    assert!(harness.state().active_load_id.is_none());
    assert!(!harness.state().queue.is_loading());

    let api = Rc::new(SpyApi::new());
    seed_review_annotation(
        &api,
        AnnotationGeometry::BoundingBox(BoundingBox {
            x: 0.2,
            y: 0.2,
            width: 0.3,
            height: 0.3,
        }),
        true,
    );
    let mut review = loaded_review_harness(api);
    saturate_command_queue(review.state_mut());
    review
        .state_mut()
        .request_review(labello_domain::ReviewDecision::Approved);
    assert!(!review.state().loading.saving);
    assert!(review.state().active_operation_id.is_none());

    let annotation_id = review.state().selected_annotation.clone().unwrap();
    review.state_mut().start_correction();
    review.state_mut().edit_correction_bbox(BoundingBoxEdit {
        annotation_id,
        bounding_box: BoundingBox {
            x: 0.3,
            y: 0.3,
            width: 0.2,
            height: 0.2,
        },
    });
    saturate_command_queue(review.state_mut());
    review.state_mut().request_correction();
    assert!(!review.state().loading.saving);
    assert!(review.state().active_operation_id.is_none());
    assert!(review.state().correction_draft.is_some());

    review.state_mut().view = AppView::Adjudicate;
    review.state_mut().assignment.as_mut().unwrap().kind = AssignmentKind::Adjudication;
    saturate_command_queue(review.state_mut());
    review
        .state_mut()
        .request_adjudication(labello_domain::AdjudicationDecision::AcceptAnnotation);
    assert!(!review.state().loading.saving);
    assert!(review.state().active_operation_id.is_none());
}

#[test]
fn stale_auth_and_workspace_messages_cannot_mutate_current_owners() {
    let mut app = base_live_app(Rc::new(SpyApi::new()));
    let stale_auth = test_request(&app, 100, None);
    app.begin_auth_epoch();
    app.loading.datasets = true;
    app.runtime.active_requests.insert(101);
    app.runtime
        .tx
        .send(UiMessage::DatasetList {
            request: stale_auth,
            result: Ok(vec![DatasetSummary {
                dataset_id: DatasetId::from("stale"),
                name: "Stale".to_string(),
                roles: vec![DatasetRole::DataAdmin],
                total_images: 999,
            }]),
        })
        .unwrap();
    app.process_messages(&egui::Context::default());
    assert!(app.loading.datasets);
    assert!(app.datasets.summaries.is_empty());
    assert!(app.runtime.active_requests.contains(&101));

    let stale_workspace = test_request(&app, 102, Some("demo"));
    app.begin_workspace_epoch();
    app.config.dataset_id = DatasetId::from("other");
    app.loading.admin = true;
    app.runtime.active_requests.insert(103);
    app.runtime
        .tx
        .send(UiMessage::AdminSaved {
            request: stale_workspace,
            result: Box::new(Ok(SpyApi::new().metadata())),
        })
        .unwrap();
    app.process_messages(&egui::Context::default());
    assert!(app.loading.admin);
    assert!(app.datasets.admin_config.is_none());
    assert!(app.runtime.active_requests.contains(&103));
}

#[test]
fn api_login_logout_dataset_and_view_boundaries_rotate_epochs() {
    let mut app = base_live_app(Rc::new(SpyApi::new()));
    let initial_auth = app.auth_epoch;
    let initial_workspace = app.workspace_epoch;
    app.datasets.requested_view = Some(AppView::Admin);
    app.runtime.persistence.restoration_attempted = true;

    app.rebuild_http_api();
    assert!(app.auth_epoch > initial_auth);
    assert!(app.workspace_epoch > initial_workspace);
    assert!(app.datasets.requested_view.is_none());
    assert!(!app.runtime.persistence.restoration_attempted);

    let rebuilt_auth = app.auth_epoch;
    app.request_session();
    assert!(app.auth_epoch > rebuilt_auth);
    let login_request = app.runtime.commands.back().unwrap().request();
    assert_eq!(login_request.auth_epoch, app.auth_epoch);
    assert_eq!(login_request.workspace_epoch, app.workspace_epoch);

    app.loading.session = false;
    let login_auth = app.auth_epoch;
    app.request_logout();
    assert!(app.auth_epoch > login_auth);
    let logout_request = app.runtime.commands.back().unwrap().request();
    assert_eq!(logout_request.auth_epoch, app.auth_epoch);

    app.loading.logout = false;
    app.runtime.commands.clear();
    let before_dataset = app.workspace_epoch;
    app.request_load_dataset();
    assert!(app.workspace_epoch > before_dataset);

    app.loading.dataset = false;
    app.runtime.commands.clear();
    app.datasets.metadata = Some(SpyApi::new().metadata());
    app.view = AppView::Annotate;
    let before_view = app.workspace_epoch;
    app.execute_transition(crate::app::PendingTransition::View(AppView::Stats));
    assert!(app.workspace_epoch > before_view);
}

#[test]
fn dataset_creation_completion_accepts_its_new_dataset_identity() {
    let mut app = base_live_app(Rc::new(SpyApi::new()));
    let mut metadata = SpyApi::new().metadata();
    metadata.dataset_id = DatasetId::from("new-dataset");
    metadata.name = "New dataset".to_string();
    let request = test_request(&app, 700, Some("new-dataset"));
    app.loading.dataset = true;
    app.runtime.active_requests.insert(request.request_id);
    app.runtime
        .tx
        .send(UiMessage::DatasetCreated {
            request,
            result: Box::new(Ok(metadata)),
        })
        .unwrap();

    app.process_messages(&egui::Context::default());

    assert_eq!(app.config.dataset_id, DatasetId::from("new-dataset"));
    assert_eq!(app.datasets.requested_view, Some(AppView::Admin));
    assert!(app.loading.dataset);
    let load = app.runtime.commands.front().unwrap();
    assert_eq!(
        load.request().dataset_id.as_ref(),
        Some(&DatasetId::from("new-dataset"))
    );
}

#[test]
fn explicit_dataset_transition_suppresses_workspace_restoration() {
    let mut app = base_live_app(Rc::new(SpyApi::new()));
    app.config.dataset_id = DatasetId::from("new-dataset");
    app.datasets.requested_view = Some(AppView::Admin);
    app.runtime.persistence.preference = Some(WorkspacePreference {
        version: 1,
        dataset_id: DatasetId::from("demo"),
        view: StoredView::Annotate,
        task_id: None,
        assignment_id: None,
        assignment_image_id: None,
        assignment_kind: None,
        drawer: None,
        show_settings: false,
        show_tutorial: false,
        selected_annotation: None,
        canvas: StoredCanvasTransform {
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
        },
    });
    app.request_load_dataset();
    assert!(app.runtime.persistence.restoration_attempted);
    app.loading.dataset = false;
    app.datasets.requested_view = None;
    let workspace_epoch = app.workspace_epoch;

    app.reopen_previous_workspace();

    assert_eq!(app.config.dataset_id, DatasetId::from("new-dataset"));
    assert!(app.datasets.requested_view.is_none());
    assert_eq!(app.workspace_epoch, workspace_epoch);
}

#[test]
fn dataset_list_success_only_clears_its_own_error() {
    let mut app = base_live_app(Rc::new(SpyApi::new()));
    app.runtime.error = Some("dataset load failed".to_string());
    app.request_dataset_list();
    let UiCommand::DatasetList { request } = app.runtime.commands.pop_back().unwrap() else {
        panic!("expected dataset list command");
    };
    app.runtime
        .tx
        .send(UiMessage::DatasetList {
            request,
            result: Ok(Vec::new()),
        })
        .unwrap();

    app.process_messages(&egui::Context::default());

    assert_eq!(app.runtime.error.as_deref(), Some("dataset load failed"));

    app.datasets.summaries_error = Some("list failed".to_string());
    app.runtime.error = Some("list failed".to_string());
    app.request_dataset_list();
    assert!(app.datasets.summaries_error.is_none());
    assert!(app.runtime.error.is_none());
    let UiCommand::DatasetList { request } = app.runtime.commands.pop_back().unwrap() else {
        panic!("expected dataset list command");
    };
    app.runtime
        .tx
        .send(UiMessage::DatasetList {
            request,
            result: Ok(Vec::new()),
        })
        .unwrap();

    app.process_messages(&egui::Context::default());

    assert!(app.runtime.error.is_none());
}

#[test]
fn setup_recommends_a_single_continue_work_action() {
    let api = Rc::new(SpyApi::new());
    let mut harness = live_harness(api);
    step_until(&mut harness, 8, |app| !app.datasets.summaries.is_empty());

    let recommended = harness
        .get_by_label("Recommended dataset Demo Dataset")
        .rect();
    let all_datasets = harness.get_by_label("All datasets").rect();
    let dataset = harness.get_by_label("Dataset card Demo Dataset").rect();
    assert!(recommended.bottom() < all_datasets.top());
    assert!(all_datasets.bottom() < dataset.top());
    assert!(
        harness
            .query_all_by_role_and_label(egui::accesskit::Role::Button, "Annotate Demo Dataset",)
            .next()
            .is_none()
    );
    assert_eq!(
        harness
            .query_all_by_role_and_label(
                egui::accesskit::Role::Button,
                "Continue with Demo Dataset",
            )
            .count(),
        1
    );
    click(&mut harness, "Continue with Demo Dataset");
    step_until(&mut harness, 12, |app| app.current.is_some());
    assert_eq!(harness.state().view, AppView::Annotate);
}

#[test]
fn setup_does_not_recommend_a_dataset_without_an_available_destination() {
    let api = Rc::new(SpyApi::new());
    api.set_summary_roles(Vec::new());
    let mut harness = live_harness(api);
    step_until(&mut harness, 8, |app| !app.datasets.summaries.is_empty());

    assert!(harness.query_by_label("Recommended").is_none());
    assert!(
        harness
            .query_by_label("Continue with Demo Dataset")
            .is_none()
    );
    assert!(
        harness
            .query_by_label("Dataset card Demo Dataset")
            .is_some()
    );
}

#[test]
fn setup_describes_a_data_admin_recommendation_as_statistics() {
    let api = Rc::new(SpyApi::new());
    api.set_summary_roles(vec![DatasetRole::DataAdmin]);
    let mut harness = live_harness(api);
    step_until(&mut harness, 8, |app| !app.datasets.summaries.is_empty());

    assert!(
        harness
            .query_by_label("View statistics for this dataset.")
            .is_some()
    );
    assert!(
        harness
            .query_by_label("Open the suggested work queue for this dataset.")
            .is_none()
    );
    assert!(harness.query_by_label("Stats Demo Dataset").is_none());
    assert!(harness.query_by_label("Admin Demo Dataset").is_some());
}

#[test]
fn signed_in_setup_sections_label_and_size_inputs() {
    let api = Rc::new(SpyApi::new());
    let mut harness = live_harness(api);
    step_until(&mut harness, 8, |app| !app.datasets.summaries.is_empty());

    assert!(harness.query_by_label("Choose where to work").is_some());
    assert!(harness.query_by_label("API URL").is_none());
    assert!(harness.state().setup.create_dataset_id.is_empty());
    assert!(harness.state().setup.create_dataset_name.is_empty());

    select_setup_section(&mut harness, "Connection");
    let api_url = harness
        .query_all_by_role_and_label(egui::accesskit::Role::TextInput, "API URL")
        .next()
        .expect("API URL field should have an accessible label");
    assert!(api_url.rect().height() <= 25.0);
    assert!(harness.query_by_label("Development user ID").is_none());
    assert!(harness.query_by_label("Dev token").is_none());
    harness.set_size(egui::vec2(900.0, 1200.0));
    harness.step();
    select_setup_section(&mut harness, "Create");
    for label in ["Dataset ID", "Dataset name"] {
        let field = harness
            .get_by_role_and_label(egui::accesskit::Role::TextInput, label)
            .rect();
        assert!((field.height() - theme::COMPACT_TEXT_FIELD_HEIGHT).abs() <= 1.0);
    }

    harness.set_size(egui::vec2(390.0, 844.0));
    harness.step();
    select_setup_section(&mut harness, "Connection");
    let compact_api_url = harness
        .query_all_by_role_and_label(egui::accesskit::Role::TextInput, "API URL")
        .next()
        .expect("compact API URL field should retain its accessible label");
    assert!(compact_api_url.rect().height() <= 25.0);
    assert!(compact_api_url.rect().right() <= 390.5);
}

#[test]
fn api_url_focus_loss_does_not_reconnect_and_enter_commits() {
    let mut app = LabelloApp {
        view: AppView::Setup,
        ..Default::default()
    };
    app.setup.section = SetupSection::Connection;
    let original_url = app.config.api_base_url.clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 780.0))
        .build_eframe(move |_| app);
    let input = harness
        .query_all_by_role_and_label(egui::accesskit::Role::TextInput, "API URL")
        .next()
        .expect("API URL field")
        .rect()
        .center();
    click_at(&mut harness, input);
    harness.state_mut().setup.api_base_url_draft = "not a URL".to_string();
    harness.step();
    let auth_epoch = harness.state().auth_epoch;

    let connection = harness.get_by_label("Connection").rect().center();
    click_at(&mut harness, connection);
    assert_eq!(harness.state().config.api_base_url, original_url);
    assert_eq!(harness.state().auth_epoch, auth_epoch);
    assert!(harness.query_by_label("Reconnect").is_some());

    let input = harness
        .query_all_by_role_and_label(egui::accesskit::Role::TextInput, "API URL")
        .next()
        .unwrap()
        .rect()
        .center();
    click_at(&mut harness, input);
    harness.key_press(egui::Key::Enter);
    harness.step();
    assert_eq!(harness.state().config.api_base_url, "not a URL");
    assert!(harness.state().auth_epoch > auth_epoch);
    harness.state_mut().setup.section = SetupSection::Datasets;
    harness.step();
    assert_eq!(harness.state().setup.section, SetupSection::Connection);
    assert!(harness.query_by_label("Reconnect").is_some());
    assert!(
        harness
            .query_by_label("Checking dataset access...")
            .is_none()
    );
}

#[test]
fn dataset_states_distinguish_loading_and_stale_refresh_failure() {
    let api = Rc::new(SpyApi::new());
    let mut harness = live_harness(api);
    step_until(&mut harness, 8, |app| !app.datasets.summaries.is_empty());
    let summaries = harness.state().datasets.summaries.clone();

    harness.state_mut().auth.checked = false;
    harness.state_mut().loading.session = true;
    harness.step();
    assert!(
        harness
            .query_by_label("Checking dataset access...")
            .is_some()
    );
    assert!(
        harness
            .query_by_label("Continue with Demo Dataset")
            .is_none()
    );
    harness.state_mut().auth.checked = true;
    harness.state_mut().loading.session = false;
    harness.step();

    harness.state_mut().datasets.summaries.clear();
    harness.state_mut().loading.datasets = true;
    harness.step();
    assert!(harness.query_by_label("Loading datasets...").is_some());
    assert!(
        harness
            .query_by_label("No accessible datasets yet.")
            .is_none()
    );

    harness.state_mut().loading.datasets = false;
    harness.state_mut().datasets.summaries_error = Some("initial failure".to_string());
    harness.step();
    assert!(
        harness
            .query_by_label("Could not load datasets: initial failure")
            .is_some()
    );
    assert!(
        harness
            .query_by_label("No accessible datasets yet.")
            .is_none()
    );

    harness.state_mut().datasets.summaries = summaries.clone();
    harness.state_mut().request_dataset_list();
    let UiCommand::DatasetList { request } = harness
        .state_mut()
        .runtime
        .commands
        .pop_back()
        .expect("dataset list command")
    else {
        panic!("expected dataset list command");
    };
    harness
        .state_mut()
        .runtime
        .tx
        .send(UiMessage::DatasetList {
            request,
            result: Err("dataset service unavailable".to_string()),
        })
        .unwrap();
    harness
        .state_mut()
        .process_messages(&egui::Context::default());
    harness.step();

    assert_eq!(harness.state().datasets.summaries, summaries);
    assert!(
        harness
            .query_by_label("Showing saved results. Refresh failed: dataset service unavailable")
            .is_some()
    );
    assert!(
        harness
            .query_by_label("Continue with Demo Dataset")
            .is_some()
    );
    harness.set_size(egui::vec2(320.0, 568.0));
    harness.state_mut().loading.dataset = true;
    harness.step();
    let refresh = harness.get_by_role_and_label(egui::accesskit::Role::Button, "Refresh");
    let retry = harness.get_by_role_and_label(egui::accesskit::Role::Button, "Retry");
    let opening = harness.get_by_label("Opening dataset...").rect();
    assert!(opening.top() >= refresh.rect().bottom());
    assert!(refresh.accesskit_node().is_disabled());
    assert!(retry.accesskit_node().is_disabled());
}

#[test]
fn failed_admin_navigation_stays_in_admin_with_page_retry() {
    let api = Rc::new(SpyApi::new());
    let metadata = api.metadata();
    let mut app = base_live_app(api);
    app.auth.options_checked = true;
    app.auth.checked = true;
    app.datasets.metadata = Some(metadata.clone());
    let total_images = metadata.images.len();
    app.datasets.summaries = vec![DatasetSummary {
        dataset_id: metadata.dataset_id,
        name: metadata.name,
        roles: vec![DatasetRole::DataAdmin],
        total_images,
    }];

    app.execute_transition(crate::app::PendingTransition::View(AppView::Admin));
    assert_eq!(app.view, AppView::Admin);
    let UiCommand::LoadAdmin { request, .. } =
        app.runtime.commands.pop_back().expect("admin load command")
    else {
        panic!("expected admin load command");
    };
    app.runtime
        .tx
        .send(UiMessage::AdminLoaded {
            request,
            result: Box::new(Err("admin service unavailable".to_string())),
        })
        .unwrap();
    app.process_messages(&egui::Context::default());
    assert_eq!(app.view, AppView::Admin);
    assert_eq!(
        app.admin_tools.load_error.as_deref(),
        Some("admin service unavailable")
    );

    let harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 780.0))
        .build_eframe(move |_| app);
    assert!(harness.query_by_label("Dataset Admin").is_some());
    assert!(
        harness
            .query_by_label("Admin load failed: admin service unavailable")
            .is_some()
    );
    assert!(harness.query_by_label("Retry admin load").is_some());
    assert!(harness.query_by_label("Retry image load").is_none());
}

#[test]
fn demo_submit_and_skip_advance_images() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1500.0, 780.0))
        .build_eframe(|_| LabelloApp::default());
    assert_eq!(
        harness.state().current.as_ref().unwrap().image.file_name,
        "demo_1.jpg"
    );

    click(&mut harness, "Submit & next");
    assert_eq!(
        harness.state().current.as_ref().unwrap().image.file_name,
        "demo_2.jpg"
    );

    click(&mut harness, "Skip");
    assert_eq!(
        harness.state().current.as_ref().unwrap().image.file_name,
        "demo_3.jpg"
    );

    click(&mut harness, "Skip");
    assert_eq!(
        harness.state().current.as_ref().unwrap().image.file_name,
        "demo_4.jpg"
    );
}

#[test]
fn skip_releases_then_claims_another_assignment() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    let original = harness
        .state()
        .assignment
        .as_ref()
        .unwrap()
        .image_id
        .clone();

    step_until(&mut harness, 12, |app| app.queue.len() == 2);
    let previews_before = api.counts().get_image_preview;
    click(&mut harness, "Skip");
    assert!(
        harness
            .query_by_label("Switch active assignment?")
            .is_none()
    );
    step_until(&mut harness, 16, |app| {
        app.assignment
            .as_ref()
            .is_some_and(|assignment| assignment.image_id != original)
    });

    assert_eq!(api.counts().release_assignment, 1);
    assert_eq!(api.counts().complete_assignment, 0);
    assert_eq!(api.counts().get_image_preview, previews_before);
    assert!(api.exclusions().last().unwrap().contains(&original));
}

#[test]
fn previous_assignment_reopens_the_exact_skipped_image_from_compact_actions() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    harness.set_size(egui::vec2(1500.0, 780.0));
    harness.step();
    let original = harness.state().assignment.clone().unwrap();
    step_until(&mut harness, 12, |app| app.queue.len() == 2);

    click(&mut harness, "Skip");
    step_until(&mut harness, 16, |app| {
        app.assignment
            .as_ref()
            .is_some_and(|assignment| assignment.image_id != original.image_id)
            && app.previous_annotation_assignment.is_some()
    });
    assert!(harness.query_by_label("Previous").is_some());

    harness.set_size(egui::vec2(320.0, 568.0));
    harness.step();
    click(&mut harness, "More actions");
    assert!(
        harness
            .query_by_label_contains("Previous assignment")
            .is_some()
    );
    click_accesskit_button(&mut harness, "Previous assignment");
    step_until(&mut harness, 20, |app| {
        app.assignment
            .as_ref()
            .is_some_and(|assignment| assignment.image_id == original.image_id)
            && !app.loading.image
    });

    assert_eq!(api.counts().reopen_assignment, 1);
    assert_ne!(
        harness.state().assignment.as_ref().unwrap().assignment_id,
        original.assignment_id
    );
    assert!(harness.state().previous_annotation_assignment.is_none());
}

#[test]
fn previous_assignment_reopens_the_exact_submitted_image() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    harness.set_size(egui::vec2(1500.0, 780.0));
    harness.step();
    let original = harness.state().assignment.clone().unwrap();
    step_until(&mut harness, 12, |app| app.queue.len() == 2);

    click(&mut harness, "Submit & next");
    step_until(&mut harness, 16, |app| {
        app.assignment
            .as_ref()
            .is_some_and(|assignment| assignment.image_id != original.image_id)
            && app.previous_annotation_assignment.is_some()
    });
    click(&mut harness, "Previous");
    step_until(&mut harness, 20, |app| {
        app.assignment
            .as_ref()
            .is_some_and(|assignment| assignment.image_id == original.image_id)
            && !app.loading.image
    });

    assert_eq!(api.counts().reopen_assignment, 1);
    assert_ne!(
        harness.state().assignment.as_ref().unwrap().assignment_id,
        original.assignment_id
    );
}

#[test]
fn expired_locally_retained_previous_assignment_is_not_loaded() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    let mut previous = harness.state().assignment.clone().unwrap();
    previous.assignment_id = AssignmentId::generate();
    previous.expires_at = Some(now() - chrono::Duration::seconds(1));
    harness.state_mut().previous_annotation_assignment = Some(previous);

    harness.state_mut().return_to_previous_assignment();

    assert!(harness.state().previous_annotation_assignment.is_none());
    assert_eq!(api.counts().reopen_assignment, 0);
    assert!(
        harness
            .state()
            .runtime
            .error
            .as_deref()
            .is_some_and(|error| error.contains("lease expired"))
    );
}

#[test]
fn skip_remains_active_in_review() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_review_harness(api.clone());

    click(&mut harness, "Skip");
    step_until(&mut harness, 8, |_| api.counts().release_assignment == 1);

    assert_eq!(api.counts().release_assignment, 1);
}

#[test]
fn entering_admin_clears_the_released_assignment() {
    let mut harness = loaded_work_harness(Rc::new(SpyApi::new()));

    click_application_menu_item(&mut harness, "Admin");
    release_and_switch(&mut harness);
    step_until(&mut harness, 12, |app| app.view == AppView::Admin);

    assert!(harness.state().assignment.is_none());
    click(&mut harness, "Annotate");
    assert!(
        harness
            .query_by_label("Switch active assignment?")
            .is_none()
    );
}

#[test]
fn failed_refill_keeps_the_one_shot_image_excluded() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    step_until(&mut harness, 12, |app| app.queue.len() == 2);
    harness.state_mut().queue.pop_prepared();
    let skipped = ImageId::from("img_skipped");
    harness.state_mut().one_shot_excluded_image_id = Some(skipped.clone());
    api.fail_next_preview();

    harness.state_mut().request_prefetch();
    harness.step();
    step_until(&mut harness, 16, |app| app.queue.failed());

    assert_eq!(
        harness.state().one_shot_excluded_image_id.as_ref(),
        Some(&skipped)
    );
    assert!(api.exclusions().last().unwrap().contains(&skipped));
}

#[test]
fn dirty_skip_requires_an_explicit_discard_or_submit_choice() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    click(&mut harness, "Accept");
    assert_eq!(harness.state().save_status, SaveStatus::Dirty);

    click(&mut harness, "Skip");
    assert_eq!(api.counts().release_assignment, 0);
    assert!(
        harness
            .query_by_label("Unsaved annotation changes")
            .is_some()
    );
    assert!(harness.query_by_label("Discard edits and skip").is_some());
    assert!(
        harness
            .query_by_label("Switch active assignment?")
            .is_none()
    );
    assert!(!harness.state().loading.saving);

    let batches = api.counts().annotation_batch;
    harness.state_mut().last_edit_at = Some(Instant::now() - Duration::from_secs(1));
    harness.state_mut().autosave_if_due();
    assert_eq!(harness.state().save_status, SaveStatus::Dirty);
    assert!(!harness.state().loading.saving);
    assert_eq!(api.counts().annotation_batch, batches);

    click_accesskit_button(&mut harness, "Cancel");
    assert!(harness.state().pending_transition.is_none());
    assert_eq!(api.counts().release_assignment, 0);

    harness.state_mut().last_edit_at = Some(Instant::now());
    click(&mut harness, "Skip");
    click_accesskit_button(&mut harness, "Discard edits and skip");
    step_until(&mut harness, 16, |_| api.counts().release_assignment == 1);
}

#[test]
fn dataset_summary_roles_survive_sanitized_metadata_and_show_all_tabs() {
    let api = Rc::new(SpyApi::new());
    api.sanitize_metadata_roles();
    let mut harness = loaded_work_harness(api);

    assert!(
        harness
            .state()
            .datasets
            .metadata
            .as_ref()
            .unwrap()
            .role_assignments
            .is_empty()
    );
    harness.set_size(egui::vec2(600.0, 800.0));
    harness.step();
    click(&mut harness, "More application actions");
    for label in ["Annotate", "Review", "Adjudicate", "Admin", "Statistics"] {
        assert!(
            harness.query_all_by_label(label).next().is_some(),
            "missing authorized {label} destination"
        );
    }
}

#[test]
fn annotator_and_reviewer_roles_are_independent_capabilities() {
    let api = Rc::new(SpyApi::new());
    api.set_summary_roles(vec![DatasetRole::Annotator, DatasetRole::Reviewer]);
    let mut harness = loaded_work_harness(api);

    harness.set_size(egui::vec2(600.0, 800.0));
    harness.step();
    click(&mut harness, "More application actions");
    for label in ["Annotate", "Review", "Statistics"] {
        assert!(
            harness.query_all_by_label(label).next().is_some(),
            "missing authorized {label} destination"
        );
    }
    for label in ["Adjudicate", "Admin"] {
        assert!(
            harness.query_all_by_label(label).next().is_none(),
            "unexpected unauthorized {label} destination"
        );
    }
}

#[test]
fn reviewer_only_workspace_does_not_fetch_prelabels() {
    let api = Rc::new(SpyApi::new());
    let mut harness = live_harness(api.clone());
    step_until(&mut harness, 8, |app| app.datasets.summaries.len() == 1);
    harness
        .state_mut()
        .open_dataset(DatasetId::from("demo"), AppView::Review);
    step_until(&mut harness, 12, |app| {
        app.view == AppView::Review && app.current.is_some()
    });

    assert_eq!(api.counts().prelabel_suggestions, 0);
    assert!(harness.query_by_label("Prelabels").is_none());
}

#[test]
fn stale_assignment_operations_do_not_clear_the_active_loading_owner() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);
    let assignment = harness.state().assignment.clone().unwrap();
    let state = harness.state().current_state.clone().unwrap();
    harness.state_mut().active_operation_id = Some(77);
    harness.state_mut().loading.saving = true;
    harness.state_mut().runtime.active_requests.insert(77);
    harness
        .state()
        .runtime
        .tx
        .send(UiMessage::SaveFinished {
            request: test_request(harness.state(), 76, Some("demo")),
            operation_id: 76,
            assignment_id: assignment.assignment_id.clone(),
            edit_generation: 0,
            completed: false,
            result: Box::new(Ok(state.clone())),
        })
        .unwrap();
    harness.step();
    assert!(harness.state().loading.saving);
    assert_eq!(harness.state().active_operation_id, Some(77));

    harness
        .state()
        .runtime
        .tx
        .send(UiMessage::SaveFinished {
            request: test_request(harness.state(), 77, Some("demo")),
            operation_id: 77,
            assignment_id: assignment.assignment_id,
            edit_generation: 0,
            completed: false,
            result: Box::new(Ok(state)),
        })
        .unwrap();
    harness.step();
    assert!(!harness.state().loading.saving);
    assert_eq!(harness.state().active_operation_id, None);
}

#[test]
fn desktop_app_bar_shows_direct_navigation_and_accessible_icon_actions() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);
    harness.set_size(egui::vec2(1500.0, 780.0));
    harness.step();

    for label in ["Annotate", "Review", "Adjudicate", "Statistics", "Admin"] {
        assert_control_inside(
            &harness,
            label,
            egui::accesskit::Role::Button,
            1500.0,
            780.0,
        );
    }
    assert!(harness.query_by_label("More application actions").is_none());
    assert!(harness.query_by_label("Navigation").is_none());
    assert!(harness.query_by_label("Workspace").is_none());
    assert!(harness.query_by_label("Desktop navigation").is_none());

    for label in ["Open setup", "Open tutorial", "Open settings", "Sign out"] {
        let action = harness
            .get_by_role_and_label(egui::accesskit::Role::Button, label)
            .rect();
        assert!(action.width() >= 43.0 && action.height() >= 43.0);
        assert!(
            action.width() <= 45.0 && (action.width() - action.height()).abs() <= 1.0,
            "{label} is not square: {action:?}",
        );
    }
    assert!(harness.get_by_label("Admin User").rect().width() <= 96.5);
}

#[test]
fn responsive_workspace_has_one_action_set_and_a_usable_canvas() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);
    let image_name = harness
        .state()
        .current
        .as_ref()
        .unwrap()
        .image
        .file_name
        .clone();
    let workflow_label = harness.state().selected_workflow().unwrap().label();
    let sizes = viewport_sizes();
    let mut boundary_widths = Vec::new();
    for (width, height) in sizes {
        harness.set_size(egui::vec2(width, height));
        harness.step();
        assert!(harness.query_all_by_label(&image_name).next().is_some());
        assert!(harness.query_all_by_label(&workflow_label).next().is_some());
        let dataset_badge = harness.get_by_label("Dataset Demo Dataset").rect();
        let status_badge = harness.get_by_label("Status: Idle").rect();
        assert!(
            dataset_badge.height() > 0.0,
            "dataset badge is missing at {width}x{height}",
        );
        assert!(status_badge.height() > 0.0);
        let layout = LayoutMode::for_width(width);
        let menu = harness
            .query_by_label("More application actions")
            .or_else(|| {
                harness
                    .query_all_by_role_and_label(egui::accesskit::Role::Button, "Annotate")
                    .next()
            })
            .expect("top-bar navigation control")
            .rect();
        assert!((dataset_badge.center().x - width / 2.0).abs() <= 1.0);
        assert!(menu.right() <= dataset_badge.left() + 0.5);
        assert!(dataset_badge.right() <= status_badge.left() + 0.5);
        assert!(
            menu.left() <= 25.0,
            "application menu is not left-aligned at {width}x{height}: {menu:?}",
        );
        if layout == LayoutMode::Wide {
            let sign_out = harness.get_by_label("Sign out").rect();
            assert!(status_badge.right() <= sign_out.left() + 0.5);
        }
        let canvas = harness.get_by_label("Annotation canvas");
        let app_bar = harness.get_by_label("Application bar").rect();
        let context_bar = harness.get_by_label("Workspace context bar").rect();
        assert!(app_bar.bottom() <= context_bar.top() + 0.5);
        assert!(context_bar.bottom() <= canvas.rect().top() + 0.5);
        let minimum_width = if width < 600.0 { width - 40.0 } else { 560.0 };
        let minimum_height = match height as u32 {
            568 => 210.0,
            667 => 290.0,
            768 => 390.0,
            800 => 420.0,
            820 => 450.0,
            _ => 620.0,
        };
        assert!(
            canvas.rect().width() >= minimum_width,
            "canvas too narrow at {width}x{height}: {:?}",
            canvas.rect(),
        );
        assert!(
            canvas.rect().height() >= minimum_height,
            "canvas too short at {width}x{height}: {:?}",
            canvas.rect(),
        );
        let wide_baseline = match (width as u32, height as u32) {
            (1288, 820) => Some((668.0, 593.0)),
            (1366, 768) => Some((746.0, 541.0)),
            (1440, 900) => Some((820.0, 673.0)),
            _ => None,
        };
        if let Some((baseline_width, baseline_height)) = wide_baseline {
            assert!(
                canvas.rect().width() >= baseline_width,
                "canvas narrower than baseline at {width}x{height}: {:?}",
                canvas.rect(),
            );
            assert!(
                canvas.rect().height() >= baseline_height,
                "canvas shorter than baseline at {width}x{height}: {:?}",
                canvas.rect(),
            );
        }
        if width < 600.0 {
            assert_control_inside(
                &harness,
                "Submit & next",
                egui::accesskit::Role::Button,
                width,
                height,
            );
            assert_control_inside(
                &harness,
                "More actions",
                egui::accesskit::Role::Button,
                width,
                height,
            );
        } else {
            for label in ["Save", "Submit & next", "Skip"] {
                assert_eq!(
                    harness
                        .query_all_by_label_contains(label)
                        .filter(|node| {
                            node.accesskit_node().role() == egui::accesskit::Role::Button
                        })
                        .count(),
                    1,
                    "duplicate {label} controls at {width}"
                );
                assert_control_inside(
                    &harness,
                    label,
                    egui::accesskit::Role::Button,
                    width,
                    height,
                );
            }
        }
        if width == 1239.0 || width == 1240.0 {
            boundary_widths.push(canvas.rect().width());
        }
    }
    assert_eq!(boundary_widths.len(), 2);
    assert!((boundary_widths[0] - boundary_widths[1]).abs() <= 2.0);

    harness.set_size(egui::vec2(320.0, 568.0));
    for (status, label) in [
        (SaveStatus::Dirty, "Unsaved"),
        (SaveStatus::Saved, "Saved"),
        (SaveStatus::Saving, "Saving"),
        (SaveStatus::Retry, "Retry"),
    ] {
        harness.state_mut().save_status = status;
        harness.step();
        let status_label = format!("Status: {label}");
        assert!(harness.query_by_label(&status_label).is_some());
        assert_visible_controls_clamped(&harness, 320.0, 568.0);
    }

    assert_eq!(
        harness
            .query_all_by_role_and_label(egui::accesskit::Role::Button, "More application actions")
            .count(),
        1
    );
    click(&mut harness, "More application actions");
    assert!(harness.query_by_label("Navigation").is_none());
    assert!(harness.query_by_label("Workspace").is_none());
    assert!(harness.query_by_label("Status").is_none());
    for label in [
        "Setup",
        "Annotate",
        "Review",
        "Adjudicate",
        "Statistics",
        "Admin",
        "Tutorial",
        "Settings",
        "Sign out",
    ] {
        let item = harness
            .get_by_role_and_label(egui::accesskit::Role::Button, label)
            .rect();
        assert!(item.height() >= 43.0);
        assert!(
            item.width() >= 200.0,
            "{label} has narrow menu bounds: {item:?}"
        );
    }
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Sign out")
        .scroll_to_me();
    for _ in 0..4 {
        harness.step();
    }
    assert_control_inside(
        &harness,
        "Sign out",
        egui::accesskit::Role::Button,
        320.0,
        568.0,
    );
    assert_visible_controls_clamped(&harness, 320.0, 568.0);
    harness.key_press(egui::Key::Escape);
    harness.step();

    harness.set_size(egui::vec2(320.0, 320.0));
    harness.step();
    let canvas = harness.get_by_label("Annotation canvas").rect();
    assert!(canvas.top() >= 0.0 && canvas.bottom() <= 320.0 && canvas.height() >= 100.0);
    for label in [
        "Pan",
        "Zoom out",
        "Zoom in",
        "Fit",
        "Submit & next",
        "More actions",
    ] {
        assert_control_inside(&harness, label, egui::accesskit::Role::Button, 320.0, 320.0);
    }
}

#[test]
fn compact_long_work_context_preserves_canvas_and_controls() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);
    harness
        .state_mut()
        .current
        .as_mut()
        .unwrap()
        .image
        .file_name =
        "a-very-long-image-name-that-must-not-collapse-the-annotation-workspace.jpg".to_string();
    harness
        .state_mut()
        .tasks
        .iter_mut()
        .find(|task| task.task_id == TaskId::from("bounding_box:person"))
        .unwrap()
        .name = "A deliberately long workflow name for compact layout testing".to_string();

    for (width, height, minimum_canvas_height) in [
        (320.0, 568.0, 200.0),
        (390.0, 667.0, 320.0),
        (390.0, 844.0, 500.0),
    ] {
        harness.set_size(egui::vec2(width, height));
        harness.step();

        let canvas = harness.get_by_label("Annotation canvas").rect();
        assert!(
            canvas.height() >= minimum_canvas_height,
            "canvas too short at {width}x{height}: {canvas:?}",
        );
        for label in ["Pan", "Zoom out", "Zoom in", "Fit"] {
            assert_control_inside(
                &harness,
                label,
                egui::accesskit::Role::Button,
                width,
                height,
            );
        }
        let workflow = harness
            .get_by_label("A deliberately long workflow name for compact layout testing")
            .rect();
        assert!(
            workflow.height() <= 44.0,
            "workflow badge wrapped vertically at {width}x{height}: {workflow:?}",
        );
    }
}

#[test]
fn tutorial_overlay_does_not_change_canvas_geometry() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);
    harness.set_size(egui::vec2(390.0, 667.0));
    harness.step();
    let before = harness.get_by_label("Annotation canvas").rect();
    let selected = harness.state().selected_task_id.clone().unwrap();
    harness
        .state_mut()
        .tasks
        .iter_mut()
        .find(|task| task.task_id == selected)
        .unwrap()
        .instructions
        .example_text = "Detailed tutorial guidance. ".repeat(100);

    harness.state_mut().show_tutorial = true;
    harness.step();

    assert_eq!(harness.get_by_label("Annotation canvas").rect(), before);
    let tutorial = harness.get_by_label("Tutorial").rect();
    let context = harness.get_by_label("Workspace context bar").rect();
    assert!(tutorial.top() >= context.bottom());
    assert!(tutorial.bottom() <= 667.0);
}

#[test]
fn setup_geometry_stays_clamped_at_supported_viewports() {
    let api = Rc::new(SpyApi::new());
    let mut harness = live_harness(api);
    step_until(&mut harness, 8, |app| !app.datasets.summaries.is_empty());
    harness.state_mut().config.dataset_id = DatasetId::from(
        "a-very-long-dataset-name-that-must-be-truncated-without-growing-the-shell",
    );

    for (width, height) in viewport_sizes() {
        harness.set_size(egui::vec2(width, height));
        harness.step();
        assert_label_inside(&harness, "Choose where to work", width, height);
        assert!(harness.query_by_label("Desktop navigation").is_none());
        let setup_control = if harness.query_by_label("Open setup").is_some() {
            "Open setup"
        } else {
            "More application actions"
        };
        assert_control_inside(
            &harness,
            setup_control,
            egui::accesskit::Role::Button,
            width,
            height,
        );
        assert_visible_controls_clamped(&harness, width, height);
    }

    harness.set_size(egui::vec2(1440.0, 320.0));
    harness.step();
    assert_control_inside(
        &harness,
        "Sign out",
        egui::accesskit::Role::Button,
        1440.0,
        320.0,
    );
    assert_visible_controls_clamped(&harness, 1440.0, 320.0);

    for width in [320.0, 600.0] {
        harness.set_size(egui::vec2(width, 320.0));
        harness.step();
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Continue with Demo Dataset")
            .scroll_to_me();
        for _ in 0..4 {
            harness.step();
        }
        assert_control_inside(
            &harness,
            "Continue with Demo Dataset",
            egui::accesskit::Role::Button,
            width,
            320.0,
        );
    }
}

#[test]
fn review_correction_drawer_and_actions_stay_reachable() {
    let api = Rc::new(SpyApi::new());
    seed_review_annotation(
        &api,
        AnnotationGeometry::BoundingBox(BoundingBox {
            x: 0.2,
            y: 0.2,
            width: 0.3,
            height: 0.3,
        }),
        true,
    );
    let mut harness = loaded_review_harness(api);
    harness.state_mut().start_correction();

    for (width, height) in viewport_sizes() {
        harness.state_mut().drawer =
            (LayoutMode::for_width(width) != LayoutMode::Wide).then_some(Drawer::Inspector);
        harness.set_size(egui::vec2(width, height));
        harness.step();
        assert_canvas_geometry(&harness, width, height);
        for label in ["Object", "Reason", "Actions"] {
            assert!(
                harness.query_by_label(label).is_some(),
                "missing correction section {label} at {width}x{height}"
            );
        }
        assert!(
            harness
                .query_by_role_and_label(
                    egui::accesskit::Role::MultilineTextInput,
                    "Reason (optional)",
                )
                .is_some()
        );
        let finalize =
            harness.get_by_role_and_label(egui::accesskit::Role::Button, "Correct & finalize");
        finalize.scroll_to_me();
        for _ in 0..4 {
            harness.step();
        }
        assert_control_inside(
            &harness,
            "Correct & finalize",
            egui::accesskit::Role::Button,
            width,
            height,
        );
        assert_visible_controls_clamped(&harness, width, height);
    }

    for (width, height) in [(320.0, 320.0), (600.0, 568.0), (600.0, 320.0)] {
        harness.state_mut().drawer = Some(Drawer::Inspector);
        harness.set_size(egui::vec2(width, height));
        harness.step();
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Correct & finalize")
            .scroll_to_me();
        for _ in 0..8 {
            harness.step();
        }
        assert_control_inside(
            &harness,
            "Correct & finalize",
            egui::accesskit::Role::Button,
            width,
            height,
        );
    }
}

#[test]
fn review_primary_decisions_stay_visible_at_supported_viewports() {
    let api = Rc::new(SpyApi::new());
    seed_review_annotation(
        &api,
        AnnotationGeometry::BoundingBox(BoundingBox {
            x: 0.2,
            y: 0.2,
            width: 0.3,
            height: 0.3,
        }),
        false,
    );
    let mut harness = loaded_review_harness(api);

    click(&mut harness, "Zoom in");
    assert!(harness.state().canvas.current_zoom() > 1.0);
    let pan_before = harness.get_by_label("Pan").rect();
    let zoom_out_before = harness.get_by_label("Zoom out").rect();
    click(&mut harness, "Pan");
    assert!(harness.state().canvas.pan_mode());
    assert_eq!(harness.get_by_label("Pan").rect(), pan_before);
    assert_eq!(harness.get_by_label("Zoom out").rect(), zoom_out_before);
    click(&mut harness, "Pan");
    assert!(!harness.state().canvas.pan_mode());
    click(&mut harness, "Fit");
    assert_eq!(harness.state().canvas.current_zoom(), 1.0);
    harness.key_press(egui::Key::Plus);
    harness.step();
    assert!(harness.state().canvas.current_zoom() > 1.0);
    click(&mut harness, "Fit");

    for (width, height) in viewport_sizes() {
        harness.set_size(egui::vec2(width, height));
        harness.step();
        assert_label_inside(&harness, "Object 1 of 1", width, height);
        for label in ["Pan", "Zoom out", "Zoom in", "Fit"] {
            assert_control_inside(
                &harness,
                label,
                egui::accesskit::Role::Button,
                width,
                height,
            );
        }
        let (approve, reject) = ("Approve object", "Reject object & finish");
        for label in [approve, reject] {
            assert_control_inside(
                &harness,
                label,
                egui::accesskit::Role::Button,
                width,
                height,
            );
        }
        if LayoutMode::for_width(width) != LayoutMode::Wide {
            harness.state_mut().drawer = Some(Drawer::Inspector);
            harness.step();
            assert_eq!(
                harness
                    .query_all_by_role_and_label(egui::accesskit::Role::Button, approve)
                    .count(),
                1,
                "review action duplicated when the Inspector drawer opened"
            );
            harness.state_mut().drawer = None;
        }
    }

    harness.set_size(egui::vec2(320.0, 320.0));
    harness.step();
    for label in [
        "Pan",
        "Zoom out",
        "Zoom in",
        "Fit",
        "Approve object",
        "Reject object & finish",
        "More",
    ] {
        assert_control_inside(&harness, label, egui::accesskit::Role::Button, 320.0, 320.0);
    }

    harness.state_mut().review_index = 1;
    harness.set_size(egui::vec2(320.0, 568.0));
    harness.step();
    assert_label_inside(&harness, "Final check", 320.0, 568.0);
}

#[test]
fn adjudication_primary_decisions_stay_visible_at_supported_viewports() {
    let api = Rc::new(SpyApi::new());
    seed_review_annotation(
        &api,
        AnnotationGeometry::BoundingBox(BoundingBox {
            x: 0.2,
            y: 0.2,
            width: 0.3,
            height: 0.3,
        }),
        false,
    );
    let mut harness = loaded_adjudication_harness(api);

    click(&mut harness, "Zoom in");
    assert!(harness.state().canvas.current_zoom() > 1.0);
    click(&mut harness, "Fit");
    assert_eq!(harness.state().canvas.current_zoom(), 1.0);

    for (width, height) in viewport_sizes() {
        harness.set_size(egui::vec2(width, height));
        harness.step();
        for label in ["Pan", "Zoom out", "Zoom in", "Fit"] {
            assert_control_inside(
                &harness,
                label,
                egui::accesskit::Role::Button,
                width,
                height,
            );
        }
        let (accept, correct) = if LayoutMode::for_width(width) == LayoutMode::Compact {
            ("Accept all", "Send back")
        } else {
            ("Accept all annotations", "Send back for correction")
        };
        for label in [accept, correct] {
            assert_control_inside(
                &harness,
                label,
                egui::accesskit::Role::Button,
                width,
                height,
            );
        }
        if LayoutMode::for_width(width) != LayoutMode::Wide {
            harness.state_mut().drawer = Some(Drawer::Inspector);
            harness.step();
            assert_eq!(
                harness
                    .query_all_by_role_and_label(egui::accesskit::Role::Button, accept)
                    .count(),
                1,
                "adjudication action duplicated when the Inspector drawer opened"
            );
            harness.state_mut().drawer = None;
        }
    }
}

#[test]
fn admin_geometry_keeps_compact_save_and_discard_in_the_header() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_admin_harness(api);
    harness
        .state_mut()
        .datasets
        .admin_config
        .as_mut()
        .unwrap()
        .name = String::new();

    for (width, height) in viewport_sizes() {
        harness.set_size(egui::vec2(width, height));
        harness.step();
        assert_label_inside(&harness, "Dataset Admin", width, height);
        let description = harness
            .get_by_label("Manage access, inspect images, and configure labeling workflows.")
            .rect();
        for label in ["Save Admin changes", "Discard staged changes"] {
            assert_control_inside(
                &harness,
                label,
                egui::accesskit::Role::Button,
                width,
                height,
            );
            let action = harness
                .get_by_role_and_label(egui::accesskit::Role::Button, label)
                .rect();
            assert!(
                action.width() <= 45.0,
                "{label} is not icon-sized: {action:?}"
            );
            assert!((action.width() - action.height()).abs() <= 1.0);
            if LayoutMode::for_width(width) != LayoutMode::Wide {
                assert!(
                    !action.intersects(description),
                    "{label} overlaps the Admin description at {width}x{height}: action={action:?}, description={description:?}"
                );
            }
        }
        assert_label_inside(
            &harness,
            "Configuration changes staged; 1 validation error(s)",
            width,
            height,
        );
        assert!(harness.query_by_label("Unsaved admin changes").is_none());
        assert_visible_controls_clamped(&harness, width, height);
    }
}

#[test]
fn stats_geometry_keeps_header_actions_and_equal_cards_in_view() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);
    harness.state_mut().clear_current_image();
    harness.state_mut().view = AppView::Stats;
    harness.state_mut().request_stats();
    step_until(&mut harness, 8, |app| !app.loading.stats);

    for (width, height) in viewport_sizes() {
        harness.set_size(egui::vec2(width, height));
        harness.step();
        assert_label_inside(&harness, "Live Statistics", width, height);
        assert_control_inside(
            &harness,
            "Refresh now",
            egui::accesskit::Role::Button,
            width,
            height,
        );
        let cards = ["Metric Images", "Metric Completed", "Metric Pending"]
            .map(|label| harness.get_by_label(label).rect());
        if width >= 600.0 {
            assert!(
                (cards[0].width() - cards[1].width()).abs() <= 2.0,
                "metric cards are not equal at {width}x{height}: {cards:?}",
            );
        }
        if LayoutMode::for_width(width) == LayoutMode::Compact {
            assert!(harness.query_by_label("Person boxes").is_some());
            let rows = [
                "Pending: 1  Unreviewed: 1",
                "Approved: 1  Rejected: 0",
                "Finalized: 1  Done: 1",
            ]
            .map(|label| {
                harness
                    .query_by_label_contains(label)
                    .unwrap_or_else(|| panic!("missing compact task statistics row {label}"))
                    .rect()
                    .top()
            });
            assert!(
                rows.windows(2).all(|pair| pair[0] < pair[1]),
                "compact task statistics do not follow workflow order: {rows:?}"
            );
        } else {
            assert!(harness.query_by_label("Done").is_some());
            assert!(harness.query_by_label("Completed tasks").is_some());
            if LayoutMode::for_width(width) == LayoutMode::Wide {
                let header_y = harness.get_by_label("Done").rect().center().y;
                let columns = [
                    "Pending",
                    "Unreviewed",
                    "Reviewed",
                    "Approved",
                    "Rejected",
                    "Corrected",
                    "Finalized",
                    "Done",
                ]
                .map(|label| {
                    harness
                        .query_all_by_label(label)
                        .find(|node| (node.rect().center().y - header_y).abs() <= 1.0)
                        .unwrap_or_else(|| panic!("missing task statistics column {label}"))
                        .rect()
                        .left()
                });
                assert!(
                    columns.windows(2).all(|pair| pair[0] < pair[1]),
                    "task statistics columns do not follow workflow order: {columns:?}"
                );
            }
        }
        assert_visible_controls_clamped(&harness, width, height);
    }
}

#[test]
fn stats_tables_render_all_rows_without_nested_vertical_scrolling() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);
    harness.state_mut().clear_current_image();
    harness.state_mut().view = AppView::Stats;
    harness.state_mut().request_stats();
    step_until(&mut harness, 8, |app| !app.loading.stats);

    let task_stats = harness
        .state()
        .datasets
        .stats
        .per_task
        .values()
        .next()
        .unwrap()
        .clone();
    let class_stats = harness
        .state()
        .datasets
        .stats
        .per_class
        .values()
        .next()
        .unwrap()
        .clone();
    for index in 0..10 {
        harness
            .state_mut()
            .datasets
            .stats
            .per_task
            .insert(TaskId::from(format!("zz-task-{index}")), task_stats.clone());
        harness.state_mut().datasets.stats.per_class.insert(
            ClassId::from(format!("zz-class-{index}")),
            class_stats.clone(),
        );
    }
    harness.set_size(egui::vec2(1440.0, 1600.0));
    harness.step();

    assert!(harness.query_by_label("zz-task-9").is_some());
    assert!(harness.query_by_label("zz-class-9").is_some());
}

#[test]
fn settings_and_transition_modals_are_viewport_constrained() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);
    for (width, height) in viewport_sizes() {
        harness.set_size(egui::vec2(width, height));
        harness.state_mut().show_settings = true;
        harness.step();
        harness.step();
        assert_label_inside(&harness, "Keyboard shortcuts", width, height);
        assert_visible_controls_clamped(&harness, width, height);
        if width == 320.0 {
            let draft = harness
                .state_mut()
                .shortcut_settings
                .draft
                .as_mut()
                .expect("settings draft");
            let chord = draft.bindings[&labello_domain::UserAction::UndoEdit].clone();
            draft
                .bindings
                .insert(labello_domain::UserAction::RedoEdit, chord);
            harness.step();
            harness.step();
            assert!(
                harness
                    .query_by_label("Resolve 1 shortcut conflict(s) before saving.")
                    .is_some()
            );
            for label in ["Restore all defaults", "Cancel", "Save changes"] {
                assert_control_inside(
                    &harness,
                    label,
                    egui::accesskit::Role::Button,
                    width,
                    height,
                );
            }
            assert_visible_controls_clamped(&harness, width, height);
        }

        harness.state_mut().show_settings = false;
        harness.state_mut().pending_transition =
            Some(crate::app::PendingTransition::View(AppView::Review));
        harness.step();
        for label in ["Release and switch", "Cancel"] {
            assert_control_inside(
                &harness,
                label,
                egui::accesskit::Role::Button,
                width,
                height,
            );
        }
        assert_visible_controls_clamped(&harness, width, height);
        harness.state_mut().pending_transition = None;
    }

    harness.set_size(egui::vec2(600.0, 568.0));
    harness.state_mut().show_settings = true;
    harness.step();
    harness.step();
    for label in ["Restore all defaults", "Cancel", "Save changes"] {
        assert_control_inside(&harness, label, egui::accesskit::Role::Button, 600.0, 568.0);
    }
    assert_visible_controls_clamped(&harness, 600.0, 568.0);

    for width in [320.0, 600.0] {
        harness.set_size(egui::vec2(width, 320.0));
        harness.step();
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Save changes")
            .scroll_to_me();
        for _ in 0..8 {
            harness.step();
        }
        for label in ["Restore all defaults", "Cancel", "Save changes"] {
            assert_control_inside(&harness, label, egui::accesskit::Role::Button, width, 320.0);
        }
    }
}

#[test]
fn responsive_modes_do_not_switch_at_1240() {
    assert_eq!(LayoutMode::for_width(599.0), LayoutMode::Compact);
    assert_eq!(LayoutMode::for_width(600.0), LayoutMode::Medium);
    assert_eq!(LayoutMode::for_width(1239.0), LayoutMode::Medium);
    assert_eq!(LayoutMode::for_width(1240.0), LayoutMode::Medium);
    assert_eq!(LayoutMode::for_width(1287.0), LayoutMode::Medium);
    assert_eq!(LayoutMode::for_width(1288.0), LayoutMode::Wide);
    assert_eq!(LayoutMode::for_width(1366.0), LayoutMode::Wide);
}

#[test]
fn work_workflow_draws_saves_submits_reviews_and_adjudicates() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    assert!(harness.state().current.is_some());
    assert_eq!(harness.state().queue.queue_size(), IMAGE_QUEUE_SIZE);
    assert!(harness.query_by_label("Assignment").is_some());
    assert!(harness.query_by_label("Approve object").is_none());
    assert!(harness.query_by_label("Reject object & finish").is_none());
    assert!(harness.query_by_label("Accept all annotations").is_none());

    click_application_menu_item(&mut harness, "Tutorial");
    harness.step();
    assert!(
        harness
            .query_by_label("Label every visible person")
            .is_some()
    );

    click(&mut harness, "Accept");
    harness.step();
    assert_eq!(harness.state().annotations.len(), 1);
    assert_eq!(
        harness.state().selected_annotation.as_ref(),
        Some(&harness.state().annotations[0].annotation_id)
    );
    assert_eq!(harness.state().save_status, SaveStatus::Dirty);

    let canvas = harness.get_by_label("Annotation canvas");
    let rect = canvas.rect();
    assert!(rect.width() > 100.0 && rect.height() > 100.0);
    let start = rect.left_top() + rect.size() * 0.55;
    let end = rect.left_top() + rect.size() * 0.82;
    harness.drag_at(start);
    harness.step();
    harness.hover_at(end);
    harness.step();
    harness.drop_at(end);
    harness.step();
    assert_eq!(harness.state().annotations.len(), 2);

    click(&mut harness, "Save");
    step_until(&mut harness, 10, |app| app.save_status == SaveStatus::Saved);
    let counts = api.counts();
    assert!(counts.append_event >= 2);
    assert_eq!(counts.annotation_batch, 1);
    assert_eq!(counts.rebuild_image, 0);

    click(&mut harness, "Submit & next");
    assert!(
        harness
            .query_by_label("Switch active assignment?")
            .is_none()
    );
    step_until(&mut harness, 10, |app| {
        app.current
            .as_ref()
            .is_some_and(|current| current.image.image_id == ImageId::from("img_2"))
    });
    assert_eq!(api.counts().complete_assignment, 1);

    assert!(api.counts().assign_next_image >= 2);

    harness.state_mut().drawer = Some(Drawer::Inspector);
    click_application_menu_item(&mut harness, "Review");
    assert!(
        harness
            .query_by_label("Switch active assignment?")
            .is_some()
    );
    release_and_switch(&mut harness);
    step_until(&mut harness, 10, |app| {
        app.view == AppView::Review && !app.loading.image
    });
    assert!(harness.state().drawer.is_none());
    assert!(harness.query_by_label("Tutorial").is_none());
    assert!(harness.query_by_label("Approve object").is_some());
    assert!(harness.query_by_label("Reject object & finish").is_some());
    assert!(harness.query_by_label("Accept").is_none());
    harness.key_press(egui::Key::Y);
    harness.step();
    step_until(&mut harness, 10, |app| !app.loading.saving);
    assert_eq!(api.counts().record_review, 1);

    click(&mut harness, "Reject object & finish");
    step_until(&mut harness, 10, |app| !app.loading.saving);
    // Rejecting an object records the object decision and the task-level
    // correction outcome that closes the review assignment.
    assert_eq!(api.counts().record_review, 3);
    step_until(&mut harness, 10, |app| {
        app.assignment
            .as_ref()
            .is_some_and(|assignment| api.has_active_assignment(&assignment.assignment_id))
    });

    click_application_menu_item(&mut harness, "Adjudicate");
    assert!(
        harness
            .query_by_label("Switch active assignment?")
            .is_some()
    );
    release_and_switch(&mut harness);
    step_until(&mut harness, 10, |app| {
        app.view == AppView::Adjudicate && !app.loading.image
    });
    assert!(harness.query_by_label("Accept all annotations").is_some());
    assert!(harness.query_by_label("Send back for correction").is_some());
    assert!(harness.query_by_label("Approve object").is_none());
    click(&mut harness, "Accept all annotations");
    step_until(&mut harness, 10, |app| !app.loading.saving);
    assert_eq!(api.counts().record_adjudication, 1);

    let claims_before_arrow = api.counts().assign_next_image;
    harness.key_press(egui::Key::ArrowRight);
    step_until(&mut harness, 10, |app| !app.loading.image);
    assert_eq!(api.counts().assign_next_image, claims_before_arrow);
}

#[test]
fn dirty_workflow_changes_save_before_loading_the_new_assignment() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    let original_image = harness
        .state()
        .current
        .as_ref()
        .unwrap()
        .image
        .image_id
        .clone();

    click(&mut harness, "Accept");
    assert_eq!(harness.state().save_status, SaveStatus::Dirty);
    click(&mut harness, "Vehicle boxes");
    assert!(
        harness
            .query_by_label("Switch active assignment?")
            .is_some()
    );
    assert_eq!(
        harness.state().selected_task_id.as_ref(),
        Some(&TaskId::from("bounding_box:person"))
    );
    harness.state_mut().submit_pending_transition();
    harness.step();
    step_until(&mut harness, 12, |app| {
        app.selected_class_id() == Some(&ClassId::from("vehicle"))
            && app.current.is_some()
            && !app.loading.saving
    });

    assert!(api.counts().append_event >= 1);
    assert_eq!(api.counts().complete_assignment, 1);
    assert_ne!(
        harness.state().current.as_ref().unwrap().image.image_id,
        original_image
    );
}

#[test]
fn editing_a_persisted_box_saves_a_new_annotation_version() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    click(&mut harness, "Accept");
    click(&mut harness, "Save");
    step_until(&mut harness, 10, |app| app.save_status == SaveStatus::Saved);

    let annotation_id = harness.state().annotations[0].annotation_id.clone();
    let origin = harness.state().annotations[0].origin.clone();
    let object_group_id = harness.state().annotations[0].object_group_id.clone();
    harness.state_mut().edit_bbox(BoundingBoxEdit {
        annotation_id: annotation_id.clone(),
        bounding_box: BoundingBox {
            x: 0.2,
            y: 0.25,
            width: 0.3,
            height: 0.35,
        },
    });
    assert_eq!(harness.state().annotations[0].version, 2);
    assert_eq!(harness.state().annotations[0].origin, origin);
    assert_eq!(
        harness.state().annotations[0].object_group_id,
        object_group_id
    );
    assert!(matches!(
        harness.state().annotations[0].revision_source,
        RevisionSource::Human {
            action: HumanRevisionKind::Edited
        }
    ));
    assert_eq!(
        harness.state().annotations[0].author_user_id,
        UserId::from("admin")
    );
    assert!(matches!(
        origin,
        AnnotationOrigin::Native { legacy_v2: false }
    ));
    harness.state_mut().autosave();
    step_until(&mut harness, 10, |app| app.save_status == SaveStatus::Saved);

    assert!(api.events().iter().any(|payload| matches!(
        payload,
        EventPayload::AnnotationVersionCreated {
            annotation,
            previous_version: Some(1),
            ..
        } if annotation.annotation_id == annotation_id && annotation.version == 2
    )));
}

#[test]
fn skeleton_workflow_places_configured_keypoints_in_order() {
    let api = Rc::new(SpyApi::new());
    {
        let mut state = api.state.borrow_mut();
        let task = &mut state.metadata.tasks[0];
        task.annotation_type = AnnotationType::Skeleton;
        task.prelabel_config_ids.clear();
        task.skeleton = Some(SkeletonSpec {
            keypoints: vec![
                KeypointSpec {
                    name: "head".to_string(),
                    required: true,
                },
                KeypointSpec {
                    name: "tail".to_string(),
                    required: true,
                },
            ],
            edges: Vec::new(),
            allow_hidden: true,
            allow_absent: false,
        });
    }
    let mut harness = loaded_work_harness(api);
    let canvas = harness.get_by_label("Annotation canvas");
    let rect = canvas.rect();
    click_at(&mut harness, rect.center());
    click_at(
        &mut harness,
        rect.center() + egui::vec2(rect.width() * 0.15, rect.height() * 0.1),
    );

    assert_eq!(harness.state().annotations.len(), 1);
    let AnnotationGeometry::Skeleton(skeleton) = &harness.state().annotations[0].geometry else {
        panic!("expected skeleton annotation");
    };
    assert_eq!(skeleton.keypoints.len(), 2);
    assert!(
        skeleton
            .keypoints
            .iter()
            .all(|keypoint| keypoint.point.is_some())
    );
    assert!(harness.state().active_skeleton.is_none());
}

#[test]
fn reviewer_correction_controls_follow_task_config_and_keep_an_isolated_bbox_draft() {
    let disabled_api = Rc::new(SpyApi::new());
    seed_review_annotation(
        &disabled_api,
        AnnotationGeometry::BoundingBox(BoundingBox {
            x: 0.2,
            y: 0.2,
            width: 0.3,
            height: 0.3,
        }),
        false,
    );
    let disabled = loaded_review_harness(disabled_api);
    assert!(disabled.query_by_label("Correct object").is_none());

    let api = Rc::new(SpyApi::new());
    let original = BoundingBox {
        x: 0.2,
        y: 0.2,
        width: 0.3,
        height: 0.3,
    };
    let annotation_id =
        seed_review_annotation(&api, AnnotationGeometry::BoundingBox(original), true);
    let mut harness = loaded_review_harness(api.clone());
    click(&mut harness, "Correct object");
    harness.state_mut().edit_correction_bbox(BoundingBoxEdit {
        annotation_id,
        bounding_box: BoundingBox {
            x: 0.3,
            y: 0.25,
            width: 0.25,
            height: 0.35,
        },
    });
    harness.step();

    assert!(
        harness
            .state()
            .correction_draft
            .as_ref()
            .unwrap()
            .geometry_changed()
    );
    assert!(matches!(
        harness.state().annotations[0].geometry,
        AnnotationGeometry::BoundingBox(box_geometry) if box_geometry == original
    ));
    assert_eq!(api.counts().annotation_batch, 0);
    harness
        .state_mut()
        .request_review(labello_domain::ReviewDecision::Approved);
    assert_eq!(api.counts().record_review, 0);
    assert!(harness.state().correction_draft.is_some());

    api.fail_next_correction();
    click(&mut harness, "Correct & finalize");
    step_until(&mut harness, 8, |app| !app.loading.saving);
    assert_eq!(api.counts().record_correction, 1);
    assert!(harness.state().correction_draft.is_some());
    assert!(harness.state().current.is_some());

    click(&mut harness, "Correct & finalize");
    step_until(&mut harness, 12, |_| api.counts().record_correction == 2);
    let request = api.last_correction().unwrap();
    assert_eq!(request.expected_version, 1);
    assert!(matches!(
        request.geometry,
        AnnotationGeometry::BoundingBox(_)
    ));
    assert_eq!(api.counts().annotation_batch, 0);
}

#[test]
fn review_target_is_canonical_and_full_image_phase_cannot_correct() {
    let api = Rc::new(SpyApi::new());
    seed_review_annotation(
        &api,
        AnnotationGeometry::BoundingBox(BoundingBox {
            x: 0.2,
            y: 0.2,
            width: 0.3,
            height: 0.3,
        }),
        true,
    );
    let mut harness = loaded_review_harness(api);
    let canonical = harness.state().annotations[0].annotation_id.clone();
    let mut arbitrary = harness.state().annotations[0].clone();
    arbitrary.annotation_id = labello_domain::AnnotationId::from("arbitrary");
    harness.state_mut().annotations.push(arbitrary.clone());
    harness.state_mut().selected_annotation = Some(arbitrary.annotation_id.clone());

    harness
        .state_mut()
        .request_review(labello_domain::ReviewDecision::Approved);
    assert_eq!(
        harness.state().selected_annotation.as_ref(),
        Some(&canonical)
    );
    let UiCommand::Review { review, .. } = harness.state().runtime.commands.back().unwrap() else {
        panic!("expected review command");
    };
    assert!(matches!(
        &review.target,
        ReviewTarget::AnnotationVersion { annotation_id, .. } if annotation_id == &canonical
    ));

    harness.state_mut().runtime.commands.clear();
    harness.state_mut().runtime.active_requests.clear();
    harness.state_mut().active_operation_id = None;
    harness.state_mut().loading.saving = false;
    harness.state_mut().review_index = harness.state().annotations.len();
    harness.state_mut().selected_annotation = Some(arbitrary.annotation_id);
    harness.state_mut().sync_review_selection();
    assert!(harness.state().selected_annotation.is_none());
    assert!(!harness.state().can_correct_review_object());
    harness.state_mut().start_correction();
    assert!(harness.state().correction_draft.is_none());
}

#[test]
fn correction_mode_blocks_review_shortcuts_and_saturation_never_discards_the_draft() {
    let api = Rc::new(SpyApi::new());
    seed_review_annotation(
        &api,
        AnnotationGeometry::BoundingBox(BoundingBox {
            x: 0.2,
            y: 0.2,
            width: 0.3,
            height: 0.3,
        }),
        true,
    );
    let mut harness = loaded_review_harness(api.clone());
    harness.state_mut().start_correction();
    assert!(harness.state().correction_draft.is_some());

    harness.key_press(egui::Key::Y);
    harness.step();
    harness.key_press(egui::Key::N);
    harness.step();
    assert_eq!(api.counts().record_review, 0);
    assert!(harness.state().correction_draft.is_some());

    saturate_command_queue(harness.state_mut());
    harness
        .state_mut()
        .request_review(labello_domain::ReviewDecision::Rejected);
    assert!(harness.state().correction_draft.is_some());
    assert!(!harness.state().loading.saving);

    harness.state_mut().runtime.commands.clear();
    harness.state_mut().runtime.active_requests.clear();
    harness
        .state_mut()
        .request_review(labello_domain::ReviewDecision::Rejected);
    assert!(harness.state().correction_draft.is_none());
    assert!(harness.state().loading.saving);
}

#[test]
fn review_and_save_responses_propagate_renewed_assignments_without_refetching_state() {
    let review_api = Rc::new(SpyApi::new());
    seed_review_annotation(
        &review_api,
        AnnotationGeometry::BoundingBox(BoundingBox {
            x: 0.2,
            y: 0.2,
            width: 0.3,
            height: 0.3,
        }),
        true,
    );
    let mut review = loaded_review_harness(review_api.clone());
    let original_review_expiry = review.state().assignment.as_ref().unwrap().expires_at;
    let state_reads = review_api.counts().get_image_state;
    review
        .state_mut()
        .request_review(labello_domain::ReviewDecision::Approved);
    step_until(&mut review, 8, |app| !app.loading.saving);
    assert_eq!(review_api.counts().get_image_state, state_reads);
    assert!(
        review.state().assignment.as_ref().unwrap().expires_at > original_review_expiry,
        "review response did not renew the active assignment"
    );

    let save_api = Rc::new(SpyApi::new());
    let mut work = loaded_work_harness(save_api);
    click(&mut work, "Accept");
    let original_save_expiry = work.state().assignment.as_ref().unwrap().expires_at;
    work.state_mut().request_save(false);
    step_until(&mut work, 8, |app| !app.loading.saving);
    assert!(
        work.state().assignment.as_ref().unwrap().expires_at > original_save_expiry,
        "save response did not renew the active assignment"
    );
}

#[test]
fn reviewer_correction_edits_existing_keypoint_and_visibility_with_undo() {
    let api = Rc::new(SpyApi::new());
    {
        let mut state = api.state.borrow_mut();
        state.metadata.tasks[0].annotation_type = AnnotationType::Skeleton;
        state.metadata.tasks[0].prelabel_config_ids.clear();
        state.metadata.tasks[0].skeleton = Some(SkeletonSpec {
            keypoints: vec![KeypointSpec {
                name: "nose".to_string(),
                required: true,
            }],
            edges: Vec::new(),
            allow_hidden: true,
            allow_absent: false,
        });
    }
    let annotation_id = seed_review_annotation(
        &api,
        AnnotationGeometry::Skeleton(SkeletonGeometry {
            keypoints: vec![KeypointAnnotation {
                name: "nose".to_string(),
                state: KeypointState::Visible,
                point: Some(NormalizedPoint { x: 0.5, y: 0.5 }),
            }],
        }),
        true,
    );
    let mut harness = loaded_review_harness(api);
    harness.set_size(egui::vec2(1500.0, 1100.0));
    harness.step();
    click(&mut harness, "Correct object");
    harness.state_mut().select_correction_keypoint(0);
    harness.step();
    for label in ["Object", "Keypoints", "Reason", "Actions"] {
        assert!(harness.query_by_label(label).is_some());
    }
    click(&mut harness, "Hidden");
    harness
        .state_mut()
        .edit_correction_keypoint(crate::canvas::KeypointEdit {
            annotation_id: annotation_id.clone(),
            keypoint_index: 0,
            point: NormalizedPoint { x: 0.65, y: 0.4 },
        });

    let draft = harness.state().correction_draft.as_ref().unwrap();
    let AnnotationGeometry::Skeleton(skeleton) = &draft.edited_geometry else {
        panic!("expected skeleton correction draft");
    };
    assert_eq!(skeleton.keypoints[0].state, KeypointState::Hidden);
    assert_eq!(skeleton.keypoints[0].point.unwrap().x, 0.65);
    assert!(matches!(
        harness.state().annotations[0].geometry,
        AnnotationGeometry::Skeleton(ref original)
            if original.keypoints[0].state == KeypointState::Visible
                && original.keypoints[0].point.unwrap().x == 0.5
    ));

    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::Z);
    harness.step();
    let draft = harness.state().correction_draft.as_ref().unwrap();
    let AnnotationGeometry::Skeleton(skeleton) = &draft.edited_geometry else {
        panic!("expected skeleton correction draft");
    };
    assert_eq!(skeleton.keypoints[0].state, KeypointState::Hidden);
    assert_eq!(skeleton.keypoints[0].point.unwrap().x, 0.5);

    harness
        .state_mut()
        .edit_correction_keypoint(crate::canvas::KeypointEdit {
            annotation_id,
            keypoint_index: 0,
            point: NormalizedPoint { x: 0.65, y: 0.4 },
        });
    click(&mut harness, "Undo correction");
    let draft = harness.state().correction_draft.as_ref().unwrap();
    let AnnotationGeometry::Skeleton(skeleton) = &draft.edited_geometry else {
        panic!("expected skeleton correction draft");
    };
    assert_eq!(skeleton.keypoints[0].state, KeypointState::Hidden);
    assert_eq!(skeleton.keypoints[0].point.unwrap().x, 0.5);
}

#[test]
fn annotation_inspector_exposes_objects_and_visible_deletion() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);
    harness.state_mut().create_bbox(BoundingBox {
        x: 0.1,
        y: 0.1,
        width: 0.2,
        height: 0.2,
    });
    harness.step();

    assert!(
        harness
            .query_by_label("Object 1 | Person | Selected")
            .is_some()
    );
    click_accesskit_button(&mut harness, "Geometry details for Object 1");
    assert!(
        harness
            .query_by_label_contains("Position: 10% from left")
            .is_some()
    );
    click(&mut harness, "Delete selected annotation");
    assert!(harness.state().annotations[0].deleted);
    assert!(harness.state().selected_annotation.is_none());
}

#[test]
fn history_covers_bbox_edits_deletion_and_keypoint_creation() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);
    harness.state_mut().create_bbox(BoundingBox {
        x: 0.1,
        y: 0.1,
        width: 0.2,
        height: 0.2,
    });
    let annotation_id = harness.state().annotations[0].annotation_id.clone();
    assert_eq!(
        harness.state().selected_annotation.as_ref(),
        Some(&annotation_id)
    );
    harness.state_mut().edit_bbox(BoundingBoxEdit {
        annotation_id: annotation_id.clone(),
        bounding_box: BoundingBox {
            x: 0.3,
            y: 0.25,
            width: 0.4,
            height: 0.35,
        },
    });
    harness.state_mut().undo();
    assert!(matches!(
        harness.state().annotations[0].geometry,
        AnnotationGeometry::BoundingBox(BoundingBox { x, .. }) if (x - 0.1).abs() < f32::EPSILON
    ));

    harness.key_press(egui::Key::Delete);
    harness.step();
    assert!(harness.state().annotations[0].deleted);
    assert!(harness.state().selected_annotation.is_none());
    harness.state_mut().undo();
    assert!(!harness.state().annotations[0].deleted);

    let api = Rc::new(SpyApi::new());
    api.state.borrow_mut().metadata.tasks[0].annotation_type = AnnotationType::Skeleton;
    api.state.borrow_mut().metadata.tasks[0]
        .prelabel_config_ids
        .clear();
    api.state.borrow_mut().metadata.tasks[0].skeleton = Some(SkeletonSpec {
        keypoints: vec![KeypointSpec {
            name: "center".to_string(),
            required: true,
        }],
        edges: Vec::new(),
        allow_hidden: true,
        allow_absent: false,
    });
    let mut harness = loaded_work_harness(api);
    harness
        .state_mut()
        .place_keypoint(labello_domain::NormalizedPoint { x: 0.5, y: 0.5 });
    assert_eq!(harness.state().annotations.len(), 1);
    harness.state_mut().undo();
    assert!(harness.state().annotations.is_empty());
    harness.state_mut().redo();
    assert_eq!(harness.state().annotations.len(), 1);
}

#[test]
fn stats_and_responsive_layouts_render_without_losing_primary_actions() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    assert_eq!(api.counts().dataset_stats, 0);

    click_application_menu_item(&mut harness, "Statistics");
    release_and_switch(&mut harness);
    step_until(&mut harness, 8, |app| app.view == AppView::Stats);
    harness.step();
    assert!(harness.query_by_label("Live Statistics").is_some());
    click(&mut harness, "Refresh now");
    step_until(&mut harness, 8, |app| !app.loading.stats);
    assert!(api.counts().dataset_stats >= 1);

    harness.set_size(egui::vec2(390.0, 760.0));
    harness.step();
    assert!(harness.query_by_label("More application actions").is_some());

    harness.set_size(egui::vec2(1280.0, 820.0));
    harness.step();
    click_application_menu_item(&mut harness, "Annotate");
    harness.step();
    assert!(harness.query_by_label_contains("Save").is_some());
    assert!(harness.query_by_label_contains("Submit & next").is_some());
    assert!(harness.query_by_label_contains("Skip").is_some());
}

#[test]
fn stats_remote_states_never_replace_real_data_with_placeholders() {
    let mut app = LabelloApp {
        view: AppView::Stats,
        ..Default::default()
    };
    app.loading.stats = true;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 780.0))
        .build_eframe(move |_| app);

    assert!(harness.query_by_label("Loading statistics...").is_some());
    assert!(harness.query_by_label("Metric Images").is_none());

    harness.state_mut().loading.stats = false;
    harness.state_mut().datasets.stats_error = Some("statistics unavailable".to_string());
    harness.step();
    assert!(harness.query_by_label("Statistics unavailable").is_some());
    assert!(harness.query_by_label("Retry statistics").is_some());
    assert!(harness.query_by_label("Metric Images").is_none());

    harness.state_mut().datasets.stats = stats(12);
    harness.state_mut().datasets.last_stats_completion = Some(Instant::now());
    harness.step();
    assert!(harness.query_by_label("Metric Images").is_some());
    assert!(
        harness
            .query_by_label("Statistics may be stale. Last refresh failed: statistics unavailable")
            .is_some()
    );

    harness.state_mut().datasets.stats_error = None;
    harness.state_mut().loading.stats = true;
    harness.step();
    assert!(harness.query_by_label("Refreshing statistics").is_some());
    assert!(harness.query_by_label("Metric Images").is_some());

    harness.state_mut().loading.stats = false;
    harness.step();
    assert!(harness.query_by_label("Refreshing statistics").is_none());
    assert!(harness.query_by_label("Metric Images").is_some());
}

#[test]
fn throughput_chart_exposes_each_daily_value_to_accessibility() {
    let mut app = LabelloApp {
        view: AppView::Stats,
        ..Default::default()
    };
    app.datasets.stats = stats(12);
    app.datasets.stats.throughput = vec![
        labello_domain::ThroughputPoint {
            day: "2026-07-22".to_string(),
            annotations: 12_345,
            reviews: 1,
        },
        labello_domain::ThroughputPoint {
            day: "2026-07-23".to_string(),
            annotations: 5,
            reviews: 2,
        },
    ];
    app.datasets.last_stats_completion = Some(Instant::now());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 1600.0))
        .build_eframe(move |_| app);

    for width in [1440.0, 600.0, 320.0] {
        harness.set_size(egui::vec2(width, 1600.0));
        harness.step();
        assert!(harness.query_by_label("Daily throughput chart").is_some());
        for label in [
            "2026-07-22: 12345 annotations, 1 review",
            "2026-07-23: 5 annotations, 2 reviews",
        ] {
            assert!(
                harness
                    .query_by_role_and_label(egui::accesskit::Role::Label, label)
                    .is_some(),
                "missing accessible throughput value at width {width}: {label}"
            );
        }
    }
}

#[test]
fn command_and_message_budgets_preserve_frame_responsiveness() {
    let api = Rc::new(SpyApi::new());
    let mut app = base_live_app(api);
    app.setup.started = true;
    app.view = AppView::Stats;
    for _ in 0..80 {
        app.request_stats();
        app.loading.stats = false;
    }
    assert_eq!(app.runtime.commands.len(), 64);

    app.start_next_command();
    assert_eq!(app.runtime.commands.len(), 63);
    app.start_next_command();
    assert_eq!(app.runtime.commands.len(), 62);

    let mut app = base_live_app(Rc::new(SpyApi::new()));
    app.setup.started = true;
    app.view = AppView::Stats;
    app.datasets.active_stats_request = Some((20, DatasetId::from("demo")));
    app.loading.stats = true;
    app.runtime.active_requests.insert(20);
    for index in 0..20 {
        app.runtime
            .tx
            .send(UiMessage::StatsLoaded {
                request: test_request(&app, index as u64 + 1, Some("demo")),
                result: Ok(stats(index)),
            })
            .unwrap();
    }
    app.process_messages(&egui::Context::default());
    assert_eq!(app.datasets.stats.total_images, 0);
    app.process_messages(&egui::Context::default());
    assert_eq!(app.datasets.stats.total_images, 0);
    app.process_messages(&egui::Context::default());
    assert_eq!(app.datasets.stats.total_images, 19);
    assert!(!app.loading.stats);

    let upload_request = test_request(&app, 90_000, Some("demo"));
    app.runtime
        .tx
        .send(UiMessage::FolderUploadProgress {
            request: upload_request.clone(),
            progress: FolderUploadProgress {
                uploaded_files: 12,
                total_files: 24,
                current_batch: 2,
                message: "Uploading batch 2".to_string(),
            },
        })
        .unwrap();
    app.process_messages(&egui::Context::default());
    assert!(app.loading.uploading);
    assert_eq!(
        app.loading
            .upload_progress
            .as_ref()
            .map(|progress| progress.fraction()),
        Some(0.5)
    );

    app.begin_workspace_epoch();
    app.runtime
        .tx
        .send(UiMessage::FolderUploadFinished {
            request: upload_request,
            result: Ok("Uploaded stale files".to_string()),
        })
        .unwrap();
    app.process_messages(&egui::Context::default());
    assert!(!app.loading.uploading);
    assert_ne!(app.runtime.notice.as_deref(), Some("Uploaded stale files"));
    assert_eq!(app.view, AppView::Stats);
}

#[test]
fn stats_ignore_stale_request_and_dataset_responses() {
    let mut app = base_live_app(Rc::new(SpyApi::new()));
    app.view = AppView::Stats;
    app.loading.stats = true;
    app.datasets.active_stats_request = Some((2, DatasetId::from("demo")));
    app.datasets.stats_error = Some("stale refresh failure".to_string());
    app.runtime.active_requests.insert(2);

    for (request_id, dataset_id) in [(1, "demo"), (2, "other")] {
        app.runtime
            .tx
            .send(UiMessage::StatsLoaded {
                request: test_request(&app, request_id, Some(dataset_id)),
                result: Ok(stats(request_id as usize)),
            })
            .unwrap();
    }
    app.process_messages(&egui::Context::default());
    assert!(app.loading.stats);
    assert_eq!(app.datasets.stats.total_images, 0);

    app.runtime
        .tx
        .send(UiMessage::StatsLoaded {
            request: test_request(&app, 2, Some("demo")),
            result: Ok(stats(42)),
        })
        .unwrap();
    app.process_messages(&egui::Context::default());
    assert!(!app.loading.stats);
    assert_eq!(app.datasets.stats.total_images, 42);
    assert!(app.datasets.last_stats_completion.is_some());
    assert!(app.datasets.stats_error.is_none());
}

#[test]
fn stats_polling_is_scheduled_from_completion_and_queue_failure_recovers() {
    let api = Rc::new(SpyApi::new());
    let metadata = api.state.borrow().metadata.clone();
    let mut app = base_live_app(api);
    app.setup.started = true;
    app.view = AppView::Stats;
    app.datasets.metadata = Some(metadata);
    app.datasets.last_stats_attempt = Some(Instant::now());

    app.refresh_stats_if_due();
    assert!(app.runtime.commands.is_empty());

    app.datasets.last_stats_attempt = Some(Instant::now() - Duration::from_secs(4));
    app.refresh_stats_if_due();
    assert!(app.loading.stats);
    assert_eq!(app.runtime.commands.len(), 1);

    app.loading.stats = false;
    app.datasets.active_stats_request = None;
    app.runtime.commands.clear();
    for request_id in 10_000..10_064 {
        app.runtime.commands.push_back(UiCommand::DatasetList {
            request: test_request(&app, request_id, None),
        });
    }
    app.request_stats();
    assert!(!app.loading.stats);
    assert!(app.datasets.active_stats_request.is_none());
    assert!(app.datasets.last_stats_attempt.is_some());
    assert!(app.datasets.last_stats_completion.is_none());
}

#[test]
fn changing_datasets_cancels_an_inflight_stats_request() {
    let mut app = base_live_app(Rc::new(SpyApi::new()));
    app.loading.stats = true;
    app.datasets.active_stats_request = Some((7, DatasetId::from("demo")));
    app.datasets.stats = stats(99);

    app.open_dataset(DatasetId::from("other"), AppView::Stats);

    assert!(!app.loading.stats);
    assert!(app.datasets.active_stats_request.is_none());
    assert_eq!(app.datasets.stats, DatasetStats::default());
}
