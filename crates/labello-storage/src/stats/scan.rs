use super::*;

const MAX_STATS_SCAN_WORKERS: usize = 32;

impl DatasetRepository {
    pub(super) async fn compute_dataset_stats(&self) -> StorageResult<DatasetStats> {
        let metadata = self.load_dataset().await?;
        let mut aggregation = StatsAggregation::new(&metadata);

        let mut image_ids = metadata.images.keys().cloned();
        let mut workers = tokio::task::JoinSet::new();
        for image_id in image_ids.by_ref().take(MAX_STATS_SCAN_WORKERS) {
            let repository = self.clone();
            workers.spawn(async move { repository.load_image_state(&image_id).await });
        }

        while let Some(result) = workers.join_next().await {
            let state = result.map_err(|error| {
                crate::StorageError::BackgroundTask(format!(
                    "dataset statistics worker failed: {error}"
                ))
            })??;
            aggregation.record_image(&metadata, &state);
            if let Some(image_id) = image_ids.next() {
                let repository = self.clone();
                workers.spawn(async move { repository.load_image_state(&image_id).await });
            }
        }

        Ok(aggregation.finish())
    }
}
