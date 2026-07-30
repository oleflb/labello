pub trait TaskApi {
    fn list_tasks<'a>(&'a self, dataset_id: &'a DatasetId) -> ApiFuture<'a, Vec<TaskDefinition>>;
    fn add_task<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        task: TaskDefinition,
    ) -> ApiFuture<'a, TaskDefinition>;
}

pub trait ImageApi {
    fn assignment_availability<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: AssignmentAvailabilityRequest,
    ) -> ApiFuture<'a, AssignmentAvailability>;

    fn list_images<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _query: ImageExplorerQuery,
    ) -> ApiFuture<'a, ImageExplorerPage> {
        Box::pin(async {
            Err(ClientError::Demo(
                "image explorer is not implemented by this client".to_string(),
            ))
        })
    }
    fn assign_next_image<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: AssignNextRequest,
    ) -> ApiFuture<'a, Option<Assignment>>;
    fn revalidate_assignment<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _image_id: &'a ImageId,
        _request: AssignmentActionRequest,
    ) -> ApiFuture<'a, Option<AssignmentRevalidation>> {
        Box::pin(async {
            Err(ClientError::Demo(
                "assignment revalidation is not implemented by this client".to_string(),
            ))
        })
    }
    fn release_assignment<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _request: AssignmentActionRequest,
    ) -> ApiFuture<'a, Assignment> {
        Box::pin(async {
            Err(ClientError::Demo(
                "assignment release is not implemented by this client".to_string(),
            ))
        })
    }
    fn complete_assignment<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _request: AssignmentActionRequest,
    ) -> ApiFuture<'a, Assignment> {
        Box::pin(async {
            Err(ClientError::Demo(
                "assignment completion is not implemented by this client".to_string(),
            ))
        })
    }
    fn reopen_assignment<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _request: AssignmentActionRequest,
    ) -> ApiFuture<'a, Assignment> {
        Box::pin(async {
            Err(ClientError::Demo(
                "assignment reopen is not implemented by this client".to_string(),
            ))
        })
    }
    fn get_image_state<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> ApiFuture<'a, ImageState>;
    fn get_image_record<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> ApiFuture<'a, ImageRecord>;
    fn get_image_file<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> ApiFuture<'a, ImageFile>;
    fn get_image_preview<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        max_dimension: u32,
    ) -> ApiFuture<'a, ImagePreview>;
    fn rebuild_image<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> ApiFuture<'a, ImageState>;
}

pub trait AnnotationApi {
    fn append_event<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: AppendEventRequest,
    ) -> ApiFuture<'a, EventLogEntry>;

    fn append_payload<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        payload: EventPayload,
    ) -> ApiFuture<'a, EventLogEntry> {
        self.append_event(dataset_id, image_id, AppendEventRequest { payload })
    }

    fn append_assigned_event<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        request: AppendEventRequest,
    ) -> ApiFuture<'a, EventLogEntry> {
        Box::pin(async move {
            self.append_event(dataset_id, &assignment.image_id, request)
                .await
        })
    }

    fn apply_annotation_batch<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        request: AnnotationBatchRequest,
    ) -> ApiFuture<'a, ImageState>;
}

pub trait ReviewApi {
    fn record_review<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        review: ReviewRecord,
    ) -> ApiFuture<'a, ImageState>;

    fn record_assigned_review<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        review: ReviewRecord,
    ) -> ApiFuture<'a, ImageState> {
        Box::pin(async move {
            self.record_review(dataset_id, &assignment.image_id, review)
                .await
        })
    }

    fn record_correction<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: CorrectionRequest,
    ) -> ApiFuture<'a, EventLogEntry>;

    fn record_assigned_correction<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        request: CorrectionRequest,
    ) -> ApiFuture<'a, EventLogEntry> {
        Box::pin(async move {
            self.record_correction(dataset_id, &assignment.image_id, request)
                .await
        })
    }
}

pub trait AdjudicationApi {
    fn record_adjudication<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        adjudication: AdjudicationRecord,
    ) -> ApiFuture<'a, EventLogEntry>;

    fn record_assigned_adjudication<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        adjudication: AdjudicationRecord,
    ) -> ApiFuture<'a, EventLogEntry> {
        Box::pin(async move {
            self.record_adjudication(dataset_id, &assignment.image_id, adjudication)
                .await
        })
    }
}

pub trait OfflineApi {
    fn offline_bundle<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: OfflineBundleRequest,
    ) -> ApiFuture<'a, OfflineBundle>;

    fn sync_offline_events<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: OfflineSyncRequest,
    ) -> ApiFuture<'a, OfflineSyncResult>;
}
