use eframe::egui;
use labello_domain::{
    AnnotationGeometry, AnnotationId, AnnotationSource, AnnotationType, BoundingBox, ClassId,
    DatasetId, ImageId, ImageRecord, KeybindingSet, LabelClass, PrelabelSuggestion, TaskDefinition,
    TaskId, TutorialContent, UserId,
};

use crate::{
    canvas::CanvasState,
    queue::{ImageQueue, QueuedImage},
    theme,
};

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub user_id: UserId,
    pub dataset_id: DatasetId,
    pub queue_size: usize,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            user_id: UserId::from("demo_user"),
            dataset_id: DatasetId::from("demo"),
            queue_size: 4,
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

pub struct LabelloApp {
    pub(crate) config: AppConfig,
    pub(crate) classes: Vec<LabelClass>,
    pub(crate) tasks: Vec<TaskDefinition>,
    pub(crate) selected_task: usize,
    pub(crate) tool: Tool,
    pub(crate) current: Option<QueuedImage>,
    pub(crate) queue: ImageQueue,
    pub(crate) annotations: Vec<labello_domain::AnnotationVersion>,
    pub(crate) accepted_prelabels: Vec<String>,
    pub(crate) selected_annotation: Option<AnnotationId>,
    pub(crate) keybindings: KeybindingSet,
    pub(crate) canvas: CanvasState,
    pub(crate) save_status: SaveStatus,
    pub(crate) offline: bool,
    pub(crate) review_index: usize,
    pub(crate) show_tutorial: bool,
    pub(crate) theme_applied: bool,
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
        let mut queue = ImageQueue::new(config.queue_size);
        for index in 1..=config.queue_size {
            queue.push_if_room(demo_image(index));
        }
        let current = queue.pop_next();
        Self {
            keybindings: KeybindingSet::defaults_for(config.user_id.clone()),
            config,
            classes,
            tasks,
            selected_task: 0,
            tool: Tool::BoundingBox,
            current,
            queue,
            annotations: Vec::new(),
            accepted_prelabels: Vec::new(),
            selected_annotation: None,
            canvas: CanvasState::default(),
            save_status: SaveStatus::Idle,
            offline: false,
            review_index: 0,
            show_tutorial: false,
            theme_applied: false,
        }
    }

    pub(crate) fn selected_task(&self) -> Option<&TaskDefinition> {
        self.tasks.get(self.selected_task)
    }

    pub(crate) fn next_image(&mut self) {
        self.autosave();
        self.current = self.queue.pop_next();
        self.annotations.clear();
        self.accepted_prelabels.clear();
        self.selected_annotation = None;
        self.replenish_demo_queue();
    }

    pub(crate) fn autosave(&mut self) {
        if self.save_status == SaveStatus::Dirty {
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
        let Some(class_id) = task.class_ids.first() else {
            return;
        };
        let timestamp = labello_domain::now();
        self.annotations.push(labello_domain::AnnotationVersion {
            annotation_id: AnnotationId::generate(),
            version: 1,
            task_id: task.task_id.clone(),
            class_id: class_id.clone(),
            annotation_type: AnnotationType::BoundingBox,
            source: AnnotationSource::Human,
            geometry: AnnotationGeometry::BoundingBox(bbox),
            author_user_id: self.config.user_id.clone(),
            created_at: timestamp,
            updated_at: timestamp,
            deleted: false,
        });
        self.save_status = SaveStatus::Dirty;
    }

    pub(crate) fn accept_prelabel(&mut self, suggestion: &PrelabelSuggestion) {
        if self
            .accepted_prelabels
            .iter()
            .any(|id| id == &suggestion.suggestion_id)
        {
            return;
        }
        let timestamp = labello_domain::now();
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
            author_user_id: self.config.user_id.clone(),
            created_at: timestamp,
            updated_at: timestamp,
            deleted: false,
        });
        self.accepted_prelabels
            .push(suggestion.suggestion_id.clone());
        self.save_status = SaveStatus::Dirty;
    }

    fn delete_selected(&mut self) {
        if let Some(selected) = &self.selected_annotation
            && let Some(annotation) = self
                .annotations
                .iter_mut()
                .find(|annotation| &annotation.annotation_id == selected)
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

impl eframe::App for LabelloApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if !self.theme_applied {
            theme::apply(ui.ctx());
            self.theme_applied = true;
        }
        self.handle_shortcuts(ui.ctx());
        egui::Panel::top("top_bar")
            .frame(theme::top_bar_frame())
            .show(ui, |ui| self.top_bar(ui));
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
        egui::CentralPanel::default()
            .frame(theme::central_frame())
            .show(ui, |ui| self.central(ui));
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
