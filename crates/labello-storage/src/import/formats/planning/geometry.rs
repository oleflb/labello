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
