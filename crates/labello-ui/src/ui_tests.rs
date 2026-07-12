use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};

use eframe::egui;
use egui_kittest::{Harness, kittest::Queryable};
use labello_client::{
    AdjudicationApi, AnnotationApi, ApiFuture, AppendEventRequest, AssignNextRequest,
    AssignmentActionRequest, AuthApi, ClientError, ClientResult, CorrectionRequest,
    CreateDatasetRequest, DatasetApi, DatasetSummary, ImageApi, ImageFile, ImagePreview, IngestJob,
    IngestJobStatus, IngestReport, KeybindingApi, OAuthCallbackRequest, OAuthLoginRequest,
    OfflineApi, OfflineBundleRequest, PrelabelApi, PrelabelSuggestionRequest, ReviewApi, StatsApi,
    TaskApi, UpdateDatasetConfigRequest,
};
use labello_domain::{
    AdjudicationRecord, AnnotationGeometry, AnnotationType, Assignment, AssignmentId,
    AssignmentKind, AssignmentStatus, BoundingBox, BrowserAcceleration, ClassId, DatasetId,
    DatasetMetadata, DatasetRole, DatasetRoleAssignment, DatasetStats, EventLogEntry, EventPayload,
    ImageId, ImageRecord, ImageState, KeybindingSet, KeypointSpec, LabelClass, ModelSpec,
    OfflineBundle, OfflineSyncRequest, OfflineSyncResult, OutputProcessing, PrelabelConfig,
    PrelabelConfigId, PrelabelExecution, PrelabelSuggestion, ReviewConfig, ReviewRecord,
    SCHEMA_VERSION, SkeletonSpec, TaskDefinition, TaskId, TutorialContent, UserAccount, UserId,
};

use crate::app::{
    AppConfig, AppView, FolderUploadProgress, IMAGE_QUEUE_SIZE, LabelloApp, SaveStatus, UiMessage,
};
use crate::canvas::BoundingBoxEdit;

#[test]
fn setup_create_open_and_admin_workflows_use_live_commands() {
    let api = Rc::new(SpyApi::new());
    let mut harness = live_harness(api.clone());
    step_until(&mut harness, 8, |app| app.datasets.summaries.len() == 1);

    assert!(harness.query_by_label("Connect To Labello").is_some());
    assert!(harness.query_all_by_label("Annotate").next().is_some());
    assert!(harness.query_all_by_label("Admin").next().is_some());

    click(&mut harness, "Create as current user");
    step_until(&mut harness, 20, |app| app.current.is_some());
    assert_eq!(api.counts().create_dataset, 1);
    assert_eq!(harness.state().view, AppView::Annotate);
    assert!(harness.state().current.is_some());

    click(&mut harness, "Setup");
    release_and_switch(&mut harness);
    step_until(&mut harness, 8, |app| app.view == AppView::Setup);
    click(&mut harness, "Admin");
    step_until(&mut harness, 8, |app| app.view == AppView::Admin);
    assert_eq!(api.counts().get_admin_dataset, 1);
    assert!(harness.query_by_label("Dataset Admin").is_some());
}

#[test]
fn admin_workflow_saves_ingests_and_handles_browser_only_folder_upload() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_admin_harness(api.clone());
    harness.set_size(egui::vec2(1180.0, 4000.0));
    harness.step();

    assert!(harness.query_by_label("Dataset Admin").is_some());
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

    click_accesskit_button(&mut harness, "Add image root");
    harness.step();
    click_accesskit_button(&mut harness, "Add bounding box class workflow");
    harness.step();
    click_accesskit_button(&mut harness, "Add browser prelabel config");
    harness.step();
    click_accesskit_button(&mut harness, "Add role assignment");
    harness.step();

    let config = harness.state().datasets.admin_config.as_ref().unwrap();
    assert_eq!(config.image_roots.len(), 2);
    assert_eq!(config.label_classes.len(), 3);
    assert_eq!(config.prelabel_configs.len(), 2);
    assert_eq!(config.role_assignments.len(), 2);
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

    click(&mut harness, "Admin");
    step_until(&mut harness, 8, |app| app.view == AppView::Admin);
    let before_ingest = api.counts();
    harness.state_mut().request_ingest();
    harness.step();
    let badge = harness.get_by_label("Dataset demo");
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

    click(&mut harness, "Annotate");
    step_until(&mut harness, 12, |app| app.current.is_some());
    assert_eq!(harness.state().view, AppView::Annotate);
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

    click(&mut harness, "Discard staged changes");
    assert_eq!(
        harness.state().datasets.admin_config.as_ref().unwrap().name,
        original_name
    );
    assert_eq!(api.counts().get_admin_dataset, 1);
}

#[test]
fn image_load_failure_shows_retry_and_loads_image() {
    let api = Rc::new(SpyApi::new());
    api.fail_next_preview();
    let mut harness = live_harness(api.clone());
    step_until(&mut harness, 8, |app| app.datasets.summaries.len() == 1);
    click(&mut harness, "Annotate");
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
    assert!(harness.query_by_label("Retry image load").is_some());
    click(&mut harness, "Retry image load");
    step_until(&mut harness, 12, |app| app.current.is_some());
    assert!(api.counts().get_image_preview >= 2);
    assert_eq!(api.counts().assign_next_image, 1);
}

#[test]
fn workers_select_class_specific_workflows() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);

    assert!(
        harness
            .query_all_by_label("Person bounding box")
            .next()
            .is_some()
    );
    assert!(
        harness
            .query_all_by_label("Vehicle bounding box")
            .next()
            .is_some()
    );
    click(&mut harness, "Vehicle bounding box");
    release_and_switch(&mut harness);
    step_until(&mut harness, 12, |app| {
        app.selected_class_id() == Some(&ClassId::from("vehicle")) && app.current.is_some()
    });

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
    click(&mut harness, "Annotate");
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
    click(&mut harness, "Annotate");
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
            operation_id: u64::MAX,
            assignment_id: AssignmentId::generate(),
            completed: false,
            result: Ok(ImageState::new(ImageId::from("img_stale"))),
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
    click(&mut harness, "Settings");
    assert!(harness.query_by_label("Keyboard shortcuts").is_some());
    click(&mut harness, "Reset defaults");
    click(&mut harness, "Save shortcuts");
    step_until(&mut harness, 8, |app| !app.loading.keybindings);

    assert_eq!(api.counts().save_keybindings, 1);
    assert_eq!(
        harness.state().runtime.notice.as_deref(),
        Some("Keyboard shortcuts saved")
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

    harness.key_press(egui::Key::ArrowRight);
    step_until(&mut harness, 16, |app| {
        app.assignment
            .as_ref()
            .is_some_and(|assignment| assignment.image_id != original)
    });

    assert_eq!(api.counts().complete_assignment, 1);
    assert_eq!(api.counts().release_assignment, 0);
}

#[test]
fn save_keeps_the_same_assignment_active() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
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
    assert_eq!(api.counts().assign_next_image, 1);
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

    click(&mut harness, "Skip");
    step_until(&mut harness, 16, |app| {
        app.assignment
            .as_ref()
            .is_some_and(|assignment| assignment.image_id != original)
    });

    assert_eq!(api.counts().release_assignment, 1);
    assert_eq!(api.counts().complete_assignment, 0);
    assert_eq!(api.counts().assign_next_image, 2);
}

#[test]
fn dataset_summary_roles_survive_sanitized_metadata_and_show_all_tabs() {
    let api = Rc::new(SpyApi::new());
    api.sanitize_metadata_roles();
    let harness = loaded_work_harness(api);

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
    let harness = loaded_work_harness(api);

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
    harness
        .state()
        .runtime
        .tx
        .send(UiMessage::SaveFinished {
            operation_id: 76,
            assignment_id: assignment.assignment_id.clone(),
            completed: false,
            result: Ok(state.clone()),
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
            operation_id: 77,
            assignment_id: assignment.assignment_id,
            completed: false,
            result: Ok(state),
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
    for width in [900.0, 1024.0, 1180.0, 1280.0, 1500.0] {
        harness.set_size(egui::vec2(width, 820.0));
        harness.step();
        let canvas = harness.get_by_label("Annotation canvas");
        assert!(
            canvas.rect().width() >= 560.0,
            "canvas too narrow at {width}"
        );
        assert!(
            canvas.rect().height() >= 500.0,
            "canvas too short at {width}"
        );
        for label in ["Save", "Submit & next", "Skip"] {
            assert_eq!(
                harness
                    .query_all_by_role_and_label(egui::accesskit::Role::Button, label)
                    .count(),
                1,
                "duplicate {label} controls at {width}"
            );
        }
        if width < 1240.0 {
            assert!(harness.query_by_label("Workflow").is_some());
            assert!(harness.query_by_label("Inspector").is_some());
        }
    }
}

#[test]
fn work_workflow_draws_saves_submits_reviews_and_adjudicates() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    assert!(harness.state().current.is_some());
    assert_eq!(harness.state().queue.queue_size(), IMAGE_QUEUE_SIZE);
    assert!(harness.query_by_label("Assignment").is_some());
    assert!(harness.query_by_label("Approve  Y").is_none());
    assert!(harness.query_by_label("Reject  N").is_none());
    assert!(harness.query_by_label("Adjudicate accept").is_none());

    click(&mut harness, "Tutorial");
    harness.step();
    assert!(
        harness
            .query_by_label("Label every visible person")
            .is_some()
    );

    click(&mut harness, "Accept");
    harness.step();
    assert_eq!(harness.state().annotations.len(), 1);
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
    assert_eq!(counts.rebuild_image, 1);

    click(&mut harness, "Submit & next");
    step_until(&mut harness, 10, |app| {
        app.current
            .as_ref()
            .is_some_and(|current| current.image.image_id == ImageId::from("img_2"))
    });
    assert_eq!(api.counts().complete_assignment, 1);

    assert!(api.counts().assign_next_image >= 2);

    click(&mut harness, "Review");
    assert!(
        harness
            .query_by_label("Switch active assignment?")
            .is_some()
    );
    release_and_switch(&mut harness);
    step_until(&mut harness, 10, |app| {
        app.view == AppView::Review && !app.loading.image
    });
    assert!(harness.query_by_label("Approve  Y").is_some());
    assert!(harness.query_by_label("Reject  N").is_some());
    assert!(harness.query_by_label("Accept").is_none());
    harness.key_press(egui::Key::Y);
    harness.step();
    step_until(&mut harness, 10, |app| !app.loading.saving);
    assert_eq!(api.counts().record_review, 1);

    click(&mut harness, "Reject  N");
    step_until(&mut harness, 10, |app| !app.loading.saving);
    // Rejecting an object records the object decision and the task-level
    // correction outcome that closes the review assignment.
    assert_eq!(api.counts().record_review, 3);

    click(&mut harness, "Adjudicate");
    assert!(
        harness
            .query_by_label("Switch active assignment?")
            .is_some()
    );
    release_and_switch(&mut harness);
    step_until(&mut harness, 10, |app| {
        app.view == AppView::Adjudicate && !app.loading.image
    });
    assert!(harness.query_by_label("Adjudicate accept").is_some());
    assert!(harness.query_by_label("Needs correction").is_some());
    assert!(harness.query_by_label("Approve  Y").is_none());
    click(&mut harness, "Adjudicate accept");
    step_until(&mut harness, 10, |app| !app.loading.saving);
    click(&mut harness, "Needs correction");
    step_until(&mut harness, 10, |app| !app.loading.saving);
    assert_eq!(api.counts().record_adjudication, 2);

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
    click(&mut harness, "Vehicle bounding box");
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
    assert!(api.counts().rebuild_image >= 1);
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
fn stats_and_responsive_layouts_render_without_losing_primary_actions() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    assert_eq!(api.counts().dataset_stats, 0);

    click(&mut harness, "Stats");
    release_and_switch(&mut harness);
    step_until(&mut harness, 8, |app| app.view == AppView::Stats);
    harness.step();
    assert!(harness.query_by_label("Live Statistics").is_some());
    click(&mut harness, "Refresh now");
    step_until(&mut harness, 8, |app| !app.loading.stats);
    assert!(api.counts().dataset_stats >= 1);

    harness.set_size(egui::vec2(390.0, 760.0));
    harness.step();
    assert!(harness.query_by_label("Setup").is_some());
    assert!(harness.query_by_label("Annotate").is_some());
    assert!(harness.query_by_label("Stats").is_some());

    harness.set_size(egui::vec2(1280.0, 820.0));
    harness.step();
    click(&mut harness, "Annotate");
    harness.step();
    assert!(harness.query_by_label("Save").is_some());
    assert!(harness.query_by_label("Submit & next").is_some());
    assert!(harness.query_by_label("Skip").is_some());
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
    for index in 0..20 {
        app.runtime
            .tx
            .send(UiMessage::StatsLoaded(Ok(stats(index))))
            .unwrap();
    }
    app.process_messages(&egui::Context::default());
    assert_eq!(app.datasets.stats.total_images, 7);
    app.process_messages(&egui::Context::default());
    assert_eq!(app.datasets.stats.total_images, 15);

    app.runtime
        .tx
        .send(UiMessage::FolderUploadProgress(FolderUploadProgress {
            uploaded_files: 12,
            total_files: 24,
            current_batch: 2,
            message: "Uploading batch 2".to_string(),
        }))
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
    click(&mut harness, "Annotate");
    step_until(&mut harness, 12, |app| app.current.is_some());
    harness
}

fn loaded_admin_harness(api: Rc<SpyApi>) -> Harness<'static, LabelloApp> {
    let mut harness = live_harness(api);
    step_until(&mut harness, 8, |app| app.datasets.summaries.len() == 1);
    click(&mut harness, "Admin");
    step_until(&mut harness, 8, |app| app.view == AppView::Admin);
    harness
}

fn base_live_app(api: Rc<SpyApi>) -> LabelloApp {
    let mut app = LabelloApp::live_http(AppConfig {
        api_base_url: "http://example.invalid".to_string(),
        dev_token: "dev".to_string(),
        user_id: UserId::from("admin"),
        dataset_id: DatasetId::from("demo"),
        queue_size: IMAGE_QUEUE_SIZE,
    });
    app.runtime.api = Some(api);
    app.runtime.error = None;
    app
}

fn click(harness: &mut Harness<'static, LabelloApp>, label: &str) {
    let clicked = click_visible(harness, label);
    assert!(clicked, "button or label {label:?} was not visible");
    harness.step();
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
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct CallCounts {
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
    rebuild_image: usize,
    record_review: usize,
    record_adjudication: usize,
    dataset_stats: usize,
    get_keybindings: usize,
    save_keybindings: usize,
    prelabel_suggestions: usize,
}

struct SpyState {
    metadata: DatasetMetadata,
    states: BTreeMap<ImageId, ImageState>,
    counts: CallCounts,
    next_image: usize,
    events: Vec<EventPayload>,
    fail_next_preview: bool,
    no_assignment: bool,
    active_assignment: Option<Assignment>,
    summary_roles: Vec<DatasetRole>,
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
        let states = [image_1, image_2]
            .into_iter()
            .map(|image| (image.image_id.clone(), ImageState::new(image.image_id)))
            .collect();

        Self {
            metadata,
            states,
            counts: CallCounts::default(),
            next_image: 0,
            events: Vec::new(),
            fail_next_preview: false,
            no_assignment: false,
            active_assignment: None,
            summary_roles: vec![
                DatasetRole::DataAdmin,
                DatasetRole::Annotator,
                DatasetRole::Reviewer,
                DatasetRole::Adjudicator,
            ],
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
        state.metadata.dataset_id = request.dataset_id;
        state.metadata.name = request.name;
        ready(Ok(state.metadata.clone()))
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
    fn assign_next_image<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        request: AssignNextRequest,
    ) -> ApiFuture<'a, Option<Assignment>> {
        let mut state = self.state.borrow_mut();
        state.counts.assign_next_image += 1;
        if state.no_assignment {
            return ready(Ok(None));
        }
        let kind = request.kind.unwrap_or(AssignmentKind::Annotation);
        if let Some(active) = state.active_assignment.as_ref()
            && active.task_id == request.task_id
            && active.kind == kind
        {
            return ready(Ok(Some(active.clone())));
        }
        let image_id = state
            .metadata
            .images
            .keys()
            .nth(state.next_image % state.metadata.images.len())
            .cloned()
            .unwrap();
        state.next_image += 1;
        let assignment = Assignment {
            assignment_id: AssignmentId::generate(),
            image_id,
            task_id: request.task_id,
            assigned_to: UserId::from("admin"),
            kind,
            status: AssignmentStatus::Active,
            created_at: now(),
            updated_at: now(),
        };
        state.active_assignment = Some(assignment.clone());
        ready(Ok(Some(assignment)))
    }

    fn release_assignment<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        request: AssignmentActionRequest,
    ) -> ApiFuture<'a, Assignment> {
        let mut state = self.state.borrow_mut();
        state.counts.release_assignment += 1;
        let Some(mut assignment) = state.active_assignment.take() else {
            return ready(Err(ClientError::Demo("no active assignment".to_string())));
        };
        if !assignment_matches(&assignment, &request) {
            state.active_assignment = Some(assignment);
            return ready(Err(ClientError::Demo("stale assignment".to_string())));
        }
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
        let Some(mut assignment) = state.active_assignment.take() else {
            return ready(Err(ClientError::Demo("no active assignment".to_string())));
        };
        if !assignment_matches(&assignment, &request) {
            state.active_assignment = Some(assignment);
            return ready(Err(ClientError::Demo("stale assignment".to_string())));
        }
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
            .active_assignment
            .as_ref()
            .is_some_and(|active| assignment_matches(active, &assignment))
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
            assignment.image_id,
            UserId::from("admin"),
            DatasetRole::Annotator,
            now(),
            request.payload,
        );
        image_state.apply_event(&event).unwrap();
        ready(Ok(event))
    }
}

impl ReviewApi for SpyApi {
    fn record_review<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        review: ReviewRecord,
    ) -> ApiFuture<'a, EventLogEntry> {
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
        ready(Ok(event))
    }

    fn record_assigned_review<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        review: ReviewRecord,
    ) -> ApiFuture<'a, EventLogEntry> {
        if !self
            .state
            .borrow()
            .active_assignment
            .as_ref()
            .is_some_and(|active| assignment_matches(active, &assignment))
        {
            return ready(Err(ClientError::Demo("stale assignment".to_string())));
        }
        let complete = matches!(&review.target, labello_domain::ReviewTarget::Task { .. });
        let mut state = self.state.borrow_mut();
        state.counts.record_review += 1;
        let image_state = state
            .states
            .entry(assignment.image_id.clone())
            .or_insert_with(|| ImageState::new(assignment.image_id.clone()));
        let event = EventLogEntry::new(
            image_state.current_sequence + 1,
            assignment.image_id,
            UserId::from("admin"),
            DatasetRole::Reviewer,
            now(),
            EventPayload::ReviewRecorded { review },
        );
        image_state.apply_event(&event).unwrap();
        if complete {
            state.active_assignment = None;
        }
        ready(Ok(event))
    }

    fn record_correction<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: CorrectionRequest,
    ) -> ApiFuture<'a, EventLogEntry> {
        ready(Ok(EventLogEntry::new(
            1,
            image_id.clone(),
            UserId::from("admin"),
            DatasetRole::Reviewer,
            now(),
            EventPayload::AnnotationVersionCreated {
                annotation: request.annotation,
                previous_version: Some(request.previous_version),
                reason: request.reason,
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
            .active_assignment
            .as_ref()
            .is_some_and(|active| assignment_matches(active, &assignment))
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
            assignment.image_id,
            UserId::from("admin"),
            DatasetRole::Adjudicator,
            now(),
            EventPayload::AdjudicationRecorded { adjudication },
        );
        image_state.apply_event(&event).unwrap();
        state.active_assignment = None;
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
    fn github_login_url<'a>(&'a self, _request: OAuthLoginRequest) -> ApiFuture<'a, String> {
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
        per_task,
        per_class,
        throughput: Vec::new(),
    }
}

fn now() -> labello_domain::Timestamp {
    labello_domain::now()
}
