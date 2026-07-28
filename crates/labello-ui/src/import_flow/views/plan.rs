impl LabelloApp {
    fn import_preflight_step(&mut self, ui: &mut egui::Ui) {
        let previous_report = self.import_flow.plan.is_none()
            && self.import_flow.pending_plan_request.is_none()
            && self
                .import_flow
                .job
                .as_ref()
                .is_some_and(|job| job.preflight_report.is_some());
        let report_stale =
            previous_report || (self.import_flow.plan.is_some() && !self.import_plan_is_current());
        let report = self
            .import_flow
            .pending_plan_request
            .is_none()
            .then(|| {
                self.import_flow
                    .plan
                    .as_ref()
                    .map(|plan| &plan.report)
                    .or_else(|| {
                        self.import_flow
                            .job
                            .as_ref()
                            .and_then(|job| job.preflight_report.as_ref())
                    })
            })
            .flatten()
            .cloned();
        if let Some(report) = report {
            if report_stale {
                theme::inline_message(
                    ui,
                    theme::Intent::Warning,
                    "Last accepted preflight — current edits are not included. Save the corrected mappings to refresh diagnostics and readiness.",
                );
            }
            ui.label(RichText::new("Preflight summary").strong());
            status_row(ui, "Images", report.source.images.to_string());
            status_row(ui, "Objects", report.source.objects.to_string());
            status_row(
                ui,
                "Output annotations",
                report.output.annotations.to_string(),
            );
            status_row(
                ui,
                "Geometry",
                format!(
                    "{} direct, {} clipped, {} skipped",
                    report.geometry.direct, report.geometry.clipped, report.geometry.skipped
                ),
            );
            status_row(
                ui,
                "Coverage",
                format!(
                    "{} complete, {} empty, {} incomplete, {} excluded",
                    report.coverage.complete,
                    report.coverage.verified_empty,
                    report.coverage.incomplete,
                    report.coverage.excluded
                ),
            );
            ui.separator();
            self.import_diagnostics_disclosure(ui, &report.diagnostics);
        } else {
            ui.small("Deterministic preflight checks are in progress.");
        }
        ui.separator();
        self.import_mapping_editor(ui);
        let mappings_complete = self.import_mappings_complete();
        let plan_covers_source = self.import_plan_covers_all_categories();
        if let Some(plan) = self
            .import_flow
            .plan
            .as_ref()
            .filter(|_| !plan_covers_source)
        {
            let (required_categories, required_tasks) = self.import_required_output_counts();
            theme::inline_message(
                ui,
                theme::Intent::Error,
                format!(
                    "Required outputs for the current mapping — categories: \
                     {required_categories}, tasks: {required_tasks}. Accepted preflight outputs — \
                     categories: {}, tasks: {}. Click “Save mappings and re-run preflight”; commit \
                     remains disabled until the refreshed plan includes every required output.",
                    plan.report.output.classes, plan.report.output.tasks
                ),
            );
        }
        let commit_ready = self
            .import_flow
            .plan
            .as_ref()
            .is_some_and(|plan| plan.commit_ready)
            && self.import_plan_is_current()
            && plan_covers_source
            && mappings_complete;
        if theme::primary_button(
            ui,
            !self.import_flow.busy && mappings_complete,
            egui::Button::new("Save mappings and re-run preflight"),
        )
        .on_disabled_hover_text(
            "Represent every discovered category and complete the selected workflow before saving.",
        )
        .clicked()
        {
            self.request_update_import_plan();
        }
        if theme::primary_button(
            ui,
            !self.import_flow.busy && commit_ready,
            egui::Button::new("Commit import"),
        )
        .on_disabled_hover_text(
            "Save these exact mappings again after every edit, resolve diagnostics, and acknowledge warnings.",
        )
        .clicked()
        {
            self.request_commit_import();
        }
        if ui
            .add_enabled(!self.import_flow.busy, egui::Button::new("Cancel import"))
            .clicked()
        {
            self.request_cancel_import();
        }
    }

    fn import_diagnostics_disclosure(
        &mut self,
        ui: &mut egui::Ui,
        diagnostics: &[labello_client::ImportDiagnosticSummary],
    ) {
        let overview = ImportDiagnosticOverview::from_diagnostics(
            diagnostics,
            &self.import_flow.acknowledgements,
        );
        let compact = ui.available_width() < 480.0;
        let label = overview.disclosure_label(compact);
        let color = overview.color();

        let disclosure = egui::CollapsingHeader::new(RichText::new(label).strong().color(color))
            .id_salt("import-preflight-diagnostics")
            .default_open(true)
            .show_background(true)
            .show(ui, |ui| {
                if diagnostics.is_empty() {
                    theme::inline_message(ui, theme::Intent::Success, "No diagnostics reported.");
                }
                for diagnostic in diagnostics {
                    let intent = match diagnostic.severity {
                        ImportDiagnosticSeverity::Error => theme::Intent::Error,
                        ImportDiagnosticSeverity::WarningRequiresAck
                        | ImportDiagnosticSeverity::Warning => theme::Intent::Warning,
                        ImportDiagnosticSeverity::Info | ImportDiagnosticSeverity::Unknown => {
                            theme::Intent::Info
                        }
                    };
                    theme::inline_message(
                        ui,
                        intent,
                        format!(
                            "{} diagnostic {}: {} ({} affected)",
                            diagnostic_severity_label(diagnostic.severity),
                            diagnostic.code,
                            diagnostic.safe_summary,
                            diagnostic.count
                        ),
                    );
                    if diagnostic.impact.requires_acknowledgement {
                        let mut acknowledged =
                            self.import_flow.acknowledgements.contains(&diagnostic.code);
                        if ui
                            .checkbox(
                                &mut acknowledged,
                                format!("Acknowledge {}", diagnostic.code),
                            )
                            .changed()
                        {
                            if acknowledged {
                                self.import_flow
                                    .acknowledgements
                                    .insert(diagnostic.code.clone());
                            } else {
                                self.import_flow.acknowledgements.remove(&diagnostic.code);
                            }
                        }
                    }
                }
                if !self.import_flow.diagnostics.is_empty() {
                    ui.label(RichText::new("Diagnostic details").strong());
                    for diagnostic in &self.import_flow.diagnostics {
                        ui.label(format!(
                            "{} diagnostic {}: {}",
                            diagnostic_severity_label(diagnostic.severity),
                            diagnostic.code,
                            diagnostic.safe_summary
                        ));
                    }
                }
                if self.import_flow.diagnostics_cursor.is_some()
                    && ui
                        .add_enabled(
                            !self.import_flow.busy,
                            egui::Button::new("Load more diagnostics"),
                        )
                        .clicked()
                {
                    self.request_import_diagnostics(false);
                }
            });
        let expanded =
            egui::collapsing_header::CollapsingState::load(ui.ctx(), disclosure.header_response.id)
                .is_some_and(|state| state.is_open());
        ui.ctx()
            .accesskit_node_builder(disclosure.header_response.id, |node| {
                node.set_expanded(expanded);
            });
    }

    fn import_mapping_editor(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Category and task mapping").strong());
        let validation = self.import_mapping_validation();
        let errors = validation.error_count();
        let warnings = validation.warning_count();
        if errors > 0 {
            theme::inline_message(
                ui,
                theme::Intent::Error,
                format!(
                    "{errors} mapping {} must be fixed before saving. Each affected input is explained below.",
                    if errors == 1 { "error" } else { "errors" }
                ),
            );
        } else if warnings > 0 {
            theme::inline_message(
                ui,
                theme::Intent::Warning,
                format!(
                    "{warnings} mapping {}. Review the highlighted consequences before saving.",
                    if warnings == 1 { "warning" } else { "warnings" }
                ),
            );
        } else {
            theme::inline_message(
                ui,
                theme::Intent::Success,
                "All mapping inputs are locally valid.",
            );
        }
        show_mapping_issues(ui, &validation, None, ImportMappingField::Form);
        show_mapping_issues(ui, &validation, None, ImportMappingField::CategorySelection);
        let discovered = self
            .import_flow
            .plan
            .as_ref()
            .map(|plan| plan.report.source.categories)
            .or_else(|| {
                self.import_flow
                    .job
                    .as_ref()
                    .and_then(|job| job.preflight_report.as_ref())
                    .map(|report| report.source.categories)
            })
            .unwrap_or(0);
        if discovered > 0 && self.import_flow.categories.len() != discovered as usize {
            theme::inline_message(
                ui,
                theme::Intent::Error,
                "This API contract reports only a category count, not the discovered category keys, IDs, names, or skeleton schemas required for a valid plan. Mapping and commit are disabled; Labello will not guess sparse source IDs.",
            );
            if ui
                .add_enabled(
                    !self.import_flow.busy,
                    egui::Button::new("Restart import setup"),
                )
                .clicked()
            {
                self.restart_import_setup();
            }
            return;
        }
        ui.label(format!(
            "{} mapping rows for {discovered} discovered categories",
            self.import_flow.categories.len()
        ));
        for (index, category) in self.import_flow.categories.iter_mut().enumerate() {
            ui.push_id(("import-category", index), |ui| {
                let (category_errors, category_warnings) = validation.category_counts(index);
                let status = if category_errors > 0 {
                    format!("{category_errors} errors")
                } else if category_warnings > 0 {
                    format!("{category_warnings} warnings")
                } else {
                    "Valid".to_string()
                };
                ui.label(
                    RichText::new(format!(
                        "Category {} · {} · {status}",
                        index + 1,
                        category.source_name
                    ))
                    .strong(),
                );
                ui.checkbox(&mut category.selected, "Include this source category");
                show_mapping_issues(
                    ui,
                    &validation,
                    Some(index),
                    ImportMappingField::CategorySelection,
                );
                show_mapping_issues(ui, &validation, Some(index), ImportMappingField::Form);
                status_row(ui, "Source category key", &category.source_category_key);
                status_row(ui, "Source category ID", &category.source_category_id);
                status_row(ui, "Source category name", &category.source_name);
                status_row(
                    ui,
                    "Direct geometry",
                    category
                        .direct_geometry
                        .iter()
                        .map(|kind| match kind {
                            ImportGeometryKind::BoundingBox => "bounding box",
                            ImportGeometryKind::Skeleton => "skeleton",
                        })
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                theme::labeled_text_field(
                    ui,
                    "Class ID",
                    &mut category.class_id,
                    theme::COMPACT_TEXT_FIELD_HEIGHT,
                );
                show_mapping_issues(ui, &validation, Some(index), ImportMappingField::ClassId);
                theme::labeled_text_field(
                    ui,
                    "Class name",
                    &mut category.class_name,
                    theme::COMPACT_TEXT_FIELD_HEIGHT,
                );
                show_mapping_issues(ui, &validation, Some(index), ImportMappingField::ClassName);
                theme::labeled_text_field(
                    ui,
                    "Class color",
                    &mut category.class_color,
                    theme::COMPACT_TEXT_FIELD_HEIGHT,
                );
                show_mapping_issues(ui, &validation, Some(index), ImportMappingField::ClassColor);
                let active_target = |target| {
                    category.geometry_mappings.iter().any(|mapping| {
                        mapping.target_geometry == target
                            && mapping.policy != ImportGeometryPolicy::Omit
                    })
                };
                let bounding_box_task = active_target(ImportGeometryKind::BoundingBox);
                let skeleton_task = active_target(ImportGeometryKind::Skeleton);
                let task_identity_invalid = [
                    ImportMappingField::BoundingBoxTaskId,
                    ImportMappingField::BoundingBoxTaskName,
                    ImportMappingField::SkeletonTaskId,
                    ImportMappingField::SkeletonTaskName,
                ]
                .into_iter()
                .any(|field| validation.for_field(Some(index), field).next().is_some());
                egui::CollapsingHeader::new("Advanced task identity")
                    .default_open(task_identity_invalid)
                    .show(ui, |ui| {
                        if bounding_box_task {
                            theme::labeled_text_field(
                                ui,
                                "Bounding-box task ID",
                                &mut category.bounding_box_task_id,
                                theme::COMPACT_TEXT_FIELD_HEIGHT,
                            );
                            show_mapping_issues(
                                ui,
                                &validation,
                                Some(index),
                                ImportMappingField::BoundingBoxTaskId,
                            );
                            theme::labeled_text_field(
                                ui,
                                "Bounding-box task name",
                                &mut category.bounding_box_task_name,
                                theme::COMPACT_TEXT_FIELD_HEIGHT,
                            );
                            show_mapping_issues(
                                ui,
                                &validation,
                                Some(index),
                                ImportMappingField::BoundingBoxTaskName,
                            );
                        }
                        if skeleton_task {
                            theme::labeled_text_field(
                                ui,
                                "Skeleton task ID",
                                &mut category.skeleton_task_id,
                                theme::COMPACT_TEXT_FIELD_HEIGHT,
                            );
                            show_mapping_issues(
                                ui,
                                &validation,
                                Some(index),
                                ImportMappingField::SkeletonTaskId,
                            );
                            theme::labeled_text_field(
                                ui,
                                "Skeleton task name",
                                &mut category.skeleton_task_name,
                                theme::COMPACT_TEXT_FIELD_HEIGHT,
                            );
                            show_mapping_issues(
                                ui,
                                &validation,
                                Some(index),
                                ImportMappingField::SkeletonTaskName,
                            );
                        }
                    });
                if !category.geometry_mappings.is_empty() {
                    let needs_target_keypoint_names = category.source_skeleton.is_none()
                        && category.geometry_mappings.iter().any(|mapping| {
                            mapping.target_geometry == ImportGeometryKind::Skeleton
                                && mapping.source_geometry == ImportGeometryKind::BoundingBox
                                && mapping.policy != ImportGeometryPolicy::Omit
                        });
                    let editing_target_keypoint_names = if needs_target_keypoint_names {
                        ui.label(RichText::new("Target skeleton schema").strong());
                        let response = theme::labeled_text_field(
                            ui,
                            "Target keypoint names (comma separated)",
                            &mut category.target_keypoint_names,
                            theme::COMPACT_TEXT_FIELD_HEIGHT,
                        );
                        ui.small(
                            "Each keypoint name creates one template-point control after editing is finished.",
                        );
                        show_mapping_issues(
                            ui,
                            &validation,
                            Some(index),
                            ImportMappingField::TargetKeypointNames,
                        );
                        response.has_focus()
                    } else {
                        false
                    };
                    ui.label(RichText::new("Category geometry outputs").strong());
                    let direct_geometry = category.direct_geometry.clone();
                    let source_skeleton = category.source_skeleton.clone();
                    let target_keypoint_names = category.target_keypoint_names.clone();
                    for (mapping_index, mapping) in
                        category.geometry_mappings.iter_mut().enumerate()
                    {
                        let target = match mapping.target_geometry {
                            ImportGeometryKind::BoundingBox => "bounding box",
                            ImportGeometryKind::Skeleton => "skeleton",
                        };
                        let choices =
                            geometry_choices_for_target(
                                mapping.target_geometry,
                                &direct_geometry,
                                self.import_flow.capabilities.as_ref().is_some_and(
                                    |capabilities| capabilities.manual_box_guide_migration,
                                ),
                                source_skeleton.is_some(),
                            );
                        let current = choices.iter().copied().find(|choice| {
                            choice.policy == mapping.policy
                                && (mapping.policy == ImportGeometryPolicy::Omit
                                    || choice.source == mapping.source_geometry)
                        });
                        let mut selected = current.unwrap_or(ImportGeometryChoice {
                            source: mapping.source_geometry,
                            policy: mapping.policy,
                            label: "Invalid output mapping",
                        });
                        let previous = selected;
                        egui::ComboBox::from_label(format!("{target} output"))
                            .selected_text(selected.label)
                            .show_ui(ui, |ui| {
                                for choice in choices {
                                    ui.selectable_value(&mut selected, choice, choice.label);
                                }
                            });
                        if selected != previous {
                            mapping.source_geometry = selected.source;
                            mapping.policy = selected.policy;
                            mapping.parameters.clear();
                        }
                        ui.push_id(("mapping-parameters", mapping_index), |ui| {
                            mapping_parameter_editor(
                                ui,
                                mapping,
                                source_skeleton.as_ref(),
                                &target_keypoint_names,
                                !editing_target_keypoint_names,
                            );
                        });
                        show_mapping_issues(
                            ui,
                            &validation,
                            Some(index),
                            ImportMappingField::Geometry(mapping.target_geometry),
                        );
                    }
                    if category
                        .direct_geometry
                        .contains(&ImportGeometryKind::BoundingBox)
                        && !category
                            .geometry_mappings
                            .iter()
                            .any(|mapping| mapping.target_geometry == ImportGeometryKind::Skeleton)
                        && ui.button("Add skeleton output").clicked()
                    {
                        category
                            .geometry_mappings
                            .push(ImportGeometryMappingRequest {
                                source_category_key: category.source_category_key.clone(),
                                source_geometry: ImportGeometryKind::BoundingBox,
                                target_geometry: ImportGeometryKind::Skeleton,
                                policy: ImportGeometryPolicy::BoxRelativeTemplateV1,
                                parameters: Vec::new(),
                            });
                    }
                    egui::ComboBox::from_label("Category workflow intent")
                        .selected_text(intent_label(category.workflow_intent))
                        .show_ui(ui, |ui| {
                            for intent in [
                                ImportWorkflowIntent::AuthoritativeGroundTruth,
                                ImportWorkflowIntent::RequireApproval,
                                ImportWorkflowIntent::SeedFutureAnnotation,
                            ] {
                                ui.selectable_value(
                                    &mut category.workflow_intent,
                                    intent,
                                    intent_label(intent),
                                );
                            }
                        });
                    show_mapping_issues(
                        ui,
                        &validation,
                        Some(index),
                        ImportMappingField::WorkflowIntent,
                    );
                }
                ui.separator();
            });
        }
        if self.import_flow.categories.iter().any(|category| {
            category.selected
                && category.workflow_intent == ImportWorkflowIntent::SeedFutureAnnotation
        }) {
            theme::inline_message(
                ui,
                theme::Intent::Warning,
                "Seed workflow keeps imported geometry pending for future human annotation instead of completing it as ground truth.",
            );
            ui.checkbox(
                &mut self.import_flow.seed_workflow_confirmed,
                "Create the selected pending seed workflows",
            );
            show_mapping_issues(ui, &validation, None, ImportMappingField::SeedConfirmation);
        }
        ui.separator();
        ui.label(RichText::new("Compatibility policies").strong());
        if matches!(
            self.import_flow.profile,
            ImportProfile::UltralyticsYoloDetectV1 | ImportProfile::UltralyticsYoloPoseV1
        ) {
            egui::ComboBox::from_label("YOLO missing labels")
                .selected_text(format!("{:?}", self.import_flow.yolo_missing_labels))
                .show_ui(ui, |ui| {
                    for policy in [
                        labello_client::YoloMissingLabelPolicy::Block,
                        labello_client::YoloMissingLabelPolicy::Incomplete,
                        labello_client::YoloMissingLabelPolicy::MissingIsBackground,
                    ] {
                        ui.selectable_value(
                            &mut self.import_flow.yolo_missing_labels,
                            policy,
                            format!("{policy:?}"),
                        );
                    }
                });
            show_mapping_issues(
                ui,
                &validation,
                None,
                ImportMappingField::Compatibility(ImportCompatibilityField::YoloMissingLabels),
            );
            egui::ComboBox::from_label("YOLO duplicate rows")
                .selected_text(format!("{:?}", self.import_flow.yolo_duplicate_rows))
                .show_ui(ui, |ui| {
                    for policy in [
                        labello_client::YoloDuplicateRowPolicy::Block,
                        labello_client::YoloDuplicateRowPolicy::Deduplicate,
                    ] {
                        ui.selectable_value(
                            &mut self.import_flow.yolo_duplicate_rows,
                            policy,
                            format!("{policy:?}"),
                        );
                    }
                });
            show_mapping_issues(
                ui,
                &validation,
                None,
                ImportMappingField::Compatibility(ImportCompatibilityField::YoloDuplicateRows),
            );
            if self.import_flow.profile == ImportProfile::UltralyticsYoloPoseV1 {
                egui::ComboBox::from_label("Missing keypoint names")
                    .selected_text(format!("{:?}", self.import_flow.missing_keypoint_names))
                    .show_ui(ui, |ui| {
                        for policy in [
                            labello_client::MissingKeypointNamesPolicy::Block,
                            labello_client::MissingKeypointNamesPolicy::GenerateIndexed,
                        ] {
                            ui.selectable_value(
                                &mut self.import_flow.missing_keypoint_names,
                                policy,
                                format!("{policy:?}"),
                            );
                        }
                    });
                show_mapping_issues(
                    ui,
                    &validation,
                    None,
                    ImportMappingField::Compatibility(
                        ImportCompatibilityField::MissingKeypointNames,
                    ),
                );
            }
        }
        if matches!(
            self.import_flow.profile,
            ImportProfile::CocoInstancesGtV1 | ImportProfile::CocoKeypointsGtV1
        ) {
            egui::ComboBox::from_label("COCO crowd objects")
                .selected_text(format!("{:?}", self.import_flow.coco_crowds))
                .show_ui(ui, |ui| {
                    for policy in [
                        labello_client::CocoCrowdPolicy::Block,
                        labello_client::CocoCrowdPolicy::Incomplete,
                        labello_client::CocoCrowdPolicy::ExcludeImageTask,
                    ] {
                        ui.selectable_value(
                            &mut self.import_flow.coco_crowds,
                            policy,
                            format!("{policy:?}"),
                        );
                    }
                });
            show_mapping_issues(
                ui,
                &validation,
                None,
                ImportMappingField::Compatibility(ImportCompatibilityField::CocoCrowds),
            );
            egui::ComboBox::from_label("COCO structure")
                .selected_text(format!("{:?}", self.import_flow.coco_structure))
                .show_ui(ui, |ui| {
                    for policy in [
                        labello_client::CocoStructurePolicy::Canonical,
                        labello_client::CocoStructurePolicy::BboxCompatibility,
                    ] {
                        ui.selectable_value(
                            &mut self.import_flow.coco_structure,
                            policy,
                            format!("{policy:?}"),
                        );
                    }
                });
            show_mapping_issues(
                ui,
                &validation,
                None,
                ImportMappingField::Compatibility(ImportCompatibilityField::CocoStructure),
            );
        }
        egui::ComboBox::from_label("Out-of-bounds geometry")
            .selected_text(format!("{:?}", self.import_flow.geometry_bounds))
            .show_ui(ui, |ui| {
                for policy in [
                    labello_client::GeometryBoundsPolicy::Reject,
                    labello_client::GeometryBoundsPolicy::Clip,
                ] {
                    ui.selectable_value(
                        &mut self.import_flow.geometry_bounds,
                        policy,
                        format!("{policy:?}"),
                    );
                }
            });
        show_mapping_issues(
            ui,
            &validation,
            None,
            ImportMappingField::Compatibility(ImportCompatibilityField::GeometryBounds),
        );
        egui::ComboBox::from_label("Cross-split duplicates")
            .selected_text(format!("{:?}", self.import_flow.cross_split_duplicates))
            .show_ui(ui, |ui| {
                for policy in [
                    labello_client::CrossSplitDuplicatePolicy::Block,
                    labello_client::CrossSplitDuplicatePolicy::MergeMemberships,
                ] {
                    ui.selectable_value(
                        &mut self.import_flow.cross_split_duplicates,
                        policy,
                        format!("{policy:?}"),
                    );
                }
            });
        show_mapping_issues(
            ui,
            &validation,
            None,
            ImportMappingField::Compatibility(ImportCompatibilityField::CrossSplitDuplicates),
        );
    }

}
