use super::*;

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
    step_until(&mut harness, 12, |app| app.current.is_some());
    harness
}

pub(super) fn loaded_review_harness(api: Rc<SpyApi>) -> Harness<'static, LabelloApp> {
    let mut harness = live_harness(api);
    step_until(&mut harness, 8, |app| app.datasets.summaries.len() == 1);
    click(&mut harness, "Review Demo Dataset");
    step_until(&mut harness, 12, |app| {
        app.view == AppView::Review && app.current.is_some()
    });
    harness
}

pub(super) fn loaded_adjudication_harness(api: Rc<SpyApi>) -> Harness<'static, LabelloApp> {
    let mut harness = live_harness(api);
    step_until(&mut harness, 8, |app| app.datasets.summaries.len() == 1);
    click(&mut harness, "Adjudicate Demo Dataset");
    step_until(&mut harness, 12, |app| {
        app.view == AppView::Adjudicate && app.current.is_some()
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

pub(super) fn assert_control_inside(
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

pub(super) fn assert_label_inside(
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

pub(super) fn assert_canvas_geometry(
    harness: &Harness<'static, LabelloApp>,
    width: f32,
    height: f32,
) {
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

pub(super) fn assert_visible_controls_clamped(
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

pub(super) fn click(harness: &mut Harness<'static, LabelloApp>, label: &str) {
    assert!(
        click_visible(harness, label),
        "button or label {label:?} was not visible"
    );
    harness.step();
}

pub(super) fn click_application_menu_item(harness: &mut Harness<'static, LabelloApp>, label: &str) {
    if click_visible(harness, "Menu") {
        harness.step();
    }
    let section = match label {
        "Setup" | "Annotate" | "Review" | "Adjudicate" | "Admin" | "Stats" => "Navigation",
        _ => "Workspace",
    };
    click(harness, section);
    click_accesskit_button(harness, label);
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
pub(super) struct SpyApi {
    pub(super) state: Rc<RefCell<SpyState>>,
}

impl SpyApi {
    pub(super) fn new() -> Self {
        Self {
            state: Rc::new(RefCell::new(SpyState::new())),
        }
    }

    pub(super) fn counts(&self) -> CallCounts {
        self.state.borrow().counts.clone()
    }

    pub(super) fn metadata(&self) -> DatasetMetadata {
        self.state.borrow().metadata.clone()
    }

    pub(super) fn events(&self) -> Vec<EventPayload> {
        self.state.borrow().events.clone()
    }

    pub(super) fn fail_next_preview(&self) {
        self.state.borrow_mut().fail_next_preview = true;
    }

    pub(super) fn clear_workflows(&self) {
        self.state.borrow_mut().metadata.tasks.clear();
    }

    pub(super) fn set_no_assignment(&self, value: bool) {
        self.state.borrow_mut().no_assignment = value;
    }

    pub(super) fn sanitize_metadata_roles(&self) {
        self.state.borrow_mut().metadata.role_assignments.clear();
    }

    pub(super) fn set_summary_roles(&self, roles: Vec<DatasetRole>) {
        self.state.borrow_mut().summary_roles = roles;
    }

    pub(super) fn fail_me(&self) {
        self.state.borrow_mut().fail_me = true;
    }

    pub(super) fn dataset_users(&self) -> Vec<DatasetUser> {
        self.state.borrow().users.clone()
    }

    pub(super) fn last_image_query(&self) -> Option<ImageExplorerQuery> {
        self.state.borrow().last_image_query.clone()
    }

    pub(super) fn last_oauth_return_to(&self) -> Option<String> {
        self.state.borrow().last_oauth_return_to.clone()
    }

    pub(super) fn fail_next_correction(&self) {
        self.state.borrow_mut().fail_next_correction = true;
    }

    pub(super) fn fail_next_batch(&self) {
        self.state.borrow_mut().fail_next_batch = true;
    }

    pub(super) fn fail_next_admin_save(&self) {
        self.state.borrow_mut().fail_next_admin_save = true;
    }

    pub(super) fn fail_role_save_at(&self, call: usize) {
        self.state.borrow_mut().fail_role_save_at = Some(call);
    }

    pub(super) fn last_correction(&self) -> Option<CorrectionRequest> {
        self.state.borrow().last_correction.clone()
    }

    pub(super) fn exclusions(&self) -> Vec<Vec<ImageId>> {
        self.state.borrow().exclusions.clone()
    }

    pub(super) fn has_active_assignment(&self, assignment_id: &AssignmentId) -> bool {
        self.state
            .borrow()
            .active_assignments
            .iter()
            .any(|assignment| &assignment.assignment_id == assignment_id)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct CallCounts {
    pub(super) auth_options: usize,
    pub(super) local_admin_login: usize,
    pub(super) me: usize,
    pub(super) logout: usize,
    pub(super) list_datasets: usize,
    pub(super) create_dataset: usize,
    pub(super) get_dataset: usize,
    pub(super) get_admin_dataset: usize,
    pub(super) update_dataset_config: usize,
    pub(super) ingest_dataset: usize,
    pub(super) assign_next_image: usize,
    pub(super) release_assignment: usize,
    pub(super) complete_assignment: usize,
    pub(super) reopen_assignment: usize,
    pub(super) get_image_record: usize,
    pub(super) get_image_state: usize,
    pub(super) get_image_preview: usize,
    pub(super) append_event: usize,
    pub(super) annotation_batch: usize,
    pub(super) rebuild_image: usize,
    pub(super) record_review: usize,
    pub(super) record_correction: usize,
    pub(super) record_adjudication: usize,
    pub(super) dataset_stats: usize,
    pub(super) get_keybindings: usize,
    pub(super) save_keybindings: usize,
    pub(super) prelabel_suggestions: usize,
    pub(super) list_dataset_users: usize,
    pub(super) set_dataset_roles: usize,
    pub(super) list_images: usize,
    pub(super) list_snapshots: usize,
    pub(super) create_snapshot: usize,
    pub(super) get_snapshot_file: usize,
}

pub(super) struct SpyState {
    pub(super) metadata: DatasetMetadata,
    pub(super) states: BTreeMap<ImageId, ImageState>,
    pub(super) counts: CallCounts,
    pub(super) next_image: usize,
    pub(super) events: Vec<EventPayload>,
    pub(super) fail_next_preview: bool,
    pub(super) no_assignment: bool,
    pub(super) active_assignments: Vec<Assignment>,
    pub(super) reopenable_assignments: Vec<Assignment>,
    pub(super) exclusions: Vec<Vec<ImageId>>,
    pub(super) completed_images: BTreeSet<ImageId>,
    pub(super) summary_roles: Vec<DatasetRole>,
    pub(super) users: Vec<DatasetUser>,
    pub(super) fail_me: bool,
    pub(super) last_image_query: Option<ImageExplorerQuery>,
    pub(super) last_oauth_return_to: Option<String>,
    pub(super) snapshots: Vec<DatasetSnapshot>,
    pub(super) fail_next_correction: bool,
    pub(super) fail_next_batch: bool,
    pub(super) fail_next_admin_save: bool,
    pub(super) fail_role_save_at: Option<usize>,
    pub(super) last_correction: Option<CorrectionRequest>,
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
            reopenable_assignments: Vec::new(),
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
            fail_next_admin_save: false,
            fail_role_save_at: None,
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
        if std::mem::take(&mut state.fail_next_admin_save) {
            return ready(Err(ClientError::Demo("admin save failed".to_string())));
        }
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
        state.reopenable_assignments.push(assignment.clone());
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
        state.reopenable_assignments.push(assignment.clone());
        ready(Ok(assignment))
    }

    fn reopen_assignment<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        request: AssignmentActionRequest,
    ) -> ApiFuture<'a, Assignment> {
        let mut state = self.state.borrow_mut();
        state.counts.reopen_assignment += 1;
        let Some(previous) = state
            .reopenable_assignments
            .iter()
            .find(|assignment| {
                assignment.assignment_id == request.assignment_id
                    && assignment.image_id == request.image_id
                    && assignment.task_id == request.task_id
                    && assignment.kind == request.kind
            })
            .cloned()
        else {
            return ready(Err(ClientError::Demo(
                "assignment cannot be reopened".to_string(),
            )));
        };
        let assignment = Assignment {
            assignment_id: AssignmentId::generate(),
            image_id: previous.image_id.clone(),
            task_id: previous.task_id,
            assigned_to: previous.assigned_to,
            kind: AssignmentKind::Annotation,
            status: AssignmentStatus::Active,
            expires_at: None,
            created_at: now(),
            updated_at: now(),
        };
        state.completed_images.remove(&assignment.image_id);
        state.active_assignments.push(assignment.clone());
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
            if let Some(position) = state
                .active_assignments
                .iter()
                .position(|active| assignment_matches(active, &assignment))
            {
                let mut completed = state.active_assignments.remove(position);
                completed.status = AssignmentStatus::Completed;
                completed.updated_at = now();
                state.reopenable_assignments.push(completed);
            }
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

    fn local_admin_login<'a>(&'a self) -> ApiFuture<'a, SessionInfo> {
        let mut state = self.state.borrow_mut();
        state.counts.local_admin_login += 1;
        ready(Ok(SessionInfo {
            account: state.users[0].account.clone(),
            can_create_datasets: true,
        }))
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

    fn me<'a>(&'a self) -> ApiFuture<'a, SessionInfo> {
        let mut state = self.state.borrow_mut();
        state.counts.me += 1;
        if state.fail_me {
            ready(Err(ClientError::Api {
                status: 401,
                message: "login required".to_string(),
            }))
        } else {
            ready(Ok(SessionInfo {
                account: state.users[0].account.clone(),
                can_create_datasets: true,
            }))
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
        if state.fail_role_save_at == Some(state.counts.set_dataset_roles) {
            state.fail_role_save_at = None;
            return ready(Err(ClientError::Demo("role save failed".to_string())));
        }
        let user = state
            .users
            .iter_mut()
            .find(|user| user.account.user_id == request.user_id)
            .unwrap();
        user.roles = request.roles;
        ready(Ok(user.clone()))
    }
}

pub(super) fn ready<'a, T: 'a>(result: ClientResult<T>) -> ApiFuture<'a, T> {
    Box::pin(async move { result })
}

pub(super) fn assignment_matches(
    assignment: &Assignment,
    request: &AssignmentActionRequest,
) -> bool {
    assignment.assignment_id == request.assignment_id
        && assignment.image_id == request.image_id
        && assignment.task_id == request.task_id
        && assignment.kind == request.kind
        && assignment.status == AssignmentStatus::Active
}

pub(super) fn image_record(
    image_id: &str,
    file_name: &str,
    width: u32,
    height: u32,
) -> ImageRecord {
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

pub(super) fn test_snapshot(dataset_id: DatasetId) -> DatasetSnapshot {
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

pub(super) fn task(id: &str, name: &str, prelabel_configs: Vec<&str>) -> TaskDefinition {
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

pub(super) fn seed_review_annotation(
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

pub(super) fn prelabel_config(id: &str) -> PrelabelConfig {
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

pub(super) fn stats(total_images: usize) -> DatasetStats {
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

pub(super) fn now() -> labello_domain::Timestamp {
    labello_domain::now()
}
