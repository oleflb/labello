#[cfg(feature = "inspector-presets")]
#[test]
fn target_keypoint_typing_does_not_echo_into_template_controls() {
    use crate::inspector_presets::{self, InspectorPreset};

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1180.0, 1600.0))
        .build_eframe(|ctx| {
            let mut app = inspector_presets::build(InspectorPreset::ImportReady, &ctx.egui_ctx);
            let category = &mut app.import.categories[0];
            category.target_keypoint_names = "nose".to_string();
            category
                .geometry_mappings
                .push(labello_client::ImportGeometryMappingRequest {
                    source_category_key: category.source_category_key.clone(),
                    source_geometry: labello_client::ImportGeometryKind::BoundingBox,
                    target_geometry: labello_client::ImportGeometryKind::Skeleton,
                    policy: labello_client::ImportGeometryPolicy::BoxRelativeTemplateV1,
                    parameters: Vec::new(),
                });
            app
        });
    harness.step();

    assert!(
        harness
            .query_by_label(
                "Required outputs for the current mapping — categories: 1, tasks: 2. Accepted \
                 preflight outputs — categories: 1, tasks: 1. Click “Save mappings and re-run \
                 preflight”; commit remains disabled until the refreshed plan includes every \
                 required output."
            )
            .is_some()
    );

    let input = harness.get_by_role_and_label(
        egui::accesskit::Role::TextInput,
        "Target keypoint names (comma separated)",
    );
    let input_top = input.rect().top();
    assert!(
        harness
            .get_by_label("Template point positions")
            .rect()
            .top()
            > input_top
    );

    input.focus();
    harness.step();
    assert!(
        harness
            .query_by_label(
                "Template-point controls will update after you finish editing keypoint names."
            )
            .is_some()
    );
    assert!(harness.query_by_label("Template point positions").is_none());

    harness
        .get_by_role_and_label(
            egui::accesskit::Role::TextInput,
            "Target keypoint names (comma separated)",
        )
        .type_text(",left_eye");
    harness.step();
    assert!(harness.query_by_label("Template point positions").is_none());

    harness.key_press(egui::Key::Tab);
    harness.step();
    assert!(harness.query_by_label("Template point positions").is_some());
    assert_eq!(
        harness.state().import.categories[0].geometry_mappings[1]
            .parameters
            .iter()
            .filter_map(|parameter| {
                let labello_client::ImportMappingParameter::Point { name, .. } = parameter else {
                    return None;
                };
                Some(name.as_str())
            })
            .collect::<Vec<_>>(),
        ["nose", "left_eye"]
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

    assert!(harness.state().work.current.is_none());
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
    step_until(&mut harness, 12, |app| app.work.current.is_some());
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
        app.selected_class_id() == Some(&ClassId::from("vehicle")) && app.work.current.is_some()
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

    let annotation = harness.state().work.annotations.last().unwrap();
    assert_eq!(annotation.task_id, TaskId::from("bounding_box:vehicle"));
    assert_eq!(annotation.class_id, ClassId::from("vehicle"));
}

#[test]
fn workflow_selector_uses_equal_compact_cards_and_type_icons() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);
    let mut skeleton = harness.state().work.tasks[0].clone();
    skeleton.task_id = TaskId::from("skeleton:person");
    skeleton.name = "Person skeleton with a deliberately long workflow name".to_string();
    skeleton.annotation_type = AnnotationType::Skeleton;
    skeleton.skeleton = Some(SkeletonSpec {
        keypoints: vec![KeypointSpec {
            name: "head".to_string(),
            required: true,
        }],
        edges: Vec::new(),
        allow_hidden: true,
        allow_absent: true,
    });
    harness.state_mut().work.tasks.push(skeleton);
    harness.step();

    let bounding_box = harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Person boxes")
        .rect();
    let vehicle = harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Vehicle boxes")
        .rect();
    let skeleton = harness
        .get_by_role_and_label(
            egui::accesskit::Role::Button,
            "Person skeleton with a deliberately long workflow name",
        )
        .rect();
    assert_eq!(bounding_box.width(), vehicle.width());
    assert_eq!(bounding_box.width(), skeleton.width());
    assert_eq!(bounding_box.height(), vehicle.height());
    assert_eq!(bounding_box.height(), skeleton.height());
    assert!(
        bounding_box.width() > LayoutMode::TASK_PANEL_WIDTH,
        "the longest workflow pill should expand the workflow panel: {bounding_box:?}"
    );
    assert!(bounding_box.height() <= 52.0);
    assert!(
        skeleton.top() - bounding_box.bottom() <= 8.0,
        "bounding_box={bounding_box:?} skeleton={skeleton:?}"
    );
    assert!(
        vehicle.top() - skeleton.bottom() <= 8.0,
        "skeleton={skeleton:?} vehicle={vehicle:?}"
    );
    assert!(
        harness
            .query_all_by_label("bounding box annotation type")
            .next()
            .is_some()
    );
    assert!(harness.query_by_label("skeleton annotation type").is_some());
}

#[test]
fn wide_workflow_panel_keeps_its_toggle_beside_fit() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);
    harness.set_size(egui::vec2(1500.0, 780.0));
    for _ in 0..4 {
        harness.step();
    }

    let expanded_canvas = harness.get_by_label("Annotation canvas").rect();
    let fit = harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Fit")
        .rect();
    let collapse = harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Collapse workflow panel")
        .rect();
    assert!(
        collapse.left() >= fit.right() && collapse.left() - fit.right() <= 12.0,
        "the collapse control should sit beside Fit: fit={fit:?} collapse={collapse:?}"
    );
    assert_eq!(collapse.top(), fit.top());
    assert_eq!(collapse.bottom(), fit.bottom());
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::Button, "Person boxes")
            .is_some()
    );

    click_accesskit_button(&mut harness, "Collapse workflow panel");
    assert!(
        harness.output().platform_output.num_completed_passes > 0,
        "collapsing should settle its geometry before presenting the frame"
    );
    harness.step();

    let collapsed_canvas = harness.get_by_label("Annotation canvas").rect();
    assert!(harness.state().work.workflow_panel_collapsed);
    assert!(
        harness.query_by_label("Inspector").is_some(),
        "collapsing the workflow panel should keep the inspector visible"
    );
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::Button, "Person boxes")
            .is_none()
    );
    assert!(
        harness
            .query_by_role_and_label(
                egui::accesskit::Role::Button,
                "Expand workflow panel",
            )
            .is_some()
    );
    let fit = harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Fit")
        .rect();
    let expand = harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Expand workflow panel")
        .rect();
    assert!(
        expand.left() >= fit.right() && expand.left() - fit.right() <= 12.0,
        "the expand control should sit beside Fit: fit={fit:?} expand={expand:?}"
    );
    assert_eq!(expand.top(), fit.top());
    assert_eq!(expand.bottom(), fit.bottom());
    assert!(
        collapsed_canvas.left() < expanded_canvas.left(),
        "collapsing should return horizontal space to the canvas: \
         expanded={expanded_canvas:?} collapsed={collapsed_canvas:?}"
    );
    assert!(
        collapsed_canvas.left() <= theme::SPACE_2 + 1.0,
        "the collapsed panel should not leave a rail: {collapsed_canvas:?}"
    );

    click_accesskit_button(&mut harness, "Expand workflow panel");
    for _ in 0..4 {
        harness.step();
    }

    assert!(!harness.state().work.workflow_panel_collapsed);
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::Button, "Person boxes")
            .is_some()
    );
    assert_eq!(
        harness.get_by_label("Annotation canvas").rect().left(),
        expanded_canvas.left()
    );
}

#[test]
fn workflow_availability_disables_cards_skips_keyboard_cycles_and_retries_failures() {
    let api = Rc::new(SpyApi::new());
    api.set_workflow_availability("bounding_box:vehicle", false);
    let mut harness = loaded_work_harness(api.clone());
    step_until(&mut harness, 12, |app| {
        app.workflow_availability(&TaskId::from("bounding_box:vehicle")) == Some(false)
    });

    let vehicle = harness
        .query_all_by_label_contains("Vehicle boxes")
        .find(|node| node.accesskit_node().role() == egui::accesskit::Role::Button)
        .unwrap();
    assert!(vehicle.accesskit_node().is_disabled());
    let vehicle = harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Vehicle boxes");
    assert_eq!(
        vehicle.accesskit_node().description(),
        Some("No assignments available".to_string())
    );
    harness.state_mut().work.availability.tasks.clear();
    harness.state_mut().work.availability.loading = true;
    harness.step();
    assert!(
        harness
            .query_by_role_and_label(
                egui::accesskit::Role::ProgressIndicator,
                "Loading workflow assignment availability",
            )
            .is_some(),
        "the initial availability check should retain its spinner"
    );
    let availability_spinner = harness
        .get_by_role_and_label(
            egui::accesskit::Role::ProgressIndicator,
            "Loading workflow assignment availability",
        )
        .rect();
    let context_bar = harness.get_by_label("Workspace context bar").rect();
    let workflow_pill = harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Person boxes")
        .rect();
    assert!(
        context_bar.contains_rect(availability_spinner),
        "availability spinner should live in the workspace context bar: \
         spinner={availability_spinner:?} context={context_bar:?}"
    );
    assert!(
        context_bar.right() - availability_spinner.right() <= 16.0,
        "availability spinner should be right aligned: \
         spinner={availability_spinner:?} context={context_bar:?}"
    );
    assert!(
        availability_spinner.left() > workflow_pill.right(),
        "availability spinner should no longer live in the workflow panel: \
         spinner={availability_spinner:?} pill={workflow_pill:?}"
    );
    harness.set_size(egui::vec2(390.0, 844.0));
    harness.step();
    let compact_spinner = harness
        .get_by_role_and_label(
            egui::accesskit::Role::ProgressIndicator,
            "Loading workflow assignment availability",
        )
        .rect();
    let compact_context = harness.get_by_label("Workspace context bar").rect();
    assert!(compact_context.contains_rect(compact_spinner));
    assert!(
        compact_context.right() - compact_spinner.right() <= 16.0,
        "compact availability spinner should be right aligned: \
         spinner={compact_spinner:?} context={compact_context:?}"
    );
    harness.set_size(egui::vec2(1500.0, 780.0));
    harness.step();
    harness
        .state_mut()
        .work
        .availability
        .tasks
        .insert(TaskId::from("bounding_box:vehicle"), false);
    harness.step();
    assert!(
        harness
            .query_by_role_and_label(
                egui::accesskit::Role::ProgressIndicator,
                "Loading workflow assignment availability",
            )
            .is_none(),
        "background refreshes should keep the resolved workflow state stable"
    );
    harness.state_mut().work.availability.loading = false;

    let mut skeleton = harness.state().work.tasks[0].clone();
    skeleton.task_id = TaskId::from("skeleton:person");
    skeleton.name = "Person skeleton".to_string();
    skeleton.annotation_type = AnnotationType::Skeleton;
    skeleton.skeleton = Some(SkeletonSpec {
        keypoints: vec![KeypointSpec {
            name: "head".to_string(),
            required: true,
        }],
        edges: Vec::new(),
        allow_hidden: true,
        allow_absent: true,
    });
    harness.state_mut().work.tasks.push(skeleton);
    harness
        .state_mut()
        .work.availability
        .tasks
        .insert(TaskId::from("skeleton:person"), true);
    harness
        .state_mut()
        .trigger_user_action(labello_domain::UserAction::SelectNextWorkflow);
    assert!(matches!(
        harness.state().work.pending_transition,
        Some(crate::app::PendingTransition::Workflow(ref task_id))
            if task_id == &TaskId::from("skeleton:person")
    ));

    harness.state_mut().work.pending_transition = None;
    api.fail_next_availability();
    harness.state_mut().work.availability.last_attempt = None;
    harness.state_mut().request_assignment_availability();
    step_until(&mut harness, 8, |app| app.work.availability.error.is_some());
    assert!(
        !harness
            .query_all_by_label_contains("Vehicle boxes")
            .find(|node| node.accesskit_node().role() == egui::accesskit::Role::Button)
            .unwrap()
            .accesskit_node()
            .is_disabled(),
        "failed availability remains advisory"
    );
    click_accesskit_button(&mut harness, "Retry availability");
    step_until(&mut harness, 8, |app| {
        !app.work.availability.loading && app.work.availability.error.is_none()
    });
    assert!(api.counts().assignment_availability >= 3);
}

#[test]
fn assignment_load_waits_for_availability_and_selects_the_next_available_workflow() {
    let api = Rc::new(SpyApi::new());
    let mut app = base_live_app(api.clone());
    app.sync_work_config(api.metadata());
    app.view = AppView::Annotate;

    app.request_next_image();

    assert!(!app.loading.image);
    assert!(app.work.active_load_id.is_none());
    assert!(app.work.availability.load_after_resolution);
    let UiCommand::AssignmentAvailability { request, .. } =
        app.runtime.commands.pop_back().unwrap()
    else {
        panic!("expected availability before assignment claim");
    };
    assert!(app.runtime.commands.is_empty());

    app.runtime
        .tx
        .send(UiMessage::AssignmentAvailabilityLoaded {
            request,
            result: Ok(labello_client::AssignmentAvailability {
                kind: AssignmentKind::Annotation,
                tasks: BTreeMap::from([
                    (TaskId::from("bounding_box:person"), false),
                    (TaskId::from("bounding_box:vehicle"), true),
                ]),
                related: vec![labello_client::AssignmentAvailabilityEntry {
                    kind: AssignmentKind::Adjudication,
                    tasks: BTreeMap::from([
                        (TaskId::from("bounding_box:person"), true),
                        (TaskId::from("bounding_box:vehicle"), false),
                    ]),
                }],
            }),
        })
        .unwrap();
    app.process_messages(&egui::Context::default());

    assert!(app.work.availability.resolved);
    assert!(!app.work.availability.load_after_resolution);
    assert_eq!(
        app.work.selected_task_id.as_ref(),
        Some(&TaskId::from("bounding_box:vehicle"))
    );
    let UiCommand::ClaimAssignment { task_id, .. } = app.runtime.commands.pop_back().unwrap()
    else {
        panic!("expected assignment claim after availability");
    };
    assert_eq!(task_id, TaskId::from("bounding_box:vehicle"));

    app.execute_transition(crate::app::PendingTransition::View(
        AppView::Adjudicate,
    ));

    assert!(app.runtime.commands.iter().any(|command| matches!(
        command,
        UiCommand::ClaimAssignment {
            kind: AssignmentKind::Adjudication,
            task_id,
            ..
        } if task_id == &TaskId::from("bounding_box:person")
    )));
    assert!(app.runtime.commands.iter().all(|command| !matches!(
        command,
        UiCommand::AssignmentAvailability { .. }
    )));
}

#[test]
fn fresh_cached_availability_survives_reload_without_another_check() {
    let api = Rc::new(SpyApi::new());
    api.set_workflow_availability("bounding_box:person", false);
    let mut harness = loaded_work_harness(api.clone());
    step_until(&mut harness, 8, |app| {
        app.runtime
            .persistence
            .preference
            .as_ref()
            .is_some_and(|preference| preference.availability.is_some())
    });
    let mut preference = harness
        .state()
        .runtime
        .persistence
        .preference
        .clone()
        .unwrap();
    preference.task_id = Some(TaskId::from("bounding_box:person"));
    preference.availability.as_mut().unwrap().checked_at =
        labello_domain::now() - Duration::from_secs(20);

    let mut reloaded = base_live_app(api.clone());
    reloaded.runtime.persistence.preference = Some(preference);
    reloaded.sync_work_config(api.metadata());
    reloaded.view = AppView::Annotate;

    assert!(reloaded.restore_cached_assignment_availability());
    assert!(
        reloaded
            .work
            .availability
            .last_attempt
            .is_some_and(|attempt| attempt.elapsed() < Duration::from_secs(1)),
        "restoring a wall-clock cache must not backdate the new page's monotonic clock"
    );
    assert!(
        reloaded
            .assignment_availability_cache_age()
            .is_some_and(|age| age >= Duration::from_secs(19))
    );
    reloaded.request_next_image();

    assert_eq!(
        reloaded.work.selected_task_id.as_ref(),
        Some(&TaskId::from("bounding_box:vehicle"))
    );
    assert!(
        reloaded
            .runtime
            .commands
            .iter()
            .all(|command| !matches!(command, UiCommand::AssignmentAvailability { .. }))
    );
    assert!(reloaded.runtime.commands.iter().any(|command| matches!(
        command,
        UiCommand::ClaimAssignment { task_id, .. }
            if task_id == &TaskId::from("bounding_box:vehicle")
    )));

    reloaded.refresh_assignment_availability_if_due();
    assert!(
        reloaded
            .runtime
            .commands
            .iter()
            .all(|command| !matches!(command, UiCommand::AssignmentAvailability { .. }))
    );
}

#[test]
fn fresh_availability_survives_workflow_and_view_navigation() {
    let api = Rc::new(SpyApi::new());
    let mut app = base_live_app(api.clone());
    app.sync_work_config(api.metadata());
    app.view = AppView::Annotate;
    app.work.availability.dataset_id = Some(app.config.dataset_id.clone());
    app.work.availability.kind = Some(AssignmentKind::Annotation);
    app.work.availability.tasks = app
        .work
        .tasks
        .iter()
        .map(|task| (task.task_id.clone(), true))
        .collect();
    app.work.availability.resolved = true;
    app.work.availability.checked_at = Some(labello_domain::now());

    app.execute_transition(crate::app::PendingTransition::Workflow(TaskId::from(
        "bounding_box:vehicle",
    )));

    assert!(app.runtime.commands.iter().any(|command| matches!(
        command,
        UiCommand::ClaimAssignment { task_id, .. }
            if task_id == &TaskId::from("bounding_box:vehicle")
    )));
    assert!(app.runtime.commands.iter().all(|command| !matches!(
        command,
        UiCommand::AssignmentAvailability { .. }
    )));

    app.execute_transition(crate::app::PendingTransition::View(AppView::Stats));
    app.execute_transition(crate::app::PendingTransition::View(AppView::Annotate));

    assert!(app.runtime.commands.iter().any(|command| matches!(
        command,
        UiCommand::ClaimAssignment { task_id, .. }
            if task_id == &TaskId::from("bounding_box:vehicle")
    )));
    assert!(app.runtime.commands.iter().all(|command| !matches!(
        command,
        UiCommand::AssignmentAvailability { .. }
    )));
}

#[test]
fn fresh_availability_is_cached_per_assignment_kind() {
    let api = Rc::new(SpyApi::new());
    let mut app = base_live_app(api.clone());
    app.sync_work_config(api.metadata());
    app.view = AppView::Annotate;
    app.work.availability.dataset_id = Some(app.config.dataset_id.clone());
    app.work.availability.kind = Some(AssignmentKind::Annotation);
    app.work.availability.tasks = app
        .work
        .tasks
        .iter()
        .map(|task| (task.task_id.clone(), true))
        .collect();
    app.work.availability.resolved = true;
    app.work.availability.checked_at = Some(labello_domain::now());

    app.execute_transition(crate::app::PendingTransition::View(AppView::Review));
    assert!(app.runtime.commands.iter().any(|command| matches!(
        command,
        UiCommand::AssignmentAvailability {
            kind: AssignmentKind::Review,
            ..
        }
    )));

    app.work.availability.dataset_id = Some(app.config.dataset_id.clone());
    app.work.availability.kind = Some(AssignmentKind::Review);
    app.work.availability.tasks = app
        .work
        .tasks
        .iter()
        .map(|task| (task.task_id.clone(), true))
        .collect();
    app.work.availability.resolved = true;
    app.work.availability.checked_at = Some(labello_domain::now());
    app.work.availability.loading = false;
    app.execute_transition(crate::app::PendingTransition::View(AppView::Stats));
    app.execute_transition(crate::app::PendingTransition::View(AppView::Annotate));

    assert_eq!(
        app.work.availability.kind,
        Some(AssignmentKind::Annotation)
    );
    assert!(app.runtime.commands.iter().all(|command| !matches!(
        command,
        UiCommand::AssignmentAvailability { .. }
    )));

    app.execute_transition(crate::app::PendingTransition::View(AppView::Review));

    assert_eq!(app.work.availability.kind, Some(AssignmentKind::Review));
    assert!(app.runtime.commands.iter().all(|command| !matches!(
        command,
        UiCommand::AssignmentAvailability { .. }
    )));
}

#[test]
fn released_workflow_transition_reuses_fresh_availability() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    step_until(&mut harness, 12, |app| {
        app.work.current.is_some() && app.work.availability.resolved
    });
    let checks_before = api.counts().assignment_availability;
    harness.state_mut().work.pending_transition =
        Some(crate::app::PendingTransition::Workflow(TaskId::from(
            "bounding_box:vehicle",
        )));
    harness.state_mut().release_pending_transition();

    step_until(&mut harness, 12, |app| {
        app.work.selected_task_id.as_ref() == Some(&TaskId::from("bounding_box:vehicle"))
            && app.work.current.is_some()
    });

    assert_eq!(api.counts().assignment_availability, checks_before);
}

#[test]
fn expired_or_wrong_scope_cached_availability_requires_a_new_check() {
    let api = Rc::new(SpyApi::new());
    let metadata = api.metadata();
    let tasks = metadata
        .tasks
        .iter()
        .map(|task| (task.task_id.clone(), true))
        .collect();
    let preference = WorkspacePreference {
        version: 2,
        dataset_id: DatasetId::from("demo"),
        view: StoredView::Annotate,
        task_id: Some(TaskId::from("bounding_box:person")),
        assignment_id: None,
        assignment_image_id: None,
        assignment_kind: None,
        drawer: None,
        workflow_panel_collapsed: false,
        inspector_panel_collapsed: false,
        show_settings: false,
        show_tutorial: false,
        selected_annotation: None,
        canvas: StoredCanvasTransform {
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
        },
        availability: Some(StoredAssignmentAvailability {
            kind: AssignmentKind::Annotation,
            tasks,
            checked_at: labello_domain::now() - Duration::from_secs(31),
        }),
    };

    for stale in [
        preference.clone(),
        WorkspacePreference {
            availability: Some(StoredAssignmentAvailability {
                kind: AssignmentKind::Review,
                checked_at: labello_domain::now(),
                ..preference.availability.clone().unwrap()
            }),
            ..preference
        },
    ] {
        let mut app = base_live_app(api.clone());
        app.sync_work_config(metadata.clone());
        app.view = AppView::Annotate;
        app.runtime.persistence.preference = Some(stale.clone());

        assert!(!app.restore_cached_assignment_availability());
        app.request_next_image();
        assert!(matches!(
            app.runtime.commands.pop_front(),
            Some(UiCommand::AssignmentAvailability { .. })
        ));
    }
}

#[test]
fn failed_or_empty_availability_never_starts_an_assignment_load() {
    let api = Rc::new(SpyApi::new());
    let mut app = base_live_app(api.clone());
    app.sync_work_config(api.metadata());
    app.view = AppView::Annotate;
    app.request_next_image();
    let UiCommand::AssignmentAvailability { request, .. } =
        app.runtime.commands.pop_back().unwrap()
    else {
        panic!("expected availability request");
    };

    app.runtime
        .tx
        .send(UiMessage::AssignmentAvailabilityLoaded {
            request,
            result: Err("availability failed".to_string()),
        })
        .unwrap();
    app.process_messages(&egui::Context::default());

    assert!(!app.work.availability.resolved);
    assert!(app.work.availability.load_after_resolution);
    assert!(app.runtime.commands.is_empty());
    assert!(!app.loading.image);

    app.request_assignment_availability();
    let UiCommand::AssignmentAvailability { request, .. } =
        app.runtime.commands.pop_back().unwrap()
    else {
        panic!("expected availability retry");
    };
    app.runtime
        .tx
        .send(UiMessage::AssignmentAvailabilityLoaded {
            request,
            result: Ok(labello_client::AssignmentAvailability {
                kind: AssignmentKind::Annotation,
                tasks: BTreeMap::from([
                    (TaskId::from("bounding_box:person"), false),
                    (TaskId::from("bounding_box:vehicle"), false),
                ]),
                related: Vec::new(),
            }),
        })
        .unwrap();
    app.process_messages(&egui::Context::default());

    assert!(app.work.availability.resolved);
    assert!(app.work.availability.load_after_resolution);
    assert!(app.runtime.commands.is_empty());
    assert!(!app.loading.image);
    assert!(
        app.runtime
            .notice
            .as_deref()
            .is_some_and(|notice| notice.contains("currently available"))
    );
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
        !app.loading.dataset && !app.loading.image && app.work.availability.resolved
    });

    assert!(harness.state().work.current.is_none());
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
fn right_arrow_submits_and_claims_a_different_image() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    let original = harness
        .state()
        .work.assignment
        .as_ref()
        .unwrap()
        .image_id
        .clone();

    step_until(&mut harness, 12, |app| app.work.queue.len() == 2);
    let next = harness.state().work.queue.prepared_image_ids()[0].clone();
    let previews_before = api.counts().get_image_preview;
    harness.key_press(egui::Key::ArrowRight);
    step_until(&mut harness, 16, |app| {
        app.work.assignment
            .as_ref()
            .is_some_and(|assignment| assignment.image_id != original)
    });

    assert_eq!(api.counts().complete_assignment, 1);
    assert_eq!(api.counts().release_assignment, 0);
    assert_eq!(harness.state().work.assignment.as_ref().unwrap().image_id, next);
    assert_eq!(api.counts().get_image_preview, previews_before);
    assert!(!harness.state().loading.image);
    assert!(harness.state().work.current_texture.is_some());
}

#[test]
fn annotation_prefetch_fills_two_without_blocking_the_current_image() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    step_until(&mut harness, 12, |app| app.work.queue.len() == 2);

    assert_eq!(harness.state().work.queue.queue_size(), 2);
    assert!(!harness.state().loading.image);
    let selected_workflow =
        harness.get_by_role_and_label(egui::accesskit::Role::Button, "Person boxes");
    assert_eq!(
        selected_workflow.accesskit_node().description(),
        Some("Loaded assignment queue: 2 of 2".to_string())
    );
    assert!(
        harness.query_by_label("2/2").is_none(),
        "the assignment queue should not be rendered inside the workflow pill"
    );
    selected_workflow.hover();
    harness.run_steps(3);
    assert!(
        harness
            .query_by_label_contains("Loaded assignment queue: 2 of 2")
            .is_some(),
        "the selected workflow tooltip should expose the assignment queue"
    );
    assert!(harness.query_by_label("Assignment").is_none());
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
    step_until(&mut harness, 12, |app| app.work.queue.len() == 2);
    harness.state_mut().work.queue.clear();

    click(&mut harness, "Submit & next");
    harness.step();
    assert!(harness.state().loading.image);
    assert!(harness.state().work.current.is_none());
    harness.step();
    assert!(harness.state().work.current.is_some());
}

#[test]
fn submit_failure_preserves_current_and_prepared_queue() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    step_until(&mut harness, 12, |app| app.work.queue.len() == 2);
    let current = harness
        .state()
        .work.assignment
        .as_ref()
        .unwrap()
        .image_id
        .clone();
    let queued = harness.state().work.queue.prepared_image_ids();
    api.fail_next_batch();

    click(&mut harness, "Submit & next");
    step_until(&mut harness, 8, |app| !app.loading.saving);

    assert_eq!(
        harness.state().work.assignment.as_ref().unwrap().image_id,
        current
    );
    assert_eq!(harness.state().work.queue.prepared_image_ids(), queued);
}

#[test]
fn save_keeps_the_same_assignment_active() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    step_until(&mut harness, 12, |app| app.work.queue.len() == 2);
    let claims_before = api.counts().assign_next_image;
    click(&mut harness, "Accept");
    let assignment_id = harness
        .state()
        .work.assignment
        .as_ref()
        .unwrap()
        .assignment_id
        .clone();

    click(&mut harness, "Save");
    step_until(&mut harness, 10, |app| app.work.save_status == SaveStatus::Saved);

    assert_eq!(
        harness
            .state()
            .work.assignment
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
    assert_eq!(harness.state().work.save_status, SaveStatus::Dirty);
    assert_eq!(api.counts().append_event, 0);
    click(&mut harness, "More actions");
    assert!(harness.query_by_label_contains("Undo").is_some());
    harness.key_press(egui::Key::Escape);
    harness.step();

    harness.state_mut().undo();
    assert!(harness.state().work.annotations.is_empty());
    harness.state_mut().redo();
    assert_eq!(harness.state().work.annotations.len(), 1);

    harness.state_mut().work.last_edit_at = Some(Instant::now() - Duration::from_secs(1));
    harness.state_mut().autosave_if_due();
    assert_eq!(harness.state().work.save_status, SaveStatus::Saving);
    step_until(&mut harness, 10, |app| app.work.save_status == SaveStatus::Saved);
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
            .work.annotations
            .iter()
            .all(|annotation| annotation.deleted)
    );
    harness.state_mut().redo();
    assert_eq!(
        harness
            .state()
            .work.annotations
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
    assert!(harness.state().work.canvas.is_dragging());

    harness.state_mut().work.last_edit_at = Some(Instant::now() - Duration::from_secs(1));
    harness.state_mut().autosave_if_due();

    assert_eq!(harness.state().work.save_status, SaveStatus::Dirty);
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
    assert_eq!(harness.state().work.save_status, SaveStatus::Dirty);
    assert_eq!(harness.state().work.annotations.len(), 2);
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
    assert_eq!(harness.state().work.active_operation_id, None);
    assert_eq!(harness.state().work.save_status, SaveStatus::Retry);
    assert!(harness.state().work.pending_transition.is_none());
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
        workflow_panel_collapsed: false,
        inspector_panel_collapsed: false,
        show_settings: false,
        show_tutorial: false,
        selected_annotation: None,
        canvas: StoredCanvasTransform {
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
        },
        availability: None,
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
fn demo_submit_and_skip_advance_images() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1500.0, 780.0))
        .build_eframe(|_| LabelloApp::default());
    assert_eq!(
        harness.state().work.current.as_ref().unwrap().image.file_name,
        "demo_1.jpg"
    );

    click(&mut harness, "Submit & next");
    assert_eq!(
        harness.state().work.current.as_ref().unwrap().image.file_name,
        "demo_2.jpg"
    );

    click(&mut harness, "Skip");
    assert_eq!(
        harness.state().work.current.as_ref().unwrap().image.file_name,
        "demo_3.jpg"
    );

    click(&mut harness, "Skip");
    assert_eq!(
        harness.state().work.current.as_ref().unwrap().image.file_name,
        "demo_4.jpg"
    );
}

#[test]
fn skip_releases_then_claims_another_assignment() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    let original = harness
        .state()
        .work.assignment
        .as_ref()
        .unwrap()
        .image_id
        .clone();

    step_until(&mut harness, 12, |app| app.work.queue.len() == 2);
    let previews_before = api.counts().get_image_preview;
    click(&mut harness, "Skip");
    assert!(
        harness
            .query_by_label("Switch active assignment?")
            .is_none()
    );
    step_until(&mut harness, 16, |app| {
        app.work.assignment
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
    let original = harness.state().work.assignment.clone().unwrap();
    step_until(&mut harness, 12, |app| app.work.queue.len() == 2);

    click(&mut harness, "Skip");
    step_until(&mut harness, 16, |app| {
        app.work.assignment
            .as_ref()
            .is_some_and(|assignment| assignment.image_id != original.image_id)
            && app.work.previous_annotation_assignment.is_some()
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
        app.work.assignment
            .as_ref()
            .is_some_and(|assignment| assignment.image_id == original.image_id)
            && !app.loading.image
    });

    assert_eq!(api.counts().reopen_assignment, 1);
    assert_ne!(
        harness.state().work.assignment.as_ref().unwrap().assignment_id,
        original.assignment_id
    );
    assert!(harness.state().work.previous_annotation_assignment.is_none());
}

#[test]
fn previous_assignment_reopens_the_exact_submitted_image() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    harness.set_size(egui::vec2(1500.0, 780.0));
    harness.step();
    let original = harness.state().work.assignment.clone().unwrap();
    step_until(&mut harness, 12, |app| app.work.queue.len() == 2);

    click(&mut harness, "Submit & next");
    step_until(&mut harness, 16, |app| {
        app.work.assignment
            .as_ref()
            .is_some_and(|assignment| assignment.image_id != original.image_id)
            && app.work.previous_annotation_assignment.is_some()
    });
    click(&mut harness, "Previous");
    step_until(&mut harness, 20, |app| {
        app.work.assignment
            .as_ref()
            .is_some_and(|assignment| assignment.image_id == original.image_id)
            && !app.loading.image
    });

    assert_eq!(api.counts().reopen_assignment, 1);
    assert_ne!(
        harness.state().work.assignment.as_ref().unwrap().assignment_id,
        original.assignment_id
    );
}

#[test]
fn expired_locally_retained_previous_assignment_is_not_loaded() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    let mut previous = harness.state().work.assignment.clone().unwrap();
    previous.assignment_id = AssignmentId::generate();
    previous.expires_at = Some(now() - chrono::Duration::seconds(1));
    harness.state_mut().work.previous_annotation_assignment = Some(previous);

    harness.state_mut().return_to_previous_assignment();

    assert!(harness.state().work.previous_annotation_assignment.is_none());
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
fn failed_refill_keeps_the_one_shot_image_excluded() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    step_until(&mut harness, 12, |app| app.work.queue.len() == 2);
    harness.state_mut().work.queue.pop_prepared();
    let skipped = ImageId::from("img_skipped");
    harness.state_mut().work.one_shot_excluded_image_id = Some(skipped.clone());
    api.fail_next_preview();

    harness.state_mut().request_prefetch();
    harness.step();
    step_until(&mut harness, 16, |app| app.work.queue.failed());

    assert_eq!(
        harness.state().work.one_shot_excluded_image_id.as_ref(),
        Some(&skipped)
    );
    assert!(api.exclusions().last().unwrap().contains(&skipped));
}

#[test]
fn dirty_skip_requires_an_explicit_discard_or_submit_choice() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    click(&mut harness, "Accept");
    assert_eq!(harness.state().work.save_status, SaveStatus::Dirty);

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
    harness.state_mut().work.last_edit_at = Some(Instant::now() - Duration::from_secs(1));
    harness.state_mut().autosave_if_due();
    assert_eq!(harness.state().work.save_status, SaveStatus::Dirty);
    assert!(!harness.state().loading.saving);
    assert_eq!(api.counts().annotation_batch, batches);

    click_accesskit_button(&mut harness, "Cancel");
    assert!(harness.state().work.pending_transition.is_none());
    assert_eq!(api.counts().release_assignment, 0);

    harness.state_mut().work.last_edit_at = Some(Instant::now());
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
        app.view == AppView::Review && app.work.current.is_some()
    });

    assert_eq!(api.counts().prelabel_suggestions, 0);
    assert!(harness.query_by_label("Prelabels").is_none());
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
        harness.state_mut().work.drawer =
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
        harness.state_mut().work.drawer = Some(Drawer::Inspector);
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
fn work_workflow_draws_saves_submits_reviews_and_adjudicates() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    assert!(harness.state().work.current.is_some());
    assert_eq!(harness.state().work.queue.queue_size(), IMAGE_QUEUE_SIZE);
    assert!(harness.query_by_label("Assignment").is_none());
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
    assert_eq!(harness.state().work.annotations.len(), 1);
    assert_eq!(
        harness.state().work.selected_annotation.as_ref(),
        Some(&harness.state().work.annotations[0].annotation_id)
    );
    assert_eq!(harness.state().work.save_status, SaveStatus::Dirty);

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
    assert_eq!(harness.state().work.annotations.len(), 2);

    click(&mut harness, "Save");
    step_until(&mut harness, 10, |app| app.work.save_status == SaveStatus::Saved);
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
        app.work.current
            .as_ref()
            .is_some_and(|current| current.image.image_id == ImageId::from("img_2"))
    });
    assert_eq!(api.counts().complete_assignment, 1);

    assert!(api.counts().assign_next_image >= 2);

    harness.state_mut().work.drawer = Some(Drawer::Inspector);
    click_application_menu_item(&mut harness, "Review");
    assert!(
        harness
            .query_by_label("Switch active assignment?")
            .is_some()
    );
    release_and_switch(&mut harness);
    step_until(&mut harness, 10, |app| {
        app.view == AppView::Review && app.work.current.is_some() && !app.loading.image
    });
    assert!(harness.state().work.drawer.is_none());
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
        app.work.assignment
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
        app.view == AppView::Adjudicate && app.work.current.is_some() && !app.loading.image
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
        .work.current
        .as_ref()
        .unwrap()
        .image
        .image_id
        .clone();

    click(&mut harness, "Accept");
    assert_eq!(harness.state().work.save_status, SaveStatus::Dirty);
    click(&mut harness, "Vehicle boxes");
    assert!(
        harness
            .query_by_label("Switch active assignment?")
            .is_some()
    );
    assert_eq!(
        harness.state().work.selected_task_id.as_ref(),
        Some(&TaskId::from("bounding_box:person"))
    );
    harness.state_mut().submit_pending_transition();
    harness.step();
    step_until(&mut harness, 12, |app| {
        app.selected_class_id() == Some(&ClassId::from("vehicle"))
            && app.work.current.is_some()
            && !app.loading.saving
    });

    assert!(api.counts().append_event >= 1);
    assert_eq!(api.counts().complete_assignment, 1);
    assert_ne!(
        harness.state().work.current.as_ref().unwrap().image.image_id,
        original_image
    );
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

    assert_eq!(harness.state().work.annotations.len(), 1);
    let AnnotationGeometry::Skeleton(skeleton) = &harness.state().work.annotations[0].geometry else {
        panic!("expected skeleton annotation");
    };
    assert_eq!(skeleton.keypoints.len(), 2);
    assert!(
        skeleton
            .keypoints
            .iter()
            .all(|keypoint| keypoint.point.is_some())
    );
    assert!(harness.state().work.active_skeleton.is_none());
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
            .work.correction_draft
            .as_ref()
            .unwrap()
            .geometry_changed()
    );
    assert!(matches!(
        harness.state().work.annotations[0].geometry,
        AnnotationGeometry::BoundingBox(box_geometry) if box_geometry == original
    ));
    assert_eq!(api.counts().annotation_batch, 0);
    harness
        .state_mut()
        .request_review(labello_domain::ReviewDecision::Approved);
    assert_eq!(api.counts().record_review, 0);
    assert!(harness.state().work.correction_draft.is_some());

    api.fail_next_correction();
    click(&mut harness, "Correct & finalize");
    step_until(&mut harness, 8, |app| !app.loading.saving);
    assert_eq!(api.counts().record_correction, 1);
    assert!(harness.state().work.correction_draft.is_some());
    assert!(harness.state().work.current.is_some());

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
    let canonical = harness.state().work.annotations[0].annotation_id.clone();
    let mut arbitrary = harness.state().work.annotations[0].clone();
    arbitrary.annotation_id = labello_domain::AnnotationId::from("arbitrary");
    harness.state_mut().work.annotations.push(arbitrary.clone());
    harness.state_mut().work.selected_annotation = Some(arbitrary.annotation_id.clone());

    harness
        .state_mut()
        .request_review(labello_domain::ReviewDecision::Approved);
    assert_eq!(
        harness.state().work.selected_annotation.as_ref(),
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
    harness.state_mut().work.active_operation_id = None;
    harness.state_mut().loading.saving = false;
    harness.state_mut().work.review_index = harness.state().work.annotations.len();
    harness.state_mut().work.selected_annotation = Some(arbitrary.annotation_id);
    harness.state_mut().sync_review_selection();
    assert!(harness.state().work.selected_annotation.is_none());
    assert!(!harness.state().can_correct_review_object());
    harness.state_mut().start_correction();
    assert!(harness.state().work.correction_draft.is_none());
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
    let original_review_expiry = review.state().work.assignment.as_ref().unwrap().expires_at;
    let state_reads = review_api.counts().get_image_state;
    review
        .state_mut()
        .request_review(labello_domain::ReviewDecision::Approved);
    step_until(&mut review, 8, |app| !app.loading.saving);
    assert_eq!(review_api.counts().get_image_state, state_reads);
    assert!(
        review.state().work.assignment.as_ref().unwrap().expires_at > original_review_expiry,
        "review response did not renew the active assignment"
    );

    let save_api = Rc::new(SpyApi::new());
    let mut work = loaded_work_harness(save_api);
    click(&mut work, "Accept");
    let original_save_expiry = work.state().work.assignment.as_ref().unwrap().expires_at;
    work.state_mut().request_save(false);
    step_until(&mut work, 8, |app| !app.loading.saving);
    assert!(
        work.state().work.assignment.as_ref().unwrap().expires_at > original_save_expiry,
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

    let draft = harness.state().work.correction_draft.as_ref().unwrap();
    let AnnotationGeometry::Skeleton(skeleton) = &draft.edited_geometry else {
        panic!("expected skeleton correction draft");
    };
    assert_eq!(skeleton.keypoints[0].state, KeypointState::Hidden);
    assert_eq!(skeleton.keypoints[0].point.unwrap().x, 0.65);
    assert!(matches!(
        harness.state().work.annotations[0].geometry,
        AnnotationGeometry::Skeleton(ref original)
            if original.keypoints[0].state == KeypointState::Visible
                && original.keypoints[0].point.unwrap().x == 0.5
    ));

    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::Z);
    harness.step();
    let draft = harness.state().work.correction_draft.as_ref().unwrap();
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
    let draft = harness.state().work.correction_draft.as_ref().unwrap();
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
    assert!(harness.state().work.annotations[0].deleted);
    assert!(harness.state().work.selected_annotation.is_none());
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
    let annotation_id = harness.state().work.annotations[0].annotation_id.clone();
    assert_eq!(
        harness.state().work.selected_annotation.as_ref(),
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
        harness.state().work.annotations[0].geometry,
        AnnotationGeometry::BoundingBox(BoundingBox { x, .. }) if (x - 0.1).abs() < f32::EPSILON
    ));

    harness.key_press(egui::Key::Delete);
    harness.step();
    assert!(harness.state().work.annotations[0].deleted);
    assert!(harness.state().work.selected_annotation.is_none());
    harness.state_mut().undo();
    assert!(!harness.state().work.annotations[0].deleted);

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
    assert_eq!(harness.state().work.annotations.len(), 1);
    harness.state_mut().undo();
    assert!(harness.state().work.annotations.is_empty());
    harness.state_mut().redo();
    assert_eq!(harness.state().work.annotations.len(), 1);
}
