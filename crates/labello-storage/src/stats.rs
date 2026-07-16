use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
};

#[cfg(test)]
use std::sync::Arc;

use labello_domain::{
    ClassStats, DatasetStats, ImageState, ReviewDecision, ReviewTarget, TaskId, TaskOutcome,
    TaskStats, TaskStatus,
};

use tokio::sync::Mutex;

#[cfg(test)]
use tokio::sync::Notify;

use crate::{DatasetRepository, StorageResult};

#[derive(Debug)]
struct CachedStats {
    generation: u64,
    stats: DatasetStats,
}

#[cfg(test)]
#[derive(Debug, Default)]
struct StatsScanPause {
    started: Notify,
    resume: Notify,
}

#[derive(Debug, Default)]
pub(crate) struct StatsCache {
    generation: AtomicU64,
    value: Mutex<Option<CachedStats>>,
    refresh: Mutex<()>,
    #[cfg(test)]
    scans: AtomicU64,
    #[cfg(test)]
    refresh_attempts: AtomicU64,
    #[cfg(test)]
    refresh_attempted: Notify,
    #[cfg(test)]
    scan_pause: Mutex<Option<Arc<StatsScanPause>>>,
}

impl StatsCache {
    pub(crate) fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    #[cfg(test)]
    async fn pause_after_next_scan(&self) -> Arc<StatsScanPause> {
        let pause = Arc::new(StatsScanPause::default());
        *self.scan_pause.lock().await = Some(pause.clone());
        pause
    }

    #[cfg(test)]
    async fn wait_for_refresh_attempts(&self, target: u64) {
        loop {
            let attempted = self.refresh_attempted.notified();
            if self.refresh_attempts.load(Ordering::Acquire) >= target {
                return;
            }
            attempted.await;
        }
    }
}

impl DatasetRepository {
    pub async fn dataset_stats(&self) -> StorageResult<DatasetStats> {
        let requested_generation = self.stats_cache.generation.load(Ordering::Acquire);
        {
            let cached = self.stats_cache.value.lock().await;
            if let Some(cached) = cached.as_ref()
                && cached.generation == requested_generation
            {
                return Ok(cached.stats.clone());
            }
        }

        // Keep the scan in the requesting task. Cancellation stops the scan and releases
        // the permit instead of leaving detached blocking scans behind.
        #[cfg(test)]
        {
            self.stats_cache
                .refresh_attempts
                .fetch_add(1, Ordering::Release);
            self.stats_cache.refresh_attempted.notify_one();
        }
        let _refresh = self.stats_cache.refresh.lock().await;
        let generation = self.stats_cache.generation.load(Ordering::Acquire);
        {
            let cached = self.stats_cache.value.lock().await;
            if let Some(cached) = cached.as_ref()
                && cached.generation == generation
            {
                return Ok(cached.stats.clone());
            }
        }

        #[cfg(test)]
        self.stats_cache.scans.fetch_add(1, Ordering::Relaxed);

        let stats = self.compute_dataset_stats().await?;
        #[cfg(test)]
        if let Some(pause) = self.stats_cache.scan_pause.lock().await.take() {
            pause.started.notify_one();
            pause.resume.notified().await;
        }
        let mut cached = self.stats_cache.value.lock().await;
        *cached = Some(CachedStats {
            generation,
            stats: stats.clone(),
        });
        // A concurrent write may make this snapshot stale, but returning it bounds request
        // completion. The next request observes the generation mismatch and refreshes it.
        Ok(stats)
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
                let reviewer_corrected =
                    state
                        .task_states
                        .get(&task.task_id)
                        .is_some_and(|task_state| {
                            task_state.outcome == Some(TaskOutcome::ReviewerCorrected)
                        });
                let review_decision = (!reviewer_corrected)
                    .then(|| current_task_review_decision(&state, &task.task_id))
                    .flatten();
                if review_decision == Some(ReviewDecision::Approved) {
                    stats.reviewed_tasks += 1;
                    task_stats.reviewed += 1;
                }
                match review_decision.as_ref() {
                    Some(ReviewDecision::Approved) => {
                        stats.approved_tasks += 1;
                        task_stats.approved += 1;
                    }
                    Some(ReviewDecision::Rejected) => {
                        stats.rejected_tasks += 1;
                        task_stats.rejected += 1;
                    }
                    None => {}
                }
                if let Some(outcome) = state.task_states.get(&task.task_id).and_then(|task_state| {
                    (task_state.status == TaskStatus::Completed)
                        .then_some(task_state.outcome.as_ref())
                        .flatten()
                }) {
                    stats.finalized_tasks += 1;
                    task_stats.finalized += 1;
                    if outcome == &TaskOutcome::ReviewerCorrected {
                        stats.rejected_tasks += 1;
                        task_stats.rejected += 1;
                        stats.reviewer_corrected_tasks += 1;
                        task_stats.reviewer_corrected += 1;
                    }
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

fn current_task_review_decision(state: &ImageState, task_id: &TaskId) -> Option<ReviewDecision> {
    let task_state = state.task_states.get(task_id)?;
    let task_reviews = state.reviews.iter().filter(|review| {
        matches!(
            &review.target,
            ReviewTarget::Task { task_id: reviewed } if reviewed == task_id
        )
    });

    match task_state.status {
        TaskStatus::Submitted => {
            let round_started_at = task_state.completed_at?;
            task_reviews
                .filter(|review| review.timestamp >= round_started_at)
                .max_by_key(|review| review.timestamp)
                .map(|review| review.decision.clone())
        }
        // Completing an approval round replaces the submitted TaskState timestamp, so the
        // final review, rather than that timestamp, identifies the current round.
        TaskStatus::Completed if task_state.outcome == Some(TaskOutcome::Approved) => task_reviews
            .max_by_key(|review| review.timestamp)
            .map(|review| review.decision.clone()),
        TaskStatus::Completed => None,
        TaskStatus::NeedsCorrection => task_reviews
            .max_by_key(|review| review.timestamp)
            .filter(|review| review.decision == ReviewDecision::Rejected)
            .map(|review| review.decision.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    use labello_domain::{
        Actor, AnnotationGeometry, AnnotationId, AnnotationSource, AnnotationType, Assignment,
        AssignmentId, AssignmentKind, AssignmentStatus, BoundingBox, ClassId, DatasetId,
        DatasetMetadata, DatasetRole, EventPayload, ImageId, ImageRecord, ImageState, ImagesIndex,
        ReviewId, ReviewRecord, SCHEMA_VERSION, TaskId, TaskOutcome, TaskState, UserId, now,
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

    #[tokio::test]
    async fn assignment_only_events_do_not_invalidate_cached_stats() {
        let (_temp, repository) = empty_repository().await;
        repository.dataset_stats().await.unwrap();

        let timestamp = now();
        repository
            .append_payload(
                &ImageId::from("img_1"),
                &Actor {
                    user_id: UserId::from("annotator"),
                    role: DatasetRole::Annotator,
                },
                EventPayload::AssignmentUpdated {
                    assignment: Assignment {
                        assignment_id: AssignmentId::generate(),
                        image_id: ImageId::from("img_1"),
                        task_id: TaskId::from("boxes"),
                        assigned_to: UserId::from("annotator"),
                        kind: AssignmentKind::Annotation,
                        status: AssignmentStatus::Active,
                        expires_at: None,
                        created_at: timestamp,
                        updated_at: timestamp,
                    },
                },
            )
            .await
            .unwrap();

        repository.dataset_stats().await.unwrap();
        assert_eq!(repository.stats_scan_count(), 1);
    }

    #[tokio::test]
    async fn stats_complete_while_invalidations_continue() {
        let (_temp, repository) = empty_repository().await;
        let images_by_hash = (0..128)
            .map(|index| {
                let image = ImageRecord {
                    image_id: ImageId::from(format!("img_{index}")),
                    blake3: format!("hash_{index}"),
                    canonical_path: format!("images/{index}.png"),
                    known_paths: vec![format!("images/{index}.png")],
                    duplicate_paths: Vec::new(),
                    file_name: format!("{index}.png"),
                    byte_size: 4,
                    width: 2,
                    height: 2,
                    media_type: "image/png".to_string(),
                };
                (image.blake3.clone(), image)
            })
            .collect();
        repository
            .save_images_index(&ImagesIndex {
                schema_version: SCHEMA_VERSION,
                image_count: 128,
                images_by_hash,
            })
            .await
            .unwrap();

        let keep_writing = Arc::new(AtomicBool::new(true));
        let writer_flag = keep_writing.clone();
        let writer_repository = repository.clone();
        let writer = tokio::spawn(async move {
            while writer_flag.load(Ordering::Relaxed) {
                writer_repository.stats_cache.invalidate();
                tokio::task::yield_now().await;
            }
        });

        let stats = tokio::time::timeout(Duration::from_secs(2), repository.dataset_stats())
            .await
            .expect("stats request should not wait for a quiet generation")
            .unwrap();
        keep_writing.store(false, Ordering::Relaxed);
        writer.await.unwrap();
        assert_eq!(stats.total_images, 128);
        assert_eq!(repository.stats_scan_count(), 1);
    }

    #[tokio::test]
    async fn waiter_rejects_a_paused_scan_invalidated_before_cache_publish() {
        let (_temp, repository) = empty_repository().await;
        let pause = repository.stats_cache.pause_after_next_scan().await;
        let scanning_repository = repository.clone();
        let scanning = tokio::spawn(async move { scanning_repository.dataset_stats().await });

        pause.started.notified().await;
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

        let waiting_repository = repository.clone();
        let waiting = tokio::spawn(async move { waiting_repository.dataset_stats().await });
        repository.stats_cache.wait_for_refresh_attempts(2).await;
        pause.resume.notify_one();

        let stale = scanning.await.unwrap().unwrap();
        let refreshed = waiting.await.unwrap().unwrap();
        assert_eq!(stale.total_images, 0);
        assert_eq!(refreshed.total_images, 1);
        assert_eq!(repository.stats_scan_count(), 2);
        let generation = repository.stats_cache.generation.load(Ordering::Acquire);
        let cached = repository.stats_cache.value.lock().await;
        assert_eq!(cached.as_ref().unwrap().generation, generation);
    }

    #[tokio::test]
    async fn reviewed_status_uses_the_current_submission_round() {
        let image_id = ImageId::from("img_1");
        let task_id = TaskId::from("boxes");
        let mut state = ImageState::new(image_id);
        state.reviews.push(ReviewRecord {
            review_id: ReviewId::generate(),
            target: ReviewTarget::Task {
                task_id: task_id.clone(),
            },
            reviewer_user_id: UserId::from("reviewer"),
            decision: ReviewDecision::Approved,
            timestamp: now(),
            comment: None,
        });
        tokio::time::sleep(Duration::from_millis(2)).await;
        let submitted_at = now();
        state.task_states.insert(
            task_id.clone(),
            TaskState {
                task_id: task_id.clone(),
                status: TaskStatus::Submitted,
                outcome: None,
                assigned_to: None,
                completed_by: Some(UserId::from("annotator")),
                completed_at: Some(submitted_at),
                updated_at: submitted_at,
            },
        );

        assert_eq!(current_task_review_decision(&state, &task_id), None);

        state.task_states.insert(
            task_id.clone(),
            TaskState {
                task_id: task_id.clone(),
                status: TaskStatus::Completed,
                outcome: Some(TaskOutcome::ReviewerCorrected),
                assigned_to: None,
                completed_by: Some(UserId::from("reviewer")),
                completed_at: Some(submitted_at),
                updated_at: submitted_at,
            },
        );
        assert_eq!(current_task_review_decision(&state, &task_id), None);

        let reviewed_at = now();
        state.reviews.push(ReviewRecord {
            review_id: ReviewId::generate(),
            target: ReviewTarget::Task {
                task_id: task_id.clone(),
            },
            reviewer_user_id: UserId::from("reviewer_2"),
            decision: ReviewDecision::Rejected,
            timestamp: reviewed_at,
            comment: None,
        });
        state.task_states.insert(
            task_id.clone(),
            TaskState {
                task_id: task_id.clone(),
                status: TaskStatus::NeedsCorrection,
                outcome: None,
                assigned_to: None,
                completed_by: Some(UserId::from("reviewer_2")),
                completed_at: Some(reviewed_at),
                updated_at: reviewed_at,
            },
        );
        assert_eq!(
            current_task_review_decision(&state, &task_id),
            Some(ReviewDecision::Rejected)
        );
    }
}
