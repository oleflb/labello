impl LabelloApp {
    fn admin_schema(&mut self, ui: &mut egui::Ui) {
        ui.heading("Schema");
        ui.label(
            RichText::new("Configure label classes, skeletons, and labeling workflows.")
                .color(theme::TEXT_MUTED),
        );
        let enabled = !self.loading.admin
            && self.loading.roles_user.is_none()
            && !self.loading.uploading
            && !self.loading.ingesting;
        if let Some(config) = self.datasets.admin_config.as_mut() {
            ui.add_enabled_ui(enabled, |ui| {
                edit_quick_workflows(ui, config);
                edit_labels(ui, &mut config.label_classes, &mut config.tasks);
                edit_tasks(
                    ui,
                    &mut config.tasks,
                    &config.label_classes,
                    &config.prelabel_configs,
                );
            });
        }
    }
}

fn admin_card(ui: &mut egui::Ui, label: &'static str, add_contents: impl FnOnce(&mut egui::Ui)) {
    let response = theme::card_frame().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        add_contents(ui);
    });
    response
        .response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, label));
}

fn edit_quick_workflows(ui: &mut egui::Ui, config: &mut DatasetMetadata) {
    admin_card(ui, "Class Workflows card", |ui| {
        ui.heading("Class Workflows");
        ui.label(
            RichText::new("Fast path: create a class and its worker-visible task together.")
                .color(theme::MUTED),
        );
        ui.horizontal_wrapped(|ui| {
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
            ui.horizontal_wrapped(|ui| {
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
        manual_box_guide_migration: None,
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
    config
        .tasks
        .iter()
        .any(|task| &task.annotation_type == annotation_type && task.class_ids.contains(class_id))
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
        ui.horizontal_wrapped(|ui| {
            let field_label = ui.label(label);
            ui.add_sized(
                [ui.available_width().min(360.0), 44.0],
                theme::singleline_text_edit(value),
            )
            .labelled_by(field_label.id)
            .on_hover_text("Dataset-relative path under the dataset root.");
            if destructive_button(ui, "Remove", format!("{label} '{value}'")) {
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
    admin_card(ui, "Classes card", |ui| {
        ui.heading("Classes");
        ui.label(
            RichText::new("Classes define the objects annotators can label.").color(theme::MUTED),
        );
        let mut remove = None;
        let wide = ui.available_width() >= 600.0;
        for (index, label) in labels.iter_mut().enumerate() {
            ui.add_space(4.0);
            theme::inset_frame().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                if edit_class_card(ui, index, label, tasks, wide) {
                    remove = Some(index);
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

fn edit_class_card(
    ui: &mut egui::Ui,
    index: usize,
    label: &mut LabelClass,
    tasks: &mut [TaskDefinition],
    wide: bool,
) -> bool {
    let mut class_id = label.class_id.to_string();
    let mut description = label.description.clone().unwrap_or_default();
    let mut remove = false;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("Class {}", index + 1)).strong());
        ui.label(RichText::new(&label.name).color(theme::BLUE));
        remove = destructive_button(
            ui,
            "Remove class",
            format!("class '{}' ({})", label.name, label.class_id),
        );
    });

    let (id_changed, description_changed) = if wide {
        let mut id_changed = false;
        let mut description_changed = false;
        let spacing = ui.spacing().item_spacing.x;
        let unit = (ui.available_width() - 3.0 * spacing) / 6.0;
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(unit, 68.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let field_label = ui.label("Name");
                    ui.add_sized(
                        [unit, theme::COMPACT_TEXT_FIELD_HEIGHT],
                        theme::singleline_text_edit(&mut label.name),
                    )
                    .labelled_by(field_label.id)
                    .on_hover_text("Display name shown to annotators.");
                },
            );
            ui.allocate_ui_with_layout(
                egui::vec2(unit, 68.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let field_label = ui.label("ID");
                    id_changed = ui
                        .add_sized(
                            [unit, theme::COMPACT_TEXT_FIELD_HEIGHT],
                            theme::singleline_text_edit(&mut class_id),
                        )
                        .labelled_by(field_label.id)
                        .on_hover_text("Stable class id used by annotations and linked workflows.")
                        .changed();
                },
            );
            ui.allocate_ui_with_layout(
                egui::vec2(unit, 68.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let field_label = ui.label("Color");
                    ui.add_sized(
                        [unit, theme::COMPACT_TEXT_FIELD_HEIGHT],
                        theme::singleline_text_edit(&mut label.color),
                    )
                    .labelled_by(field_label.id)
                    .on_hover_text("Class color as a hex value, for example #5eead4.");
                },
            );
            ui.allocate_ui_with_layout(
                egui::vec2(3.0 * unit, 68.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let field_label = ui.label("Description");
                    description_changed = theme::resizable_multiline_text_edit(
                        ui,
                        ui.make_persistent_id(("class-description", index)),
                        &mut description,
                        1,
                        None,
                    )
                    .labelled_by(field_label.id)
                    .on_hover_text("Optional guidance about what belongs in this class.")
                    .changed();
                },
            );
        });
        (id_changed, description_changed)
    } else {
        theme::labeled_text_field(
            ui,
            "Name",
            &mut label.name,
            theme::COMPACT_TEXT_FIELD_HEIGHT,
        )
        .on_hover_text("Display name shown to annotators.");
        let id_changed =
            theme::labeled_text_field(ui, "ID", &mut class_id, theme::COMPACT_TEXT_FIELD_HEIGHT)
                .on_hover_text("Stable class id used by annotations and linked workflows.")
                .changed();
        theme::labeled_text_field(
            ui,
            "Color",
            &mut label.color,
            theme::COMPACT_TEXT_FIELD_HEIGHT,
        )
        .on_hover_text("Class color as a hex value, for example #5eead4.");
        let field_label = ui.label("Description");
        let description_changed = theme::resizable_multiline_text_edit(
            ui,
            ui.make_persistent_id(("class-description", index)),
            &mut description,
            1,
            None,
        )
        .labelled_by(field_label.id)
        .on_hover_text("Optional guidance about what belongs in this class.")
        .changed();
        (id_changed, description_changed)
    };

    if id_changed {
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
    if description_changed {
        label.description = (!description.trim().is_empty()).then_some(description);
    }

    remove
}

fn edit_tasks(
    ui: &mut egui::Ui,
    tasks: &mut Vec<TaskDefinition>,
    labels: &[LabelClass],
    prelabels: &[PrelabelConfig],
) {
    admin_card(ui, "Labeling Workflows card", |ui| {
        ui.heading("Labeling Workflows");
        ui.label(
            RichText::new(
                "Each workflow is one annotation type and one class. Annotators choose between these workflows before claiming work.",
            )
            .color(theme::MUTED),
        );
        let mut remove = None;
        for (index, task) in tasks.iter_mut().enumerate() {
            normalize_task_annotation(task);
            let class_name = task
                .class_ids
                .first()
                .and_then(|class_id| labels.iter().find(|label| &label.class_id == class_id))
                .map(|label| label.name.as_str())
                .unwrap_or("No class");
            let summary = format!(
                "{} | {} | {} | {}",
                task.name,
                task.annotation_type,
                class_name,
                if task.enabled { "Enabled" } else { "Disabled" }
            );
            ui.add_space(4.0);
            theme::inset_frame().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                egui::CollapsingHeader::new(summary)
                    .id_salt(("workflow-editor", index))
                    .default_open(false)
                    .show(ui, |ui| {
                        let remove_clicked = if ui.available_width() >= 760.0 {
                            let mut remove_clicked = false;
                            ui.columns(2, |columns| {
                                remove_clicked = edit_workflow_basics(
                                    &mut columns[0],
                                    index,
                                    task,
                                    labels,
                                    prelabels,
                                );
                                edit_workflow_instructions(&mut columns[1], task);
                            });
                            remove_clicked
                        } else {
                            let remove_clicked =
                                edit_workflow_basics(ui, index, task, labels, prelabels);
                            edit_workflow_instructions(ui, task);
                            remove_clicked
                        };
                        if remove_clicked {
                            remove = Some(index);
                        }
                    });
            });
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
                manual_box_guide_migration: None,
                enabled: true,
            });
        }
        show_issues(ui, &task_issues(tasks, labels, prelabels));
    });
}

fn edit_workflow_basics(
    ui: &mut egui::Ui,
    index: usize,
    task: &mut TaskDefinition,
    labels: &[LabelClass],
    prelabels: &[PrelabelConfig],
) -> bool {
    ui.label(RichText::new("Workflow").color(theme::BLUE).strong());
    let mut task_id = task.task_id.to_string();
    if theme::labeled_text_field(
        ui,
        "Task ID",
        &mut task_id,
        theme::COMPACT_TEXT_FIELD_HEIGHT,
    )
    .on_hover_text("Stable task id used by assignments and event logs.")
    .changed()
    {
        task.task_id = TaskId::from(task_id);
    }
    theme::labeled_text_field(ui, "Name", &mut task.name, theme::COMPACT_TEXT_FIELD_HEIGHT)
        .on_hover_text("Task name shown in the work panel.");
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut task.enabled, "Enabled");
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
                ui.selectable_value(&mut annotation_type, AnnotationType::Skeleton, "skeleton");
            });
        if annotation_type != task.annotation_type {
            set_task_annotation_type(task, annotation_type);
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label("Class");
        if labels.is_empty() {
            ui.label(RichText::new("Add a class first.").color(theme::RED));
        } else {
            let mut selected = task.class_ids.first().cloned();
            let selected_text = selected
                .as_ref()
                .and_then(|class_id| labels.iter().find(|label| &label.class_id == class_id))
                .map(|label| label.name.clone())
                .unwrap_or_else(|| "Select a class".to_string());
            egui::ComboBox::from_id_salt(format!("task-class-{index}"))
                .selected_text(selected_text)
                .show_ui(ui, |ui| {
                    for label in labels {
                        ui.selectable_value(
                            &mut selected,
                            Some(label.class_id.clone()),
                            &label.name,
                        );
                    }
                });
            if selected.as_ref() != task.class_ids.first()
                || task.class_ids.len() != usize::from(selected.is_some())
            {
                task.class_ids = selected.into_iter().collect();
            }
        }
    });
    if task.annotation_type == AnnotationType::Skeleton
        && let Some(skeleton) = task.skeleton.as_mut()
    {
        edit_skeleton(ui, index, skeleton);
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
    destructive_button(
        ui,
        "Remove workflow",
        format!("workflow '{}' ({})", task.name, task.task_id),
    )
}

fn edit_workflow_instructions(ui: &mut egui::Ui, task: &mut TaskDefinition) {
    ui.label(
        RichText::new("Annotator instructions")
            .color(theme::BLUE)
            .strong(),
    );
    theme::labeled_text_field(
        ui,
        "Title",
        &mut task.instructions.title,
        theme::COMPACT_TEXT_FIELD_HEIGHT,
    )
    .on_hover_text("Tutorial/instruction title.");
    let instructions_label = ui.label("Tutorial instructions");
    theme::resizable_multiline_text_edit(
        ui,
        ui.make_persistent_id("tutorial-instructions"),
        &mut task.instructions.example_text,
        3,
        None,
    )
    .labelled_by(instructions_label.id)
    .on_hover_text("Instructions annotators see in the tutorial panel.");
    ui.label("Tutorial example images");
    edit_string_list(
        ui,
        &mut task.instructions.example_images,
        "Image path",
        "Add example image",
        "tutorial/example.png",
    );
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
    egui::CollapsingHeader::new("Skeleton configuration")
        .id_salt(("skeleton-configuration", task_index))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut skeleton.allow_hidden, "Allow occluded keypoints")
                    .on_hover_text(
                        "Annotators may place an estimated position for an occluded keypoint.",
                    );
                ui.checkbox(&mut skeleton.allow_absent, "Allow not-present keypoints")
                    .on_hover_text(
                        "Annotators may record an optional keypoint without a position.",
                    );
            });

            ui.label(RichText::new("Keypoints").strong());
            let mut remove_keypoint = None;
            let mut renames = Vec::new();
            for (keypoint_index, keypoint) in skeleton.keypoints.iter_mut().enumerate() {
                let previous_name = keypoint.name.clone();
                ui.horizontal_wrapped(|ui| {
                    ui.label("Name");
                    ui.add_sized(
                        [ui.available_width().min(280.0), 44.0],
                        theme::singleline_text_edit(&mut keypoint.name),
                    )
                    .on_hover_text("Unique keypoint name used by skeleton edges.");
                    ui.checkbox(&mut keypoint.required, "Required");
                    if destructive_button(
                        ui,
                        "Remove keypoint",
                        format!("keypoint '{}'", keypoint.name),
                    ) {
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
                ui.horizontal_wrapped(|ui| {
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
                    egui::ComboBox::from_id_salt(format!(
                        "skeleton-edge-to-{task_index}-{edge_index}"
                    ))
                    .selected_text(&edge.to)
                    .show_ui(ui, |ui| {
                        for name in &keypoint_names {
                            ui.selectable_value(&mut edge.to, name.clone(), name);
                        }
                    });
                    if destructive_button(
                        ui,
                        "Remove edge",
                        format!("edge '{} -> {}'", edge.from, edge.to),
                    ) {
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
    egui::CollapsingHeader::new("Review configuration")
        .id_salt(("review-configuration", task_index))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Workflow");
                let previous = task.review.workflow.clone();
                egui::ComboBox::from_id_salt(format!("review-workflow-{task_index}"))
                    .selected_text(review_workflow_name(&task.review.workflow))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut task.review.workflow,
                            ReviewWorkflow::None,
                            "none",
                        );
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
            ui.horizontal_wrapped(|ui| {
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
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Agreement metric");
                        ui.label(match agreement.metric {
                            AgreementMetric::Iou => "intersection over union",
                            AgreementMetric::KeypointMeanDistance => "keypoint mean distance",
                        });
                        ui.add(
                            egui::Slider::new(&mut agreement.threshold, 0.0..=1.0)
                                .text("threshold"),
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
    admin_card(ui, "Prelabels card", |ui| {
        ui.heading("Prelabels");
        let mut remove = None;
        for (index, config) in configs.iter_mut().enumerate() {
            ui.separator();
            let wide = ui.available_width() >= 600.0;
            let mut config_id = config.config_id.to_string();
            let config_id_changed = if wide {
                let mut changed = false;
                ui.columns(2, |columns| {
                    changed = theme::labeled_text_field(
                        &mut columns[0],
                        "Prelabel ID",
                        &mut config_id,
                        theme::COMPACT_TEXT_FIELD_HEIGHT,
                    )
                    .on_hover_text("Stable prelabel config id referenced by tasks.")
                    .changed();
                    theme::labeled_text_field(
                        &mut columns[1],
                        "Name",
                        &mut config.name,
                        theme::COMPACT_TEXT_FIELD_HEIGHT,
                    )
                    .on_hover_text("Display name for this prelabel source.");
                });
                changed
            } else {
                let changed = theme::labeled_text_field(
                    ui,
                    "Prelabel ID",
                    &mut config_id,
                    theme::COMPACT_TEXT_FIELD_HEIGHT,
                )
                .on_hover_text("Stable prelabel config id referenced by tasks.")
                .changed();
                theme::labeled_text_field(
                    ui,
                    "Name",
                    &mut config.name,
                    theme::COMPACT_TEXT_FIELD_HEIGHT,
                )
                .on_hover_text("Display name for this prelabel source.");
                changed
            };
            if config_id_changed {
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
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(
                    &mut config.available_to_annotators,
                    "Available to annotators",
                );
                if destructive_button(
                    ui,
                    "Remove prelabel",
                    format!("prelabel '{}' ({})", config.name, config.config_id),
                ) {
                    remove = Some(index);
                }
            });
            if wide {
                ui.columns(2, |columns| {
                    theme::labeled_text_field(
                        &mut columns[0],
                        "Model ID",
                        &mut config.model.model_id,
                        theme::COMPACT_TEXT_FIELD_HEIGHT,
                    )
                    .on_hover_text("Stable model id.");
                    theme::labeled_text_field(
                        &mut columns[1],
                        "Model name",
                        &mut config.model.display_name,
                        theme::COMPACT_TEXT_FIELD_HEIGHT,
                    )
                    .on_hover_text("Model display name.");
                });
            } else {
                theme::labeled_text_field(
                    ui,
                    "Model ID",
                    &mut config.model.model_id,
                    theme::COMPACT_TEXT_FIELD_HEIGHT,
                )
                .on_hover_text("Stable model id.");
                theme::labeled_text_field(
                    ui,
                    "Model name",
                    &mut config.model.display_name,
                    theme::COMPACT_TEXT_FIELD_HEIGHT,
                )
                .on_hover_text("Model display name.");
            }
            theme::labeled_text_field(
                ui,
                "Location",
                &mut config.model.location,
                theme::COMPACT_TEXT_FIELD_HEIGHT,
            )
            .on_hover_text("Server/browser model location, depending on execution mode.");
            ui.scope(|ui| {
                ui.spacing_mut().slider_width = (ui.available_width() - 140.0).max(100.0);
                ui.add_sized(
                    [ui.available_width(), 44.0],
                    egui::Slider::new(
                        &mut config.output_processing.confidence_threshold,
                        0.0..=1.0,
                    )
                    .text("confidence"),
                );
            });
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
    admin_card(ui, "Assignment Balance card", |ui| {
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
            ui.horizontal_wrapped(|ui| {
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
        if task.enabled && task.class_ids.len() != 1 {
            issues.push(format!(
                "{context}: enabled workflow '{}' must select exactly one class.",
                task.task_id
            ));
        } else if task.class_ids.len() > 1 {
            issues.push(format!(
                "{context}: workflow '{}' can reference only one class.",
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

fn destructive_button(ui: &mut egui::Ui, label: &str, item: String) -> bool {
    let response = theme::danger_button(ui, true, egui::Button::new(label));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{label}: {item}"))
    });
    let modal_id = response.id.with("confirmation");
    if response.clicked() {
        ui.ctx().data_mut(|data| data.insert_temp(modal_id, true));
    }
    if !ui
        .ctx()
        .data(|data| data.get_temp::<bool>(modal_id).unwrap_or(false))
    {
        return false;
    }

    let mut confirmed = false;
    let response = theme::modal(ui.ctx(), modal_id).show(ui.ctx(), |ui| {
        ui.set_max_width((ui.ctx().content_rect().width() - 48.0).clamp(240.0, 480.0));
        ui.heading("Confirm removal");
        ui.label(format!(
            "Remove {item} from the staged dataset configuration? Related staged references will also be removed when required."
        ));
        ui.horizontal_wrapped(|ui| {
            if theme::danger_button(ui, true, egui::Button::new("Confirm removal")).clicked() {
                confirmed = true;
                ui.ctx().data_mut(|data| data.remove::<bool>(modal_id));
            }
            if theme::quiet_button(ui, true, egui::Button::new("Cancel")).clicked() {
                ui.ctx().data_mut(|data| data.remove::<bool>(modal_id));
            }
        });
    });
    response.response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Window,
            true,
            format!("Confirm removal: {item}"),
        )
    });
    if response.should_close() {
        ui.ctx().data_mut(|data| data.remove::<bool>(modal_id));
    }
    confirmed
}

fn show_issues(ui: &mut egui::Ui, issues: &[String]) {
    for issue in issues {
        ui.label(RichText::new(format!("- {issue}")).color(theme::DANGER));
    }
}
