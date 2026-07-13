use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
};

use labello_domain::{
    ClassStats, DatasetStats, ReviewDecision, ReviewTarget, TaskStats, TaskStatus,
};

use tokio::sync::Mutex;

use crate::{DatasetRepository, StorageError, StorageResult};

#[derive(Debug, Default)]
pub(crate) struct StatsCache {
    generation: AtomicU64,
    value: Mutex<Option<(u64, DatasetStats)>>,
    #[cfg(test)]
    scans: AtomicU64,
}

impl StatsCache {
    pub(crate) fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }
}

impl DatasetRepository {
    pub async fn dataset_stats(&self) -> StorageResult<DatasetStats> {
        let mut cached = self.stats_cache.value.lock().await;

        loop {
            let generation = self.stats_cache.generation.load(Ordering::Acquire);
            if let Some((cached_generation, stats)) = cached.as_ref()
                && *cached_generation == generation
            {
                return Ok(stats.clone());
            }

            #[cfg(test)]
            self.stats_cache.scans.fetch_add(1, Ordering::Relaxed);

            let repository = self.clone();
            let runtime = tokio::runtime::Handle::current();
            let stats = tokio::task::spawn_blocking(move || {
                runtime.block_on(repository.compute_dataset_stats())
            })
            .await
            .map_err(|error| StorageError::BackgroundTask(error.to_string()))??;

            if self.stats_cache.generation.load(Ordering::Acquire) == generation {
                *cached = Some((generation, stats.clone()));
                return Ok(stats);
            }
        }
    }

    async fn compute_dataset_stats(&self) -> StorageResult<DatasetStats> {
        let metadata = self.load_dataset().await?;
        let mut stats = DatasetStats {
            total_images: metadata.images.len(),
            per_task: metadata
                .tasks
                .iter()
                .filter(|task| task.enabled)
                .map(|task| (task.task_id.clone(), TaskStats::default()))
                .collect(),
            per_class: metadata
                .label_classes
                .iter()
                .map(|class| (class.class_id.clone(), ClassStats::default()))
                .collect(),
            ..DatasetStats::default()
        };
        let mut throughput: BTreeMap<String, (usize, usize)> = BTreeMap::new();

        for image_id in metadata.images.keys() {
            let state = self.load_image_state(image_id).await?;
            for task in &metadata.tasks {
                if !task.enabled {
                    continue;
                }
                let task_stats = stats.per_task.entry(task.task_id.clone()).or_default();
                match state
                    .task_states
                    .get(&task.task_id)
                    .map(|state| &state.status)
                    .unwrap_or(&TaskStatus::Pending)
                {
                    TaskStatus::Completed => {
                        stats.completed_tasks += 1;
                        task_stats.completed += 1;
                        for class_id in &task.class_ids {
                            stats
                                .per_class
                                .entry(class_id.clone())
                                .or_default()
                                .completed_tasks += 1;
                        }
                    }
                    TaskStatus::Submitted => {
                        stats.unreviewed_tasks += 1;
                        task_stats.unreviewed += 1;
                    }
                    _ => {
                        stats.pending_tasks += 1;
                        task_stats.pending += 1;
                    }
                }
                let reviewed = state.reviews.iter().any(|review| {
                    review.decision == ReviewDecision::Approved
                        && matches!(
                            &review.target,
                            ReviewTarget::Task { task_id } if task_id == &task.task_id
                        )
                });
                if reviewed {
                    stats.reviewed_tasks += 1;
                    task_stats.reviewed += 1;
                }
            }
            for annotation in state.active_annotations() {
                stats
                    .per_class
                    .entry(annotation.class_id.clone())
                    .or_default()
                    .annotations += 1;
                let day = annotation.created_at.date_naive().to_string();
                throughput.entry(day).or_default().0 += 1;
            }
            for review in &state.reviews {
                let day = review.timestamp.date_naive().to_string();
                throughput.entry(day).or_default().1 += 1;
            }
        }

        stats.throughput = throughput
            .into_iter()
            .map(
                |(day, (annotations, reviews))| labello_domain::ThroughputPoint {
                    day,
                    annotations,
                    reviews,
                },
            )
            .collect();
        Ok(stats)
    }

    #[cfg(test)]
    fn stats_scan_count(&self) -> u64 {
        self.stats_cache.scans.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use labello_domain::{
        Actor, AnnotationGeometry, AnnotationId, AnnotationSource, AnnotationType, BoundingBox,
        ClassId, DatasetId, DatasetMetadata, DatasetRole, EventPayload, ImageId, ImageRecord,
        ImagesIndex, SCHEMA_VERSION, TaskId, UserId, now,
    };

    use super::*;

    async fn empty_repository() -> (tempfile::TempDir, DatasetRepository) {
        let temp = tempfile::tempdir().unwrap();
        let repository = DatasetRepository::new(temp.path());
        repository
            .initialize(DatasetMetadata::new(
                DatasetId::from("ds"),
                "Dataset",
                now(),
            ))
            .await
            .unwrap();
        (temp, repository)
    }

    #[tokio::test]
    async fn concurrent_and_repeated_requests_share_one_scan() {
        let (_temp, repository) = empty_repository().await;

        let (first, second, third) = tokio::join!(
            repository.dataset_stats(),
            repository.dataset_stats(),
            repository.dataset_stats()
        );

        assert_eq!(first.unwrap(), DatasetStats::default());
        assert_eq!(second.unwrap(), DatasetStats::default());
        assert_eq!(third.unwrap(), DatasetStats::default());
        assert_eq!(repository.stats_scan_count(), 1);

        repository.dataset_stats().await.unwrap();
        assert_eq!(repository.stats_scan_count(), 1);
    }

    #[tokio::test]
    async fn index_and_image_state_writes_invalidate_cached_stats() {
        let (_temp, repository) = empty_repository().await;
        assert_eq!(repository.dataset_stats().await.unwrap().total_images, 0);

        let image = ImageRecord {
            image_id: ImageId::from("img_1"),
            blake3: "hash".to_string(),
            canonical_path: "images/one.png".to_string(),
            known_paths: vec!["images/one.png".to_string()],
            duplicate_paths: Vec::new(),
            file_name: "one.png".to_string(),
            byte_size: 4,
            width: 2,
            height: 2,
            media_type: "image/png".to_string(),
        };
        repository
            .save_images_index(&ImagesIndex {
                schema_version: SCHEMA_VERSION,
                image_count: 1,
                images_by_hash: BTreeMap::from([("hash".to_string(), image)]),
            })
            .await
            .unwrap();

        assert_eq!(repository.dataset_stats().await.unwrap().total_images, 1);
        assert_eq!(repository.stats_scan_count(), 2);

        let user_id = UserId::from("annotator");
        repository
            .append_payload(
                &ImageId::from("img_1"),
                &Actor {
                    user_id: user_id.clone(),
                    role: DatasetRole::Annotator,
                },
                EventPayload::AnnotationVersionCreated {
                    annotation: labello_domain::AnnotationVersion {
                        annotation_id: AnnotationId::from("ann_1"),
                        version: 1,
                        task_id: TaskId::from("boxes"),
                        class_id: ClassId::from("person"),
                        annotation_type: AnnotationType::BoundingBox,
                        source: AnnotationSource::Human,
                        geometry: AnnotationGeometry::BoundingBox(BoundingBox {
                            x: 0.1,
                            y: 0.1,
                            width: 0.2,
                            height: 0.2,
                        }),
                        author_user_id: user_id,
                        created_at: now(),
                        updated_at: now(),
                        deleted: false,
                    },
                    previous_version: None,
                    reason: None,
                },
            )
            .await
            .unwrap();

        let stats = repository.dataset_stats().await.unwrap();
        assert_eq!(stats.per_class[&ClassId::from("person")].annotations, 1);
        assert_eq!(repository.stats_scan_count(), 3);
    }
}
