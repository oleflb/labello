impl ImportApi for DemoLabelloApi {
    fn import_capabilities<'a>(&'a self) -> crate::ApiFuture<'a, crate::ImportCapabilities> {
        Box::pin(async {
            Ok(crate::ImportCapabilities {
                available: false,
                unavailable_reason: Some(
                    "dataset import is unavailable in the demo backend".to_string(),
                ),
                ..Default::default()
            })
        })
    }

    fn browse_server_import_root<'a>(
        &'a self,
        _root_id: &'a str,
        _request: crate::BrowseServerImportRootRequest,
    ) -> crate::ApiFuture<'a, crate::ImportBrowsePage> {
        import_unavailable()
    }

    fn create_import<'a>(
        &'a self,
        _request: crate::CreateImportRequest,
        _idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ImportJob> {
        import_unavailable()
    }

    fn get_import<'a>(
        &'a self,
        _import_id: &'a ImportId,
    ) -> crate::ApiFuture<'a, crate::ImportJob> {
        import_unavailable()
    }

    fn register_import_files<'a>(
        &'a self,
        _import_id: &'a ImportId,
        _request: crate::RegisterImportFilesRequest,
        _idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::RegisterImportFilesResult> {
        import_unavailable()
    }

    fn upload_import_chunk<'a>(
        &'a self,
        _import_id: &'a ImportId,
        _file_id: &'a str,
        _upload: crate::ImportChunkUpload,
        _idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ImportChunkResult> {
        import_unavailable()
    }

    fn browse_import_source<'a>(
        &'a self,
        _import_id: &'a ImportId,
        _request: crate::BrowseImportSourceRequest,
    ) -> crate::ApiFuture<'a, crate::ImportBrowsePage> {
        import_unavailable()
    }

    fn inspect_yolo_descriptor<'a>(
        &'a self,
        _import_id: &'a ImportId,
        _request: crate::InspectYoloDescriptorRequest,
    ) -> crate::ApiFuture<'a, crate::YoloDescriptorInspection> {
        import_unavailable()
    }

    fn seal_import<'a>(
        &'a self,
        _import_id: &'a ImportId,
        _request: crate::SealImportRequest,
        _idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::SealImportResult> {
        import_unavailable()
    }

    fn preflight_import<'a>(
        &'a self,
        _import_id: &'a ImportId,
        _request: crate::StartImportPreflightRequest,
        _idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ImportJob> {
        import_unavailable()
    }

    fn update_import_plan<'a>(
        &'a self,
        _import_id: &'a ImportId,
        _request: crate::UpdateImportPlanRequest,
        _idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ImportPlan> {
        import_unavailable()
    }

    fn import_diagnostics<'a>(
        &'a self,
        _import_id: &'a ImportId,
        _query: crate::ImportDiagnosticsQuery,
    ) -> crate::ApiFuture<'a, crate::ImportDiagnosticsPage> {
        import_unavailable()
    }

    fn commit_import<'a>(
        &'a self,
        _import_id: &'a ImportId,
        _request: crate::CommitImportRequest,
        _idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::CommitImportResult> {
        import_unavailable()
    }

    fn cancel_import<'a>(
        &'a self,
        _import_id: &'a ImportId,
        _request: crate::CancelImportRequest,
        _idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::CancelImportResult> {
        import_unavailable()
    }

    fn save_migration_skeleton<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _image_id: &'a ImageId,
        _request: crate::SaveMigrationSkeletonRequest,
        _idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult> {
        import_unavailable()
    }

    fn add_migration_skeleton<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _image_id: &'a ImageId,
        _request: crate::AddMigrationSkeletonRequest,
        _idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult> {
        import_unavailable()
    }

    fn exclude_migration_target<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _image_id: &'a ImageId,
        _request: crate::ExcludeMigrationTargetRequest,
        _idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult> {
        import_unavailable()
    }

    fn reopen_migration_target<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _image_id: &'a ImageId,
        _request: crate::ReopenMigrationTargetRequest,
        _idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult> {
        import_unavailable()
    }

    fn revisit_migration_target<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _image_id: &'a ImageId,
        _request: crate::RevisitMigrationTargetRequest,
        _idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult> {
        import_unavailable()
    }

    fn start_migration_pass<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _image_id: &'a ImageId,
        _request: crate::StartMigrationPassRequest,
        _idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult> {
        import_unavailable()
    }

    fn keep_migration_target<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _image_id: &'a ImageId,
        _request: crate::KeepMigrationTargetRequest,
        _idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult> {
        import_unavailable()
    }

    fn confirm_migration<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _image_id: &'a ImageId,
        _request: crate::ConfirmMigrationRequest,
        _idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult> {
        import_unavailable()
    }

    fn review_migration<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _image_id: &'a ImageId,
        _request: crate::ReviewMigrationRequest,
        _idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult> {
        import_unavailable()
    }
}

fn import_unavailable<'a, T>() -> crate::ApiFuture<'a, T> {
    Box::pin(async {
        Err(ClientError::Demo(
            "dataset import is unavailable in the demo backend".to_string(),
        ))
    })
}
