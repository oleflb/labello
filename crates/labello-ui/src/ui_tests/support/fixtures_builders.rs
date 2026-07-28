fn test_import_capabilities() -> labello_client::ImportCapabilities {
    labello_client::ImportCapabilities {
        available: true,
        profiles: vec![labello_client::ImportProfileCapability {
            profile: labello_client::ImportProfile::CocoInstancesGtV1,
            enabled: true,
            display_name: "COCO instances".to_string(),
            profile_version: 1,
        }],
        transports: vec![
            labello_client::ImportTransportCapability {
                transport: labello_client::ImportTransport::BrowserFolder,
                enabled: true,
                resumable: true,
            },
            labello_client::ImportTransportCapability {
                transport: labello_client::ImportTransport::ServerDirectory,
                enabled: true,
                resumable: true,
            },
        ],
        server_roots: vec![labello_client::ServerImportRoot {
            root_id: "staging".to_string(),
            display_name: "Staging datasets".to_string(),
        }],
        schema_version: SCHEMA_VERSION,
        parser_version: "test-parser".to_string(),
        tool_version: "test-tool".to_string(),
        manual_box_guide_migration: true,
        ..Default::default()
    }
}

pub(super) fn test_import_job(
    dataset_id: DatasetId,
    name: String,
    profile: labello_client::ImportProfile,
    transport: labello_client::ImportTransport,
) -> labello_client::ImportJob {
    labello_client::ImportJob {
        import_id: ImportId::from("imp_test"),
        owner_user_id: UserId::from("admin"),
        destination_dataset_id: dataset_id,
        destination_name: name,
        profile,
        transport,
        lifecycle: labello_client::ImportLifecycle::Registering,
        progress: Default::default(),
        failure: None,
        source_fingerprint: None,
        plan_hash: None,
        preflight_report: None,
        can_cancel: true,
        created_at: now(),
        updated_at: now(),
        expires_at: None,
        recovery: None,
    }
}

pub(super) fn test_import_report() -> labello_client::ImportPreflightReport {
    labello_client::ImportPreflightReport {
        source_fingerprint: "source-test".to_string(),
        source: labello_client::ImportSourceCounts {
            files: 3,
            images: 2,
            objects: 4,
            categories: 1,
            ..Default::default()
        },
        output: labello_client::ImportOutputEstimate {
            annotations: 4,
            tasks: 1,
            classes: 1,
            ..Default::default()
        },
        ..Default::default()
    }
}

pub(super) fn contract_import_category() -> crate::import_flow::ImportCategoryDraft {
    crate::import_flow::ImportCategoryDraft {
        selected: true,
        source_category_key: "release:person:17".to_string(),
        source_category_id: "17".to_string(),
        source_name: "Person".to_string(),
        class_id: "person".to_string(),
        class_name: "Person".to_string(),
        class_color: "#5eead4".to_string(),
        bounding_box_task_id: "bounding_box:person".to_string(),
        bounding_box_task_name: "Person bounding boxes".to_string(),
        skeleton_task_id: "skeleton:person".to_string(),
        skeleton_task_name: "Person skeletons".to_string(),
        source_skeleton: Some(SkeletonSpec {
            keypoints: vec![KeypointSpec {
                name: "nose".to_string(),
                required: false,
            }],
            edges: Vec::new(),
            allow_hidden: true,
            allow_absent: true,
        }),
        direct_geometry: vec![
            labello_client::ImportGeometryKind::BoundingBox,
            labello_client::ImportGeometryKind::Skeleton,
        ],
        geometry_mappings: Vec::new(),
        task_mappings: Vec::new(),
        skeleton_mappings: Vec::new(),
        workflow_intent: labello_client::ImportWorkflowIntent::AuthoritativeGroundTruth,
        target_keypoint_names: "nose".to_string(),
    }
}

pub(super) fn contract_import_plan(import_id: ImportId) -> labello_client::ImportPlan {
    let category = contract_import_category();
    let category_mapping = labello_client::ImportCategoryMappingRequest {
        source_category_key: category.source_category_key.clone(),
        source_category_id: category.source_category_id.clone(),
        class_id: ClassId::from(category.class_id.clone()),
        class_name: category.class_name.clone(),
        color: category.class_color.clone(),
        selected: true,
    };
    let task_mapping = labello_client::ImportTaskMappingRequest {
        source_category_key: category.source_category_key.clone(),
        task: task(
            &category.bounding_box_task_id,
            &category.bounding_box_task_name,
            Vec::new(),
        ),
        workflow_intent: labello_client::ImportWorkflowIntent::AuthoritativeGroundTruth,
    };
    let geometry_mapping = labello_client::ImportGeometryMappingRequest {
        source_category_key: category.source_category_key.clone(),
        source_geometry: labello_client::ImportGeometryKind::BoundingBox,
        target_geometry: labello_client::ImportGeometryKind::BoundingBox,
        policy: labello_client::ImportGeometryPolicy::Direct,
        parameters: Vec::new(),
    };
    let request = labello_client::UpdateImportPlanRequest {
        category_mappings: vec![category_mapping.clone()],
        geometry_mappings: vec![geometry_mapping.clone()],
        task_mappings: vec![task_mapping.clone()],
        skeleton_mappings: Vec::new(),
        compatibility: Default::default(),
        acknowledgements: Vec::new(),
    };
    labello_client::ImportPlan {
        import_id,
        source_fingerprint: "source-recovered".to_string(),
        plan_hash: "plan-recovered".to_string(),
        commit_ready: true,
        blocking_diagnostic_codes: Vec::new(),
        required_acknowledgement_codes: Vec::new(),
        report: test_import_report(),
        source_categories: vec![labello_client::ImportSourceCategory {
            source_category_key: category.source_category_key,
            source_category_id: category.source_category_id,
            source_name: category.source_name,
            source_supercategory: Some("person".to_string()),
            source_namespace: "release".to_string(),
            direct_geometry: category.direct_geometry,
            keypoint_schema: category.source_skeleton,
            generated_category_mapping: category_mapping.clone(),
            generated_task_mappings: vec![task_mapping.clone()],
            current_category_mapping: category_mapping,
            current_geometry_mappings: vec![geometry_mapping],
            current_task_mappings: vec![task_mapping],
            current_skeleton_mappings: Vec::new(),
        }],
        accepted_request: Some(request),
    }
}

fn migration_progress(
    state: &ImageState,
    task_id: &TaskId,
) -> labello_client::ManualMigrationProgress {
    let dispositions = state
        .migration_dispositions
        .get(task_id)
        .into_iter()
        .flat_map(|values| values.values());
    let mut progress = labello_client::ManualMigrationProgress::default();
    for disposition in dispositions {
        progress.expected += 1;
        match disposition.status {
            MigrationDispositionStatus::Pending => progress.pending += 1,
            MigrationDispositionStatus::Annotated { .. } => progress.annotated += 1,
            MigrationDispositionStatus::Excluded { .. } => progress.excluded += 1,
        }
    }
    progress
}

pub(super) fn ready<'a, T: 'a>(result: ClientResult<T>) -> ApiFuture<'a, T> {
    Box::pin(async move { result })
}

pub(super) fn assignment_matches(
    assignment: &Assignment,
    request: &AssignmentActionRequest,
) -> bool {
    assignment.assignment_id == request.assignment_id
        && assignment.image_id == request.image_id
        && assignment.task_id == request.task_id
        && assignment.kind == request.kind
        && assignment.status == AssignmentStatus::Active
}

pub(super) fn image_record(
    image_id: &str,
    file_name: &str,
    width: u32,
    height: u32,
) -> ImageRecord {
    ImageRecord {
        image_id: ImageId::from(image_id),
        blake3: format!("hash-{image_id}"),
        canonical_path: format!("images/{file_name}"),
        known_paths: vec![format!("images/{file_name}")],
        duplicate_paths: Vec::new(),
        source_memberships: None,
        file_name: file_name.to_string(),
        byte_size: 64,
        width,
        height,
        media_type: "image/png".to_string(),
    }
}

pub(super) fn test_snapshot(dataset_id: DatasetId) -> DatasetSnapshot {
    DatasetSnapshot {
        schema_version: SCHEMA_VERSION,
        snapshot_id: "snapshot-test".to_string(),
        dataset_id,
        created_at: now(),
        includes_image_bytes: false,
        total_bytes: 32,
        files: vec![SnapshotFileEntry {
            path: "snapshot.json".to_string(),
            byte_size: 32,
            blake3: "snapshot-hash".to_string(),
        }],
        imports: Vec::new(),
    }
}

pub(super) fn task(id: &str, name: &str, prelabel_configs: Vec<&str>) -> TaskDefinition {
    let class_id = id.split(':').nth(1).unwrap_or("person");
    TaskDefinition {
        task_id: TaskId::from(id),
        name: name.to_string(),
        annotation_type: AnnotationType::BoundingBox,
        class_ids: vec![ClassId::from(class_id)],
        instructions: TutorialContent {
            title: "Label every visible person".to_string(),
            example_text: "Draw tight boxes around every person.".to_string(),
            example_images: vec!["tutorial/example.png".to_string()],
        },
        skeleton: None,
        review: ReviewConfig::default(),
        prelabel_config_ids: prelabel_configs
            .into_iter()
            .map(PrelabelConfigId::from)
            .collect(),
        manual_box_guide_migration: None,
        enabled: true,
    }
}

pub(super) fn seed_review_annotation(
    api: &SpyApi,
    geometry: AnnotationGeometry,
    allow_reviewer_corrections: bool,
) -> labello_domain::AnnotationId {
    let mut spy = api.state.borrow_mut();
    let annotation_type = match &geometry {
        AnnotationGeometry::BoundingBox(_) => AnnotationType::BoundingBox,
        AnnotationGeometry::Skeleton(_) => AnnotationType::Skeleton,
    };
    spy.metadata.tasks[0].annotation_type = annotation_type.clone();
    spy.metadata.tasks[0].review.allow_reviewer_corrections = allow_reviewer_corrections;
    let task_id = spy.metadata.tasks[0].task_id.clone();
    let class_id = spy.metadata.tasks[0].class_ids[0].clone();
    let image_id = spy.metadata.images.keys().next().unwrap().clone();
    let annotation_id = labello_domain::AnnotationId::from("review_annotation");
    let timestamp = now();
    let annotation = labello_domain::AnnotationVersion {
        annotation_id: annotation_id.clone(),
        version: 1,
        object_group_id: None,
        origin: labello_domain::AnnotationOrigin::native(),
        task_id,
        class_id,
        annotation_type,
        revision_source: labello_domain::RevisionSource::Human {
            action: labello_domain::HumanRevisionKind::Authored,
        },
        geometry,
        author_user_id: UserId::from("annotator"),
        created_at: timestamp,
        updated_at: timestamp,
        deleted: false,
    };
    let event = EventLogEntry::new(
        1,
        image_id.clone(),
        UserId::from("annotator"),
        DatasetRole::Annotator,
        timestamp,
        EventPayload::AnnotationVersionCreated {
            annotation,
            previous_version: None,
            reason: None,
        },
    );
    spy.states
        .get_mut(&image_id)
        .unwrap()
        .apply_event(&event)
        .unwrap();
    annotation_id
}

pub(super) fn prelabel_config(id: &str) -> PrelabelConfig {
    PrelabelConfig {
        config_id: PrelabelConfigId::from(id),
        name: "Demo prelabels".to_string(),
        model: ModelSpec {
            model_id: "model".to_string(),
            display_name: "Demo model".to_string(),
            version: Some("1".to_string()),
            location: "browser".to_string(),
        },
        execution: PrelabelExecution::BrowserLocal {
            acceleration: BrowserAcceleration::WasmCpuFallback,
        },
        output_processing: OutputProcessing {
            confidence_threshold: 0.5,
            suppress_overlaps_iou: None,
        },
        available_to_annotators: true,
    }
}

pub(super) fn stats(total_images: usize) -> DatasetStats {
    let mut per_task = BTreeMap::new();
    per_task.insert(
        TaskId::from("bounding_box:person"),
        labello_domain::TaskStats {
            completed: 1,
            pending: 1,
            reviewed: 1,
            unreviewed: 1,
            approved: 1,
            rejected: 0,
            reviewer_corrected: 0,
            finalized: 1,
            provenance: Default::default(),
            migration: Default::default(),
        },
    );
    let mut per_class = BTreeMap::new();
    per_class.insert(
        ClassId::from("person"),
        labello_domain::ClassStats {
            annotations: 2,
            completed_tasks: 1,
            provenance: Default::default(),
        },
    );
    DatasetStats {
        total_images,
        completed_tasks: 1,
        pending_tasks: 1,
        reviewed_tasks: 1,
        unreviewed_tasks: 1,
        approved_tasks: 1,
        rejected_tasks: 0,
        reviewer_corrected_tasks: 0,
        finalized_tasks: 1,
        per_task,
        per_class,
        throughput: Vec::new(),
        provenance: Default::default(),
        migration: Default::default(),
        import_coverage: Default::default(),
    }
}

pub(super) fn now() -> labello_domain::Timestamp {
    labello_domain::now()
}
