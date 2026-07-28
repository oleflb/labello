impl LabelloApp {
    #[cfg(test)]
    fn import_descriptors_valid(&self) -> bool {
        self.import_descriptor_error().is_none()
    }

    fn import_descriptor_error(&self) -> Option<String> {
        let coco = is_coco_profile(self.import.profile);
        if !valid_identity_component(&self.import.source_namespace) {
            return Some(
                "Source namespace must use only letters, numbers, '.', '_', or '-'.".to_string(),
            );
        }
        let reference_valid =
            |reference: &str| {
                !reference.trim().is_empty()
                    && (self.import.transport == ImportTransport::ServerDirectory
                        || self.import.registered_paths.iter().any(|path| {
                            path.file_id == reference || path.client_file_id == reference
                        }))
            };
        if !coco {
            let Some(descriptor) = self.import.descriptors.first() else {
                return Some("Select one Dataset YAML.".to_string());
            };
            if self.import.descriptors.len() != 1
                || descriptor.kind != ImportDescriptorKind::YoloDataset
            {
                return Some("YOLO imports require exactly one Dataset YAML.".to_string());
            }
            if !reference_valid(&descriptor.descriptor_file_id) {
                return Some("Select a registered Dataset YAML.".to_string());
            }
            if !valid_identity_component(&descriptor.release) {
                return Some(
                    "Release must use only letters, numbers, '.', '_', or '-'.".to_string(),
                );
            }
            if self.import.yolo_inspection_loading {
                return Some("Wait for YAML split inspection to finish.".to_string());
            }
            if let Some(error) = &self.import.yolo_inspection_error {
                return Some(error.clone());
            }
            if self
                .import
                .yolo_inspected_descriptor_file_id
                .as_deref()
                .map(str::trim)
                != Some(descriptor.descriptor_file_id.trim())
            {
                return Some("Inspect the selected YAML before sealing the source.".to_string());
            }
            if !self
                .import
                .yolo_splits
                .iter()
                .any(|split| split.usable && split.selected)
            {
                return Some("Select at least one usable YAML split.".to_string());
            }
            return None;
        }
        let mut descriptor_references = std::collections::BTreeSet::new();
        let mut descriptor_identities = std::collections::BTreeSet::new();
        let valid = !self.import.descriptors.is_empty()
            && self.import.descriptors.iter().all(|descriptor| {
                descriptor_kind_allowed(self.import.profile, descriptor.kind)
                    && reference_valid(&descriptor.descriptor_file_id)
                    && descriptor_references.insert(descriptor.descriptor_file_id.trim())
                    && valid_identity_component(&descriptor.release)
                    && valid_identity_component(&descriptor.split)
                    && (descriptor.pairing_group.trim().is_empty()
                        || valid_identity_component(&descriptor.pairing_group))
                    && descriptor_identities.insert((
                        descriptor_kind_label(descriptor.kind),
                        descriptor.release.trim(),
                        descriptor.split.trim(),
                        descriptor.pairing_group.trim(),
                    ))
                    && (!coco || reference_valid(&descriptor.image_root_file_id))
            })
            && valid_identity_component(&self.import.source_namespace);
        (!valid).then(|| {
            "Every COCO descriptor needs a unique registered JSON file, valid release and split, and an exact registered image root."
                .to_string()
        })
    }

    fn import_mapping_validation(&self) -> ImportMappingValidation {
        let mut validation = ImportMappingValidation::default();
        let discovered = self
            .import
            .job
            .as_ref()
            .and_then(|job| job.preflight_report.as_ref())
            .map(|report| report.source.categories as usize)
            .or_else(|| {
                self.import
                    .plan
                    .as_ref()
                    .map(|plan| plan.report.source.categories as usize)
            })
            .unwrap_or(0);
        if discovered == 0 {
            push_mapping_issue(
                &mut validation,
                ImportMappingIssueSeverity::Error,
                None,
                ImportMappingField::Form,
                "Preflight has not reported any source categories to map.",
            );
        } else if self.import.categories.len() != discovered {
            push_mapping_issue(
                &mut validation,
                ImportMappingIssueSeverity::Error,
                None,
                ImportMappingField::Form,
                format!(
                    "Expected {discovered} source categories, but the mapping contract contains {}.",
                    self.import.categories.len()
                ),
            );
        }

        let selected_indices = self
            .import
            .categories
            .iter()
            .enumerate()
            .filter_map(|(index, category)| category.selected.then_some(index))
            .collect::<Vec<_>>();
        if selected_indices.is_empty() {
            push_mapping_issue(
                &mut validation,
                ImportMappingIssueSeverity::Error,
                None,
                ImportMappingField::CategorySelection,
                "Include at least one source category.",
            );
        }

        let mut source_key_owners = std::collections::BTreeMap::<String, Vec<usize>>::new();
        let mut class_id_owners = std::collections::BTreeMap::<String, Vec<usize>>::new();
        let mut task_id_owners =
            std::collections::BTreeMap::<String, Vec<(usize, ImportMappingField)>>::new();
        let mut generated_tasks = 0_usize;
        let manual_available = self
            .import
            .capabilities
            .as_ref()
            .is_some_and(|capabilities| capabilities.manual_box_guide_migration);
        let max_keypoints = self
            .import
            .capabilities
            .as_ref()
            .map(|capabilities| capabilities.limits.max_keypoints_per_skeleton as usize)
            .unwrap_or(usize::MAX);

        for (index, category) in self.import.categories.iter().enumerate() {
            source_key_owners
                .entry(category.source_category_key.trim().to_string())
                .or_default()
                .push(index);
            if category.source_category_key.trim().is_empty()
                || category.source_category_id.trim().is_empty()
            {
                push_mapping_issue(
                    &mut validation,
                    ImportMappingIssueSeverity::Error,
                    Some(index),
                    ImportMappingField::Form,
                    "The server mapping contract is missing this category's source key or ID.",
                );
            }
            if ClassId::from(category.class_id.trim())
                .validate_path_segment()
                .is_err()
            {
                push_mapping_issue(
                    &mut validation,
                    ImportMappingIssueSeverity::Error,
                    Some(index),
                    ImportMappingField::ClassId,
                    "Class ID must be a non-empty safe path segment of at most 255 bytes.",
                );
            }
            if category.class_name.trim().is_empty() {
                push_mapping_issue(
                    &mut validation,
                    ImportMappingIssueSeverity::Error,
                    Some(index),
                    ImportMappingField::ClassName,
                    "Class name cannot be empty.",
                );
            }
            if !valid_color(category.class_color.trim()) {
                push_mapping_issue(
                    &mut validation,
                    ImportMappingIssueSeverity::Error,
                    Some(index),
                    ImportMappingField::ClassColor,
                    "Class color must use #RRGGBB hexadecimal format.",
                );
            }
            if category.selected {
                class_id_owners
                    .entry(category.class_id.trim().to_string())
                    .or_default()
                    .push(index);
            }

            if !category.selected {
                push_mapping_issue(
                    &mut validation,
                    ImportMappingIssueSeverity::Warning,
                    Some(index),
                    ImportMappingField::CategorySelection,
                    "This source category and its annotations will be excluded from the import.",
                );
                continue;
            }

            let mut targets = std::collections::BTreeSet::new();
            let active_targets = category
                .geometry_mappings
                .iter()
                .filter(|mapping| mapping.policy != ImportGeometryPolicy::Omit)
                .map(|mapping| mapping.target_geometry)
                .collect::<std::collections::BTreeSet<_>>();
            if active_targets.is_empty() {
                push_mapping_issue(
                    &mut validation,
                    ImportMappingIssueSeverity::Error,
                    Some(index),
                    ImportMappingField::CategorySelection,
                    "An included category must produce at least one annotation output.",
                );
            }

            for mapping in &category.geometry_mappings {
                let field = ImportMappingField::Geometry(mapping.target_geometry);
                if !targets.insert(mapping.target_geometry) {
                    push_mapping_issue(
                        &mut validation,
                        ImportMappingIssueSeverity::Error,
                        Some(index),
                        field,
                        "Each category can define a target output only once.",
                    );
                }
                if mapping.policy != ImportGeometryPolicy::Omit
                    && !category.direct_geometry.contains(&mapping.source_geometry)
                {
                    push_mapping_issue(
                        &mut validation,
                        ImportMappingIssueSeverity::Error,
                        Some(index),
                        field,
                        "The selected source geometry was not discovered for this category.",
                    );
                }
                if !geometry_choices_for_target(
                    mapping.target_geometry,
                    &category.direct_geometry,
                    manual_available,
                    category.source_skeleton.is_some(),
                )
                .iter()
                .any(|choice| {
                    choice.policy == mapping.policy
                        && (mapping.policy == ImportGeometryPolicy::Omit
                            || choice.source == mapping.source_geometry)
                }) {
                    push_mapping_issue(
                        &mut validation,
                        ImportMappingIssueSeverity::Error,
                        Some(index),
                        field,
                        "Choose a supported source-to-output transformation.",
                    );
                }
                let target_skeleton = category.source_skeleton.as_ref().cloned().or_else(|| {
                    let names = split_csv(&category.target_keypoint_names);
                    (!names.is_empty()).then(|| SkeletonSpec {
                        keypoints: names
                            .into_iter()
                            .map(|name| labello_domain::KeypointSpec {
                                name,
                                required: false,
                            })
                            .collect(),
                        edges: Vec::new(),
                        allow_hidden: true,
                        allow_absent: true,
                    })
                });
                for message in mapping_parameter_errors(mapping, target_skeleton.as_ref()) {
                    push_mapping_issue(
                        &mut validation,
                        ImportMappingIssueSeverity::Error,
                        Some(index),
                        field,
                        message,
                    );
                }
                match mapping.policy {
                    ImportGeometryPolicy::KeypointEnvelopeV1 => push_mapping_issue(
                        &mut validation,
                        ImportMappingIssueSeverity::Warning,
                        Some(index),
                        field,
                        "Derived envelopes are pending seeds and require acknowledgement after preflight.",
                    ),
                    ImportGeometryPolicy::BoxRelativeTemplateV1 => push_mapping_issue(
                        &mut validation,
                        ImportMappingIssueSeverity::Warning,
                        Some(index),
                        field,
                        "Template skeletons are derived pending seeds and require acknowledgement after preflight.",
                    ),
                    ImportGeometryPolicy::ManualBoxGuideV1 => {
                        if !manual_available {
                            push_mapping_issue(
                                &mut validation,
                                ImportMappingIssueSeverity::Error,
                                Some(index),
                                field,
                                "Manual box-guide migration is not available on this server.",
                            );
                        }
                        if !self.import.exhaustive {
                            push_mapping_issue(
                                &mut validation,
                                ImportMappingIssueSeverity::Error,
                                Some(index),
                                field,
                                "Manual box-guide migration requires an exhaustive source attestation.",
                            );
                        }
                        if category.source_skeleton.is_some()
                            || category
                                .direct_geometry
                                .contains(&ImportGeometryKind::Skeleton)
                        {
                            push_mapping_issue(
                                &mut validation,
                                ImportMappingIssueSeverity::Error,
                                Some(index),
                                field,
                                "Manual box-guide migration only supports categories without source skeleton geometry.",
                            );
                        }
                        if !active_targets.contains(&ImportGeometryKind::BoundingBox) {
                            push_mapping_issue(
                                &mut validation,
                                ImportMappingIssueSeverity::Error,
                                Some(index),
                                field,
                                "Manual migration requires a direct bounding-box guide output.",
                            );
                        }
                    }
                    ImportGeometryPolicy::Omit => push_mapping_issue(
                        &mut validation,
                        ImportMappingIssueSeverity::Warning,
                        Some(index),
                        field,
                        "This annotation output will not be imported for the category.",
                    ),
                    ImportGeometryPolicy::Direct => {}
                }
            }

            let bounding_boxes = active_targets.contains(&ImportGeometryKind::BoundingBox);
            let skeletons = active_targets.contains(&ImportGeometryKind::Skeleton);
            if bounding_boxes {
                generated_tasks += 1;
                if TaskId::from(category.bounding_box_task_id.trim())
                    .validate_path_segment()
                    .is_err()
                {
                    push_mapping_issue(
                        &mut validation,
                        ImportMappingIssueSeverity::Error,
                        Some(index),
                        ImportMappingField::BoundingBoxTaskId,
                        "Bounding-box task ID must be a non-empty safe path segment of at most 255 bytes.",
                    );
                }
                if category.bounding_box_task_name.trim().is_empty() {
                    push_mapping_issue(
                        &mut validation,
                        ImportMappingIssueSeverity::Error,
                        Some(index),
                        ImportMappingField::BoundingBoxTaskName,
                        "Bounding-box task name cannot be empty.",
                    );
                }
                task_id_owners
                    .entry(category.bounding_box_task_id.trim().to_string())
                    .or_default()
                    .push((index, ImportMappingField::BoundingBoxTaskId));
            }
            if skeletons {
                generated_tasks += 1;
                if TaskId::from(category.skeleton_task_id.trim())
                    .validate_path_segment()
                    .is_err()
                {
                    push_mapping_issue(
                        &mut validation,
                        ImportMappingIssueSeverity::Error,
                        Some(index),
                        ImportMappingField::SkeletonTaskId,
                        "Skeleton task ID must be a non-empty safe path segment of at most 255 bytes.",
                    );
                }
                if category.skeleton_task_name.trim().is_empty() {
                    push_mapping_issue(
                        &mut validation,
                        ImportMappingIssueSeverity::Error,
                        Some(index),
                        ImportMappingField::SkeletonTaskName,
                        "Skeleton task name cannot be empty.",
                    );
                }
                task_id_owners
                    .entry(category.skeleton_task_id.trim().to_string())
                    .or_default()
                    .push((index, ImportMappingField::SkeletonTaskId));
                let target_names = category.source_skeleton.as_ref().map_or_else(
                    || split_csv(&category.target_keypoint_names),
                    |skeleton| {
                        skeleton
                            .keypoints
                            .iter()
                            .map(|point| point.name.clone())
                            .collect()
                    },
                );
                let unique = target_names
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>();
                if target_names.is_empty() {
                    push_mapping_issue(
                        &mut validation,
                        ImportMappingIssueSeverity::Error,
                        Some(index),
                        ImportMappingField::TargetKeypointNames,
                        "Skeleton output requires at least one target keypoint.",
                    );
                } else if unique.len() != target_names.len() {
                    push_mapping_issue(
                        &mut validation,
                        ImportMappingIssueSeverity::Error,
                        Some(index),
                        ImportMappingField::TargetKeypointNames,
                        "Target keypoint names must be unique.",
                    );
                }
                if target_names.len() > max_keypoints {
                    push_mapping_issue(
                        &mut validation,
                        ImportMappingIssueSeverity::Error,
                        Some(index),
                        ImportMappingField::TargetKeypointNames,
                        format!(
                            "Target skeleton has {} keypoints; the server limit is {max_keypoints}.",
                            target_names.len()
                        ),
                    );
                }
            }

            if category.workflow_intent == ImportWorkflowIntent::AuthoritativeGroundTruth
                && !self.import.exhaustive
            {
                push_mapping_issue(
                    &mut validation,
                    ImportMappingIssueSeverity::Error,
                    Some(index),
                    ImportMappingField::WorkflowIntent,
                    "Authoritative ground truth requires an exhaustive source attestation.",
                );
            }
            if category.workflow_intent == ImportWorkflowIntent::SeedFutureAnnotation {
                push_mapping_issue(
                    &mut validation,
                    ImportMappingIssueSeverity::Warning,
                    Some(index),
                    ImportMappingField::WorkflowIntent,
                    "Imported geometry remains pending for future human annotation.",
                );
            } else if category.workflow_intent == ImportWorkflowIntent::RequireApproval {
                push_mapping_issue(
                    &mut validation,
                    ImportMappingIssueSeverity::Warning,
                    Some(index),
                    ImportMappingField::WorkflowIntent,
                    "Imported annotations require one human approval before completion.",
                );
            }
        }

        for owners in source_key_owners.values().filter(|owners| owners.len() > 1) {
            for index in owners {
                push_mapping_issue(
                    &mut validation,
                    ImportMappingIssueSeverity::Error,
                    Some(*index),
                    ImportMappingField::Form,
                    "The server mapping contract contains a duplicate source category key.",
                );
            }
        }
        for owners in class_id_owners.values().filter(|owners| owners.len() > 1) {
            for index in owners {
                push_mapping_issue(
                    &mut validation,
                    ImportMappingIssueSeverity::Error,
                    Some(*index),
                    ImportMappingField::ClassId,
                    "Included categories must use unique class IDs.",
                );
            }
        }
        for owners in task_id_owners.values().filter(|owners| owners.len() > 1) {
            for (index, field) in owners {
                push_mapping_issue(
                    &mut validation,
                    ImportMappingIssueSeverity::Error,
                    Some(*index),
                    *field,
                    "Every generated task must use a unique task ID.",
                );
            }
        }
        if let Some(limit) = self
            .import
            .capabilities
            .as_ref()
            .map(|capabilities| capabilities.limits.max_generated_tasks as usize)
            && generated_tasks > limit
        {
            push_mapping_issue(
                &mut validation,
                ImportMappingIssueSeverity::Error,
                None,
                ImportMappingField::Form,
                format!(
                    "The mapping generates {generated_tasks} tasks; the server limit is {limit}."
                ),
            );
        }

        let has_seed_workflow = self.import.categories.iter().any(|category| {
            category.selected
                && category.workflow_intent == ImportWorkflowIntent::SeedFutureAnnotation
        });
        if has_seed_workflow && !self.import.seed_workflow_confirmed {
            push_mapping_issue(
                &mut validation,
                ImportMappingIssueSeverity::Error,
                None,
                ImportMappingField::SeedConfirmation,
                "Confirm the selected pending seed workflows before saving.",
            );
        }

        self.add_compatibility_policy_issues(&mut validation);
        validation
    }

    fn add_compatibility_policy_issues(&self, validation: &mut ImportMappingValidation) {
        let profile = self.import.profile;
        if matches!(
            profile,
            ImportProfile::UltralyticsYoloDetectV1 | ImportProfile::UltralyticsYoloPoseV1
        ) {
            match self.import.yolo_missing_labels {
                labello_client::YoloMissingLabelPolicy::Incomplete => push_mapping_issue(
                    validation,
                    ImportMappingIssueSeverity::Warning,
                    None,
                    ImportMappingField::Compatibility(ImportCompatibilityField::YoloMissingLabels),
                    "If a label file is missing, coverage remains incomplete.",
                ),
                labello_client::YoloMissingLabelPolicy::MissingIsBackground => push_mapping_issue(
                    validation,
                    ImportMappingIssueSeverity::Warning,
                    None,
                    ImportMappingField::Compatibility(ImportCompatibilityField::YoloMissingLabels),
                    "Missing label files will be treated as verified background and require acknowledgement when encountered.",
                ),
                labello_client::YoloMissingLabelPolicy::Block => {}
            }
            if self.import.yolo_duplicate_rows
                == labello_client::YoloDuplicateRowPolicy::Deduplicate
            {
                push_mapping_issue(
                    validation,
                    ImportMappingIssueSeverity::Warning,
                    None,
                    ImportMappingField::Compatibility(ImportCompatibilityField::YoloDuplicateRows),
                    "Exact duplicate YOLO rows will be discarded and require acknowledgement when encountered.",
                );
            }
            if profile == ImportProfile::UltralyticsYoloPoseV1
                && self.import.missing_keypoint_names
                    == labello_client::MissingKeypointNamesPolicy::GenerateIndexed
            {
                push_mapping_issue(
                    validation,
                    ImportMappingIssueSeverity::Warning,
                    None,
                    ImportMappingField::Compatibility(
                        ImportCompatibilityField::MissingKeypointNames,
                    ),
                    "Missing keypoint names will be generated by index without inferred edges.",
                );
            }
        }
        if matches!(
            profile,
            ImportProfile::CocoInstancesGtV1 | ImportProfile::CocoKeypointsGtV1
        ) {
            match self.import.coco_crowds {
                labello_client::CocoCrowdPolicy::Incomplete => push_mapping_issue(
                    validation,
                    ImportMappingIssueSeverity::Warning,
                    None,
                    ImportMappingField::Compatibility(ImportCompatibilityField::CocoCrowds),
                    "Crowd objects will be skipped and leave coverage incomplete.",
                ),
                labello_client::CocoCrowdPolicy::ExcludeImageTask => push_mapping_issue(
                    validation,
                    ImportMappingIssueSeverity::Warning,
                    None,
                    ImportMappingField::Compatibility(ImportCompatibilityField::CocoCrowds),
                    "Crowd objects will exclude the affected image-task pair.",
                ),
                labello_client::CocoCrowdPolicy::Block => {}
            }
            if self.import.coco_structure
                == labello_client::CocoStructurePolicy::BboxCompatibility
            {
                push_mapping_issue(
                    validation,
                    ImportMappingIssueSeverity::Warning,
                    None,
                    ImportMappingField::Compatibility(ImportCompatibilityField::CocoStructure),
                    "BBox compatibility may synthesize noncanonical COCO fields and always requires acknowledgement.",
                );
            }
        }
        if self.import.geometry_bounds == labello_client::GeometryBoundsPolicy::Clip {
            push_mapping_issue(
                validation,
                ImportMappingIssueSeverity::Warning,
                None,
                ImportMappingField::Compatibility(ImportCompatibilityField::GeometryBounds),
                "Out-of-bounds geometry will be clipped as derived pending data and requires acknowledgement.",
            );
        }
        if self.import.cross_split_duplicates
            == labello_client::CrossSplitDuplicatePolicy::MergeMemberships
        {
            push_mapping_issue(
                validation,
                ImportMappingIssueSeverity::Warning,
                None,
                ImportMappingField::Compatibility(ImportCompatibilityField::CrossSplitDuplicates),
                "Duplicate images will retain multiple split memberships and require acknowledgement when encountered.",
            );
        }
    }

    fn import_mappings_complete(&self) -> bool {
        self.import_mapping_validation().is_valid()
    }

    fn import_plan_is_current(&self) -> bool {
        let Some(plan) = self.import.plan.as_ref() else {
            return false;
        };
        self.import
            .accepted_plan_request
            .as_ref()
            .or(plan.accepted_request.as_ref())
            .is_some_and(|accepted| accepted == &self.import_plan_request())
    }

    fn import_plan_covers_all_categories(&self) -> bool {
        let Some(plan) = self.import.plan.as_ref() else {
            return false;
        };
        let (selected, required_tasks) = self.import_required_output_counts();
        selected > 0
            && plan.report.output.classes == selected
            && plan.report.output.tasks >= required_tasks
    }

    fn import_required_output_counts(&self) -> (u64, u64) {
        let selected = self
            .import
            .categories
            .iter()
            .filter(|category| category.selected)
            .count() as u64;
        let required_tasks = self
            .import
            .categories
            .iter()
            .filter(|category| category.selected)
            .map(|category| {
                category
                    .geometry_mappings
                    .iter()
                    .filter(|mapping| mapping.policy != ImportGeometryPolicy::Omit)
                    .map(|mapping| mapping.target_geometry)
                    .collect::<std::collections::BTreeSet<_>>()
                    .len() as u64
            })
            .sum();
        (selected, required_tasks)
    }
}
