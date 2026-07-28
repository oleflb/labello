// These values are the complete persisted annotation boundary; grouping them
// into a second context type would duplicate `AnnotationVersion` without
// removing any current caller complexity.
#[allow(clippy::too_many_arguments)]
fn imported_annotation(
    job: &ImportJob,
    object: &IrObject,
    group_id: labello_domain::ObjectGroupId,
    task_id: TaskId,
    class_id: ClassId,
    geometry: AnnotationGeometry,
    transform: Option<ImportTransform>,
    owner: &UserId,
    timestamp: labello_domain::Timestamp,
    kind: &str,
) -> AnnotationVersion {
    let annotation_id = AnnotationId::from(deterministic_id(
        "ann",
        job.import_id.as_str(),
        &object.source_object_key,
        task_id.as_str(),
    ));
    AnnotationVersion {
        annotation_id,
        version: 1,
        object_group_id: Some(group_id),
        origin: AnnotationOrigin::Imported {
            imported: ImportedOrigin {
                import_id: job.import_id.clone(),
                source_profile: SourceProfile {
                    profile_id: job.profile.id().to_string(),
                    profile_version: 1,
                },
                source_namespace: object.source_namespace.clone(),
                source_object_key: object.source_object_key.clone(),
                geometry_provenance: if let Some(transform) = transform {
                    ImportGeometryProvenance::Derived { transform }
                } else {
                    ImportGeometryProvenance::Direct
                },
            },
        },
        task_id,
        class_id,
        annotation_type: if kind == "bounding_box" {
            AnnotationType::BoundingBox
        } else {
            AnnotationType::Skeleton
        },
        revision_source: RevisionSource::Import {
            import_id: job.import_id.clone(),
        },
        geometry,
        author_user_id: owner.clone(),
        created_at: timestamp,
        updated_at: timestamp,
        deleted: false,
    }
}

fn clipping_transform() -> ImportTransform {
    ImportTransform {
        transform_id: "clip_to_image_bounds".to_string(),
        version: 1,
        parameters: BTreeMap::from([("clipped".to_string(), "true".to_string())]),
    }
}

fn envelope_transform(
    padding_ratio: f64,
    minimum_pixels: u32,
    include_hidden: bool,
    clipped: bool,
) -> ImportTransform {
    ImportTransform {
        transform_id: "keypoint_envelope".to_string(),
        version: 1,
        parameters: BTreeMap::from([
            ("padding_ratio".to_string(), padding_ratio.to_string()),
            ("minimum_pixels".to_string(), minimum_pixels.to_string()),
            ("include_hidden".to_string(), include_hidden.to_string()),
            ("clipped".to_string(), clipped.to_string()),
        ]),
    }
}

fn template_transform(keypoints: &[TemplateKeypoint], source_box_clipped: bool) -> ImportTransform {
    let mut parameters = keypoints
        .iter()
        .map(|point| {
            (
                format!("keypoint.{}", point.name),
                format!("{},{},{:?}", point.x, point.y, point.state).to_ascii_lowercase(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    parameters.insert(
        "source_box_clipped".to_string(),
        source_box_clipped.to_string(),
    );
    ImportTransform {
        transform_id: "box_relative_template".to_string(),
        version: 1,
        parameters,
    }
}

fn initial_state(
    task_id: TaskId,
    coverage: ImportCoverage,
    intent: ImportIntent,
    owner: &UserId,
    timestamp: labello_domain::Timestamp,
    manual: bool,
) -> TaskState {
    if manual {
        return TaskState::new(task_id, timestamp);
    }
    let authoritative = matches!(
        coverage,
        ImportCoverage::Complete | ImportCoverage::VerifiedEmpty
    );
    match (intent, authoritative) {
        (ImportIntent::AuthoritativeGroundTruth, true) => TaskState {
            task_id,
            status: TaskStatus::Completed,
            outcome: Some(TaskOutcome::ImportedGroundTruth),
            assigned_to: None,
            completed_by: Some(owner.clone()),
            completed_at: Some(timestamp),
            updated_at: timestamp,
        },
        (ImportIntent::RequireApproval, true) => TaskState {
            task_id,
            status: TaskStatus::Submitted,
            outcome: None,
            assigned_to: None,
            completed_by: None,
            completed_at: None,
            updated_at: timestamp,
        },
        _ => TaskState::new(task_id, timestamp),
    }
}
