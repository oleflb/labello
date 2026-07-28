fn parse_yolo(
    source: &SourceAccess<'_>,
    request: &PreflightRequest,
    limits: &ImportLimits,
    pose: bool,
    diagnostics: &mut Diagnostics,
    decoded_memory: &DecodedImageMemoryLimiter,
    cancelled: &AtomicBool,
) -> StorageResult<ImportIr> {
    if request.descriptor_paths.len() != 1 || request.selected_splits.is_empty() {
        return Err(import_error(
            "yolo_selection_invalid",
            "YOLO requires one descriptor and at least one selected split",
        ));
    }
    let descriptor_path = &request.descriptor_paths[0];
    let object = parse_yolo_mapping(source, descriptor_path, limits)?;
    if object.contains_key("download") {
        diagnostics.add(
            "yolo_download_ignored",
            DiagnosticSeverity::Warning,
            "YAML download directive is ignored and never executed",
            false,
            false,
            false,
            Some(example_path(descriptor_path)),
        );
    }
    let names = parse_yolo_names(object.get("names"), object.get("nc"), diagnostics)?;
    if names.len() > limits.selected_categories {
        return Err(import_error(
            "category_limit",
            "YOLO categories exceed the configured limit",
        ));
    }
    let descriptor_parent = parent_source_path(descriptor_path);
    let dataset_root = match object.get("path").and_then(Value::as_str) {
        None | Some("") | Some(".") => descriptor_parent.to_string(),
        Some(path) => join_source_path(descriptor_parent, path, limits)?,
    };
    let (keypoint_count, dimensions) = if pose {
        parse_kpt_shape(object.get("kpt_shape"), limits)?
    } else {
        (0, 0)
    };
    let keypoint_names = if pose {
        parse_yolo_keypoint_names(
            object.get("kpt_names"),
            keypoint_count,
            &names,
            &request.policies,
            diagnostics,
        )?
    } else {
        BTreeMap::new()
    };
    let mut ir = ImportIr::new();
    for (index, name) in &names {
        let key = index.to_string();
        ir.categories.insert(
            key.clone(),
            IrCategory {
                key: key.clone(),
                source_namespace: request.source_namespace.clone(),
                name: name.clone(),
                source_id: key,
                supercategory: None,
                keypoint_names: keypoint_names.get(index).cloned().unwrap_or_default(),
                edges: Vec::new(),
                allow_hidden: pose && dimensions == 3,
            },
        );
    }
    let mut selections = Vec::<YoloImageSelection>::new();
    let mut selection_indices = BTreeMap::<String, usize>::new();
    for split in &request.selected_splits {
        check_cancelled(cancelled)?;
        let values = yaml_strings(object.get(split).ok_or_else(|| {
            import_error(
                "yolo_split_missing",
                "selected YOLO split is absent from the descriptor",
            )
        })?)?;
        for value in values {
            let resolved = join_source_path(&dataset_root, &value, limits)?;
            let image_paths = yolo_split_images(source, &resolved, &dataset_root, limits)?;
            for image_path in image_paths {
                if let Some(index) = selection_indices.get(&image_path).copied() {
                    if selections[index].split_memberships.insert(split.clone()) {
                        match request.policies.cross_split_duplicates {
                            CrossSplitDuplicatePolicy::Block => diagnostics.add(
                                "yolo_split_overlap",
                                DiagnosticSeverity::Error,
                                "one logical image occurs in multiple selected splits",
                                true,
                                false,
                                false,
                                Some(example_path(&image_path)),
                            ),
                            CrossSplitDuplicatePolicy::MultipleMemberships => diagnostics.add(
                                "yolo_split_overlap_membership",
                                DiagnosticSeverity::WarningRequiresAck,
                                "one logical image is retained with multiple split memberships",
                                false,
                                true,
                                false,
                                Some(example_path(&image_path)),
                            ),
                        }
                    }
                    continue;
                }
                if selections.len() >= limits.selected_images {
                    return Err(import_error(
                        "selected_image_limit",
                        "selected images exceed the configured limit",
                    ));
                }
                selection_indices.insert(image_path.clone(), selections.len());
                selections.push(YoloImageSelection {
                    source_path: image_path,
                    split_memberships: BTreeSet::from([split.clone()]),
                });
            }
        }
    }

    let mut used_labels = BTreeSet::new();
    let mut declared_label_roots = BTreeSet::new();
    let mut row_objects = BTreeMap::<String, usize>::new();
    for selection_batch in selections.chunks(limits.image_validation_workers) {
        let validation_work = selection_batch
            .iter()
            .map(|selection| {
                let registered = source.file(&selection.source_path)?;
                Ok(ImageValidationWork {
                    source_path: selection.source_path.clone(),
                    physical_path: source.physical_path(registered),
                    registered: registered.clone(),
                })
            })
            .collect::<StorageResult<Vec<_>>>()?;
        let mut validated_images = validate_images(
            validation_work,
            limits.image_validation_workers,
            limits,
            decoded_memory,
            cancelled,
        )?;
        for selection in selection_batch {
            check_cancelled(cancelled)?;
            let image_path = &selection.source_path;
            let source_key = format!("{}:{}", request.source_namespace, image_path);
            let registered = source.file(image_path)?;
            let validated = validated_images
                .remove(image_path)
                .expect("every unique YOLO image was validated")?;
            ir.images.insert(
                source_key.clone(),
                IrImage {
                    source_key: source_key.clone(),
                    file_id: registered.file_id.clone(),
                    source_path: image_path.clone(),
                    display_name: image_path
                        .rsplit('/')
                        .next()
                        .unwrap_or(image_path)
                        .to_string(),
                    split_memberships: selection.split_memberships.clone(),
                    source_namespace: request.source_namespace.clone(),
                    blake3: validated.blake3,
                    byte_size: validated.byte_size,
                    width: validated.width,
                    height: validated.height,
                    media_type: validated.media_type,
                    extension: validated.extension,
                },
            );
            let label_path = yolo_label_path(image_path)?;
            if let Some(root) = label_tree_root(&label_path) {
                declared_label_roots.insert(root);
            }
            let label = match source.file(&label_path) {
                Ok(file) => file,
                Err(_) => {
                    match request.policies.yolo_missing_labels {
                        YoloMissingLabelPolicy::Block => diagnostics.add(
                            "yolo_label_missing",
                            DiagnosticSeverity::Error,
                            "a selected image has no corresponding label file",
                            true,
                            false,
                            true,
                            Some(example_path(image_path)),
                        ),
                        YoloMissingLabelPolicy::MissingIsBackground => diagnostics.add(
                            "yolo_missing_is_background",
                            DiagnosticSeverity::WarningRequiresAck,
                            "missing label is treated as verified background",
                            false,
                            true,
                            true,
                            Some(example_path(image_path)),
                        ),
                        YoloMissingLabelPolicy::RetainIncomplete => diagnostics.add(
                            "yolo_label_missing_incomplete",
                            DiagnosticSeverity::Warning,
                            "missing label leaves image-task coverage incomplete",
                            false,
                            false,
                            true,
                            Some(example_path(image_path)),
                        ),
                    }
                    for category in names.keys() {
                        let key = coverage_key(&source_key, &category.to_string());
                        if request.policies.yolo_missing_labels
                            != YoloMissingLabelPolicy::MissingIsBackground
                        {
                            ir.coverage_overrides
                                .insert(key.clone(), ImportCoverage::Incomplete);
                        }
                        ir.equivalence_facts
                            .entry(key)
                            .or_default()
                            .insert("missing_label".to_string());
                    }
                    continue;
                }
            };
            used_labels.insert(label_path.clone());
            let mut any_rows = false;
            let mut row_ordinal = 0_usize;
            for_each_bounded_line(
                &source.physical_path(label),
                limits.yolo_line_bytes,
                |line_index, line| {
                    check_cancelled(cancelled)?;
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        return Ok(());
                    }
                    row_ordinal += 1;
                    any_rows = true;
                    let columns = trimmed.split_ascii_whitespace().collect::<Vec<_>>();
                    if columns.len() > limits.yolo_columns {
                        return Err(import_error(
                            "yolo_column_limit",
                            "YOLO row exceeds the configured column limit",
                        ));
                    }
                    let expected = if pose {
                        5 + keypoint_count * dimensions
                    } else {
                        5
                    };
                    if columns.len() != expected {
                        let code = if !pose && columns.len() == 6 {
                            "yolo_confidence_result_rejected"
                        } else {
                            "yolo_row_shape_invalid"
                        };
                        diagnostics.add(
                            code,
                            DiagnosticSeverity::Error,
                            "YOLO row has an unsupported result or geometry shape",
                            true,
                            false,
                            true,
                            Some(example_line(&label_path, line_index)),
                        );
                        return Ok(());
                    }
                    let class_index = parse_integer_token(columns[0], "yolo_class_invalid")?;
                    if !names.contains_key(&class_index) {
                        diagnostics.add(
                            "yolo_class_unknown",
                            DiagnosticSeverity::Error,
                            "YOLO row references an unknown class",
                            true,
                            false,
                            true,
                            Some(example_line(&label_path, line_index)),
                        );
                        return Ok(());
                    }
                    let numbers = columns[1..]
                        .iter()
                        .map(|value| parse_finite(value, "yolo_number_invalid"))
                        .collect::<StorageResult<Vec<_>>>()?;
                    let (mut bbox, boundary_normalized) = normalize_yolo_bbox_boundary(
                        numbers[0], numbers[1], numbers[2], numbers[3],
                    );
                    if boundary_normalized {
                        diagnostics.add(
                            "yolo_boundary_rounding_normalized",
                            DiagnosticSeverity::Info,
                            "tiny YOLO boundary rounding was normalized to the image edge",
                            false,
                            false,
                            false,
                            Some(example_line(&label_path, line_index)),
                        );
                    }
                    let clipped = validate_or_clip_box(
                        &mut bbox,
                        request.policies.geometry_bounds,
                        diagnostics,
                        &label_path,
                        line_index,
                    )?;
                    let canonical_row = columns.join(" ");
                    let duplicate_key = format!("{source_key}\0{canonical_row}");
                    if let Some(existing_index) = row_objects.get(&duplicate_key).copied() {
                        match request.policies.yolo_duplicate_rows {
                            DuplicateRowPolicy::Block => diagnostics.add(
                                "yolo_duplicate_row",
                                DiagnosticSeverity::Error,
                                "exact duplicate YOLO row is blocked in strict mode",
                                true,
                                false,
                                true,
                                Some(example_line(&label_path, line_index)),
                            ),
                            DuplicateRowPolicy::Deduplicate => {
                                diagnostics.add(
                                    "yolo_duplicate_row_deduplicated",
                                    DiagnosticSeverity::WarningRequiresAck,
                                    "exact duplicate YOLO row is deduplicated",
                                    false,
                                    true,
                                    false,
                                    Some(example_line(&label_path, line_index)),
                                );
                                ir.objects[existing_index]
                                    .row_references
                                    .push(format!("{}:{}", label_path, row_ordinal));
                            }
                        }
                        return Ok(());
                    }
                    if ir.objects.len() >= limits.annotations_total {
                        return Err(import_error(
                            "annotation_limit",
                            "source objects exceed the configured limit",
                        ));
                    }
                    let object_key = format!(
                        "{}:{}:{}:{}",
                        request.source_namespace, image_path, label.blake3, row_ordinal
                    );
                    let mut skeleton = if pose {
                        Some(parse_yolo_keypoints(
                            &numbers[4..],
                            &keypoint_names[&class_index],
                            dimensions,
                            &label_path,
                            row_ordinal,
                        )?)
                    } else {
                        None
                    };
                    if skeleton.as_ref().is_some_and(|points| {
                        points
                            .iter()
                            .all(|point| point.state == KeypointState::Absent)
                    }) {
                        let key = coverage_key(&source_key, &class_index.to_string());
                        ir.zero_keypoint_coverage.insert(key.clone());
                        ir.equivalence_facts
                            .entry(key)
                            .or_default()
                            .insert("zero_keypoints".to_string());
                        skeleton = None;
                        diagnostics.add(
                                "yolo_zero_keypoints",
                                DiagnosticSeverity::Warning,
                                "zero-keypoint object keeps its box but makes skeleton coverage incomplete",
                                false,
                                false,
                                true,
                                Some(example_line(&label_path, line_index)),
                            );
                    }
                    row_objects.insert(duplicate_key, ir.objects.len());
                    ir.objects.push(IrObject {
                        source_object_key: object_key,
                        source_namespace: request.source_namespace.clone(),
                        source_image_key: source_key.clone(),
                        source_category_key: class_index.to_string(),
                        direct_bbox: Some(bbox),
                        direct_skeleton: skeleton,
                        source_bbox: Some(vec![numbers[0], numbers[1], numbers[2], numbers[3]]),
                        source_area: None,
                        source_iscrowd: 0,
                        source_segmentation: None,
                        derived_bbox: false,
                        clipped,
                        boundary_rounding_normalized: boundary_normalized,
                        row_references: vec![format!("{}:{}", label_path, row_ordinal)],
                    });
                    Ok(())
                },
            )?;
            if !any_rows && !request.exhaustive_attested {
                for category in names.keys() {
                    ir.coverage_overrides.insert(
                        coverage_key(&source_key, &category.to_string()),
                        ImportCoverage::Incomplete,
                    );
                }
            }
        }
    }
    for label in source
        .all_files()
        .filter(|file| source_extension(&file.relative_path).as_deref() == Some("txt"))
    {
        if declared_label_roots
            .iter()
            .any(|root| below(&label.relative_path, root))
            && !used_labels.contains(&label.relative_path)
        {
            diagnostics.add(
                "yolo_orphan_label",
                DiagnosticSeverity::Error,
                "declared labels tree contains an orphan label",
                true,
                false,
                false,
                Some(example_path(&label.relative_path)),
            );
        }
    }
    Ok(ir)
}

fn parse_yolo_mapping(
    source: &SourceAccess<'_>,
    descriptor_path: &str,
    limits: &ImportLimits,
) -> StorageResult<serde_json::Map<String, Value>> {
    let bytes = source.read_limited(descriptor_path, limits.descriptor_bytes)?;
    enforce_yaml_alias_limit(&bytes, MAX_YAML_ALIASES)?;
    let yaml_value: serde_yaml_ng::Value = serde_yaml_ng::from_slice(&bytes).map_err(|error| {
        import_error(
            "yolo_yaml_invalid",
            format!("YOLO descriptor is invalid: {error}"),
        )
    })?;
    validate_yaml_value(&yaml_value, limits.structured_data_nesting, 1, &mut 0)?;
    let yaml: Value = serde_json::to_value(yaml_value).map_err(|error| {
        import_error(
            "yolo_yaml_invalid",
            format!("YOLO descriptor contains unsupported mapping keys: {error}"),
        )
    })?;
    enforce_value_depth(&yaml, limits.structured_data_nesting, "yolo_yaml_nesting")?;
    yaml.as_object()
        .cloned()
        .ok_or_else(|| import_error("yolo_yaml_invalid", "YOLO descriptor must be a mapping"))
}

pub(super) fn inspect_yolo_descriptor(
    source: &SourceAccess<'_>,
    descriptor_path: &str,
    limits: &ImportLimits,
) -> StorageResult<YoloDescriptorInspection> {
    let object = parse_yolo_mapping(source, descriptor_path, limits)?;
    let splits = YOLO_SPLIT_KEYS
        .into_iter()
        .filter_map(|name| {
            let value = object.get(name)?;
            Some(match yaml_strings(value) {
                Ok(_) => YoloSplitInspection {
                    name: name.to_string(),
                    usable: true,
                    issue: None,
                },
                Err(StorageError::Import { message, .. }) => YoloSplitInspection {
                    name: name.to_string(),
                    usable: false,
                    issue: Some(message),
                },
                Err(_) => YoloSplitInspection {
                    name: name.to_string(),
                    usable: false,
                    issue: Some("YOLO split value is invalid".to_string()),
                },
            })
        })
        .collect();
    Ok(YoloDescriptorInspection { splits })
}


fn parse_yolo_names(
    names: Option<&Value>,
    nc: Option<&Value>,
    diagnostics: &mut Diagnostics,
) -> StorageResult<BTreeMap<u64, String>> {
    let mut output = BTreeMap::new();
    match names {
        Some(Value::Array(values)) => {
            for (index, value) in values.iter().enumerate() {
                output.insert(
                    index as u64,
                    value
                        .as_str()
                        .filter(|name| !name.trim().is_empty())
                        .ok_or_else(|| {
                            import_error(
                                "yolo_names_invalid",
                                "YOLO names must be nonempty strings",
                            )
                        })?
                        .to_string(),
                );
            }
        }
        Some(Value::Object(values)) => {
            for (key, value) in values {
                let index = key.parse::<u64>().map_err(|_| {
                    import_error(
                        "yolo_names_invalid",
                        "YOLO name map keys must be zero-based integers",
                    )
                })?;
                output.insert(
                    index,
                    value
                        .as_str()
                        .filter(|name| !name.trim().is_empty())
                        .ok_or_else(|| {
                            import_error(
                                "yolo_names_invalid",
                                "YOLO names must be nonempty strings",
                            )
                        })?
                        .to_string(),
                );
            }
        }
        None if nc.and_then(Value::as_u64).is_some() => {
            let count = nc.and_then(Value::as_u64).unwrap();
            diagnostics.add(
                "yolo_generated_class_names",
                DiagnosticSeverity::WarningRequiresAck,
                "legacy nc without names generates indexed class names",
                false,
                true,
                false,
                None,
            );
            for index in 0..count {
                output.insert(index, format!("class_{index}"));
            }
        }
        _ => {
            return Err(import_error(
                "yolo_names_invalid",
                "YOLO descriptor requires names as a list or contiguous map",
            ));
        }
    }
    if output.is_empty() || output.keys().copied().ne(0..output.len() as u64) {
        return Err(import_error(
            "yolo_names_invalid",
            "YOLO names must be contiguous from zero",
        ));
    }
    Ok(output)
}

fn parse_kpt_shape(value: Option<&Value>, limits: &ImportLimits) -> StorageResult<(usize, usize)> {
    let values = value
        .and_then(Value::as_array)
        .filter(|values| values.len() == 2)
        .ok_or_else(|| {
            import_error(
                "yolo_kpt_shape_invalid",
                "YOLO pose requires kpt_shape [K, D]",
            )
        })?;
    let count = values[0]
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0 && *value <= limits.keypoints_per_skeleton)
        .ok_or_else(|| import_error("yolo_kpt_shape_invalid", "YOLO keypoint count is invalid"))?;
    let dimensions = values[1]
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| matches!(*value, 2 | 3))
        .ok_or_else(|| {
            import_error(
                "yolo_kpt_shape_invalid",
                "YOLO keypoint dimension must be 2 or 3",
            )
        })?;
    Ok((count, dimensions))
}

fn parse_yolo_keypoint_names(
    value: Option<&Value>,
    count: usize,
    categories: &BTreeMap<u64, String>,
    policies: &CompatibilityPolicies,
    diagnostics: &mut Diagnostics,
) -> StorageResult<BTreeMap<u64, Vec<String>>> {
    let parse_names = |value: &Value| {
        value.as_array().and_then(|values| {
            let names = values
                .iter()
                .map(Value::as_str)
                .collect::<Option<Vec<_>>>()?;
            (names.len() == count)
                .then(|| names.into_iter().map(str::to_string).collect::<Vec<_>>())
        })
    };
    let parsed = match value {
        Some(value @ Value::Array(_)) => parse_names(value).map(|names| {
            categories
                .keys()
                .map(|index| (*index, names.clone()))
                .collect::<BTreeMap<_, _>>()
        }),
        Some(Value::Object(values)) => {
            let mut output = BTreeMap::new();
            let mut valid = true;
            for index in categories.keys() {
                if let Some(names) = values.get(&index.to_string()).and_then(&parse_names) {
                    output.insert(*index, names);
                } else {
                    valid = false;
                    break;
                }
            }
            valid.then_some(output)
        }
        _ => None,
    };
    let names = match parsed {
        Some(names) => names,
        None if policies.yolo_keypoint_names == YoloKeypointNamePolicy::GenerateIndexed => {
            diagnostics.add(
                "yolo_keypoint_names_generated",
                DiagnosticSeverity::WarningRequiresAck,
                "YOLO keypoint names are generated by index with no inferred edges",
                false,
                true,
                false,
                None,
            );
            let generated = (0..count)
                .map(|index| format!("keypoint_{index}"))
                .collect::<Vec<_>>();
            categories
                .keys()
                .map(|index| (*index, generated.clone()))
                .collect()
        }
        None => {
            return Err(import_error(
                "yolo_keypoint_names_missing",
                "YOLO pose requires valid kpt_names or the named generated-index compatibility policy",
            ));
        }
    };
    if names.values().any(|names| {
        names.iter().any(|name| name.trim().is_empty())
            || names.iter().collect::<BTreeSet<_>>().len() != names.len()
    }) {
        return Err(import_error(
            "yolo_keypoint_names_invalid",
            "YOLO keypoint names must be unique and nonempty",
        ));
    }
    Ok(names)
}

fn yolo_split_images(
    source: &SourceAccess<'_>,
    resolved: &str,
    dataset_root: &str,
    limits: &ImportLimits,
) -> StorageResult<Vec<String>> {
    if source.file(resolved).is_ok() {
        if source_extension(resolved).as_deref() != Some("txt") {
            return if is_image_path(resolved) {
                Ok(vec![resolved.to_string()])
            } else {
                Err(import_error(
                    "yolo_split_invalid",
                    "YOLO split file must be an image manifest",
                ))
            };
        }
        let bytes = source.read_limited(resolved, limits.descriptor_bytes)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| import_error("yolo_manifest_utf8", "YOLO split manifest must be UTF-8"))?;
        let parent = parent_source_path(resolved);
        let mut images = Vec::new();
        for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
            let anchor = if line.starts_with("./") {
                parent
            } else {
                dataset_root
            };
            let path = join_source_path(anchor, line.trim_start_matches("./"), limits)?;
            if !is_image_path(&path) {
                return Err(import_error(
                    "yolo_manifest_entry_invalid",
                    "YOLO manifest entry is not a supported image path",
                ));
            }
            source.file(&path)?;
            images.push(path);
        }
        Ok(images)
    } else {
        let mut images = source
            .files_below(resolved)
            .filter(|file| is_image_path(&file.relative_path))
            .map(|file| file.relative_path.clone())
            .collect::<Vec<_>>();
        images.sort();
        if images.is_empty() {
            Err(import_error(
                "yolo_split_empty",
                "YOLO split directory contains no supported images",
            ))
        } else {
            Ok(images)
        }
    }
}

fn yolo_label_path(image_path: &str) -> StorageResult<String> {
    let mut components = image_path
        .split('/')
        .map(str::to_string)
        .collect::<Vec<_>>();
    let position = components
        .iter()
        .rposition(|component| *component == "images")
        .ok_or_else(|| {
            import_error(
                "yolo_images_component_missing",
                "YOLO image path requires an exact images component",
            )
        })?;
    components[position] = "labels".to_string();
    let last = components.last_mut().expect("nonempty path");
    let stem = last.rsplit_once('.').map(|(stem, _)| stem).ok_or_else(|| {
        import_error(
            "yolo_image_extension_missing",
            "YOLO image path requires an extension",
        )
    })?;
    *last = format!("{stem}.txt");
    Ok(components.join("/"))
}

fn label_tree_root(label_path: &str) -> Option<String> {
    let components = label_path.split('/').collect::<Vec<_>>();
    let index = components
        .iter()
        .rposition(|component| *component == "labels")?;
    Some(components[..=index].join("/"))
}

fn parse_yolo_keypoints(
    values: &[f64],
    names: &[String],
    dimensions: usize,
    path: &str,
    line: usize,
) -> StorageResult<Vec<IrKeypoint>> {
    let mut output = Vec::with_capacity(names.len());
    for (name, values) in names.iter().zip(values.chunks_exact(dimensions)) {
        let (x, y) = (values[0], values[1]);
        let state = if dimensions == 2 {
            KeypointState::Visible
        } else {
            match values[2] {
                0.0 if x == 0.0 && y == 0.0 => KeypointState::Absent,
                0.0 => {
                    return Err(import_error(
                        "yolo_visibility_coordinates_invalid",
                        "absent YOLO keypoints require zero coordinates",
                    ));
                }
                1.0 => KeypointState::Hidden,
                2.0 => KeypointState::Visible,
                _ => {
                    return Err(import_error(
                        "yolo_visibility_invalid",
                        "YOLO visibility must be 0, 1, or 2",
                    ));
                }
            }
        };
        if state != KeypointState::Absent
            && (!x.is_finite()
                || !y.is_finite()
                || !(0.0..=1.0).contains(&x)
                || !(0.0..=1.0).contains(&y))
        {
            return Err(import_error(
                "yolo_keypoint_bounds",
                format!("YOLO keypoint is outside bounds at {path}:{line}"),
            ));
        }
        output.push(IrKeypoint {
            name: name.clone(),
            x: (state != KeypointState::Absent).then_some(x),
            y: (state != KeypointState::Absent).then_some(y),
            state,
        });
    }
    Ok(output)
}

type CocoSkeletonSchema = (Vec<String>, Vec<(String, String)>);
