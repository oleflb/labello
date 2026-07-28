impl TaskApi for HttpLabelloApi {
    fn list_tasks<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, Vec<TaskDefinition>> {
        Box::pin(async move {
            Self::json(
                self.request(Method::GET, &format!("/datasets/{dataset_id}/tasks"))?
                    .send()
                    .await?,
            )
            .await
        })
    }

    fn add_task<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        task: TaskDefinition,
    ) -> crate::ApiFuture<'a, TaskDefinition> {
        Box::pin(async move {
            Self::send_json(
                self.request(Method::POST, &format!("/datasets/{dataset_id}/tasks"))?,
                &task,
            )
            .await
        })
    }
}

impl ImageApi for HttpLabelloApi {
    fn assignment_availability<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: crate::AssignmentAvailabilityRequest,
    ) -> crate::ApiFuture<'a, crate::AssignmentAvailability> {
        Box::pin(async move {
            let response = self
                .request(
                    Method::GET,
                    &format!("/datasets/{dataset_id}/assignments/availability"),
                )?
                .query(&request)
                .timeout(ASSIGNMENT_AVAILABILITY_REQUEST_TIMEOUT)
                .send()
                .await?;
            Self::json(response).await
        })
    }

    fn list_images<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        query: ImageExplorerQuery,
    ) -> crate::ApiFuture<'a, ImageExplorerPage> {
        Box::pin(async move {
            let response = self
                .request(Method::GET, &format!("/datasets/{dataset_id}/images"))?
                .query(&query)
                .send()
                .await?;
            Self::json(response).await
        })
    }

    fn assign_next_image<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: AssignNextRequest,
    ) -> crate::ApiFuture<'a, Option<Assignment>> {
        Box::pin(async move {
            Self::send_json(
                self.request(Method::POST, &format!("/datasets/{dataset_id}/images/next"))?,
                &request,
            )
            .await
        })
    }

    fn release_assignment<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: AssignmentActionRequest,
    ) -> crate::ApiFuture<'a, Assignment> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!("/datasets/{dataset_id}/assignments/release"),
                )?,
                &request,
            )
            .await
        })
    }

    fn complete_assignment<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: AssignmentActionRequest,
    ) -> crate::ApiFuture<'a, Assignment> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!("/datasets/{dataset_id}/assignments/complete"),
                )?,
                &request,
            )
            .await
        })
    }

    fn reopen_assignment<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: AssignmentActionRequest,
    ) -> crate::ApiFuture<'a, Assignment> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!("/datasets/{dataset_id}/assignments/reopen"),
                )?,
                &request,
            )
            .await
        })
    }

    fn get_image_state<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> crate::ApiFuture<'a, ImageState> {
        Box::pin(async move {
            Self::json(
                self.request(
                    Method::GET,
                    &format!("/datasets/{dataset_id}/images/{image_id}"),
                )?
                .send()
                .await?,
            )
            .await
        })
    }

    fn get_image_record<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> crate::ApiFuture<'a, ImageRecord> {
        Box::pin(async move {
            Self::json(
                self.request(
                    Method::GET,
                    &format!("/datasets/{dataset_id}/images/{image_id}/record"),
                )?
                .send()
                .await?,
            )
            .await
        })
    }

    fn get_image_file<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> crate::ApiFuture<'a, ImageFile> {
        Box::pin(async move {
            let response = self
                .request(
                    Method::GET,
                    &format!("/datasets/{dataset_id}/images/{image_id}/file"),
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
            Ok(ImageFile {
                image_id: image_id.clone(),
                media_type,
                bytes: response.bytes().await?.to_vec(),
            })
        })
    }

    fn get_image_preview<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        max_dimension: u32,
    ) -> crate::ApiFuture<'a, ImagePreview> {
        Box::pin(async move {
            let response = self
                .request(
                    Method::GET,
                    &format!(
                        "/datasets/{dataset_id}/images/{image_id}/preview?max={max_dimension}"
                    ),
                )?
                .send()
                .await?;
            let response = Self::ensure_success(response).await?;
            let width = preview_dimension(response.headers(), "x-image-width")?;
            let height = preview_dimension(response.headers(), "x-image-height")?;
            Ok(ImagePreview {
                image_id: image_id.clone(),
                width,
                height,
                rgba: response.bytes().await?.to_vec(),
            })
        })
    }

    fn rebuild_image<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> crate::ApiFuture<'a, ImageState> {
        Box::pin(async move {
            Self::json(
                self.request(
                    Method::POST,
                    &format!("/datasets/{dataset_id}/images/{image_id}/rebuild"),
                )?
                .send()
                .await?,
            )
            .await
        })
    }
}

fn preview_dimension(headers: &reqwest::header::HeaderMap, name: &str) -> ClientResult<u32> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| ClientError::Demo(format!("missing preview header {name}")))
}

impl AnnotationApi for HttpLabelloApi {
    fn append_event<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: AppendEventRequest,
    ) -> crate::ApiFuture<'a, EventLogEntry> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!("/datasets/{dataset_id}/images/{image_id}/events"),
                )?,
                &request,
            )
            .await
        })
    }

    fn append_assigned_event<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        request: AppendEventRequest,
    ) -> crate::ApiFuture<'a, EventLogEntry> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!(
                        "/datasets/{dataset_id}/images/{}/events",
                        assignment.image_id
                    ),
                )?
                .query(&assignment),
                &request,
            )
            .await
        })
    }

    fn apply_annotation_batch<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        request: AnnotationBatchRequest,
    ) -> crate::ApiFuture<'a, ImageState> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!(
                        "/datasets/{dataset_id}/images/{}/annotation-batch",
                        assignment.image_id
                    ),
                )?
                .query(&assignment),
                &request,
            )
            .await
        })
    }
}

impl ReviewApi for HttpLabelloApi {
    fn record_review<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        review: ReviewRecord,
    ) -> crate::ApiFuture<'a, ImageState> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!("/datasets/{dataset_id}/images/{image_id}/reviews"),
                )?,
                &review,
            )
            .await
        })
    }

    fn record_assigned_review<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        review: ReviewRecord,
    ) -> crate::ApiFuture<'a, ImageState> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!(
                        "/datasets/{dataset_id}/images/{}/reviews",
                        assignment.image_id
                    ),
                )?
                .query(&assignment),
                &review,
            )
            .await
        })
    }

    fn record_correction<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: CorrectionRequest,
    ) -> crate::ApiFuture<'a, EventLogEntry> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!("/datasets/{dataset_id}/images/{image_id}/corrections"),
                )?,
                &request,
            )
            .await
        })
    }

    fn record_assigned_correction<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        request: CorrectionRequest,
    ) -> crate::ApiFuture<'a, EventLogEntry> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!(
                        "/datasets/{dataset_id}/images/{}/corrections",
                        assignment.image_id
                    ),
                )?
                .query(&assignment),
                &request,
            )
            .await
        })
    }
}

impl AdjudicationApi for HttpLabelloApi {
    fn record_adjudication<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        adjudication: AdjudicationRecord,
    ) -> crate::ApiFuture<'a, EventLogEntry> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!("/datasets/{dataset_id}/images/{image_id}/adjudications"),
                )?,
                &adjudication,
            )
            .await
        })
    }

    fn record_assigned_adjudication<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        adjudication: AdjudicationRecord,
    ) -> crate::ApiFuture<'a, EventLogEntry> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!(
                        "/datasets/{dataset_id}/images/{}/adjudications",
                        assignment.image_id
                    ),
                )?
                .query(&assignment),
                &adjudication,
            )
            .await
        })
    }
}

impl OfflineApi for HttpLabelloApi {
    fn offline_bundle<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: OfflineBundleRequest,
    ) -> crate::ApiFuture<'a, OfflineBundle> {
        Box::pin(async move {
            let response = self
                .request(
                    Method::GET,
                    &format!("/datasets/{dataset_id}/offline-bundle"),
                )?
                .query(&request)
                .send()
                .await?;
            Self::versioned_json(response).await
        })
    }

    fn sync_offline_events<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: OfflineSyncRequest,
    ) -> crate::ApiFuture<'a, OfflineSyncResult> {
        Box::pin(async move {
            Self::send_json(
                self.request(
                    Method::POST,
                    &format!("/datasets/{dataset_id}/offline-sync"),
                )?,
                &request,
            )
            .await
        })
    }
}
