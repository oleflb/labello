use std::collections::BTreeSet;

use eframe::egui::{self, RichText};
use labello_domain::{
    AgreementMetric, AgreementThreshold, AnnotationType, BrowserAcceleration, ClassId,
    DatasetMetadata, DatasetRole, DatasetRoleAssignment, ImbalanceConfig, KeypointSpec, LabelClass,
    ModelSpec, OutputProcessing, PrelabelConfig, PrelabelConfigId, PrelabelExecution, ReviewConfig,
    ReviewWorkflow, SkeletonEdge, SkeletonSpec, TaskDefinition, TaskId, TutorialContent, UserId,
};

use crate::{app::LabelloApp, theme};

impl LabelloApp {
    pub(crate) fn admin_view(&mut self, ui: &mut egui::Ui) {
        let admin_dirty = self.datasets.admin_config != self.datasets.admin_baseline;
        let mut discard_changes = false;
        ui.horizontal(|ui| {
            ui.heading("Dataset Admin");
            if self.loading.admin {
                ui.spinner();
            }
            if ui
                .add_enabled(!admin_dirty, egui::Button::new("Reload Admin Config"))
                .on_hover_text(if admin_dirty {
                    "Discard or save staged changes before reloading."
                } else {
                    "Reload configuration from the server."
                })
                .clicked()
            {
                self.request_admin_dataset();
            }
            if admin_dirty && ui.button("Discard staged changes").clicked() {
                discard_changes = true;
            }
        });
        if discard_changes {
            self.datasets.admin_config = self.datasets.admin_baseline.clone();
            self.runtime.notice = Some("Staged admin changes discarded".to_string());
        }
        let current_user = self.config.user_id.clone();
        let ingesting_now = self.loading.ingesting;
        let uploading_now = self.loading.uploading;
        let upload_progress = self.loading.upload_progress.clone();
        let Some(config) = self.datasets.admin_config.as_mut() else {
            ui.label(RichText::new("Admin config is not loaded.").color(theme::MUTED));
            if ui.button("Load admin config").clicked() {
                self.request_admin_dataset();
            }
            return;
        };

        let mut save = false;
        let mut ingest = false;
        let mut upload_folder = false;
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.label(
                RichText::new(
                    "Edits are staged here. Review the validation summary before saving them.",
                )
                .color(theme::MUTED),
            );
            theme::card_frame().show(ui, |ui| {
                ui.heading("Dataset Details");
                labeled_text(ui, "Dataset name", &mut config.name)
                    .on_hover_text("Human-readable name stored in labello.dataset.toml.");
                show_issues(ui, &dataset_name_issues(&config.name));
            });

            theme::card_frame().show(ui, |ui| {
                ui.heading("Image Roots");
                edit_string_list(
                    ui,
                    &mut config.image_roots,
                    "Root path",
                    "Add image root",
                    "images",
                );
                ui.horizontal(|ui| {
                    if ui
                        .button("Pick folder and upload")
                        .on_hover_text("Open a browser folder picker, upload files to a new dataset-relative root, then ingest them.")
                        .clicked()
                    {
                        upload_folder = true;
                    }
                    if uploading_now {
                        ui.spinner();
                    }
                });
                if let Some(progress) = upload_progress.as_ref() {
                    ui.add(
                        egui::ProgressBar::new(progress.fraction())
                            .desired_width(ui.available_width().min(460.0))
                            .text(progress.label()),
                    );
                    if progress.current_batch > 0 {
                        ui.small(format!("Batch {}", progress.current_batch));
                    }
                }
                ui.small("Paths are relative to the dataset root and may be edited in labello.dataset.toml.");
                show_issues(ui, &image_root_issues(&config.image_roots));
            });

            edit_quick_workflows(ui, config);
            edit_labels(ui, &mut config.label_classes, &mut config.tasks);
            edit_tasks(
                ui,
                &mut config.tasks,
                &config.label_classes,
                &config.prelabel_configs,
            );
            edit_prelabels(ui, &mut config.prelabel_configs, &mut config.tasks);
            edit_imbalance(ui, &mut config.imbalance);
            edit_roles(ui, &mut config.role_assignments, &config.dataset_id, &current_user);

            let issues = config_issues(config, &current_user);
            theme::card_frame().show(ui, |ui| {
                ui.heading("Validation Summary");
                if issues.is_empty() {
                    ui.label(RichText::new("Configuration is ready to save.").color(theme::TEAL));
                } else {
                    ui.label(
                        RichText::new(format!(
                            "Fix {} configuration error(s) before saving:",
                            issues.len()
                        ))
                        .color(theme::RED)
                        .strong(),
                    );
                    show_issues(ui, &issues);
                }
            });

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        issues.is_empty() && admin_dirty && !self.loading.admin,
                        egui::Button::new("Save Admin Config"),
                    )
                    .on_hover_text(if !admin_dirty {
                        "No staged changes to save."
                    } else if issues.is_empty() {
                        "Persist the staged settings to labello.dataset.toml."
                    } else {
                        "Fix the errors in the validation summary before saving."
                    })
                    .clicked()
                {
                    save = issues.is_empty();
                }
                if ui
                    .add_enabled(!ingesting_now, egui::Button::new("Run Ingest"))
                    .on_hover_text("Scan configured image roots and update the dataset image index.")
                    .clicked()
                {
                    ingest = true;
                }
                if ingesting_now {
                    ui.spinner();
                    ui.small("Ingest running...");
                }
            });
        });
        if save {
            self.request_admin_save();
        }
        if ingest {
            self.request_ingest();
        }
        if upload_folder {
            self.request_folder_upload();
        }
    }

    pub(crate) fn stats_view(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Live Statistics");
            if self.loading.stats {
                ui.spinner();
            }
            if ui
                .button("Refresh now")
                .on_hover_text("Refresh statistics immediately. They also refresh automatically.")
                .clicked()
            {
                self.request_stats();
            }
        });
        ui.add_space(8.0);
        let metrics = [
            ("Images", self.datasets.stats.total_images),
            ("Completed", self.datasets.stats.completed_tasks),
            ("Pending", self.datasets.stats.pending_tasks),
            ("Reviewed", self.datasets.stats.reviewed_tasks),
            ("Unreviewed", self.datasets.stats.unreviewed_tasks),
        ];
        let column_count = if ui.available_width() < 520.0 {
            1
        } else if ui.available_width() < 900.0 {
            2
        } else {
            5
        };
        for row in metrics.chunks(column_count) {
            ui.columns(column_count, |columns| {
                for (column, (label, value)) in columns.iter_mut().zip(row) {
                    metric(column, label, value.to_string());
                }
            });
        }
        ui.add_space(12.0);
        theme::card_frame().show(ui, |ui| {
            ui.heading("Per Task");
            for (task_id, stats) in &self.datasets.stats.per_task {
                ui.label(format!(
                    "{task_id}: {} completed, {} pending, {} reviewed, {} unreviewed",
                    stats.completed, stats.pending, stats.reviewed, stats.unreviewed
                ));
            }
        });
        theme::card_frame().show(ui, |ui| {
            ui.heading("Per Class");
            for (class_id, stats) in &self.datasets.stats.per_class {
                ui.label(format!(
                    "{class_id}: {} annotations, {} completed tasks",
                    stats.annotations, stats.completed_tasks
                ));
            }
        });
        theme::card_frame().show(ui, |ui| {
            ui.heading("Throughput");
            if self.datasets.stats.throughput.is_empty() {
                ui.label(RichText::new("No completed activity yet.").color(theme::MUTED));
            }
            for point in self.datasets.stats.throughput.iter().rev().take(14).rev() {
                ui.label(format!(
                    "{}: {} annotations, {} reviews",
                    point.day, point.annotations, point.reviews
                ));
            }
        });
    }
}

fn edit_quick_workflows(ui: &mut egui::Ui, config: &mut DatasetMetadata) {
    theme::card_frame().show(ui, |ui| {
        ui.heading("Class Workflows");
        ui.label(
            RichText::new("Fast path: create a class and its worker-visible task together.")
                .color(theme::MUTED),
        );
        ui.horizontal(|ui| {
            if ui.button("Add bounding box class workflow").clicked() {
                add_class_workflow(config, AnnotationType::BoundingBox);
            }
            if ui.button("Add skeleton class workflow").clicked() {
                add_class_workflow(config, AnnotationType::Skeleton);
            }
        });
        ui.add_space(8.0);
        let labels = config.label_classes.clone();
        if labels.is_empty() {
            ui.small("No classes yet. Use one of the buttons above to create the first workflow.");
        }
        for label in labels {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&label.name).strong());
                ui.small(format!("{}", label.class_id));
                let annotation_type = AnnotationType::BoundingBox;
                let exists = has_task_for_class(config, &label.class_id, &annotation_type);
                if ui
                    .add_enabled(!exists, egui::Button::new("Add bounding_box task"))
                    .clicked()
                {
                    add_task_for_class(config, &label, annotation_type);
                }
                let annotation_type = AnnotationType::Skeleton;
                let exists = has_task_for_class(config, &label.class_id, &annotation_type);
                if ui
                    .add_enabled(!exists, egui::Button::new("Add skeleton task"))
                    .clicked()
                {
                    add_task_for_class(config, &label, annotation_type);
                }
            });
        }
    });
}

fn add_class_workflow(config: &mut DatasetMetadata, annotation_type: AnnotationType) {
    let index = config.label_classes.len() + 1;
    let class = LabelClass {
        class_id: ClassId::from(next_class_id(config)),
        name: if index == 1 {
            "Object".to_string()
        } else {
            format!("Object {index}")
        },
        color: default_class_color(index),
        description: None,
    };
    config.label_classes.push(class.clone());
    add_task_for_class(config, &class, annotation_type);
}

fn add_task_for_class(
    config: &mut DatasetMetadata,
    class: &LabelClass,
    annotation_type: AnnotationType,
) {
    if has_task_for_class(config, &class.class_id, &annotation_type) {
        return;
    }
    config
        .tasks
        .push(workflow_task_for_class(class, annotation_type));
}

fn workflow_task_for_class(class: &LabelClass, annotation_type: AnnotationType) -> TaskDefinition {
    let task_id = match annotation_type {
        AnnotationType::BoundingBox => format!("bounding_box:{}", class.class_id),
        AnnotationType::Skeleton => format!("skeleton:{}", class.class_id),
    };
    let name = match annotation_type {
        AnnotationType::BoundingBox => format!("{} bounding boxes", class.name),
        AnnotationType::Skeleton => format!("{} skeletons", class.name),
    };
    let skeleton = (annotation_type == AnnotationType::Skeleton).then(starter_skeleton_spec);
    TaskDefinition {
        task_id: TaskId::from(task_id),
        name,
        annotation_type,
        class_ids: vec![class.class_id.clone()],
        instructions: TutorialContent {
            title: "Label every visible object".to_string(),
            example_text: "Annotate every visible instance of the configured class.".to_string(),
            example_images: Vec::new(),
        },
        skeleton,
        review: ReviewConfig::default(),
        prelabel_config_ids: Vec::new(),
        enabled: true,
    }
}

fn starter_skeleton_spec() -> SkeletonSpec {
    SkeletonSpec {
        keypoints: vec![KeypointSpec {
            name: "keypoint_1".to_string(),
            required: true,
        }],
        edges: Vec::new(),
        allow_hidden: false,
        allow_absent: false,
    }
}

fn has_task_for_class(
    config: &DatasetMetadata,
    class_id: &ClassId,
    annotation_type: &AnnotationType,
) -> bool {
    config.tasks.iter().any(|task| {
        task.enabled
            && &task.annotation_type == annotation_type
            && task.class_ids.contains(class_id)
    })
}

fn next_class_id(config: &DatasetMetadata) -> String {
    for index in 1.. {
        let candidate = if index == 1 {
            "object".to_string()
        } else {
            format!("object_{index}")
        };
        if !config
            .label_classes
            .iter()
            .any(|class| class.class_id.as_str() == candidate)
        {
            return candidate;
        }
    }
    unreachable!()
}

fn default_class_color(index: usize) -> String {
    const COLORS: [&str; 6] = [
        "#5eead4", "#60a5fa", "#fbbf24", "#f472b6", "#a78bfa", "#34d399",
    ];
    COLORS[(index - 1) % COLORS.len()].to_string()
}

fn edit_string_list(
    ui: &mut egui::Ui,
    values: &mut Vec<String>,
    label: &str,
    button: &str,
    default: &str,
) {
    let mut remove = None;
    for (index, value) in values.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.label(label);
            ui.text_edit_singleline(value)
                .on_hover_text("Dataset-relative path under the dataset root.");
            if destructive_button(ui, "Remove") {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        values.remove(index);
    }
    if ui
        .button(button)
        .on_hover_text("Add another entry.")
        .clicked()
    {
        values.push(default.to_string());
    }
}

fn edit_labels(ui: &mut egui::Ui, labels: &mut Vec<LabelClass>, tasks: &mut [TaskDefinition]) {
    theme::card_frame().show(ui, |ui| {
        ui.heading("Classes");
        ui.label(
            RichText::new("Classes define the objects annotators can label.").color(theme::MUTED),
        );
        let mut remove = None;
        for (index, label) in labels.iter_mut().enumerate() {
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut label.name)
                    .on_hover_text("Display name shown to annotators.");
                ui.label("ID");
                let mut class_id = label.class_id.to_string();
                if ui
                    .text_edit_singleline(&mut class_id)
                    .on_hover_text("Stable class id used by annotations and tasks. Existing task links update when this changes.")
                    .changed()
                {
                    let previous = label.class_id.clone();
                    let updated = ClassId::from(class_id);
                    label.class_id = updated.clone();
                    for task in tasks.iter_mut() {
                        for class_id in &mut task.class_ids {
                            if class_id == &previous {
                                *class_id = updated.clone();
                            }
                        }
                    }
                }
                ui.label("Color");
                ui.text_edit_singleline(&mut label.color)
                    .on_hover_text("Class color as a hex value, for example #5eead4.");
                if destructive_button(ui, "Remove class") {
                    remove = Some(index);
                }
            });
            ui.horizontal(|ui| {
                ui.label("Description");
                let mut description = label.description.clone().unwrap_or_default();
                if ui
                    .text_edit_singleline(&mut description)
                    .on_hover_text("Optional guidance about what belongs in this class.")
                    .changed()
                {
                    label.description = if description.trim().is_empty() {
                        None
                    } else {
                        Some(description)
                    };
                }
            });
        }
        if let Some(index) = remove {
            let removed = labels.remove(index).class_id;
            for task in tasks.iter_mut() {
                task.class_ids.retain(|class_id| class_id != &removed);
            }
        }
        if ui.button("Add class").clicked() {
            labels.push(LabelClass {
                class_id: ClassId::from(next_numbered_id(
                    "class",
                    labels.iter().map(|label| label.class_id.as_str()),
                )),
                name: "New class".to_string(),
                color: default_class_color(labels.len() + 1),
                description: None,
            });
        }
        show_issues(ui, &class_issues(labels));
    });
}

fn edit_tasks(
    ui: &mut egui::Ui,
    tasks: &mut Vec<TaskDefinition>,
    labels: &[LabelClass],
    prelabels: &[PrelabelConfig],
) {
    theme::card_frame().show(ui, |ui| {
        ui.heading("Advanced Workflows");
        ui.label(
            RichText::new("Most datasets only need the Class Workflows card above.")
                .color(theme::MUTED),
        );
        let mut remove = None;
        for (index, task) in tasks.iter_mut().enumerate() {
            normalize_task_annotation(task);
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Task ID");
                let mut task_id = task.task_id.to_string();
                if ui
                    .text_edit_singleline(&mut task_id)
                    .on_hover_text("Stable task id used by assignments and event logs.")
                    .changed()
                {
                    task.task_id = TaskId::from(task_id);
                }
                ui.label("Name");
                ui.text_edit_singleline(&mut task.name)
                    .on_hover_text("Task name shown in the work panel.");
                ui.checkbox(&mut task.enabled, "Enabled");
                if destructive_button(ui, "Remove workflow") {
                    remove = Some(index);
                }
            });
            ui.horizontal(|ui| {
                ui.label("Annotation type");
                let mut annotation_type = task.annotation_type.clone();
                egui::ComboBox::from_id_salt(format!("task-type-{index}"))
                    .selected_text(annotation_type.to_string())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut annotation_type,
                            AnnotationType::BoundingBox,
                            "bounding_box",
                        );
                        ui.selectable_value(
                            &mut annotation_type,
                            AnnotationType::Skeleton,
                            "skeleton",
                        );
                    });
                if annotation_type != task.annotation_type {
                    set_task_annotation_type(task, annotation_type);
                }
            });
            if task.annotation_type == AnnotationType::Skeleton
                && let Some(skeleton) = task.skeleton.as_mut()
            {
                edit_skeleton(ui, index, skeleton);
            }
            ui.horizontal(|ui| {
                ui.label("Instruction title");
                ui.text_edit_singleline(&mut task.instructions.title)
                    .on_hover_text("Tutorial/instruction title.");
            });
            ui.label("Tutorial instructions");
            ui.text_edit_multiline(&mut task.instructions.example_text)
                .on_hover_text("Instructions annotators see in the tutorial panel.");
            ui.label("Tutorial example images");
            edit_string_list(
                ui,
                &mut task.instructions.example_images,
                "Image path",
                "Add example image",
                "tutorial/example.png",
            );
            ui.label("Allowed classes");
            if labels.is_empty() {
                ui.label(
                    RichText::new("Add a class before configuring this task.").color(theme::RED),
                );
            }
            for label in labels {
                let mut enabled = task.class_ids.contains(&label.class_id);
                if ui
                    .checkbox(&mut enabled, format!("{} ({})", label.name, label.class_id))
                    .changed()
                {
                    if enabled {
                        task.class_ids.push(label.class_id.clone());
                    } else {
                        task.class_ids
                            .retain(|class_id| class_id != &label.class_id);
                    }
                }
            }
            ui.label("Prelabel sources");
            if prelabels.is_empty() {
                ui.small("No prelabel sources configured.");
            }
            for prelabel in prelabels {
                let mut enabled = task.prelabel_config_ids.contains(&prelabel.config_id);
                if ui
                    .checkbox(
                        &mut enabled,
                        format!("{} ({})", prelabel.name, prelabel.config_id),
                    )
                    .changed()
                {
                    if enabled {
                        task.prelabel_config_ids.push(prelabel.config_id.clone());
                    } else {
                        task.prelabel_config_ids
                            .retain(|config_id| config_id != &prelabel.config_id);
                    }
                }
            }
            edit_review(ui, index, task);
        }
        if let Some(index) = remove {
            tasks.remove(index);
        }
        if ui.button("Add workflow").clicked() {
            let class_ids = labels
                .first()
                .map(|label| vec![label.class_id.clone()])
                .unwrap_or_default();
            tasks.push(TaskDefinition {
                task_id: TaskId::from(next_numbered_id(
                    "task",
                    tasks.iter().map(|task| task.task_id.as_str()),
                )),
                name: "New task".to_string(),
                annotation_type: AnnotationType::BoundingBox,
                class_ids,
                instructions: TutorialContent {
                    title: "Instructions".to_string(),
                    example_text: "Describe what annotators should label.".to_string(),
                    example_images: Vec::new(),
                },
                skeleton: None,
                review: ReviewConfig::default(),
                prelabel_config_ids: Vec::new(),
                enabled: true,
            });
        }
        show_issues(ui, &task_issues(tasks, labels, prelabels));
    });
}

fn normalize_task_annotation(task: &mut TaskDefinition) {
    match task.annotation_type {
        AnnotationType::BoundingBox => task.skeleton = None,
        AnnotationType::Skeleton => {
            task.skeleton.get_or_insert_with(starter_skeleton_spec);
        }
    }
}

fn set_task_annotation_type(task: &mut TaskDefinition, annotation_type: AnnotationType) {
    task.annotation_type = annotation_type;
    normalize_task_annotation(task);
    if let Some(agreement) = task.review.agreement_threshold.as_mut() {
        agreement.metric = match task.annotation_type {
            AnnotationType::BoundingBox => AgreementMetric::Iou,
            AnnotationType::Skeleton => AgreementMetric::KeypointMeanDistance,
        };
    }
}

fn edit_skeleton(ui: &mut egui::Ui, task_index: usize, skeleton: &mut SkeletonSpec) {
    ui.collapsing("Skeleton configuration", |ui| {
        ui.horizontal(|ui| {
            ui.checkbox(&mut skeleton.allow_hidden, "Allow hidden keypoints")
                .on_hover_text("Annotators may mark a keypoint as hidden behind another object.");
            ui.checkbox(&mut skeleton.allow_absent, "Allow absent keypoints")
                .on_hover_text("Annotators may mark a keypoint as outside the image or absent.");
        });

        ui.label(RichText::new("Keypoints").strong());
        let mut remove_keypoint = None;
        let mut renames = Vec::new();
        for (keypoint_index, keypoint) in skeleton.keypoints.iter_mut().enumerate() {
            let previous_name = keypoint.name.clone();
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut keypoint.name)
                    .on_hover_text("Unique keypoint name used by skeleton edges.");
                ui.checkbox(&mut keypoint.required, "Required");
                if destructive_button(ui, "Remove keypoint") {
                    remove_keypoint = Some(keypoint_index);
                }
            });
            if keypoint.name != previous_name {
                renames.push((previous_name, keypoint.name.clone()));
            }
        }
        for (previous, updated) in renames {
            for edge in &mut skeleton.edges {
                if edge.from == previous {
                    edge.from = updated.clone();
                }
                if edge.to == previous {
                    edge.to = updated.clone();
                }
            }
        }
        if let Some(index) = remove_keypoint {
            let removed = skeleton.keypoints.remove(index).name;
            skeleton
                .edges
                .retain(|edge| edge.from != removed && edge.to != removed);
        }
        if ui.button("Add keypoint").clicked() {
            skeleton.keypoints.push(KeypointSpec {
                name: next_keypoint_name(skeleton),
                required: true,
            });
        }

        ui.add_space(4.0);
        ui.label(RichText::new("Edges").strong());
        let keypoint_names: Vec<_> = skeleton
            .keypoints
            .iter()
            .map(|keypoint| keypoint.name.clone())
            .collect();
        let mut remove_edge = None;
        for (edge_index, edge) in skeleton.edges.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label("From");
                egui::ComboBox::from_id_salt(format!(
                    "skeleton-edge-from-{task_index}-{edge_index}"
                ))
                .selected_text(&edge.from)
                .show_ui(ui, |ui| {
                    for name in &keypoint_names {
                        ui.selectable_value(&mut edge.from, name.clone(), name);
                    }
                });
                ui.label("To");
                egui::ComboBox::from_id_salt(format!("skeleton-edge-to-{task_index}-{edge_index}"))
                    .selected_text(&edge.to)
                    .show_ui(ui, |ui| {
                        for name in &keypoint_names {
                            ui.selectable_value(&mut edge.to, name.clone(), name);
                        }
                    });
                if destructive_button(ui, "Remove edge") {
                    remove_edge = Some(edge_index);
                }
            });
        }
        if let Some(index) = remove_edge {
            skeleton.edges.remove(index);
        }
        let next_edge = next_skeleton_edge(skeleton);
        if ui
            .add_enabled(next_edge.is_some(), egui::Button::new("Add edge"))
            .on_hover_text(if next_edge.is_some() {
                "Connect two keypoints that are not already connected."
            } else {
                "Add at least two keypoints, or remove an existing edge first."
            })
            .clicked()
            && let Some(edge) = next_edge
        {
            skeleton.edges.push(edge);
        }

        show_issues(ui, &skeleton_issues(skeleton, "Skeleton"));
    });
}

fn next_keypoint_name(skeleton: &SkeletonSpec) -> String {
    next_numbered_id(
        "keypoint",
        skeleton
            .keypoints
            .iter()
            .map(|keypoint| keypoint.name.as_str()),
    )
}

fn next_skeleton_edge(skeleton: &SkeletonSpec) -> Option<SkeletonEdge> {
    for (from_index, from) in skeleton.keypoints.iter().enumerate() {
        for to in skeleton.keypoints.iter().skip(from_index + 1) {
            if from.name == to.name {
                continue;
            }
            let candidate = canonical_edge(&from.name, &to.name);
            let exists = skeleton
                .edges
                .iter()
                .any(|edge| canonical_edge(&edge.from, &edge.to) == candidate);
            if !exists {
                return Some(SkeletonEdge {
                    from: from.name.clone(),
                    to: to.name.clone(),
                });
            }
        }
    }
    None
}

fn canonical_edge<'a>(from: &'a str, to: &'a str) -> (&'a str, &'a str) {
    if from <= to { (from, to) } else { (to, from) }
}

fn edit_review(ui: &mut egui::Ui, task_index: usize, task: &mut TaskDefinition) {
    ui.collapsing("Review configuration", |ui| {
        ui.horizontal(|ui| {
            ui.label("Workflow");
            let previous = task.review.workflow.clone();
            egui::ComboBox::from_id_salt(format!("review-workflow-{task_index}"))
                .selected_text(review_workflow_name(&task.review.workflow))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut task.review.workflow, ReviewWorkflow::None, "none");
                    ui.selectable_value(
                        &mut task.review.workflow,
                        ReviewWorkflow::Approval,
                        "approval",
                    );
                    ui.selectable_value(
                        &mut task.review.workflow,
                        ReviewWorkflow::IndependentAgreement,
                        "independent agreement",
                    );
                });
            if task.review.workflow != previous {
                match task.review.workflow {
                    ReviewWorkflow::None => {
                        task.review.required_reviews = 0;
                        task.review.agreement_threshold = None;
                    }
                    ReviewWorkflow::Approval => {
                        task.review.required_reviews = task.review.required_reviews.max(1);
                        task.review.agreement_threshold = None;
                    }
                    ReviewWorkflow::IndependentAgreement => {
                        task.review.required_reviews = task.review.required_reviews.max(2);
                        task.review.agreement_threshold = Some(default_agreement(task));
                    }
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Required reviews");
            ui.add(
                egui::DragValue::new(&mut task.review.required_reviews)
                    .range(0..=100)
                    .speed(1),
            )
            .on_hover_text("Number of completed reviews required for this task.");
            ui.checkbox(
                &mut task.review.allow_reviewer_corrections,
                "Allow reviewer correction",
            );
        });
        if task.review.workflow == ReviewWorkflow::IndependentAgreement {
            let mut enabled = task.review.agreement_threshold.is_some();
            if ui
                .checkbox(&mut enabled, "Use agreement threshold")
                .changed()
            {
                task.review.agreement_threshold = enabled.then(|| default_agreement(task));
            }
            if let Some(agreement) = task.review.agreement_threshold.as_mut() {
                agreement.metric = match task.annotation_type {
                    AnnotationType::BoundingBox => AgreementMetric::Iou,
                    AnnotationType::Skeleton => AgreementMetric::KeypointMeanDistance,
                };
                ui.horizontal(|ui| {
                    ui.label("Agreement metric");
                    ui.label(match agreement.metric {
                        AgreementMetric::Iou => "intersection over union",
                        AgreementMetric::KeypointMeanDistance => "keypoint mean distance",
                    });
                    ui.add(
                        egui::Slider::new(&mut agreement.threshold, 0.0..=1.0).text("threshold"),
                    );
                });
            }
        }
    });
}

fn default_agreement(task: &TaskDefinition) -> AgreementThreshold {
    AgreementThreshold {
        metric: match task.annotation_type {
            AnnotationType::BoundingBox => AgreementMetric::Iou,
            AnnotationType::Skeleton => AgreementMetric::KeypointMeanDistance,
        },
        threshold: 0.5,
    }
}

fn review_workflow_name(workflow: &ReviewWorkflow) -> &'static str {
    match workflow {
        ReviewWorkflow::None => "none",
        ReviewWorkflow::Approval => "approval",
        ReviewWorkflow::IndependentAgreement => "independent agreement",
    }
}

fn edit_prelabels(
    ui: &mut egui::Ui,
    configs: &mut Vec<PrelabelConfig>,
    tasks: &mut [TaskDefinition],
) {
    theme::card_frame().show(ui, |ui| {
        ui.heading("Prelabels");
        let mut remove = None;
        for (index, config) in configs.iter_mut().enumerate() {
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Prelabel ID");
                let mut config_id = config.config_id.to_string();
                if ui
                    .text_edit_singleline(&mut config_id)
                    .on_hover_text("Stable prelabel config id referenced by tasks.")
                    .changed()
                {
                    let previous = config.config_id.clone();
                    let updated = PrelabelConfigId::from(config_id);
                    config.config_id = updated.clone();
                    for task in tasks.iter_mut() {
                        for config_id in &mut task.prelabel_config_ids {
                            if config_id == &previous {
                                *config_id = updated.clone();
                            }
                        }
                    }
                }
                ui.label("Name");
                ui.text_edit_singleline(&mut config.name)
                    .on_hover_text("Display name for this prelabel source.");
                ui.checkbox(
                    &mut config.available_to_annotators,
                    "Available to annotators",
                );
                if destructive_button(ui, "Remove prelabel") {
                    remove = Some(index);
                }
            });
            ui.horizontal(|ui| {
                ui.label("Model ID");
                ui.text_edit_singleline(&mut config.model.model_id)
                    .on_hover_text("Stable model id.");
                ui.label("Model name");
                ui.text_edit_singleline(&mut config.model.display_name)
                    .on_hover_text("Model display name.");
            });
            ui.horizontal(|ui| {
                ui.label("Location");
                ui.text_edit_singleline(&mut config.model.location)
                    .on_hover_text("Server/browser model location, depending on execution mode.");
            });
            ui.add(
                egui::Slider::new(
                    &mut config.output_processing.confidence_threshold,
                    0.0..=1.0,
                )
                .text("confidence"),
            );
        }
        if let Some(index) = remove {
            let removed = configs.remove(index).config_id;
            for task in tasks.iter_mut() {
                task.prelabel_config_ids
                    .retain(|config_id| config_id != &removed);
            }
        }
        if ui.button("Add browser prelabel config").clicked() {
            configs.push(PrelabelConfig {
                config_id: PrelabelConfigId::from(next_numbered_id(
                    "prelabel",
                    configs.iter().map(|config| config.config_id.as_str()),
                )),
                name: "New prelabel".to_string(),
                model: ModelSpec {
                    model_id: "model".to_string(),
                    display_name: "Model".to_string(),
                    version: None,
                    location: "models/model.onnx".to_string(),
                },
                execution: PrelabelExecution::BrowserLocal {
                    acceleration: BrowserAcceleration::WasmCpuFallback,
                },
                output_processing: OutputProcessing {
                    confidence_threshold: 0.5,
                    suppress_overlaps_iou: None,
                },
                available_to_annotators: true,
            });
        }
        show_issues(ui, &prelabel_issues(configs));
    });
}

fn edit_imbalance(ui: &mut egui::Ui, imbalance: &mut Option<ImbalanceConfig>) {
    theme::card_frame().show(ui, |ui| {
        ui.heading("Assignment Balance");
        ui.label(
            RichText::new("Limit how unevenly work may be distributed across classes.")
                .color(theme::MUTED),
        );
        let mut configured = imbalance.is_some();
        if ui
            .checkbox(&mut configured, "Configure imbalance limits")
            .changed()
        {
            *imbalance = configured.then(ImbalanceConfig::default);
        }
        if let Some(imbalance) = imbalance.as_mut() {
            ui.horizontal(|ui| {
                ui.label("Maximum class ratio");
                ui.add(
                    egui::DragValue::new(&mut imbalance.max_ratio)
                        .range(1.0..=1000.0)
                        .speed(0.1),
                )
                .on_hover_text(
                    "Largest allowed ratio between over- and under-represented classes.",
                );
                ui.checkbox(&mut imbalance.enforce, "Enforce limit");
            });
            show_issues(ui, &imbalance_issues(Some(imbalance)));
        }
    });
}

fn edit_roles(
    ui: &mut egui::Ui,
    assignments: &mut Vec<DatasetRoleAssignment>,
    dataset_id: &labello_domain::DatasetId,
    current_user: &UserId,
) {
    theme::card_frame().show(ui, |ui| {
        ui.heading("Access Roles");
        ui.label(
            RichText::new("At least one user, including you, must remain a data admin.")
                .color(theme::MUTED),
        );
        let mut remove = None;
        for (index, assignment) in assignments.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label("User ID");
                let mut user_id = assignment.user_id.to_string();
                if ui
                    .text_edit_singleline(&mut user_id)
                    .on_hover_text("User id receiving these dataset roles.")
                    .changed()
                {
                    assignment.user_id = UserId::from(user_id);
                }
                role_checkbox(
                    ui,
                    &mut assignment.roles,
                    DatasetRole::Annotator,
                    "annotator",
                );
                role_checkbox(ui, &mut assignment.roles, DatasetRole::Reviewer, "reviewer");
                role_checkbox(
                    ui,
                    &mut assignment.roles,
                    DatasetRole::Adjudicator,
                    "adjudicator",
                );
                role_checkbox(
                    ui,
                    &mut assignment.roles,
                    DatasetRole::DataAdmin,
                    "data_admin",
                );
                if assignment.user_id != *current_user
                    && destructive_button(ui, "Remove assignment")
                {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            assignments.remove(index);
        }
        if ui.button("Add role assignment").clicked() {
            assignments.push(DatasetRoleAssignment {
                dataset_id: dataset_id.clone(),
                user_id: UserId::from(next_user_id(assignments)),
                roles: BTreeSet::from([DatasetRole::Annotator]),
                assigned_at: labello_domain::now(),
                assigned_by: Some(current_user.clone()),
            });
        }
        show_issues(ui, &role_issues(assignments, dataset_id, current_user));
    });
}

fn next_user_id(assignments: &[DatasetRoleAssignment]) -> String {
    for index in 1.. {
        let candidate = if index == 1 {
            "new_user".to_string()
        } else {
            format!("new_user_{index}")
        };
        if assignments
            .iter()
            .all(|assignment| assignment.user_id.as_str() != candidate)
        {
            return candidate;
        }
    }
    unreachable!()
}

fn next_numbered_id<'a>(prefix: &str, values: impl Iterator<Item = &'a str>) -> String {
    let values: BTreeSet<_> = values.collect();
    for index in 1.. {
        let candidate = format!("{prefix}_{index}");
        if !values.contains(candidate.as_str()) {
            return candidate;
        }
    }
    unreachable!()
}

fn role_checkbox(
    ui: &mut egui::Ui,
    roles: &mut BTreeSet<DatasetRole>,
    role: DatasetRole,
    label: &str,
) {
    let mut enabled = roles.contains(&role);
    if ui.checkbox(&mut enabled, label).changed() {
        if enabled {
            roles.insert(role);
        } else {
            roles.remove(&role);
        }
    }
}

fn config_issues(config: &DatasetMetadata, current_user: &UserId) -> Vec<String> {
    let mut issues = dataset_issues(&config.name, &config.image_roots);
    issues.extend(class_issues(&config.label_classes));
    issues.extend(task_issues(
        &config.tasks,
        &config.label_classes,
        &config.prelabel_configs,
    ));
    issues.extend(prelabel_issues(&config.prelabel_configs));
    issues.extend(imbalance_issues(config.imbalance.as_ref()));
    issues.extend(role_issues(
        &config.role_assignments,
        &config.dataset_id,
        current_user,
    ));
    issues
}

fn dataset_issues(name: &str, image_roots: &[String]) -> Vec<String> {
    let mut issues = dataset_name_issues(name);
    issues.extend(image_root_issues(image_roots));
    issues
}

fn dataset_name_issues(name: &str) -> Vec<String> {
    let mut issues = Vec::new();
    if name.trim().is_empty() {
        issues.push("Dataset: enter a non-empty dataset name.".to_string());
    }
    issues
}

fn image_root_issues(image_roots: &[String]) -> Vec<String> {
    let mut issues = Vec::new();
    if image_roots.is_empty() {
        issues.push("Image roots: add at least one dataset-relative root path.".to_string());
    }
    for (index, root) in image_roots.iter().enumerate() {
        if !is_safe_relative_path(root) {
            issues.push(format!(
                "Image roots: root {} must be a non-empty relative path without '..' or backslashes.",
                index + 1
            ));
        }
    }
    issues
}

fn class_issues(labels: &[LabelClass]) -> Vec<String> {
    let mut issues = Vec::new();
    let mut ids = BTreeSet::new();
    for (index, label) in labels.iter().enumerate() {
        let context = format!("Class {}", index + 1);
        validate_id(&mut issues, &context, label.class_id.as_str());
        if !ids.insert(label.class_id.as_str()) {
            issues.push(format!(
                "Classes: class ID '{}' is duplicated; choose a unique ID.",
                label.class_id
            ));
        }
        if label.name.trim().is_empty() {
            issues.push(format!("{context}: enter a non-empty class name."));
        }
        if !is_hex_color(&label.color) {
            issues.push(format!(
                "{context}: color '{}' is invalid; use # followed by six hexadecimal digits.",
                label.color
            ));
        }
    }
    issues
}

fn task_issues(
    tasks: &[TaskDefinition],
    labels: &[LabelClass],
    prelabels: &[PrelabelConfig],
) -> Vec<String> {
    let mut issues = Vec::new();
    let class_ids: BTreeSet<_> = labels.iter().map(|label| &label.class_id).collect();
    let prelabel_ids: BTreeSet<_> = prelabels.iter().map(|config| &config.config_id).collect();
    let mut task_ids = BTreeSet::new();
    for (index, task) in tasks.iter().enumerate() {
        let context = format!("Workflow {}", index + 1);
        validate_id(&mut issues, &context, task.task_id.as_str());
        if !task_ids.insert(task.task_id.as_str()) {
            issues.push(format!(
                "Workflows: task ID '{}' is duplicated; choose a unique ID.",
                task.task_id
            ));
        }
        if task.name.trim().is_empty() {
            issues.push(format!("{context}: enter a non-empty task name."));
        }
        if task.class_ids.is_empty() {
            issues.push(format!(
                "{context}: select at least one allowed class for task '{}'.",
                task.task_id
            ));
        }
        let mut linked_classes = BTreeSet::new();
        for class_id in &task.class_ids {
            if !class_ids.contains(class_id) {
                issues.push(format!(
                    "{context}: task '{}' references missing class '{}'; select an existing class or remove the reference.",
                    task.task_id, class_id
                ));
            }
            if !linked_classes.insert(class_id) {
                issues.push(format!(
                    "{context}: class '{}' is selected more than once.",
                    class_id
                ));
            }
        }
        let mut linked_prelabels = BTreeSet::new();
        for config_id in &task.prelabel_config_ids {
            if !prelabel_ids.contains(config_id) {
                issues.push(format!(
                    "{context}: task '{}' references missing prelabel '{}'; select an existing source or remove the reference.",
                    task.task_id, config_id
                ));
            }
            if !linked_prelabels.insert(config_id) {
                issues.push(format!(
                    "{context}: prelabel '{}' is selected more than once.",
                    config_id
                ));
            }
        }
        if task.instructions.title.trim().is_empty() {
            issues.push(format!("{context}: enter a non-empty instruction title."));
        }
        for (image_index, path) in task.instructions.example_images.iter().enumerate() {
            if !is_safe_relative_path(path) {
                issues.push(format!(
                    "{context}: tutorial image path {} must be a non-empty dataset-relative path without '..' or backslashes.",
                    image_index + 1
                ));
            }
        }
        if task.annotation_type == AnnotationType::Skeleton {
            if let Some(skeleton) = task.skeleton.as_ref() {
                issues.extend(skeleton_issues(skeleton, &format!("{context} skeleton")));
            } else {
                issues.push(format!(
                    "{context}: skeleton task '{}' needs a skeleton specification.",
                    task.task_id
                ));
            }
        }
        validate_review(&mut issues, &context, task);
    }
    issues
}

fn skeleton_issues(skeleton: &SkeletonSpec, context: &str) -> Vec<String> {
    let mut issues = Vec::new();
    if skeleton.keypoints.is_empty() {
        issues.push(format!("{context}: add at least one keypoint."));
    }

    let mut keypoint_names = BTreeSet::new();
    for (index, keypoint) in skeleton.keypoints.iter().enumerate() {
        if keypoint.name.trim().is_empty() {
            issues.push(format!(
                "{context}: keypoint {} needs a non-empty name.",
                index + 1
            ));
        }
        if !keypoint_names.insert(keypoint.name.as_str()) {
            issues.push(format!(
                "{context}: keypoint name '{}' is duplicated; choose a unique name.",
                keypoint.name
            ));
        }
    }

    let mut edges = BTreeSet::new();
    for (index, edge) in skeleton.edges.iter().enumerate() {
        let edge_context = format!("{context} edge {}", index + 1);
        if !keypoint_names.contains(edge.from.as_str()) {
            issues.push(format!(
                "{edge_context}: from endpoint '{}' is not an existing keypoint.",
                edge.from
            ));
        }
        if !keypoint_names.contains(edge.to.as_str()) {
            issues.push(format!(
                "{edge_context}: to endpoint '{}' is not an existing keypoint.",
                edge.to
            ));
        }
        if edge.from == edge.to {
            issues.push(format!(
                "{edge_context}: from and to must be different keypoints."
            ));
        }
        if !edges.insert(canonical_edge(&edge.from, &edge.to)) {
            issues.push(format!(
                "{edge_context}: edge '{} - {}' is duplicated.",
                edge.from, edge.to
            ));
        }
    }
    issues
}

fn validate_review(issues: &mut Vec<String>, context: &str, task: &TaskDefinition) {
    match task.review.workflow {
        ReviewWorkflow::None => {}
        ReviewWorkflow::Approval if task.review.required_reviews == 0 => issues.push(format!(
            "{context}: approval workflow requires at least one review."
        )),
        ReviewWorkflow::IndependentAgreement if task.review.required_reviews < 2 => issues.push(
            format!("{context}: independent agreement requires at least two reviews."),
        ),
        _ => {}
    }
    if task.review.workflow == ReviewWorkflow::IndependentAgreement
        && task.review.agreement_threshold.is_none()
    {
        issues.push(format!(
            "{context}: enable an agreement threshold for independent agreement."
        ));
    }
    if task.review.workflow == ReviewWorkflow::IndependentAgreement
        && let Some(agreement) = &task.review.agreement_threshold
    {
        if !agreement.threshold.is_finite() || !(0.0..=1.0).contains(&agreement.threshold) {
            issues.push(format!(
                "{context}: agreement threshold must be between 0 and 1."
            ));
        }
        let metric_matches = matches!(
            (&task.annotation_type, &agreement.metric),
            (AnnotationType::BoundingBox, AgreementMetric::Iou)
                | (
                    AnnotationType::Skeleton,
                    AgreementMetric::KeypointMeanDistance
                )
        );
        if !metric_matches {
            issues.push(format!(
                "{context}: agreement metric must match the annotation type."
            ));
        }
    }
}

fn prelabel_issues(configs: &[PrelabelConfig]) -> Vec<String> {
    let mut issues = Vec::new();
    let mut ids = BTreeSet::new();
    for (index, config) in configs.iter().enumerate() {
        let context = format!("Prelabel {}", index + 1);
        validate_id(&mut issues, &context, config.config_id.as_str());
        if !ids.insert(config.config_id.as_str()) {
            issues.push(format!(
                "Prelabels: prelabel ID '{}' is duplicated; choose a unique ID.",
                config.config_id
            ));
        }
        if config.name.trim().is_empty() {
            issues.push(format!("{context}: enter a non-empty prelabel name."));
        }
        if config.model.model_id.trim().is_empty() {
            issues.push(format!("{context}: enter a non-empty model ID."));
        }
        if config.model.display_name.trim().is_empty() {
            issues.push(format!("{context}: enter a non-empty model name."));
        }
        if config.model.location.trim().is_empty() {
            issues.push(format!("{context}: enter a model location."));
        }
        let confidence = config.output_processing.confidence_threshold;
        if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
            issues.push(format!(
                "{context}: confidence threshold must be between 0 and 1."
            ));
        }
        if let Some(iou) = config.output_processing.suppress_overlaps_iou
            && (!iou.is_finite() || !(0.0..=1.0).contains(&iou))
        {
            issues.push(format!(
                "{context}: overlap suppression IoU must be between 0 and 1."
            ));
        }
    }
    issues
}

fn imbalance_issues(imbalance: Option<&ImbalanceConfig>) -> Vec<String> {
    let Some(imbalance) = imbalance else {
        return Vec::new();
    };
    if imbalance.max_ratio.is_finite() && imbalance.max_ratio >= 1.0 {
        Vec::new()
    } else {
        vec![
            "Assignment balance: maximum class ratio must be a finite value of at least 1."
                .to_string(),
        ]
    }
}

fn role_issues(
    assignments: &[DatasetRoleAssignment],
    dataset_id: &labello_domain::DatasetId,
    current_user: &UserId,
) -> Vec<String> {
    let mut issues = Vec::new();
    let mut users = BTreeSet::new();
    for (index, assignment) in assignments.iter().enumerate() {
        let context = format!("Role assignment {}", index + 1);
        validate_id(&mut issues, &context, assignment.user_id.as_str());
        if !users.insert(assignment.user_id.as_str()) {
            issues.push(format!(
                "Access roles: user '{}' has duplicate assignments; combine their roles into one row.",
                assignment.user_id
            ));
        }
        if assignment.dataset_id != *dataset_id {
            issues.push(format!(
                "{context}: assignment belongs to another dataset; remove and recreate it."
            ));
        }
        if assignment.roles.is_empty() {
            issues.push(format!("{context}: select at least one role."));
        }
    }
    let has_admin = assignments.iter().any(|assignment| {
        assignment.dataset_id == *dataset_id && assignment.roles.contains(&DatasetRole::DataAdmin)
    });
    if !has_admin {
        issues.push("Access roles: assign at least one data admin.".to_string());
    }
    let current_user_is_admin = assignments.iter().any(|assignment| {
        assignment.dataset_id == *dataset_id
            && assignment.user_id == *current_user
            && assignment.roles.contains(&DatasetRole::DataAdmin)
    });
    if !current_user_is_admin {
        issues.push(format!(
            "Access roles: keep data_admin enabled for the current user '{}'.",
            current_user
        ));
    }
    issues
}

fn validate_id(issues: &mut Vec<String>, context: &str, value: &str) {
    if value.is_empty() {
        issues.push(format!("{context}: enter a non-empty ID."));
    } else if !is_safe_id(value) {
        issues.push(format!(
            "{context}: ID '{value}' is unsafe; use one path-safe segment under 256 bytes with no '/', '\\', control characters, '.' or '..'."
        ));
    }
}

fn is_safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && !matches!(value, "." | "..")
        && !value
            .bytes()
            .any(|byte| byte.is_ascii_control() || matches!(byte, b'/' | b'\\'))
}

fn is_safe_relative_path(value: &str) -> bool {
    !value.trim().is_empty()
        && value == value.trim()
        && !value.starts_with('/')
        && !value.contains('\\')
        && !value.bytes().any(|byte| byte.is_ascii_control())
        && !value.split('/').any(|part| part.is_empty() || part == "..")
}

fn is_hex_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}

fn labeled_text(ui: &mut egui::Ui, label: &str, value: &mut String) -> egui::Response {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.text_edit_singleline(value)
    })
    .inner
}

fn destructive_button(ui: &mut egui::Ui, label: &str) -> bool {
    ui.small_button(format!("Double-click {label}"))
        .on_hover_text("Double-click to confirm this removal.")
        .double_clicked()
}

fn show_issues(ui: &mut egui::Ui, issues: &[String]) {
    for issue in issues {
        ui.label(RichText::new(format!("- {issue}")).color(theme::RED));
    }
}

fn metric(ui: &mut egui::Ui, label: &str, value: String) {
    theme::card_frame().show(ui, |ui| {
        ui.label(RichText::new(label).color(theme::MUTED));
        ui.heading(value);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_skeleton_is_valid() {
        let skeleton = starter_skeleton_spec();

        assert!(skeleton_issues(&skeleton, "Skeleton").is_empty());
        assert_eq!(skeleton.keypoints.len(), 1);
        assert!(skeleton.keypoints[0].required);
    }

    #[test]
    fn skeleton_validation_rejects_invalid_keypoints_and_edges() {
        let skeleton = SkeletonSpec {
            keypoints: vec![
                KeypointSpec {
                    name: "joint".to_string(),
                    required: true,
                },
                KeypointSpec {
                    name: "joint".to_string(),
                    required: false,
                },
                KeypointSpec {
                    name: " ".to_string(),
                    required: false,
                },
            ],
            edges: vec![
                SkeletonEdge {
                    from: "joint".to_string(),
                    to: "joint".to_string(),
                },
                SkeletonEdge {
                    from: "missing".to_string(),
                    to: "joint".to_string(),
                },
                SkeletonEdge {
                    from: "joint".to_string(),
                    to: "missing".to_string(),
                },
            ],
            allow_hidden: true,
            allow_absent: true,
        };

        let issues = skeleton_issues(&skeleton, "Skeleton").join("\n");
        assert!(issues.contains("non-empty name"));
        assert!(issues.contains("duplicated; choose a unique name"));
        assert!(issues.contains("from and to must be different"));
        assert!(issues.contains("from endpoint 'missing'"));
        assert!(issues.contains("to endpoint 'missing'"));
    }

    #[test]
    fn skeleton_validation_requires_a_keypoint() {
        let mut skeleton = starter_skeleton_spec();
        skeleton.keypoints.clear();

        assert!(
            skeleton_issues(&skeleton, "Skeleton")
                .iter()
                .any(|issue| issue.contains("add at least one keypoint"))
        );
    }

    #[test]
    fn skeleton_validation_treats_reversed_edges_as_duplicates() {
        let skeleton = SkeletonSpec {
            keypoints: vec![
                KeypointSpec {
                    name: "left".to_string(),
                    required: true,
                },
                KeypointSpec {
                    name: "right".to_string(),
                    required: true,
                },
            ],
            edges: vec![
                SkeletonEdge {
                    from: "left".to_string(),
                    to: "right".to_string(),
                },
                SkeletonEdge {
                    from: "right".to_string(),
                    to: "left".to_string(),
                },
            ],
            allow_hidden: false,
            allow_absent: false,
        };

        assert!(
            skeleton_issues(&skeleton, "Skeleton")
                .iter()
                .any(|issue| issue.contains("is duplicated"))
        );
    }

    #[test]
    fn switching_annotation_type_initializes_and_clears_skeleton() {
        let class = LabelClass {
            class_id: ClassId::from("person"),
            name: "Person".to_string(),
            color: "#5eead4".to_string(),
            description: None,
        };
        let mut task = workflow_task_for_class(&class, AnnotationType::BoundingBox);

        set_task_annotation_type(&mut task, AnnotationType::Skeleton);
        assert!(task.skeleton.is_some());
        assert!(skeleton_issues(task.skeleton.as_ref().unwrap(), "Skeleton").is_empty());

        set_task_annotation_type(&mut task, AnnotationType::BoundingBox);
        assert!(task.skeleton.is_none());
    }
}
