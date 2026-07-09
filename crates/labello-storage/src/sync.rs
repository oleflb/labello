use labello_domain::{
    DatasetRole, OfflineBundle, OfflineImageBundle, OfflineSyncRequest, OfflineSyncResult,
    SyncConflict, require_role,
};

use crate::{DatasetRepository, StorageResult};

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
        })
    }

    pub async fn sync_offline_events(
        &self,
        request: OfflineSyncRequest,
    ) -> StorageResult<OfflineSyncResult> {
        labello_domain::validate_schema_version(request.schema_version)?;
        let metadata = self.load_dataset().await?;
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
            for event in &fragment.events {
                require_role(
                    &metadata.role_assignments,
                    &metadata.dataset_id,
                    &event.actor_user_id,
                    event.actor_role.clone(),
                )?;
            }
            result.merged_events += self
                .append_resequenced_events(&fragment.image_id, &mut state, &fragment.events)
                .await?;
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use labello_domain::{
        Actor, AnnotationGeometry, AnnotationId, AnnotationSource, AnnotationType, BoundingBox,
        ClassId, DatasetId, DatasetMetadata, DatasetRoleAssignment, EventLogEntry, EventPayload,
        ImageId, TaskId, UserId, now,
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
        let actor = Actor {
            user_id: user_id.clone(),
            role: DatasetRole::Annotator,
        };
        let payload = EventPayload::AnnotationVersionCreated {
            annotation: labello_domain::AnnotationVersion {
                annotation_id: AnnotationId::from("ann_online"),
                version: 1,
                task_id: TaskId::from("bounding_box:person"),
                class_id: ClassId::from("person"),
                annotation_type: AnnotationType::BoundingBox,
                source: AnnotationSource::Human,
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
            vec![labello_domain::EventLogFragment {
                image_id: image_id.clone(),
                base_sequence: 0,
                events: vec![EventLogEntry::new(
                    1,
                    image_id,
                    user_id,
                    DatasetRole::Annotator,
                    now(),
                    EventPayload::TaskStateChanged {
                        task_state: labello_domain::TaskState::new(TaskId::from("x"), now()),
                    },
                )],
            }],
        );
        let result = repo.sync_offline_events(request).await.unwrap();
        assert_eq!(result.merged_events, 0);
        assert_eq!(result.conflicts.len(), 1);
    }
}
