use std::collections::BTreeSet;

use eframe::egui::{self, RichText};
use labello_domain::{
    AnnotationType, BrowserAcceleration, ClassId, DatasetRole, DatasetRoleAssignment, LabelClass,
    ModelSpec, OutputProcessing, PrelabelConfig, PrelabelConfigId, PrelabelExecution, ReviewConfig,
    TaskDefinition, TaskId, TutorialContent, UserId,
};

use crate::{app::LabelloApp, theme};

impl LabelloApp {
    pub(crate) fn admin_view(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Dataset Admin");
            if self.loading.admin {
                ui.spinner();
            }
            if ui.button("Reload Admin Config").clicked() {
                self.request_admin_dataset();
            }
        });
        let current_user = self.config.user_id.clone();
        let ingesting_now = self.loading.ingesting;
        let Some(config) = self.datasets.admin_config.as_mut() else {
            ui.label(RichText::new("Admin config is not loaded.").color(theme::MUTED));
            if ui.button("Load admin config").clicked() {
                self.request_admin_dataset();
            }
            return;
        };

        let mut save = false;
        let mut ingest = false;
        egui::ScrollArea::vertical().show(ui, |ui| {
            theme::card_frame().show(ui, |ui| {
                ui.heading("Dataset");
                ui.text_edit_singleline(&mut config.name);
            });

            theme::card_frame().show(ui, |ui| {
                ui.heading("Image Roots");
                edit_string_list(ui, &mut config.image_roots, "Add image root", "images");
                ui.small("Paths are relative to the dataset root and may be edited in labello.dataset.toml.");
            });

            edit_labels(ui, &mut config.label_classes);
            edit_tasks(ui, &mut config.tasks, &config.label_classes);
            edit_prelabels(ui, &mut config.prelabel_configs);
            edit_roles(ui, &mut config.role_assignments, &config.dataset_id, &current_user);

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Save Admin Config").clicked() {
                    save = true;
                }
                if ui.button("Run Ingest").clicked() {
                    ingest = true;
                }
                if ingesting_now {
                    ui.spinner();
                }
            });
        });
        if save {
            self.request_admin_save();
        }
        if ingest {
            self.request_ingest();
        }
    }

    pub(crate) fn stats_view(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Live Statistics");
            if self.loading.stats {
                ui.spinner();
            }
            if ui.button("Refresh now").clicked() {
                self.request_stats();
            }
        });
        ui.add_space(8.0);
        ui.columns(4, |columns| {
            metric(
                &mut columns[0],
                "Images",
                self.datasets.stats.total_images.to_string(),
            );
            metric(
                &mut columns[1],
                "Completed",
                self.datasets.stats.completed_tasks.to_string(),
            );
            metric(
                &mut columns[2],
                "Pending",
                self.datasets.stats.pending_tasks.to_string(),
            );
            metric(
                &mut columns[3],
                "Unreviewed",
                self.datasets.stats.unreviewed_tasks.to_string(),
            );
        });
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
    }
}

fn edit_string_list(ui: &mut egui::Ui, values: &mut Vec<String>, button: &str, default: &str) {
    let mut remove = None;
    for (index, value) in values.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.text_edit_singleline(value);
            if ui.small_button("Remove").clicked() {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        values.remove(index);
    }
    if ui.button(button).clicked() {
        values.push(default.to_string());
    }
}

fn edit_labels(ui: &mut egui::Ui, labels: &mut Vec<LabelClass>) {
    theme::card_frame().show(ui, |ui| {
        ui.heading("Labels");
        let mut remove = None;
        for (index, label) in labels.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                let mut class_id = label.class_id.to_string();
                if ui.text_edit_singleline(&mut class_id).changed() {
                    label.class_id = ClassId::from(class_id);
                }
                ui.text_edit_singleline(&mut label.name);
                ui.text_edit_singleline(&mut label.color);
                let description = label.description.get_or_insert_with(String::new);
                ui.text_edit_singleline(description);
                if ui.small_button("Remove").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            labels.remove(index);
        }
        if ui.button("Add label").clicked() {
            labels.push(LabelClass {
                class_id: ClassId::from(format!("class_{}", labels.len() + 1)),
                name: "New class".to_string(),
                color: "#5eead4".to_string(),
                description: None,
            });
        }
    });
}

fn edit_tasks(ui: &mut egui::Ui, tasks: &mut Vec<TaskDefinition>, labels: &[LabelClass]) {
    theme::card_frame().show(ui, |ui| {
        ui.heading("Tasks");
        let mut remove = None;
        for (index, task) in tasks.iter_mut().enumerate() {
            ui.separator();
            ui.horizontal(|ui| {
                let mut task_id = task.task_id.to_string();
                if ui.text_edit_singleline(&mut task_id).changed() {
                    task.task_id = TaskId::from(task_id);
                }
                ui.text_edit_singleline(&mut task.name);
                ui.checkbox(&mut task.enabled, "enabled");
                if ui.small_button("Remove").clicked() {
                    remove = Some(index);
                }
            });
            egui::ComboBox::from_id_salt(format!("task-type-{index}"))
                .selected_text(task.annotation_type.to_string())
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut task.annotation_type,
                        AnnotationType::BoundingBox,
                        "bounding_box",
                    );
                    ui.selectable_value(
                        &mut task.annotation_type,
                        AnnotationType::Skeleton,
                        "skeleton",
                    );
                });
            ui.horizontal(|ui| {
                ui.label("Instruction title");
                ui.text_edit_singleline(&mut task.instructions.title);
            });
            ui.text_edit_multiline(&mut task.instructions.example_text);
            ui.label("Allowed classes");
            for label in labels {
                let mut enabled = task.class_ids.contains(&label.class_id);
                if ui.checkbox(&mut enabled, &label.name).changed() {
                    if enabled {
                        task.class_ids.push(label.class_id.clone());
                    } else {
                        task.class_ids
                            .retain(|class_id| class_id != &label.class_id);
                    }
                }
            }
        }
        if let Some(index) = remove {
            tasks.remove(index);
        }
        if ui.button("Add task").clicked() {
            let class_ids = labels
                .first()
                .map(|label| vec![label.class_id.clone()])
                .unwrap_or_default();
            tasks.push(TaskDefinition {
                task_id: TaskId::from(format!("task_{}", tasks.len() + 1)),
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
    });
}

fn edit_prelabels(ui: &mut egui::Ui, configs: &mut Vec<PrelabelConfig>) {
    theme::card_frame().show(ui, |ui| {
        ui.heading("Prelabels");
        let mut remove = None;
        for (index, config) in configs.iter_mut().enumerate() {
            ui.separator();
            ui.horizontal(|ui| {
                let mut config_id = config.config_id.to_string();
                if ui.text_edit_singleline(&mut config_id).changed() {
                    config.config_id = PrelabelConfigId::from(config_id);
                }
                ui.text_edit_singleline(&mut config.name);
                ui.checkbox(&mut config.available_to_annotators, "available");
                if ui.small_button("Remove").clicked() {
                    remove = Some(index);
                }
            });
            ui.horizontal(|ui| {
                ui.label("Model");
                ui.text_edit_singleline(&mut config.model.model_id);
                ui.text_edit_singleline(&mut config.model.display_name);
            });
            ui.horizontal(|ui| {
                ui.label("Location");
                ui.text_edit_singleline(&mut config.model.location);
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
            configs.remove(index);
        }
        if ui.button("Add browser prelabel config").clicked() {
            configs.push(PrelabelConfig {
                config_id: PrelabelConfigId::from(format!("prelabel_{}", configs.len() + 1)),
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
    });
}

fn edit_roles(
    ui: &mut egui::Ui,
    assignments: &mut Vec<DatasetRoleAssignment>,
    dataset_id: &labello_domain::DatasetId,
    current_user: &UserId,
) {
    theme::card_frame().show(ui, |ui| {
        ui.heading("Roles");
        let mut remove = None;
        for (index, assignment) in assignments.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                let mut user_id = assignment.user_id.to_string();
                if ui.text_edit_singleline(&mut user_id).changed() {
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
                if assignment.user_id != *current_user && ui.small_button("Remove").clicked() {
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
                user_id: UserId::from("new_user"),
                roles: BTreeSet::from([DatasetRole::Annotator]),
                assigned_at: labello_domain::now(),
                assigned_by: Some(current_user.clone()),
            });
        }
    });
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

fn metric(ui: &mut egui::Ui, label: &str, value: String) {
    theme::card_frame().show(ui, |ui| {
        ui.label(RichText::new(label).color(theme::MUTED));
        ui.heading(value);
    });
}
