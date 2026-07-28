fn convert_preflight(
    job: &storage::ImportJob,
    control: &JobControl,
    seal: &client::SealImportRequest,
) -> ApiResult<storage::PreflightRequest> {
    if !seal.source.selected_category_keys.is_empty() {
        return Err(ApiError::Unprocessable(
            "selectedCategoryKeys is not supported; select categories in the import plan"
                .to_string(),
        ));
    }
    if seal.attestations != control.create_request.attestations {
        return Err(ApiError::Unprocessable(
            "seal attestations must match the import creation attestations".to_string(),
        ));
    }
    validate_identity_component(&seal.source.source_namespace, "source namespace")?;
    if seal.source.selected_splits.is_empty()
        || seal
            .source
            .selected_splits
            .iter()
            .any(|split| validate_identity_component(split, "selected split").is_err())
        || seal
            .source
            .selected_splits
            .iter()
            .collect::<BTreeSet<_>>()
            .len()
            != seal.source.selected_splits.len()
    {
        return Err(ApiError::Unprocessable(
            "selected splits must be unique nonempty identity components".to_string(),
        ));
    }
    let mut descriptor_paths = Vec::new();
    let mut coco_descriptors = Vec::new();
    let mut descriptor_identities = BTreeSet::new();
    for descriptor in &seal.source.descriptors {
        let path = resolve_source_reference(control, &descriptor.descriptor_file_id)?;
        validate_identity_component(&descriptor.release, "descriptor release")?;
        validate_identity_component(&descriptor.split, "descriptor split")?;
        if !seal.source.selected_splits.contains(&descriptor.split) {
            return Err(ApiError::Unprocessable(
                "every descriptor split must be selected".to_string(),
            ));
        }
        if descriptor
            .pairing_group
            .as_deref()
            .is_some_and(|value| validate_identity_component(value, "pairing group").is_err())
        {
            return Err(ApiError::Unprocessable(
                "pairing group must be a nonempty identity component".to_string(),
            ));
        }
        if !descriptor_identities.insert((
            path.clone(),
            descriptor_kind_name(descriptor.kind),
            descriptor.release.clone(),
            descriptor.split.clone(),
            descriptor.pairing_group.clone(),
        )) {
            return Err(ApiError::Unprocessable(
                "descriptor identities must be unique".to_string(),
            ));
        }
        let kind_allowed = match job.profile {
            storage::ImportProfile::UltralyticsYoloDetectV1
            | storage::ImportProfile::UltralyticsYoloPoseV1 => {
                descriptor.kind == client::ImportDescriptorKind::YoloDataset
            }
            storage::ImportProfile::CocoInstancesGtV1 => {
                descriptor.kind == client::ImportDescriptorKind::CocoInstances
            }
            storage::ImportProfile::CocoKeypointsGtV1 => matches!(
                descriptor.kind,
                client::ImportDescriptorKind::CocoInstances
                    | client::ImportDescriptorKind::CocoKeypoints
            ),
        };
        if !kind_allowed {
            return Err(ApiError::Unprocessable(
                "descriptor kind does not match the selected import profile".to_string(),
            ));
        }
        match descriptor.kind {
            client::ImportDescriptorKind::YoloDataset => {
                if descriptor.image_root_file_id.is_some() || descriptor.pairing_group.is_some() {
                    return Err(ApiError::Unprocessable(
                        "YOLO descriptors do not support image-root or pairing inputs".to_string(),
                    ));
                }
                descriptor_paths.push(path);
            }
            client::ImportDescriptorKind::CocoInstances
            | client::ImportDescriptorKind::CocoKeypoints => {
                let image_reference =
                    descriptor.image_root_file_id.as_deref().ok_or_else(|| {
                        ApiError::Unprocessable(
                            "COCO descriptors require an explicit registered image-root reference"
                                .to_string(),
                        )
                    })?;
                let image_path = resolve_source_reference(control, image_reference)?;
                let image_root = Path::new(&image_path)
                    .parent()
                    .and_then(Path::to_str)
                    .filter(|parent| !parent.is_empty())
                    .unwrap_or(&image_path)
                    .replace('\\', "/");
                coco_descriptors.push(storage::CocoDescriptorSelection {
                    kind: match descriptor.kind {
                        client::ImportDescriptorKind::CocoInstances => {
                            labello_domain::ImportDescriptorKind::CocoInstances
                        }
                        client::ImportDescriptorKind::CocoKeypoints => {
                            labello_domain::ImportDescriptorKind::CocoKeypoints
                        }
                        client::ImportDescriptorKind::YoloDataset => unreachable!(),
                    },
                    descriptor_path: path,
                    image_root,
                    split: descriptor.split.clone(),
                    source_namespace: seal.source.source_namespace.clone(),
                    release: descriptor.release.clone(),
                    pairing_group: descriptor.pairing_group.clone(),
                });
            }
        }
    }
    if matches!(
        job.profile,
        storage::ImportProfile::UltralyticsYoloDetectV1
            | storage::ImportProfile::UltralyticsYoloPoseV1
    ) && descriptor_paths.len() != 1
    {
        return Err(ApiError::Unprocessable(
            "YOLO imports require exactly one descriptor".to_string(),
        ));
    }
    let source_release = seal
        .source
        .descriptors
        .first()
        .map(|descriptor| descriptor.release.clone())
        .unwrap_or_default();
    Ok(storage::PreflightRequest {
        descriptor_paths,
        selected_splits: seal.source.selected_splits.clone(),
        coco_descriptors,
        ground_truth_attested: seal.attestations.ground_truth,
        exhaustive_attested: seal.attestations.exhaustive,
        source_namespace: seal.source.source_namespace.clone(),
        source_release,
        coverage_scope: seal.attestations.coverage_scope.clone(),
        attestation_provenance: seal.attestations.provenance.clone(),
        intent: if seal.attestations.exhaustive {
            storage::ImportIntent::AuthoritativeGroundTruth
        } else {
            storage::ImportIntent::RequireApproval
        },
        policies: storage::CompatibilityPolicies::default(),
        output: storage::OutputPolicy::defaults_for(job.profile),
        acknowledged_warning_codes: Vec::new(),
        category_mappings: Vec::new(),
        task_mappings: Vec::new(),
        geometry_mappings: Vec::new(),
    })
}

fn convert_plan_update(
    mut current: storage::PreflightRequest,
    request: client::UpdateImportPlanRequest,
) -> ApiResult<storage::PreflightRequest> {
    if request.category_mappings.is_empty() || request.task_mappings.is_empty() {
        return Err(ApiError::Unprocessable(
            "at least one category and task mapping is required".to_string(),
        ));
    }
    let category_keys = request
        .category_mappings
        .iter()
        .map(|mapping| mapping.source_category_key.clone())
        .collect::<BTreeSet<_>>();
    let selected = request
        .category_mappings
        .iter()
        .filter(|mapping| mapping.selected)
        .map(|mapping| mapping.source_category_key.clone())
        .collect::<BTreeSet<_>>();
    if selected.is_empty() || category_keys.len() != request.category_mappings.len() {
        return Err(ApiError::Unprocessable(
            "source category keys must be unique and at least one must be selected".to_string(),
        ));
    }
    let mut class_ids = BTreeSet::new();
    for mapping in &request.category_mappings {
        if mapping.source_category_key.trim().is_empty()
            || mapping.source_category_id.trim().is_empty()
            || mapping.class_name.trim().is_empty()
            || !valid_color(&mapping.color)
            || mapping.class_id.validate_path_segment().is_err()
            || (mapping.selected && !class_ids.insert(mapping.class_id.clone()))
        {
            return Err(ApiError::Unprocessable(
                "category mappings require valid unique source keys and selected class IDs"
                    .to_string(),
            ));
        }
    }

    let mut skeletons = BTreeMap::new();
    let mut skeleton_categories = BTreeSet::new();
    for mapping in &request.skeleton_mappings {
        if !selected.contains(&mapping.source_category_key)
            || !mapping.names_confirmed
            || !skeleton_categories.insert(mapping.source_category_key.clone())
            || skeletons
                .insert(mapping.target_task_id.clone(), mapping)
                .is_some()
        {
            return Err(ApiError::Unprocessable(
                "skeleton mappings must uniquely target selected categories and tasks".to_string(),
            ));
        }
        validate_skeleton(&mapping.skeleton)?;
        let source_names = mapping
            .source_keypoint_names
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if source_names.len() != mapping.source_keypoint_names.len()
            || source_names.iter().any(|name| name.trim().is_empty())
        {
            return Err(ApiError::Unprocessable(
                "source keypoint names must be unique and nonempty".to_string(),
            ));
        }
    }
    let mut first_intent = None;
    let mut task_ids = BTreeSet::new();
    let mut category_task_types = BTreeSet::new();
    let mut task_mappings = Vec::new();
    for mapping in &request.task_mappings {
        if !selected.contains(&mapping.source_category_key)
            || !task_ids.insert(mapping.task.task_id.clone())
            || !category_task_types.insert((
                mapping.source_category_key.clone(),
                geometry_kind(mapping.task.annotation_type.clone()),
            ))
        {
            return Err(ApiError::Unprocessable(
                "task mappings must have unique IDs and types for selected source categories"
                    .to_string(),
            ));
        }
        let category = request
            .category_mappings
            .iter()
            .find(|category| category.source_category_key == mapping.source_category_key)
            .expect("selected category was validated");
        if mapping.task.task_id.validate_path_segment().is_err()
            || mapping.task.name.trim().is_empty()
            || mapping.task.instructions.title.trim().is_empty()
            || mapping.task.instructions.example_text.trim().is_empty()
            || mapping.task.class_ids != [category.class_id.clone()]
            || !mapping.task.enabled
            || !mapping.task.prelabel_config_ids.is_empty()
        {
            return Err(ApiError::Unprocessable(
                "mapped tasks must be enabled, use exactly the mapped class, and have no prelabels"
                    .to_string(),
            ));
        }
        let task = mapping.task.clone();
        if let Some(skeleton) = skeletons.get(&task.task_id) {
            if task.annotation_type != labello_domain::AnnotationType::Skeleton
                || skeleton.source_category_key != mapping.source_category_key
                || task.skeleton.as_ref() != Some(&skeleton.skeleton)
            {
                return Err(ApiError::Unprocessable(
                    "skeleton mappings must exactly match their skeleton task and category"
                        .to_string(),
                ));
            }
        } else if task.annotation_type == labello_domain::AnnotationType::Skeleton {
            return Err(ApiError::Unprocessable(
                "every skeleton task requires an explicit confirmed skeleton mapping".to_string(),
            ));
        } else if task.skeleton.is_some() || task.manual_box_guide_migration.is_some() {
            return Err(ApiError::Unprocessable(
                "bounding-box tasks cannot contain skeleton or manual-guide configuration"
                    .to_string(),
            ));
        }
        validate_workflow(&task, mapping.workflow_intent)?;
        let intent = match mapping.workflow_intent {
            client::ImportWorkflowIntent::AuthoritativeGroundTruth => {
                storage::ImportIntent::AuthoritativeGroundTruth
            }
            client::ImportWorkflowIntent::RequireApproval => storage::ImportIntent::RequireApproval,
            client::ImportWorkflowIntent::SeedFutureAnnotation => {
                storage::ImportIntent::SeedFutureAnnotation
            }
        };
        first_intent.get_or_insert(intent);
        task_mappings.push(storage::ImportTaskMapping {
            source_category_key: mapping.source_category_key.clone(),
            task,
            intent,
        });
    }
    if skeletons.len()
        != task_mappings
            .iter()
            .filter(|mapping| {
                mapping.task.annotation_type == labello_domain::AnnotationType::Skeleton
            })
            .count()
    {
        return Err(ApiError::Unprocessable(
            "orphan skeleton mappings are not supported".to_string(),
        ));
    }
    current.intent = first_intent.unwrap_or(current.intent);
    current.category_mappings = request
        .category_mappings
        .iter()
        .map(|mapping| storage::ImportCategoryMapping {
            source_category_key: mapping.source_category_key.clone(),
            source_category_id: mapping.source_category_id.clone(),
            class_id: mapping.class_id.clone(),
            class_name: mapping.class_name.clone(),
            color: mapping.color.clone(),
            selected: mapping.selected,
        })
        .collect();
    current.task_mappings = task_mappings.clone();

    let mut bounding_boxes = false;
    let mut skeleton_output = false;
    let mut manual_schemas = Vec::new();
    let mut geometry_targets = BTreeSet::new();
    let mut manual_categories = Vec::new();
    let mut geometry_mappings = Vec::new();
    for mapping in &request.geometry_mappings {
        if !selected.contains(&mapping.source_category_key) {
            return Err(ApiError::Unprocessable(
                "geometry mappings may only reference selected categories".to_string(),
            ));
        }
        if !geometry_targets.insert((mapping.source_category_key.clone(), mapping.target_geometry))
        {
            return Err(ApiError::Unprocessable(
                "geometry mapping targets must be unique".to_string(),
            ));
        }
        let matching_task = task_mappings.iter().find(|task| {
            task.source_category_key == mapping.source_category_key
                && geometry_kind(task.task.annotation_type.clone()) == mapping.target_geometry
        });
        let policy = match mapping.policy {
            client::ImportGeometryPolicy::Direct => {
                if !mapping.parameters.is_empty()
                    || mapping.source_geometry != mapping.target_geometry
                    || matching_task.is_none()
                {
                    return Err(ApiError::Unprocessable(
                        "direct geometry must keep its type and target a matching task".to_string(),
                    ));
                }
                match mapping.target_geometry {
                    client::ImportGeometryKind::BoundingBox => bounding_boxes = true,
                    client::ImportGeometryKind::Skeleton => {
                        let skeleton = request
                            .skeleton_mappings
                            .iter()
                            .find(|skeleton| {
                                skeleton.source_category_key == mapping.source_category_key
                            })
                            .expect("skeleton task mapping was validated");
                        let target_names = skeleton
                            .skeleton
                            .keypoints
                            .iter()
                            .map(|point| &point.name)
                            .collect::<Vec<_>>();
                        if skeleton.source_keypoint_names.iter().collect::<Vec<_>>() != target_names
                        {
                            return Err(ApiError::Unprocessable(
                                "direct skeleton mappings cannot rename or reorder source keypoints"
                                    .to_string(),
                            ));
                        }
                        skeleton_output = true;
                    }
                }
                labello_domain::ImportGeometryPolicy::Direct
            }
            client::ImportGeometryPolicy::KeypointEnvelopeV1 => {
                if mapping.source_geometry != client::ImportGeometryKind::Skeleton
                    || mapping.target_geometry != client::ImportGeometryKind::BoundingBox
                    || matching_task.is_none()
                {
                    return Err(ApiError::Unprocessable(
                        "keypoint-envelope geometry requires a skeleton source and bounding-box task"
                            .to_string(),
                    ));
                }
                let (padding_ratio, minimum_pixels, include_hidden) =
                    envelope_parameters(&mapping.parameters)?;
                bounding_boxes = true;
                labello_domain::ImportGeometryPolicy::KeypointEnvelopeV1 {
                    padding_ratio,
                    minimum_pixels,
                    include_hidden,
                }
            }
            client::ImportGeometryPolicy::BoxRelativeTemplateV1 => {
                if mapping.source_geometry != client::ImportGeometryKind::BoundingBox
                    || mapping.target_geometry != client::ImportGeometryKind::Skeleton
                    || matching_task.is_none()
                {
                    return Err(ApiError::Unprocessable(
                        "box-relative templates require a bounding-box source and skeleton task"
                            .to_string(),
                    ));
                }
                let skeleton = request
                    .skeleton_mappings
                    .iter()
                    .find(|skeleton| skeleton.source_category_key == mapping.source_category_key)
                    .ok_or_else(|| {
                        ApiError::Unprocessable(
                            "box-relative templates require a confirmed skeleton mapping"
                                .to_string(),
                        )
                    })?;
                if !skeleton.source_keypoint_names.is_empty() {
                    return Err(ApiError::Unprocessable(
                        "box-relative templates cannot declare source keypoint names".to_string(),
                    ));
                }
                skeleton_output = true;
                labello_domain::ImportGeometryPolicy::BoxRelativeTemplateV1 {
                    keypoints: template_parameters(&mapping.parameters, &skeleton.skeleton)?,
                }
            }
            client::ImportGeometryPolicy::ManualBoxGuideV1 => {
                if !mapping.parameters.is_empty()
                    || mapping.source_geometry != client::ImportGeometryKind::BoundingBox
                    || mapping.target_geometry != client::ImportGeometryKind::Skeleton
                    || matching_task.is_none()
                {
                    return Err(ApiError::Unprocessable(
                        "manual box-guide requires a box-to-skeleton category and task".to_string(),
                    ));
                }
                manual_categories.push(mapping.source_category_key.clone());
                bounding_boxes = true;
                skeleton_output = true;
                let skeleton = request
                    .skeleton_mappings
                    .iter()
                    .find(|skeleton| skeleton.source_category_key == mapping.source_category_key)
                    .ok_or_else(|| {
                        ApiError::Unprocessable(
                            "manual box-guide mapping requires a skeleton mapping".to_string(),
                        )
                    })?;
                if !skeleton.source_keypoint_names.is_empty() {
                    return Err(ApiError::Unprocessable(
                        "manual box-guide mappings cannot declare source keypoint names"
                            .to_string(),
                    ));
                }
                manual_schemas.push(storage::BoxToSkeletonPolicy::ManualBoxGuide {
                    keypoint_names: skeleton
                        .skeleton
                        .keypoints
                        .iter()
                        .map(|point| point.name.clone())
                        .collect(),
                    edges: skeleton
                        .skeleton
                        .edges
                        .iter()
                        .map(|edge| (edge.from.clone(), edge.to.clone()))
                        .collect(),
                });
                labello_domain::ImportGeometryPolicy::ManualBoxGuideV1
            }
            client::ImportGeometryPolicy::Omit => {
                if !mapping.parameters.is_empty() || matching_task.is_some() {
                    return Err(ApiError::Unprocessable(
                        "omitted geometry cannot have a matching task mapping".to_string(),
                    ));
                }
                labello_domain::ImportGeometryPolicy::Omit
            }
        };
        geometry_mappings.push(labello_domain::ImportGeometryMapping {
            source_category_key: mapping.source_category_key.clone(),
            source_geometry: domain_geometry_kind(mapping.source_geometry),
            target_geometry: domain_geometry_kind(mapping.target_geometry),
            policy,
        });
    }
    for task in &task_mappings {
        if !geometry_targets.contains(&(
            task.source_category_key.clone(),
            geometry_kind(task.task.annotation_type.clone()),
        )) {
            return Err(ApiError::Unprocessable(
                "every mapped task requires one matching geometry mapping".to_string(),
            ));
        }
    }
    for category_key in manual_categories {
        let skeleton_task = task_mappings
            .iter()
            .find(|mapping| {
                mapping.source_category_key == category_key
                    && mapping.task.annotation_type == labello_domain::AnnotationType::Skeleton
            })
            .expect("manual geometry task was validated");
        let guide = task_mappings
            .iter()
            .find(|mapping| {
                mapping.source_category_key == category_key
                    && mapping.task.annotation_type == labello_domain::AnnotationType::BoundingBox
            })
            .ok_or_else(|| {
                ApiError::Unprocessable(
                    "manual box-guide migration requires a direct bounding-box guide task"
                        .to_string(),
                )
            })?;
        skeleton_task
            .task
            .validate_manual_migration(&guide.task)
            .map_err(|_| {
                ApiError::Unprocessable(
                    "manual box-guide task and guide configuration are inconsistent".to_string(),
                )
            })?;
    }
    // The legacy output summary can represent only one schema. Explicit geometry and
    // task mappings remain authoritative when categories use different schemas.
    let box_to_skeleton = manual_schemas
        .first()
        .filter(|schema| manual_schemas.iter().all(|candidate| candidate == *schema))
        .cloned()
        .unwrap_or(storage::BoxToSkeletonPolicy::None);
    current.output = storage::OutputPolicy {
        bounding_boxes,
        skeletons: skeleton_output,
        box_to_skeleton,
    };
    current.geometry_mappings = geometry_mappings;
    current.policies = storage::CompatibilityPolicies {
        yolo_missing_labels: match request.compatibility.yolo_missing_labels {
            client::YoloMissingLabelPolicy::Block => storage::YoloMissingLabelPolicy::Block,
            client::YoloMissingLabelPolicy::Incomplete => {
                storage::YoloMissingLabelPolicy::RetainIncomplete
            }
            client::YoloMissingLabelPolicy::MissingIsBackground => {
                storage::YoloMissingLabelPolicy::MissingIsBackground
            }
        },
        yolo_duplicate_rows: match request.compatibility.yolo_duplicate_rows {
            client::YoloDuplicateRowPolicy::Block => storage::DuplicateRowPolicy::Block,
            client::YoloDuplicateRowPolicy::Deduplicate => storage::DuplicateRowPolicy::Deduplicate,
        },
        coco_crowds: match request.compatibility.coco_crowds {
            client::CocoCrowdPolicy::Block => storage::CocoCrowdPolicy::Block,
            client::CocoCrowdPolicy::Incomplete => storage::CocoCrowdPolicy::Incomplete,
            client::CocoCrowdPolicy::ExcludeImageTask => storage::CocoCrowdPolicy::ExcludeImageTask,
        },
        coco_bbox_only: request.compatibility.coco_structure
            == client::CocoStructurePolicy::BboxCompatibility,
        geometry_bounds: match request.compatibility.geometry_bounds {
            client::GeometryBoundsPolicy::Reject => storage::GeometryBoundsPolicy::Block,
            client::GeometryBoundsPolicy::Clip => storage::GeometryBoundsPolicy::ClipDerived,
        },
        cross_split_duplicates: match request.compatibility.cross_split_duplicates {
            client::CrossSplitDuplicatePolicy::Block => storage::CrossSplitDuplicatePolicy::Block,
            client::CrossSplitDuplicatePolicy::MergeMemberships => {
                storage::CrossSplitDuplicatePolicy::MultipleMemberships
            }
        },
        yolo_keypoint_names: match request.compatibility.missing_keypoint_names {
            client::MissingKeypointNamesPolicy::Block => {
                storage::YoloKeypointNamePolicy::RequireSourceNames
            }
            client::MissingKeypointNamesPolicy::GenerateIndexed => {
                storage::YoloKeypointNamePolicy::GenerateIndexed
            }
        },
    };
    current.acknowledged_warning_codes = request
        .acknowledgements
        .into_iter()
        .filter(|acknowledgement| acknowledgement.acknowledged)
        .map(|acknowledgement| acknowledgement.diagnostic_code)
        .collect();
    current.acknowledged_warning_codes.sort();
    current.acknowledged_warning_codes.dedup();
    Ok(current)
}

fn validate_plan_update_against_current(
    current: &storage::ImportPlan,
    request: &client::UpdateImportPlanRequest,
) -> ApiResult<()> {
    let known_categories = if current.request.category_mappings.is_empty() {
        current.class_ids.keys().collect::<BTreeSet<_>>()
    } else {
        current
            .request
            .category_mappings
            .iter()
            .map(|mapping| &mapping.source_category_key)
            .collect()
    };
    if request.category_mappings.iter().any(|mapping| {
        !known_categories.contains(&mapping.source_category_key)
            || current
                .source_categories
                .get(&mapping.source_category_key)
                .is_none_or(|source| source.source_category_id != mapping.source_category_id)
    }) {
        return Err(ApiError::Unprocessable(
            "category mappings must preserve discovered source category keys and IDs".to_string(),
        ));
    }

    let mut acknowledged = BTreeSet::new();
    for acknowledgement in request
        .acknowledgements
        .iter()
        .filter(|acknowledgement| acknowledgement.acknowledged)
    {
        let diagnostic = current
            .diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.code == acknowledgement.diagnostic_code
                    && diagnostic.requires_acknowledgement
            })
            .ok_or_else(|| {
                ApiError::Unprocessable(
                    "acknowledgements must reference a current acknowledgement-required diagnostic"
                        .to_string(),
                )
            })?;
        if acknowledgement.policy.trim().is_empty()
            || acknowledgement.affected_count != diagnostic.count
            || !acknowledged.insert(&acknowledgement.diagnostic_code)
        {
            return Err(ApiError::Unprocessable(
                "acknowledgement policy, count, and diagnostic code must match the current plan"
                    .to_string(),
            ));
        }
    }
    Ok(())
}

fn valid_color(color: &str) -> bool {
    color.len() == 7
        && color.starts_with('#')
        && color[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn domain_geometry_kind(kind: client::ImportGeometryKind) -> labello_domain::ImportGeometryKind {
    match kind {
        client::ImportGeometryKind::BoundingBox => labello_domain::ImportGeometryKind::BoundingBox,
        client::ImportGeometryKind::Skeleton => labello_domain::ImportGeometryKind::Skeleton,
    }
}

fn envelope_parameters(
    parameters: &[client::ImportMappingParameter],
) -> ApiResult<(f64, u32, bool)> {
    let mut padding_ratio = None;
    let mut minimum_pixels = None;
    let mut include_hidden = None;
    for parameter in parameters {
        match parameter {
            client::ImportMappingParameter::Scalar { name, value }
                if matches!(name.as_str(), "padding" | "padding_ratio" | "paddingRatio")
                    && padding_ratio.replace(*value).is_none() => {}
            client::ImportMappingParameter::Scalar { name, value }
                if matches!(
                    name.as_str(),
                    "minimum_pixels" | "minimumPixels" | "min_pixels" | "minPixels"
                ) && minimum_pixels.replace(*value).is_none() => {}
            client::ImportMappingParameter::Boolean { name, value }
                if matches!(name.as_str(), "include_hidden" | "includeHidden" | "hidden")
                    && include_hidden.replace(*value).is_none() => {}
            _ => {
                return Err(ApiError::Unprocessable(
                    "keypoint-envelope parameters must contain one padding, minimum-pixels, and hidden value"
                        .to_string(),
                ));
            }
        }
    }
    let padding_ratio =
        padding_ratio.filter(|value| value.is_finite() && (0.0..=1.0).contains(value));
    let minimum_pixels = minimum_pixels.and_then(|value| {
        (value.is_finite() && value.fract() == 0.0 && value >= 1.0 && value <= f64::from(u32::MAX))
            .then_some(value as u32)
    });
    match (padding_ratio, minimum_pixels, include_hidden) {
        (Some(padding_ratio), Some(minimum_pixels), Some(include_hidden)) => {
            Ok((padding_ratio, minimum_pixels, include_hidden))
        }
        _ => Err(ApiError::Unprocessable(
            "keypoint-envelope parameters are missing or invalid".to_string(),
        )),
    }
}

fn template_parameters(
    parameters: &[client::ImportMappingParameter],
    skeleton: &labello_domain::SkeletonSpec,
) -> ApiResult<Vec<labello_domain::ImportTemplateKeypoint>> {
    if parameters.len() != skeleton.keypoints.len() || parameters.is_empty() {
        return Err(ApiError::Unprocessable(
            "box-relative templates must define every target skeleton keypoint exactly once"
                .to_string(),
        ));
    }
    let mut keypoints = Vec::with_capacity(parameters.len());
    for (parameter, spec) in parameters.iter().zip(&skeleton.keypoints) {
        let client::ImportMappingParameter::Point { name, x, y, state } = parameter else {
            return Err(ApiError::Unprocessable(
                "box-relative template parameters must be named points".to_string(),
            ));
        };
        if name != &spec.name
            || !x.is_finite()
            || !y.is_finite()
            || !(0.0..=1.0).contains(x)
            || !(0.0..=1.0).contains(y)
            || match state {
                labello_domain::KeypointState::Visible => false,
                labello_domain::KeypointState::Hidden => !skeleton.allow_hidden,
                labello_domain::KeypointState::Absent => !skeleton.allow_absent || spec.required,
            }
        {
            return Err(ApiError::Unprocessable(
                "box-relative template points must exactly match the schema and visibility policy"
                    .to_string(),
            ));
        }
        keypoints.push(labello_domain::ImportTemplateKeypoint {
            name: name.clone(),
            x: *x,
            y: *y,
            state: state.clone(),
        });
    }
    if keypoints
        .iter()
        .all(|point| point.state == labello_domain::KeypointState::Absent)
    {
        return Err(ApiError::Unprocessable(
            "box-relative templates cannot contain only absent keypoints".to_string(),
        ));
    }
    Ok(keypoints)
}

fn validate_identity_component(value: &str, name: &str) -> ApiResult<()> {
    if value.is_empty()
        || value.len() > 255
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(ApiError::Unprocessable(format!(
            "{name} must contain only ASCII letters, digits, '.', '_', or '-'"
        )));
    }
    Ok(())
}

fn descriptor_kind_name(kind: client::ImportDescriptorKind) -> &'static str {
    match kind {
        client::ImportDescriptorKind::YoloDataset => "yolo_dataset",
        client::ImportDescriptorKind::CocoInstances => "coco_instances",
        client::ImportDescriptorKind::CocoKeypoints => "coco_keypoints",
    }
}

fn geometry_kind(annotation_type: labello_domain::AnnotationType) -> client::ImportGeometryKind {
    match annotation_type {
        labello_domain::AnnotationType::BoundingBox => client::ImportGeometryKind::BoundingBox,
        labello_domain::AnnotationType::Skeleton => client::ImportGeometryKind::Skeleton,
    }
}

fn validate_skeleton(skeleton: &labello_domain::SkeletonSpec) -> ApiResult<()> {
    let names = skeleton
        .keypoints
        .iter()
        .map(|point| point.name.as_str())
        .collect::<BTreeSet<_>>();
    if names.is_empty()
        || names.len() != skeleton.keypoints.len()
        || names.iter().any(|name| name.trim().is_empty())
        || skeleton.edges.iter().any(|edge| {
            edge.from == edge.to
                || !names.contains(edge.from.as_str())
                || !names.contains(edge.to.as_str())
        })
    {
        return Err(ApiError::Unprocessable(
            "skeleton keypoints and edges must form a valid unique schema".to_string(),
        ));
    }
    Ok(())
}

fn validate_workflow(
    task: &labello_domain::TaskDefinition,
    intent: client::ImportWorkflowIntent,
) -> ApiResult<()> {
    use labello_domain::ReviewWorkflow;

    let review = &task.review;
    let structurally_valid = match &review.workflow {
        ReviewWorkflow::None => {
            review.required_reviews == 0
                && !review.allow_reviewer_corrections
                && review.agreement_threshold.is_none()
        }
        ReviewWorkflow::Approval => {
            review.required_reviews >= 1
                && !review.allow_reviewer_corrections
                && review.agreement_threshold.is_none()
        }
        ReviewWorkflow::IndependentAgreement => {
            review.required_reviews >= 2
                && !review.allow_reviewer_corrections
                && review
                    .agreement_threshold
                    .as_ref()
                    .is_some_and(|threshold| {
                        threshold.threshold.is_finite()
                            && (0.0..=1.0).contains(&threshold.threshold)
                    })
        }
    };
    let intent_valid = if task.manual_box_guide_migration.is_some() {
        review.workflow == ReviewWorkflow::Approval
    } else {
        match intent {
            client::ImportWorkflowIntent::AuthoritativeGroundTruth => {
                review.workflow == ReviewWorkflow::None
            }
            client::ImportWorkflowIntent::RequireApproval => {
                review.workflow == ReviewWorkflow::Approval
            }
            client::ImportWorkflowIntent::SeedFutureAnnotation => {
                review.workflow != ReviewWorkflow::None
            }
        }
    };
    if !structurally_valid || !intent_valid {
        return Err(ApiError::Unprocessable(
            "task review workflow is inconsistent with its import workflow intent".to_string(),
        ));
    }
    Ok(())
}

fn resolve_source_reference(control: &JobControl, reference: &str) -> ApiResult<String> {
    if let Some(file) = control.files.get(reference) {
        return Ok(file.relative_path.clone());
    }
    if let Some(file) = control
        .files
        .values()
        .find(|file| file.client_file_id.as_deref() == Some(reference))
    {
        return Ok(file.relative_path.clone());
    }
    if Path::new(reference).is_absolute()
        || reference.is_empty()
        || reference
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(ApiError::Unprocessable(
            "descriptor source reference is invalid".to_string(),
        ));
    }
    control
        .files
        .values()
        .find(|file| file.relative_path == reference)
        .map(|file| file.relative_path.clone())
        .ok_or_else(|| {
            ApiError::Unprocessable("descriptor source reference was not registered".to_string())
        })
}
