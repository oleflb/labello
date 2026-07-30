impl TaskApi for DemoLabelloApi {
    fn list_tasks<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, Vec<TaskDefinition>> {
        Box::pin(async move { Ok(self.dataset(dataset_id)?.tasks) })
    }

    fn add_task<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        task: TaskDefinition,
    ) -> crate::ApiFuture<'a, TaskDefinition> {
        Box::pin(async move {
            let mut state = self.state.borrow_mut();
            let dataset = state
                .datasets
                .get_mut(dataset_id)
                .ok_or_else(|| ClientError::Demo(format!("dataset {dataset_id} does not exist")))?;
            dataset
                .tasks
                .retain(|existing| existing.task_id != task.task_id);
            dataset.tasks.push(task.clone());
            Ok(task)
        })
    }
}

impl ImageApi for DemoLabelloApi {
    fn assignment_availability<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        request: crate::AssignmentAvailabilityRequest,
    ) -> crate::ApiFuture<'a, crate::AssignmentAvailability> {
        Box::pin(async move {
            Ok(crate::AssignmentAvailability {
                kind: request.kind,
                tasks: std::collections::BTreeMap::new(),
                related: Vec::new(),
            })
        })
    }

    fn assign_next_image<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _request: AssignNextRequest,
    ) -> crate::ApiFuture<'a, Option<Assignment>> {
        Box::pin(async move { Ok(None) })
    }

    fn revalidate_assignment<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _image_id: &'a ImageId,
        _request: AssignmentActionRequest,
    ) -> crate::ApiFuture<'a, Option<crate::AssignmentRevalidation>> {
        Box::pin(async move { Ok(None) })
    }

    fn release_assignment<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _request: AssignmentActionRequest,
    ) -> crate::ApiFuture<'a, Assignment> {
        Box::pin(async move {
            Err(ClientError::Demo(
                "the demo backend does not create assignments".to_string(),
            ))
        })
    }

    fn complete_assignment<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _request: AssignmentActionRequest,
    ) -> crate::ApiFuture<'a, Assignment> {
        Box::pin(async move {
            Err(ClientError::Demo(
                "the demo backend does not create assignments".to_string(),
            ))
        })
    }

    fn reopen_assignment<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _request: AssignmentActionRequest,
    ) -> crate::ApiFuture<'a, Assignment> {
        Box::pin(async move {
            Err(ClientError::Demo(
                "the demo backend does not create assignments".to_string(),
            ))
        })
    }

    fn get_image_state<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> crate::ApiFuture<'a, ImageState> {
        Box::pin(async move { Ok(ImageState::new(image_id.clone())) })
    }

    fn get_image_record<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> crate::ApiFuture<'a, ImageRecord> {
        Box::pin(async move {
            Ok(ImageRecord {
                image_id: image_id.clone(),
                blake3: image_id.to_string(),
                canonical_path: format!("images/{image_id}.png"),
                known_paths: vec![],
                duplicate_paths: vec![],
                file_name: format!("{image_id}.png"),
                byte_size: 4,
                width: 1,
                height: 1,
                media_type: "image/png".to_string(),
                source_memberships: None,
            })
        })
    }

    fn get_image_file<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> crate::ApiFuture<'a, ImageFile> {
        Box::pin(async move {
            Ok(ImageFile {
                image_id: image_id.clone(),
                media_type: "application/octet-stream".to_string(),
                bytes: Vec::new(),
            })
        })
    }

    fn get_image_preview<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        _max_dimension: u32,
    ) -> crate::ApiFuture<'a, ImagePreview> {
        Box::pin(async move {
            Ok(ImagePreview {
                image_id: image_id.clone(),
                width: 1,
                height: 1,
                rgba: vec![18, 23, 34, 255],
            })
        })
    }

    fn rebuild_image<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> crate::ApiFuture<'a, ImageState> {
        self.get_image_state(dataset_id, image_id)
    }
}

impl AnnotationApi for DemoLabelloApi {
    fn append_event<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: AppendEventRequest,
    ) -> crate::ApiFuture<'a, EventLogEntry> {
        Box::pin(async move {
            Ok(EventLogEntry::new(
                1,
                image_id.clone(),
                UserId::from("demo_user"),
                labello_domain::DatasetRole::Annotator,
                labello_domain::now(),
                request.payload,
            ))
        })
    }

    fn apply_annotation_batch<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        request: AnnotationBatchRequest,
    ) -> crate::ApiFuture<'a, ImageState> {
        Box::pin(async move {
            let mut state = ImageState::new(assignment.image_id.clone());
            for payload in request.payloads {
                let event = EventLogEntry::new(
                    state.current_sequence + 1,
                    assignment.image_id.clone(),
                    UserId::from("demo_user"),
                    labello_domain::DatasetRole::Annotator,
                    labello_domain::now(),
                    payload,
                );
                state
                    .apply_event(&event)
                    .map_err(|error| ClientError::Demo(error.to_string()))?;
            }
            Ok(state)
        })
    }
}

impl ReviewApi for DemoLabelloApi {
    fn record_review<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        review: ReviewRecord,
    ) -> crate::ApiFuture<'a, ImageState> {
        let _ = dataset_id;
        Box::pin(async move {
            let mut state = ImageState::new(image_id.clone());
            let event = EventLogEntry::new(
                1,
                image_id.clone(),
                UserId::from("demo_user"),
                labello_domain::DatasetRole::Reviewer,
                labello_domain::now(),
                EventPayload::ReviewRecorded { review },
            );
            state
                .apply_event(&event)
                .map_err(|error| ClientError::Demo(error.to_string()))?;
            Ok(state)
        })
    }

    fn record_correction<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _image_id: &'a ImageId,
        _request: CorrectionRequest,
    ) -> crate::ApiFuture<'a, EventLogEntry> {
        Box::pin(async move {
            Err(ClientError::Demo(
                "the demo backend does not create review assignments".to_string(),
            ))
        })
    }
}

impl AdjudicationApi for DemoLabelloApi {
    fn record_adjudication<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        adjudication: AdjudicationRecord,
    ) -> crate::ApiFuture<'a, EventLogEntry> {
        self.append_payload(
            dataset_id,
            image_id,
            EventPayload::AdjudicationRecorded { adjudication },
        )
    }
}

impl OfflineApi for DemoLabelloApi {
    fn offline_bundle<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        _request: OfflineBundleRequest,
    ) -> crate::ApiFuture<'a, OfflineBundle> {
        Box::pin(async move {
            let dataset = self.dataset(dataset_id)?;
            Ok(OfflineBundle {
                schema_version: labello_domain::SCHEMA_VERSION,
                dataset_id: dataset.dataset_id,
                user_id: UserId::from("demo_user"),
                created_at: labello_domain::now(),
                expires_at: None,
                roles: Vec::new(),
                tasks: dataset.tasks,
                images: Vec::new(),
                import_manifests: Vec::new(),
            })
        })
    }

    fn sync_offline_events<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _request: OfflineSyncRequest,
    ) -> crate::ApiFuture<'a, OfflineSyncResult> {
        Box::pin(async move {
            Ok(OfflineSyncResult {
                merged_events: 0,
                conflicts: Vec::new(),
            })
        })
    }
}
