pub(super) fn live_harness(api: Rc<SpyApi>) -> Harness<'static, LabelloApp> {
    Harness::builder()
        .with_size(egui::vec2(1500.0, 780.0))
        .with_max_steps(80)
        .build_eframe(|_| base_live_app(api))
}

pub(super) fn loaded_work_harness(api: Rc<SpyApi>) -> Harness<'static, LabelloApp> {
    let mut harness = live_harness(api);
    step_until(&mut harness, 8, |app| app.datasets.summaries.len() == 1);
    click(&mut harness, "Continue with Demo Dataset");
    step_until(&mut harness, 12, |app| app.work.current.is_some());
    harness
}

pub(super) fn loaded_review_harness(api: Rc<SpyApi>) -> Harness<'static, LabelloApp> {
    let mut harness = live_harness(api);
    step_until(&mut harness, 8, |app| app.datasets.summaries.len() == 1);
    click(&mut harness, "Review Demo Dataset");
    step_until(&mut harness, 12, |app| {
        app.view == AppView::Review && app.work.current.is_some()
    });
    harness
}

pub(super) fn loaded_admin_harness(api: Rc<SpyApi>) -> Harness<'static, LabelloApp> {
    let mut harness = live_harness(api);
    step_until(&mut harness, 8, |app| app.datasets.summaries.len() == 1);
    click(&mut harness, "Admin Demo Dataset");
    step_until(&mut harness, 8, |app| {
        app.view == AppView::Admin && app.datasets.admin_config.is_some() && !app.loading.admin
    });
    harness
}

pub(super) fn base_live_app(api: Rc<SpyApi>) -> LabelloApp {
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

pub(super) fn test_request(
    app: &LabelloApp,
    request_id: u64,
    dataset_id: Option<&str>,
) -> RequestIdentity {
    RequestIdentity {
        auth_epoch: app.auth_epoch,
        workspace_epoch: app.workspace_epoch,
        request_id,
        dataset_id: dataset_id.map(DatasetId::from),
    }
}

pub(super) fn saturate_command_queue(app: &mut LabelloApp) {
    app.runtime.commands.clear();
    app.runtime.active_requests.clear();
    for request_id in 80_000..80_064 {
        app.runtime.commands.push_back(UiCommand::DatasetList {
            request: test_request(app, request_id, None),
        });
    }
}

pub(super) fn viewport_sizes() -> [(f32, f32); 10] {
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

pub(super) fn click(harness: &mut Harness<'static, LabelloApp>, label: &str) {
    assert!(
        click_visible(harness, label),
        "button or label {label:?} was not visible"
    );
    harness.step();
}

pub(super) fn click_application_menu_item(harness: &mut Harness<'static, LabelloApp>, label: &str) {
    let direct_label = match label {
        "Setup" => "Open setup",
        "Tutorial" => "Open tutorial",
        "Settings" => "Open settings",
        other => other,
    };
    if click_visible(harness, direct_label) {
        harness.step();
        return;
    }
    click(harness, "Open navigation");
    click_accesskit_button(harness, label);
}

pub(super) fn select_setup_section(harness: &mut Harness<'static, LabelloApp>, label: &str) {
    harness.state_mut().setup.section = match label {
        "Datasets" => SetupSection::Datasets,
        "Connection" => SetupSection::Connection,
        "Create" => SetupSection::Create,
        "Import" => SetupSection::Import,
        _ => panic!("unknown setup section {label:?}"),
    };
    harness.step();
}

pub(super) fn select_admin_section(harness: &mut Harness<'static, LabelloApp>, label: &str) {
    let section = match label {
        "Overview" => AdminSection::Overview,
        "People" => AdminSection::People,
        "Images" => AdminSection::Images,
        "Schema" => AdminSection::Schema,
        "Automation" => AdminSection::Automation,
        "Backups" => AdminSection::Backups,
        _ => panic!("unknown Admin section {label:?}"),
    };
    if harness
        .query_by_role_and_label(egui::accesskit::Role::ComboBox, "Admin section")
        .is_some()
    {
        harness.state_mut().admin.section = section;
        harness.step();
    } else {
        click_accesskit_button(harness, label);
    }
}

pub(super) fn click_at(harness: &mut Harness<'static, LabelloApp>, pos: egui::Pos2) {
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

pub(super) fn drag_at(
    harness: &mut Harness<'static, LabelloApp>,
    start: egui::Pos2,
    end: egui::Pos2,
) {
    harness.event(egui::Event::PointerMoved(start));
    harness.event(egui::Event::PointerButton {
        pos: start,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    harness.event(egui::Event::PointerMoved(end));
    harness.step();
    harness.event(egui::Event::PointerButton {
        pos: end,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
}

pub(super) fn release_and_switch(harness: &mut Harness<'static, LabelloApp>) {
    assert!(harness.query_by_label("Release and switch").is_some());
    harness.state_mut().release_pending_transition();
    harness.step();
}

pub(super) fn click_accesskit_button(harness: &mut Harness<'static, LabelloApp>, label: &str) {
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

pub(super) fn click_visible(harness: &Harness<'static, LabelloApp>, label: &str) -> bool {
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

pub(super) fn step_until(
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
        "view={:?} setup={:?} import(open={}, capabilities_loading={}) current={:?} assignment={:?} loading(dataset={}, image={}, saving={}) pending={:?} error={:?}",
        harness.state().view,
        harness.state().setup.section,
        harness.state().import.open,
        harness.state().import.capabilities_loading,
        harness
            .state()
            .work.current
            .as_ref()
            .map(|current| current.image.image_id.clone()),
        harness
            .state()
            .work.assignment
            .as_ref()
            .map(|assignment| assignment.assignment_id.clone()),
        harness.state().loading.dataset,
        harness.state().loading.image,
        harness.state().loading.saving,
        harness.state().work.pending_transition,
        harness.state().runtime.error,
    );
}
