fn parse_coco(
    source: &SourceAccess<'_>,
    request: &PreflightRequest,
    limits: &ImportLimits,
    keypoints_profile: bool,
    diagnostics: &mut Diagnostics,
    decoded_memory: &DecodedImageMemoryLimiter,
    cancelled: &AtomicBool,
) -> StorageResult<ImportIr> {
    if request.coco_descriptors.is_empty() {
        return Err(import_error(
            "coco_selection_invalid",
            "COCO requires at least one descriptor and image root selection",
        ));
    }
    validate_coco_selections(&request.coco_descriptors, keypoints_profile)?;
    let descriptor_bytes =
        request
            .coco_descriptors
            .iter()
            .try_fold(0_u64, |total, selection| {
                let file = source.file(&selection.descriptor_path)?;
                total.checked_add(file.byte_size).ok_or_else(|| {
                    import_error(
                        "descriptor_byte_limit",
                        "COCO descriptor byte count overflowed",
                    )
                })
            })?;
    if descriptor_bytes > limits.descriptor_bytes {
        return Err(import_error(
            "descriptor_byte_limit",
            "combined COCO descriptors exceed the configured byte limit",
        ));
    }
    let mut ir = ImportIr::new();
    let mut object_payloads: BTreeMap<String, IrObject> = BTreeMap::new();
    let mut declared_annotations = 0_usize;
    for selection in &request.coco_descriptors {
        check_cancelled(cancelled)?;
        let descriptor_keypoints =
            selection.kind == labello_domain::ImportDescriptorKind::CocoKeypoints;
        let bytes = source.read_limited(&selection.descriptor_path, limits.descriptor_bytes)?;
        enforce_json_nesting(&bytes, limits.structured_data_nesting)?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|error| {
            import_error(
                "coco_json_invalid",
                format!("COCO descriptor is invalid: {error}"),
            )
        })?;
        validate_json_value(&value, &mut 0)?;
        let root = value.as_object().ok_or_else(|| {
            if value.is_array() {
                import_error(
                    "coco_results_rejected",
                    "COCO result arrays are not ground-truth descriptors",
                )
            } else {
                import_error(
                    "coco_root_invalid",
                    "COCO descriptor must be a top-level object",
                )
            }
        })?;
        let categories = required_array(root, "categories")?;
        let identity_namespace = coco_identity_namespace(selection);
        let mut category_ids = BTreeSet::new();
        for category in categories {
            let object = category.as_object().ok_or_else(|| {
                import_error("coco_category_invalid", "COCO category must be an object")
            })?;
            let id = json_id(object.get("id"), "coco_category_id_invalid")?;
            if !category_ids.insert(id) {
                return Err(import_error(
                    "coco_category_id_duplicate",
                    "COCO category IDs must be unique within a descriptor",
                ));
            }
            let name = required_string(object, "name", "coco_category_name_invalid")?.to_string();
            let category_key = format!("{identity_namespace}:{id}");
            if !ir.categories.contains_key(&category_key)
                && ir.categories.len() >= limits.selected_categories
            {
                return Err(import_error(
                    "category_limit",
                    "combined COCO categories exceed the configured limit",
                ));
            }
            let (keypoint_names, edges) = parse_coco_schema(object, limits, descriptor_keypoints)?;
            let mut candidate = IrCategory {
                key: category_key.clone(),
                source_namespace: selection.source_namespace.clone(),
                name,
                source_id: id.to_string(),
                supercategory: object
                    .get("supercategory")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                keypoint_names,
                edges,
                allow_hidden: descriptor_keypoints,
            };
            if let Some(existing) = ir.categories.get_mut(&category_key) {
                if existing.name != candidate.name
                    || existing.source_id != candidate.source_id
                    || existing.supercategory != candidate.supercategory
                    || (!existing.keypoint_names.is_empty()
                        && !candidate.keypoint_names.is_empty()
                        && (existing.keypoint_names != candidate.keypoint_names
                            || existing.edges != candidate.edges))
                {
                    return Err(import_error(
                        "coco_category_conflict",
                        "paired COCO categories disagree",
                    ));
                }
                if existing.keypoint_names.is_empty() {
                    existing.keypoint_names = std::mem::take(&mut candidate.keypoint_names);
                    existing.edges = std::mem::take(&mut candidate.edges);
                    existing.allow_hidden = candidate.allow_hidden;
                }
            } else {
                ir.categories.insert(category_key.clone(), candidate);
            }
        }
        let images = required_array(root, "images")?;
        let mut image_ids = BTreeSet::new();
        let mut source_keys = BTreeMap::new();
        for image in images {
            check_cancelled(cancelled)?;
            let object = image.as_object().ok_or_else(|| {
                import_error("coco_image_invalid", "COCO image must be an object")
            })?;
            let id = json_id(object.get("id"), "coco_image_id_invalid")?;
            if !image_ids.insert(id) {
                return Err(import_error(
                    "coco_image_id_duplicate",
                    "COCO image IDs must be unique within a descriptor",
                ));
            }
            let source_key = format!("{identity_namespace}:{id}");
            source_keys.insert(id, source_key.clone());
            if !ir.images.contains_key(&source_key) && ir.images.len() >= limits.selected_images {
                return Err(import_error(
                    "selected_image_limit",
                    "combined COCO images exceed the configured limit",
                ));
            }
            let file_name = required_string(object, "file_name", "coco_file_name_invalid")?;
            let image_path = join_source_path(&selection.image_root, file_name, limits)?;
            let file = source.file(&image_path)?;
            let declared_width = json_u32(object.get("width"), "coco_dimension_invalid")?;
            let declared_height = json_u32(object.get("height"), "coco_dimension_invalid")?;
            if let Some(existing) = ir.images.get_mut(&source_key) {
                if existing.file_id != file.file_id
                    || existing.source_path != image_path
                    || existing.width != declared_width
                    || existing.height != declared_height
                {
                    return Err(import_error(
                        "coco_image_conflict",
                        "paired COCO image records disagree",
                    ));
                }
                existing.split_memberships.insert(selection.split.clone());
                continue;
            }
            let image = {
                let validated = validate_image(
                    &source.physical_path(file),
                    &image_path,
                    file,
                    limits,
                    decoded_memory,
                    cancelled,
                )?;
                IrImage {
                    source_key: source_key.clone(),
                    file_id: file.file_id.clone(),
                    source_path: image_path.clone(),
                    display_name: file_name
                        .rsplit('/')
                        .next()
                        .unwrap_or(file_name)
                        .to_string(),
                    split_memberships: BTreeSet::from([selection.split.clone()]),
                    source_namespace: selection.source_namespace.clone(),
                    blake3: validated.blake3,
                    byte_size: validated.byte_size,
                    width: validated.width,
                    height: validated.height,
                    media_type: validated.media_type,
                    extension: validated.extension,
                }
            };
            if (declared_width, declared_height) != (image.width, image.height) {
                return Err(import_error(
                    "coco_dimension_mismatch",
                    "COCO dimensions do not match decoded image dimensions",
                ));
            }
            ir.images.insert(source_key.clone(), image);
        }
        let annotations = required_array(root, "annotations")?;
        declared_annotations = declared_annotations
            .checked_add(annotations.len())
            .ok_or_else(|| import_error("annotation_limit", "COCO annotation count overflowed"))?;
        if declared_annotations > limits.annotations_total {
            return Err(import_error(
                "annotation_limit",
                "combined COCO annotations exceed the configured limit",
            ));
        }
        let mut annotation_ids = BTreeSet::new();
        for annotation in annotations {
            check_cancelled(cancelled)?;
            let object = annotation.as_object().ok_or_else(|| {
                import_error(
                    "coco_annotation_invalid",
                    "COCO annotation must be an object",
                )
            })?;
            if object.contains_key("score") {
                return Err(import_error(
                    "coco_score_rejected",
                    "COCO ground-truth annotations cannot contain score",
                ));
            }
            let id = json_id(object.get("id"), "coco_annotation_id_invalid")?;
            if !annotation_ids.insert(id) {
                return Err(import_error(
                    "coco_annotation_id_duplicate",
                    "COCO annotation IDs must be unique within a descriptor",
                ));
            }
            let image_id = json_id(object.get("image_id"), "coco_image_reference_invalid")?;
            let category_id =
                json_id(object.get("category_id"), "coco_category_reference_invalid")?;
            let source_image_key = source_keys
                .get(&image_id)
                .ok_or_else(|| {
                    import_error(
                        "coco_image_reference_invalid",
                        "COCO annotation references a missing image",
                    )
                })?
                .clone();
            let category_key = format!("{identity_namespace}:{category_id}");
            let category = ir.categories.get(&category_key).ok_or_else(|| {
                import_error(
                    "coco_category_reference_invalid",
                    "COCO annotation references a missing category",
                )
            })?;
            let image = ir
                .images
                .get(&source_image_key)
                .expect("validated image reference");
            let bbox_values = finite_array(object.get("bbox"), 4, "coco_bbox_invalid")?;
            let mut bbox = F64Box {
                x: bbox_values[0] / f64::from(image.width),
                y: bbox_values[1] / f64::from(image.height),
                width: bbox_values[2] / f64::from(image.width),
                height: bbox_values[3] / f64::from(image.height),
            };
            let clipped = validate_or_clip_box(
                &mut bbox,
                request.policies.geometry_bounds,
                diagnostics,
                &selection.descriptor_path,
                0,
            )?;
            let iscrowd = match object.get("iscrowd") {
                Some(value) => json_id(Some(value), "coco_iscrowd_invalid")?,
                None if request.policies.coco_bbox_only => {
                    diagnostics.add(
                        "coco_iscrowd_synthesized",
                        DiagnosticSeverity::WarningRequiresAck,
                        "missing iscrowd is synthesized as zero",
                        false,
                        true,
                        false,
                        Some(example_object(&selection.descriptor_path, id)),
                    );
                    0
                }
                None => {
                    return Err(import_error(
                        "coco_iscrowd_missing",
                        "canonical COCO annotation requires iscrowd",
                    ));
                }
            };
            if iscrowd > 1 {
                return Err(import_error(
                    "coco_iscrowd_invalid",
                    "COCO iscrowd must be zero or one",
                ));
            }
            if iscrowd == 1 {
                ir.equivalence_facts
                    .entry(coverage_key(&source_image_key, &category_key))
                    .or_default()
                    .insert("crowd".to_string());
                match request.policies.coco_crowds {
                    CocoCrowdPolicy::Block => diagnostics.add(
                        "coco_crowd",
                        DiagnosticSeverity::Error,
                        "COCO crowd objects are blocked in strict mode",
                        true,
                        false,
                        true,
                        Some(example_object(&selection.descriptor_path, id)),
                    ),
                    CocoCrowdPolicy::Incomplete => {
                        diagnostics.add(
                            "coco_crowd_incomplete",
                            DiagnosticSeverity::WarningRequiresAck,
                            "COCO crowd object is skipped and coverage remains incomplete",
                            false,
                            true,
                            true,
                            Some(example_object(&selection.descriptor_path, id)),
                        );
                        ir.coverage_overrides.insert(
                            coverage_key(&source_image_key, &category_key),
                            ImportCoverage::Incomplete,
                        );
                    }
                    CocoCrowdPolicy::ExcludeImageTask => {
                        diagnostics.add(
                            "coco_crowd_excluded",
                            DiagnosticSeverity::WarningRequiresAck,
                            "COCO crowd object excludes the image-task pair",
                            false,
                            true,
                            true,
                            Some(example_object(&selection.descriptor_path, id)),
                        );
                        ir.coverage_overrides.insert(
                            coverage_key(&source_image_key, &category_key),
                            ImportCoverage::Excluded,
                        );
                    }
                }
                continue;
            }
            let segmentation = object.get("segmentation");
            if segmentation.is_none() && !request.policies.coco_bbox_only {
                return Err(import_error(
                    "coco_segmentation_missing",
                    "canonical COCO annotation requires segmentation",
                ));
            }
            if let Some(segmentation) = segmentation {
                validate_segmentation(segmentation, image.width, image.height, limits)?;
                ir.discarded_segmentation += 1;
            }
            let area = match object.get("area") {
                Some(value) => {
                    let area = value
                        .as_f64()
                        .filter(|value| value.is_finite() && *value >= 0.0)
                        .ok_or_else(|| {
                            import_error(
                                "coco_area_invalid",
                                "COCO area must be finite and non-negative",
                            )
                        })?;
                    Some(area)
                }
                None if request.policies.coco_bbox_only => {
                    diagnostics.add(
                        "coco_area_synthesized",
                        DiagnosticSeverity::WarningRequiresAck,
                        "missing area uses a noncanonical bbox-area surrogate",
                        false,
                        true,
                        false,
                        Some(example_object(&selection.descriptor_path, id)),
                    );
                    Some(bbox_values[2] * bbox_values[3])
                }
                None => {
                    return Err(import_error(
                        "coco_area_missing",
                        "canonical COCO annotation requires area",
                    ));
                }
            };
            if !descriptor_keypoints && object.contains_key("keypoints") {
                return Err(import_error(
                    "coco_profile_mismatch",
                    "COCO keypoint annotations require the keypoints profile",
                ));
            }
            if descriptor_keypoints && !object.contains_key("keypoints") {
                return Err(import_error(
                    "coco_keypoints_missing",
                    "COCO keypoints descriptor annotations require keypoints",
                ));
            }
            let mut skeleton = if descriptor_keypoints {
                Some(parse_coco_keypoints(
                    object,
                    category,
                    image.width,
                    image.height,
                    limits,
                )?)
            } else {
                None
            };
            if descriptor_keypoints
                && skeleton.as_ref().is_some_and(|points| {
                    points
                        .iter()
                        .all(|point| point.state == KeypointState::Absent)
                })
            {
                let key = coverage_key(&source_image_key, &category_key);
                ir.zero_keypoint_coverage.insert(key.clone());
                ir.equivalence_facts
                    .entry(key)
                    .or_default()
                    .insert("zero_keypoints".to_string());
                skeleton = None;
                diagnostics.add(
                    "coco_zero_keypoints",
                    DiagnosticSeverity::Warning,
                    "zero-keypoint object keeps its box but makes skeleton coverage incomplete",
                    false,
                    false,
                    true,
                    Some(example_object(&selection.descriptor_path, id)),
                );
            }
            let object_key = format!("{identity_namespace}:{id}");
            let candidate = IrObject {
                source_object_key: object_key.clone(),
                source_namespace: selection.source_namespace.clone(),
                source_image_key,
                source_category_key: category_key,
                direct_bbox: Some(bbox),
                direct_skeleton: skeleton,
                source_bbox: Some(bbox_values),
                source_area: area,
                source_iscrowd: iscrowd,
                source_segmentation: segmentation.cloned(),
                derived_bbox: false,
                clipped,
                boundary_rounding_normalized: false,
                row_references: vec![format!("{}#{}", selection.descriptor_path, id)],
            };
            if let Some(existing) = object_payloads.get_mut(&object_key) {
                merge_coco_object(existing, candidate)?;
            } else {
                object_payloads.insert(object_key, candidate);
            }
        }
    }
    if keypoints_profile
        && ir
            .categories
            .values()
            .any(|category| category.keypoint_names.is_empty())
    {
        return Err(import_error(
            "coco_keypoint_schema_missing",
            "every selected COCO keypoint category requires a paired keypoint schema",
        ));
    }
    ir.objects = object_payloads.into_values().collect();
    Ok(ir)
}


fn parse_coco_schema(
    object: &serde_json::Map<String, Value>,
    limits: &ImportLimits,
    required: bool,
) -> StorageResult<CocoSkeletonSchema> {
    let Some(values) = object.get("keypoints") else {
        return if required {
            Err(import_error(
                "coco_keypoint_schema_missing",
                "COCO keypoint category requires keypoints",
            ))
        } else {
            Ok((Vec::new(), Vec::new()))
        };
    };
    let names = values
        .as_array()
        .ok_or_else(|| {
            import_error(
                "coco_keypoint_schema_invalid",
                "COCO category keypoints must be an array",
            )
        })?
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .ok_or_else(|| {
                    import_error(
                        "coco_keypoint_schema_invalid",
                        "COCO keypoint names must be nonempty strings",
                    )
                })
        })
        .collect::<StorageResult<Vec<_>>>()?;
    if names.len() > limits.keypoints_per_skeleton
        || names.iter().collect::<BTreeSet<_>>().len() != names.len()
    {
        return Err(import_error(
            "coco_keypoint_schema_invalid",
            "COCO keypoint names must be unique and bounded",
        ));
    }
    let mut edges = Vec::new();
    for edge in object
        .get("skeleton")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let endpoints = edge
            .as_array()
            .filter(|values| values.len() == 2)
            .ok_or_else(|| {
                import_error(
                    "coco_skeleton_edge_invalid",
                    "COCO skeleton edges require two endpoints",
                )
            })?;
        let from = endpoints[0]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value >= 1 && *value <= names.len())
            .ok_or_else(|| {
                import_error(
                    "coco_skeleton_edge_invalid",
                    "COCO skeleton edge endpoint is out of range",
                )
            })?;
        let to = endpoints[1]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value >= 1 && *value <= names.len())
            .ok_or_else(|| {
                import_error(
                    "coco_skeleton_edge_invalid",
                    "COCO skeleton edge endpoint is out of range",
                )
            })?;
        let edge = (names[from - 1].clone(), names[to - 1].clone());
        if from == to || edges.contains(&edge) {
            return Err(import_error(
                "coco_skeleton_edge_invalid",
                "COCO skeleton edges must be distinct and non-self",
            ));
        }
        edges.push(edge);
    }
    Ok((names, edges))
}

fn parse_coco_keypoints(
    object: &serde_json::Map<String, Value>,
    category: &IrCategory,
    width: u32,
    height: u32,
    limits: &ImportLimits,
) -> StorageResult<Vec<IrKeypoint>> {
    let values = finite_array(
        object.get("keypoints"),
        category.keypoint_names.len() * 3,
        "coco_keypoints_invalid",
    )?;
    if category.keypoint_names.len() > limits.keypoints_per_skeleton {
        return Err(import_error(
            "keypoint_limit",
            "COCO keypoints exceed configured limit",
        ));
    }
    let mut labeled = 0_u64;
    let mut output = Vec::with_capacity(category.keypoint_names.len());
    for (name, values) in category.keypoint_names.iter().zip(values.chunks_exact(3)) {
        let state = match values[2] {
            0.0 if values[0] == 0.0 && values[1] == 0.0 => KeypointState::Absent,
            0.0 => {
                return Err(import_error(
                    "coco_keypoint_absent_coordinates",
                    "absent COCO keypoint requires zero coordinates",
                ));
            }
            1.0 => {
                labeled += 1;
                KeypointState::Hidden
            }
            2.0 => {
                labeled += 1;
                KeypointState::Visible
            }
            _ => {
                return Err(import_error(
                    "coco_keypoint_visibility",
                    "COCO keypoint visibility must be 0, 1, or 2",
                ));
            }
        };
        let (x, y) = (values[0] / f64::from(width), values[1] / f64::from(height));
        if state != KeypointState::Absent
            && (!(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y))
        {
            return Err(import_error(
                "coco_keypoint_bounds",
                "COCO keypoint is outside decoded image bounds",
            ));
        }
        output.push(IrKeypoint {
            name: name.clone(),
            x: (state != KeypointState::Absent).then_some(x),
            y: (state != KeypointState::Absent).then_some(y),
            state,
        });
    }
    if json_id(object.get("num_keypoints"), "coco_num_keypoints_invalid")? != labeled {
        return Err(import_error(
            "coco_num_keypoints_mismatch",
            "COCO num_keypoints does not match labeled keypoints",
        ));
    }
    Ok(output)
}

fn validate_segmentation(
    value: &Value,
    image_width: u32,
    image_height: u32,
    limits: &ImportLimits,
) -> StorageResult<()> {
    enforce_value_depth(
        value,
        limits.structured_data_nesting,
        "coco_segmentation_nesting",
    )?;
    if let Some(polygons) = value.as_array() {
        if polygons.is_empty() {
            return Err(import_error(
                "coco_segmentation_invalid",
                "COCO polygon segmentation cannot be empty",
            ));
        }
        for polygon in polygons {
            let coordinates = polygon
                .as_array()
                .filter(|values| values.len() >= 6 && values.len() % 2 == 0)
                .ok_or_else(|| {
                    import_error(
                        "coco_segmentation_invalid",
                        "COCO polygons require at least three coordinate pairs",
                    )
                })?;
            if coordinates
                .iter()
                .any(|value| value.as_f64().is_none_or(|value| !value.is_finite()))
            {
                return Err(import_error(
                    "coco_segmentation_invalid",
                    "COCO polygon coordinates must be finite numbers",
                ));
            }
        }
        return Ok(());
    }
    if let Some(rle) = value.as_object() {
        let size = rle
            .get("size")
            .and_then(Value::as_array)
            .filter(|values| values.len() == 2)
            .ok_or_else(|| {
                import_error(
                    "coco_segmentation_invalid",
                    "COCO RLE requires size [height, width]",
                )
            })?;
        let height = json_u32(size.first(), "coco_segmentation_invalid")?;
        let width = json_u32(size.get(1), "coco_segmentation_invalid")?;
        if (width, height) != (image_width, image_height) {
            return Err(import_error(
                "coco_segmentation_invalid",
                "COCO RLE size must match decoded image dimensions",
            ));
        }
        let pixels = u64::from(image_width) * u64::from(image_height);
        let counts_valid = match rle.get("counts") {
            Some(Value::String(counts)) => validate_compressed_coco_rle(counts, pixels),
            Some(Value::Array(counts)) => {
                if counts.len() > pixels.saturating_add(1) as usize {
                    return Err(import_error(
                        "coco_segmentation_invalid",
                        "COCO RLE has more runs than image pixels permit",
                    ));
                }
                let total = counts
                    .iter()
                    .try_fold(0_u64, |total, count| total.checked_add(count.as_u64()?));
                total == Some(pixels)
            }
            _ => false,
        };
        if !counts_valid {
            return Err(import_error(
                "coco_segmentation_invalid",
                "COCO RLE counts must have valid bounded runs totaling the image dimensions",
            ));
        }
        return Ok(());
    }
    Err(import_error(
        "coco_segmentation_invalid",
        "COCO segmentation must be polygon arrays or RLE object",
    ))
}

fn validate_compressed_coco_rle(counts: &str, pixels: u64) -> bool {
    if counts.is_empty() {
        return false;
    }
    let bytes = counts.as_bytes();
    let mut run_count = 0_u64;
    let mut penultimate = 0_i128;
    let mut previous = 0_i128;
    let mut offset = 0_usize;
    let mut total = 0_u64;
    while offset < bytes.len() {
        if run_count > pixels {
            return false;
        }
        let mut value = 0_i128;
        let mut shift = 0_u32;
        loop {
            let Some(encoded) = bytes.get(offset).and_then(|byte| byte.checked_sub(48)) else {
                return false;
            };
            if encoded > 0x3f || shift >= 125 {
                return false;
            }
            offset += 1;
            value |= i128::from(encoded & 0x1f) << shift;
            shift += 5;
            if encoded & 0x20 == 0 {
                if encoded & 0x10 != 0 {
                    value |= (-1_i128).checked_shl(shift).unwrap_or(0);
                }
                break;
            }
            if offset == bytes.len() {
                return false;
            }
        }
        if run_count > 2 {
            let Some(decoded) = value.checked_add(penultimate) else {
                return false;
            };
            value = decoded;
        }
        let Ok(run) = u64::try_from(value) else {
            return false;
        };
        let Some(next_total) = total.checked_add(run) else {
            return false;
        };
        if next_total > pixels {
            return false;
        }
        total = next_total;
        penultimate = previous;
        previous = value;
        run_count += 1;
    }
    total == pixels
}

fn merge_coco_object(existing: &mut IrObject, candidate: IrObject) -> StorageResult<()> {
    if existing.source_image_key != candidate.source_image_key
        || existing.source_category_key != candidate.source_category_key
        || existing.direct_bbox != candidate.direct_bbox
        || existing.source_bbox != candidate.source_bbox
        || existing.source_area != candidate.source_area
        || existing.source_iscrowd != candidate.source_iscrowd
        || existing.source_segmentation != candidate.source_segmentation
        || existing.clipped != candidate.clipped
    {
        return Err(import_error(
            "coco_paired_object_conflict",
            "paired COCO annotation IDs have divergent common fields",
        ));
    }
    match (&existing.direct_skeleton, candidate.direct_skeleton) {
        (None, Some(skeleton)) => existing.direct_skeleton = Some(skeleton),
        (Some(left), Some(right)) if left != &right => {
            return Err(import_error(
                "coco_paired_object_conflict",
                "paired COCO annotation IDs have divergent keypoints",
            ));
        }
        _ => {}
    }
    existing.row_references.extend(candidate.row_references);
    Ok(())
}

fn validate_coco_selections(
    selections: &[CocoDescriptorSelection],
    keypoints_profile: bool,
) -> StorageResult<()> {
    let mut descriptor_paths = BTreeSet::new();
    let mut pairing_groups = BTreeMap::<(&str, &str, &str, &str), BTreeSet<_>>::new();
    for selection in selections {
        if !valid_coco_namespace(&selection.source_namespace)
            || !valid_coco_namespace(&selection.release)
            || !valid_coco_namespace(&selection.split)
            || !descriptor_paths.insert(selection.descriptor_path.as_str())
        {
            return Err(import_error(
                "coco_descriptor_namespace_invalid",
                "every COCO descriptor requires explicit namespace, release, split, and unique path",
            ));
        }
        if selection.kind == labello_domain::ImportDescriptorKind::YoloDataset
            || (!keypoints_profile
                && selection.kind != labello_domain::ImportDescriptorKind::CocoInstances)
        {
            return Err(import_error(
                "coco_profile_mismatch",
                "COCO descriptor kind does not match the selected profile",
            ));
        }
        match selection.pairing_group.as_deref() {
            Some(group) if !valid_coco_namespace(group) => {
                return Err(import_error(
                    "coco_pairing_group_invalid",
                    "COCO pairing groups must be explicit nonempty identifiers",
                ));
            }
            Some(group) => {
                let kinds = pairing_groups
                    .entry((
                        selection.source_namespace.as_str(),
                        selection.release.as_str(),
                        selection.split.as_str(),
                        group,
                    ))
                    .or_default();
                if !kinds.insert(selection.kind) {
                    return Err(import_error(
                        "coco_pairing_group_invalid",
                        "a COCO pairing group may contain only one descriptor of each kind",
                    ));
                }
            }
            None => {}
        }
    }
    let expected = BTreeSet::from([
        labello_domain::ImportDescriptorKind::CocoInstances,
        labello_domain::ImportDescriptorKind::CocoKeypoints,
    ]);
    if pairing_groups.values().any(|kinds| kinds != &expected) {
        return Err(import_error(
            "coco_pairing_group_invalid",
            "a COCO pairing group must contain one instances and one keypoints descriptor",
        ));
    }
    if keypoints_profile
        && !selections
            .iter()
            .any(|selection| selection.kind == labello_domain::ImportDescriptorKind::CocoKeypoints)
    {
        return Err(import_error(
            "coco_profile_mismatch",
            "COCO keypoints profile requires a keypoints descriptor",
        ));
    }
    Ok(())
}

fn valid_coco_namespace(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= 255
        && !value.bytes().any(|byte| byte.is_ascii_control())
}

fn coco_identity_namespace(selection: &CocoDescriptorSelection) -> String {
    let identity = serde_json::json!({
        "namespace": selection.source_namespace,
        "release": selection.release,
        "split": selection.split,
        "pairingGroup": selection.pairing_group,
        "descriptor": selection.pairing_group.is_none().then_some(&selection.descriptor_path),
    });
    format!(
        "src_{}",
        blake3::hash(&serde_json::to_vec(&identity).expect("identity is serializable")).to_hex()
    )
}
