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

#[cfg(feature = "inspector-presets")]
#[test]
fn assignment_reload_discards_stale_manual_cursor_pass_and_local_draft() {
    use crate::app::LoadedImage;
    use crate::inspector_presets::{self, InspectorPreset};

    let mut app =
        inspector_presets::build(InspectorPreset::MigrationObject, &egui::Context::default());
    let loaded = LoadedImage {
        assignment: app.work.assignment.clone().unwrap(),
        queued: app.work.current.clone().unwrap(),
        annotations: app.work.annotations.clone(),
        state: app.work.current_state.clone().unwrap(),
        color_image: None,
    };
    app.work.migration.cursor = Some(labello_domain::MigrationCursor::FullImage);
    app.work.migration.active_pass_id = Some(labello_domain::MigrationPassId::from("stale-pass"));
    app.work.migration.draft =
        Some(crate::manual_migration::ManualMigrationState::empty_skeleton(["stale".to_string()]));
    app.work.migration.draft_group = Some(labello_domain::ObjectGroupId::from("stale-group"));
    app.work.migration.error = Some("stale failure".to_string());
    let operation_id = 77_001;
    let request = test_request(&app, operation_id, Some("demo"));
    app.work.active_load_id = Some(operation_id);
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

    assert!(app.work.migration.cursor.is_none());
    assert!(app.work.migration.active_pass_id.is_none());
    assert!(app.work.migration.draft.is_none());
    assert!(app.work.migration.draft_group.is_none());
    assert!(app.work.migration.error.is_none());
    app.sync_manual_migration();
    assert!(matches!(
        app.work.migration.cursor,
        Some(labello_domain::MigrationCursor::Object { ref object_group_id, .. })
            if object_group_id == &labello_domain::ObjectGroupId::from("group-left")
    ));
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
            assert!(!app.admin.snapshots_loaded);
        } else {
            assert!(app.admin.snapshots_loaded);
        }
    }
    assert_eq!(
        app.admin.snapshots_error.as_deref(),
        Some("refresh failure")
    );
}

#[test]
fn assignment_availability_poll_waits_for_the_in_flight_request() {
    let api = Rc::new(SpyApi::new());
    let mut app = base_live_app(api);
    app.view = AppView::Annotate;
    app.work.availability.dataset_id = Some(app.config.dataset_id.clone());
    app.work.availability.kind = Some(AssignmentKind::Annotation);
    app.work.availability.loading = true;
    app.work.availability.last_attempt = Some(Instant::now() - Duration::from_secs(11));
    let queued_before = app.runtime.commands.len();

    app.refresh_assignment_availability_if_due();

    assert_eq!(app.runtime.commands.len(), queued_before);
    assert!(!app.work.availability.refresh_after_load);
    assert!(app.work.availability.loading);
}

#[test]
fn assignment_availability_poll_is_scheduled_from_completion() {
    let api = Rc::new(SpyApi::new());
    let mut app = base_live_app(api);
    app.view = AppView::Annotate;
    app.request_assignment_availability();
    let UiCommand::AssignmentAvailability { request, .. } =
        app.runtime.commands.pop_back().unwrap()
    else {
        panic!("expected availability request");
    };
    app.work.availability.last_attempt = Some(Instant::now() - Duration::from_secs(30));
    app.runtime
        .tx
        .send(UiMessage::AssignmentAvailabilityLoaded {
            request,
            result: Ok(labello_client::AssignmentAvailability {
                kind: AssignmentKind::Annotation,
                tasks: BTreeMap::from([(TaskId::from("bounding_box:person"), true)]),
                related: Vec::new(),
            }),
        })
        .unwrap();

    app.process_messages(&egui::Context::default());
    let queued_before = app.runtime.commands.len();
    app.refresh_assignment_availability_if_due();

    assert!(!app.work.availability.loading);
    assert_eq!(app.runtime.commands.len(), queued_before);
    assert!(
        app.work.availability
            .last_attempt
            .is_some_and(|completed| completed.elapsed() < Duration::from_secs(1))
    );
}

#[test]
fn assignment_affecting_mutations_invalidate_the_persisted_availability() {
    let api = Rc::new(SpyApi::new());
    let mut app = base_live_app(api.clone());
    app.sync_work_config(api.metadata());
    app.view = AppView::Annotate;
    app.work.availability.dataset_id = Some(app.config.dataset_id.clone());
    app.work.availability.kind = Some(AssignmentKind::Annotation);
    app.work.availability.tasks = app
        .work.tasks
        .iter()
        .map(|task| (task.task_id.clone(), true))
        .collect();
    app.work.availability.resolved = true;
    app.work.availability.checked_at = Some(labello_domain::now());
    let request = test_request(&app, 42, Some("demo"));

    app.queue_command(UiCommand::Ingest {
        request,
        dataset_id: app.config.dataset_id.clone(),
    });

    assert!(app.work.availability.checked_at.is_none());
}

#[test]
fn stale_availability_is_discarded_after_refresh_and_dataset_switch() {
    let api = Rc::new(SpyApi::new());
    let mut app = base_live_app(api);
    app.view = AppView::Annotate;
    app.request_assignment_availability();
    let UiCommand::AssignmentAvailability { request, .. } =
        app.runtime.commands.pop_back().unwrap()
    else {
        panic!("expected availability request");
    };

    app.request_assignment_availability();
    app.runtime
        .tx
        .send(UiMessage::AssignmentAvailabilityLoaded {
            request,
            result: Ok(labello_client::AssignmentAvailability {
                kind: AssignmentKind::Annotation,
                tasks: BTreeMap::from([(TaskId::from("bounding_box:person"), false)]),
                related: Vec::new(),
            }),
        })
        .unwrap();
    app.process_messages(&egui::Context::default());
    assert_eq!(
        app.workflow_availability(&TaskId::from("bounding_box:person")),
        None,
        "a result superseded by a transition refresh must remain advisory"
    );
    let UiCommand::AssignmentAvailability { request, .. } =
        app.runtime.commands.pop_back().unwrap()
    else {
        panic!("expected replacement availability request");
    };

    app.begin_workspace_epoch();
    app.config.dataset_id = DatasetId::from("other");
    app.runtime
        .tx
        .send(UiMessage::AssignmentAvailabilityLoaded {
            request,
            result: Ok(labello_client::AssignmentAvailability {
                kind: AssignmentKind::Annotation,
                tasks: BTreeMap::from([(TaskId::from("bounding_box:person"), false)]),
                related: Vec::new(),
            }),
        })
        .unwrap();
    app.process_messages(&egui::Context::default());
    assert!(app.work.availability.tasks.is_empty());
    assert_eq!(app.work.availability.dataset_id, None);
}

#[test]
fn stale_save_responses_cannot_replace_the_current_image_state() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);
    let current_id = harness
        .state()
        .work.current
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
        harness.state().work.current.as_ref().unwrap().image.image_id,
        current_id
    );
    assert_eq!(
        harness.state().work.current_state.as_ref().unwrap().image_id,
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
        harness.state().work.shortcut_settings.recording,
        Some(labello_domain::UserAction::NextImage),
    );
    harness.key_press(egui::Key::Enter);
    harness.step();
    assert_eq!(harness.state().work.shortcut_settings.recording, None);
    assert_eq!(
        harness
            .state()
            .work.shortcut_settings
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
        harness.state().work.keybindings.bindings[&labello_domain::UserAction::NextImage].key,
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
    app.work.shortcut_settings
        .draft
        .as_mut()
        .unwrap()
        .bindings
        .get_mut(&labello_domain::UserAction::NextImage)
        .unwrap()
        .key = "Enter".to_string();
    let draft = app.work.shortcut_settings.draft.clone();

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

    assert_eq!(app.work.shortcut_settings.draft, draft);
    assert_eq!(
        app.work.shortcut_settings.error.as_deref(),
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
    assert!(harness.state().work.show_settings);
    assert_eq!(harness.state().work.shortcut_settings.recording, None);
    assert!(!harness.state().work.shortcut_settings.confirm_discard);
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

    assert!(!harness.state().work.show_settings);
    assert_eq!(
        harness.state().work.keybindings.bindings[&labello_domain::UserAction::NextImage].key,
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

    assert!(!harness.state().work.show_settings);
    assert!(harness.state().runtime.persistence.recovery.is_some());
}

#[test]
fn overlays_and_menus_block_background_shortcuts() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    let image_id = harness
        .state()
        .work.assignment
        .as_ref()
        .unwrap()
        .image_id
        .clone();
    harness.state_mut().work.drawer = Some(Drawer::Inspector);
    harness.step();

    harness.key_press(egui::Key::ArrowRight);
    harness.step();

    assert_eq!(api.counts().complete_assignment, 0);
    assert_eq!(
        harness.state().work.assignment.as_ref().unwrap().image_id,
        image_id
    );

    harness.state_mut().work.canvas.zoom_in();
    harness.state_mut().work.canvas.toggle_pan_mode();
    harness.key_press(egui::Key::Escape);
    harness.step();
    assert!(harness.state().work.canvas.pan_mode());
    harness.state_mut().work.drawer = None;
    harness.step();

    harness.set_size(egui::vec2(320.0, 568.0));
    harness.step();
    click(&mut harness, "More application actions");
    harness.key_press(egui::Key::ArrowRight);
    harness.step();
    assert_eq!(api.counts().complete_assignment, 0);
    assert_eq!(
        harness.state().work.assignment.as_ref().unwrap().image_id,
        image_id
    );
}

#[test]
fn pan_mode_shortcut_requires_zoom_and_escape_returns_to_annotation_mode() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);
    let zoom = harness.state().work.keybindings.bindings[&labello_domain::UserAction::ZoomIn].clone();
    harness
        .state_mut()
        .work.keybindings
        .bindings
        .insert(labello_domain::UserAction::RetryImageLoad, zoom);
    assert!(harness.state().work.keybindings.validate().is_ok());

    harness.key_press(egui::Key::P);
    harness.step();
    assert!(!harness.state().work.canvas.pan_mode());
    harness.key_press(egui::Key::Plus);
    harness.step();
    assert!(harness.state().work.canvas.current_zoom() > 1.0);
    harness.key_press(egui::Key::P);
    harness.step();
    assert!(harness.state().work.canvas.pan_mode());
    assert!(harness.query_by_label("Pan").is_some());

    harness.key_press(egui::Key::Escape);
    harness.step();
    assert!(!harness.state().work.canvas.pan_mode());
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
    assert!(harness.state().work.show_tutorial);
}

#[test]
fn stale_prefetch_response_cannot_enter_the_queue() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    step_until(&mut harness, 12, |app| app.work.queue.len() == 2);
    let loaded = harness.state_mut().work.queue.pop_prepared().unwrap();
    harness.state_mut().work.queue.clear();
    let operation_id = 90_001;
    let request = test_request(harness.state(), operation_id, Some("demo"));
    harness.state_mut().work.active_prefetch_id = Some(operation_id);
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
    assert!(harness.state().work.queue.is_empty());
    step_until(&mut harness, 8, |_| api.counts().release_assignment > 0);
}

#[test]
fn stale_blocking_claim_releases_its_assignment() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    step_until(&mut harness, 12, |app| app.work.queue.len() == 2);
    let assignment = harness.state_mut().work.queue.pop_prepared().unwrap().assignment;
    harness.state_mut().work.queue.clear();
    let operation_id = 90_002;
    let request = test_request(harness.state(), operation_id, Some("demo"));
    harness.state_mut().work.active_load_id = Some(operation_id);
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
    assert!(app.admin.load_error.is_some());

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
    assert!(app.admin.pending_role_saves.is_empty());

    saturate_command_queue(&mut app);
    app.request_images();
    assert!(!app.loading.images);
    assert!(app.admin.images_error.is_some());

    saturate_command_queue(&mut app);
    app.request_snapshots();
    assert!(!app.loading.snapshots);
    assert!(app.admin.snapshots_error.is_some());

    saturate_command_queue(&mut app);
    app.request_snapshot_create();
    assert!(!app.loading.creating_snapshot);
    assert!(app.admin.snapshot_action_error.is_some());

    saturate_command_queue(&mut app);
    app.request_snapshot_download("snapshot".to_string(), "manifest.json".to_string());
    assert!(app.loading.snapshot_file.is_none());
    assert!(app.admin.snapshot_action_error.is_some());

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
    assert!(app.work.shortcut_settings.error.is_some());

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
    assert!(harness.state().work.active_operation_id.is_none());
    assert!(harness.state().work.pending_transition.is_none());

    harness.state_mut().clear_current_image();
    saturate_command_queue(harness.state_mut());
    harness.state_mut().request_next_image();
    assert!(!harness.state().loading.image);
    assert!(harness.state().work.active_load_id.is_none());
    assert!(!harness.state().work.queue.is_loading());

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
    assert!(review.state().work.active_operation_id.is_none());

    let annotation_id = review.state().work.selected_annotation.clone().unwrap();
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
    assert!(review.state().work.active_operation_id.is_none());
    assert!(review.state().work.correction_draft.is_some());

    review.state_mut().view = AppView::Adjudicate;
    review.state_mut().work.assignment.as_mut().unwrap().kind = AssignmentKind::Adjudication;
    saturate_command_queue(review.state_mut());
    review
        .state_mut()
        .request_adjudication(labello_domain::AdjudicationDecision::AcceptAnnotation);
    assert!(!review.state().loading.saving);
    assert!(review.state().work.active_operation_id.is_none());
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
fn stale_assignment_operations_do_not_clear_the_active_loading_owner() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);
    let assignment = harness.state().work.assignment.clone().unwrap();
    let state = harness.state().work.current_state.clone().unwrap();
    harness.state_mut().work.active_operation_id = Some(77);
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
    assert_eq!(harness.state().work.active_operation_id, Some(77));

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
    assert_eq!(harness.state().work.active_operation_id, None);
}

#[test]
fn editing_a_persisted_box_saves_a_new_annotation_version() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    click(&mut harness, "Accept");
    click(&mut harness, "Save");
    step_until(&mut harness, 10, |app| app.work.save_status == SaveStatus::Saved);

    let annotation_id = harness.state().work.annotations[0].annotation_id.clone();
    let origin = harness.state().work.annotations[0].origin.clone();
    let object_group_id = harness.state().work.annotations[0].object_group_id.clone();
    harness.state_mut().edit_bbox(BoundingBoxEdit {
        annotation_id: annotation_id.clone(),
        bounding_box: BoundingBox {
            x: 0.2,
            y: 0.25,
            width: 0.3,
            height: 0.35,
        },
    });
    assert_eq!(harness.state().work.annotations[0].version, 2);
    assert_eq!(harness.state().work.annotations[0].origin, origin);
    assert_eq!(
        harness.state().work.annotations[0].object_group_id,
        object_group_id
    );
    assert!(matches!(
        harness.state().work.annotations[0].revision_source,
        RevisionSource::Human {
            action: HumanRevisionKind::Edited
        }
    ));
    assert_eq!(
        harness.state().work.annotations[0].author_user_id,
        UserId::from("admin")
    );
    assert!(matches!(
        origin,
        AnnotationOrigin::Native { legacy_v2: false }
    ));
    harness.state_mut().autosave();
    step_until(&mut harness, 10, |app| app.work.save_status == SaveStatus::Saved);

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
    assert!(harness.state().work.correction_draft.is_some());

    harness.key_press(egui::Key::Y);
    harness.step();
    harness.key_press(egui::Key::N);
    harness.step();
    assert_eq!(api.counts().record_review, 0);
    assert!(harness.state().work.correction_draft.is_some());

    saturate_command_queue(harness.state_mut());
    harness
        .state_mut()
        .request_review(labello_domain::ReviewDecision::Rejected);
    assert!(harness.state().work.correction_draft.is_some());
    assert!(!harness.state().loading.saving);

    harness.state_mut().runtime.commands.clear();
    harness.state_mut().runtime.active_requests.clear();
    harness
        .state_mut()
        .request_review(labello_domain::ReviewDecision::Rejected);
    assert!(harness.state().work.correction_draft.is_none());
    assert!(harness.state().loading.saving);
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
