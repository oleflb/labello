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
