use labello_domain::{
    Actor, AnnotationGeometry, AnnotationId, AnnotationOrigin, AnnotationType, BoundingBox,
    ClassId, DatasetId, DatasetMetadata, DatasetRole, EventPayload, HumanRevisionKind, ImageId,
    ImageRecord, ImagesIndex, ImportId, ImportManifest, RevisionSource, SourceProfile, TaskId,
    TaskState, UserId, now,
};

use super::*;

#[tokio::test]
async fn image_record_lookups_share_one_cold_index_load_and_follow_saves() {
    let temp = tempfile::tempdir().unwrap();
    let writer = DatasetRepository::new(temp.path());
    writer
        .initialize(DatasetMetadata::new(
            DatasetId::from("ds"),
            "Dataset",
            now(),
        ))
        .await
        .unwrap();
    let first = image_record("img_first", "hash-first");
    writer
        .save_images_index(&ImagesIndex {
            schema_version: SCHEMA_VERSION,
            image_count: 1,
            images_by_hash: BTreeMap::from([(first.blake3.clone(), first.clone())]),
        })
        .await
        .unwrap();

    let repository = DatasetRepository::new(temp.path());
    let left = repository.clone();
    let right = repository.clone();
    let image_id = first.image_id.clone();
    let (left_record, right_record) = tokio::join!(
        left.load_image_record(&image_id),
        right.load_image_record(&image_id)
    );
    assert_eq!(left_record.unwrap(), first);
    assert_eq!(right_record.unwrap(), first);
    assert_eq!(repository.images_index_load_count(), 1);

    let second = image_record("img_second", "hash-second");
    repository
        .save_images_index(&ImagesIndex {
            schema_version: SCHEMA_VERSION,
            image_count: 1,
            images_by_hash: BTreeMap::from([(second.blake3.clone(), second.clone())]),
        })
        .await
        .unwrap();
    assert_eq!(
        repository
            .load_image_record(&second.image_id)
            .await
            .unwrap(),
        second
    );
    assert!(repository.load_image_record(&image_id).await.is_err());
    assert_eq!(
        repository.images_index_load_count(),
        1,
        "publishing an index should replace the cached value without reloading it"
    );

    let restarted = DatasetRepository::new(temp.path());
    assert_eq!(
        restarted.load_image_record(&second.image_id).await.unwrap(),
        second
    );
    assert_eq!(restarted.images_index_load_count(), 1);
}

#[tokio::test]
async fn failed_image_index_save_clears_the_cached_value() {
    let temp = tempfile::tempdir().unwrap();
    let repository = DatasetRepository::new(temp.path());
    repository
        .initialize(DatasetMetadata::new(
            DatasetId::from("ds"),
            "Dataset",
            now(),
        ))
        .await
        .unwrap();
    assert!(repository.images_index_cache.read().await.is_some());

    tokio::fs::remove_file(repository.images_index_path())
        .await
        .unwrap();
    tokio::fs::create_dir(repository.images_index_path())
        .await
        .unwrap();
    assert!(
        repository
            .save_images_index(&ImagesIndex::default())
            .await
            .is_err()
    );
    assert!(
        repository.images_index_cache.read().await.is_none(),
        "a failed publication must not leave stale membership cached"
    );
}

#[tokio::test]
async fn appends_events_and_rebuilds_state() {
    let temp = tempfile::tempdir().unwrap();
    let repo = DatasetRepository::new(temp.path());
    repo.initialize(DatasetMetadata::new(
        DatasetId::from("ds"),
        "Dataset",
        now(),
    ))
    .await
    .unwrap();
    let image_id = ImageId::from("img_test");
    let actor = Actor {
        user_id: UserId::from("user_1"),
        role: DatasetRole::Annotator,
    };
    let task_state = TaskState::new(TaskId::from("bounding_box:person"), now());

    let event = repo
        .append_payload(
            &image_id,
            &actor,
            EventPayload::TaskStateChanged { task_state },
        )
        .await
        .unwrap();
    assert_eq!(event.event_sequence, 1);

    let events = repo.load_events(&image_id).await.unwrap();
    assert_eq!(events.len(), 1);
    let rebuilt = repo.rebuild_image_state(&image_id).await.unwrap();
    assert_eq!(rebuilt.current_sequence, 1);
}

#[tokio::test]
async fn loading_missing_image_state_does_not_create_state_file() {
    let temp = tempfile::tempdir().unwrap();
    let repo = DatasetRepository::new(temp.path());
    repo.initialize(DatasetMetadata::new(
        DatasetId::from("ds"),
        "Dataset",
        now(),
    ))
    .await
    .unwrap();
    let image_id = ImageId::from("img_empty");

    let state = repo.load_image_state(&image_id).await.unwrap();

    assert_eq!(state.current_sequence, 0);
    assert!(
        !tokio::fs::try_exists(repo.state_path(&image_id))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn rejects_reinitializing_an_existing_dataset() {
    let temp = tempfile::tempdir().unwrap();
    let repo = DatasetRepository::new(temp.path());
    repo.initialize(DatasetMetadata::new(
        DatasetId::from("ds"),
        "Original",
        now(),
    ))
    .await
    .unwrap();

    let error = repo
        .initialize(DatasetMetadata::new(
            DatasetId::from("ds"),
            "Replacement",
            now(),
        ))
        .await
        .unwrap_err();

    assert!(matches!(error, StorageError::AlreadyExists(_)));
    assert_eq!(repo.load_dataset_config().await.unwrap().name, "Original");
}

#[tokio::test]
async fn validates_event_against_cloned_state_before_append() {
    let temp = tempfile::tempdir().unwrap();
    let repo = DatasetRepository::new(temp.path());
    repo.initialize(DatasetMetadata::new(
        DatasetId::from("ds"),
        "Dataset",
        now(),
    ))
    .await
    .unwrap();
    let image_id = ImageId::from("img_test");
    let actor = Actor {
        user_id: UserId::from("user_1"),
        role: DatasetRole::Annotator,
    };

    let error = repo
        .append_payload(
            &image_id,
            &actor,
            EventPayload::AnnotationDeleted {
                annotation_id: labello_domain::AnnotationId::from("ann_missing"),
                version: 1,
                reason: None,
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(error, StorageError::Domain(_)));
    assert!(repo.load_events(&image_id).await.unwrap().is_empty());
}

#[tokio::test]
async fn validates_entire_resequenced_batch_before_append() {
    let temp = tempfile::tempdir().unwrap();
    let repo = DatasetRepository::new(temp.path());
    repo.initialize(DatasetMetadata::new(
        DatasetId::from("ds"),
        "Dataset",
        now(),
    ))
    .await
    .unwrap();
    let image_id = ImageId::from("img_test");
    let user_id = UserId::from("user_1");
    let mut state = repo.load_image_state(&image_id).await.unwrap();
    let events = vec![
        EventLogEntry::new(
            1,
            image_id.clone(),
            user_id.clone(),
            DatasetRole::Annotator,
            now(),
            EventPayload::TaskStateChanged {
                task_state: TaskState::new(TaskId::from("task_1"), now()),
            },
        ),
        EventLogEntry::new(
            2,
            image_id.clone(),
            user_id,
            DatasetRole::Annotator,
            now(),
            EventPayload::AnnotationDeleted {
                annotation_id: labello_domain::AnnotationId::from("ann_missing"),
                version: 1,
                reason: None,
            },
        ),
    ];

    let error = repo
        .append_resequenced_events(&image_id, &mut state, &events)
        .await
        .unwrap_err();

    assert!(matches!(error, StorageError::Domain(_)));
    assert!(repo.load_events(&image_id).await.unwrap().is_empty());
    assert_eq!(state.current_sequence, 0);
}

#[tokio::test]
async fn load_recovers_state_left_behind_a_complete_event_batch() {
    let temp = tempfile::tempdir().unwrap();
    let repo = DatasetRepository::new(temp.path());
    repo.initialize(DatasetMetadata::new(
        DatasetId::from("ds"),
        "Dataset",
        now(),
    ))
    .await
    .unwrap();
    let image_id = ImageId::from("img_recovery");
    let actor = Actor {
        user_id: UserId::from("user_1"),
        role: DatasetRole::Annotator,
    };
    repo.append_payload(
        &image_id,
        &actor,
        EventPayload::TaskStateChanged {
            task_state: TaskState::new(TaskId::from("task_1"), now()),
        },
    )
    .await
    .unwrap();
    let stale = repo.load_image_state(&image_id).await.unwrap();

    let lock = repo.image_lock(&image_id);
    let _guard = lock.lock().await;
    repo.append_payloads_unlocked(
        &image_id,
        &actor,
        vec![
            EventPayload::TaskStateChanged {
                task_state: TaskState::new(TaskId::from("task_2"), now()),
            },
            EventPayload::TaskStateChanged {
                task_state: TaskState::new(TaskId::from("task_3"), now()),
            },
        ],
    )
    .await
    .unwrap();
    write_json_atomic(&repo.state_path(&image_id), &stale)
        .await
        .unwrap();
    drop(_guard);

    let recovered = repo.load_image_state(&image_id).await.unwrap();
    assert_eq!(recovered.current_sequence, 3);
    assert_eq!(recovered.task_states.len(), 3);
    assert_eq!(
        read_json::<ImageState>(&repo.state_path(&image_id))
            .await
            .unwrap(),
        recovered
    );
}

#[tokio::test]
async fn load_rebuilds_an_absent_state_cache_from_events() {
    let temp = tempfile::tempdir().unwrap();
    let repo = DatasetRepository::new(temp.path());
    repo.initialize(DatasetMetadata::new(
        DatasetId::from("ds"),
        "Dataset",
        now(),
    ))
    .await
    .unwrap();
    let image_id = ImageId::from("img_recovery");
    let task_id = TaskId::from("task_1");
    repo.append_payload(
        &image_id,
        &Actor {
            user_id: UserId::from("user_1"),
            role: DatasetRole::Annotator,
        },
        EventPayload::TaskStateChanged {
            task_state: TaskState::new(task_id.clone(), now()),
        },
    )
    .await
    .unwrap();
    tokio::fs::remove_file(repo.state_path(&image_id))
        .await
        .unwrap();

    let recovered = repo.load_image_state(&image_id).await.unwrap();

    assert_eq!(recovered.current_sequence, 1);
    assert!(recovered.task_states.contains_key(&task_id));
    assert_eq!(
        read_json::<ImageState>(&repo.state_path(&image_id))
            .await
            .unwrap(),
        recovered
    );
}

#[tokio::test]
async fn snapshot_replays_events_and_omits_images_auth_and_keybindings() {
    let temp = tempfile::tempdir().unwrap();
    let repo = DatasetRepository::new(temp.path());
    repo.initialize(DatasetMetadata::new(
        DatasetId::from("ds"),
        "Dataset",
        now(),
    ))
    .await
    .unwrap();
    let image_id = ImageId::from("img_1");
    repo.save_images_index(&ImagesIndex {
        schema_version: SCHEMA_VERSION,
        image_count: 1,
        images_by_hash: BTreeMap::from([(
            "hash".to_string(),
            ImageRecord {
                image_id: image_id.clone(),
                blake3: "hash".to_string(),
                canonical_path: "images/one.png".to_string(),
                known_paths: vec!["images/one.png".to_string()],
                duplicate_paths: Vec::new(),
                file_name: "one.png".to_string(),
                byte_size: 11,
                width: 1,
                height: 1,
                media_type: "image/png".to_string(),
                source_memberships: None,
            },
        )]),
    })
    .await
    .unwrap();
    let dataset_text = tokio::fs::read_to_string(repo.dataset_path())
        .await
        .unwrap()
        .replace("schemaVersion = 3", "schemaVersion = 2");
    assert!(dataset_text.contains("schemaVersion = 2"));
    tokio::fs::write(repo.dataset_path(), &dataset_text)
        .await
        .unwrap();
    let mut index_value: serde_json::Value = read_json(&repo.images_index_path()).await.unwrap();
    index_value["schemaVersion"] = serde_json::json!(2);
    write_json_atomic(&repo.images_index_path(), &index_value)
        .await
        .unwrap();
    let repo = DatasetRepository::new(temp.path());
    assert_eq!(
        repo.load_dataset_config().await.unwrap().schema_version,
        SCHEMA_VERSION
    );
    assert_eq!(
        repo.load_images_index().await.unwrap().schema_version,
        SCHEMA_VERSION
    );
    let task_id = TaskId::from("task_1");
    repo.append_payload(
        &image_id,
        &Actor {
            user_id: UserId::from("user_1"),
            role: DatasetRole::Annotator,
        },
        EventPayload::TaskStateChanged {
            task_state: TaskState::new(task_id.clone(), now()),
        },
    )
    .await
    .unwrap();

    write_json_atomic(
        &repo.state_path(&image_id),
        &ImageState::new(image_id.clone()),
    )
    .await
    .unwrap();
    tokio::fs::write(temp.path().join("images/one.png"), b"image bytes")
        .await
        .unwrap();
    tokio::fs::create_dir_all(temp.path().join("users/user_1"))
        .await
        .unwrap();
    tokio::fs::write(
        temp.path().join("users/user_1/keybindings.toml"),
        b"secret binding",
    )
    .await
    .unwrap();
    tokio::fs::create_dir_all(temp.path().join(".labello-server"))
        .await
        .unwrap();
    tokio::fs::write(
        temp.path().join(".labello-server/auth.json"),
        b"secret auth state",
    )
    .await
    .unwrap();

    let snapshot = repo.create_snapshot().await.unwrap();
    let snapshotted_config_bytes = repo
        .snapshot_file(&snapshot.snapshot_id, paths::DATASET_FILE)
        .await
        .unwrap();
    let snapshotted_config: DatasetConfig =
        toml::from_str(std::str::from_utf8(&snapshotted_config_bytes).unwrap()).unwrap();
    assert_eq!(snapshotted_config.schema_version, SCHEMA_VERSION);
    assert_eq!(snapshotted_config.migration_history.len(), 1);
    assert_eq!(
        snapshotted_config_bytes,
        tokio::fs::read(repo.dataset_path()).await.unwrap()
    );

    assert_eq!(snapshot.schema_version, SCHEMA_VERSION);
    assert!(!snapshot.includes_image_bytes);
    assert!(
        snapshot
            .files
            .iter()
            .all(|file| !file.path.starts_with("images/")
                && !file.path.starts_with("users/")
                && !file.path.starts_with(".labello-server/"))
    );
    let state_path = format!("annotations/{image_id}/state.json");
    let snapshotted_state: ImageState = serde_json::from_slice(
        &repo
            .snapshot_file(&snapshot.snapshot_id, &state_path)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(snapshotted_state.current_sequence, 1);
    assert!(snapshotted_state.task_states.contains_key(&task_id));
}

#[tokio::test]
async fn mixed_v2_v3_repository_rebuilds_current_state_and_preserves_event_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let repo = DatasetRepository::new(temp.path());
    repo.initialize(DatasetMetadata::new(
        DatasetId::from("ds"),
        "Dataset",
        now(),
    ))
    .await
    .unwrap();
    let image_id = ImageId::from("img_mixed");
    repo.save_images_index(&ImagesIndex {
        schema_version: SCHEMA_VERSION,
        image_count: 1,
        images_by_hash: BTreeMap::from([(
            "hash".to_string(),
            ImageRecord {
                image_id: image_id.clone(),
                blake3: "hash".to_string(),
                canonical_path: "images/mixed.png".to_string(),
                known_paths: vec!["images/mixed.png".to_string()],
                duplicate_paths: Vec::new(),
                file_name: "mixed.png".to_string(),
                byte_size: 1,
                width: 10,
                height: 10,
                media_type: "image/png".to_string(),
                source_memberships: None,
            },
        )]),
    })
    .await
    .unwrap();

    let timestamp = now();
    let first = labello_domain::AnnotationVersion {
        annotation_id: AnnotationId::from("ann_legacy"),
        version: 1,
        object_group_id: None,
        origin: AnnotationOrigin::legacy_v2(),
        task_id: TaskId::from("boxes"),
        class_id: ClassId::from("person"),
        annotation_type: AnnotationType::BoundingBox,
        revision_source: RevisionSource::Human {
            action: HumanRevisionKind::Authored,
        },
        geometry: AnnotationGeometry::BoundingBox(BoundingBox {
            x: 0.1,
            y: 0.1,
            width: 0.2,
            height: 0.2,
        }),
        author_user_id: UserId::from("annotator"),
        created_at: timestamp,
        updated_at: timestamp,
        deleted: false,
    };
    let mut legacy_event = EventLogEntry::new(
        1,
        image_id.clone(),
        UserId::from("annotator"),
        DatasetRole::Annotator,
        timestamp,
        EventPayload::AnnotationVersionCreated {
            annotation: first.clone(),
            previous_version: None,
            reason: None,
        },
    );
    legacy_event.schema_version = labello_domain::LEGACY_SCHEMA_VERSION;
    let legacy_bytes = format!("  {}  \n", serde_json::to_string(&legacy_event).unwrap());
    tokio::fs::create_dir_all(repo.annotations_dir(&image_id))
        .await
        .unwrap();
    tokio::fs::write(repo.events_path(&image_id), legacy_bytes.as_bytes())
        .await
        .unwrap();

    let rebuilt = repo.load_image_state(&image_id).await.unwrap();
    assert_eq!(rebuilt.schema_version, SCHEMA_VERSION);
    assert!(
        rebuilt
            .current_annotation(&first.annotation_id)
            .unwrap()
            .origin
            .is_legacy_v2()
    );

    let mut second = first;
    second.version = 2;
    second.revision_source = RevisionSource::Human {
        action: HumanRevisionKind::Edited,
    };
    second.updated_at = now();
    repo.append_payload(
        &image_id,
        &Actor {
            user_id: UserId::from("annotator"),
            role: DatasetRole::Annotator,
        },
        EventPayload::AnnotationVersionCreated {
            annotation: second,
            previous_version: Some(1),
            reason: Some("edit".to_string()),
        },
    )
    .await
    .unwrap();
    let repository_event_bytes = tokio::fs::read(repo.events_path(&image_id)).await.unwrap();
    assert!(repository_event_bytes.starts_with(legacy_bytes.as_bytes()));
    assert_eq!(
        repo.load_events(&image_id)
            .await
            .unwrap()
            .iter()
            .map(|event| event.schema_version)
            .collect::<Vec<_>>(),
        vec![labello_domain::LEGACY_SCHEMA_VERSION, SCHEMA_VERSION]
    );

    let snapshot = repo.create_snapshot().await.unwrap();
    let events_relative = format!("annotations/{image_id}/events.jsonl");
    assert_eq!(
        repo.snapshot_file(&snapshot.snapshot_id, &events_relative)
            .await
            .unwrap(),
        repository_event_bytes
    );
    let state_relative = format!("annotations/{image_id}/state.json");
    let snapshotted_state: ImageState = serde_json::from_slice(
        &repo
            .snapshot_file(&snapshot.snapshot_id, &state_relative)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(snapshotted_state.schema_version, SCHEMA_VERSION);
    assert_eq!(snapshotted_state.current_sequence, 2);
}

async fn prepare_v2_artifact_migration_fixture(root: &Path) -> (ImageId, Vec<u8>, UserId) {
    let repo = DatasetRepository::new(root);
    repo.initialize(DatasetMetadata::new(
        DatasetId::from("ds"),
        "Dataset",
        now(),
    ))
    .await
    .unwrap();
    let image_id = ImageId::from("img_migration");
    repo.save_images_index(&ImagesIndex {
        schema_version: SCHEMA_VERSION,
        image_count: 1,
        images_by_hash: BTreeMap::from([(
            "hash".to_string(),
            ImageRecord {
                image_id: image_id.clone(),
                blake3: "hash".to_string(),
                canonical_path: "images/migration.png".to_string(),
                known_paths: vec!["images/migration.png".to_string()],
                duplicate_paths: Vec::new(),
                file_name: "migration.png".to_string(),
                byte_size: 1,
                width: 10,
                height: 10,
                media_type: "image/png".to_string(),
                source_memberships: None,
            },
        )]),
    })
    .await
    .unwrap();
    let user_id = UserId::from("user_1");
    repo.save_keybindings(&KeybindingSet::defaults_for(user_id.clone()))
        .await
        .unwrap();

    let timestamp = now();
    let mut v2 = EventLogEntry::new(
        1,
        image_id.clone(),
        user_id.clone(),
        DatasetRole::Annotator,
        timestamp,
        EventPayload::TaskStateChanged {
            task_state: TaskState::new(TaskId::from("legacy_task"), timestamp),
        },
    );
    v2.schema_version = LEGACY_SCHEMA_VERSION;
    let v3 = EventLogEntry::new(
        2,
        image_id.clone(),
        user_id.clone(),
        DatasetRole::Annotator,
        timestamp,
        EventPayload::TaskStateChanged {
            task_state: TaskState::new(TaskId::from("current_task"), timestamp),
        },
    );
    let event_bytes = format!(
        "  {}  \n{}\n",
        serde_json::to_string(&v2).unwrap(),
        serde_json::to_string(&v3).unwrap()
    )
    .into_bytes();
    tokio::fs::create_dir_all(repo.annotations_dir(&image_id))
        .await
        .unwrap();
    tokio::fs::write(repo.events_path(&image_id), &event_bytes)
        .await
        .unwrap();
    let mut stale_state = serde_json::to_value(ImageState::new(image_id.clone())).unwrap();
    stale_state["schemaVersion"] = serde_json::json!(LEGACY_SCHEMA_VERSION);
    write_json_atomic(&repo.state_path(&image_id), &stale_state)
        .await
        .unwrap();

    for path in [
        repo.dataset_path(),
        root.join(paths::USERS_DIR)
            .join(user_id.as_str())
            .join(paths::KEYBINDINGS_FILE),
    ] {
        let text = tokio::fs::read_to_string(&path).await.unwrap();
        tokio::fs::write(
            &path,
            text.replace("schemaVersion = 3", "schemaVersion = 2"),
        )
        .await
        .unwrap();
    }
    let mut index: serde_json::Value = read_json(&repo.images_index_path()).await.unwrap();
    index["schemaVersion"] = serde_json::json!(LEGACY_SCHEMA_VERSION);
    write_json_atomic(&repo.images_index_path(), &index)
        .await
        .unwrap();
    write_json_atomic(
        &repo.schema_path(),
        &serde_json::json!({ "schemaVersion": LEGACY_SCHEMA_VERSION }),
    )
    .await
    .unwrap();
    (image_id, event_bytes, user_id)
}

fn image_record(image_id: &str, hash: &str) -> ImageRecord {
    ImageRecord {
        image_id: ImageId::from(image_id),
        blake3: hash.to_string(),
        canonical_path: format!("images/{image_id}.png"),
        known_paths: vec![format!("images/{image_id}.png")],
        duplicate_paths: Vec::new(),
        file_name: format!("{image_id}.png"),
        byte_size: 1,
        width: 10,
        height: 10,
        media_type: "image/png".to_string(),
        source_memberships: None,
    }
}

#[tokio::test]
async fn artifact_migration_recovers_after_every_phase_and_preserves_mixed_events() {
    let phases = [
        ArtifactMigrationPhase::GenerationPrepared,
        ArtifactMigrationPhase::DatasetConfigPublished,
        ArtifactMigrationPhase::ImagesIndexPublished,
        ArtifactMigrationPhase::SchemaPublished,
        ArtifactMigrationPhase::KeybindingsPublished,
        ArtifactMigrationPhase::StatesRebuilt,
        ArtifactMigrationPhase::Completed,
    ];

    for failed_phase in phases {
        let temp = tempfile::tempdir().unwrap();
        let (image_id, event_bytes, user_id) =
            prepare_v2_artifact_migration_fixture(temp.path()).await;
        let interrupted = DatasetRepository::new(temp.path());
        interrupted.fail_artifact_migration_after(failed_phase);
        assert!(
            interrupted.load_dataset_config().await.is_err(),
            "migration did not fail after {failed_phase:?}"
        );

        let interrupted_journal: ArtifactMigrationJournal =
            read_json(&interrupted.artifact_migration_journal_path())
                .await
                .unwrap();
        assert_eq!(interrupted_journal.phase, failed_phase);
        assert_eq!(
            tokio::fs::read(interrupted.events_path(&image_id))
                .await
                .unwrap(),
            event_bytes
        );

        let restarted = DatasetRepository::new(temp.path());
        let dataset = restarted.load_dataset().await.unwrap();
        assert_eq!(dataset.schema_version, SCHEMA_VERSION);
        assert_eq!(dataset.images.len(), 1);
        assert_eq!(dataset.migration_history.len(), 1);
        assert_eq!(dataset.migration_history[0].from_version, 2);
        assert_eq!(dataset.migration_history[0].to_version, 3);
        assert_eq!(
            dataset.migration_history[0].name,
            "schema-v2-to-v3-artifacts"
        );
        let index = restarted.load_images_index().await.unwrap();
        assert_eq!(index.schema_version, SCHEMA_VERSION);
        assert_eq!(index.image_count, 1);
        let schema: serde_json::Value = read_json(&restarted.schema_path()).await.unwrap();
        assert_eq!(schema["schemaVersion"], SCHEMA_VERSION);
        assert_eq!(
            schema["eventLogEntry"]["oneOf"].as_array().unwrap().len(),
            2
        );
        assert_eq!(
            restarted
                .load_keybindings(&user_id)
                .await
                .unwrap()
                .schema_version,
            SCHEMA_VERSION
        );
        let state = restarted.load_image_state(&image_id).await.unwrap();
        assert_eq!(state.schema_version, SCHEMA_VERSION);
        assert_eq!(state.current_sequence, 2);
        assert!(state.task_states.contains_key(&TaskId::from("legacy_task")));
        assert!(
            state
                .task_states
                .contains_key(&TaskId::from("current_task"))
        );
        assert_eq!(
            tokio::fs::read(restarted.events_path(&image_id))
                .await
                .unwrap(),
            event_bytes
        );

        let completed: ArtifactMigrationJournal =
            read_json(&restarted.artifact_migration_journal_path())
                .await
                .unwrap();
        assert_eq!(completed.generation, interrupted_journal.generation);
        assert_eq!(completed.phase, ArtifactMigrationPhase::Completed);
        assert_eq!(
            completed
                .phase_history
                .iter()
                .map(|record| record.phase)
                .collect::<Vec<_>>(),
            phases
        );
        for file in completed.files {
            let generated = restarted
                .artifact_migration_generation_dir(completed.generation)
                .join(file.relative_path);
            let bytes = tokio::fs::read(generated).await.unwrap();
            assert_eq!(blake3::hash(&bytes).to_hex().as_str(), file.blake3);
        }
    }
}

#[tokio::test]
async fn artifact_migration_finishes_a_preexisting_hybrid_dataset() {
    let temp = tempfile::tempdir().unwrap();
    let (image_id, event_bytes, _) = prepare_v2_artifact_migration_fixture(temp.path()).await;
    let dataset_path = temp.path().join(paths::DATASET_FILE);
    let text = tokio::fs::read_to_string(&dataset_path).await.unwrap();
    tokio::fs::write(
        &dataset_path,
        text.replace("schemaVersion = 2", "schemaVersion = 3"),
    )
    .await
    .unwrap();

    let restarted = DatasetRepository::new(temp.path());
    let dataset = restarted.load_dataset().await.unwrap();

    assert_eq!(dataset.schema_version, SCHEMA_VERSION);
    assert_eq!(dataset.migration_history.len(), 1);
    assert_eq!(
        read_schema_version(&restarted.images_index_path())
            .await
            .unwrap(),
        SCHEMA_VERSION
    );
    assert_eq!(
        tokio::fs::read(restarted.events_path(&image_id))
            .await
            .unwrap(),
        event_bytes
    );
    assert_eq!(
        read_json::<ArtifactMigrationJournal>(&restarted.artifact_migration_journal_path())
            .await
            .unwrap()
            .phase,
        ArtifactMigrationPhase::Completed
    );
}

#[tokio::test]
async fn snapshot_includes_committed_import_records() {
    let temp = tempfile::tempdir().unwrap();
    let repo = DatasetRepository::new(temp.path());
    repo.initialize(DatasetMetadata::new(
        DatasetId::from("ds"),
        "Dataset",
        now(),
    ))
    .await
    .unwrap();
    let import_id = ImportId::from("imp_1");
    let import_directory = repo.imports_dir().join(import_id.as_str());
    tokio::fs::create_dir_all(&import_directory).await.unwrap();
    let manifest = ImportManifest {
        schema_version: SCHEMA_VERSION,
        import_id: import_id.clone(),
        dataset_id: DatasetId::from("ds"),
        source_profile: SourceProfile {
            profile_id: "fixture".to_string(),
            profile_version: 1,
        },
        source_fingerprint: "source".to_string(),
        plan_hash: "plan".to_string(),
        parser_version: "1".to_string(),
        tool_version: "1".to_string(),
        descriptors: Vec::new(),
        source_files: Vec::new(),
        attestations: labello_domain::ImportAttestations {
            ground_truth: true,
            exhaustive: true,
            coverage_scope: Vec::new(),
            provenance: "fixture".to_string(),
        },
        compatibility_policies: Default::default(),
        transform_policies: Default::default(),
        acknowledged_warning_codes: Vec::new(),
        category_mappings: Vec::new(),
        geometry_mappings: Vec::new(),
        task_mappings: Vec::new(),
        skeleton_mappings: Vec::new(),
        manual_migration_mappings: Vec::new(),
        source_memberships: Default::default(),
        coverage_totals: Default::default(),
        migration_totals: Default::default(),
        output_totals: Default::default(),
        output_integrity: Default::default(),
        created_by: UserId::from("admin"),
        created_at: now(),
    };
    write_json_atomic(
        &import_directory.join(paths::IMPORT_MANIFEST_FILE),
        &manifest,
    )
    .await
    .unwrap();
    let source_objects = b"{\"sourceObjectKey\":\"object/1\"}\n";
    tokio::fs::write(
        import_directory.join(paths::IMPORT_SOURCE_OBJECTS_FILE),
        source_objects,
    )
    .await
    .unwrap();

    assert_eq!(
        repo.load_import_manifests().await.unwrap(),
        vec![manifest.clone()]
    );
    let snapshot = repo.create_snapshot().await.unwrap();
    assert_eq!(snapshot.imports.len(), 1);
    assert_eq!(snapshot.imports[0].import_id, import_id);
    let snapshotted_manifest: ImportManifest = serde_json::from_slice(
        &repo
            .snapshot_file(&snapshot.snapshot_id, &snapshot.imports[0].manifest_path)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(snapshotted_manifest, manifest);
    assert_eq!(
        repo.snapshot_file(
            &snapshot.snapshot_id,
            &snapshot.imports[0].source_objects_path,
        )
        .await
        .unwrap(),
        source_objects
    );
}

#[test]
fn rejects_image_path_traversal() {
    let repo = DatasetRepository::new("/tmp/labello-dataset");
    assert!(repo.image_path("images/frame.png").is_ok());
    assert!(repo.image_path("../secret.png").is_err());
    assert!(repo.image_path("/etc/passwd").is_err());
}

#[test]
fn extracts_image_count_hint_from_index_prefix() {
    assert_eq!(
        extract_image_count_hint(r#"{"schemaVersion":1,"imageCount":42,"imagesByHash":{}}"#),
        Some(42)
    );
    assert_eq!(extract_image_count_hint(r#"{"imagesByHash":{}}"#), None);
}
