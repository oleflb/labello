use super::*;

impl DatasetRepository {
    pub(super) async fn compute_dataset_stats(&self) -> StorageResult<DatasetStats> {
        let metadata = self.load_dataset().await?;
        let mut aggregation = StatsAggregation::new(&metadata);
        for image_id in metadata.images.keys() {
            let state = self.load_image_state(image_id).await?;
            aggregation.record_image(&metadata, &state);
        }
        Ok(aggregation.finish())
    }
}
