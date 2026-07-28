impl DatasetApi for HttpLabelloApi {
    fn list_datasets<'a>(&'a self) -> crate::ApiFuture<'a, Vec<DatasetSummary>> {
        Box::pin(
            async move { Self::json(self.request(Method::GET, "/datasets")?.send().await?).await },
        )
    }

    fn create_dataset<'a>(
        &'a self,
        request: CreateDatasetRequest,
    ) -> crate::ApiFuture<'a, DatasetMetadata> {
        Box::pin(async move {
            Self::send_json(self.request(Method::POST, "/datasets")?, &request).await
        })
    }

    fn get_dataset<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, DatasetMetadata> {
        Box::pin(async move {
            Self::json(
                self.request(Method::GET, &format!("/datasets/{dataset_id}"))?
                    .send()
                    .await?,
            )
            .await
        })
    }

    fn get_admin_dataset<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, DatasetMetadata> {
        Box::pin(async move {
            Self::json(
                self.request(Method::GET, &format!("/datasets/{dataset_id}/admin"))?
                    .send()
                    .await?,
            )
            .await
        })
    }

    fn update_dataset_config<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: UpdateDatasetConfigRequest,
    ) -> crate::ApiFuture<'a, DatasetMetadata> {
        Box::pin(async move {
            Self::send_json(
                self.request(Method::PUT, &format!("/datasets/{dataset_id}/admin"))?,
                &request,
            )
            .await
        })
    }

    fn ingest_dataset<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, IngestReport> {
        Box::pin(async move {
            Self::json(
                self.request(Method::POST, &format!("/datasets/{dataset_id}/ingest"))?
                    .send()
                    .await?,
            )
            .await
        })
    }

    fn start_ingest_job<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, IngestJob> {
        Box::pin(async move {
            Self::json(
                self.request(Method::POST, &format!("/datasets/{dataset_id}/ingest-jobs"))?
                    .send()
                    .await?,
            )
            .await
        })
    }

    fn get_ingest_job<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        job_id: &'a str,
    ) -> crate::ApiFuture<'a, IngestJob> {
        Box::pin(async move {
            Self::json(
                self.request(
                    Method::GET,
                    &format!("/datasets/{dataset_id}/ingest-jobs/{job_id}"),
                )?
                .send()
                .await?,
            )
            .await
        })
    }

    fn create_snapshot<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, DatasetSnapshot> {
        Box::pin(async move {
            Self::json(
                self.request(Method::POST, &format!("/datasets/{dataset_id}/snapshots"))?
                    .send()
                    .await?,
            )
            .await
        })
    }

    fn list_snapshots<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, Vec<DatasetSnapshot>> {
        Box::pin(async move {
            Self::json(
                self.request(Method::GET, &format!("/datasets/{dataset_id}/snapshots"))?
                    .send()
                    .await?,
            )
            .await
        })
    }

    fn get_snapshot_file<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        snapshot_id: &'a str,
        path: &'a str,
    ) -> crate::ApiFuture<'a, crate::SnapshotFile> {
        Box::pin(async move {
            let encoded_path = path
                .split('/')
                .map(|part| urlencoding::encode(part))
                .collect::<Vec<_>>()
                .join("/");
            let response = self
                .request(
                    Method::GET,
                    &format!("/datasets/{dataset_id}/snapshots/{snapshot_id}/files/{encoded_path}"),
                )?
                .send()
                .await?;
            let response = Self::ensure_success(response).await?;
            let media_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("application/octet-stream")
                .to_string();
            Ok(crate::SnapshotFile {
                file_name: path.to_string(),
                media_type,
                bytes: response.bytes().await?.to_vec(),
            })
        })
    }
}
