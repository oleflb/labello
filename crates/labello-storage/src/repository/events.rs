use super::*;

impl DatasetRepository {
    pub async fn load_events(&self, image_id: &ImageId) -> StorageResult<Vec<EventLogEntry>> {
        let path = self.events_path(image_id);
        if !tokio::fs::try_exists(&path).await.with_path(&path)? {
            return Ok(Vec::new());
        }
        let text = tokio::fs::read_to_string(&path).await.with_path(&path)?;
        let events = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).with_json_path(&path))
            .collect::<StorageResult<Vec<EventLogEntry>>>()?;
        for event in &events {
            labello_domain::validate_supported_schema_version(event.schema_version)?;
        }
        Ok(events)
    }

    pub async fn rebuild_image_state(&self, image_id: &ImageId) -> StorageResult<ImageState> {
        let events = self.load_events(image_id).await?;
        let state = rebuild_state(image_id.clone(), &events)?;
        write_json_atomic(&self.state_path(image_id), &state).await?;
        if events.iter().any(stats_relevant_event) {
            self.stats_cache.invalidate();
        }
        self.assignment_availability_cache.invalidate();
        Ok(state)
    }

    pub async fn load_image_state(&self, image_id: &ImageId) -> StorageResult<ImageState> {
        #[cfg(test)]
        self.image_state_loads.fetch_add(1, Ordering::Relaxed);
        self.ensure_artifact_migration().await?;
        let path = self.state_path(image_id);
        let cache_exists = tokio::fs::try_exists(&path).await.with_path(&path)?;
        let cached = if cache_exists {
            let schema_version = read_schema_version(&path).await?;
            labello_domain::validate_supported_schema_version(schema_version)?;
            if schema_version == SCHEMA_VERSION {
                Some(read_current_json::<ImageState>(&path).await?)
            } else {
                None
            }
        } else {
            None
        };
        let events = self.load_events(image_id).await?;
        let event_sequence = events
            .last()
            .map(|event| event.event_sequence)
            .unwrap_or_default();
        if let Some(state) = cached.as_ref()
            && state.image_id == *image_id
            && state.current_sequence == event_sequence
        {
            return Ok(state.clone());
        }
        let state = rebuild_state(image_id.clone(), &events)?;
        if cache_exists || !events.is_empty() {
            tracing::warn!(
                event = "image_state.cache.rebuilt",
                image_id = %image_id,
                cached = cached.is_some(),
                event_sequence,
                "image state cache rebuilt from events"
            );
            write_json_atomic(&path, &state).await?;
        }
        Ok(state)
    }

    #[cfg(test)]
    pub(crate) fn reset_image_state_load_count(&self) {
        self.image_state_loads.store(0, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) fn image_state_load_count(&self) -> u64 {
        self.image_state_loads.load(Ordering::Relaxed)
    }

    pub(crate) async fn append_events_atomic(
        &self,
        image_id: &ImageId,
        events: &[EventLogEntry],
    ) -> StorageResult<()> {
        if events.is_empty() {
            return Ok(());
        }
        let path = self.events_path(image_id);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.with_path(parent)?;
        }
        let temporary = path.with_extension(format!("tmp-{}", uuid::Uuid::now_v7().simple()));
        let existing = if tokio::fs::try_exists(&path).await.with_path(&path)? {
            tokio::fs::read(&path).await.with_path(&path)?
        } else {
            Vec::new()
        };
        let mut file = tokio::fs::File::create(&temporary)
            .await
            .with_path(&temporary)?;
        file.write_all(&existing).await.with_path(&temporary)?;
        if !existing.is_empty() && !existing.ends_with(b"\n") {
            file.write_all(b"\n").await.with_path(&temporary)?;
        }
        for event in events {
            let line = serde_json::to_string(event).with_json_path(&path)?;
            file.write_all(line.as_bytes())
                .await
                .with_path(&temporary)?;
            file.write_all(b"\n").await.with_path(&temporary)?;
        }
        file.sync_all().await.with_path(&temporary)?;
        drop(file);
        tokio::fs::rename(&temporary, &path)
            .await
            .with_path(&path)?;
        if let Some(parent) = path.parent() {
            tokio::fs::File::open(parent)
                .await
                .with_path(parent)?
                .sync_all()
                .await
                .with_path(parent)?;
        }
        Ok(())
    }
}

pub(crate) fn stats_relevant_event(event: &EventLogEntry) -> bool {
    !matches!(&event.payload, EventPayload::AssignmentUpdated { .. })
}
