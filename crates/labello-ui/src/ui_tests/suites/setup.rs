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
