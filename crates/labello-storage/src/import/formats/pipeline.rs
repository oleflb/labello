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
