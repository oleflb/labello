pub trait DatasetApi {
    fn list_datasets<'a>(&'a self) -> ApiFuture<'a, Vec<DatasetSummary>>;
    fn create_dataset<'a>(
        &'a self,
        request: CreateDatasetRequest,
    ) -> ApiFuture<'a, DatasetMetadata>;
    fn get_dataset<'a>(&'a self, dataset_id: &'a DatasetId) -> ApiFuture<'a, DatasetMetadata>;
    fn get_admin_dataset<'a>(&'a self, dataset_id: &'a DatasetId)
    -> ApiFuture<'a, DatasetMetadata>;
    fn update_dataset_config<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: UpdateDatasetConfigRequest,
    ) -> ApiFuture<'a, DatasetMetadata>;
    fn ingest_dataset<'a>(&'a self, dataset_id: &'a DatasetId) -> ApiFuture<'a, IngestReport>;
    fn start_ingest_job<'a>(&'a self, dataset_id: &'a DatasetId) -> ApiFuture<'a, IngestJob>;
    fn get_ingest_job<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        job_id: &'a str,
    ) -> ApiFuture<'a, IngestJob>;
    fn create_snapshot<'a>(&'a self, _dataset_id: &'a DatasetId) -> ApiFuture<'a, DatasetSnapshot> {
        Box::pin(async {
            Err(ClientError::Demo(
                "snapshots are not implemented by this client".to_string(),
            ))
        })
    }
    fn list_snapshots<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
    ) -> ApiFuture<'a, Vec<DatasetSnapshot>> {
        Box::pin(async {
            Err(ClientError::Demo(
                "snapshots are not implemented by this client".to_string(),
            ))
        })
    }
    fn get_snapshot_file<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _snapshot_id: &'a str,
        _path: &'a str,
    ) -> ApiFuture<'a, crate::SnapshotFile> {
        Box::pin(async {
            Err(ClientError::Demo(
                "snapshot downloads are not implemented by this client".to_string(),
            ))
        })
    }
}

pub trait ImportApi {
    fn import_capabilities<'a>(&'a self) -> ApiFuture<'a, crate::ImportCapabilities>;

    fn browse_server_import_root<'a>(
        &'a self,
        root_id: &'a str,
        request: crate::BrowseServerImportRootRequest,
    ) -> ApiFuture<'a, crate::ImportBrowsePage>;

    fn create_import<'a>(
        &'a self,
        request: crate::CreateImportRequest,
        idempotency_key: &'a str,
    ) -> ApiFuture<'a, crate::ImportJob>;

    fn get_import<'a>(&'a self, import_id: &'a ImportId) -> ApiFuture<'a, crate::ImportJob>;

    fn register_import_files<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: crate::RegisterImportFilesRequest,
        idempotency_key: &'a str,
    ) -> ApiFuture<'a, crate::RegisterImportFilesResult>;

    fn upload_import_chunk<'a>(
        &'a self,
        import_id: &'a ImportId,
        file_id: &'a str,
        upload: crate::ImportChunkUpload,
        idempotency_key: &'a str,
    ) -> ApiFuture<'a, crate::ImportChunkResult>;

    fn browse_import_source<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: crate::BrowseImportSourceRequest,
    ) -> ApiFuture<'a, crate::ImportBrowsePage>;

    fn inspect_yolo_descriptor<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: crate::InspectYoloDescriptorRequest,
    ) -> ApiFuture<'a, crate::YoloDescriptorInspection>;

    fn seal_import<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: crate::SealImportRequest,
        idempotency_key: &'a str,
    ) -> ApiFuture<'a, crate::SealImportResult>;

    fn preflight_import<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: crate::StartImportPreflightRequest,
        idempotency_key: &'a str,
    ) -> ApiFuture<'a, crate::ImportJob>;

    fn update_import_plan<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: crate::UpdateImportPlanRequest,
        idempotency_key: &'a str,
    ) -> ApiFuture<'a, crate::ImportPlan>;

    fn import_diagnostics<'a>(
        &'a self,
        import_id: &'a ImportId,
        query: crate::ImportDiagnosticsQuery,
    ) -> ApiFuture<'a, crate::ImportDiagnosticsPage>;

    fn commit_import<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: crate::CommitImportRequest,
        idempotency_key: &'a str,
    ) -> ApiFuture<'a, crate::CommitImportResult>;

    fn cancel_import<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: crate::CancelImportRequest,
        idempotency_key: &'a str,
    ) -> ApiFuture<'a, crate::CancelImportResult>;

    fn save_migration_skeleton<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: crate::SaveMigrationSkeletonRequest,
        idempotency_key: &'a str,
    ) -> ApiFuture<'a, crate::ManualMigrationCommandResult>;

    fn add_migration_skeleton<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: crate::AddMigrationSkeletonRequest,
        idempotency_key: &'a str,
    ) -> ApiFuture<'a, crate::ManualMigrationCommandResult>;

    fn exclude_migration_target<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: crate::ExcludeMigrationTargetRequest,
        idempotency_key: &'a str,
    ) -> ApiFuture<'a, crate::ManualMigrationCommandResult>;

    fn reopen_migration_target<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: crate::ReopenMigrationTargetRequest,
        idempotency_key: &'a str,
    ) -> ApiFuture<'a, crate::ManualMigrationCommandResult>;

    fn revisit_migration_target<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: crate::RevisitMigrationTargetRequest,
        idempotency_key: &'a str,
    ) -> ApiFuture<'a, crate::ManualMigrationCommandResult>;

    fn start_migration_pass<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: crate::StartMigrationPassRequest,
        idempotency_key: &'a str,
    ) -> ApiFuture<'a, crate::ManualMigrationCommandResult>;

    fn keep_migration_target<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: crate::KeepMigrationTargetRequest,
        idempotency_key: &'a str,
    ) -> ApiFuture<'a, crate::ManualMigrationCommandResult>;

    fn confirm_migration<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: crate::ConfirmMigrationRequest,
        idempotency_key: &'a str,
    ) -> ApiFuture<'a, crate::ManualMigrationCommandResult>;

    fn review_migration<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: crate::ReviewMigrationRequest,
        idempotency_key: &'a str,
    ) -> ApiFuture<'a, crate::ManualMigrationCommandResult>;
}
