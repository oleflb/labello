use std::{
    collections::{BTreeSet, VecDeque},
    ops::{Deref, DerefMut},
    rc::Rc,
    sync::mpsc,
    time::Instant,
};

use eframe::egui::{self, TextureHandle};
use labello_client::{DatasetSummary, IngestJob, LabelloApi};
use labello_domain::{
    AdjudicationRecord, AnnotationGeometry, AnnotationId, AnnotationSource, AnnotationType,
    AssignmentKind, BoundingBox, ClassId, DatasetId, DatasetMetadata, DatasetRole, DatasetStats,
    ImageId, ImageRecord, ImageState, KeybindingSet, LabelClass, PrelabelConfigId,
    PrelabelSuggestion, ReviewRecord, TaskDefinition, TaskId, TutorialContent, UserId,
};

use crate::{
    canvas::CanvasState,
    queue::{ImageQueue, QueuedImage},
    theme,
};

pub const IMAGE_QUEUE_SIZE: usize = 8;

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub api_base_url: String,
    pub dev_token: String,
    pub user_id: UserId,
    pub role: DatasetRole,
    pub dataset_id: DatasetId,
    pub queue_size: usize,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            api_base_url: "http://127.0.0.1:8080".to_string(),
            dev_token: "dev-local-token".to_string(),
            user_id: UserId::from("demo_user"),
            role: DatasetRole::DataAdmin,
            dataset_id: DatasetId::from("demo"),
            queue_size: IMAGE_QUEUE_SIZE,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tool {
    BoundingBox,
    Keypoints,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SaveStatus {
    Idle,
    Dirty,
    Saved,
    Syncing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppView {
    Setup,
    Annotate,
    Admin,
    Stats,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QueueMode {
    Annotate,
    Review,
    Adjudicate,
}

#[derive(Debug)]
pub(crate) enum UiMessage {
    DatasetList(Result<Vec<DatasetSummary>, String>),
    DatasetCreated(Result<DatasetMetadata, String>),
    DatasetLoaded(Result<LoadedDataset, String>),
    AdminLoaded(Result<DatasetMetadata, String>),
    AdminSaved(Result<DatasetMetadata, String>),
    ImageLoaded(Result<LoadedImage, String>),
    SaveFinished(Result<ImageState, String>),
    ReviewFinished(Result<(), String>),
    AdjudicationFinished(Result<(), String>),
    IngestJobLoaded(Result<IngestJob, String>),
    StatsLoaded(Result<DatasetStats, String>),
    #[allow(dead_code)]
    FolderUploadProgress(FolderUploadProgress),
    #[allow(dead_code)]
    FolderUploadFinished(Result<String, String>),
}

pub(crate) enum UiCommand {
    DatasetList,
    CreateDataset {
        dataset_id: DatasetId,
        name: String,
        admin_user_id: UserId,
    },
    LoadDataset {
        dataset_id: DatasetId,
        user_id: UserId,
    },
    LoadAdmin {
        dataset_id: DatasetId,
    },
    SaveAdmin {
        metadata: DatasetMetadata,
    },
    Ingest {
        dataset_id: DatasetId,
    },
    PollIngest {
        dataset_id: DatasetId,
        job_id: String,
    },
    Stats {
        dataset_id: DatasetId,
    },
    NextImage {
        dataset_id: DatasetId,
        task_id: TaskId,
        prelabel_config_ids: Vec<PrelabelConfigId>,
        kind: AssignmentKind,
    },
    SaveAnnotations {
        dataset_id: DatasetId,
        image_id: ImageId,
        user_id: UserId,
        task_id: Option<TaskId>,
        annotations: Vec<labello_domain::AnnotationVersion>,
        persisted: BTreeSet<AnnotationId>,
        submit: bool,
    },
    Review {
        dataset_id: DatasetId,
        image_id: ImageId,
        review: ReviewRecord,
    },
    Adjudication {
        dataset_id: DatasetId,
        image_id: ImageId,
        adjudication: AdjudicationRecord,
    },
}

#[derive(Debug)]
pub(crate) struct LoadedDataset {
    pub metadata: DatasetMetadata,
    pub keybindings: KeybindingSet,
}

#[derive(Debug)]
pub(crate) struct LoadedImage {
    pub queued: QueuedImage,
    pub annotations: Vec<labello_domain::AnnotationVersion>,
    pub state: ImageState,
    pub color_image: Option<egui::ColorImage>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FolderUploadProgress {
    pub uploaded_files: u32,
    pub total_files: u32,
    pub current_batch: u32,
    pub message: String,
}

impl FolderUploadProgress {
    pub(crate) fn fraction(&self) -> f32 {
        if self.total_files == 0 {
            0.0
        } else {
            (self.uploaded_files as f32 / self.total_files as f32).clamp(0.0, 1.0)
        }
    }

    pub(crate) fn label(&self) -> String {
        if self.total_files == 0 {
            self.message.clone()
        } else {
            format!(
                "{} of {} files - {}",
                self.uploaded_files, self.total_files, self.message
            )
        }
    }
}

pub(crate) struct RuntimeState {
    pub api: Option<Rc<dyn LabelloApi>>,
    pub tx: mpsc::Sender<UiMessage>,
    pub rx: mpsc::Receiver<UiMessage>,
    pub commands: VecDeque<UiCommand>,
    pub error: Option<String>,
    pub notice: Option<String>,
}

impl RuntimeState {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            api: None,
            tx,
            rx,
            commands: VecDeque::new(),
            error: None,
            notice: None,
        }
    }
}

#[derive(Default)]
pub(crate) struct LoadingState {
    pub datasets: bool,
    pub dataset: bool,
    pub admin: bool,
    pub image: bool,
    pub saving: bool,
    pub ingesting: bool,
    pub ingest_polling: bool,
    pub ingest_job_id: Option<String>,
    pub last_ingest_poll: Option<Instant>,
    pub uploading: bool,
    pub upload_progress: Option<FolderUploadProgress>,
    pub stats: bool,
}

pub(crate) struct SetupState {
    pub create_dataset_id: String,
    pub create_dataset_name: String,
    pub started: bool,
}

pub(crate) struct DatasetState {
    pub summaries: Vec<DatasetSummary>,
    pub metadata: Option<DatasetMetadata>,
    pub admin_config: Option<DatasetMetadata>,
    pub stats: DatasetStats,
    pub last_stats_request: Option<Instant>,
}

impl DatasetState {
    fn new() -> Self {
        Self {
            summaries: Vec::new(),
            metadata: None,
            admin_config: None,
            stats: DatasetStats::default(),
            last_stats_request: None,
        }
    }
}

pub struct WorkState {
    pub(crate) queue_mode: QueueMode,
    pub(crate) classes: Vec<LabelClass>,
    pub(crate) tasks: Vec<TaskDefinition>,
    pub(crate) selected_task: usize,
    pub(crate) selected_class_id: Option<ClassId>,
    pub(crate) tool: Tool,
    pub(crate) current: Option<QueuedImage>,
    pub(crate) current_state: Option<ImageState>,
    pub(crate) current_texture: Option<TextureHandle>,
    pub(crate) queue: ImageQueue,
    pub(crate) annotations: Vec<labello_domain::AnnotationVersion>,
    pub(crate) persisted_annotations: BTreeSet<AnnotationId>,
    pub(crate) accepted_prelabels: Vec<String>,
    pub(crate) selected_annotation: Option<AnnotationId>,
    pub(crate) keybindings: KeybindingSet,
    pub(crate) canvas: CanvasState,
    pub(crate) save_status: SaveStatus,
    pub(crate) offline: bool,
    pub(crate) review_index: usize,
    pub(crate) show_tutorial: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkflowChoice {
    pub task_index: usize,
    pub task_name: String,
    pub class_id: ClassId,
    pub class_name: String,
    pub annotation_type: AnnotationType,
}

impl WorkflowChoice {
    pub(crate) fn label(&self) -> String {
        format!(
            "{} {}",
            self.class_name,
            annotation_type_label(&self.annotation_type)
        )
    }
}

pub struct LabelloApp {
    pub(crate) config: AppConfig,
    pub(crate) runtime: RuntimeState,
    pub(crate) loading: LoadingState,
    pub(crate) setup: SetupState,
    pub(crate) datasets: DatasetState,
    pub(crate) work: WorkState,
    pub(crate) view: AppView,
    pub(crate) theme_applied: bool,
}

impl Deref for LabelloApp {
    type Target = WorkState;

    fn deref(&self) -> &Self::Target {
        &self.work
    }
}

impl DerefMut for LabelloApp {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.work
    }
}

impl Default for LabelloApp {
    fn default() -> Self {
        Self::demo(AppConfig::default())
    }
}

impl LabelloApp {
    pub fn demo(config: AppConfig) -> Self {
        let classes = vec![LabelClass {
            class_id: ClassId::from("person"),
            name: "Person".to_string(),
            color: "#5eead4".to_string(),
            description: Some("Visible people in the image".to_string()),
        }];
        let tasks = vec![TaskDefinition {
            task_id: TaskId::from("bounding_box:person"),
            name: "Person bounding boxes".to_string(),
            annotation_type: AnnotationType::BoundingBox,
            class_ids: vec![ClassId::from("person")],
            instructions: TutorialContent {
                title: "Label every visible person".to_string(),
                example_text: "Draw tight boxes around each visible person. Include partially visible people, but skip reflections and posters.".to_string(),
                example_images: vec!["tutorial/person-box-example.png".to_string()],
            },
            skeleton: None,
            review: labello_domain::ReviewConfig::default(),
            prelabel_config_ids: vec![],
            enabled: true,
        }];
        let mut queue = ImageQueue::new(IMAGE_QUEUE_SIZE);
        for index in 1..=IMAGE_QUEUE_SIZE {
            queue.push_if_room(demo_image(index));
        }
        let current = queue.pop_next();
        let setup = SetupState {
            create_dataset_id: config.dataset_id.to_string(),
            create_dataset_name: "Demo Dataset".to_string(),
            started: true,
        };
        let work = WorkState {
            queue_mode: QueueMode::Annotate,
            classes,
            tasks,
            selected_task: 0,
            selected_class_id: Some(ClassId::from("person")),
            tool: Tool::BoundingBox,
            current,
            current_state: None,
            current_texture: None,
            queue,
            annotations: Vec::new(),
            persisted_annotations: BTreeSet::new(),
            accepted_prelabels: Vec::new(),
            selected_annotation: None,
            keybindings: KeybindingSet::defaults_for(config.user_id.clone()),
            canvas: CanvasState::default(),
            save_status: SaveStatus::Idle,
            offline: false,
            review_index: 0,
            show_tutorial: false,
        };
        Self {
            runtime: RuntimeState::new(),
            loading: LoadingState::default(),
            setup,
            datasets: DatasetState::new(),
            work,
            view: AppView::Annotate,
            config,
            theme_applied: false,
        }
    }

    pub fn live_http(config: AppConfig) -> Self {
        let mut app = Self::demo(config);
        app.view = AppView::Setup;
        app.setup.started = false;
        app.current = None;
        app.queue.clear();
        app.rebuild_http_api();
        app
    }

    pub(crate) fn selected_task(&self) -> Option<&TaskDefinition> {
        self.tasks
            .get(self.selected_task)
            .filter(|task| task.enabled && !task.class_ids.is_empty())
    }

    pub(crate) fn selected_class_id(&self) -> Option<&ClassId> {
        let task = self.selected_task()?;
        self.selected_class_id
            .as_ref()
            .filter(|class_id| task.class_ids.contains(class_id))
    }

    pub(crate) fn workflow_choices(&self) -> Vec<WorkflowChoice> {
        let mut choices = Vec::new();
        for (task_index, task) in self.tasks.iter().enumerate() {
            if !task.enabled {
                continue;
            }
            for class_id in &task.class_ids {
                choices.push(WorkflowChoice {
                    task_index,
                    task_name: task.name.clone(),
                    class_id: class_id.clone(),
                    class_name: self.class_name(class_id),
                    annotation_type: task.annotation_type.clone(),
                });
            }
        }
        choices
    }

    pub(crate) fn selected_workflow(&self) -> Option<WorkflowChoice> {
        let task = self.selected_task()?;
        let class_id = self.selected_class_id()?;
        Some(WorkflowChoice {
            task_index: self.selected_task,
            task_name: task.name.clone(),
            class_id: class_id.clone(),
            class_name: self.class_name(class_id),
            annotation_type: task.annotation_type.clone(),
        })
    }

    pub(crate) fn select_workflow(&mut self, task_index: usize, class_id: ClassId) -> bool {
        let Some(task) = self.tasks.get(task_index) else {
            return false;
        };
        if !task.enabled || !task.class_ids.contains(&class_id) {
            return false;
        }
        if self.selected_task == task_index && self.selected_class_id.as_ref() == Some(&class_id) {
            return false;
        }
        let annotation_type = task.annotation_type.clone();
        self.selected_task = task_index;
        self.selected_class_id = Some(class_id);
        self.tool = tool_for_annotation_type(&annotation_type);
        true
    }

    pub(crate) fn ensure_valid_task_selection(&mut self) -> bool {
        if self.selected_class_id().is_some() {
            return true;
        }
        let Some((index, class_id, annotation_type)) = self
            .tasks
            .iter()
            .enumerate()
            .find(|(_, task)| task.enabled && !task.class_ids.is_empty())
            .map(|(index, task)| {
                (
                    index,
                    task.class_ids[0].clone(),
                    task.annotation_type.clone(),
                )
            })
        else {
            return false;
        };
        self.selected_task = index;
        self.selected_class_id = Some(class_id);
        self.tool = tool_for_annotation_type(&annotation_type);
        true
    }

    pub(crate) fn sync_work_config(&mut self, metadata: DatasetMetadata) {
        self.classes = metadata.label_classes.clone();
        self.tasks = metadata.tasks.clone();
        self.datasets.metadata = Some(metadata);
        self.ensure_valid_task_selection();
    }

    pub(crate) fn annotation_matches_selected_workflow(
        &self,
        annotation: &labello_domain::AnnotationVersion,
    ) -> bool {
        let Some(task) = self.selected_task() else {
            return false;
        };
        let Some(class_id) = self.selected_class_id() else {
            return false;
        };
        annotation.task_id == task.task_id && &annotation.class_id == class_id
    }

    fn class_name(&self, class_id: &ClassId) -> String {
        self.classes
            .iter()
            .find(|class| &class.class_id == class_id)
            .map(|class| class.name.clone())
            .unwrap_or_else(|| class_id.to_string())
    }

    pub(crate) fn next_image(&mut self) {
        self.autosave();
        self.current_texture = None;
        self.current_state = None;
        self.current = self.queue.pop_next();
        self.annotations.clear();
        self.persisted_annotations.clear();
        self.accepted_prelabels.clear();
        self.selected_annotation = None;
        if self.runtime.api.is_some() {
            self.request_next_image();
        } else {
            self.replenish_demo_queue();
        }
    }

    pub(crate) fn autosave(&mut self) {
        if self.save_status == SaveStatus::Dirty {
            if self.runtime.api.is_some() {
                self.request_save(false);
                return;
            }
            self.save_status = if self.offline {
                SaveStatus::Syncing
            } else {
                SaveStatus::Saved
            };
        }
    }

    pub(crate) fn replenish_demo_queue(&mut self) {
        let next_index = self.queue.len() + 1;
        while self.queue.len() < self.queue.queue_size() {
            let image_number = next_index + self.queue.len();
            self.queue.push_if_room(demo_image(image_number));
        }
        self.queue.set_loading(false);
    }

    pub(crate) fn create_bbox(&mut self, bbox: BoundingBox) {
        let Some(task) = self.selected_task() else {
            return;
        };
        let Some(class_id) = self.selected_class_id() else {
            return;
        };
        let task_id = task.task_id.clone();
        let class_id = class_id.clone();
        let user_id = self.config.user_id.clone();
        let timestamp = labello_domain::now();
        self.annotations.push(labello_domain::AnnotationVersion {
            annotation_id: AnnotationId::generate(),
            version: 1,
            task_id,
            class_id,
            annotation_type: AnnotationType::BoundingBox,
            source: AnnotationSource::Human,
            geometry: AnnotationGeometry::BoundingBox(bbox),
            author_user_id: user_id,
            created_at: timestamp,
            updated_at: timestamp,
            deleted: false,
        });
        self.save_status = SaveStatus::Dirty;
    }

    pub(crate) fn accept_prelabel(&mut self, suggestion: &PrelabelSuggestion) {
        let Some(task) = self.selected_task() else {
            return;
        };
        let Some(class_id) = self.selected_class_id() else {
            return;
        };
        if suggestion.task_id != task.task_id || &suggestion.class_id != class_id {
            return;
        }
        if self
            .accepted_prelabels
            .iter()
            .any(|id| id == &suggestion.suggestion_id)
        {
            return;
        }
        let timestamp = labello_domain::now();
        let user_id = self.config.user_id.clone();
        self.annotations.push(labello_domain::AnnotationVersion {
            annotation_id: AnnotationId::generate(),
            version: 1,
            task_id: suggestion.task_id.clone(),
            class_id: suggestion.class_id.clone(),
            annotation_type: match suggestion.geometry {
                AnnotationGeometry::BoundingBox(_) => AnnotationType::BoundingBox,
                AnnotationGeometry::Skeleton(_) => AnnotationType::Skeleton,
            },
            source: AnnotationSource::PrelabelSuggestion {
                config_id: suggestion.config_id.clone(),
                model_id: "browser-local-or-server".to_string(),
                confidence: suggestion.confidence,
            },
            geometry: suggestion.geometry.clone(),
            author_user_id: user_id,
            created_at: timestamp,
            updated_at: timestamp,
            deleted: false,
        });
        self.accepted_prelabels
            .push(suggestion.suggestion_id.clone());
        self.save_status = SaveStatus::Dirty;
    }

    fn delete_selected(&mut self) {
        if let Some(selected) = self.selected_annotation.clone()
            && let Some(annotation) = self
                .annotations
                .iter_mut()
                .find(|annotation| annotation.annotation_id == selected)
        {
            annotation.deleted = true;
            annotation.updated_at = labello_domain::now();
            self.save_status = SaveStatus::Dirty;
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| input.key_pressed(egui::Key::ArrowRight)) {
            self.next_image();
        }
        if ctx.input(|input| input.key_pressed(egui::Key::S) && input.modifiers.ctrl) {
            self.autosave();
        }
        if ctx.input(|input| input.key_pressed(egui::Key::Delete)) {
            self.delete_selected();
        }
        if ctx.input(|input| input.key_pressed(egui::Key::Y)) {
            self.review_index = self.review_index.saturating_add(1);
        }
        if ctx.input(|input| input.key_pressed(egui::Key::N)) {
            self.save_status = SaveStatus::Dirty;
            self.review_index = self.review_index.saturating_add(1);
        }
    }
}

pub(crate) fn tool_for_annotation_type(annotation_type: &AnnotationType) -> Tool {
    match annotation_type {
        AnnotationType::BoundingBox => Tool::BoundingBox,
        AnnotationType::Skeleton => Tool::Keypoints,
    }
}

pub(crate) fn annotation_type_label(annotation_type: &AnnotationType) -> &'static str {
    match annotation_type {
        AnnotationType::BoundingBox => "bounding box",
        AnnotationType::Skeleton => "skeleton",
    }
}

impl eframe::App for LabelloApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if !self.theme_applied {
            theme::apply(ui.ctx());
            self.theme_applied = true;
        }
        self.process_messages(ui.ctx());
        self.start_setup_load();
        self.refresh_stats_if_due();
        self.refresh_ingest_if_due();
        self.handle_shortcuts(ui.ctx());
        egui::Panel::top("top_bar")
            .exact_size(56.0)
            .frame(theme::top_bar_frame())
            .show(ui, |ui| self.top_bar(ui));
        if self.view == AppView::Annotate {
            egui::Panel::left("task_panel")
                .resizable(false)
                .default_size(280.0)
                .frame(theme::side_frame())
                .show(ui, |ui| self.task_panel(ui));
            egui::Panel::right("review_panel")
                .resizable(false)
                .default_size(320.0)
                .frame(theme::side_frame())
                .show(ui, |ui| self.right_panel(ui));
        }
        egui::CentralPanel::default()
            .frame(theme::central_frame())
            .show(ui, |ui| self.central(ui));
        self.start_next_command();
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(150));
    }
}

fn demo_image(index: usize) -> QueuedImage {
    let image = ImageRecord {
        image_id: ImageId::from(format!("img_demo_{index}")),
        blake3: format!("demo_hash_{index}"),
        canonical_path: format!("images/demo_{index}.jpg"),
        known_paths: vec![format!("images/demo_{index}.jpg")],
        duplicate_paths: vec![],
        file_name: format!("demo_{index}.jpg"),
        byte_size: 1024,
        width: 1280,
        height: 800,
        media_type: "image/jpeg".to_string(),
    };
    let prelabels = vec![PrelabelSuggestion {
        suggestion_id: format!("pre_demo_{index}"),
        config_id: labello_domain::PrelabelConfigId::from("demo-prelabel"),
        task_id: TaskId::from("bounding_box:person"),
        class_id: ClassId::from("person"),
        confidence: 0.82,
        geometry: AnnotationGeometry::BoundingBox(BoundingBox {
            x: 0.32,
            y: 0.22,
            width: 0.2,
            height: 0.46,
        }),
    }];
    QueuedImage { image, prelabels }
}
