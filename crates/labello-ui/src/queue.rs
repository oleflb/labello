use std::collections::VecDeque;

use labello_domain::{ImageRecord, PrelabelSuggestion};

#[derive(Clone, Debug)]
pub struct QueuedImage {
    pub image: ImageRecord,
    pub prelabels: Vec<PrelabelSuggestion>,
}

#[derive(Clone, Debug)]
pub struct ImageQueue {
    queue_size: usize,
    loading: bool,
    items: VecDeque<QueuedImage>,
}

impl ImageQueue {
    pub fn new(queue_size: usize) -> Self {
        Self {
            queue_size: queue_size.max(1),
            loading: false,
            items: VecDeque::new(),
        }
    }

    pub fn queue_size(&self) -> usize {
        self.queue_size
    }

    pub fn set_queue_size(&mut self, queue_size: usize) {
        self.queue_size = queue_size.max(1);
        while self.items.len() > self.queue_size {
            self.items.pop_back();
        }
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn push_if_room(&mut self, image: QueuedImage) -> bool {
        if self.items.len() < self.queue_size {
            self.items.push_back(image);
            true
        } else {
            false
        }
    }

    pub fn pop_next(&mut self) -> Option<QueuedImage> {
        self.items.pop_front()
    }

    pub fn clear(&mut self) {
        self.items.clear();
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
    }

    fn queued(id: &str) -> QueuedImage {
        QueuedImage {
            image: labello_domain::ImageRecord {
                image_id: ImageId::from(id),
                blake3: id.to_string(),
                canonical_path: format!("images/{id}.png"),
                known_paths: vec![],
                duplicate_paths: vec![],
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
