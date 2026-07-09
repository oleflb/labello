use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};

use eframe::egui;
use egui_kittest::{Harness, kittest::Queryable};
use labello_client::{
    AdjudicationApi, AnnotationApi, ApiFuture, AppendEventRequest, AssignNextRequest, AuthApi,
    ClientError, ClientResult, CorrectionRequest, CreateDatasetRequest, DatasetApi, DatasetSummary,
    ImageApi, ImageFile, ImagePreview, IngestJob, IngestJobStatus, IngestReport, KeybindingApi,
    OAuthCallbackRequest, OAuthLoginRequest, OfflineApi, OfflineBundleRequest, PrelabelApi,
    PrelabelSuggestionRequest, ReviewApi, StatsApi, TaskApi, UpdateDatasetConfigRequest,
};
use labello_domain::{
    AdjudicationRecord, AnnotationGeometry, AnnotationType, Assignment, AssignmentId,
    AssignmentKind, AssignmentStatus, BoundingBox, BrowserAcceleration, ClassId, DatasetId,
    DatasetMetadata, DatasetRole, DatasetRoleAssignment, DatasetStats, EventLogEntry, EventPayload,
    ImageId, ImageRecord, ImageState, KeybindingSet, LabelClass, ModelSpec, OfflineBundle,
    OfflineSyncRequest, OfflineSyncResult, OutputProcessing, PrelabelConfig, PrelabelConfigId,
    PrelabelExecution, PrelabelSuggestion, ReviewConfig, ReviewRecord, SCHEMA_VERSION,
    TaskDefinition, TaskId, TutorialContent, UserAccount, UserId,
};

use crate::app::{
    AppConfig, AppView, FolderUploadProgress, IMAGE_QUEUE_SIZE, LabelloApp, QueueMode, SaveStatus,
    UiMessage,
};

#[test]
fn setup_create_open_and_admin_workflows_use_live_commands() {
    let api = Rc::new(SpyApi::new());
    let mut harness = live_harness(api.clone());
    step_until(&mut harness, 8, |app| app.datasets.summaries.len() == 1);

    assert!(harness.query_by_label("Connect To Labello").is_some());
    assert!(harness.query_by_label("Open").is_some());
    assert!(harness.query_all_by_label("Admin").next().is_some());

    click(&mut harness, "Create as current user");
    step_until(&mut harness, 20, |app| app.current.is_some());
    assert_eq!(api.counts().create_dataset, 1);
    assert_eq!(harness.state().view, AppView::Annotate);
    assert!(harness.state().current.is_some());

    click(&mut harness, "Setup");
    harness.step();
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
    click_accesskit_button(&mut harness, "Add label");
    harness.step();
    click_accesskit_button(&mut harness, "Add task");
    harness.step();
    click_accesskit_button(&mut harness, "Add browser prelabel config");
    harness.step();
    click_accesskit_button(&mut harness, "Add role assignment");
    harness.step();
    click_accesskit_button(&mut harness, "Use bounding box workflow");
    harness.step();

    let config = harness.state().datasets.admin_config.as_ref().unwrap();
    assert_eq!(config.image_roots.len(), 2);
    assert_eq!(config.label_classes.len(), 3);
    assert_eq!(config.prelabel_configs.len(), 2);
    assert_eq!(config.role_assignments.len(), 2);
    assert_eq!(config.tasks.len(), 1);
    assert_eq!(config.tasks[0].annotation_type, AnnotationType::BoundingBox);
    assert_eq!(config.tasks[0].class_ids.len(), 1);

    click_accesskit_button(&mut harness, "Save Admin Config");
    step_until(&mut harness, 8, |_| api.counts().update_dataset_config == 1);
    assert_eq!(api.counts().update_dataset_config, 1);
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
            .contains("Ingested")
    );

    click(&mut harness, "Work");
    step_until(&mut harness, 12, |app| app.current.is_some());
    assert_eq!(harness.state().view, AppView::Annotate);
}

#[test]
fn image_load_failure_shows_retry_and_loads_image() {
    let api = Rc::new(SpyApi::new());
    api.fail_next_preview();
    let mut harness = live_harness(api.clone());
    step_until(&mut harness, 8, |app| app.datasets.summaries.len() == 1);
    click(&mut harness, "Open");
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
}

#[test]
fn workers_select_class_specific_workflows() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api);

    assert!(harness.query_by_label("Person bounding box").is_some());
    assert!(harness.query_by_label("Vehicle bounding box").is_some());
    click(&mut harness, "Vehicle bounding box");
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
    click(&mut harness, "Open");
    step_until(&mut harness, 20, |app| {
        app.runtime
            .error
            .as_deref()
            .is_some_and(|error| error.contains("No enabled workflow"))
    });

    assert!(
        harness
            .state()
            .runtime
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("No enabled workflow")
    );
}

#[test]
fn work_workflow_draws_saves_submits_reviews_and_adjudicates() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    assert!(harness.state().current.is_some());
    assert_eq!(harness.state().queue.queue_size(), IMAGE_QUEUE_SIZE);
    assert!(harness.query_by_label("Queue size: 8").is_some());
    assert!(harness.query_by_label("Approve y").is_none());
    assert!(harness.query_by_label("Reject n").is_none());
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

    click(&mut harness, "Submit");
    step_until(&mut harness, 10, |app| {
        app.save_status == SaveStatus::Saved && !app.loading.saving
    });
    assert!(api.events().iter().any(|payload| matches!(
        payload,
        EventPayload::TaskStateChanged { task_state }
            if task_state.status == labello_domain::TaskStatus::Submitted
    )));

    click(&mut harness, "Next image");
    step_until(&mut harness, 10, |app| {
        app.current
            .as_ref()
            .is_some_and(|current| current.image.image_id == ImageId::from("img_2"))
    });
    assert!(api.counts().assign_next_image >= 2);

    click(&mut harness, "Review");
    step_until(&mut harness, 10, |app| {
        app.queue_mode == QueueMode::Review && !app.loading.image
    });
    assert!(harness.query_by_label("Approve y").is_some());
    assert!(harness.query_by_label("Reject n").is_some());
    assert!(harness.query_by_label("Accept").is_none());
    click(&mut harness, "Approve y");
    step_until(&mut harness, 10, |app| !app.loading.saving);
    assert_eq!(api.counts().record_review, 1);

    click(&mut harness, "Reject n");
    step_until(&mut harness, 10, |app| !app.loading.saving);
    assert_eq!(api.counts().record_review, 2);

    click(&mut harness, "Adjudicate");
    step_until(&mut harness, 10, |app| {
        app.queue_mode == QueueMode::Adjudicate && !app.loading.image
    });
    assert!(harness.query_by_label("Adjudicate accept").is_some());
    assert!(harness.query_by_label("Needs correction").is_some());
    assert!(harness.query_by_label("Approve y").is_none());
    click(&mut harness, "Adjudicate accept");
    step_until(&mut harness, 10, |app| !app.loading.saving);
    click(&mut harness, "Needs correction");
    step_until(&mut harness, 10, |app| !app.loading.saving);
    assert_eq!(api.counts().record_adjudication, 2);

    harness.key_press(egui::Key::ArrowRight);
    step_until(&mut harness, 10, |app| !app.loading.image);
    assert!(api.counts().assign_next_image >= 3);
}

#[test]
fn stats_and_responsive_layouts_render_without_losing_primary_actions() {
    let api = Rc::new(SpyApi::new());
    let mut harness = loaded_work_harness(api.clone());
    assert_eq!(api.counts().dataset_stats, 0);

    click(&mut harness, "Stats");
    harness.step();
    assert!(harness.query_by_label("Live Statistics").is_some());
    click(&mut harness, "Refresh now");
    step_until(&mut harness, 8, |app| !app.loading.stats);
    assert!(api.counts().dataset_stats >= 1);

    harness.set_size(egui::vec2(390.0, 760.0));
    harness.step();
    assert!(harness.query_by_label("Setup").is_some());
    assert!(harness.query_by_label("Work").is_some());
    assert!(harness.query_by_label("Stats").is_some());

    harness.set_size(egui::vec2(1280.0, 820.0));
    harness.step();
    click(&mut harness, "Work");
    harness.step();
    assert!(harness.query_by_label("Next image").is_some());
    assert!(harness.query_by_label("Save").is_some());
    assert!(harness.query_by_label("Submit").is_some());
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
        .with_size(egui::vec2(1180.0, 780.0))
        .with_max_steps(80)
        .build_eframe(|_| base_live_app(api))
}

fn loaded_work_harness(api: Rc<SpyApi>) -> Harness<'static, LabelloApp> {
    let mut harness = live_harness(api);
    step_until(&mut harness, 8, |app| app.datasets.summaries.len() == 1);
    click(&mut harness, "Open");
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
        role: DatasetRole::DataAdmin,
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
    assert!(predicate(harness.state()));
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
    get_image_record: usize,
    get_image_state: usize,
    get_image_preview: usize,
    append_event: usize,
    rebuild_image: usize,
    record_review: usize,
    record_adjudication: usize,
    dataset_stats: usize,
    get_keybindings: usize,
    prelabel_suggestions: usize,
}

struct SpyState {
    metadata: DatasetMetadata,
    states: BTreeMap<ImageId, ImageState>,
    counts: CallCounts,
    next_image: usize,
    events: Vec<EventPayload>,
    fail_next_preview: bool,
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
            roles: vec![
                DatasetRole::DataAdmin,
                DatasetRole::Annotator,
                DatasetRole::Reviewer,
                DatasetRole::Adjudicator,
            ],
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
        let image_id = state
            .metadata
            .images
            .keys()
            .nth(state.next_image % state.metadata.images.len())
            .cloned()
            .unwrap();
        state.next_image += 1;
        ready(Ok(Some(Assignment {
            assignment_id: AssignmentId::generate(),
            image_id,
            task_id: request.task_id,
            assigned_to: UserId::from("admin"),
            kind: request.kind.unwrap_or(AssignmentKind::Annotation),
            status: AssignmentStatus::Active,
            created_at: now(),
            updated_at: now(),
        })))
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
