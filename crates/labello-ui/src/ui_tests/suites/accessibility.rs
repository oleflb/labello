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
                    app.work.drawer = Some(Drawer::Inspector);
                }
                app
            });
        migration.step();
        assert!(migration.query_by_label("Annotation canvas").is_some());
        if width >= 600.0 || height >= 667.0 {
            assert!(
                migration
                    .query_by_label("Canonical bounding-box guide · read only")
                    .is_some()
            );
            assert!(
                migration
                    .query_by_label("Object 1 of 2")
                    .is_some()
            );
            assert!(
                migration
                    .query_by_label("Exclude object")
                    .is_some()
            );
        }
        if LayoutMode::for_width(width) == LayoutMode::Wide {
            let canvas = migration.get_by_label("Annotation canvas").rect();
            let inspector = migration.get_by_label("Inspector").rect();
            let workflow_label = migration.state().selected_workflow().unwrap().label();
            let workflow_boundary = migration
                .get_by_role_and_label(egui::accesskit::Role::Button, &workflow_label)
                .rect()
                .right()
                + theme::SPACE_4
                + theme::side_frame().stroke.width;
            let inspector_boundary = width - LayoutMode::INSPECTOR_PANEL_WIDTH;
            let selected_workflow = migration
                .get_by_role_and_label(egui::accesskit::Role::Button, &workflow_label);
            assert_eq!(
                selected_workflow.accesskit_node().description(),
                Some("Loaded assignment queue: 2 of 2".to_string())
            );
            let workflow_gutter = canvas.left() - workflow_boundary;
            let inspector_gutter = inspector_boundary - canvas.right();
            assert!(
                canvas.right() <= inspector_boundary + 0.5,
                "canvas crosses the inspector boundary: canvas={canvas:?} boundary={inspector_boundary}",
            );
            assert!(
                (workflow_gutter - inspector_gutter).abs() <= 0.5,
                "canvas gutters differ: workflow={workflow_gutter} inspector={inspector_gutter} canvas={canvas:?}",
            );
            assert!(
                inspector.left() >= inspector_boundary + 26.0,
                "inspector text touches its scroll clip: inspector={inspector:?} boundary={inspector_boundary}",
            );
            assert!(
                canvas.right() + 15.0 <= inspector.left(),
                "canvas crowds the inspector text: canvas={canvas:?} inspector={inspector:?}",
            );
            let exclusion_note = migration
                .get_by_role_and_label(
                    egui::accesskit::Role::MultilineTextInput,
                    "Note (optional)",
                )
                .rect();
            assert!(
                exclusion_note.right() <= width - 16.5,
                "migration controls overflow the inspector panel: note={exclusion_note:?}",
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
            .query_by_label_contains("Confirm all guides & finish")
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
    let task_id = no_guides_app.work.selected_task_id.clone().unwrap();
    let state = no_guides_app.work.current_state.as_mut().unwrap();
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
    no_guides_app.work.migration.cursor = Some(labello_domain::MigrationCursor::FullImage);
    no_guides_app.work.migration.progress = None;
    let no_guides_api = Rc::new(SpyApi::new());
    no_guides_api.set_image_state(no_guides_app.work.current_state.clone().unwrap());
    let mut no_guides = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_max_steps(40)
        .build_eframe(|_| no_guides_app);
    no_guides.step();
    assert!(
        no_guides
            .query_by_label_contains("Confirm no guides & finish")
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
    step_until(&mut no_guides, 8, |app| !app.work.migration.busy);
    assert_eq!(no_guides_api.counts().migration_commands, 1);

    let mut deleted = Harness::builder()
        .with_size(egui::vec2(390.0, 667.0))
        .build_eframe(|ctx| {
            let mut app =
                inspector_presets::build(InspectorPreset::MigrationGuideDeleted, &ctx.egui_ctx);
            app.work.drawer = Some(Drawer::Inspector);
            app
        });
    deleted.step();
    assert!(deleted.query_by_label("Object 1 of 2").is_some());
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

    for size in [egui::vec2(1180.0, 1000.0), egui::vec2(390.0, 900.0)] {
        let mut yolo = Harness::builder().with_size(size).build_eframe(|ctx| {
            inspector_presets::build(InspectorPreset::ImportYoloSplits, &ctx.egui_ctx)
        });
        yolo.step();
        for split in ["train", "val", "test"] {
            assert!(
                yolo.query_by_role_and_label(egui::accesskit::Role::CheckBox, split)
                    .is_some(),
                "missing {split} at {size:?}"
            );
        }
        assert!(yolo.query_by_label("Pairing group (optional)").is_none());
        assert!(yolo.query_by_label("Add COCO descriptor").is_none());
        assert!(
            !yolo
                .get_by_role_and_label(
                    egui::accesskit::Role::Button,
                    "Seal source and run preflight"
                )
                .accesskit_node()
                .is_disabled()
        );
        assert_visible_controls_clamped(&yolo, size.x, size.y);
    }

    for (preset, action) in [
        (
            InspectorPreset::ImportServerFolderPicker,
            "Open folder release-2026",
        ),
        (
            InspectorPreset::ImportServerDescriptorPicker,
            "Select dataset.yaml",
        ),
    ] {
        for size in [
            egui::vec2(1180.0, 900.0),
            egui::vec2(390.0, 844.0),
            egui::vec2(320.0, 320.0),
        ] {
            let mut picker = Harness::builder()
                .with_size(size)
                .build_eframe(|ctx| inspector_presets::build(preset, &ctx.egui_ctx));
            picker.step();
            assert!(
                picker
                    .query_by_role_and_label(egui::accesskit::Role::Window, "Server source picker")
                    .is_some()
            );
            assert!(picker.query_by_label(action).is_some());
            assert!(picker.query_by_label("Close picker").is_some());
            assert_visible_controls_clamped(&picker, size.x, size.y);
        }
    }

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
            .query_by_label("Refocus box")
            .is_some()
    );
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
    harness.state_mut().work.save_status = SaveStatus::Dirty;
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
fn desktop_app_bar_shows_direct_navigation_and_accessible_icon_actions() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);
    harness.set_size(egui::vec2(1500.0, 780.0));
    harness.step();

    for label in ["Annotate", "Review", "Statistics", "Admin"] {
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
