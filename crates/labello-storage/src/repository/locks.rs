use super::*;

impl DatasetRepository {
    pub(crate) fn image_lock(&self, image_id: &ImageId) -> Arc<AsyncMutex<()>> {
        self.locks
            .lock()
            .entry(image_id.clone())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }
}
