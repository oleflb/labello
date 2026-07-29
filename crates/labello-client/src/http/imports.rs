impl ImportApi for HttpLabelloApi {
    fn import_capabilities<'a>(&'a self) -> crate::ApiFuture<'a, crate::ImportCapabilities> {
        Box::pin(async move {
            Self::json(
                self.request(Method::GET, "/import-capabilities")?
                    .send()
                    .await?,
            )
            .await
        })
    }

    fn browse_server_import_root<'a>(
        &'a self,
        root_id: &'a str,
        request: crate::BrowseServerImportRootRequest,
    ) -> crate::ApiFuture<'a, crate::ImportBrowsePage> {
        Box::pin(async move {
            let root_id = urlencoding::encode(root_id);
            Self::send_json(
                self.request(Method::POST, &format!("/import-roots/{root_id}/browse"))?,
                &request,
            )
            .await
        })
    }

    fn create_import<'a>(
        &'a self,
        request: crate::CreateImportRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ImportJob> {
        Box::pin(async move {
            Self::send_json(
                Self::idempotent(self.request(Method::POST, "/imports")?, idempotency_key),
                &request,
            )
            .await
        })
    }

    fn get_import<'a>(&'a self, import_id: &'a ImportId) -> crate::ApiFuture<'a, crate::ImportJob> {
        Box::pin(async move {
            Self::json(
                self.request(Method::GET, &format!("/imports/{import_id}"))?
                    .send()
                    .await?,
            )
            .await
        })
    }

    fn register_import_files<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: crate::RegisterImportFilesRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::RegisterImportFilesResult> {
        Box::pin(async move {
            Self::send_json(
                Self::idempotent(
                    self.request(
                        Method::POST,
                        &format!("/imports/{import_id}/files/register"),
                    )?,
                    idempotency_key,
                ),
                &request,
            )
            .await
        })
    }

    fn upload_import_chunk<'a>(
        &'a self,
        import_id: &'a ImportId,
        file_id: &'a str,
        upload: crate::ImportChunkUpload,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ImportChunkResult> {
        Box::pin(async move {
            let file_id = urlencoding::encode(file_id);
            let request = self.request(
                Method::POST,
                &format!("/imports/{import_id}/files/{file_id}/chunks"),
            )?;
            let response = Self::idempotent(request, idempotency_key)
                .header(CONTENT_TYPE, "application/octet-stream")
                .header(UPLOAD_OFFSET_HEADER, upload.offset)
                .header(UPLOAD_LENGTH_HEADER, upload.length)
                .header(DIGEST_HEADER, upload.digest)
                .body(upload.bytes)
                .send()
                .await?;
            Self::json(response).await
        })
    }

    fn browse_import_source<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: crate::BrowseImportSourceRequest,
    ) -> crate::ApiFuture<'a, crate::ImportBrowsePage> {
        Box::pin(async move {
            Self::send_json(
                self.request(Method::POST, &format!("/imports/{import_id}/source/browse"))?,
                &request,
            )
            .await
        })
    }

    fn inspect_yolo_descriptor<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: crate::InspectYoloDescriptorRequest,
    ) -> crate::ApiFuture<'a, crate::YoloDescriptorInspection> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!("/imports/{import_id}/yolo-descriptor/inspect"),
                )?,
                &request,
            )
            .await
        })
    }

    fn seal_import<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: crate::SealImportRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::SealImportResult> {
        Box::pin(async move {
            Self::send_json(
                Self::idempotent(
                    self.request(Method::POST, &format!("/imports/{import_id}/seal"))?,
                    idempotency_key,
                ),
                &request,
            )
            .await
        })
    }

    fn preflight_import<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: crate::StartImportPreflightRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ImportJob> {
        Box::pin(async move {
            Self::send_json(
                Self::idempotent(
                    self.request(Method::POST, &format!("/imports/{import_id}/preflight"))?,
                    idempotency_key,
                ),
                &request,
            )
            .await
        })
    }

    fn update_import_plan<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: crate::UpdateImportPlanRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ImportPlan> {
        Box::pin(async move {
            Self::send_json(
                Self::idempotent(
                    self.request(Method::PUT, &format!("/imports/{import_id}/plan"))?,
                    idempotency_key,
                ),
                &request,
            )
            .await
        })
    }

    fn import_diagnostics<'a>(
        &'a self,
        import_id: &'a ImportId,
        query: crate::ImportDiagnosticsQuery,
    ) -> crate::ApiFuture<'a, crate::ImportDiagnosticsPage> {
        Box::pin(async move {
            let response = self
                .request(Method::GET, &format!("/imports/{import_id}/diagnostics"))?
                .query(&query)
                .send()
                .await?;
            Self::json(response).await
        })
    }

    fn commit_import<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: crate::CommitImportRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::CommitImportResult> {
        Box::pin(async move {
            Self::send_json(
                Self::idempotent(
                    self.request(Method::POST, &format!("/imports/{import_id}/commit"))?,
                    idempotency_key,
                ),
                &request,
            )
            .await
        })
    }

    fn cancel_import<'a>(
        &'a self,
        import_id: &'a ImportId,
        request: crate::CancelImportRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::CancelImportResult> {
        Box::pin(async move {
            Self::send_json(
                Self::idempotent(
                    self.request(Method::POST, &format!("/imports/{import_id}/cancel"))?,
                    idempotency_key,
                ),
                &request,
            )
            .await
        })
    }

    fn save_migration_skeleton<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: crate::SaveMigrationSkeletonRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult> {
        self.migration_json(dataset_id, image_id, "skeleton", request, idempotency_key)
    }

    fn add_migration_skeleton<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: crate::AddMigrationSkeletonRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult> {
        self.migration_json(
            dataset_id,
            image_id,
            "skeletons",
            request,
            idempotency_key,
        )
    }

    fn exclude_migration_target<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: crate::ExcludeMigrationTargetRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult> {
        self.migration_json(dataset_id, image_id, "exclude", request, idempotency_key)
    }

    fn reopen_migration_target<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: crate::ReopenMigrationTargetRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult> {
        self.migration_json(dataset_id, image_id, "reopen", request, idempotency_key)
    }

    fn revisit_migration_target<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: crate::RevisitMigrationTargetRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult> {
        self.migration_json(dataset_id, image_id, "revisit", request, idempotency_key)
    }

    fn start_migration_pass<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: crate::StartMigrationPassRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult> {
        self.migration_json(dataset_id, image_id, "passes", request, idempotency_key)
    }

    fn keep_migration_target<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: crate::KeepMigrationTargetRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult> {
        self.migration_json(dataset_id, image_id, "keep", request, idempotency_key)
    }

    fn confirm_migration<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: crate::ConfirmMigrationRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult> {
        self.migration_json(dataset_id, image_id, "confirm", request, idempotency_key)
    }

    fn review_migration<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: crate::ReviewMigrationRequest,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult> {
        self.migration_json(dataset_id, image_id, "review", request, idempotency_key)
    }
}

impl HttpLabelloApi {
    fn migration_json<'a, B>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        command: &'a str,
        body: B,
        idempotency_key: &'a str,
    ) -> crate::ApiFuture<'a, crate::ManualMigrationCommandResult>
    where
        B: Serialize + 'a,
    {
        Box::pin(async move {
            Self::send_json(
                Self::idempotent(
                    self.request(
                        Method::POST,
                        &format!("/datasets/{dataset_id}/images/{image_id}/migration/{command}"),
                    )?,
                    idempotency_key,
                ),
                &body,
            )
            .await
        })
    }
}
