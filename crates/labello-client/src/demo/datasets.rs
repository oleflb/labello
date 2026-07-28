impl DatasetApi for DemoLabelloApi {
    fn list_datasets<'a>(&'a self) -> crate::ApiFuture<'a, Vec<DatasetSummary>> {
        Box::pin(async move {
            Ok(self
                .state
                .borrow()
                .datasets
                .values()
                .map(|metadata| DatasetSummary {
                    dataset_id: metadata.dataset_id.clone(),
                    name: metadata.name.clone(),
                    roles: Vec::new(),
                    total_images: metadata.images.len(),
                })
                .collect())
        })
    }

    fn create_dataset<'a>(
        &'a self,
        request: CreateDatasetRequest,
    ) -> crate::ApiFuture<'a, DatasetMetadata> {
        Box::pin(async move {
            let metadata = DatasetMetadata::new(
                request.dataset_id.clone(),
                request.name,
                labello_domain::now(),
            );
            self.state
                .borrow_mut()
                .datasets
                .insert(request.dataset_id, metadata.clone());
            Ok(metadata)
        })
    }

    fn get_dataset<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, DatasetMetadata> {
        Box::pin(async move { self.dataset(dataset_id) })
    }

    fn get_admin_dataset<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, DatasetMetadata> {
        self.get_dataset(dataset_id)
    }

    fn update_dataset_config<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: UpdateDatasetConfigRequest,
    ) -> crate::ApiFuture<'a, DatasetMetadata> {
        Box::pin(async move {
            let mut state = self.state.borrow_mut();
            let metadata = state
                .datasets
                .get_mut(dataset_id)
                .ok_or_else(|| ClientError::Demo(format!("dataset {dataset_id} does not exist")))?;
            metadata.name = request.name;
            metadata.image_roots = request.image_roots;
            metadata.label_classes = request.label_classes;
            metadata.tasks = request.tasks;
            metadata.role_assignments = request.role_assignments;
            metadata.imbalance = request.imbalance;
            metadata.prelabel_configs = request.prelabel_configs;
            metadata.updated_at = labello_domain::now();
            Ok(metadata.clone())
        })
    }

    fn ingest_dataset<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, IngestReport> {
        Box::pin(async move { Ok(IngestReport::default()) })
    }

    fn start_ingest_job<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, IngestJob> {
        Box::pin(async move {
            Ok(IngestJob {
                job_id: "demo-ingest".to_string(),
                dataset_id: dataset_id.clone(),
                status: IngestJobStatus::Completed,
                report: Some(IngestReport::default()),
                error: None,
            })
        })
    }

    fn get_ingest_job<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        job_id: &'a str,
    ) -> crate::ApiFuture<'a, IngestJob> {
        Box::pin(async move {
            Ok(IngestJob {
                job_id: job_id.to_string(),
                dataset_id: dataset_id.clone(),
                status: IngestJobStatus::Completed,
                report: Some(IngestReport::default()),
                error: None,
            })
        })
    }
}
