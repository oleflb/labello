fn validate_mappings(ir: &ImportIr, request: &PreflightRequest, diagnostics: &mut Diagnostics) {
    let covered = request.coverage_scope.iter().collect::<BTreeSet<_>>();
    let authoritative_categories = if request.task_mappings.is_empty() {
        let intent_is_authoritative = request.intent == ImportIntent::AuthoritativeGroundTruth;
        planned_ids(ir, request)
            .0
            .into_keys()
            .filter(|_| intent_is_authoritative)
            .collect::<BTreeSet<_>>()
    } else {
        request
            .task_mappings
            .iter()
            .filter(|mapping| mapping.intent == ImportIntent::AuthoritativeGroundTruth)
            .map(|mapping| mapping.source_category_key.clone())
            .collect()
    };
    for _ in authoritative_categories.iter().filter(|key| {
        !request.exhaustive_attested || !ir.categories.contains_key(*key) || !covered.contains(*key)
    }) {
        diagnostics.add(
            "authoritative_coverage_invalid",
            DiagnosticSeverity::Error,
            "authoritative task mappings require exhaustive coverage for their exact source category keys",
            true,
            false,
            true,
            None,
        );
    }
    if request.category_mappings.is_empty() && request.task_mappings.is_empty() {
        return;
    }
    let selected = request
        .category_mappings
        .iter()
        .filter(|mapping| mapping.selected)
        .collect::<Vec<_>>();
    let category_keys = selected
        .iter()
        .map(|mapping| mapping.source_category_key.as_str())
        .collect::<BTreeSet<_>>();
    let class_ids = selected
        .iter()
        .map(|mapping| mapping.class_id.as_str())
        .collect::<BTreeSet<_>>();
    let invalid_categories = category_keys.len() != selected.len()
        || class_ids.len() != selected.len()
        || selected.iter().any(|mapping| {
            !ir.categories.contains_key(&mapping.source_category_key)
                || mapping.class_name.trim().is_empty()
                || mapping.color.trim().is_empty()
        });
    let task_ids = request
        .task_mappings
        .iter()
        .map(|mapping| mapping.task.task_id.as_str())
        .collect::<BTreeSet<_>>();
    let invalid_tasks = task_ids.len() != request.task_mappings.len()
        || request.task_mappings.iter().any(|mapping| {
            let mapped_class = selected
                .iter()
                .find(|category| category.source_category_key == mapping.source_category_key)
                .map(|category| &category.class_id);
            !category_keys.contains(mapping.source_category_key.as_str())
                || mapping.task.task_id.validate_path_segment().is_err()
                || mapping.task.name.trim().is_empty()
                || mapping.task.class_ids.is_empty()
                || mapped_class.is_none_or(|class_id| mapping.task.class_ids != [class_id.clone()])
        });
    let skeleton_mappings = request
        .task_mappings
        .iter()
        .filter(|mapping| mapping.task.annotation_type == labello_domain::AnnotationType::Skeleton)
        .collect::<Vec<_>>();
    let manual_invalid = matches!(
        request.output.box_to_skeleton,
        BoxToSkeletonPolicy::ManualBoxGuide { .. }
    ) && request.geometry_mappings.is_empty()
        && (skeleton_mappings.is_empty()
            || skeleton_mappings.iter().any(|mapping| {
                let Some(config) = &mapping.task.manual_box_guide_migration else {
                    return true;
                };
                request
                    .task_mappings
                    .iter()
                    .find(|candidate| candidate.task.task_id == config.guide_task_id)
                    .is_none_or(|guide| {
                        mapping.task.validate_manual_migration(&guide.task).is_err()
                    })
            }));
    let geometry_targets = request
        .geometry_mappings
        .iter()
        .map(|mapping| (&mapping.source_category_key, mapping.target_geometry))
        .collect::<BTreeSet<_>>();
    let invalid_geometry = !request.geometry_mappings.is_empty()
        && (geometry_targets.len() != request.geometry_mappings.len()
            || request.geometry_mappings.iter().any(|mapping| {
                !category_keys.contains(mapping.source_category_key.as_str())
                    || !valid_geometry_policy(mapping, request, ir)
                    || (!matches!(mapping.policy, ImportGeometryPolicy::Omit)
                        && !request.task_mappings.iter().any(|task| {
                            task.source_category_key == mapping.source_category_key
                                && geometry_kind_for_annotation(&task.task.annotation_type)
                                    == mapping.target_geometry
                        }))
            })
            || request.task_mappings.iter().any(|task| {
                !geometry_targets.contains(&(
                    &task.source_category_key,
                    geometry_kind_for_annotation(&task.task.annotation_type),
                ))
            }));
    if invalid_categories || invalid_tasks || manual_invalid {
        diagnostics.add(
            "manual_mapping_invalid",
            DiagnosticSeverity::Error,
            "manual category and task mappings must be unique, selected, and valid",
            true,
            false,
            false,
            None,
        );
    }
    if invalid_geometry {
        diagnostics.add(
            "geometry_mapping_invalid",
            DiagnosticSeverity::Error,
            "geometry mappings must use valid source, target, schema, and versioned policy parameters",
            true,
            false,
            true,
            None,
        );
    }
}

pub(super) fn planned_ids(
    ir: &ImportIr,
    request: &PreflightRequest,
) -> (BTreeMap<String, String>, BTreeMap<String, Vec<String>>) {
    if !request.category_mappings.is_empty() || !request.task_mappings.is_empty() {
        let classes = request
            .category_mappings
            .iter()
            .filter(|mapping| {
                mapping.selected && ir.categories.contains_key(&mapping.source_category_key)
            })
            .map(|mapping| {
                (
                    mapping.source_category_key.clone(),
                    mapping.class_id.to_string(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut tasks = classes
            .keys()
            .map(|key| (key.clone(), Vec::new()))
            .collect::<BTreeMap<_, _>>();
        for mapping in &request.task_mappings {
            if let Some(values) = tasks.get_mut(&mapping.source_category_key) {
                values.push(mapping.task.task_id.to_string());
            }
        }
        for values in tasks.values_mut() {
            values.sort();
            values.dedup();
        }
        return (classes, tasks);
    }
    let mut used = BTreeSet::new();
    let mut classes = BTreeMap::new();
    let mut tasks = BTreeMap::new();
    for (key, category) in &ir.categories {
        let base = slug(&category.name);
        let mut class_id = if base.is_empty() {
            "class".to_string()
        } else {
            base
        };
        if used.contains(&class_id) {
            class_id = format!(
                "{}-{}",
                class_id,
                &blake3::hash(key.as_bytes()).to_hex()[..8]
            );
        }
        used.insert(class_id.clone());
        classes.insert(key.clone(), class_id.clone());
        let mut values = Vec::new();
        if request.output.bounding_boxes {
            values.push(format!("bounding_box:{class_id}"));
        }
        let direct_skeleton = !category.keypoint_names.is_empty();
        let box_migration = !matches!(request.output.box_to_skeleton, BoxToSkeletonPolicy::None);
        if request.output.skeletons && (direct_skeleton || box_migration) {
            values.push(format!("skeleton:{class_id}"));
        }
        tasks.insert(key.clone(), values);
    }
    (classes, tasks)
}
