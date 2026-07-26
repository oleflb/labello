use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, Read},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::Instant,
};

use labello_domain::{
    ImportCoverage, ImportGeometryKind, ImportGeometryPolicy, KeypointState, SCHEMA_VERSION,
};
use serde_json::Value;

use super::{
    image_validation::{DecodedImageMemoryLimiter, validate_image},
    ir::{F64Box, ImportIr, IrCategory, IrImage, IrKeypoint, IrObject},
    source::{
        SourceAccess, SourceIndex, import_error, join_source_path, parent_source_path,
        source_extension,
    },
    types::*,
};
use crate::{StorageError, StorageResult};

const MAX_STRUCTURED_NODES: usize = 1_000_000;
const MAX_STRUCTURED_VALUE_BYTES: usize = 1024 * 1024;
const MAX_YAML_ALIASES: usize = 0;
const YOLO_SPLIT_KEYS: [&str; 3] = ["train", "val", "test"];
pub(super) const YOLO_BOUNDARY_ROUNDING_TOLERANCE: f64 = 1e-6;

pub(super) struct PreflightOutput {
    pub plan: ImportPlan,
    pub ir: ImportIr,
    pub timings: PreflightTimings,
}

pub(super) struct PreflightTimings {
    pub parse_ms: u64,
    pub semantic_validation_ms: u64,
    pub plan_assembly_ms: u64,
    pub plan_hash_ms: u64,
}

struct ImageValidationWork {
    source_path: String,
    physical_path: PathBuf,
    registered: RegisteredFile,
}

struct YoloImageSelection {
    source_path: String,
    split_memberships: BTreeSet<String>,
}

pub(super) fn preflight(
    job_dir: &std::path::Path,
    index: &SourceIndex,
    job: &ImportJob,
    mut request: PreflightRequest,
    limits: &ImportLimits,
    decoded_memory: &DecodedImageMemoryLimiter,
    cancelled: &AtomicBool,
) -> StorageResult<PreflightOutput> {
    check_cancelled(cancelled)?;
    if !index.sealed || index.source_fingerprint.as_deref() != job.source_fingerprint.as_deref() {
        return Err(import_error(
            "source_not_sealed",
            "preflight requires the matching sealed source",
        ));
    }
    let source = SourceAccess::new(job_dir, index);
    let mut diagnostics = Diagnostics::new(job.profile, limits.diagnostic_examples_per_code);
    let parse_started = Instant::now();
    let parsed = match job.profile {
        ImportProfile::UltralyticsYoloDetectV1 => parse_yolo(
            &source,
            &request,
            limits,
            false,
            &mut diagnostics,
            decoded_memory,
            cancelled,
        ),
        ImportProfile::UltralyticsYoloPoseV1 => parse_yolo(
            &source,
            &request,
            limits,
            true,
            &mut diagnostics,
            decoded_memory,
            cancelled,
        ),
        ImportProfile::CocoInstancesGtV1 => parse_coco(
            &source,
            &request,
            limits,
            false,
            &mut diagnostics,
            decoded_memory,
            cancelled,
        ),
        ImportProfile::CocoKeypointsGtV1 => parse_coco(
            &source,
            &request,
            limits,
            true,
            &mut diagnostics,
            decoded_memory,
            cancelled,
        ),
    };
    let mut ir = match parsed {
        Ok(ir) => ir,
        Err(error) => {
            let (code, summary) = match &error {
                StorageError::Import { code, message } => (code.clone(), message.clone()),
                _ => (
                    error.kind().to_string(),
                    "source parsing failed".to_string(),
                ),
            };
            diagnostics.add(
                &code,
                DiagnosticSeverity::Error,
                &summary,
                true,
                false,
                false,
                None,
            );
            ImportIr::new()
        }
    };
    let parse_ms = elapsed_ms(parse_started);
    check_cancelled(cancelled)?;
    let semantic_validation_started = Instant::now();
    resolve_coverage_scope(&ir, &mut request, &mut diagnostics);
    validate_mappings(&ir, &request, &mut diagnostics);
    validate_duplicate_images(&mut ir, &request, &mut diagnostics)?;
    enforce_ir_limits(&ir, &request, limits, &mut diagnostics);
    check_cancelled(cancelled)?;
    let semantic_validation_ms = elapsed_ms(semantic_validation_started);
    let plan_assembly_started = Instant::now();
    let (class_ids, task_ids) = planned_ids(&ir, &request);
    let output_tasks = task_ids.values().map(Vec::len).sum();
    let output_annotations = estimated_annotations(&ir, &request, &class_ids);
    let coverage = coverage_totals(&ir, &request, &task_ids, cancelled)?;
    let mut direct_geometry_by_category = BTreeMap::<&str, (bool, bool)>::new();
    for object in &ir.objects {
        let direct = direct_geometry_by_category
            .entry(&object.source_category_key)
            .or_default();
        direct.0 |= object.direct_bbox.is_some();
        direct.1 |= object.direct_skeleton.is_some();
    }
    let source_categories = ir
        .categories
        .iter()
        .map(|(key, category)| {
            let direct = direct_geometry_by_category
                .get(key.as_str())
                .copied()
                .unwrap_or_default();
            (
                key.clone(),
                ImportSourceCategory {
                    source_namespace: category.source_namespace.clone(),
                    source_category_id: category.source_id.clone(),
                    source_name: category.name.clone(),
                    source_supercategory: category.supercategory.clone(),
                    direct_bounding_boxes: direct.0,
                    direct_skeletons: direct.1,
                    keypoint_names: category.keypoint_names.clone(),
                    edges: category.edges.clone(),
                    allow_hidden: category.allow_hidden,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let (clipped_geometry, envelope_derived, template_derived) =
        estimated_derived_geometry(&ir, &request, &class_ids);
    let totals = ImportTotals {
        source_files: index.files.len(),
        source_bytes: index.files.values().map(|file| file.byte_size).sum(),
        descriptors: request
            .descriptor_paths
            .len()
            .max(request.coco_descriptors.len()),
        images: ir
            .images
            .values()
            .map(|image| &image.blake3)
            .collect::<BTreeSet<_>>()
            .len(),
        categories: ir.categories.len(),
        source_objects: ir.objects.len(),
        keypoints: ir
            .objects
            .iter()
            .filter_map(|object| object.direct_skeleton.as_ref())
            .map(Vec::len)
            .sum(),
        direct_boxes: ir
            .objects
            .iter()
            .filter(|object| object.direct_bbox.is_some())
            .count(),
        direct_skeletons: ir
            .objects
            .iter()
            .filter(|object| object.direct_skeleton.is_some())
            .count(),
        derived_geometry: clipped_geometry + envelope_derived + template_derived,
        clipped_geometry,
        envelope_derived,
        template_derived,
        output_tasks,
        output_annotations,
        estimated_output_bytes: ir
            .images
            .values()
            .fold(BTreeMap::<&str, u64>::new(), |mut images, image| {
                images.entry(&image.blake3).or_insert(image.byte_size);
                images
            })
            .into_values()
            .sum::<u64>()
            .saturating_add((output_annotations as u64).saturating_mul(4096))
            .saturating_add((ir.images.len() as u64).saturating_mul(4096)),
    };
    if ir.discarded_segmentation > 0 {
        diagnostics.add_count(
            "coco_segmentation_discarded",
            DiagnosticSeverity::Warning,
            "segmentation metadata is retained only as a reported discarded feature",
            ir.discarded_segmentation,
            false,
            false,
            false,
        );
    }
    let safety_margin = totals.estimated_output_bytes / 10 + 64 * 1024 * 1024;
    let peak_staged_bytes = totals
        .source_bytes
        .saturating_add(totals.estimated_output_bytes);
    if peak_staged_bytes > limits.staged_bytes {
        diagnostics.add(
            "staging_quota_exceeded",
            DiagnosticSeverity::Error,
            "staged source plus estimated native output exceeds the configured staging quota",
            true,
            false,
            false,
            None,
        );
    }
    if free_space_bytes(job_dir)
        .is_some_and(|free| totals.estimated_output_bytes.saturating_add(safety_margin) > free)
    {
        diagnostics.add(
            "free_space_insufficient",
            DiagnosticSeverity::Error,
            "available filesystem space is below the output estimate and safety margin",
            true,
            false,
            false,
            None,
        );
    }
    let source_fingerprint = job.source_fingerprint.clone().expect("validated above");
    let diagnostics = diagnostics.finish();
    check_cancelled(cancelled)?;
    let hash_input = serde_json::json!({
        "domain": "labello:import-plan:v1",
        "importId": job.import_id,
        "destinationDatasetId": job.destination_dataset_id,
        "profile": job.profile,
        "sourceFingerprint": source_fingerprint,
        "parserVersion": IMPORT_PARSER_VERSION,
        "request": request,
        "totals": totals,
        "coverage": coverage,
        "sourceCategories": source_categories,
        "classIds": class_ids,
        "taskIds": task_ids,
        "diagnostics": diagnostics,
    });
    let plan_assembly_ms = elapsed_ms(plan_assembly_started);
    let plan_hash_started = Instant::now();
    let canonical = serde_json::to_vec(&hash_input).map_err(|source| StorageError::Json {
        path: job_dir.join("plan.json"),
        source,
    })?;
    let plan_hash = blake3::hash(&canonical).to_hex().to_string();
    let plan_hash_ms = elapsed_ms(plan_hash_started);
    let plan = ImportPlan {
        schema_version: SCHEMA_VERSION,
        import_id: job.import_id.clone(),
        destination_dataset_id: job.destination_dataset_id.clone(),
        source_fingerprint,
        plan_hash,
        request,
        totals,
        coverage,
        diagnostics,
        source_categories,
        class_ids,
        task_ids,
    };
    Ok(PreflightOutput {
        plan,
        ir,
        timings: PreflightTimings {
            parse_ms,
            semantic_validation_ms,
            plan_assembly_ms,
            plan_hash_ms,
        },
    })
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn check_cancelled(cancelled: &AtomicBool) -> StorageResult<()> {
    if cancelled.load(Ordering::Relaxed) {
        Err(import_error(
            "parser_cancelled",
            "import parsing was cancelled",
        ))
    } else {
        Ok(())
    }
}

fn validate_images(
    work: Vec<ImageValidationWork>,
    worker_limit: usize,
    limits: &ImportLimits,
    decoded_memory: &DecodedImageMemoryLimiter,
    cancelled: &AtomicBool,
) -> StorageResult<BTreeMap<String, StorageResult<super::image_validation::ValidatedImage>>> {
    check_cancelled(cancelled)?;
    if work.is_empty() {
        return Ok(BTreeMap::new());
    }
    let worker_count = worker_limit.min(work.len()).max(1);
    if worker_count == 1 {
        return Ok(work
            .into_iter()
            .map(|work| {
                let validated = check_cancelled(cancelled).and_then(|()| {
                    validate_image(
                        &work.physical_path,
                        &work.source_path,
                        &work.registered,
                        limits,
                        decoded_memory,
                        cancelled,
                    )
                });
                (work.source_path, validated)
            })
            .collect());
    }

    let next = AtomicUsize::new(0);
    let lowest_error = AtomicUsize::new(usize::MAX);
    let (sender, receiver) = mpsc::channel();
    let mut results = std::iter::repeat_with(|| None)
        .take(work.len())
        .collect::<Vec<_>>();
    std::thread::scope(|scope| {
        for _ in 0..worker_count {
            let sender = sender.clone();
            let work = &work;
            let next = &next;
            let lowest_error = &lowest_error;
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = work.get(index) else {
                        break;
                    };
                    let result = check_cancelled(cancelled).and_then(|()| {
                        if index > lowest_error.load(Ordering::Relaxed) {
                            return Err(import_error(
                                "parser_cancelled",
                                "image validation stopped after an earlier error",
                            ));
                        }
                        validate_image(
                            &item.physical_path,
                            &item.source_path,
                            &item.registered,
                            limits,
                            decoded_memory,
                            cancelled,
                        )
                    });
                    if result.is_err() {
                        lowest_error.fetch_min(index, Ordering::Relaxed);
                    }
                    if sender.send((index, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);
        for (index, result) in receiver {
            results[index] = Some(result);
        }
    });

    Ok(work
        .into_iter()
        .zip(results)
        .map(|(work, result)| {
            (
                work.source_path,
                result.expect("every image validation worker returned a result"),
            )
        })
        .collect())
}

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

fn validate_duplicate_images(
    ir: &mut ImportIr,
    request: &PreflightRequest,
    diagnostics: &mut Diagnostics,
) -> StorageResult<()> {
    let mut by_hash: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for image in ir.images.values() {
        by_hash
            .entry(image.blake3.clone())
            .or_default()
            .push(image.source_key.clone());
    }
    for keys in by_hash.values().filter(|keys| keys.len() > 1) {
        let signatures = keys
            .iter()
            .map(|key| object_signature(ir, key, request))
            .collect::<BTreeSet<_>>();
        if signatures.len() != 1 {
            diagnostics.add(
                "duplicate_image_divergent_annotations",
                DiagnosticSeverity::Error,
                "equal image bytes have divergent annotation sets",
                true,
                false,
                true,
                None,
            );
            continue;
        }
        let splits = keys
            .iter()
            .flat_map(|key| ir.images[key].split_memberships.iter())
            .collect::<BTreeSet<_>>();
        if splits.len() > 1 {
            match request.policies.cross_split_duplicates {
                CrossSplitDuplicatePolicy::Block => diagnostics.add(
                    "cross_split_duplicate_image",
                    DiagnosticSeverity::Error,
                    "equal image bytes occur across selected splits",
                    true,
                    false,
                    false,
                    None,
                ),
                CrossSplitDuplicatePolicy::MultipleMemberships => diagnostics.add(
                    "cross_split_duplicate_membership",
                    DiagnosticSeverity::WarningRequiresAck,
                    "equal image bytes are imported with multiple split memberships",
                    false,
                    true,
                    false,
                    None,
                ),
            }
        }
    }
    Ok(())
}

fn object_signature(ir: &ImportIr, source_image_key: &str, request: &PreflightRequest) -> String {
    let mut values = ir
        .objects
        .iter()
        .filter(|object| object.source_image_key == source_image_key)
        .map(|object| {
            format!(
                "{}:{:?}:{:?}:{:?}:{:?}:{}:{:?}:{}:{}",
                object.source_category_key,
                object.direct_bbox,
                object.direct_skeleton,
                object.source_bbox,
                object.source_area,
                object.source_iscrowd,
                object.source_segmentation,
                object.derived_bbox,
                object.clipped
            )
        })
        .collect::<Vec<_>>();
    values.sort();
    let mut category_facts = ir
        .categories
        .keys()
        .map(|category_key| {
            let key = coverage_key(source_image_key, category_key);
            let objects = ir
                .objects
                .iter()
                .filter(|object| {
                    object.source_image_key == source_image_key
                        && object.source_category_key == *category_key
                })
                .collect::<Vec<_>>();
            format!(
                "{category_key}:{:?}:{:?}:{:?}:{:?}:{}",
                coverage_for(ir, source_image_key, category_key, &objects, false, request),
                coverage_for(ir, source_image_key, category_key, &objects, true, request),
                ir.coverage_overrides.get(&key),
                ir.equivalence_facts.get(&key),
                ir.zero_keypoint_coverage.contains(&key)
            )
        })
        .collect::<Vec<_>>();
    category_facts.sort();
    format!("{}\n{}", values.join("|"), category_facts.join("|"))
}

fn enforce_ir_limits(
    ir: &ImportIr,
    request: &PreflightRequest,
    limits: &ImportLimits,
    diagnostics: &mut Diagnostics,
) {
    let (class_ids, _) = planned_ids(ir, request);
    let checks = [
        (
            ir.images.len() > limits.selected_images,
            "selected_image_limit",
            "selected images exceed the configured limit",
        ),
        (
            ir.categories.len() > limits.selected_categories,
            "category_limit",
            "selected categories exceed the configured limit",
        ),
        (
            ir.objects.len() > limits.annotations_total,
            "annotation_limit",
            "source objects exceed the configured annotation limit",
        ),
        (
            ir.images.len().saturating_mul(ir.categories.len()) > limits.coverage_entries,
            "coverage_limit",
            "image-task coverage entries exceed the configured limit",
        ),
        (
            estimated_annotations(ir, request, &class_ids) > limits.annotations_total,
            "annotation_limit",
            "output annotations exceed the configured limit",
        ),
    ];
    for (failed, code, summary) in checks {
        if failed {
            diagnostics.add(
                code,
                DiagnosticSeverity::Error,
                summary,
                true,
                false,
                false,
                None,
            );
        }
    }
    for object in &ir.objects {
        let Some(ResolvedGeometryPolicy::KeypointEnvelopeV1 {
            padding_ratio,
            minimum_pixels,
            include_hidden,
        }) = geometry_policy(
            request,
            &object.source_category_key,
            ImportGeometryKind::BoundingBox,
        )
        else {
            continue;
        };
        let envelope = object.direct_skeleton.as_deref().and_then(|points| {
            let image = ir.images.get(&object.source_image_key)?;
            keypoint_envelope(
                points,
                image.width,
                image.height,
                padding_ratio,
                minimum_pixels,
                include_hidden,
            )
            .ok()
        });
        if envelope.is_none() {
            diagnostics.add(
                "keypoint_envelope_invalid",
                DiagnosticSeverity::Error,
                "keypoint envelope requires two selected points with nonzero horizontal and vertical span",
                true,
                false,
                true,
                None,
            );
        } else if envelope.is_some_and(|(_, clipped)| clipped) {
            diagnostics.add(
                "keypoint_envelope_clipped",
                DiagnosticSeverity::WarningRequiresAck,
                "keypoint-envelope padding crosses image bounds and is clipped",
                false,
                true,
                true,
                None,
            );
        }
    }
    let mut per_image = BTreeMap::new();
    for object in &ir.objects {
        *per_image.entry(&object.source_image_key).or_insert(0_usize) += 1;
    }
    if per_image
        .values()
        .any(|count| *count > limits.annotations_per_image)
    {
        diagnostics.add(
            "annotations_per_image_limit",
            DiagnosticSeverity::Error,
            "an image exceeds the configured annotation limit",
            true,
            false,
            false,
            None,
        );
    }
    if selected_policies(request)
        .into_iter()
        .any(|policy| matches!(policy, ResolvedGeometryPolicy::BoxRelativeTemplateV1 { .. }))
    {
        diagnostics.add(
            "template_skeleton_derived",
            DiagnosticSeverity::WarningRequiresAck,
            "box-relative template skeletons are derived pending seeds",
            false,
            true,
            true,
            None,
        );
    }
    if selected_policies(request)
        .into_iter()
        .any(|policy| matches!(policy, ResolvedGeometryPolicy::KeypointEnvelopeV1 { .. }))
    {
        diagnostics.add(
            "keypoint_envelope_derived",
            DiagnosticSeverity::WarningRequiresAck,
            "keypoint-envelope boxes are derived pending seeds",
            false,
            true,
            true,
            None,
        );
    }
    let manual_categories = if request.geometry_mappings.is_empty()
        && matches!(
            request.output.box_to_skeleton,
            BoxToSkeletonPolicy::ManualBoxGuide { .. }
        ) {
        ir.categories.keys().map(String::as_str).collect::<Vec<_>>()
    } else if request.geometry_mappings.is_empty() {
        Vec::new()
    } else {
        manual_category_keys(request)
    };
    for category_key in manual_categories {
        let schema_invalid = if let BoxToSkeletonPolicy::ManualBoxGuide {
            keypoint_names,
            edges,
        } = &request.output.box_to_skeleton
            && request.geometry_mappings.is_empty()
        {
            let names = keypoint_names.iter().collect::<BTreeSet<_>>();
            keypoint_names.is_empty()
                || names.len() != keypoint_names.len()
                || keypoint_names.iter().any(|name| name.trim().is_empty())
                || edges
                    .iter()
                    .any(|(from, to)| from == to || !names.contains(from) || !names.contains(to))
        } else {
            request
                .task_mappings
                .iter()
                .find(|mapping| {
                    mapping.source_category_key == category_key
                        && mapping.task.annotation_type == labello_domain::AnnotationType::Skeleton
                })
                .and_then(|mapping| mapping.task.skeleton.as_ref())
                .is_none_or(|skeleton| {
                    let names = skeleton
                        .keypoints
                        .iter()
                        .map(|point| &point.name)
                        .collect::<BTreeSet<_>>();
                    skeleton.keypoints.is_empty()
                        || names.len() != skeleton.keypoints.len()
                        || skeleton
                            .keypoints
                            .iter()
                            .any(|point| point.name.trim().is_empty())
                        || skeleton.edges.iter().any(|edge| {
                            edge.from == edge.to
                                || !names.contains(&edge.from)
                                || !names.contains(&edge.to)
                        })
                })
        };
        let guides_invalid = !request.output.bounding_boxes
            || !request.output.skeletons
            || !request.exhaustive_attested
            || ir
                .objects
                .iter()
                .filter(|object| object.source_category_key == category_key)
                .any(|object| {
                    object.direct_bbox.is_none()
                        || object.clipped
                        || object.direct_skeleton.is_some()
                });
        if schema_invalid {
            diagnostics.add(
                "manual_migration_schema_invalid",
                DiagnosticSeverity::Error,
                "manual box-guide migration requires a valid explicit skeleton schema",
                true,
                false,
                false,
                None,
            );
        }
        if guides_invalid {
            diagnostics.add("manual_migration_guide_incomplete", DiagnosticSeverity::Error, "manual box-guide migration requires exhaustive direct box coverage without source skeletons or transforms", true, false, true, None);
        }
    }
    if request.policies.geometry_bounds == GeometryBoundsPolicy::ClipDerived {
        diagnostics.add(
            "geometry_clipping_enabled",
            DiagnosticSeverity::WarningRequiresAck,
            "out-of-bounds geometry is clipped as a derived pending seed",
            false,
            true,
            true,
            None,
        );
    }
    if request.policies.coco_bbox_only {
        diagnostics.add(
            "coco_bbox_compatibility",
            DiagnosticSeverity::WarningRequiresAck,
            "COCO bbox-only compatibility may synthesize canonical fields",
            false,
            true,
            false,
            None,
        );
    }
}

fn resolve_coverage_scope(
    ir: &ImportIr,
    request: &mut PreflightRequest,
    diagnostics: &mut Diagnostics,
) {
    let requested = if request.coverage_scope.is_empty() {
        ir.categories.keys().cloned().collect::<Vec<_>>()
    } else {
        request.coverage_scope.clone()
    };
    let mut resolved = BTreeSet::new();
    let mut invalid = false;
    for value in requested {
        let matches = if ir.categories.contains_key(&value) {
            BTreeSet::from([value.clone()])
        } else {
            ir.categories
                .iter()
                .filter(|(_, category)| category.source_id == value || category.name == value)
                .map(|(key, _)| key.clone())
                .collect::<BTreeSet<_>>()
        };
        if matches.len() != 1 {
            invalid = true;
        } else {
            resolved.extend(matches);
        }
    }
    request.coverage_scope = resolved.into_iter().collect();
    if invalid {
        diagnostics.add(
            "coverage_scope_invalid",
            DiagnosticSeverity::Error,
            "coverage scope entries must resolve unambiguously to discovered category keys",
            true,
            false,
            true,
            None,
        );
    }
}

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

fn estimated_annotations(
    ir: &ImportIr,
    request: &PreflightRequest,
    class_ids: &BTreeMap<String, String>,
) -> usize {
    let canonical = ir
        .images
        .values()
        .fold(BTreeMap::<&str, &str>::new(), |mut values, image| {
            values
                .entry(&image.blake3)
                .and_modify(|key| {
                    if image.source_key.as_str() < *key {
                        *key = &image.source_key;
                    }
                })
                .or_insert(&image.source_key);
            values
        })
        .into_values()
        .collect::<BTreeSet<_>>();
    ir.objects
        .iter()
        .filter(|object| {
            canonical.contains(object.source_image_key.as_str())
                && class_ids.contains_key(&object.source_category_key)
        })
        .map(|object| {
            let mapped_type = |annotation_type| {
                request.task_mappings.is_empty()
                    || request.task_mappings.iter().any(|mapping| {
                        mapping.source_category_key == object.source_category_key
                            && mapping.task.annotation_type == annotation_type
                    })
            };
            let boxes = geometry_policy(
                request,
                &object.source_category_key,
                ImportGeometryKind::BoundingBox,
            )
            .is_some_and(|policy| {
                mapped_type(labello_domain::AnnotationType::BoundingBox)
                    && match policy {
                        ResolvedGeometryPolicy::Direct => object.direct_bbox.is_some(),
                        ResolvedGeometryPolicy::KeypointEnvelopeV1 { .. } => {
                            object.direct_skeleton.is_some()
                        }
                        _ => false,
                    }
            });
            let skeletons = geometry_policy(
                request,
                &object.source_category_key,
                ImportGeometryKind::Skeleton,
            )
            .is_some_and(|policy| {
                mapped_type(labello_domain::AnnotationType::Skeleton)
                    && match policy {
                        ResolvedGeometryPolicy::Direct => object.direct_skeleton.is_some(),
                        ResolvedGeometryPolicy::BoxRelativeTemplateV1 { .. } => {
                            object.direct_bbox.is_some()
                        }
                        _ => false,
                    }
            });
            usize::from(boxes) + usize::from(skeletons)
        })
        .sum()
}

fn coverage_totals(
    ir: &ImportIr,
    request: &PreflightRequest,
    task_ids: &BTreeMap<String, Vec<String>>,
    cancelled: &AtomicBool,
) -> StorageResult<labello_domain::ImportCoverageTotals> {
    let canonical = ir
        .images
        .values()
        .fold(BTreeMap::<&str, &str>::new(), |mut values, image| {
            values
                .entry(&image.blake3)
                .and_modify(|key| {
                    if image.source_key.as_str() < *key {
                        *key = &image.source_key;
                    }
                })
                .or_insert(&image.source_key);
            values
        })
        .into_values()
        .collect::<Vec<_>>();
    let mut totals = labello_domain::ImportCoverageTotals::default();
    let mut objects_by_image_category = BTreeMap::<(&str, &str), Vec<&IrObject>>::new();
    for object in &ir.objects {
        objects_by_image_category
            .entry((
                object.source_image_key.as_str(),
                object.source_category_key.as_str(),
            ))
            .or_default()
            .push(object);
    }
    for source_image_key in canonical {
        check_cancelled(cancelled)?;
        for (category_key, ids) in task_ids {
            let objects = objects_by_image_category
                .get(&(source_image_key, category_key.as_str()))
                .map(Vec::as_slice)
                .unwrap_or_default();
            for task_id in ids {
                let annotation_type = request
                    .task_mappings
                    .iter()
                    .find(|mapping| mapping.task.task_id.as_str() == task_id)
                    .map(|mapping| mapping.task.annotation_type.clone())
                    .unwrap_or_else(|| {
                        if task_id.starts_with("skeleton:") {
                            labello_domain::AnnotationType::Skeleton
                        } else {
                            labello_domain::AnnotationType::BoundingBox
                        }
                    });
                let skeleton = annotation_type == labello_domain::AnnotationType::Skeleton;
                let coverage = coverage_for(
                    ir,
                    source_image_key,
                    category_key,
                    objects,
                    skeleton,
                    request,
                );
                let counts = if skeleton {
                    &mut totals.skeletons
                } else {
                    &mut totals.bounding_boxes
                };
                match coverage {
                    ImportCoverage::Complete => counts.complete += 1,
                    ImportCoverage::VerifiedEmpty => counts.verified_empty += 1,
                    ImportCoverage::Incomplete => counts.incomplete += 1,
                    ImportCoverage::Excluded => counts.excluded += 1,
                }
            }
        }
    }
    Ok(totals)
}

pub(super) fn coverage_for(
    ir: &ImportIr,
    source_image_key: &str,
    category_key: &str,
    objects: &[&IrObject],
    skeleton: bool,
    request: &PreflightRequest,
) -> ImportCoverage {
    if let Some(value) = ir
        .coverage_overrides
        .get(&coverage_key(source_image_key, category_key))
    {
        return *value;
    }
    if skeleton
        && ir
            .zero_keypoint_coverage
            .contains(&coverage_key(source_image_key, category_key))
    {
        return ImportCoverage::Incomplete;
    }
    if skeleton
        && objects.iter().any(|object| {
            object.direct_skeleton.as_ref().is_some_and(|points| {
                points
                    .iter()
                    .all(|point| point.state == labello_domain::KeypointState::Absent)
            })
        })
    {
        return ImportCoverage::Incomplete;
    }
    let target = if skeleton {
        ImportGeometryKind::Skeleton
    } else {
        ImportGeometryKind::BoundingBox
    };
    let derived_policy = geometry_policy(request, category_key, target).is_some_and(|policy| {
        matches!(
            policy,
            ResolvedGeometryPolicy::KeypointEnvelopeV1 { .. }
                | ResolvedGeometryPolicy::BoxRelativeTemplateV1 { .. }
                | ResolvedGeometryPolicy::ManualBoxGuideV1
        )
    });
    if objects.iter().any(|object| object.clipped) || (derived_policy && !objects.is_empty()) {
        return ImportCoverage::Incomplete;
    }
    let exhaustive = request.exhaustive_attested
        && request
            .coverage_scope
            .iter()
            .any(|covered| covered == category_key);
    if objects.is_empty() {
        if exhaustive {
            ImportCoverage::VerifiedEmpty
        } else {
            ImportCoverage::Incomplete
        }
    } else if exhaustive {
        ImportCoverage::Complete
    } else {
        ImportCoverage::Incomplete
    }
}

#[derive(Clone, Copy)]
pub(super) enum ResolvedGeometryPolicy<'a> {
    Direct,
    KeypointEnvelopeV1 {
        padding_ratio: f64,
        minimum_pixels: u32,
        include_hidden: bool,
    },
    ManualBoxGuideV1,
    BoxRelativeTemplateV1 {
        keypoints: &'a [TemplateKeypoint],
    },
}

pub(super) fn geometry_policy<'a>(
    request: &'a PreflightRequest,
    category_key: &str,
    target: ImportGeometryKind,
) -> Option<ResolvedGeometryPolicy<'a>> {
    if !request.geometry_mappings.is_empty() {
        let mapping = request.geometry_mappings.iter().find(|mapping| {
            mapping.source_category_key == category_key && mapping.target_geometry == target
        })?;
        return match &mapping.policy {
            ImportGeometryPolicy::Direct => Some(ResolvedGeometryPolicy::Direct),
            ImportGeometryPolicy::KeypointEnvelopeV1 {
                padding_ratio,
                minimum_pixels,
                include_hidden,
            } => Some(ResolvedGeometryPolicy::KeypointEnvelopeV1 {
                padding_ratio: *padding_ratio,
                minimum_pixels: *minimum_pixels,
                include_hidden: *include_hidden,
            }),
            ImportGeometryPolicy::ManualBoxGuideV1 => {
                Some(ResolvedGeometryPolicy::ManualBoxGuideV1)
            }
            ImportGeometryPolicy::BoxRelativeTemplateV1 { keypoints } => {
                Some(ResolvedGeometryPolicy::BoxRelativeTemplateV1 { keypoints })
            }
            ImportGeometryPolicy::Omit => None,
        };
    }
    match target {
        ImportGeometryKind::BoundingBox if request.output.bounding_boxes => {
            Some(ResolvedGeometryPolicy::Direct)
        }
        ImportGeometryKind::Skeleton if request.output.skeletons => {
            match &request.output.box_to_skeleton {
                BoxToSkeletonPolicy::Template { keypoints } => {
                    Some(ResolvedGeometryPolicy::BoxRelativeTemplateV1 { keypoints })
                }
                BoxToSkeletonPolicy::ManualBoxGuide { .. } => {
                    Some(ResolvedGeometryPolicy::ManualBoxGuideV1)
                }
                BoxToSkeletonPolicy::None => Some(ResolvedGeometryPolicy::Direct),
            }
        }
        _ => None,
    }
}

fn selected_policies(request: &PreflightRequest) -> Vec<ResolvedGeometryPolicy<'_>> {
    if request.geometry_mappings.is_empty() {
        let mut policies = Vec::new();
        if request.output.bounding_boxes {
            policies.push(ResolvedGeometryPolicy::Direct);
        }
        if request.output.skeletons {
            match &request.output.box_to_skeleton {
                BoxToSkeletonPolicy::Template { keypoints } => {
                    policies.push(ResolvedGeometryPolicy::BoxRelativeTemplateV1 { keypoints });
                }
                BoxToSkeletonPolicy::ManualBoxGuide { .. } => {
                    policies.push(ResolvedGeometryPolicy::ManualBoxGuideV1);
                }
                BoxToSkeletonPolicy::None => policies.push(ResolvedGeometryPolicy::Direct),
            }
        }
        return policies;
    }
    let categories = request
        .category_mappings
        .iter()
        .filter(|mapping| mapping.selected)
        .map(|mapping| mapping.source_category_key.as_str())
        .collect::<BTreeSet<_>>();
    let mappings = request.geometry_mappings.iter().filter(move |mapping| {
        request.category_mappings.is_empty()
            || categories.contains(mapping.source_category_key.as_str())
    });
    mappings
        .filter_map(|mapping| {
            geometry_policy(
                request,
                &mapping.source_category_key,
                mapping.target_geometry,
            )
        })
        .collect()
}

fn manual_category_keys(request: &PreflightRequest) -> Vec<&str> {
    if request.geometry_mappings.is_empty() {
        return matches!(
            request.output.box_to_skeleton,
            BoxToSkeletonPolicy::ManualBoxGuide { .. }
        )
        .then(|| {
            request
                .category_mappings
                .iter()
                .filter(|mapping| mapping.selected)
                .map(|mapping| mapping.source_category_key.as_str())
                .collect()
        })
        .unwrap_or_default();
    }
    request
        .geometry_mappings
        .iter()
        .filter(|mapping| matches!(mapping.policy, ImportGeometryPolicy::ManualBoxGuideV1))
        .map(|mapping| mapping.source_category_key.as_str())
        .collect()
}

fn valid_geometry_policy(
    mapping: &labello_domain::ImportGeometryMapping,
    request: &PreflightRequest,
    ir: &ImportIr,
) -> bool {
    let target_task = request.task_mappings.iter().find(|task| {
        task.source_category_key == mapping.source_category_key
            && geometry_kind_for_annotation(&task.task.annotation_type) == mapping.target_geometry
    });
    let category = ir.categories.get(&mapping.source_category_key);
    match &mapping.policy {
        ImportGeometryPolicy::Direct => {
            mapping.source_geometry == mapping.target_geometry
                && match mapping.target_geometry {
                    ImportGeometryKind::BoundingBox => true,
                    ImportGeometryKind::Skeleton => {
                        category.zip(target_task).is_some_and(|(category, task)| {
                            task.task.skeleton.as_ref().is_some_and(|skeleton| {
                                category.keypoint_names
                                    == skeleton
                                        .keypoints
                                        .iter()
                                        .map(|point| point.name.clone())
                                        .collect::<Vec<_>>()
                            })
                        })
                    }
                }
        }
        ImportGeometryPolicy::KeypointEnvelopeV1 {
            padding_ratio,
            minimum_pixels,
            ..
        } => {
            mapping.source_geometry == ImportGeometryKind::Skeleton
                && mapping.target_geometry == ImportGeometryKind::BoundingBox
                && padding_ratio.is_finite()
                && (0.0..=1.0).contains(padding_ratio)
                && *minimum_pixels > 0
                && category.is_some_and(|category| !category.keypoint_names.is_empty())
        }
        ImportGeometryPolicy::ManualBoxGuideV1 => {
            mapping.source_geometry == ImportGeometryKind::BoundingBox
                && mapping.target_geometry == ImportGeometryKind::Skeleton
                && category.is_some_and(|category| category.keypoint_names.is_empty())
        }
        ImportGeometryPolicy::BoxRelativeTemplateV1 { keypoints } => {
            mapping.source_geometry == ImportGeometryKind::BoundingBox
                && mapping.target_geometry == ImportGeometryKind::Skeleton
                && !keypoints.is_empty()
                && keypoints.iter().all(|point| {
                    !point.name.trim().is_empty()
                        && point.x.is_finite()
                        && point.y.is_finite()
                        && (0.0..=1.0).contains(&point.x)
                        && (0.0..=1.0).contains(&point.y)
                })
                && target_task.is_some_and(|task| {
                    task.task.skeleton.as_ref().is_some_and(|skeleton| {
                        keypoints
                            .iter()
                            .map(|point| point.name.as_str())
                            .eq(skeleton.keypoints.iter().map(|point| point.name.as_str()))
                            && keypoints
                                .iter()
                                .zip(&skeleton.keypoints)
                                .all(|(point, spec)| match point.state {
                                    KeypointState::Visible => true,
                                    KeypointState::Hidden => skeleton.allow_hidden,
                                    KeypointState::Absent => {
                                        skeleton.allow_absent && !spec.required
                                    }
                                })
                            && keypoints
                                .iter()
                                .any(|point| point.state != KeypointState::Absent)
                    })
                })
        }
        ImportGeometryPolicy::Omit => true,
    }
}

pub(super) fn keypoint_envelope(
    points: &[IrKeypoint],
    image_width: u32,
    image_height: u32,
    padding_ratio: f64,
    minimum_pixels: u32,
    include_hidden: bool,
) -> StorageResult<(F64Box, bool)> {
    if !padding_ratio.is_finite()
        || !(0.0..=1.0).contains(&padding_ratio)
        || minimum_pixels == 0
        || image_width == 0
        || image_height == 0
    {
        return Err(import_error(
            "keypoint_envelope_parameters_invalid",
            "keypoint envelope parameters are invalid",
        ));
    }
    let selected = points
        .iter()
        .filter(|point| {
            point.state == KeypointState::Visible
                || (include_hidden && point.state == KeypointState::Hidden)
        })
        .filter_map(|point| point.x.zip(point.y))
        .collect::<Vec<_>>();
    if selected.len() < 2 {
        return Err(import_error(
            "keypoint_envelope_geometry_invalid",
            "keypoint envelope requires at least two selected points",
        ));
    }
    let min_x = selected
        .iter()
        .map(|(x, _)| *x)
        .fold(f64::INFINITY, f64::min);
    let min_y = selected
        .iter()
        .map(|(_, y)| *y)
        .fold(f64::INFINITY, f64::min);
    let max_x = selected
        .iter()
        .map(|(x, _)| *x)
        .fold(f64::NEG_INFINITY, f64::max);
    let max_y = selected
        .iter()
        .map(|(_, y)| *y)
        .fold(f64::NEG_INFINITY, f64::max);
    let span_x = max_x - min_x;
    let span_y = max_y - min_y;
    if !span_x.is_finite() || !span_y.is_finite() || span_x <= 0.0 || span_y <= 0.0 {
        return Err(import_error(
            "keypoint_envelope_geometry_invalid",
            "keypoint envelope requires nonzero horizontal and vertical span",
        ));
    }
    let pad_x = (span_x * padding_ratio).max(f64::from(minimum_pixels) / f64::from(image_width));
    let pad_y = (span_y * padding_ratio).max(f64::from(minimum_pixels) / f64::from(image_height));
    let raw = (min_x - pad_x, min_y - pad_y, max_x + pad_x, max_y + pad_y);
    let clipped = (
        raw.0.clamp(0.0, 1.0),
        raw.1.clamp(0.0, 1.0),
        raw.2.clamp(0.0, 1.0),
        raw.3.clamp(0.0, 1.0),
    );
    if clipped.2 <= clipped.0 || clipped.3 <= clipped.1 {
        return Err(import_error(
            "keypoint_envelope_geometry_invalid",
            "keypoint envelope is empty after clipping",
        ));
    }
    Ok((
        F64Box {
            x: clipped.0,
            y: clipped.1,
            width: clipped.2 - clipped.0,
            height: clipped.3 - clipped.1,
        },
        raw != clipped,
    ))
}

fn geometry_kind_for_annotation(
    annotation_type: &labello_domain::AnnotationType,
) -> ImportGeometryKind {
    match annotation_type {
        labello_domain::AnnotationType::BoundingBox => ImportGeometryKind::BoundingBox,
        labello_domain::AnnotationType::Skeleton => ImportGeometryKind::Skeleton,
    }
}

fn estimated_derived_geometry(
    ir: &ImportIr,
    request: &PreflightRequest,
    class_ids: &BTreeMap<String, String>,
) -> (usize, usize, usize) {
    ir.objects
        .iter()
        .filter(|object| class_ids.contains_key(&object.source_category_key))
        .fold((0, 0, 0), |mut counts, object| {
            let clipped_direct = usize::from(object.clipped)
                * usize::from(
                    geometry_policy(
                        request,
                        &object.source_category_key,
                        ImportGeometryKind::BoundingBox,
                    )
                    .is_some_and(|policy| matches!(policy, ResolvedGeometryPolicy::Direct)),
                );
            let envelope = usize::from(
                object.direct_skeleton.is_some()
                    && geometry_policy(
                        request,
                        &object.source_category_key,
                        ImportGeometryKind::BoundingBox,
                    )
                    .is_some_and(|policy| {
                        matches!(policy, ResolvedGeometryPolicy::KeypointEnvelopeV1 { .. })
                    }),
            );
            let template = usize::from(
                object.direct_bbox.is_some()
                    && geometry_policy(
                        request,
                        &object.source_category_key,
                        ImportGeometryKind::Skeleton,
                    )
                    .is_some_and(|policy| {
                        matches!(policy, ResolvedGeometryPolicy::BoxRelativeTemplateV1 { .. })
                    }),
            );
            counts.0 += clipped_direct;
            counts.1 += envelope;
            counts.2 += template;
            counts
        })
}

fn slug(value: &str) -> String {
    let mut output = String::new();
    let mut dash = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            output.push(character.to_ascii_lowercase());
            dash = false;
        } else if !output.is_empty() && !dash {
            output.push('-');
            dash = true;
        }
    }
    output.trim_end_matches('-').chars().take(80).collect()
}

fn validate_or_clip_box(
    bbox: &mut F64Box,
    policy: GeometryBoundsPolicy,
    diagnostics: &mut Diagnostics,
    path: &str,
    line: usize,
) -> StorageResult<bool> {
    if !bbox.x.is_finite()
        || !bbox.y.is_finite()
        || !bbox.width.is_finite()
        || !bbox.height.is_finite()
        || bbox.width <= 0.0
        || bbox.height <= 0.0
    {
        return Err(import_error(
            "geometry_invalid",
            "bounding box values must be finite and positive",
        ));
    }
    let valid =
        bbox.x >= 0.0 && bbox.y >= 0.0 && bbox.x + bbox.width <= 1.0 && bbox.y + bbox.height <= 1.0;
    if valid {
        return Ok(false);
    }
    if policy == GeometryBoundsPolicy::Block {
        return Err(import_error(
            "geometry_out_of_bounds",
            "bounding box crosses decoded image bounds",
        ));
    }
    let right = (bbox.x + bbox.width).clamp(0.0, 1.0);
    let bottom = (bbox.y + bbox.height).clamp(0.0, 1.0);
    bbox.x = bbox.x.clamp(0.0, 1.0);
    bbox.y = bbox.y.clamp(0.0, 1.0);
    bbox.width = right - bbox.x;
    bbox.height = bottom - bbox.y;
    if bbox.width <= 0.0 || bbox.height <= 0.0 {
        return Err(import_error(
            "geometry_clip_empty",
            "clipping produced an empty bounding box",
        ));
    }
    diagnostics.add(
        "geometry_clipped",
        DiagnosticSeverity::WarningRequiresAck,
        "out-of-bounds box is clipped and marked derived",
        false,
        true,
        true,
        Some(if line > 0 {
            example_line(path, line)
        } else {
            example_path(path)
        }),
    );
    Ok(true)
}

fn normalize_yolo_bbox_boundary(
    center_x: f64,
    center_y: f64,
    width: f64,
    height: f64,
) -> (F64Box, bool) {
    let left = center_x - width / 2.0;
    let right = center_x + width / 2.0;
    let top = center_y - height / 2.0;
    let bottom = center_y + height / 2.0;
    let reconstructed_right = left + width;
    let reconstructed_bottom = top + height;
    let outside =
        left < 0.0 || top < 0.0 || reconstructed_right > 1.0 || reconstructed_bottom > 1.0;
    let comparison_margin = f64::EPSILON * 8.0;
    let lower_bound = -YOLO_BOUNDARY_ROUNDING_TOLERANCE - comparison_margin;
    let upper_bound = 1.0 + YOLO_BOUNDARY_ROUNDING_TOLERANCE + comparison_margin;
    let within_rounding_tolerance = left >= lower_bound
        && top >= lower_bound
        && right <= upper_bound
        && bottom <= upper_bound
        && reconstructed_right <= upper_bound
        && reconstructed_bottom <= upper_bound;
    if outside && within_rounding_tolerance {
        let left = left.clamp(0.0, 1.0);
        let right = right.clamp(0.0, 1.0);
        let top = top.clamp(0.0, 1.0);
        let bottom = bottom.clamp(0.0, 1.0);
        return (
            F64Box {
                x: left,
                y: top,
                width: right - left,
                height: bottom - top,
            },
            true,
        );
    }
    (
        F64Box {
            x: left,
            y: top,
            width,
            height,
        },
        false,
    )
}

fn yaml_strings(value: &Value) -> StorageResult<Vec<String>> {
    let values = match value {
        Value::String(value) => vec![value.clone()],
        Value::Array(values) if !values.is_empty() => values
            .iter()
            .map(|value| {
                value.as_str().map(str::to_string).ok_or_else(|| {
                    import_error("yolo_split_invalid", "YOLO split entries must be strings")
                })
            })
            .collect::<StorageResult<Vec<_>>>()?,
        _ => Err(import_error(
            "yolo_split_invalid",
            "YOLO split must be a nonempty string or list of strings",
        ))?,
    };
    if values.iter().any(|value| value.trim().is_empty()) {
        return Err(import_error(
            "yolo_split_invalid",
            "YOLO split paths must be nonempty strings",
        ));
    }
    Ok(values)
}

fn required_array<'a>(
    root: &'a serde_json::Map<String, Value>,
    key: &str,
) -> StorageResult<&'a Vec<Value>> {
    root.get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| import_error("coco_field_invalid", format!("COCO {key} must be an array")))
}
fn required_string<'a>(
    root: &'a serde_json::Map<String, Value>,
    key: &str,
    code: &str,
) -> StorageResult<&'a str> {
    root.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| import_error(code, format!("COCO {key} must be a nonempty string")))
}
fn json_id(value: Option<&Value>, code: &str) -> StorageResult<u64> {
    value
        .and_then(Value::as_u64)
        .filter(|value| *value <= i64::MAX as u64)
        .ok_or_else(|| import_error(code, "COCO IDs must be JSON integers in 0..=i64::MAX"))
}
fn json_u32(value: Option<&Value>, code: &str) -> StorageResult<u32> {
    json_id(value, code)
        .and_then(|value| {
            u32::try_from(value).map_err(|_| import_error(code, "COCO dimension exceeds u32"))
        })
        .and_then(|value| {
            if value == 0 {
                Err(import_error(code, "COCO dimensions must be positive"))
            } else {
                Ok(value)
            }
        })
}
fn finite_array(value: Option<&Value>, length: usize, code: &str) -> StorageResult<Vec<f64>> {
    let values = value
        .and_then(Value::as_array)
        .filter(|values| values.len() == length)
        .ok_or_else(|| {
            import_error(
                code,
                format!("numeric array must contain exactly {length} values"),
            )
        })?;
    values
        .iter()
        .map(|value| {
            value
                .as_f64()
                .filter(|value| value.is_finite())
                .ok_or_else(|| import_error(code, "numeric arrays require finite JSON numbers"))
        })
        .collect()
}
fn parse_integer_token(value: &str, code: &str) -> StorageResult<u64> {
    if value.contains(['.', 'e', 'E']) {
        return Err(import_error(code, "class index must be an integer token"));
    }
    value
        .parse()
        .map_err(|_| import_error(code, "class index must be a non-negative integer"))
}
fn parse_finite(value: &str, code: &str) -> StorageResult<f64> {
    value
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| import_error(code, "numeric token must be finite"))
}
fn is_image_path(path: &str) -> bool {
    matches!(
        source_extension(path).as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp")
    )
}
fn below(path: &str, root: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}
pub(super) fn coverage_key(image: &str, category: &str) -> String {
    format!("{image}\0{category}")
}

fn enforce_json_nesting(bytes: &[u8], limit: usize) -> StorageResult<()> {
    let mut depth = 0_usize;
    let mut string = false;
    let mut escape = false;
    for byte in bytes {
        if string {
            if escape {
                escape = false;
            } else if *byte == b'\\' {
                escape = true;
            } else if *byte == b'"' {
                string = false;
            }
            continue;
        }
        match byte {
            b'"' => string = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > limit {
                    return Err(import_error(
                        "json_nesting_limit",
                        "JSON nesting exceeds configured limit",
                    ));
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

fn validate_yaml_value(
    value: &serde_yaml_ng::Value,
    limit: usize,
    depth: usize,
    nodes: &mut usize,
) -> StorageResult<()> {
    *nodes = nodes.saturating_add(1);
    if *nodes > MAX_STRUCTURED_NODES {
        return Err(import_error(
            "structured_data_node_limit",
            "structured data exceeds the parser node limit",
        ));
    }
    if depth > limit {
        return Err(import_error(
            "yolo_yaml_nesting",
            "YAML nesting exceeds the configured limit",
        ));
    }
    match value {
        serde_yaml_ng::Value::Tagged(_) => Err(import_error(
            "yolo_yaml_tag_rejected",
            "custom YAML tags are not supported",
        )),
        serde_yaml_ng::Value::Sequence(values) => {
            for value in values {
                validate_yaml_value(value, limit, depth + 1, nodes)?;
            }
            Ok(())
        }
        serde_yaml_ng::Value::Mapping(values) => {
            for (key, value) in values {
                validate_yaml_value(key, limit, depth + 1, nodes)?;
                validate_yaml_value(value, limit, depth + 1, nodes)?;
            }
            Ok(())
        }
        serde_yaml_ng::Value::String(value) if value.len() > MAX_STRUCTURED_VALUE_BYTES => {
            Err(import_error(
                "structured_data_value_limit",
                "structured data contains an oversized scalar value",
            ))
        }
        _ => Ok(()),
    }
}

fn enforce_yaml_alias_limit(bytes: &[u8], limit: usize) -> StorageResult<()> {
    let mut aliases = 0_usize;
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;
    let mut comment = false;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if comment {
            if byte == b'\n' {
                comment = false;
            }
            continue;
        }
        if double_quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                double_quoted = false;
            }
            continue;
        }
        if single_quoted {
            if byte == b'\'' {
                if bytes.get(index + 1) == Some(&b'\'') || (index > 0 && bytes[index - 1] == b'\'')
                {
                    continue;
                }
                single_quoted = false;
            }
            continue;
        }
        match byte {
            b'#' => comment = true,
            b'"' => double_quoted = true,
            b'\'' => single_quoted = true,
            b'*' | b'&'
                if index == 0
                    || bytes[index - 1].is_ascii_whitespace()
                    || matches!(bytes[index - 1], b'[' | b'{' | b',' | b':' | b'?' | b'-') =>
            {
                aliases += 1;
                if aliases > limit {
                    return Err(import_error(
                        "yolo_yaml_alias_limit",
                        "YAML anchors and aliases exceed the parser alias limit",
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_json_value(value: &Value, nodes: &mut usize) -> StorageResult<()> {
    *nodes = nodes.saturating_add(1);
    if *nodes > MAX_STRUCTURED_NODES {
        return Err(import_error(
            "structured_data_node_limit",
            "structured data exceeds the parser node limit",
        ));
    }
    match value {
        Value::String(value) if value.len() > MAX_STRUCTURED_VALUE_BYTES => Err(import_error(
            "structured_data_value_limit",
            "structured data contains an oversized scalar value",
        )),
        Value::Array(values) => {
            for value in values {
                validate_json_value(value, nodes)?;
            }
            Ok(())
        }
        Value::Object(values) => {
            for (key, value) in values {
                if key.len() > MAX_STRUCTURED_VALUE_BYTES {
                    return Err(import_error(
                        "structured_data_value_limit",
                        "structured data contains an oversized mapping key",
                    ));
                }
                validate_json_value(value, nodes)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn for_each_bounded_line(
    path: &std::path::Path,
    max_line_bytes: usize,
    mut visit: impl FnMut(usize, &str) -> StorageResult<()>,
) -> StorageResult<()> {
    let file = std::fs::File::open(path).map_err(|source| StorageError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut reader = std::io::BufReader::new(file);
    let mut line = Vec::with_capacity(max_line_bytes.min(16 * 1024));
    let mut line_number = 0_usize;
    loop {
        line.clear();
        let mut limited = (&mut reader).take(max_line_bytes.saturating_add(1) as u64);
        let read = BufRead::read_until(&mut limited, b'\n', &mut line).map_err(|source| {
            StorageError::Io {
                path: path.to_path_buf(),
                source,
            }
        })?;
        if read == 0 {
            break;
        }
        line_number += 1;
        while line
            .last()
            .is_some_and(|byte| matches!(byte, b'\n' | b'\r'))
        {
            line.pop();
        }
        if line.len() > max_line_bytes {
            return Err(import_error(
                "yolo_line_limit",
                "YOLO label line exceeds the configured limit",
            ));
        }
        let text = std::str::from_utf8(&line)
            .map_err(|_| import_error("yolo_label_utf8", "YOLO label file must be UTF-8"))?;
        visit(line_number, text)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn free_space_bytes(path: &std::path::Path) -> Option<u64> {
    rustix::fs::statvfs(path)
        .ok()
        .and_then(|value| value.f_bavail.checked_mul(value.f_frsize))
}

#[cfg(not(target_os = "linux"))]
fn free_space_bytes(_path: &std::path::Path) -> Option<u64> {
    None
}

fn enforce_value_depth(value: &Value, limit: usize, code: &str) -> StorageResult<()> {
    fn depth(value: &Value) -> usize {
        match value {
            Value::Array(values) => 1 + values.iter().map(depth).max().unwrap_or(0),
            Value::Object(values) => 1 + values.values().map(depth).max().unwrap_or(0),
            _ => 1,
        }
    }
    if depth(value) > limit {
        Err(import_error(
            code,
            "structured-data nesting exceeds configured limit",
        ))
    } else {
        Ok(())
    }
}

fn example_path(path: &str) -> DiagnosticExample {
    DiagnosticExample {
        source_path: Some(path.to_string()),
        source_image_key: None,
        source_object_key: None,
        line: None,
    }
}
fn example_line(path: &str, line: usize) -> DiagnosticExample {
    DiagnosticExample {
        source_path: Some(path.to_string()),
        source_image_key: None,
        source_object_key: None,
        line: Some(line as u64),
    }
}
fn example_object(path: &str, id: u64) -> DiagnosticExample {
    DiagnosticExample {
        source_path: Some(path.to_string()),
        source_image_key: None,
        source_object_key: Some(id.to_string()),
        line: None,
    }
}

struct Diagnostics {
    profile: ImportProfile,
    example_limit: usize,
    values: BTreeMap<String, ImportDiagnostic>,
}
impl Diagnostics {
    fn new(profile: ImportProfile, example_limit: usize) -> Self {
        Self {
            profile,
            example_limit,
            values: BTreeMap::new(),
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        code: &str,
        severity: DiagnosticSeverity,
        summary: &str,
        blocks: bool,
        ack: bool,
        coverage: bool,
        example: Option<DiagnosticExample>,
    ) {
        let value = self
            .values
            .entry(code.to_string())
            .or_insert_with(|| ImportDiagnostic {
                code: code.to_string(),
                severity,
                profile: self.profile,
                count: 0,
                summary: summary.to_string(),
                blocks_commit: blocks,
                requires_acknowledgement: ack,
                changes_coverage: coverage,
                examples: Vec::new(),
            });
        value.count += 1;
        if let Some(example) = example
            && value.examples.len() < self.example_limit
        {
            value.examples.push(example);
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn add_count(
        &mut self,
        code: &str,
        severity: DiagnosticSeverity,
        summary: &str,
        count: u64,
        blocks: bool,
        ack: bool,
        coverage: bool,
    ) {
        self.add(code, severity, summary, blocks, ack, coverage, None);
        self.values.get_mut(code).unwrap().count = count;
    }
    fn finish(self) -> Vec<ImportDiagnostic> {
        self.values.into_values().collect()
    }
}
