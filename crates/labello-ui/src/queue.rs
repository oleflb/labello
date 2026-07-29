use std::collections::VecDeque;

use labello_domain::{ImageRecord, PrelabelSuggestion};
use web_time::{Duration, Instant};

use crate::app::LoadedImage;

#[derive(Clone, Debug)]
pub struct QueuedImage {
    pub image: ImageRecord,
    pub prelabels: Vec<PrelabelSuggestion>,
}

#[derive(Clone, Debug)]
pub struct ImageQueue {
    queue_size: usize,
    loading: bool,
    failed_at: Option<Instant>,
    retry_delay: Duration,
    items: VecDeque<QueuedImage>,
    prepared: VecDeque<LoadedImage>,
}

impl ImageQueue {
    pub fn new(queue_size: usize) -> Self {
        Self {
            queue_size: queue_size.clamp(1, crate::app::IMAGE_QUEUE_SIZE),
            loading: false,
            failed_at: None,
            retry_delay: Duration::from_secs(1),
            items: VecDeque::new(),
            prepared: VecDeque::new(),
        }
    }

    pub fn queue_size(&self) -> usize {
        self.queue_size
    }

    pub fn set_queue_size(&mut self, queue_size: usize) {
        self.queue_size = queue_size.clamp(1, crate::app::IMAGE_QUEUE_SIZE);
        while self.len() > self.queue_size {
            if self.prepared.pop_back().is_some() {
                continue;
            }
            self.items.pop_back();
        }
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    pub(crate) fn mark_failed(&mut self) {
        self.mark_failed_after(Duration::from_secs(1));
    }

    pub(crate) fn mark_failed_after(&mut self, delay: Duration) {
        self.failed_at = Some(Instant::now());
        self.retry_delay = delay;
    }

    pub(crate) fn clear_failure(&mut self) {
        self.failed_at = None;
        self.retry_delay = Duration::from_secs(1);
    }

    pub(crate) fn retry_due(&self) -> bool {
        self.failed_at
            .is_some_and(|failed| failed.elapsed() >= self.retry_delay)
    }

    pub(crate) fn retry_after(&self) -> Option<Duration> {
        self.failed_at
            .map(|failed| self.retry_delay.saturating_sub(failed.elapsed()))
    }

    pub(crate) fn failed(&self) -> bool {
        self.failed_at.is_some()
    }

    pub fn len(&self) -> usize {
        self.items.len() + self.prepared.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.prepared.is_empty()
    }

    pub fn push_if_room(&mut self, image: QueuedImage) -> bool {
        if self.len() < self.queue_size {
            self.items.push_back(image);
            true
        } else {
            false
        }
    }

    pub fn pop_next(&mut self) -> Option<QueuedImage> {
        self.items.pop_front()
    }

    pub(crate) fn push_prepared(&mut self, image: LoadedImage) -> bool {
        if self.len() < self.queue_size {
            self.prepared.push_back(image);
            true
        } else {
            false
        }
    }

    pub(crate) fn pop_prepared(&mut self) -> Option<LoadedImage> {
        self.prepared.pop_front()
    }

    pub(crate) fn drain_prepared_assignments(&mut self) -> Vec<labello_domain::Assignment> {
        self.prepared
            .drain(..)
            .map(|loaded| loaded.assignment)
            .collect()
    }

    pub(crate) fn prepared_image_ids(&self) -> Vec<labello_domain::ImageId> {
        self.prepared
            .iter()
            .map(|loaded| loaded.assignment.image_id.clone())
            .collect()
    }

    pub(crate) fn remove_expired(&mut self) -> bool {
        let before = self.prepared.len();
        let now = labello_domain::now();
        self.prepared.retain(|loaded| {
            loaded
                .assignment
                .expires_at
                .is_none_or(|expires_at| expires_at > now)
        });
        before != self.prepared.len()
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.prepared.clear();
        self.failed_at = None;
        self.retry_delay = Duration::from_secs(1);
    }
}

#[cfg(test)]
mod tests {
    use labello_domain::ImageId;

    use super::*;

    #[test]
    fn keeps_configured_size() {
        let mut queue = ImageQueue::new(2);
        assert!(queue.push_if_room(queued("a")));
        assert!(queue.push_if_room(queued("b")));
        assert!(!queue.push_if_room(queued("c")));
        queue.set_queue_size(1);
        assert_eq!(queue.len(), 1);
        assert_eq!(ImageQueue::new(99).queue_size(), 2);
    }

    #[test]
    fn failed_refills_retry_after_a_short_delay() {
        let mut queue = ImageQueue::new(2);
        queue.mark_failed();
        assert!(queue.failed());
        assert!(!queue.retry_due());
        assert!(queue.retry_after().is_some());

        queue.failed_at = Some(Instant::now() - Duration::from_secs(1));
        assert!(queue.retry_due());
        queue.clear_failure();
        assert!(!queue.failed());
    }

    #[test]
    fn empty_refills_can_use_a_longer_retry_delay() {
        let mut queue = ImageQueue::new(2);
        queue.mark_failed_after(Duration::from_secs(15));
        assert!(!queue.retry_due());
        assert!(
            queue
                .retry_after()
                .is_some_and(|delay| delay > Duration::from_secs(14))
        );

        queue.failed_at = Some(Instant::now() - Duration::from_secs(15));
        assert!(queue.retry_due());
    }

    fn queued(id: &str) -> QueuedImage {
        QueuedImage {
            image: labello_domain::ImageRecord {
                image_id: ImageId::from(id),
                blake3: id.to_string(),
                canonical_path: format!("images/{id}.png"),
                known_paths: vec![],
                duplicate_paths: vec![],
                source_memberships: None,
                file_name: format!("{id}.png"),
                byte_size: 4,
                width: 10,
                height: 10,
                media_type: "image/png".to_string(),
            },
            prelabels: vec![],
        }
    }
}
