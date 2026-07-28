#[cfg(feature = "inspector-presets")]
#[test]
fn mutable_migration_spy_preserves_failure_and_durable_reload_progression() {
    use crate::inspector_presets::{self, InspectorPreset};

    let api = Rc::new(SpyApi::new());
    let mut app =
        inspector_presets::build(InspectorPreset::MigrationObject, &egui::Context::default());
    let image_id = app.work.current.as_ref().unwrap().image.image_id.clone();
    api.set_image_state(app.work.current_state.clone().unwrap());
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
    step_until(&mut harness, 8, |app| !app.work.migration.busy);
    assert!(
        harness
            .state()
            .work.migration
            .error
            .as_deref()
            .is_some_and(|error| error.contains("migration command failed")),
        "counts={:?} migration_error={:?} runtime_error={:?}",
        api.counts(),
        harness.state().work.migration.error,
        harness.state().runtime.error,
    );
    assert!(matches!(
        harness.state().work.migration.cursor,
        Some(labello_domain::MigrationCursor::Object { ref object_group_id, .. })
            if object_group_id == &labello_domain::ObjectGroupId::from("group-left")
    ));

    harness
        .state_mut()
        .request_exclude_migration_target(labello_domain::ObjectGroupId::from("group-left"));
    harness.step();
    step_until(&mut harness, 8, |app| {
        matches!(
            app.work.migration.cursor,
            Some(labello_domain::MigrationCursor::Object { ref object_group_id, .. })
                if object_group_id == &labello_domain::ObjectGroupId::from("group-right")
        )
    });
    assert_eq!(api.counts().migration_commands, 2);

    let durable = api.image_state(&image_id);
    let mut reloaded =
        inspector_presets::build(InspectorPreset::MigrationObject, &egui::Context::default());
    reloaded.work.current_state = Some(durable.clone());
    reloaded.work.annotations = durable.active_annotations().cloned().collect();
    reloaded.work.migration = Default::default();
    let mut reload_harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 1000.0))
        .build_eframe(|_| reloaded);
    reload_harness.step();
    assert!(matches!(
        reload_harness.state().work.migration.cursor,
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
        let draft = app.work.migration.draft.as_mut().unwrap();
        draft.keypoints[0].point = Some(labello_domain::NormalizedPoint { x: 0.5, y: 0.5 });
        draft.keypoints[0].state = labello_domain::KeypointState::Visible;
        app.work.migration.keypoint_index = 1;
    };

    let task_id = harness.state().work.selected_task_id.clone().unwrap();
    let guide_id = harness
        .state()
        .work.current_state
        .as_ref()
        .unwrap()
        .migration_target_sets[&task_id]
        .targets[0]
        .guide_annotation_id
        .clone();
    let guide_before = harness
        .state()
        .work.current_state
        .as_ref()
        .unwrap()
        .current_annotation(&guide_id)
        .unwrap()
        .clone();

    place_first_keypoint(harness.state_mut());
    harness.state_mut().work.migration.next_hidden = true;
    harness.step();
    click_accesskit_button(&mut harness, "Undo last keypoint");
    assert_eq!(harness.state().work.migration.keypoint_index, 0);
    assert!(
        harness.state().work.migration.draft.as_ref().unwrap().keypoints[0]
            .point
            .is_none()
    );
    assert!(!harness.state().work.migration.next_hidden);
    assert_eq!(
        harness
            .state()
            .work.current_state
            .as_ref()
            .unwrap()
            .current_annotation(&guide_id),
        Some(&guide_before)
    );

    place_first_keypoint(harness.state_mut());
    harness.step();
    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::Z);
    harness.step();
    assert_eq!(harness.state().work.migration.keypoint_index, 0);
    assert!(
        harness.state().work.migration.draft.as_ref().unwrap().keypoints[0]
            .point
            .is_none()
    );

    place_first_keypoint(harness.state_mut());
    harness.step();
    harness.key_press(egui::Key::Delete);
    harness.step();
    assert_eq!(harness.state().work.migration.keypoint_index, 0);
    assert!(
        harness.state().work.migration.draft.as_ref().unwrap().keypoints[0]
            .point
            .is_none()
    );

    place_first_keypoint(harness.state_mut());
    harness
        .state_mut()
        .work.current_state
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
    assert_eq!(harness.state().work.migration.keypoint_index, 1);
    assert!(
        harness.state().work.migration.draft.as_ref().unwrap().keypoints[0]
            .point
            .is_some()
    );
}

#[cfg(feature = "inspector-presets")]
#[test]
fn migration_confirmation_promotes_prepared_assignment_without_blocking_reload() {
    use crate::app::LoadedImage;
    use crate::inspector_presets::{self, InspectorPreset};
    use crate::queue::QueuedImage;

    let api = Rc::new(SpyApi::new());
    let mut app = inspector_presets::build(
        InspectorPreset::MigrationFullImage,
        &egui::Context::default(),
    );
    api.set_image_state(app.work.current_state.clone().unwrap());
    api.complete_next_migration_with(app.work.assignment.clone().unwrap());
    let next_image_id = ImageId::from("img_prepared_migration");
    let next_assignment = Assignment {
        assignment_id: AssignmentId::generate(),
        image_id: next_image_id.clone(),
        task_id: app.work.selected_task_id.clone().unwrap(),
        assigned_to: app.config.user_id.clone(),
        kind: AssignmentKind::Annotation,
        status: AssignmentStatus::Active,
        expires_at: Some(now() + chrono::Duration::minutes(5)),
        created_at: now(),
        updated_at: now(),
    };
    app.work.queue.clear();
    assert!(app.work.queue.push_prepared(LoadedImage {
        assignment: next_assignment,
        queued: QueuedImage {
            image: image_record(next_image_id.as_str(), "prepared-migration.png", 640, 480),
            prelabels: Vec::new(),
        },
        annotations: Vec::new(),
        state: ImageState::new(next_image_id.clone()),
        color_image: None,
    }));
    api.set_no_assignment(true);
    app.runtime.api = Some(api.clone());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_max_steps(40)
        .build_eframe(|_| app);
    harness.step();
    let previews_before = api.counts().get_image_preview;

    harness.key_press(egui::Key::ArrowRight);
    harness.step();
    step_until(&mut harness, 8, |app| {
        !app.work.migration.busy
            && app
                .work.assignment
                .as_ref()
                .is_some_and(|assignment| assignment.image_id == next_image_id)
    });
    assert_eq!(api.counts().migration_commands, 1);
    assert_eq!(api.counts().get_image_preview, previews_before);
    assert_eq!(api.counts().release_assignment, 0);
    assert!(!harness.state().loading.image);
    harness.step();
    assert_eq!(api.counts().migration_commands, 1);
}

#[cfg(feature = "inspector-presets")]
#[test]
fn migration_primary_actions_stay_visible_without_the_inspector_drawer() {
    use crate::inspector_presets::{self, InspectorPreset};

    let mut object = Harness::builder()
        .with_size(egui::vec2(390.0, 667.0))
        .build_eframe(|ctx| {
            inspector_presets::build(InspectorPreset::MigrationObject, &ctx.egui_ctx)
    });
    object.step();
    assert!(object.query_by_label_contains("Save & next").is_some());
    assert!(object.query_by_label("Workflow").is_some());
    assert!(object.query_by_label("Inspector").is_some());
    assert!(object.query_by_label("Controls").is_none());
    assert!(object.state().work.drawer.is_none());
    let primary = object.get_by_label_contains("Save & next").rect();
    let workflow = object.get_by_label("Workflow").rect();
    let inspector = object.get_by_label("Inspector").rect();
    assert!(workflow.width() <= 44.5, "{workflow:?}");
    assert!(inspector.width() <= 44.5, "{inspector:?}");
    assert!((primary.top() - workflow.top()).abs() <= 1.0);
    assert!((primary.top() - inspector.top()).abs() <= 1.0);

    click(&mut object, "More application actions");
    assert!(object.query_by_label("Workflow panel").is_none());
    assert!(object.query_by_label("Inspector panel").is_none());
    object.key_press(egui::Key::Escape);
    object.step();

    click_accesskit_button(&mut object, "Workflow");
    assert_eq!(object.state().work.drawer, Some(Drawer::Workflow));
    object.key_press(egui::Key::W);
    object.step();
    assert!(object.state().work.drawer.is_none());

    let mut roomy_compact = Harness::builder()
        .with_size(egui::vec2(590.0, 667.0))
        .build_eframe(|ctx| {
            inspector_presets::build(InspectorPreset::MigrationObject, &ctx.egui_ctx)
        });
    roomy_compact.step();
    let primary = roomy_compact.get_by_label_contains("Save & next").rect();
    let workflow = roomy_compact.get_by_label("Workflow").rect();
    let inspector = roomy_compact.get_by_label("Inspector").rect();
    assert!(workflow.width() > 80.0, "{workflow:?}");
    assert!(inspector.width() > 80.0, "{inspector:?}");
    assert!((primary.top() - workflow.top()).abs() <= 1.0);
    assert!((primary.top() - inspector.top()).abs() <= 1.0);

    let mut medium = Harness::builder()
        .with_size(egui::vec2(900.0, 667.0))
        .build_eframe(|ctx| {
            inspector_presets::build(InspectorPreset::MigrationObject, &ctx.egui_ctx)
        });
    medium.step();
    assert!(medium.query_by_label("Workflow").is_some());
    assert!(medium.query_by_label("Inspector").is_some());
    assert!(medium.query_by_label("Migration controls").is_none());
    assert!(medium.get_by_label("Workflow").rect().width() > 80.0);
    assert!(medium.get_by_label("Inspector").rect().width() > 80.0);

    click_accesskit_button(&mut object, "Inspector");
    assert_eq!(object.state().work.drawer, Some(Drawer::Inspector));
    object.step();
    assert_eq!(
        object
            .query_all_by_label_contains("Save & next")
            .filter(|node| node.accesskit_node().role() == egui::accesskit::Role::Button)
            .count(),
        1,
        "opening the drawer must not duplicate the primary migration action"
    );
    assert!(
        object
            .query_by_label("Can't annotate this object")
            .is_some()
    );
    assert!(object.query_by_label("Reason").is_none());
    object.key_press(egui::Key::I);
    object.step();
    assert!(object.state().work.drawer.is_none());

    object.set_size(egui::vec2(260.0, 667.0));
    object.step();
    let workflow = object.get_by_label("Workflow").rect();
    let inspector = object.get_by_label("Inspector").rect();
    assert!(workflow.width() <= 44.5, "{workflow:?}");
    assert!(inspector.width() <= 44.5, "{inspector:?}");
    assert!((workflow.top() - inspector.top()).abs() <= 1.0);
    assert!(
        workflow.top() > object.get_by_label_contains("Save & next").rect().top(),
        "the controls should wrap only after becoming icon-only"
    );

    let mut full_image = Harness::builder()
        .with_size(egui::vec2(390.0, 667.0))
        .build_eframe(|ctx| {
            inspector_presets::build(InspectorPreset::MigrationFullImage, &ctx.egui_ctx)
        });
    full_image.step();
    assert!(
        full_image
            .query_by_label_contains("Confirm & finish")
            .is_some()
    );
    assert!(full_image.query_by_label("Workflow").is_some());
    assert!(full_image.query_by_label("Inspector").is_some());
    assert!(full_image.query_by_label("Controls").is_none());
    assert!(full_image.state().work.drawer.is_none());
}

#[cfg(feature = "inspector-presets")]
#[test]
fn migration_save_uses_the_contextual_submit_shortcut() {
    use crate::inspector_presets::{self, InspectorPreset};
    use crate::manual_migration::ManualMigrationState;

    let api = Rc::new(SpyApi::new());
    let mut app =
        inspector_presets::build(InspectorPreset::MigrationObject, &egui::Context::default());
    let task = app.selected_task().unwrap().clone();
    let group_id = match app.work.migration.cursor.as_ref().unwrap() {
        labello_domain::MigrationCursor::Object {
            object_group_id, ..
        } => object_group_id.clone(),
        labello_domain::MigrationCursor::FullImage => panic!("expected object cursor"),
    };
    let mut draft = ManualMigrationState::empty_skeleton(
        task.skeleton
            .unwrap()
            .keypoints
            .into_iter()
            .map(|point| point.name),
    );
    for keypoint in &mut draft.keypoints {
        keypoint.point = Some(labello_domain::NormalizedPoint { x: 0.5, y: 0.5 });
        keypoint.state = labello_domain::KeypointState::Visible;
    }
    app.work.migration.keypoint_index = draft.keypoints.len();
    app.work.migration.draft = Some(draft);
    app.work.migration.draft_group = Some(group_id);
    app.work.migration.draft_dirty = true;
    api.set_image_state(app.work.current_state.clone().unwrap());
    app.runtime.api = Some(api.clone());

    let mut harness = Harness::builder()
        .with_size(egui::vec2(390.0, 667.0))
        .with_max_steps(40)
        .build_eframe(|_| app);
    harness.step();
    assert!(
        !harness
            .get_by_label_contains("Save & next")
            .accesskit_node()
            .is_disabled()
    );

    harness.key_press(egui::Key::ArrowRight);
    harness.step();
    step_until(&mut harness, 8, |app| !app.work.migration.busy);
    assert_eq!(api.counts().migration_commands, 1);
}

#[cfg(feature = "inspector-presets")]
#[test]
fn migration_review_decisions_are_visible_and_keep_their_shortcuts_on_mobile() {
    use crate::inspector_presets::{self, InspectorPreset};

    let api = Rc::new(SpyApi::new());
    let mut app =
        inspector_presets::build(InspectorPreset::MigrationReview, &egui::Context::default());
    api.set_image_state(app.work.current_state.clone().unwrap());
    app.runtime.api = Some(api.clone());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(390.0, 667.0))
        .with_max_steps(40)
        .build_eframe(|_| app);
    harness.step();

    assert!(
        harness
            .query_by_label_contains("Approve migration item")
            .is_some()
    );
    assert!(
        harness
            .query_by_label_contains("Reject migration item")
            .is_some()
    );
    assert!(harness.query_by_label("Workflow").is_some());
    assert!(harness.query_by_label("Inspector").is_some());
    assert!(harness.query_by_label("Controls").is_none());

    harness.key_press(egui::Key::Y);
    harness.step();
    step_until(&mut harness, 8, |app| !app.work.migration.busy);
    assert_eq!(api.counts().migration_commands, 1);
}
