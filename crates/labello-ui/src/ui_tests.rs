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
    ImageExplorerQuery, ImageFile, ImagePreview, IngestJob, IngestJobStatus, IngestReport,
    KeybindingApi, OAuthCallbackRequest, OAuthLoginRequest, OfflineApi, OfflineBundleRequest,
    PrelabelApi, PrelabelSuggestionRequest, ReviewApi, SetDatasetRolesRequest, SnapshotFile,
    StatsApi, TaskApi, UpdateDatasetConfigRequest, UserApi,
};
use labello_domain::{
    AdjudicationRecord, AnnotationGeometry, AnnotationType, Assignment, AssignmentId,
    AssignmentKind, AssignmentStatus, BoundingBox, BrowserAcceleration, ClassId, DatasetId,
    DatasetMetadata, DatasetRole, DatasetRoleAssignment, DatasetSnapshot, DatasetStats,
    EventLogEntry, EventPayload, ImageExplorerItem, ImageExplorerPage, ImageId, ImageRecord,
    ImageState, KeybindingSet, KeypointAnnotation, KeypointSpec, KeypointState, LabelClass,
    ModelSpec, NormalizedPoint, OfflineBundle, OfflineSyncRequest, OfflineSyncResult,
    OutputProcessing, PrelabelConfig, PrelabelConfigId, PrelabelExecution, PrelabelSuggestion,
    ReviewConfig, ReviewId, ReviewRecord, ReviewTarget, SCHEMA_VERSION, SkeletonGeometry,
    SkeletonSpec, SnapshotFileEntry, TaskDefinition, TaskId, TaskStatus, TutorialContent,
    UserAccount, UserId,
};
use web_time::{Duration, Instant};

use crate::app::{
    AdminSection, AppConfig, AppView, Drawer, FolderUploadProgress, IMAGE_QUEUE_SIZE, LabelloApp,
    LayoutMode, RequestIdentity, SaveStatus, UiCommand, UiMessage,
};
use crate::canvas::BoundingBoxEdit;
use crate::persistence::{StoredCanvasTransform, StoredView, WorkspacePreference};

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
    click(&mut harness, "Create a dataset");
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
fn admin_workflow_saves_ingests_and_handles_browser_only_folder_upload() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_admin_harness(api.clone());
    harness.set_size(egui::vec2(1180.0, 4000.0));
    harness.step();

    assert!(harness.query_by_label("Dataset Admin").is_some());
    click_accesskit_button(&mut harness, "Images");
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
    click_accesskit_button(&mut harness, "Schema");
    click_accesskit_button(&mut harness, "Add bounding box class workflow");
    harness.step();
    click_accesskit_button(&mut harness, "Automation");
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
    click_accesskit_button(&mut harness, "Save Admin Config");
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
    click_accesskit_button(&mut harness, "Images");
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

    click_accesskit_button(&mut harness, "Backups");
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
    click_accesskit_button(&mut harness, "Schema");

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
        .query_all_by_role_and_label(egui::accesskit::Role::TextInput, "Description")
        .collect::<Vec<_>>();
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
        assert!(class_description_fields[index].rect().width() >= 2.9 * unit);
        assert!(
            class_description_fields[index].rect().right() - class_name_fields[index].rect().left()
                <= 640.5,
            "class editor exceeds the 640-point form column"
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

    assert!(harness.query_by_label("Sign in with GitHub").is_some());
    assert!(harness.query_by_label("Continue as local admin").is_some());
    assert!(harness.query_by_label("Dev token").is_none());
    assert!(harness.query_by_label("Development user ID").is_none());

    click(&mut harness, "Continue as local admin");
    step_until(&mut harness, 8, |app| app.auth.account.is_some());
    assert_eq!(api.counts().local_admin_login, 1);
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
            result: Ok(account.clone()),
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
            result: Ok(account.clone()),
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
    click_accesskit_button(&mut harness, "People");
    assert!(harness.query_by_label("Reviewer Person").is_some());
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
            .is_some()
    );
    let reviewer = harness
        .state_mut()
        .datasets
        .users
        .iter_mut()
        .find(|user| user.account.user_id == UserId::from("reviewer"))
        .unwrap();
    reviewer.roles.push(DatasetRole::Reviewer);
    harness.step();
    assert!(
        harness
            .query_by_label("Unsaved permission changes")
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
    harness
        .state_mut()
        .request_role_save(UserId::from("reviewer"));
    step_until(&mut harness, 8, |app| app.loading.roles_user.is_none());
    assert!(harness.query_by_label("Admin config saved").is_some());
    assert_eq!(api.counts().set_dataset_roles, 1);
    assert!(
        api.dataset_users()
            .iter()
            .find(|user| user.account.user_id == UserId::from("reviewer"))
            .unwrap()
            .roles
            .contains(&DatasetRole::Reviewer)
    );

    let admin = harness
        .state_mut()
        .datasets
        .users
        .iter_mut()
        .find(|user| user.account.user_id == UserId::from("admin"))
        .unwrap();
    admin.roles.retain(|role| role != &DatasetRole::DataAdmin);
    harness.state_mut().request_role_save(UserId::from("admin"));
    assert_eq!(api.counts().set_dataset_roles, 1);
    assert!(
        harness
            .state()
            .runtime
            .error
            .as_deref()
            .is_some_and(|error| error.contains("own data admin"))
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

    click_accesskit_button(&mut harness, "Schema");
    assert_eq!(harness.state().admin_tools.section, AdminSection::Schema);
    click_accesskit_button(&mut harness, "Overview");
    assert_eq!(
        harness.state().datasets.admin_config.as_ref().unwrap().name,
        "Unsaved rename"
    );

    click(&mut harness, "Discard staged changes");
    assert_eq!(
        harness.state().datasets.admin_config.as_ref().unwrap().name,
        original_name
    );
    assert_eq!(api.counts().get_admin_dataset, 1);
}

#[test]
fn admin_destructive_edits_require_confirmation() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_admin_harness(api);
    harness.set_size(egui::vec2(1300.0, 4000.0));
    harness.step();
    click_accesskit_button(&mut harness, "Schema");
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
    assert!(harness.query_by_label("Refreshing admin config").is_some());
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
    assert!(
        harness
            .get_by_role_and_label(egui::accesskit::Role::TextInput, "Search people")
            .rect()
            .height()
            >= 43.0
    );
    harness.state_mut().admin_tools.section = AdminSection::Images;
    harness.step();
    for label in ["Root path", "Search images"] {
        assert!(
            harness
                .get_by_role_and_label(egui::accesskit::Role::TextInput, label)
                .rect()
                .height()
                >= 43.0,
            "{label} is not touch-friendly"
        );
    }
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
    click(&mut harness, "Record shortcut for Submit and next");
    harness.key_press(egui::Key::Enter);
    harness.step();
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
    click(&mut harness, "Record shortcut for Submit and next");
    harness.key_press(egui::Key::Enter);
    harness.step();
    click(&mut harness, "Cancel");
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
        .query_all_by_label_contains("Menu")
        .find(|node| node.accesskit_node().role() == egui::accesskit::Role::Button)
        .expect("application menu button")
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
        let status = harness.get_by_label(message).rect();
        let menu = harness.get_by_label("Menu").rect();
        assert!(dataset.right() <= status.left() + 0.5);
        assert!(status.right() <= menu.left() + 0.5);
        if width >= LayoutMode::COMPACT_MAX_WIDTH {
            let save = harness.get_by_label("Idle").rect();
            assert!(status.right() <= save.left() + 0.5);
            assert!(save.right() <= menu.left() + 0.5);
        }
        assert_visible_controls_clamped(&harness, width, height);
    }
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

    saturate_command_queue(&mut app);
    app.request_role_save(UserId::from("reviewer"));
    assert!(app.loading.roles_user.is_none());

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
fn signed_in_setup_collapses_advanced_fields_and_labels_inputs() {
    let api = Rc::new(SpyApi::new());
    let mut harness = live_harness(api);
    step_until(&mut harness, 8, |app| !app.datasets.summaries.is_empty());

    assert!(harness.query_by_label("Choose where to work").is_some());
    assert!(harness.query_by_label("API URL").is_none());
    assert!(harness.state().setup.create_dataset_id.is_empty());
    assert!(harness.state().setup.create_dataset_name.is_empty());

    click(&mut harness, "Advanced connection settings");
    let api_url = harness
        .query_all_by_role_and_label(egui::accesskit::Role::TextInput, "API URL")
        .next()
        .expect("API URL field should have an accessible label");
    assert!(api_url.rect().height() <= 25.0);
    assert!(harness.query_by_label("Development user ID").is_none());
    assert!(harness.query_by_label("Dev token").is_none());

    harness.set_size(egui::vec2(390.0, 844.0));
    harness.step();
    let compact_api_url = harness
        .query_all_by_role_and_label(egui::accesskit::Role::TextInput, "API URL")
        .next()
        .expect("compact API URL field should retain its accessible label");
    assert!(compact_api_url.rect().height() <= 25.0);
    assert!(compact_api_url.rect().right() <= 390.5);
}

#[test]
fn api_url_focus_loss_does_not_reconnect_and_enter_commits() {
    let app = LabelloApp {
        view: AppView::Setup,
        ..Default::default()
    };
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

    let datasets = harness.get_by_label("Datasets").rect().center();
    click_at(&mut harness, datasets);
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
}

#[test]
fn dataset_states_distinguish_loading_and_stale_refresh_failure() {
    let api = Rc::new(SpyApi::new());
    let mut harness = live_harness(api);
    step_until(&mut harness, 8, |app| !app.datasets.summaries.is_empty());
    let summaries = harness.state().datasets.summaries.clone();

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
    click(&mut harness, "Menu");
    click_accesskit_button(&mut harness, "Navigation");
    for label in ["Annotate", "Review", "Adjudicate", "Admin", "Stats"] {
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

    click(&mut harness, "Menu");
    click_accesskit_button(&mut harness, "Navigation");
    for label in ["Annotate", "Review", "Stats"] {
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
        let status_badge = harness.get_by_label("Idle").rect();
        assert!(
            dataset_badge.height() > 0.0,
            "dataset badge is missing at {width}x{height}",
        );
        assert!(status_badge.height() > 0.0);
        let menu = harness.get_by_label("Menu").rect();
        assert!(dataset_badge.right() <= status_badge.left() + 0.5);
        assert!(status_badge.right() <= menu.left() + 0.5);
        if width >= LayoutMode::COMPACT_MAX_WIDTH {
            assert!(
                width - menu.right() <= 25.0,
                "application menu is not right-aligned at {width}x{height}: {menu:?}",
            );
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
            (1288, 820) => Some((676.0, 593.0)),
            (1366, 768) => Some((754.0, 541.0)),
            (1440, 900) => Some((828.0, 673.0)),
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
        assert!(harness.query_by_label(label).is_some());
        assert_visible_controls_clamped(&harness, 320.0, 568.0);
    }

    click(&mut harness, "Menu");
    for label in ["Navigation", "Workspace", "Status", "Sign out"] {
        assert_control_inside(&harness, label, egui::accesskit::Role::Button, 320.0, 568.0);
    }
    click_accesskit_button(&mut harness, "Navigation");
    for label in [
        "Setup",
        "Annotate",
        "Review",
        "Adjudicate",
        "Stats",
        "Admin",
    ] {
        assert_control_inside(&harness, label, egui::accesskit::Role::Button, 320.0, 568.0);
    }
    assert_visible_controls_clamped(&harness, 320.0, 568.0);
    harness.key_press(egui::Key::Escape);
    harness.step();
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
        if LayoutMode::for_width(width) == LayoutMode::Wide {
            assert!(harness.query_by_label("Menu").is_none());
            assert!(harness.query_by_label("Desktop navigation").is_some());
            assert_control_inside(
                &harness,
                "Setup",
                egui::accesskit::Role::Button,
                width,
                height,
            );
        } else {
            assert_control_inside(
                &harness,
                "Menu",
                egui::accesskit::Role::Button,
                width,
                height,
            );
        }
        assert_visible_controls_clamped(&harness, width, height);
    }

    harness.set_size(egui::vec2(1440.0, 320.0));
    harness.step();
    assert!(harness.query_by_label("Menu").is_none());
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
        let finalize =
            harness.get_by_role_and_label(egui::accesskit::Role::Button, "Correct & finalize");
        finalize.scroll_to_me();
        harness.step();
        assert_control_inside(
            &harness,
            "Correct & finalize",
            egui::accesskit::Role::Button,
            width,
            height,
        );
        assert_visible_controls_clamped(&harness, width, height);
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

    for (width, height) in viewport_sizes() {
        harness.set_size(egui::vec2(width, height));
        harness.step();
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

    for (width, height) in viewport_sizes() {
        harness.set_size(egui::vec2(width, height));
        harness.step();
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
fn admin_geometry_keeps_save_and_discard_visible() {
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
        for label in ["Save Admin Config", "Discard staged changes"] {
            assert_control_inside(
                &harness,
                label,
                egui::accesskit::Role::Button,
                width,
                height,
            );
        }
        assert_label_inside(&harness, "1 validation error(s)", width, height);
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
    for label in ["Restore all defaults", "Cancel", "Save changes"] {
        assert_control_inside(&harness, label, egui::accesskit::Role::Button, 600.0, 568.0);
    }
    assert_visible_controls_clamped(&harness, 600.0, 568.0);
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
            .query_by_label_contains("Object 1: Person, box at")
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

    click_application_menu_item(&mut harness, "Stats");
    release_and_switch(&mut harness);
    step_until(&mut harness, 8, |app| app.view == AppView::Stats);
    harness.step();
    assert!(harness.query_by_label("Live Statistics").is_some());
    click(&mut harness, "Refresh now");
    step_until(&mut harness, 8, |app| !app.loading.stats);
    assert!(api.counts().dataset_stats >= 1);

    harness.set_size(egui::vec2(390.0, 760.0));
    harness.step();
    assert!(harness.query_by_label("Menu").is_some());

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

fn live_harness(api: Rc<SpyApi>) -> Harness<'static, LabelloApp> {
    Harness::builder()
        .with_size(egui::vec2(1500.0, 780.0))
        .with_max_steps(80)
        .build_eframe(|_| base_live_app(api))
}

fn loaded_work_harness(api: Rc<SpyApi>) -> Harness<'static, LabelloApp> {
    let mut harness = live_harness(api);
    step_until(&mut harness, 8, |app| app.datasets.summaries.len() == 1);
    click(&mut harness, "Continue with Demo Dataset");
    step_until(&mut harness, 12, |app| app.current.is_some());
    harness
}

fn loaded_review_harness(api: Rc<SpyApi>) -> Harness<'static, LabelloApp> {
    let mut harness = live_harness(api);
    step_until(&mut harness, 8, |app| app.datasets.summaries.len() == 1);
    click(&mut harness, "Review Demo Dataset");
    step_until(&mut harness, 12, |app| {
        app.view == AppView::Review && app.current.is_some()
    });
    harness
}

fn loaded_adjudication_harness(api: Rc<SpyApi>) -> Harness<'static, LabelloApp> {
    let mut harness = live_harness(api);
    step_until(&mut harness, 8, |app| app.datasets.summaries.len() == 1);
    click(&mut harness, "Adjudicate Demo Dataset");
    step_until(&mut harness, 12, |app| {
        app.view == AppView::Adjudicate && app.current.is_some()
    });
    harness
}

fn loaded_admin_harness(api: Rc<SpyApi>) -> Harness<'static, LabelloApp> {
    let mut harness = live_harness(api);
    step_until(&mut harness, 8, |app| app.datasets.summaries.len() == 1);
    click(&mut harness, "Admin Demo Dataset");
    step_until(&mut harness, 8, |app| {
        app.view == AppView::Admin && app.datasets.admin_config.is_some() && !app.loading.admin
    });
    harness
}

fn base_live_app(api: Rc<SpyApi>) -> LabelloApp {
    let mut app = LabelloApp::live_http(AppConfig {
        api_base_url: "http://example.invalid".to_string(),
        application_url: Some("https://app.example.test/label?dataset=demo".to_string()),
        user_id: UserId::from("admin"),
        dataset_id: DatasetId::from("demo"),
        queue_size: IMAGE_QUEUE_SIZE,
    });
    app.runtime.api = Some(api);
    app.runtime.error = None;
    app
}

fn test_request(app: &LabelloApp, request_id: u64, dataset_id: Option<&str>) -> RequestIdentity {
    RequestIdentity {
        auth_epoch: app.auth_epoch,
        workspace_epoch: app.workspace_epoch,
        request_id,
        dataset_id: dataset_id.map(DatasetId::from),
    }
}

fn saturate_command_queue(app: &mut LabelloApp) {
    app.runtime.commands.clear();
    app.runtime.active_requests.clear();
    for request_id in 80_000..80_064 {
        app.runtime.commands.push_back(UiCommand::DatasetList {
            request: test_request(app, request_id, None),
        });
    }
}

fn viewport_sizes() -> [(f32, f32); 10] {
    [
        (320.0, 568.0),
        (390.0, 667.0),
        (600.0, 800.0),
        (768.0, 1024.0),
        (1024.0, 768.0),
        (1239.0, 820.0),
        (1240.0, 820.0),
        (1288.0, 820.0),
        (1366.0, 768.0),
        (1440.0, 900.0),
    ]
}

fn assert_control_inside(
    harness: &Harness<'static, LabelloApp>,
    label: &str,
    role: egui::accesskit::Role,
    width: f32,
    height: f32,
) {
    let node = harness
        .query_all_by_role_and_label(role, label)
        .next()
        .or_else(|| {
            harness
                .query_all_by_label_contains(label)
                .find(|node| node.accesskit_node().role() == role)
        })
        .unwrap_or_else(|| panic!("No {role:?} found containing {label:?}"));
    let rect = node.rect();
    assert!(
        rect.left() >= -0.5
            && rect.top() >= -0.5
            && rect.right() <= width + 0.5
            && rect.bottom() <= height + 0.5,
        "{label:?} is outside {width}x{height}: {rect:?}",
    );
    if role == egui::accesskit::Role::Button {
        assert!(
            rect.height() >= 43.0,
            "{label:?} touch target is shorter than 44px: {rect:?}",
        );
    }
}

fn assert_label_inside(
    harness: &Harness<'static, LabelloApp>,
    label: &str,
    width: f32,
    height: f32,
) {
    let rect = harness.get_by_label(label).rect();
    assert!(
        rect.left() >= -0.5
            && rect.top() >= -0.5
            && rect.right() <= width + 0.5
            && rect.bottom() <= height + 0.5,
        "{label:?} is outside {width}x{height}: {rect:?}",
    );
}

fn assert_canvas_geometry(harness: &Harness<'static, LabelloApp>, width: f32, height: f32) {
    let canvas = harness.get_by_label("Annotation canvas").rect();
    let dataset = harness.get_by_label_contains("Dataset ").rect();
    assert!(
        canvas.top() >= dataset.bottom(),
        "canvas overlaps the top shell"
    );
    assert!(
        canvas.left() >= -0.5
            && canvas.top() >= -0.5
            && canvas.right() <= width + 0.5
            && canvas.bottom() <= height + 0.5,
        "canvas is outside {width}x{height}: {canvas:?}",
    );
    let minimum = if width < 600.0 { 200.0 } else { 360.0 };
    assert!(
        canvas.height() >= minimum,
        "canvas is not useful at {width}x{height}: {canvas:?}",
    );
}

fn assert_visible_controls_clamped(
    harness: &Harness<'static, LabelloApp>,
    width: f32,
    height: f32,
) {
    for role in [
        egui::accesskit::Role::Button,
        egui::accesskit::Role::CheckBox,
        egui::accesskit::Role::ComboBox,
        egui::accesskit::Role::TextInput,
    ] {
        for node in harness.query_all_by_role(role) {
            let rect = node.rect();
            // Scroll areas retain accessibility nodes just outside their clip rect.
            // Check horizontal containment for controls that are fully visible vertically.
            if rect.top() < 0.0 || rect.bottom() > height {
                continue;
            }
            assert!(
                rect.left() >= -0.5
                    && rect.right() <= width + 0.5
                    && rect.left().is_finite()
                    && rect.right().is_finite(),
                "visible {role:?} is outside {width}x{height}: {rect:?}\n{node:?}",
            );
        }
    }
}

fn click(harness: &mut Harness<'static, LabelloApp>, label: &str) {
    assert!(
        click_visible(harness, label),
        "button or label {label:?} was not visible"
    );
    harness.step();
}

fn click_application_menu_item(harness: &mut Harness<'static, LabelloApp>, label: &str) {
    click(harness, "Menu");
    let section = match label {
        "Annotate" | "Review" | "Adjudicate" | "Admin" | "Stats" => "Navigation",
        _ => "Workspace",
    };
    click_accesskit_button(harness, section);
    click_accesskit_button(harness, label);
}

fn click_at(harness: &mut Harness<'static, LabelloApp>, pos: egui::Pos2) {
    harness.event(egui::Event::PointerMoved(pos));
    harness.event(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.event(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
}

fn release_and_switch(harness: &mut Harness<'static, LabelloApp>) {
    assert!(harness.query_by_label("Release and switch").is_some());
    harness.state_mut().release_pending_transition();
    harness.step();
}

fn click_accesskit_button(harness: &mut Harness<'static, LabelloApp>, label: &str) {
    harness
        .query_all_by_role_and_label(egui::accesskit::Role::Button, label)
        .next()
        .or_else(|| {
            harness
                .query_all_by_label_contains(label)
                .find(|node| node.accesskit_node().role() == egui::accesskit::Role::Button)
        })
        .unwrap()
        .click_accesskit();
    harness.step();
}

fn click_visible(harness: &Harness<'static, LabelloApp>, label: &str) -> bool {
    if let Some(node) = harness
        .query_all_by_role_and_label(egui::accesskit::Role::Button, label)
        .next()
    {
        node.click();
        true
    } else if let Some(node) = harness.query_all_by_label(label).next() {
        node.click();
        true
    } else if let Some(node) = harness
        .query_all_by_label_contains(label)
        .find(|node| node.accesskit_node().role() == egui::accesskit::Role::Button)
    {
        node.click();
        true
    } else {
        false
    }
}

fn step_until(
    harness: &mut Harness<'static, LabelloApp>,
    max_steps: usize,
    predicate: impl Fn(&LabelloApp) -> bool,
) {
    for _ in 0..max_steps {
        if predicate(harness.state()) {
            return;
        }
        harness.step();
    }
    assert!(
        predicate(harness.state()),
        "view={:?} current={:?} assignment={:?} loading(dataset={}, image={}, saving={}) pending={:?} error={:?}",
        harness.state().view,
        harness
            .state()
            .current
            .as_ref()
            .map(|current| current.image.image_id.clone()),
        harness
            .state()
            .assignment
            .as_ref()
            .map(|assignment| assignment.assignment_id.clone()),
        harness.state().loading.dataset,
        harness.state().loading.image,
        harness.state().loading.saving,
        harness.state().pending_transition,
        harness.state().runtime.error,
    );
}

#[derive(Clone)]
struct SpyApi {
    state: Rc<RefCell<SpyState>>,
}

impl SpyApi {
    fn new() -> Self {
        Self {
            state: Rc::new(RefCell::new(SpyState::new())),
        }
    }

    fn counts(&self) -> CallCounts {
        self.state.borrow().counts.clone()
    }

    fn metadata(&self) -> DatasetMetadata {
        self.state.borrow().metadata.clone()
    }

    fn events(&self) -> Vec<EventPayload> {
        self.state.borrow().events.clone()
    }

    fn fail_next_preview(&self) {
        self.state.borrow_mut().fail_next_preview = true;
    }

    fn clear_workflows(&self) {
        self.state.borrow_mut().metadata.tasks.clear();
    }

    fn set_no_assignment(&self, value: bool) {
        self.state.borrow_mut().no_assignment = value;
    }

    fn sanitize_metadata_roles(&self) {
        self.state.borrow_mut().metadata.role_assignments.clear();
    }

    fn set_summary_roles(&self, roles: Vec<DatasetRole>) {
        self.state.borrow_mut().summary_roles = roles;
    }

    fn fail_me(&self) {
        self.state.borrow_mut().fail_me = true;
    }

    fn dataset_users(&self) -> Vec<DatasetUser> {
        self.state.borrow().users.clone()
    }

    fn last_image_query(&self) -> Option<ImageExplorerQuery> {
        self.state.borrow().last_image_query.clone()
    }

    fn last_oauth_return_to(&self) -> Option<String> {
        self.state.borrow().last_oauth_return_to.clone()
    }

    fn fail_next_correction(&self) {
        self.state.borrow_mut().fail_next_correction = true;
    }

    fn fail_next_batch(&self) {
        self.state.borrow_mut().fail_next_batch = true;
    }

    fn last_correction(&self) -> Option<CorrectionRequest> {
        self.state.borrow().last_correction.clone()
    }

    fn exclusions(&self) -> Vec<Vec<ImageId>> {
        self.state.borrow().exclusions.clone()
    }

    fn has_active_assignment(&self, assignment_id: &AssignmentId) -> bool {
        self.state
            .borrow()
            .active_assignments
            .iter()
            .any(|assignment| &assignment.assignment_id == assignment_id)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct CallCounts {
    auth_options: usize,
    local_admin_login: usize,
    me: usize,
    logout: usize,
    list_datasets: usize,
    create_dataset: usize,
    get_dataset: usize,
    get_admin_dataset: usize,
    update_dataset_config: usize,
    ingest_dataset: usize,
    assign_next_image: usize,
    release_assignment: usize,
    complete_assignment: usize,
    get_image_record: usize,
    get_image_state: usize,
    get_image_preview: usize,
    append_event: usize,
    annotation_batch: usize,
    rebuild_image: usize,
    record_review: usize,
    record_correction: usize,
    record_adjudication: usize,
    dataset_stats: usize,
    get_keybindings: usize,
    save_keybindings: usize,
    prelabel_suggestions: usize,
    list_dataset_users: usize,
    set_dataset_roles: usize,
    list_images: usize,
    list_snapshots: usize,
    create_snapshot: usize,
    get_snapshot_file: usize,
}

struct SpyState {
    metadata: DatasetMetadata,
    states: BTreeMap<ImageId, ImageState>,
    counts: CallCounts,
    next_image: usize,
    events: Vec<EventPayload>,
    fail_next_preview: bool,
    no_assignment: bool,
    active_assignments: Vec<Assignment>,
    exclusions: Vec<Vec<ImageId>>,
    completed_images: BTreeSet<ImageId>,
    summary_roles: Vec<DatasetRole>,
    users: Vec<DatasetUser>,
    fail_me: bool,
    last_image_query: Option<ImageExplorerQuery>,
    last_oauth_return_to: Option<String>,
    snapshots: Vec<DatasetSnapshot>,
    fail_next_correction: bool,
    fail_next_batch: bool,
    last_correction: Option<CorrectionRequest>,
}

impl SpyState {
    fn new() -> Self {
        let mut metadata = DatasetMetadata::new(DatasetId::from("demo"), "Demo Dataset", now());
        metadata.image_roots = vec!["images".to_string()];
        metadata.label_classes = vec![
            LabelClass {
                class_id: ClassId::from("person"),
                name: "Person".to_string(),
                color: "#5eead4".to_string(),
                description: Some("Visible people".to_string()),
            },
            LabelClass {
                class_id: ClassId::from("vehicle"),
                name: "Vehicle".to_string(),
                color: "#60a5fa".to_string(),
                description: None,
            },
        ];
        metadata.prelabel_configs = vec![prelabel_config("demo-prelabel")];
        metadata.tasks = vec![
            task("bounding_box:person", "Person boxes", vec!["demo-prelabel"]),
            task("bounding_box:vehicle", "Vehicle boxes", Vec::new()),
        ];
        metadata.role_assignments = vec![DatasetRoleAssignment {
            dataset_id: metadata.dataset_id.clone(),
            user_id: UserId::from("admin"),
            roles: BTreeSet::from([
                DatasetRole::DataAdmin,
                DatasetRole::Annotator,
                DatasetRole::Reviewer,
                DatasetRole::Adjudicator,
            ]),
            assigned_at: now(),
            assigned_by: None,
        }];

        let image_1 = image_record("img_1", "one.png", 640, 480);
        let image_2 = image_record("img_2", "two.png", 800, 600);
        metadata
            .images
            .insert(image_1.image_id.clone(), image_1.clone());
        metadata
            .images
            .insert(image_2.image_id.clone(), image_2.clone());
        let image_3 = image_record("img_3", "three.png", 1024, 768);
        metadata
            .images
            .insert(image_3.image_id.clone(), image_3.clone());
        let states = [image_1, image_2, image_3]
            .into_iter()
            .map(|image| (image.image_id.clone(), ImageState::new(image.image_id)))
            .collect();

        let timestamp = now();
        let users = vec![
            DatasetUser {
                account: UserAccount {
                    user_id: UserId::from("admin"),
                    display_name: "Admin User".to_string(),
                    github_user_id: Some("1".to_string()),
                    github_login: Some("admin".to_string()),
                    created_at: timestamp,
                    updated_at: timestamp,
                },
                roles: vec![
                    DatasetRole::DataAdmin,
                    DatasetRole::Annotator,
                    DatasetRole::Reviewer,
                    DatasetRole::Adjudicator,
                ],
            },
            DatasetUser {
                account: UserAccount {
                    user_id: UserId::from("reviewer"),
                    display_name: "Reviewer Person".to_string(),
                    github_user_id: Some("2".to_string()),
                    github_login: Some("review-person".to_string()),
                    created_at: timestamp,
                    updated_at: timestamp,
                },
                roles: Vec::new(),
            },
        ];
        Self {
            metadata,
            states,
            counts: CallCounts::default(),
            next_image: 0,
            events: Vec::new(),
            fail_next_preview: false,
            no_assignment: false,
            active_assignments: Vec::new(),
            exclusions: Vec::new(),
            completed_images: BTreeSet::new(),
            summary_roles: vec![
                DatasetRole::DataAdmin,
                DatasetRole::Annotator,
                DatasetRole::Reviewer,
                DatasetRole::Adjudicator,
            ],
            users,
            fail_me: false,
            last_image_query: None,
            last_oauth_return_to: None,
            snapshots: Vec::new(),
            fail_next_correction: false,
            fail_next_batch: false,
            last_correction: None,
        }
    }

    fn record(&self, image_id: &ImageId) -> ClientResult<ImageRecord> {
        self.metadata
            .images
            .get(image_id)
            .cloned()
            .ok_or_else(|| ClientError::Demo(format!("missing image {image_id}")))
    }
}

impl DatasetApi for SpyApi {
    fn list_datasets<'a>(&'a self) -> ApiFuture<'a, Vec<DatasetSummary>> {
        let mut state = self.state.borrow_mut();
        state.counts.list_datasets += 1;
        let metadata = state.metadata.clone();
        ready(Ok(vec![DatasetSummary {
            dataset_id: metadata.dataset_id,
            name: metadata.name,
            roles: state.summary_roles.clone(),
            total_images: metadata.images.len(),
        }]))
    }

    fn create_dataset<'a>(
        &'a self,
        request: CreateDatasetRequest,
    ) -> ApiFuture<'a, DatasetMetadata> {
        let mut state = self.state.borrow_mut();
        state.counts.create_dataset += 1;
        let timestamp = now();
        let mut metadata = DatasetMetadata::new(request.dataset_id, request.name, timestamp);
        metadata.role_assignments = vec![DatasetRoleAssignment {
            dataset_id: metadata.dataset_id.clone(),
            user_id: request.admin_user_id,
            roles: BTreeSet::from([
                DatasetRole::DataAdmin,
                DatasetRole::Annotator,
                DatasetRole::Reviewer,
                DatasetRole::Adjudicator,
            ]),
            assigned_at: timestamp,
            assigned_by: None,
        }];
        state.metadata = metadata.clone();
        state.states.clear();
        ready(Ok(metadata))
    }

    fn get_dataset<'a>(&'a self, _dataset_id: &'a DatasetId) -> ApiFuture<'a, DatasetMetadata> {
        let mut state = self.state.borrow_mut();
        state.counts.get_dataset += 1;
        ready(Ok(state.metadata.clone()))
    }

    fn get_admin_dataset<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> ApiFuture<'a, DatasetMetadata> {
        let mut state = self.state.borrow_mut();
        state.counts.get_admin_dataset += 1;
        if dataset_id == &state.metadata.dataset_id {
            ready(Ok(state.metadata.clone()))
        } else {
            ready(Err(ClientError::Demo(format!(
                "missing dataset {dataset_id}"
            ))))
        }
    }

    fn update_dataset_config<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        request: UpdateDatasetConfigRequest,
    ) -> ApiFuture<'a, DatasetMetadata> {
        let mut state = self.state.borrow_mut();
        state.counts.update_dataset_config += 1;
        state.metadata.name = request.name;
        state.metadata.image_roots = request.image_roots;
        state.metadata.label_classes = request.label_classes;
        state.metadata.tasks = request.tasks;
        state.metadata.role_assignments = request.role_assignments;
        state.metadata.imbalance = request.imbalance;
        state.metadata.prelabel_configs = request.prelabel_configs;
        ready(Ok(state.metadata.clone()))
    }

    fn ingest_dataset<'a>(&'a self, _dataset_id: &'a DatasetId) -> ApiFuture<'a, IngestReport> {
        self.state.borrow_mut().counts.ingest_dataset += 1;
        ready(Ok(IngestReport {
            discovered_files: 2,
            new_images: 1,
            ..Default::default()
        }))
    }

    fn start_ingest_job<'a>(&'a self, dataset_id: &'a DatasetId) -> ApiFuture<'a, IngestJob> {
        self.state.borrow_mut().counts.ingest_dataset += 1;
        ready(Ok(IngestJob {
            job_id: "test-ingest".to_string(),
            dataset_id: dataset_id.clone(),
            status: IngestJobStatus::Completed,
            report: Some(IngestReport {
                discovered_files: 2,
                new_images: 1,
                ..Default::default()
            }),
            error: None,
        }))
    }

    fn get_ingest_job<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        job_id: &'a str,
    ) -> ApiFuture<'a, IngestJob> {
        ready(Ok(IngestJob {
            job_id: job_id.to_string(),
            dataset_id: dataset_id.clone(),
            status: IngestJobStatus::Completed,
            report: Some(IngestReport {
                discovered_files: 2,
                new_images: 1,
                ..Default::default()
            }),
            error: None,
        }))
    }

    fn create_snapshot<'a>(&'a self, dataset_id: &'a DatasetId) -> ApiFuture<'a, DatasetSnapshot> {
        let mut state = self.state.borrow_mut();
        state.counts.create_snapshot += 1;
        let snapshot = test_snapshot(dataset_id.clone());
        state.snapshots.insert(0, snapshot.clone());
        ready(Ok(snapshot))
    }

    fn list_snapshots<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
    ) -> ApiFuture<'a, Vec<DatasetSnapshot>> {
        let mut state = self.state.borrow_mut();
        state.counts.list_snapshots += 1;
        ready(Ok(state.snapshots.clone()))
    }

    fn get_snapshot_file<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _snapshot_id: &'a str,
        path: &'a str,
    ) -> ApiFuture<'a, SnapshotFile> {
        self.state.borrow_mut().counts.get_snapshot_file += 1;
        ready(Ok(SnapshotFile {
            file_name: path.to_string(),
            media_type: "application/json".to_string(),
            bytes: br#"{"snapshotId":"snapshot-test"}"#.to_vec(),
        }))
    }
}

impl TaskApi for SpyApi {
    fn list_tasks<'a>(&'a self, _dataset_id: &'a DatasetId) -> ApiFuture<'a, Vec<TaskDefinition>> {
        ready(Ok(self.state.borrow().metadata.tasks.clone()))
    }

    fn add_task<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        task: TaskDefinition,
    ) -> ApiFuture<'a, TaskDefinition> {
        ready(Ok(task))
    }
}

impl ImageApi for SpyApi {
    fn list_images<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        query: ImageExplorerQuery,
    ) -> ApiFuture<'a, ImageExplorerPage> {
        let mut state = self.state.borrow_mut();
        state.counts.list_images += 1;
        state.last_image_query = Some(query.clone());
        let mut items = state
            .metadata
            .images
            .values()
            .cloned()
            .map(|image| ImageExplorerItem {
                image,
                task_statuses: BTreeMap::from([(
                    TaskId::from("bounding_box:person"),
                    TaskStatus::Pending,
                )]),
                class_ids: BTreeSet::from([ClassId::from("person")]),
            })
            .filter(|item| {
                query.search.as_ref().is_none_or(|search| {
                    item.image.file_name.contains(search)
                        || item.image.canonical_path.contains(search)
                }) && query.status.as_ref().is_none_or(|status| {
                    item.task_statuses
                        .values()
                        .any(|existing| existing == status)
                }) && query
                    .task_id
                    .as_ref()
                    .is_none_or(|task_id| item.task_statuses.contains_key(task_id))
                    && query
                        .class_id
                        .as_ref()
                        .is_none_or(|class_id| item.class_ids.contains(class_id))
            })
            .collect::<Vec<_>>();
        let total_items = items.len();
        let page_size = query.page_size.max(1);
        let total_pages = total_items.div_ceil(page_size);
        let start = query.page.saturating_sub(1) * page_size;
        let items = if start < items.len() {
            items
                .drain(start..items.len().min(start + page_size))
                .collect()
        } else {
            Vec::new()
        };
        ready(Ok(ImageExplorerPage {
            items,
            page: query.page,
            page_size,
            total_items,
            total_pages,
        }))
    }

    fn assign_next_image<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        request: AssignNextRequest,
    ) -> ApiFuture<'a, Option<Assignment>> {
        let mut state = self.state.borrow_mut();
        state.counts.assign_next_image += 1;
        state.exclusions.push(request.excluded_image_ids.clone());
        if state.no_assignment {
            return ready(Ok(None));
        }
        let kind = request.kind.unwrap_or(AssignmentKind::Annotation);
        if let Some(active) = state.active_assignments.iter().find(|active| {
            request.assignment_id.as_ref() == Some(&active.assignment_id)
                || (active.task_id == request.task_id
                    && active.kind == kind
                    && !request.excluded_image_ids.contains(&active.image_id))
        }) {
            return ready(Ok(Some(active.clone())));
        }
        let image_ids = state.metadata.images.keys().cloned().collect::<Vec<_>>();
        let image_id = (0..image_ids.len()).find_map(|offset| {
            let image_id = image_ids[(state.next_image + offset) % image_ids.len()].clone();
            (!request.excluded_image_ids.contains(&image_id)
                && (kind != AssignmentKind::Annotation
                    || !state.completed_images.contains(&image_id))
                && !state
                    .active_assignments
                    .iter()
                    .any(|assignment| assignment.image_id == image_id))
            .then_some(image_id)
        });
        let Some(image_id) = image_id else {
            return ready(Ok(None));
        };
        if kind == AssignmentKind::Annotation {
            state.next_image += 1;
        }
        let assignment = Assignment {
            assignment_id: AssignmentId::generate(),
            image_id,
            task_id: request.task_id,
            assigned_to: UserId::from("admin"),
            kind,
            status: AssignmentStatus::Active,
            expires_at: None,
            created_at: now(),
            updated_at: now(),
        };
        state.active_assignments.push(assignment.clone());
        ready(Ok(Some(assignment)))
    }

    fn release_assignment<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        request: AssignmentActionRequest,
    ) -> ApiFuture<'a, Assignment> {
        let mut state = self.state.borrow_mut();
        state.counts.release_assignment += 1;
        let Some(position) = state
            .active_assignments
            .iter()
            .position(|assignment| assignment_matches(assignment, &request))
        else {
            return ready(Err(ClientError::Demo("no active assignment".to_string())));
        };
        let mut assignment = state.active_assignments.remove(position);
        assignment.status = AssignmentStatus::Cancelled;
        assignment.updated_at = now();
        ready(Ok(assignment))
    }

    fn complete_assignment<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        request: AssignmentActionRequest,
    ) -> ApiFuture<'a, Assignment> {
        let mut state = self.state.borrow_mut();
        state.counts.complete_assignment += 1;
        let Some(position) = state
            .active_assignments
            .iter()
            .position(|assignment| assignment_matches(assignment, &request))
        else {
            return ready(Err(ClientError::Demo("no active assignment".to_string())));
        };
        let mut assignment = state.active_assignments.remove(position);
        assignment.status = AssignmentStatus::Completed;
        assignment.updated_at = now();
        ready(Ok(assignment))
    }

    fn get_image_state<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> ApiFuture<'a, ImageState> {
        let mut state = self.state.borrow_mut();
        state.counts.get_image_state += 1;
        ready(Ok(state
            .states
            .get(image_id)
            .cloned()
            .unwrap_or_else(|| ImageState::new(image_id.clone()))))
    }

    fn get_image_record<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> ApiFuture<'a, ImageRecord> {
        let mut state = self.state.borrow_mut();
        state.counts.get_image_record += 1;
        ready(state.record(image_id))
    }

    fn get_image_file<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> ApiFuture<'a, ImageFile> {
        ready(Ok(ImageFile {
            image_id: image_id.clone(),
            media_type: "image/png".to_string(),
            bytes: Vec::new(),
        }))
    }

    fn get_image_preview<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        _max_dimension: u32,
    ) -> ApiFuture<'a, ImagePreview> {
        let mut state = self.state.borrow_mut();
        state.counts.get_image_preview += 1;
        if state.fail_next_preview {
            state.fail_next_preview = false;
            return ready(Err(ClientError::Demo("preview failed".to_string())));
        }
        ready(Ok(ImagePreview {
            image_id: image_id.clone(),
            width: 4,
            height: 3,
            rgba: [32, 48, 64, 255].repeat(12),
        }))
    }

    fn rebuild_image<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> ApiFuture<'a, ImageState> {
        let mut state = self.state.borrow_mut();
        state.counts.rebuild_image += 1;
        ready(Ok(state
            .states
            .get(image_id)
            .cloned()
            .unwrap_or_else(|| ImageState::new(image_id.clone()))))
    }
}

impl AnnotationApi for SpyApi {
    fn append_event<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: AppendEventRequest,
    ) -> ApiFuture<'a, EventLogEntry> {
        let mut state = self.state.borrow_mut();
        state.counts.append_event += 1;
        state.events.push(request.payload.clone());
        let image_state = state
            .states
            .entry(image_id.clone())
            .or_insert_with(|| ImageState::new(image_id.clone()));
        let event = EventLogEntry::new(
            image_state.current_sequence + 1,
            image_id.clone(),
            UserId::from("admin"),
            DatasetRole::Annotator,
            now(),
            request.payload,
        );
        image_state.apply_event(&event).unwrap();
        ready(Ok(event))
    }

    fn append_assigned_event<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        request: AppendEventRequest,
    ) -> ApiFuture<'a, EventLogEntry> {
        if !self
            .state
            .borrow()
            .active_assignments
            .iter()
            .any(|active| assignment_matches(active, &assignment))
        {
            return ready(Err(ClientError::Demo("stale assignment".to_string())));
        }
        let mut state = self.state.borrow_mut();
        state.counts.append_event += 1;
        state.events.push(request.payload.clone());
        let image_state = state
            .states
            .entry(assignment.image_id.clone())
            .or_insert_with(|| ImageState::new(assignment.image_id.clone()));
        let event = EventLogEntry::new(
            image_state.current_sequence + 1,
            assignment.image_id.clone(),
            UserId::from("admin"),
            DatasetRole::Annotator,
            now(),
            request.payload,
        );
        image_state.apply_event(&event).unwrap();
        ready(Ok(event))
    }

    fn apply_annotation_batch<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        request: AnnotationBatchRequest,
    ) -> ApiFuture<'a, ImageState> {
        if !self
            .state
            .borrow()
            .active_assignments
            .iter()
            .any(|active| assignment_matches(active, &assignment))
        {
            return ready(Err(ClientError::Demo("stale assignment".to_string())));
        }
        let mut state = self.state.borrow_mut();
        if state.fail_next_batch {
            state.fail_next_batch = false;
            return ready(Err(ClientError::Demo(
                "annotation batch failed".to_string(),
            )));
        }
        state.counts.annotation_batch += 1;
        let complete = request.complete;
        if complete {
            state.counts.complete_assignment += 1;
        }
        for payload in request.payloads {
            state.counts.append_event += 1;
            state.events.push(payload.clone());
            let image_state = state
                .states
                .entry(assignment.image_id.clone())
                .or_insert_with(|| ImageState::new(assignment.image_id.clone()));
            let event = EventLogEntry::new(
                image_state.current_sequence + 1,
                assignment.image_id.clone(),
                UserId::from("admin"),
                DatasetRole::Annotator,
                now(),
                payload,
            );
            image_state.apply_event(&event).unwrap();
        }
        let mut result = state
            .states
            .get(&assignment.image_id)
            .cloned()
            .unwrap_or_else(|| ImageState::new(assignment.image_id.clone()));
        if complete {
            state.completed_images.insert(assignment.image_id.clone());
            state
                .active_assignments
                .retain(|active| !assignment_matches(active, &assignment));
        } else if let Some(renewed) = state
            .active_assignments
            .iter_mut()
            .find(|active| assignment_matches(active, &assignment))
        {
            renewed.updated_at = now();
            renewed.expires_at = Some(now() + chrono::Duration::minutes(15));
            result
                .assignments
                .retain(|existing| existing.assignment_id != renewed.assignment_id);
            result.assignments.push(renewed.clone());
        }
        ready(Ok(result))
    }
}

impl ReviewApi for SpyApi {
    fn record_review<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        review: ReviewRecord,
    ) -> ApiFuture<'a, ImageState> {
        let mut state = self.state.borrow_mut();
        state.counts.record_review += 1;
        let image_state = state
            .states
            .entry(image_id.clone())
            .or_insert_with(|| ImageState::new(image_id.clone()));
        let event = EventLogEntry::new(
            image_state.current_sequence + 1,
            image_id.clone(),
            UserId::from("admin"),
            DatasetRole::Reviewer,
            now(),
            EventPayload::ReviewRecorded { review },
        );
        image_state.apply_event(&event).unwrap();
        ready(Ok(image_state.clone()))
    }

    fn record_assigned_review<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        review: ReviewRecord,
    ) -> ApiFuture<'a, ImageState> {
        if !self
            .state
            .borrow()
            .active_assignments
            .iter()
            .any(|active| assignment_matches(active, &assignment))
        {
            return ready(Err(ClientError::Demo("stale assignment".to_string())));
        }
        let complete = matches!(&review.target, labello_domain::ReviewTarget::Task { .. });
        let mut state = self.state.borrow_mut();
        state.counts.record_review += 1;
        let mut renewed = state
            .active_assignments
            .iter()
            .find(|active| assignment_matches(active, &assignment))
            .cloned();
        if !complete && let Some(assignment) = renewed.as_mut() {
            assignment.updated_at = now();
            assignment.expires_at = Some(now() + chrono::Duration::minutes(15));
        }
        let image_state = state
            .states
            .entry(assignment.image_id.clone())
            .or_insert_with(|| ImageState::new(assignment.image_id.clone()));
        let event = EventLogEntry::new(
            image_state.current_sequence + 1,
            assignment.image_id.clone(),
            UserId::from("admin"),
            DatasetRole::Reviewer,
            now(),
            EventPayload::ReviewRecorded { review },
        );
        image_state.apply_event(&event).unwrap();
        if !complete && let Some(renewed) = renewed.clone() {
            image_state
                .assignments
                .retain(|existing| existing.assignment_id != renewed.assignment_id);
            image_state.assignments.push(renewed);
        } else if complete {
            image_state
                .assignments
                .retain(|existing| existing.assignment_id != assignment.assignment_id);
        }
        let result = image_state.clone();
        if complete {
            state
                .active_assignments
                .retain(|active| !assignment_matches(active, &assignment));
        } else if let Some(renewed) = renewed
            && let Some(active) = state
                .active_assignments
                .iter_mut()
                .find(|active| active.assignment_id == renewed.assignment_id)
        {
            *active = renewed;
        }
        ready(Ok(result))
    }

    fn record_correction<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: CorrectionRequest,
    ) -> ApiFuture<'a, EventLogEntry> {
        let mut state = self.state.borrow_mut();
        state.counts.record_correction += 1;
        state.last_correction = Some(request.clone());
        if state.fail_next_correction {
            state.fail_next_correction = false;
            return ready(Err(ClientError::Demo("correction conflict".to_string())));
        }
        state
            .active_assignments
            .retain(|active| active.image_id != *image_id);
        ready(Ok(EventLogEntry::new(
            1,
            image_id.clone(),
            UserId::from("admin"),
            DatasetRole::Reviewer,
            now(),
            EventPayload::ReviewRecorded {
                review: ReviewRecord {
                    review_id: ReviewId::generate(),
                    target: ReviewTarget::AnnotationVersion {
                        annotation_id: request.annotation_id,
                        version: request.expected_version,
                    },
                    reviewer_user_id: UserId::from("admin"),
                    decision: labello_domain::ReviewDecision::Rejected,
                    timestamp: now(),
                    comment: request.reason,
                },
            },
        )))
    }
}

impl AdjudicationApi for SpyApi {
    fn record_adjudication<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        adjudication: AdjudicationRecord,
    ) -> ApiFuture<'a, EventLogEntry> {
        let mut state = self.state.borrow_mut();
        state.counts.record_adjudication += 1;
        let image_state = state
            .states
            .entry(image_id.clone())
            .or_insert_with(|| ImageState::new(image_id.clone()));
        let event = EventLogEntry::new(
            image_state.current_sequence + 1,
            image_id.clone(),
            UserId::from("admin"),
            DatasetRole::Adjudicator,
            now(),
            EventPayload::AdjudicationRecorded { adjudication },
        );
        image_state.apply_event(&event).unwrap();
        ready(Ok(event))
    }

    fn record_assigned_adjudication<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        adjudication: AdjudicationRecord,
    ) -> ApiFuture<'a, EventLogEntry> {
        if !self
            .state
            .borrow()
            .active_assignments
            .iter()
            .any(|active| assignment_matches(active, &assignment))
        {
            return ready(Err(ClientError::Demo("stale assignment".to_string())));
        }
        let mut state = self.state.borrow_mut();
        state.counts.record_adjudication += 1;
        let image_state = state
            .states
            .entry(assignment.image_id.clone())
            .or_insert_with(|| ImageState::new(assignment.image_id.clone()));
        let event = EventLogEntry::new(
            image_state.current_sequence + 1,
            assignment.image_id.clone(),
            UserId::from("admin"),
            DatasetRole::Adjudicator,
            now(),
            EventPayload::AdjudicationRecorded { adjudication },
        );
        image_state.apply_event(&event).unwrap();
        state
            .active_assignments
            .retain(|active| !assignment_matches(active, &assignment));
        ready(Ok(event))
    }
}

impl OfflineApi for SpyApi {
    fn offline_bundle<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _request: OfflineBundleRequest,
    ) -> ApiFuture<'a, OfflineBundle> {
        let state = self.state.borrow();
        ready(Ok(OfflineBundle {
            schema_version: SCHEMA_VERSION,
            dataset_id: state.metadata.dataset_id.clone(),
            user_id: UserId::from("admin"),
            created_at: now(),
            expires_at: None,
            roles: vec![DatasetRole::Annotator],
            tasks: state.metadata.tasks.clone(),
            images: Vec::new(),
        }))
    }

    fn sync_offline_events<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _request: OfflineSyncRequest,
    ) -> ApiFuture<'a, OfflineSyncResult> {
        ready(Ok(OfflineSyncResult {
            merged_events: 0,
            conflicts: Vec::new(),
        }))
    }
}

impl StatsApi for SpyApi {
    fn dataset_stats<'a>(&'a self, _dataset_id: &'a DatasetId) -> ApiFuture<'a, DatasetStats> {
        let mut state = self.state.borrow_mut();
        state.counts.dataset_stats += 1;
        ready(Ok(stats(2)))
    }
}

impl KeybindingApi for SpyApi {
    fn get_keybindings<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        user_id: &'a UserId,
    ) -> ApiFuture<'a, KeybindingSet> {
        self.state.borrow_mut().counts.get_keybindings += 1;
        ready(Ok(KeybindingSet::defaults_for(user_id.clone())))
    }

    fn save_keybindings<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        keybindings: KeybindingSet,
    ) -> ApiFuture<'a, KeybindingSet> {
        self.state.borrow_mut().counts.save_keybindings += 1;
        ready(Ok(keybindings))
    }
}

impl PrelabelApi for SpyApi {
    fn list_prelabel_configs<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
    ) -> ApiFuture<'a, Vec<PrelabelConfig>> {
        ready(Ok(self.state.borrow().metadata.prelabel_configs.clone()))
    }

    fn add_prelabel_config<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        config: PrelabelConfig,
    ) -> ApiFuture<'a, PrelabelConfig> {
        ready(Ok(config))
    }

    fn prelabel_suggestions<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        request: PrelabelSuggestionRequest,
    ) -> ApiFuture<'a, Vec<PrelabelSuggestion>> {
        self.state.borrow_mut().counts.prelabel_suggestions += 1;
        ready(Ok(vec![PrelabelSuggestion {
            suggestion_id: "suggestion-1".to_string(),
            config_id: request.config_id,
            task_id: request.task_id,
            class_id: ClassId::from("person"),
            confidence: 0.88,
            geometry: AnnotationGeometry::BoundingBox(BoundingBox {
                x: 0.1,
                y: 0.1,
                width: 0.25,
                height: 0.35,
            }),
        }]))
    }
}

impl AuthApi for SpyApi {
    fn auth_options<'a>(&'a self) -> ApiFuture<'a, AuthOptions> {
        self.state.borrow_mut().counts.auth_options += 1;
        ready(Ok(AuthOptions {
            github_oauth: true,
            local_admin_login: true,
        }))
    }

    fn local_admin_login<'a>(&'a self) -> ApiFuture<'a, UserAccount> {
        let mut state = self.state.borrow_mut();
        state.counts.local_admin_login += 1;
        ready(Ok(state.users[0].account.clone()))
    }

    fn github_login_url<'a>(&'a self, request: OAuthLoginRequest) -> ApiFuture<'a, String> {
        self.state.borrow_mut().last_oauth_return_to = request.return_to;
        ready(Ok("https://example.invalid/login".to_string()))
    }

    fn github_callback<'a>(&'a self, _request: OAuthCallbackRequest) -> ApiFuture<'a, UserAccount> {
        ready(Ok(UserAccount {
            user_id: UserId::from("admin"),
            display_name: "Admin".to_string(),
            github_user_id: None,
            github_login: None,
            created_at: now(),
            updated_at: now(),
        }))
    }

    fn me<'a>(&'a self) -> ApiFuture<'a, UserAccount> {
        let mut state = self.state.borrow_mut();
        state.counts.me += 1;
        if state.fail_me {
            ready(Err(ClientError::Api {
                status: 401,
                message: "login required".to_string(),
            }))
        } else {
            ready(Ok(state.users[0].account.clone()))
        }
    }

    fn logout<'a>(&'a self) -> ApiFuture<'a, ()> {
        self.state.borrow_mut().counts.logout += 1;
        ready(Ok(()))
    }
}

impl UserApi for SpyApi {
    fn list_dataset_users<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
    ) -> ApiFuture<'a, Vec<DatasetUser>> {
        let mut state = self.state.borrow_mut();
        state.counts.list_dataset_users += 1;
        ready(Ok(state.users.clone()))
    }

    fn set_dataset_roles<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        request: SetDatasetRolesRequest,
    ) -> ApiFuture<'a, DatasetUser> {
        let mut state = self.state.borrow_mut();
        state.counts.set_dataset_roles += 1;
        let user = state
            .users
            .iter_mut()
            .find(|user| user.account.user_id == request.user_id)
            .unwrap();
        user.roles = request.roles;
        ready(Ok(user.clone()))
    }
}

fn ready<'a, T: 'a>(result: ClientResult<T>) -> ApiFuture<'a, T> {
    Box::pin(async move { result })
}

fn assignment_matches(assignment: &Assignment, request: &AssignmentActionRequest) -> bool {
    assignment.assignment_id == request.assignment_id
        && assignment.image_id == request.image_id
        && assignment.task_id == request.task_id
        && assignment.kind == request.kind
        && assignment.status == AssignmentStatus::Active
}

fn image_record(image_id: &str, file_name: &str, width: u32, height: u32) -> ImageRecord {
    ImageRecord {
        image_id: ImageId::from(image_id),
        blake3: format!("hash-{image_id}"),
        canonical_path: format!("images/{file_name}"),
        known_paths: vec![format!("images/{file_name}")],
        duplicate_paths: Vec::new(),
        file_name: file_name.to_string(),
        byte_size: 64,
        width,
        height,
        media_type: "image/png".to_string(),
    }
}

fn test_snapshot(dataset_id: DatasetId) -> DatasetSnapshot {
    DatasetSnapshot {
        schema_version: SCHEMA_VERSION,
        snapshot_id: "snapshot-test".to_string(),
        dataset_id,
        created_at: now(),
        includes_image_bytes: false,
        total_bytes: 32,
        files: vec![SnapshotFileEntry {
            path: "snapshot.json".to_string(),
            byte_size: 32,
            blake3: "snapshot-hash".to_string(),
        }],
    }
}

fn task(id: &str, name: &str, prelabel_configs: Vec<&str>) -> TaskDefinition {
    let class_id = id.split(':').nth(1).unwrap_or("person");
    TaskDefinition {
        task_id: TaskId::from(id),
        name: name.to_string(),
        annotation_type: AnnotationType::BoundingBox,
        class_ids: vec![ClassId::from(class_id)],
        instructions: TutorialContent {
            title: "Label every visible person".to_string(),
            example_text: "Draw tight boxes around every person.".to_string(),
            example_images: vec!["tutorial/example.png".to_string()],
        },
        skeleton: None,
        review: ReviewConfig::default(),
        prelabel_config_ids: prelabel_configs
            .into_iter()
            .map(PrelabelConfigId::from)
            .collect(),
        enabled: true,
    }
}

fn seed_review_annotation(
    api: &SpyApi,
    geometry: AnnotationGeometry,
    allow_reviewer_corrections: bool,
) -> labello_domain::AnnotationId {
    let mut spy = api.state.borrow_mut();
    let annotation_type = match &geometry {
        AnnotationGeometry::BoundingBox(_) => AnnotationType::BoundingBox,
        AnnotationGeometry::Skeleton(_) => AnnotationType::Skeleton,
    };
    spy.metadata.tasks[0].annotation_type = annotation_type.clone();
    spy.metadata.tasks[0].review.allow_reviewer_corrections = allow_reviewer_corrections;
    let task_id = spy.metadata.tasks[0].task_id.clone();
    let class_id = spy.metadata.tasks[0].class_ids[0].clone();
    let image_id = spy.metadata.images.keys().next().unwrap().clone();
    let annotation_id = labello_domain::AnnotationId::from("review_annotation");
    let timestamp = now();
    let annotation = labello_domain::AnnotationVersion {
        annotation_id: annotation_id.clone(),
        version: 1,
        task_id,
        class_id,
        annotation_type,
        source: labello_domain::AnnotationSource::Human,
        geometry,
        author_user_id: UserId::from("annotator"),
        created_at: timestamp,
        updated_at: timestamp,
        deleted: false,
    };
    let event = EventLogEntry::new(
        1,
        image_id.clone(),
        UserId::from("annotator"),
        DatasetRole::Annotator,
        timestamp,
        EventPayload::AnnotationVersionCreated {
            annotation,
            previous_version: None,
            reason: None,
        },
    );
    spy.states
        .get_mut(&image_id)
        .unwrap()
        .apply_event(&event)
        .unwrap();
    annotation_id
}

fn prelabel_config(id: &str) -> PrelabelConfig {
    PrelabelConfig {
        config_id: PrelabelConfigId::from(id),
        name: "Demo prelabels".to_string(),
        model: ModelSpec {
            model_id: "model".to_string(),
            display_name: "Demo model".to_string(),
            version: Some("1".to_string()),
            location: "browser".to_string(),
        },
        execution: PrelabelExecution::BrowserLocal {
            acceleration: BrowserAcceleration::WasmCpuFallback,
        },
        output_processing: OutputProcessing {
            confidence_threshold: 0.5,
            suppress_overlaps_iou: None,
        },
        available_to_annotators: true,
    }
}

fn stats(total_images: usize) -> DatasetStats {
    let mut per_task = BTreeMap::new();
    per_task.insert(
        TaskId::from("bounding_box:person"),
        labello_domain::TaskStats {
            completed: 1,
            pending: 1,
            reviewed: 1,
            unreviewed: 1,
            approved: 1,
            rejected: 0,
            reviewer_corrected: 0,
            finalized: 1,
        },
    );
    let mut per_class = BTreeMap::new();
    per_class.insert(
        ClassId::from("person"),
        labello_domain::ClassStats {
            annotations: 2,
            completed_tasks: 1,
        },
    );
    DatasetStats {
        total_images,
        completed_tasks: 1,
        pending_tasks: 1,
        reviewed_tasks: 1,
        unreviewed_tasks: 1,
        approved_tasks: 1,
        rejected_tasks: 0,
        reviewer_corrected_tasks: 0,
        finalized_tasks: 1,
        per_task,
        per_class,
        throughput: Vec::new(),
    }
}

fn now() -> labello_domain::Timestamp {
    labello_domain::now()
}
