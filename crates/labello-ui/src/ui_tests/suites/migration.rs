#[cfg(feature = "inspector-presets")]
#[test]
fn active_migration_discards_stale_availability_without_rechecking() {
    use crate::inspector_presets::{self, InspectorPreset};

    let api = Rc::new(SpyApi::new());
    let mut app =
        inspector_presets::build(InspectorPreset::MigrationObject, &egui::Context::default());
    api.set_image_state(app.work.current_state.clone().unwrap());
    app.runtime.api = Some(api);

    let unavailable_task = TaskId::from("bounding_box:person_cleanup");
    app.work.availability.dataset_id = Some(app.config.dataset_id.clone());
    app.work.availability.kind = Some(AssignmentKind::Annotation);
    app.work.availability.tasks = app
        .work
        .tasks
        .iter()
        .map(|task| (task.task_id.clone(), task.task_id != unavailable_task))
        .collect();
    app.work.availability.resolved = true;
    assert_eq!(
        app.workflow_availability(&unavailable_task),
        Some(false)
    );

    app.request_assignment_availability();
    let availability_request = take_assignment_availability_request(&mut app);
    app.request_exclude_migration_target(labello_domain::ObjectGroupId::from("group-left"));
    assert!(app.work.migration.busy);
    assert!(app.work.availability.refresh_after_load);
    assert_eq!(app.workflow_availability(&unavailable_task), None);
    assert_eq!(
        app.displayed_workflow_availability(&unavailable_task),
        Some(false)
    );

    deliver_assignment_availability(&mut app, availability_request, true);

    assert!(!app.work.availability.loading);
    assert!(!app.work.availability.resolved);
    assert_eq!(
        app.displayed_workflow_availability(&unavailable_task),
        Some(false),
        "discarding the stale response must retain the last known picker state"
    );
    assert!(
        app.runtime
            .commands
            .iter()
            .all(|command| !matches!(command, UiCommand::AssignmentAvailability { .. }))
    );

    app.work.availability.last_attempt = Some(Instant::now() - Duration::from_secs(31));
    app.refresh_assignment_availability_if_due();
    assert!(
        app.runtime
            .commands
            .iter()
            .all(|command| !matches!(command, UiCommand::AssignmentAvailability { .. }))
    );

    app.work.migration.busy = false;
    assert!(app.manual_migration_active());
    app.refresh_assignment_availability_if_due();
    assert!(
        app.runtime
            .commands
            .iter()
            .all(|command| !matches!(command, UiCommand::AssignmentAvailability { .. }))
    );

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(|_| app);
    harness.step();
    let unavailable_workflow = harness.get_by_role_and_label(
        egui::accesskit::Role::Button,
        "Imported person bounding-box cleanup",
    );
    assert!(
        unavailable_workflow.accesskit_node().is_disabled(),
        "the picker must keep the last known unavailable workflow disabled during migration"
    );
}

#[cfg(feature = "inspector-presets")]
#[test]
fn workflow_controls_keep_their_identity_when_a_loaded_image_enables_migration_actions() {
    use crate::inspector_presets::{self, InspectorPreset};

    let mut app =
        inspector_presets::build(InspectorPreset::MigrationObject, &egui::Context::default());
    let loaded_state = app.work.current_state.take().unwrap();
    assert!(!app.manual_migration_active());

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(|_| app);
    harness.step();
    let label = harness.state().selected_workflow().unwrap().label();
    let loading_id = harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &label)
        .accesskit_node()
        .locate()
        .0;

    harness.state_mut().work.current_state = Some(loaded_state);
    harness.step();
    assert!(harness.state().manual_migration_active());
    let loaded_id = harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &label)
        .accesskit_node()
        .locate()
        .0;

    assert_eq!(
        loaded_id, loading_id,
        "loading a migration image must not replace the workflow controls"
    );
}

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
fn migration_previous_object_navigation_immediately_revisits_for_editing() {
    use crate::inspector_presets::{self, InspectorPreset};

    let api = Rc::new(SpyApi::new());
    let mut app = inspector_presets::build(
        InspectorPreset::MigrationFullImage,
        &egui::Context::default(),
    );
    let image_id = app.work.current.as_ref().unwrap().image.image_id.clone();
    api.set_image_state(app.work.current_state.clone().unwrap());
    app.runtime.api = Some(api.clone());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_max_steps(40)
        .build_eframe(|_| app);
    harness.step();

    click_accesskit_button(&mut harness, "Previous object");
    harness.step();
    step_until(&mut harness, 8, |app| !app.work.migration.busy);
    assert_eq!(api.counts().migration_commands, 1);
    assert!(harness.state().work.migration.inspected_group_id.is_none());
    assert!(matches!(
        harness.state().work.migration.cursor,
        Some(labello_domain::MigrationCursor::Object { ref object_group_id, .. })
            if object_group_id == &labello_domain::ObjectGroupId::from("group-right")
    ));
    assert!(api.image_state(&image_id).migration_dependencies
        [&labello_domain::TaskId::from("skeleton:person")]
        .contains_key(&labello_domain::ObjectGroupId::from("group-right")));

    harness.key_press(egui::Key::ArrowUp);
    harness.step();
    step_until(&mut harness, 8, |app| !app.work.migration.busy);
    assert_eq!(api.counts().migration_commands, 2);
    assert!(matches!(
        harness.state().work.migration.cursor,
        Some(labello_domain::MigrationCursor::Object { ref object_group_id, .. })
            if object_group_id == &labello_domain::ObjectGroupId::from("group-left")
    ));
}

#[cfg(feature = "inspector-presets")]
#[test]
fn migration_previous_object_edit_confirms_before_discarding_unsaved_input() {
    use crate::inspector_presets::{self, InspectorPreset};

    let api = Rc::new(SpyApi::new());
    let mut app = inspector_presets::build(
        InspectorPreset::MigrationFullImage,
        &egui::Context::default(),
    );
    api.set_image_state(app.work.current_state.clone().unwrap());
    app.runtime.api = Some(api.clone());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_max_steps(40)
        .build_eframe(|_| app);
    harness.step();
    harness.state_mut().work.migration.draft_dirty = true;

    click_accesskit_button(&mut harness, "Previous object");
    harness.step();
    assert!(
        harness
            .query_by_label("Discard current migration draft?")
            .is_some()
    );
    assert_eq!(api.counts().migration_commands, 0);

    click_accesskit_button(&mut harness, "Discard draft and edit object");
    harness.step();
    step_until(&mut harness, 8, |app| !app.work.migration.busy);
    assert_eq!(api.counts().migration_commands, 1);
    assert!(matches!(
        harness.state().work.migration.cursor,
        Some(labello_domain::MigrationCursor::Object { ref object_group_id, .. })
            if object_group_id == &labello_domain::ObjectGroupId::from("group-right")
    ));
}

#[cfg(feature = "inspector-presets")]
#[test]
fn migration_full_image_can_add_an_object_missing_from_the_import() {
    use crate::inspector_presets::{self, InspectorPreset};

    let api = Rc::new(SpyApi::new());
    let mut app = inspector_presets::build(
        InspectorPreset::MigrationFullImage,
        &egui::Context::default(),
    );
    app.work.inspector_panel_collapsed = true;
    api.set_image_state(app.work.current_state.clone().unwrap());
    app.runtime.api = Some(api.clone());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_max_steps(40)
        .build_eframe(|_| app);
    harness.step();

    let add_action = harness.get_by_label("Add missing object M").rect();
    harness.key_press(egui::Key::M);
    harness.step();
    assert!(harness.state().work.migration.adding_missing_object);
    let cancel_action = harness.get_by_label("Cancel adding object M").rect();
    assert!(
        (add_action.left() - cancel_action.left()).abs() <= 1.0
            && (add_action.top() - cancel_action.top()).abs() <= 1.0,
        "add={add_action:?} cancel={cancel_action:?}"
    );
    click_accesskit_button(&mut harness, "Cancel adding object");
    harness.step();
    assert!(!harness.state().work.migration.adding_missing_object);
    assert!(harness.query_by_label("Add missing object M").is_some());

    harness.key_press(egui::Key::M);
    harness.step();
    assert!(harness.state().work.migration.adding_missing_object);
    harness.key_press(egui::Key::M);
    harness.step();
    assert!(!harness.state().work.migration.adding_missing_object);
    assert!(harness.query_by_label("Add missing object M").is_some());
    harness.key_press(egui::Key::M);
    harness.step();
    assert!(harness.state().work.migration.adding_missing_object);
    assert!(
        harness
            .query_by_label_contains("Save missing object")
            .unwrap()
            .accesskit_node()
            .is_disabled()
    );

    let canvas = harness.get_by_label("Annotation canvas").rect();
    click_at(&mut harness, canvas.center());
    let moved_first = canvas.center() + egui::vec2(36.0, -18.0);
    drag_at(&mut harness, canvas.center(), moved_first);
    assert_eq!(harness.state().work.migration.keypoint_index, 1);
    assert!(
        harness.state().work.migration.draft.as_ref().unwrap().keypoints[0]
            .point
            .is_some_and(|point| point.x > 0.5 && point.y < 0.5)
    );

    let keypoint_count = {
        let draft = harness.state_mut().work.migration.draft.as_mut().unwrap();
        for keypoint in draft.keypoints.iter_mut().skip(1) {
            keypoint.point = Some(labello_domain::NormalizedPoint { x: 0.5, y: 0.5 });
            keypoint.state = labello_domain::KeypointState::Visible;
        }
        draft.keypoints.len()
    };
    harness.state_mut().work.migration.keypoint_index = keypoint_count;
    harness.state_mut().work.migration.draft_dirty = true;
    harness.step();
    assert!(
        !harness
            .query_by_label_contains("Save missing object")
            .unwrap()
            .accesskit_node()
            .is_disabled()
    );

    click_accesskit_button(&mut harness, "Save missing object");
    harness.step();
    step_until(&mut harness, 8, |app| !app.work.migration.busy);
    assert_eq!(api.counts().migration_commands, 1);
    assert!(!harness.state().work.migration.adding_missing_object);
    assert!(matches!(
        harness.state().work.migration.cursor,
        Some(labello_domain::MigrationCursor::FullImage)
    ));
    let task_id = harness.state().work.selected_task_id.as_ref().unwrap();
    assert_eq!(
        harness
            .state()
            .work
            .current_state
            .as_ref()
            .unwrap()
            .active_annotations()
            .filter(|annotation| {
                annotation.task_id == *task_id
                    && annotation.object_group_id.is_none()
                    && annotation.annotation_type == labello_domain::AnnotationType::Skeleton
            })
            .count(),
        1
    );
    harness.set_size(egui::vec2(390.0, 667.0));
    harness.step();
    assert!(harness.query_by_label("Edit added").is_some());
    click_accesskit_button(&mut harness, "Edit added");
    harness.step();
    assert!(harness.state().work.migration.adding_missing_object);
    assert_eq!(
        harness
            .state()
            .work
            .migration
            .editing_missing_annotation_id
            .as_ref(),
        Some(&labello_domain::AnnotationId::from("spy-discovered"))
    );
    harness.set_size(egui::vec2(1440.0, 900.0));
    harness.step();
    assert!(
        harness
            .query_by_label_contains("Save object changes")
            .unwrap()
            .accesskit_node()
            .is_disabled()
    );
    let edited_point = labello_domain::NormalizedPoint { x: 0.7, y: 0.6 };
    harness.state_mut().work.migration.draft.as_mut().unwrap().keypoints[0].point =
        Some(edited_point);
    harness.state_mut().work.migration.draft_dirty = true;
    harness.step();
    click_accesskit_button(&mut harness, "Save object changes");
    harness.step();
    step_until(&mut harness, 8, |app| !app.work.migration.busy);
    assert_eq!(api.counts().migration_commands, 2);
    assert!(!harness.state().work.migration.adding_missing_object);
    let edited = harness
        .state()
        .work
        .current_state
        .as_ref()
        .unwrap()
        .current_annotation(&labello_domain::AnnotationId::from("spy-discovered"))
        .unwrap();
    assert_eq!(edited.version, 2);
    assert!(
        matches!(
            &edited.geometry,
            labello_domain::AnnotationGeometry::Skeleton(skeleton)
                if skeleton.keypoints[0].point == Some(edited_point)
        ),
        "{:?}",
        edited.geometry
    );
    assert!(
        harness
            .query_by_label_contains("Add missing object")
            .is_some()
    );
    click_accesskit_button(&mut harness, "Edit added object 1");
    harness.step();
    assert!(harness.query_by_label("Remove added object").is_some());
    click_accesskit_button(&mut harness, "Remove added object");
    harness.step();
    step_until(&mut harness, 8, |app| !app.work.migration.busy);
    assert_eq!(api.counts().migration_commands, 3);
    assert!(!harness.state().work.migration.adding_missing_object);
    assert!(
        harness
            .state()
            .work
            .current_state
            .as_ref()
            .unwrap()
            .current_annotation(&labello_domain::AnnotationId::from("spy-discovered"))
            .unwrap()
            .deleted
    );
    assert!(harness.query_by_label("Edit added object 1").is_none());
    assert!(
        harness
            .query_by_label_contains("Add missing object")
            .is_some()
    );
}

#[cfg(feature = "inspector-presets")]
#[test]
fn missing_object_uses_its_own_zero_position_explanation() {
    use crate::inspector_presets::{self, InspectorPreset};

    let mut app = inspector_presets::build(
        InspectorPreset::MigrationFullImage,
        &egui::Context::default(),
    );
    let task_id = app.work.selected_task_id.clone().unwrap();
    app.work
        .tasks
        .iter_mut()
        .find(|task| task.task_id == task_id)
        .unwrap()
        .skeleton = Some(labello_domain::SkeletonSpec {
        keypoints: vec![labello_domain::KeypointSpec {
            name: "center".to_string(),
            required: false,
        }],
        edges: Vec::new(),
        allow_hidden: true,
        allow_absent: true,
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(|_| app);
    harness.step();

    click_accesskit_button(&mut harness, "Add missing object");
    let not_present = harness
        .query_all_by_label_contains("Mark center as not present")
        .find(|node| node.accesskit_node().role() == egui::accesskit::Role::Button)
        .unwrap();
    assert!(not_present.accesskit_node().is_disabled());
    assert!(
        harness
            .query_by_label(
                "At least one keypoint position is required to add an object."
            )
            .is_some()
    );
    harness.key_press(egui::Key::N);
    harness.step();
    assert_eq!(harness.state().work.migration.keypoint_index, 0);
    assert!(
        harness
            .query_by_label_contains("Save missing object")
            .unwrap()
            .accesskit_node()
            .is_disabled()
    );
}

#[cfg(feature = "inspector-presets")]
#[test]
fn clicking_a_pending_box_confirms_the_current_skeleton_and_selects_the_clicked_target() {
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
        keypoint.point = Some(labello_domain::NormalizedPoint { x: 0.25, y: 0.3 });
        keypoint.state = labello_domain::KeypointState::Visible;
    }
    app.work.migration.keypoint_index = draft.keypoints.len();
    app.work.migration.draft = Some(draft);
    app.work.migration.draft_group = Some(group_id);
    app.work.migration.draft_dirty = true;
    api.set_image_state(app.work.current_state.clone().unwrap());
    app.runtime.api = Some(api.clone());

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_max_steps(40)
        .build_eframe(|_| app);
    harness.step();
    assert!(harness.state().work.canvas.zoom() > 1.0);
    harness.state_mut().work.canvas.fit_view();
    harness.step();
    let availability_checks = api.counts().assignment_availability;

    let canvas = harness.get_by_label("Annotation canvas").rect();
    let image_aspect = 1280.0 / 800.0;
    let image_size = if canvas.width() / canvas.height() > image_aspect {
        egui::vec2(canvas.height() * image_aspect, canvas.height())
    } else {
        egui::vec2(canvas.width(), canvas.width() / image_aspect)
    };
    let image = egui::Rect::from_center_size(canvas.center(), image_size);
    let pending_box_center = egui::pos2(
        image.left() + image.width() * 0.73,
        image.top() + image.height() * 0.49,
    );
    harness.event(egui::Event::PointerMoved(pending_box_center));
    harness.step();
    harness.event(egui::Event::PointerButton {
        pos: pending_box_center,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    harness.event(egui::Event::PointerButton {
        pos: pending_box_center,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    let clicked_group_id = labello_domain::ObjectGroupId::from("group-right");
    step_until(&mut harness, 8, |app| {
        app.work.migration.inspected_group_id.as_ref() == Some(&clicked_group_id)
    });
    assert_eq!(
        harness.state().work.migration.inspected_group_id,
        Some(clicked_group_id.clone())
    );
    assert!(harness.state().work.migration.busy);
    step_until(&mut harness, 8, |app| !app.work.migration.busy);

    assert_eq!(api.counts().migration_commands, 2);
    assert!(matches!(
        harness.state().work.migration.cursor,
        Some(labello_domain::MigrationCursor::Object { ref object_group_id, .. })
            if object_group_id == &clicked_group_id
    ));
    assert_eq!(
        harness.state().work.migration.draft_group,
        Some(clicked_group_id)
    );
    assert!(
        harness
            .state()
            .work
            .migration
            .draft
            .as_ref()
            .is_some_and(|draft| draft.keypoints.iter().all(|keypoint| keypoint.point.is_none()))
    );
    assert!(!harness.state().work.migration.draft_dirty);
    assert_eq!(
        api.counts().assignment_availability,
        availability_checks
    );
}

#[cfg(feature = "inspector-presets")]
#[test]
fn clicking_an_unmigrated_box_skips_an_empty_current_skeleton() {
    use crate::inspector_presets::{self, InspectorPreset};

    let api = Rc::new(SpyApi::new());
    let mut app =
        inspector_presets::build(InspectorPreset::MigrationObject, &egui::Context::default());
    let task_id = app.selected_task().unwrap().task_id.clone();
    api.set_image_state(app.work.current_state.clone().unwrap());
    app.runtime.api = Some(api.clone());

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_max_steps(40)
        .build_eframe(|_| app);
    harness.step();
    assert_eq!(harness.state().work.migration.keypoint_index, 0);
    harness.state_mut().work.canvas.fit_view();
    harness.step();

    let canvas = harness.get_by_label("Annotation canvas").rect();
    let image_aspect = 1280.0 / 800.0;
    let image_size = if canvas.width() / canvas.height() > image_aspect {
        egui::vec2(canvas.height() * image_aspect, canvas.height())
    } else {
        egui::vec2(canvas.width(), canvas.width() / image_aspect)
    };
    let image = egui::Rect::from_center_size(canvas.center(), image_size);
    let pending_box_center = egui::pos2(
        image.left() + image.width() * 0.73,
        image.top() + image.height() * 0.49,
    );
    click_at(&mut harness, pending_box_center);
    step_until(&mut harness, 8, |app| {
        matches!(
            app.work.migration.cursor,
            Some(labello_domain::MigrationCursor::Object {
                ref object_group_id,
                ..
            }) if object_group_id == &labello_domain::ObjectGroupId::from("group-right")
        )
    });

    assert_eq!(api.counts().migration_commands, 1);
    assert!(matches!(
        &harness.state().work.current_state.as_ref().unwrap().migration_dispositions[&task_id]
            [&labello_domain::ObjectGroupId::from("group-left")]
            .status,
        labello_domain::MigrationDispositionStatus::Excluded { exclusion }
            if exclusion.reason == labello_domain::MigrationExclusionReason::NoValidSkeleton
    ));
}

#[cfg(feature = "inspector-presets")]
#[test]
fn clicking_a_skipped_box_immediately_revisits_it_for_editing() {
    use crate::inspector_presets::{self, InspectorPreset};

    let api = Rc::new(SpyApi::new());
    let mut app = inspector_presets::build(
        InspectorPreset::MigrationFullImage,
        &egui::Context::default(),
    );
    let task_id = app.selected_task().unwrap().task_id.clone();
    let skipped_group_id = labello_domain::ObjectGroupId::from("group-right");
    let image_id = app.work.current.as_ref().unwrap().image.image_id.clone();
    let labello_domain::MigrationDispositionStatus::Excluded { exclusion } = &mut app
        .work
        .current_state
        .as_mut()
        .unwrap()
        .migration_dispositions
        .get_mut(&task_id)
        .unwrap()
        .get_mut(&skipped_group_id)
        .unwrap()
        .status
    else {
        panic!("expected excluded migration target");
    };
    exclusion.reason = labello_domain::MigrationExclusionReason::NoValidSkeleton;
    api.set_image_state(app.work.current_state.clone().unwrap());
    app.runtime.api = Some(api.clone());

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_max_steps(40)
        .build_eframe(|_| app);
    harness.step();
    let canvas = harness.get_by_label("Annotation canvas").rect();
    let image_aspect = 1280.0 / 800.0;
    let image_size = if canvas.width() / canvas.height() > image_aspect {
        egui::vec2(canvas.height() * image_aspect, canvas.height())
    } else {
        egui::vec2(canvas.width(), canvas.width() / image_aspect)
    };
    let image = egui::Rect::from_center_size(canvas.center(), image_size);
    let skipped_box_center = egui::pos2(
        image.left() + image.width() * 0.73,
        image.top() + image.height() * 0.49,
    );
    click_at(&mut harness, skipped_box_center);
    step_until(&mut harness, 8, |app| !app.work.migration.busy);

    assert_eq!(api.counts().migration_commands, 1);
    assert!(matches!(
        harness.state().work.migration.cursor,
        Some(labello_domain::MigrationCursor::Object { ref object_group_id, .. })
            if object_group_id == &skipped_group_id
    ));
    assert!(api.image_state(&image_id).migration_dependencies[&task_id]
        .contains_key(&skipped_group_id));
}

#[cfg(feature = "inspector-presets")]
#[test]
fn migration_skip_requires_confirmation_before_discarding_a_local_draft() {
    use crate::inspector_presets::{self, InspectorPreset};

    let api = Rc::new(SpyApi::new());
    let mut app =
        inspector_presets::build(InspectorPreset::MigrationObject, &egui::Context::default());
    api.set_image_state(app.work.current_state.clone().unwrap());
    app.runtime.api = Some(api.clone());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(|_| app);
    harness.step();
    harness.state_mut().work.migration.draft_dirty = true;

    click_accesskit_button(&mut harness, "Skip");
    harness.step();
    assert!(harness.query_by_label("Unsaved migration draft").is_some());
    assert!(
        harness
            .query_by_label("Discard draft and switch")
            .is_some()
    );
    assert!(harness.query_by_label("Submit and switch").is_none());
    assert_eq!(api.counts().release_assignment, 0);

    click_accesskit_button(&mut harness, "Cancel");
    harness.step();
    assert!(harness.state().work.pending_transition.is_none());
    assert!(harness.state().work.migration.draft_dirty);
    assert_eq!(api.counts().release_assignment, 0);
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
    harness.state_mut().work.canvas.fit_view();
    harness.step();
    let canvas = harness.get_by_label("Annotation canvas").rect();
    drag_at(
        &mut harness,
        canvas.center(),
        canvas.center() + egui::vec2(32.0, -16.0),
    );
    assert!(
        harness.state().work.migration.draft.as_ref().unwrap().keypoints[0]
            .point
            .is_some_and(|point| point.x > 0.5 && point.y < 0.5)
    );
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
    let availability_checks_before = api.counts().assignment_availability;

    harness.key_press(egui::Key::Space);
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
    assert!(
        harness
            .state()
            .work
            .previous_annotation_assignment
            .as_ref()
            .is_some_and(|assignment| assignment.status == AssignmentStatus::Completed)
    );
    harness.step();
    assert_eq!(api.counts().migration_commands, 1);
    assert!(
        api.counts().assignment_availability > availability_checks_before
    );
}

#[cfg(feature = "inspector-presets")]
#[test]
fn migration_review_approval_promotes_cached_work_without_refetching_image_data() {
    use crate::app::LoadedImage;
    use crate::inspector_presets::{self, InspectorPreset};
    use crate::queue::QueuedImage;

    let api = Rc::new(SpyApi::new());
    let mut app =
        inspector_presets::build(InspectorPreset::MigrationReview, &egui::Context::default());
    api.set_image_state(app.work.current_state.clone().unwrap());
    api.complete_next_migration_with(app.work.assignment.clone().unwrap());
    let next_image_id = ImageId::from("img_cached_migration_review");
    let next_assignment = Assignment {
        assignment_id: AssignmentId::generate(),
        image_id: next_image_id.clone(),
        task_id: app.work.selected_task_id.clone().unwrap(),
        assigned_to: app.config.user_id.clone(),
        kind: AssignmentKind::Review,
        status: AssignmentStatus::Active,
        expires_at: Some(now() + chrono::Duration::minutes(5)),
        created_at: now(),
        updated_at: now(),
    };
    api.add_active_assignment(next_assignment.clone());
    app.work.queue.clear();
    assert!(app.work.queue.push_prepared(LoadedImage {
        assignment: next_assignment,
        queued: QueuedImage {
            image: image_record(
                next_image_id.as_str(),
                "cached-migration-review.png",
                640,
                480,
            ),
            prelabels: Vec::new(),
        },
        annotations: Vec::new(),
        state: ImageState::new(next_image_id.clone()),
        color_image: Some(egui::ColorImage::from_rgba_unmultiplied(
            [1, 1],
            &[24, 48, 72, 255],
        )),
    }));
    api.set_no_assignment(true);
    app.runtime.api = Some(api.clone());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_max_steps(40)
        .build_eframe(|_| app);
    harness.step();
    let counts_before = api.counts();

    harness.key_press(egui::Key::Y);
    harness.step();
    step_until(&mut harness, 10, |app| {
        !app.work.migration.busy
            && app
                .work
                .assignment
                .as_ref()
                .is_some_and(|assignment| assignment.image_id == next_image_id)
    });

    assert_eq!(api.counts().migration_commands, 1);
    assert_eq!(api.counts().revalidate_assignment, 1);
    assert_eq!(
        api.counts().get_image_record,
        counts_before.get_image_record
    );
    assert_eq!(
        api.counts().get_image_preview,
        counts_before.get_image_preview
    );
    assert!(harness.state().work.current_texture.is_some());
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
    let context = object.get_by_label("Workspace context bar").rect();
    assert!(workflow.width() <= 44.5, "{workflow:?}");
    assert!(inspector.width() <= 44.5, "{inspector:?}");
    assert!(
        workflow.top() >= context.top() && workflow.bottom() <= context.bottom(),
        "context={context:?} workflow={workflow:?}"
    );
    assert!(inspector.top() >= context.top() && inspector.bottom() <= context.bottom());
    assert!(
        object.get_by_label("Annotation canvas").rect().bottom() <= primary.top(),
        "the canvas must stop above the compact migration action bar"
    );

    let canvas = object.get_by_label("Annotation canvas").rect();
    click_at(&mut object, canvas.center());
    object.step();
    for width in [400.0, 390.0, 360.0, 320.0] {
        object.set_size(egui::vec2(width, 667.0));
        object.step();
        let canvas = object.get_by_label("Annotation canvas").rect();
        let primary = object.get_by_label_contains("Save & next").rect();
        assert!(
            canvas.bottom() <= primary.top(),
            "width={width} canvas={canvas:?} primary={primary:?}"
        );
        for label in ["Undo last keypoint", "More"] {
            let action = object.get_by_label(label).rect();
            assert!(
                canvas.bottom() <= action.top(),
                "width={width} canvas={canvas:?} {label}={action:?}"
            );
        }
        let context = object.get_by_label("Workspace context bar").rect();
        for label in ["Workflow", "Inspector"] {
            let action = object.get_by_label(label).rect();
            assert!(
                action.top() >= context.top() && action.bottom() <= context.bottom(),
                "width={width} context={context:?} {label}={action:?}"
            );
        }
    }
    click_accesskit_button(&mut object, "Undo last keypoint");
    assert_eq!(object.state().work.migration.keypoint_index, 0);
    object.set_size(egui::vec2(320.0, 568.0));
    object.step();
    let primary = object.get_by_label_contains("Save & next").rect();
    let canvas = object.get_by_label("Annotation canvas").rect();
    assert!(
        canvas.bottom() <= primary.top(),
        "canvas={canvas:?} primary={primary:?}"
    );
    object.set_size(egui::vec2(390.0, 667.0));
    object.step();

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
        .with_size(egui::vec2(570.0, 667.0))
        .build_eframe(|ctx| {
            inspector_presets::build(InspectorPreset::MigrationObject, &ctx.egui_ctx)
        });
    roomy_compact.step();
    let workflow = roomy_compact.get_by_label("Workflow").rect();
    let inspector = roomy_compact.get_by_label("Inspector").rect();
    let context = roomy_compact
        .get_by_label("Workspace context bar")
        .rect();
    assert!(workflow.width() > 80.0, "{workflow:?}");
    assert!(inspector.width() > 80.0, "{inspector:?}");
    assert!(workflow.top() >= context.top() && workflow.bottom() <= context.bottom());
    assert!(inspector.top() >= context.top() && inspector.bottom() <= context.bottom());
    let canvas = roomy_compact.get_by_label("Annotation canvas").rect();
    click_at(&mut roomy_compact, canvas.center());
    roomy_compact.step();
    let undo = roomy_compact.get_by_label("Undo last keypoint").rect();
    let canvas = roomy_compact.get_by_label("Annotation canvas").rect();
    assert!(
        canvas.bottom() <= undo.top(),
        "canvas={canvas:?} undo={undo:?}"
    );

    let mut medium = Harness::builder()
        .with_size(egui::vec2(600.0, 667.0))
        .build_eframe(|ctx| {
            inspector_presets::build(InspectorPreset::MigrationObject, &ctx.egui_ctx)
        });
    medium.step();
    assert!(medium.query_by_label("Workflow").is_some());
    assert!(medium.query_by_label("Inspector").is_some());
    assert!(medium.query_by_label("Migration controls").is_none());
    assert!(medium.get_by_label("Workflow").rect().width() <= 44.5);
    assert!(medium.get_by_label("Inspector").rect().width() <= 44.5);
    let canvas = medium.get_by_label("Annotation canvas").rect();
    let first_row = medium.get_by_label_contains("Save skeleton & advance").rect();
    let inspector = medium.get_by_label("Inspector").rect();
    let context = medium.get_by_label("Workspace context bar").rect();
    assert!(
        canvas.bottom() <= first_row.top(),
        "canvas={canvas:?} first_row={first_row:?}"
    );
    assert!(
        inspector.top() >= context.top() && inspector.bottom() <= context.bottom(),
        "context control is clipped: context={context:?} inspector={inspector:?}"
    );

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
            .query_by_label("Exclude object")
            .is_some()
    );
    assert!(object.query_by_label("Reason").is_some());
    assert!(
        object
            .query_by_label_contains("Not present” applies")
            .is_some()
    );
    let visible = object.get_by_role_and_label(
        egui::accesskit::Role::Button,
        "Place head as visible",
    );
    let occluded = object.get_by_role_and_label(
        egui::accesskit::Role::Button,
        "Place head as occluded",
    );
    assert_eq!(
        visible.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
    assert_eq!(
        occluded.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::False)
    );
    assert!(visible.rect().height() >= 44.0);
    assert!(occluded.rect().height() >= 44.0);
    click_accesskit_button(&mut object, "Place head as occluded");
    assert!(object.state().work.migration.next_hidden);
    object.key_press(egui::Key::I);
    object.step();
    assert!(object.state().work.drawer.is_none());

    object.set_size(egui::vec2(260.0, 667.0));
    object.step();
    let workflow = object.get_by_label("Workflow").rect();
    let inspector = object.get_by_label("Inspector").rect();
    let context = object.get_by_label("Workspace context bar").rect();
    assert!(workflow.width() <= 44.5, "{workflow:?}");
    assert!(inspector.width() <= 44.5, "{inspector:?}");
    assert!((workflow.top() - inspector.top()).abs() <= 1.0);
    assert!(
        workflow.top() >= context.top()
            && workflow.bottom() <= context.bottom()
            && inspector.bottom() <= context.bottom(),
        "collapsed panel controls must remain in the context bar: \
         context={context:?} workflow={workflow:?} inspector={inspector:?}"
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
    assert!(
        full_image
            .query_by_label_contains("Add missing object")
            .is_some()
    );
    assert!(full_image.query_by_label("Workflow").is_some());
    assert!(full_image.query_by_label("Inspector").is_some());
    assert!(full_image.query_by_label("Controls").is_none());
    assert!(full_image.state().work.drawer.is_none());

    let mut wide_full_image = Harness::builder()
        .with_size(egui::vec2(1318.0, 900.0))
        .build_eframe(|ctx| {
            let mut app =
                inspector_presets::build(InspectorPreset::MigrationFullImage, &ctx.egui_ctx);
            app.work.inspector_panel_collapsed = true;
            app
        });
    wide_full_image.step();
    for label in ["Add missing object", "Confirm all guides & finish"] {
        let action = wide_full_image.get_by_label_contains(label).rect();
        assert!(
            action.right() <= 1318.0 && action.bottom() <= 900.0,
            "{label} must remain fully visible at the narrowest wide desktop size: {action:?}"
        );
    }
}

#[cfg(feature = "inspector-presets")]
#[test]
fn single_optional_migration_separates_not_present_from_object_exclusion() {
    use crate::inspector_presets::{self, InspectorPreset};

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(|ctx| {
            inspector_presets::build(
                InspectorPreset::MigrationSingleOptional,
                &ctx.egui_ctx,
            )
        });
    harness.step();

    let visible = harness.get_by_role_and_label(
        egui::accesskit::Role::Button,
        "Place center as visible",
    );
    let occluded = harness.get_by_role_and_label(
        egui::accesskit::Role::Button,
        "Place center as occluded",
    );
    assert_eq!(
        visible.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
    assert_eq!(
        occluded.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::False)
    );
    assert!(visible.rect().height() >= 44.0);
    assert!(occluded.rect().height() >= 44.0);
    assert!(
        harness
            .query_by_label("Visible: click the exact position.")
            .is_some()
    );

    let not_present = harness
        .query_all_by_label_contains("Mark center as not present")
        .find(|node| node.accesskit_node().role() == egui::accesskit::Role::Button)
        .unwrap();
    assert!(not_present.accesskit_node().is_disabled());
    harness
        .state_mut()
        .trigger_user_action(labello_domain::UserAction::MarkKeypointAbsent);
    assert_eq!(harness.state().work.migration.keypoint_index, 0);
    assert!(
        harness
            .query_by_label(
                "At least one keypoint position is required. If none can be placed, use Exclude object below.",
            )
            .is_some()
    );
    assert!(
        harness
            .get_by_label_contains("Save skeleton & advance")
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        harness
            .query_by_label("Exclude object")
            .is_some()
    );
    assert!(harness.query_by_label("Reason").is_some());
    assert!(
        harness
            .query_by_label_contains("Not present” applies")
            .is_some()
    );

    harness.key_press(egui::Key::H);
    harness.step();
    assert!(harness.state().work.migration.next_hidden);
    assert!(
        harness
            .query_by_label("Occluded: click the estimated position.")
            .is_some()
    );
    harness.key_press(egui::Key::H);
    harness.step();
    assert!(!harness.state().work.migration.next_hidden);
    click_accesskit_button(&mut harness, "Place center as occluded");
    let canvas = harness.get_by_label("Annotation canvas").rect();
    click_at(&mut harness, canvas.center());
    let draft = harness.state().work.migration.draft.as_ref().unwrap();
    assert_eq!(draft.keypoints[0].state, labello_domain::KeypointState::Hidden);
    assert!(draft.keypoints[0].point.is_some());
    assert_eq!(harness.state().work.migration.keypoint_index, 1);
    assert!(!harness.state().work.migration.next_hidden);
    assert!(!harness.state().work.migration.busy);

    assert!(
        harness
            .query_by_label("Exclude object & advance")
            .is_some()
    );
}

#[cfg(feature = "inspector-presets")]
#[test]
fn migration_decision_summary_counts_positioned_and_not_present_keypoints() {
    use crate::inspector_presets::{self, InspectorPreset};

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(|ctx| {
            inspector_presets::build(InspectorPreset::MigrationObject, &ctx.egui_ctx)
    });
    harness.step();
    let draft = harness
        .state_mut()
        .work
        .migration
        .draft
        .as_mut()
        .unwrap();
    draft.keypoints[0].point = Some(labello_domain::NormalizedPoint { x: 0.5, y: 0.5 });
    draft.keypoints[0].state = labello_domain::KeypointState::Visible;
    harness.state_mut().work.migration.keypoint_index = 1;
    harness.state_mut().work.migration.draft_dirty = true;
    harness.step();

    let first_not_present = harness
        .query_all_by_label_contains("Mark left_hand as not present")
        .find(|node| node.accesskit_node().role() == egui::accesskit::Role::Button)
        .unwrap();
    assert!(!first_not_present.accesskit_node().is_disabled());

    for index in 0..4 {
        harness.key_press(egui::Key::N);
        harness.step();
        let draft = harness.state().work.migration.draft.as_ref().unwrap();
        assert_eq!(harness.state().work.migration.keypoint_index, index + 2);
        assert_eq!(
            draft
                .keypoints
                .iter()
                .filter(|keypoint| keypoint.point.is_some())
                .count(),
            1
        );
        assert_eq!(
            draft
                .keypoints
                .iter()
                .take(harness.state().work.migration.keypoint_index)
                .filter(|keypoint| {
                    keypoint.state == labello_domain::KeypointState::Absent
                        && keypoint.point.is_none()
                })
                .count(),
            index + 1
        );
    }
    harness.step();
    assert!(
        !harness
            .get_by_label_contains("Save skeleton & advance")
            .accesskit_node()
            .is_disabled()
    );
}

#[cfg(feature = "inspector-presets")]
#[test]
fn migration_save_uses_the_contextual_submit_shortcut() {
    use crate::inspector_presets::{self, InspectorPreset};
    use crate::manual_migration::ManualMigrationState;

    let api = Rc::new(SpyApi::new());
    let mut app =
        inspector_presets::build(InspectorPreset::MigrationObject, &egui::Context::default());
    app.work.keybindings.bindings.insert(
        labello_domain::UserAction::NextImage,
        labello_domain::KeyChord::new("ArrowRight"),
    );
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
fn migration_review_refocus_restores_the_active_guide_view() {
    use crate::inspector_presets::{self, InspectorPreset};

    let app =
        inspector_presets::build(InspectorPreset::MigrationReview, &egui::Context::default());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(|_| app);
    harness.step();

    assert!(harness.state().work.canvas.current_zoom() > 1.0);
    click(&mut harness, "Fit");
    assert_eq!(harness.state().work.canvas.current_zoom(), 1.0);

    click_accesskit_button(&mut harness, "Refocus object");
    assert!(harness.state().work.canvas.current_zoom() > 1.0);
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

    let assert_review_layout =
        |harness: &Harness<'static, LabelloApp>, accept: &str, reject: &str| {
            let accept = harness.get_by_label(accept).rect();
            let reject = harness.get_by_label(reject).rect();
            let width = harness.ctx.content_rect().width();
            let workflow = harness.get_by_label("Workflow").rect();
            let inspector = harness.get_by_label("Inspector").rect();
            let context = harness.get_by_label("Workspace context bar").rect();
            assert!((accept.center().y - reject.center().y).abs() <= 1.0);
            assert!(accept.right() <= reject.left());
            assert!((accept.width() - reject.width()).abs() <= 1.0);
            assert!(accept.left() <= 16.0 && reject.right() >= width - 16.0);
            assert!(workflow.top() >= context.top() && workflow.bottom() <= context.bottom());
            assert!(inspector.top() >= context.top() && inspector.bottom() <= context.bottom());
        };

    harness.set_size(egui::vec2(570.0, 667.0));
    harness.step();
    assert_review_layout(&harness, "Accept", "Reject");
    assert!(harness.get_by_label("Workflow").rect().width() > 80.0);
    assert!(harness.get_by_label("Inspector").rect().width() > 80.0);

    harness.set_size(egui::vec2(390.0, 667.0));
    harness.step();
    assert_review_layout(&harness, "Accept", "Reject");
    assert!(harness.get_by_label("Workflow").rect().width() <= 44.5);
    assert!(harness.get_by_label("Inspector").rect().width() <= 44.5);

    harness.set_size(egui::vec2(260.0, 667.0));
    harness.step();
    assert_review_layout(&harness, "Accept", "Reject");
    assert!(harness.get_by_label("Workflow").rect().width() <= 44.5);
    assert!(harness.get_by_label("Inspector").rect().width() <= 44.5);

    harness.set_size(egui::vec2(150.0, 667.0));
    harness.step();
    let accept = harness.get_by_label("Y").rect();
    let reject = harness.get_by_label("N").rect();
    assert!((accept.center().y - reject.center().y).abs() <= 1.0);
    assert!((accept.width() - reject.width()).abs() <= 1.0);

    harness.set_size(egui::vec2(90.0, 667.0));
    harness.step();
    let row_positions = ["Y", "N"].map(|label| harness.get_by_label(label).rect().center().y);
    assert!(
        (row_positions[0] - row_positions[1]).abs() > 1.0,
        "review controls may reflow only after both decisions use their shortcuts",
    );
    assert!(harness.query_by_label("Controls").is_none());

    harness.set_size(egui::vec2(390.0, 667.0));
    harness.step();
    harness.key_press(egui::Key::Y);
    harness.step();
    step_until(&mut harness, 8, |app| !app.work.migration.busy);
    assert_eq!(api.counts().migration_commands, 1);
}
