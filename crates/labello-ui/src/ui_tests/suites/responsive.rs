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

#[cfg(feature = "inspector-presets")]
#[test]
fn preflight_diagnostics_are_an_accessible_responsive_disclosure() {
    use crate::inspector_presets::{self, InspectorPreset};

    for (size, disclosure_label, acknowledged_label) in [
        (
            egui::vec2(1180.0, 1600.0),
            "Diagnostics — 1 warning · 6 affected · 1 acknowledgement required",
            "Diagnostics — 1 warning · 6 affected",
        ),
        (
            egui::vec2(390.0, 1600.0),
            "Diagnostics (1 warning) · action required",
            "Diagnostics (1 warning)",
        ),
    ] {
        let mut harness = Harness::builder().with_size(size).build_eframe(|ctx| {
            inspector_presets::build(InspectorPreset::ImportPreflight, &ctx.egui_ctx)
        });
        harness.step();

        let disclosure =
            harness.get_by_role_and_label(egui::accesskit::Role::Button, disclosure_label);
        assert_eq!(disclosure.accesskit_node().data().is_expanded(), Some(true));
        let acknowledgement = harness.get_by_role_and_label(
            egui::accesskit::Role::CheckBox,
            "Acknowledge geometry_clipped",
        );
        acknowledgement.click_accesskit();
        harness.step();
        harness.step();
        assert!(
            harness
                .query_by_role_and_label(egui::accesskit::Role::Button, acknowledged_label)
                .is_some()
        );

        let disclosure =
            harness.get_by_role_and_label(egui::accesskit::Role::Button, acknowledged_label);
        disclosure.click_accesskit();
        harness.step();

        let collapsed =
            harness.get_by_role_and_label(egui::accesskit::Role::Button, acknowledged_label);
        assert_eq!(collapsed.accesskit_node().data().is_expanded(), Some(false));
        assert!(
            harness
                .query_by_role_and_label(
                    egui::accesskit::Role::CheckBox,
                    "Acknowledge geometry_clipped",
                )
                .is_none()
        );
        assert_visible_controls_clamped(&harness, size.x, size.y);
    }
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
        if layout == LayoutMode::Wide {
            let inspector_boundary = width - LayoutMode::INSPECTOR_PANEL_WIDTH;
            assert!(
                canvas.rect().right() <= inspector_boundary + 0.5,
                "canvas crosses the inspector boundary at {width}x{height}: {:?}",
                canvas.rect(),
            );
        }
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
