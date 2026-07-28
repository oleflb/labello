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
