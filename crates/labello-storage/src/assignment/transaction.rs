use super::*;

use labello_domain::ImageState;

use crate::{fsjson::write_json_atomic, repository::stats_relevant_event};

impl DatasetRepository {
    pub async fn append_payload(
        &self,
        image_id: &ImageId,
        actor: &Actor,
        payload: EventPayload,
    ) -> StorageResult<EventLogEntry> {
        let lock = self.image_lock(image_id);
        let _guard = lock.lock().await;
        self.append_payloads_unlocked(image_id, actor, vec![payload])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| StorageError::Unauthorized("no payload was appended".to_string()))
    }

    pub(crate) async fn append_payloads_unlocked(
        &self,
        image_id: &ImageId,
        actor: &Actor,
        payloads: Vec<EventPayload>,
    ) -> StorageResult<Vec<EventLogEntry>> {
        let (events, _) = self
            .append_payloads_with_state_unlocked(image_id, actor, payloads)
            .await?;
        Ok(events)
    }

    pub(crate) async fn append_payloads_with_state_unlocked(
        &self,
        image_id: &ImageId,
        actor: &Actor,
        mut payloads: Vec<EventPayload>,
    ) -> StorageResult<(Vec<EventLogEntry>, ImageState)> {
        // 1. Load the replay-validated cache base from the authoritative event log.
        let mut next_state = self.load_image_state(image_id).await?;
        let timestamp = labello_domain::now();
        // 2. Let assignment/migration policy finish the complete event batch.
        crate::assignment::append_guide_invalidation_payloads(
            &next_state,
            &mut payloads,
            timestamp,
        );
        // 3. Validate the entire planned batch against a cloned next state.
        let mut events = Vec::with_capacity(payloads.len());
        for payload in payloads {
            let event = EventLogEntry::new(
                next_state.current_sequence + 1,
                image_id.clone(),
                actor.user_id.clone(),
                actor.role.clone(),
                timestamp,
                payload,
            );
            next_state.apply_event(&event)?;
            events.push(event);
        }
        // 4. Atomically publish events.jsonl, the authoritative state transition.
        self.append_events_atomic(image_id, &events).await?;
        // 5. Publish state.json only after the event log; it remains rebuildable.
        write_json_atomic(&self.state_path(image_id), &next_state).await?;
        // 6. Invalidate derived process-local caches after durable publication.
        if events.iter().any(stats_relevant_event) {
            self.stats_cache.invalidate();
        }
        self.assignment_availability_cache.invalidate();
        Ok((events, next_state))
    }
}
