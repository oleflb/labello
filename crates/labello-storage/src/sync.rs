use labello_domain::{
    Actor, AnnotationOrigin, AnnotationVersion, DatasetMetadata, DatasetRole, EventLogEntry,
    EventPayload, HumanRevisionKind, ImageState, OfflineAnnotationSource, OfflineBundle,
    OfflineImageBundle, OfflineMutation, OfflineSyncRequest, OfflineSyncResult, RevisionSource,
    SyncConflict, require_role,
};

use crate::{DatasetRepository, StorageError, StorageResult};

impl DatasetRepository {
    pub async fn create_offline_bundle(
        &self,
        user_id: &labello_domain::UserId,
        limit: usize,
        include_image_bytes: bool,
    ) -> StorageResult<OfflineBundle> {
        let metadata = self.load_dataset().await?;
        let roles = metadata
            .role_assignments
            .iter()
            .find(|assignment| {
                assignment.user_id == *user_id && assignment.dataset_id == metadata.dataset_id
            })
            .map(|assignment| assignment.roles.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        require_role(
            &metadata.role_assignments,
            &metadata.dataset_id,
            user_id,
            DatasetRole::Annotator,
        )?;
        let mut images = Vec::new();
        for record in metadata.images.values().take(limit) {
            let state = self.load_image_state(&record.image_id).await?;
            let events = self.load_events(&record.image_id).await?;
            let image_bytes_base64 = if include_image_bytes {
                let path = self.image_path(&record.canonical_path)?;
                let bytes = tokio::fs::read(&path)
                    .await
                    .map_err(|source| crate::StorageError::Io { path, source })?;
                Some(base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    bytes,
                ))
            } else {
                None
            };
            images.push(OfflineImageBundle {
                image: record.clone(),
                event_log_fragment: labello_domain::EventLogFragment {
                    image_id: record.image_id.clone(),
                    base_sequence: state.current_sequence,
                    events,
                },
                state,
                image_bytes_base64,
            });
        }
        Ok(OfflineBundle {
            schema_version: labello_domain::SCHEMA_VERSION,
            dataset_id: metadata.dataset_id,
            user_id: user_id.clone(),
            created_at: labello_domain::now(),
            expires_at: None,
            roles,
            tasks: metadata.tasks,
            images,
            import_manifests: self.load_import_manifests().await?,
        })
    }

    pub async fn sync_offline_events(
        &self,
        request: OfflineSyncRequest,
    ) -> StorageResult<OfflineSyncResult> {
        labello_domain::validate_supported_schema_version(request.schema_version)?;
        let metadata = self.load_dataset().await?;
        if request.dataset_id != metadata.dataset_id {
            return Err(StorageError::InvalidAssignment(
                "offline sync dataset does not match the repository".to_string(),
            ));
        }
        if request.fragments.len() > labello_domain::MAX_OFFLINE_FRAGMENTS {
            return Err(StorageError::InvalidAssignment(
                "offline sync exceeds the fragment limit".to_string(),
            ));
        }
        let mutation_count = request
            .fragments
            .iter()
            .try_fold(0usize, |count, fragment| {
                count.checked_add(fragment.mutations.len())
            })
            .filter(|count| *count <= labello_domain::MAX_OFFLINE_MUTATIONS);
        if mutation_count.is_none() {
            return Err(StorageError::InvalidAssignment(
                "offline sync exceeds the mutation limit".to_string(),
            ));
        }
        require_role(
            &metadata.role_assignments,
            &metadata.dataset_id,
            &request.user_id,
            DatasetRole::Annotator,
        )?;
        let mut result = OfflineSyncResult {
            merged_events: 0,
            conflicts: Vec::new(),
        };

        for fragment in request.fragments {
            if fragment.mutations.len() > labello_domain::MAX_OFFLINE_MUTATIONS_PER_FRAGMENT {
                return Err(StorageError::InvalidAssignment(
                    "offline sync fragment exceeds the mutation limit".to_string(),
                ));
            }
            fragment
                .image_id
                .validate_path_segment()
                .map_err(|error| StorageError::InvalidAssignment(error.to_string()))?;
            if !metadata.images.contains_key(&fragment.image_id) {
                return Err(StorageError::InvalidAssignment(format!(
                    "image {} does not belong to the dataset",
                    fragment.image_id
                )));
            }
            let lock = self.image_lock(&fragment.image_id);
            let _guard = lock.lock().await;
            let mut state = self.load_image_state(&fragment.image_id).await?;
            if state.current_sequence != fragment.base_sequence {
                result.conflicts.push(SyncConflict {
                    image_id: fragment.image_id,
                    reason: "server event log advanced after offline bundle was created"
                        .to_string(),
                    server_sequence: state.current_sequence,
                    client_base_sequence: fragment.base_sequence,
                });
                continue;
            }
            let actor = Actor {
                user_id: request.user_id.clone(),
                role: DatasetRole::Annotator,
            };
            let timestamp = labello_domain::now();
            let mut payloads = Vec::with_capacity(fragment.mutations.len());
            for mutation in fragment.mutations {
                let payload =
                    construct_offline_mutation(&metadata, &state, &actor, timestamp, mutation)?;
                let event = EventLogEntry::new(
                    state.current_sequence + 1,
                    fragment.image_id.clone(),
                    actor.user_id.clone(),
                    actor.role.clone(),
                    timestamp,
                    payload.clone(),
                );
                state.apply_event(&event)?;
                payloads.push(payload);
            }
            result.merged_events += payloads.len();
            self.append_payloads_unlocked(&fragment.image_id, &actor, payloads)
                .await?;
        }
        Ok(result)
    }
}

fn construct_offline_mutation(
    metadata: &DatasetMetadata,
    state: &ImageState,
    actor: &Actor,
    timestamp: labello_domain::Timestamp,
    mutation: OfflineMutation,
) -> StorageResult<EventPayload> {
    match mutation {
        OfflineMutation::AnnotationUpsert {
            annotation_id,
            expected_version,
            task_id,
            class_id,
            annotation_type,
            source,
            geometry,
            reason,
        } => {
            annotation_id
                .validate_path_segment()
                .map_err(|error| StorageError::InvalidAssignment(error.to_string()))?;
            task_id
                .validate_path_segment()
                .map_err(|error| StorageError::InvalidAssignment(error.to_string()))?;
            validate_offline_reason(reason.as_deref())?;
            let task = metadata.task(&task_id).ok_or_else(|| {
                StorageError::InvalidAssignment(format!("unknown offline mutation task {task_id}"))
            })?;
            if task.manual_box_guide_migration.is_some() {
                return Err(StorageError::InvalidAssignment(
                    "manual migration mutations require the migration command workflow".to_string(),
                ));
            }
            let image = metadata.images.get(&state.image_id).ok_or_else(|| {
                StorageError::InvalidAssignment(format!("unknown offline image {}", state.image_id))
            })?;
            let current = state.current_annotation(&annotation_id);
            let (version, previous_version, origin, object_group_id, created_at, revision_source) =
                match current {
                    Some(current) => {
                        if expected_version != Some(current.version)
                            || task_id != current.task_id
                            || annotation_type != current.annotation_type
                        {
                            return Err(StorageError::InvalidAssignment(format!(
                                "offline mutation for annotation {annotation_id} is stale or changes immutable fields"
                            )));
                        }
                        let version = current.version.checked_add(1).ok_or_else(|| {
                            StorageError::InvalidAssignment(
                                "annotation version overflow".to_string(),
                            )
                        })?;
                        let action = if geometry == current.geometry {
                            HumanRevisionKind::AcceptedUnchanged
                        } else {
                            HumanRevisionKind::Edited
                        };
                        (
                            version,
                            Some(current.version),
                            current.origin.clone(),
                            current.object_group_id.clone(),
                            current.created_at,
                            RevisionSource::Human { action },
                        )
                    }
                    None => {
                        if expected_version.is_some() {
                            return Err(StorageError::InvalidAssignment(format!(
                                "offline mutation expected a missing annotation {annotation_id} to exist"
                            )));
                        }
                        let revision_source = match source {
                            OfflineAnnotationSource::Human => RevisionSource::Human {
                                action: HumanRevisionKind::Authored,
                            },
                            OfflineAnnotationSource::PrelabelSuggestion {
                                config_id,
                                model_id,
                                confidence,
                            } => {
                                let valid = confidence.is_finite()
                                    && (0.0..=1.0).contains(&confidence)
                                    && metadata.prelabel_configs.iter().any(|config| {
                                        config.available_to_annotators
                                            && config.config_id == config_id
                                            && config.model.model_id == model_id
                                    });
                                if !valid {
                                    return Err(StorageError::InvalidAssignment(
                                        "offline prelabel source is not available".to_string(),
                                    ));
                                }
                                RevisionSource::PrelabelSuggestion {
                                    config_id,
                                    model_id,
                                    confidence,
                                }
                            }
                        };
                        (
                            1,
                            None,
                            AnnotationOrigin::native(),
                            None,
                            timestamp,
                            revision_source,
                        )
                    }
                };
            let annotation = AnnotationVersion {
                annotation_id,
                version,
                object_group_id,
                origin,
                task_id,
                class_id,
                annotation_type,
                revision_source,
                geometry,
                author_user_id: actor.user_id.clone(),
                created_at,
                updated_at: timestamp,
                deleted: false,
            };
            annotation
                .validate_for_task(task, image.dimensions())
                .map_err(|error| StorageError::InvalidAssignment(error.to_string()))?;
            Ok(EventPayload::AnnotationVersionCreated {
                annotation,
                previous_version,
                reason,
            })
        }
        OfflineMutation::AnnotationDelete {
            annotation_id,
            expected_version,
            reason,
        } => {
            annotation_id
                .validate_path_segment()
                .map_err(|error| StorageError::InvalidAssignment(error.to_string()))?;
            validate_offline_reason(reason.as_deref())?;
            let current = state.current_annotation(&annotation_id).ok_or_else(|| {
                StorageError::InvalidAssignment(format!("unknown annotation {annotation_id}"))
            })?;
            if metadata
                .task(&current.task_id)
                .is_some_and(|task| task.manual_box_guide_migration.is_some())
            {
                return Err(StorageError::InvalidAssignment(
                    "manual migration mutations require the migration command workflow".to_string(),
                ));
            }
            if current.deleted || current.version != expected_version {
                return Err(StorageError::InvalidAssignment(format!(
                    "offline deletion for annotation {annotation_id} is stale"
                )));
            }
            Ok(EventPayload::AnnotationDeleted {
                annotation_id,
                version: expected_version,
                reason,
            })
        }
    }
}

fn validate_offline_reason(reason: Option<&str>) -> StorageResult<()> {
    if reason.is_some_and(|reason| reason.len() > labello_domain::MAX_OFFLINE_REASON_BYTES) {
        Err(StorageError::InvalidAssignment(
            "offline mutation reason exceeds the limit".to_string(),
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use labello_domain::{
        Actor, AnnotationGeometry, AnnotationId, AnnotationOrigin, AnnotationType, BoundingBox,
        ClassId, DatasetId, DatasetMetadata, DatasetRoleAssignment, EventPayload,
        HumanRevisionKind, ImageId, ImageRecord, ImagesIndex, ImportId, ImportManifest,
        RevisionSource, SCHEMA_VERSION, SourceProfile, TaskId, UserId, now,
    };

    use super::*;

    #[tokio::test]
    async fn detects_offline_sync_conflicts() {
        let temp = tempfile::tempdir().unwrap();
        let repo = DatasetRepository::new(temp.path());
        let user_id = UserId::from("user_1");
        let mut metadata = DatasetMetadata::new(DatasetId::from("ds"), "Dataset", now());
        metadata.role_assignments.push(DatasetRoleAssignment {
            dataset_id: metadata.dataset_id.clone(),
            user_id: user_id.clone(),
            roles: BTreeSet::from([DatasetRole::Annotator]),
            assigned_at: now(),
            assigned_by: None,
        });
        let image_id = ImageId::from("img_test");
        repo.initialize(metadata).await.unwrap();
        repo.save_images_index(&ImagesIndex {
            schema_version: SCHEMA_VERSION,
            image_count: 1,
            images_by_hash: BTreeMap::from([(
                "hash".to_string(),
                ImageRecord {
                    image_id: image_id.clone(),
                    blake3: "hash".to_string(),
                    canonical_path: "images/test.png".to_string(),
                    known_paths: vec!["images/test.png".to_string()],
                    duplicate_paths: Vec::new(),
                    file_name: "test.png".to_string(),
                    byte_size: 1,
                    width: 1,
                    height: 1,
                    media_type: "image/png".to_string(),
                    source_memberships: None,
                },
            )]),
        })
        .await
        .unwrap();
        let actor = Actor {
            user_id: user_id.clone(),
            role: DatasetRole::Annotator,
        };
        let payload = EventPayload::AnnotationVersionCreated {
            annotation: labello_domain::AnnotationVersion {
                annotation_id: AnnotationId::from("ann_online"),
                version: 1,
                object_group_id: None,
                origin: AnnotationOrigin::native(),
                task_id: TaskId::from("bounding_box:person"),
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
                author_user_id: user_id.clone(),
                created_at: now(),
                updated_at: now(),
                deleted: false,
            },
            previous_version: None,
            reason: None,
        };
        repo.append_payload(&image_id, &actor, payload)
            .await
            .unwrap();
        let request = OfflineSyncRequest::new(
            DatasetId::from("ds"),
            user_id.clone(),
            vec![labello_domain::OfflineMutationFragment {
                image_id: image_id.clone(),
                base_sequence: 0,
                mutations: Vec::new(),
            }],
        );
        let result = repo.sync_offline_events(request).await.unwrap();
        assert_eq!(result.merged_events, 0);
        assert_eq!(result.conflicts.len(), 1);
    }

    #[tokio::test]
    async fn offline_bundle_includes_committed_import_manifests() {
        let temp = tempfile::tempdir().unwrap();
        let repo = DatasetRepository::new(temp.path());
        let user_id = UserId::from("annotator");
        let mut metadata = DatasetMetadata::new(DatasetId::from("ds"), "Dataset", now());
        metadata.role_assignments.push(DatasetRoleAssignment {
            dataset_id: metadata.dataset_id.clone(),
            user_id: user_id.clone(),
            roles: BTreeSet::from([DatasetRole::Annotator]),
            assigned_at: now(),
            assigned_by: None,
        });
        repo.initialize(metadata).await.unwrap();
        let import_directory = repo.imports_dir().join("imp_1");
        tokio::fs::create_dir_all(&import_directory).await.unwrap();
        let manifest = ImportManifest {
            schema_version: SCHEMA_VERSION,
            import_id: ImportId::from("imp_1"),
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
        crate::fsjson::write_json_atomic(
            &import_directory.join(crate::paths::IMPORT_MANIFEST_FILE),
            &manifest,
        )
        .await
        .unwrap();
        tokio::fs::write(
            import_directory.join(crate::paths::IMPORT_SOURCE_OBJECTS_FILE),
            b"",
        )
        .await
        .unwrap();

        let bundle = repo
            .create_offline_bundle(&user_id, 10, false)
            .await
            .unwrap();
        assert_eq!(bundle.import_manifests, vec![manifest]);
    }
}
