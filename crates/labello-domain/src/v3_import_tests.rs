use serde_json::json;

use crate::*;

fn timestamp() -> Timestamp {
    "2026-01-02T03:04:05Z".parse().unwrap()
}

fn box_geometry(x: f32) -> AnnotationGeometry {
    AnnotationGeometry::BoundingBox(BoundingBox {
        x,
        y: 0.1,
        width: 0.2,
        height: 0.2,
    })
}

fn imported_box(
    annotation_id: &str,
    group_id: &str,
    task_id: &str,
    source_key: &str,
) -> AnnotationVersion {
    AnnotationVersion {
        annotation_id: AnnotationId::from(annotation_id),
        version: 1,
        object_group_id: Some(ObjectGroupId::from(group_id)),
        origin: AnnotationOrigin::Imported {
            imported: ImportedOrigin {
                import_id: ImportId::from("imp_1"),
                source_profile: SourceProfile {
                    profile_id: "coco_instances_gt_v1".to_string(),
                    profile_version: 1,
                },
                source_namespace: "release/train".to_string(),
                source_object_key: source_key.to_string(),
                geometry_provenance: ImportGeometryProvenance::Direct,
            },
        },
        task_id: TaskId::from(task_id),
        class_id: ClassId::from("person"),
        annotation_type: AnnotationType::BoundingBox,
        revision_source: RevisionSource::Import {
            import_id: ImportId::from("imp_1"),
        },
        geometry: box_geometry(0.1),
        author_user_id: UserId::from("admin"),
        created_at: timestamp(),
        updated_at: timestamp(),
        deleted: false,
    }
}

#[test]
fn v2_wire_upcasts_and_mixes_with_v3_without_rewriting_the_old_line() {
    let v2 = json!({
        "schemaVersion": 2,
        "eventSequence": 1,
        "eventId": "evt_v2",
        "imageId": "img_1",
        "type": "annotation_version_created",
        "actorUserId": "annotator",
        "actorRole": "annotator",
        "timestamp": "2026-01-02T03:04:05Z",
        "payload": {
            "kind": "annotation_version_created",
            "annotation": {
                "annotationId": "ann_1",
                "version": 1,
                "taskId": "bounding_box:person",
                "classId": "person",
                "type": "bounding_box",
                "source": { "source": "human" },
                "geometry": {
                    "type": "bounding_box",
                    "geometry": {
                        "x": 0.10000000149011612,
                        "y": 0.10000000149011612,
                        "width": 0.20000000298023224,
                        "height": 0.20000000298023224
                    }
                },
                "authorUserId": "annotator",
                "createdAt": "2026-01-02T03:04:05Z",
                "updatedAt": "2026-01-02T03:04:05Z",
                "deleted": false
            },
            "previous_version": null,
            "reason": null
        }
    });
    let old_event: EventLogEntry = serde_json::from_value(v2.clone()).unwrap();
    assert_eq!(serde_json::to_value(&old_event).unwrap(), v2);
    let EventPayload::AnnotationVersionCreated { annotation, .. } = &old_event.payload else {
        panic!("unexpected event")
    };
    assert!(annotation.origin.is_legacy_v2());
    assert!(matches!(
        annotation.revision_source,
        RevisionSource::Human {
            action: HumanRevisionKind::Authored
        }
    ));

    let mut corrected = annotation.clone();
    corrected.version = 2;
    corrected.geometry = box_geometry(0.2);
    corrected.revision_source = RevisionSource::Human {
        action: HumanRevisionKind::Edited,
    };
    let current_event = EventLogEntry::new(
        2,
        ImageId::from("img_1"),
        UserId::from("annotator"),
        DatasetRole::Annotator,
        timestamp(),
        EventPayload::AnnotationVersionCreated {
            annotation: corrected,
            previous_version: Some(1),
            reason: Some("move".to_string()),
        },
    );

    let state = rebuild_state(ImageId::from("img_1"), &[old_event, current_event]).unwrap();
    let current = state
        .current_annotation(&AnnotationId::from("ann_1"))
        .unwrap();
    assert_eq!(state.schema_version, 3);
    assert_eq!(current.version, 2);
    assert!(current.origin.is_legacy_v2());
}

#[test]
fn replay_rejects_origin_or_object_group_changes() {
    let image_id = ImageId::from("img_immutability");
    let first = imported_box("ann_guide", "group_1", "bounding_box:person", "object/1");
    let mut changed = first.clone();
    changed.version = 2;
    changed.object_group_id = Some(ObjectGroupId::from("group_2"));
    changed.revision_source = RevisionSource::Human {
        action: HumanRevisionKind::Edited,
    };
    let events = [
        EventLogEntry::new(
            1,
            image_id.clone(),
            UserId::from("admin"),
            DatasetRole::DataAdmin,
            timestamp(),
            EventPayload::AnnotationVersionCreated {
                annotation: first,
                previous_version: None,
                reason: None,
            },
        ),
        EventLogEntry::new(
            2,
            image_id.clone(),
            UserId::from("annotator"),
            DatasetRole::Annotator,
            timestamp(),
            EventPayload::AnnotationVersionCreated {
                annotation: changed,
                previous_version: Some(1),
                reason: None,
            },
        ),
    ];
    assert!(rebuild_state(image_id, &events).is_err());
}

#[test]
fn compact_import_and_manual_migration_replay_to_confirmation() {
    let dataset_id = DatasetId::from("dataset_1");
    let image_id = ImageId::from("img_1");
    let guide_task = TaskId::from("bounding_box:person");
    let target_task = TaskId::from("skeleton:person");
    let target = MigrationTarget {
        object_group_id: ObjectGroupId::from("group_1"),
        guide_annotation_id: AnnotationId::from("ann_guide"),
        reserved_skeleton_annotation_id: AnnotationId::from("ann_skeleton"),
        sequence_index: 0,
    };
    let target_hash = migration_target_set_hash(
        &MigrationHashContext {
            dataset_id: &dataset_id,
            image_id: &image_id,
            guide_task_id: &guide_task,
            target_task_id: &target_task,
        },
        std::slice::from_ref(&target),
    )
    .unwrap();
    let imported = EventLogEntry::new(
        1,
        image_id.clone(),
        UserId::from("admin"),
        DatasetRole::DataAdmin,
        timestamp(),
        EventPayload::ImportInitialized {
            import_id: ImportId::from("imp_1"),
            annotations: vec![imported_box(
                "ann_guide",
                "group_1",
                "bounding_box:person",
                "object/1",
            )],
            task_initializations: vec![
                ImportTaskInitialization {
                    task_id: guide_task.clone(),
                    coverage: ImportCoverage::Complete,
                    initial_state: TaskState {
                        task_id: guide_task.clone(),
                        status: TaskStatus::Completed,
                        outcome: Some(TaskOutcome::ImportedGroundTruth),
                        assigned_to: None,
                        completed_by: Some(UserId::from("admin")),
                        completed_at: Some(timestamp()),
                        updated_at: timestamp(),
                    },
                },
                ImportTaskInitialization {
                    task_id: target_task.clone(),
                    coverage: ImportCoverage::Incomplete,
                    initial_state: TaskState::new(target_task.clone(), timestamp()),
                },
            ],
            migration_target_sets: vec![MigrationTargetSetInitialization {
                dataset_id,
                guide_task_id: guide_task,
                target_task_id: target_task.clone(),
                target_set_hash: target_hash,
                targets: vec![target],
            }],
        },
    );
    let skeleton = AnnotationVersion {
        annotation_id: AnnotationId::from("ann_skeleton"),
        version: 1,
        object_group_id: Some(ObjectGroupId::from("group_1")),
        origin: AnnotationOrigin::native(),
        task_id: target_task.clone(),
        class_id: ClassId::from("person"),
        annotation_type: AnnotationType::Skeleton,
        revision_source: RevisionSource::Human {
            action: HumanRevisionKind::Authored,
        },
        geometry: AnnotationGeometry::Skeleton(SkeletonGeometry {
            keypoints: Vec::new(),
        }),
        author_user_id: UserId::from("annotator"),
        created_at: timestamp(),
        updated_at: timestamp(),
        deleted: false,
    };
    let skeleton_event = EventLogEntry::new(
        2,
        image_id.clone(),
        UserId::from("annotator"),
        DatasetRole::Annotator,
        timestamp(),
        EventPayload::AnnotationVersionCreated {
            annotation: skeleton,
            previous_version: None,
            reason: None,
        },
    );
    let disposition_event = EventLogEntry::new(
        3,
        image_id.clone(),
        UserId::from("annotator"),
        DatasetRole::Annotator,
        timestamp(),
        EventPayload::MigrationDispositionChanged {
            task_id: target_task.clone(),
            object_group_id: ObjectGroupId::from("group_1"),
            disposition: MigrationDisposition {
                disposition_version: 2,
                status: MigrationDispositionStatus::Annotated {
                    skeleton_annotation_id: AnnotationId::from("ann_skeleton"),
                    skeleton_version: 1,
                },
            },
        },
    );
    let mut state = rebuild_state(
        image_id.clone(),
        &[imported, skeleton_event, disposition_event],
    )
    .unwrap();
    let target_hash = state.migration_target_sets[&target_task]
        .target_set_hash
        .clone();
    let state_hash = state.current_migration_state_hash(&target_task).unwrap();
    let confirmation_hash = migration_confirmation_hash(&target_hash, &state_hash).unwrap();
    let terminal_without_confirmation = EventLogEntry::new(
        4,
        image_id.clone(),
        UserId::from("admin"),
        DatasetRole::DataAdmin,
        timestamp(),
        EventPayload::TaskStateChanged {
            task_state: TaskState {
                task_id: target_task.clone(),
                status: TaskStatus::Completed,
                outcome: Some(TaskOutcome::Approved),
                assigned_to: None,
                completed_by: Some(UserId::from("admin")),
                completed_at: Some(timestamp()),
                updated_at: timestamp(),
            },
        },
    );
    assert!(state.apply_event(&terminal_without_confirmation).is_err());

    let forged_confirmation = EventLogEntry::new(
        4,
        image_id.clone(),
        UserId::from("annotator"),
        DatasetRole::Annotator,
        timestamp(),
        EventPayload::MigrationFullImageConfirmed {
            confirmation: MigrationConfirmation {
                task_id: target_task.clone(),
                target_set_hash: target_hash.clone(),
                state_hash: state_hash.clone(),
                confirmation_hash: confirmation_hash.clone(),
                actor_user_id: UserId::from("someone_else"),
                timestamp: timestamp() + std::time::Duration::from_secs(1),
            },
        },
    );
    assert!(state.apply_event(&forged_confirmation).is_err());

    let confirmation_event = EventLogEntry::new(
        4,
        image_id.clone(),
        UserId::from("annotator"),
        DatasetRole::Annotator,
        timestamp(),
        EventPayload::MigrationFullImageConfirmed {
            confirmation: MigrationConfirmation {
                task_id: target_task.clone(),
                target_set_hash: target_hash,
                state_hash,
                confirmation_hash: confirmation_hash.clone(),
                actor_user_id: UserId::from("annotator"),
                timestamp: timestamp(),
            },
        },
    );
    state.apply_event(&confirmation_event).unwrap();
    let submit = EventLogEntry::new(
        5,
        image_id.clone(),
        UserId::from("annotator"),
        DatasetRole::Annotator,
        timestamp(),
        EventPayload::TaskStateChanged {
            task_state: TaskState {
                task_id: target_task.clone(),
                status: TaskStatus::Submitted,
                outcome: None,
                assigned_to: None,
                completed_by: Some(UserId::from("annotator")),
                completed_at: Some(timestamp()),
                updated_at: timestamp(),
            },
        },
    );
    state.apply_event(&submit).unwrap();
    assert_eq!(
        state.migration_confirmations[&target_task].confirmation_hash,
        confirmation_hash
    );
    assert!(!state.assignment_eligible(&TaskId::from("bounding_box:person")));
    assert!(!state.assignment_eligible(&target_task));

    let delete_guide = EventLogEntry::new(
        6,
        state.image_id.clone(),
        UserId::from("admin"),
        DatasetRole::DataAdmin,
        timestamp(),
        EventPayload::AnnotationDeleted {
            annotation_id: AnnotationId::from("ann_guide"),
            version: 1,
            reason: Some("guide correction".to_string()),
        },
    );
    state.apply_event(&delete_guide).unwrap();
    assert!(!state.migration_confirmations.contains_key(&target_task));
    assert!(matches!(
        state.migration_cursor(&target_task, None).unwrap(),
        MigrationCursor::Object {
            object_group_id,
            sequence_index: 0
        } if object_group_id == ObjectGroupId::from("group_1")
    ));
    assert!(matches!(
        state.migration_dependencies[&target_task][&ObjectGroupId::from("group_1")].kind,
        MigrationDependencyKind::GuideUnavailable
    ));

    state
        .apply_event(&EventLogEntry::new(
            7,
            image_id.clone(),
            UserId::from("admin"),
            DatasetRole::DataAdmin,
            timestamp(),
            EventPayload::TaskStateChanged {
                task_state: TaskState {
                    task_id: target_task.clone(),
                    status: TaskStatus::NeedsCorrection,
                    outcome: None,
                    assigned_to: None,
                    completed_by: None,
                    completed_at: None,
                    updated_at: timestamp(),
                },
            },
        ))
        .unwrap();
    state
        .apply_event(&EventLogEntry::new(
            8,
            image_id.clone(),
            UserId::from("annotator"),
            DatasetRole::Annotator,
            timestamp(),
            EventPayload::AnnotationDeleted {
                annotation_id: AnnotationId::from("ann_skeleton"),
                version: 1,
                reason: Some("deleted guide exclusion".to_string()),
            },
        ))
        .unwrap();
    let exclusion_event_id = EventId::from("evt_deleted_guide_exclusion");
    let exclusion = MigrationExclusion {
        reason: MigrationExclusionReason::InvalidSourceBox,
        event_id: exclusion_event_id.clone(),
        actor_user_id: UserId::from("annotator"),
        timestamp: timestamp(),
        note: None,
    };
    let exclusion_payload = EventPayload::MigrationDispositionChanged {
        task_id: target_task.clone(),
        object_group_id: ObjectGroupId::from("group_1"),
        disposition: MigrationDisposition {
            disposition_version: 4,
            status: MigrationDispositionStatus::Excluded {
                exclusion: exclusion.clone(),
            },
        },
    };
    let forged = EventLogEntry::new(
        9,
        image_id.clone(),
        UserId::from("someone_else"),
        DatasetRole::Annotator,
        timestamp(),
        exclusion_payload.clone(),
    );
    assert!(state.clone().apply_event(&forged).is_err());
    let mut exclusion_event = EventLogEntry::new(
        9,
        image_id.clone(),
        UserId::from("annotator"),
        DatasetRole::Annotator,
        timestamp(),
        exclusion_payload,
    );
    exclusion_event.event_id = exclusion_event_id;
    state.apply_event(&exclusion_event).unwrap();
    state
        .apply_event(&EventLogEntry::new(
            10,
            image_id.clone(),
            UserId::from("annotator"),
            DatasetRole::Annotator,
            timestamp(),
            EventPayload::MigrationDependencyCleared {
                task_id: target_task.clone(),
                object_group_id: ObjectGroupId::from("group_1"),
                marker_version: 1,
            },
        ))
        .unwrap();
    assert!(
        !state.migration_dependencies[&target_task].contains_key(&ObjectGroupId::from("group_1"))
    );
    assert_eq!(
        state.migration_cursor(&target_task, None).unwrap(),
        MigrationCursor::FullImage
    );
    assert!(
        state
            .current_annotation(&AnnotationId::from("ann_guide"))
            .unwrap()
            .deleted
    );
    assert!(
        state
            .current_annotation(&AnnotationId::from("ann_skeleton"))
            .unwrap()
            .deleted
    );

    let target_hash = state.migration_target_sets[&target_task]
        .target_set_hash
        .clone();
    let state_hash = state.current_migration_state_hash(&target_task).unwrap();
    let confirmation_hash = migration_confirmation_hash(&target_hash, &state_hash).unwrap();
    state
        .apply_event(&EventLogEntry::new(
            11,
            image_id.clone(),
            UserId::from("annotator"),
            DatasetRole::Annotator,
            timestamp(),
            EventPayload::MigrationFullImageConfirmed {
                confirmation: MigrationConfirmation {
                    task_id: target_task.clone(),
                    target_set_hash: target_hash,
                    state_hash,
                    confirmation_hash,
                    actor_user_id: UserId::from("annotator"),
                    timestamp: timestamp(),
                },
            },
        ))
        .unwrap();
    state
        .apply_event(&EventLogEntry::new(
            12,
            image_id,
            UserId::from("annotator"),
            DatasetRole::Annotator,
            timestamp(),
            EventPayload::TaskStateChanged {
                task_state: TaskState {
                    task_id: target_task,
                    status: TaskStatus::Completed,
                    outcome: Some(TaskOutcome::AnnotationCompleted),
                    assigned_to: None,
                    completed_by: Some(UserId::from("annotator")),
                    completed_at: Some(timestamp()),
                    updated_at: timestamp(),
                },
            },
        ))
        .unwrap();
}

#[test]
fn derived_import_requires_per_object_human_action_before_submission() {
    let mut annotation = imported_box("ann_derived", "group_1", "bounding_box:person", "object/1");
    let AnnotationOrigin::Imported { imported } = &mut annotation.origin else {
        unreachable!()
    };
    imported.geometry_provenance = ImportGeometryProvenance::Derived {
        transform: ImportTransform {
            transform_id: "clip_v1".to_string(),
            version: 1,
            parameters: Default::default(),
        },
    };
    let image_id = ImageId::from("img_derived");
    let import_event = EventLogEntry::new(
        1,
        image_id.clone(),
        UserId::from("admin"),
        DatasetRole::DataAdmin,
        timestamp(),
        EventPayload::ImportInitialized {
            import_id: ImportId::from("imp_1"),
            annotations: vec![annotation],
            task_initializations: vec![ImportTaskInitialization {
                task_id: TaskId::from("bounding_box:person"),
                coverage: ImportCoverage::Incomplete,
                initial_state: TaskState::new(TaskId::from("bounding_box:person"), timestamp()),
            }],
            migration_target_sets: Vec::new(),
        },
    );
    let submit = EventLogEntry::new(
        2,
        image_id.clone(),
        UserId::from("annotator"),
        DatasetRole::Annotator,
        timestamp(),
        EventPayload::TaskStateChanged {
            task_state: TaskState {
                task_id: TaskId::from("bounding_box:person"),
                status: TaskStatus::Submitted,
                outcome: Some(TaskOutcome::AnnotationCompleted),
                assigned_to: Some(UserId::from("annotator")),
                completed_by: Some(UserId::from("annotator")),
                completed_at: Some(timestamp()),
                updated_at: timestamp(),
            },
        },
    );
    assert!(rebuild_state(image_id, &[import_event, submit]).is_err());
}

#[test]
fn versioned_geometry_policies_preserve_typed_parameters() {
    let envelope = ImportGeometryMapping {
        source_category_key: "person".to_string(),
        source_geometry: ImportGeometryKind::Skeleton,
        target_geometry: ImportGeometryKind::BoundingBox,
        policy: ImportGeometryPolicy::KeypointEnvelopeV1 {
            padding_ratio: 0.05,
            minimum_pixels: 1,
            include_hidden: true,
        },
    };
    assert_eq!(
        serde_json::to_value(envelope).unwrap(),
        json!({
            "sourceCategoryKey": "person",
            "sourceGeometry": "skeleton",
            "targetGeometry": "bounding_box",
            "policy": {
                "policy": "keypoint_envelope_v1",
                "paddingRatio": 0.05,
                "minimumPixels": 1,
                "includeHidden": true
            }
        })
    );

    let template = ImportGeometryPolicy::BoxRelativeTemplateV1 {
        keypoints: vec![ImportTemplateKeypoint {
            name: "nose".to_string(),
            x: 0.5,
            y: 0.25,
            state: KeypointState::Visible,
        }],
    };
    let value = serde_json::to_value(template).unwrap();
    assert_eq!(value["policy"], "box_relative_template_v1");
    assert_eq!(value["keypoints"][0]["name"], "nose");
}

#[test]
fn review_targets_migration_disposition_and_confirmation_exactly() {
    let disposition = ReviewTarget::MigrationDisposition {
        task_id: TaskId::from("skeleton:person"),
        object_group_id: ObjectGroupId::from("group_1"),
        disposition_version: 4,
    };
    assert_eq!(
        serde_json::to_value(disposition).unwrap(),
        json!({
            "targetType": "migration_disposition",
            "task_id": "skeleton:person",
            "object_group_id": "group_1",
            "disposition_version": 4
        })
    );
}

#[test]
fn versioned_artifact_boundaries_upcast_v2_state_snapshot_and_offline_bundle() {
    let v2_state = json!({
        "schemaVersion": 2,
        "imageId": "img_1",
        "currentSequence": 0,
        "annotations": {},
        "reviews": [],
        "reviewerCorrections": [],
        "adjudications": [],
        "taskStates": {},
        "assignments": []
    });
    let state: ImageState =
        deserialize_current_artifact(&serde_json::to_vec(&v2_state).unwrap()).unwrap();
    assert_eq!(state.schema_version, 3);
    assert!(state.import_coverage.is_empty());

    let snapshot: DatasetSnapshot = deserialize_current_artifact(
        &serde_json::to_vec(&json!({
            "schemaVersion": 2,
            "snapshotId": "snapshot_1",
            "datasetId": "dataset_1",
            "createdAt": "2026-01-02T03:04:05Z",
            "includesImageBytes": false,
            "totalBytes": 0,
            "files": []
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(snapshot.schema_version, 3);
    assert!(snapshot.imports.is_empty());

    let bundle: OfflineBundle = deserialize_current_artifact(
        &serde_json::to_vec(&json!({
            "schemaVersion": 2,
            "datasetId": "dataset_1",
            "userId": "annotator",
            "createdAt": "2026-01-02T03:04:05Z",
            "expiresAt": null,
            "roles": ["annotator"],
            "tasks": [],
            "images": [{
                "image": {
                    "imageId": "img_1",
                    "blake3": "abc",
                    "canonicalPath": "images/abc.png",
                    "knownPaths": [],
                    "duplicatePaths": [],
                    "fileName": "image.png",
                    "byteSize": 1,
                    "width": 1,
                    "height": 1,
                    "mediaType": "image/png"
                },
                "state": v2_state,
                "eventLogFragment": {
                    "imageId": "img_1",
                    "baseSequence": 0,
                    "events": []
                },
                "imageBytesBase64": null
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(bundle.schema_version, 3);
    assert!(bundle.import_manifests.is_empty());
    assert_eq!(bundle.images[0].state.schema_version, 3);
    assert_eq!(bundle.images[0].image.source_memberships, None);

    let index: ImagesIndex = deserialize_current_artifact(
        &serde_json::to_vec(&json!({
            "schemaVersion": 2,
            "imageCount": 1,
            "imagesByHash": {
                "abc": {
                    "imageId": "img_1",
                    "blake3": "abc",
                    "canonicalPath": "images/abc.png",
                    "knownPaths": [],
                    "duplicatePaths": [],
                    "fileName": "image.png",
                    "byteSize": 1,
                    "width": 1,
                    "height": 1,
                    "mediaType": "image/png"
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(index.schema_version, 3);
    assert_eq!(index.images_by_hash["abc"].source_memberships, None);
}

#[test]
fn imported_completion_reopen_and_excluded_include_preserve_coverage_facts() {
    let image_id = ImageId::from("img_coverage");
    let excluded_task = TaskId::from("task_excluded");
    let complete_task = TaskId::from("task_complete");
    let completed = TaskState {
        task_id: complete_task.clone(),
        status: TaskStatus::Completed,
        outcome: Some(TaskOutcome::ImportedGroundTruth),
        assigned_to: None,
        completed_by: Some(UserId::from("admin")),
        completed_at: Some(timestamp()),
        updated_at: timestamp(),
    };
    let events = vec![
        EventLogEntry::new(
            1,
            image_id.clone(),
            UserId::from("admin"),
            DatasetRole::DataAdmin,
            timestamp(),
            EventPayload::ImportInitialized {
                import_id: ImportId::from("imp_coverage"),
                annotations: Vec::new(),
                task_initializations: vec![
                    ImportTaskInitialization {
                        task_id: excluded_task.clone(),
                        coverage: ImportCoverage::Excluded,
                        initial_state: TaskState::new(excluded_task.clone(), timestamp()),
                    },
                    ImportTaskInitialization {
                        task_id: complete_task.clone(),
                        coverage: ImportCoverage::VerifiedEmpty,
                        initial_state: completed,
                    },
                ],
                migration_target_sets: Vec::new(),
            },
        ),
        EventLogEntry::new(
            2,
            image_id.clone(),
            UserId::from("admin"),
            DatasetRole::DataAdmin,
            timestamp(),
            EventPayload::ImportCoverageIncluded {
                task_state: TaskState::new(excluded_task.clone(), timestamp()),
                reason: "bring image into scope".to_string(),
            },
        ),
        EventLogEntry::new(
            3,
            image_id.clone(),
            UserId::from("admin"),
            DatasetRole::DataAdmin,
            timestamp(),
            EventPayload::ImportedTaskReopened {
                task_state: TaskState::new(complete_task.clone(), timestamp()),
                reason: "audit imported negative".to_string(),
            },
        ),
    ];
    let state = rebuild_state(image_id, &events).unwrap();
    assert_eq!(
        state.import_coverage[&excluded_task],
        ImportCoverage::Excluded
    );
    assert_eq!(
        state.import_coverage[&complete_task],
        ImportCoverage::VerifiedEmpty
    );
    assert!(state.assignment_eligible(&excluded_task));
    assert!(state.included_in_completion_denominator(&excluded_task));
    assert!(state.assignment_eligible(&complete_task));
}
