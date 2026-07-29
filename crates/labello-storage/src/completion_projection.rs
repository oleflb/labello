use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};

use labello_domain::{ImageId, ImageState, ImbalanceConfig, TaskId, TaskStatus};
use parking_lot::Mutex;
use tokio::sync::Mutex as AsyncMutex;

use crate::{DatasetRepository, StorageError, StorageResult};

const MAX_COMPLETION_SCAN_WORKERS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImageCompletion {
    sequence: u64,
    annotation_completed_tasks: BTreeSet<TaskId>,
    completed_tasks: BTreeSet<TaskId>,
}

impl ImageCompletion {
    pub(crate) fn from_state(state: &ImageState) -> Self {
        let annotation_completed_tasks = state
            .task_states
            .iter()
            .filter(|(task_id, task_state)| {
                matches!(
                    task_state.status,
                    TaskStatus::Submitted | TaskStatus::Completed
                ) && state.included_in_completion_denominator(task_id)
            })
            .map(|(task_id, _)| task_id.clone())
            .collect();
        let completed_tasks = state
            .task_states
            .iter()
            .filter(|(task_id, task_state)| {
                task_state.status == TaskStatus::Completed
                    && state.included_in_completion_denominator(task_id)
            })
            .map(|(task_id, _)| task_id.clone())
            .collect();
        Self {
            sequence: state.current_sequence,
            annotation_completed_tasks,
            completed_tasks,
        }
    }

    fn has_same_progress(&self, other: &Self) -> bool {
        self.annotation_completed_tasks == other.annotation_completed_tasks
            && self.completed_tasks == other.completed_tasks
    }
}

#[derive(Debug)]
struct TaskCompletionProjection {
    membership_generation: u64,
    per_image: BTreeMap<ImageId, ImageCompletion>,
    per_annotation_task: BTreeMap<TaskId, usize>,
    per_task: BTreeMap<TaskId, usize>,
}

impl TaskCompletionProjection {
    fn from_images(
        membership_generation: u64,
        per_image: BTreeMap<ImageId, ImageCompletion>,
    ) -> Self {
        let mut per_annotation_task = BTreeMap::new();
        let mut per_task = BTreeMap::new();
        for completion in per_image.values() {
            for task_id in &completion.annotation_completed_tasks {
                *per_annotation_task.entry(task_id.clone()).or_default() += 1;
            }
            for task_id in &completion.completed_tasks {
                *per_task.entry(task_id.clone()).or_default() += 1;
            }
        }
        Self {
            membership_generation,
            per_image,
            per_annotation_task,
            per_task,
        }
    }

    fn observe(
        &mut self,
        image_id: &ImageId,
        observation: ImageCompletion,
        authoritative: bool,
    ) -> Result<(), ProjectionInvariant> {
        let Some(current) = self.per_image.get(image_id) else {
            return Ok(());
        };
        match observation.sequence.cmp(&current.sequence) {
            Ordering::Less => return Ok(()),
            Ordering::Equal if observation.has_same_progress(current) => {
                return Ok(());
            }
            Ordering::Equal if !authoritative => {
                return Err(ProjectionInvariant::EqualSequenceMismatch);
            }
            Ordering::Equal | Ordering::Greater => {}
        }

        apply_task_delta(
            &mut self.per_annotation_task,
            &current.annotation_completed_tasks,
            &observation.annotation_completed_tasks,
        )?;
        apply_task_delta(
            &mut self.per_task,
            &current.completed_tasks,
            &observation.completed_tasks,
        )?;
        self.per_image.insert(image_id.clone(), observation);
        Ok(())
    }
}

fn apply_task_delta(
    counts: &mut BTreeMap<TaskId, usize>,
    previous: &BTreeSet<TaskId>,
    next: &BTreeSet<TaskId>,
) -> Result<(), ProjectionInvariant> {
    for removed in previous.difference(next) {
        let Some(count) = counts.get_mut(removed) else {
            return Err(ProjectionInvariant::Underflow);
        };
        let Some(next_count) = count.checked_sub(1) else {
            return Err(ProjectionInvariant::Underflow);
        };
        *count = next_count;
        if next_count == 0 {
            counts.remove(removed);
        }
    }
    for added in next.difference(previous) {
        *counts.entry(added.clone()).or_default() += 1;
    }
    Ok(())
}

#[derive(Debug)]
struct ActiveScan {
    attempt_id: u64,
    membership_generation: u64,
    indexed_image_ids: BTreeSet<ImageId>,
    pending: BTreeMap<ImageId, ImageCompletion>,
}

#[derive(Debug, Default)]
struct CacheInner {
    membership_generation: u64,
    next_attempt_id: u64,
    projection: Option<TaskCompletionProjection>,
    active_scan: Option<ActiveScan>,
}

#[derive(Clone, Copy, Debug)]
enum ProjectionInvariant {
    EqualSequenceMismatch,
    Underflow,
}

#[derive(Debug, Default)]
pub(crate) struct TaskCompletionCache {
    inner: Mutex<CacheInner>,
    refresh: AsyncMutex<()>,
    #[cfg(test)]
    scans: std::sync::atomic::AtomicU64,
}

impl TaskCompletionCache {
    fn counts(&self, include_submitted: bool) -> Option<BTreeMap<TaskId, usize>> {
        let inner = self.inner.lock();
        inner
            .projection
            .as_ref()
            .filter(|projection| projection.membership_generation == inner.membership_generation)
            .map(|projection| {
                if include_submitted {
                    projection.per_annotation_task.clone()
                } else {
                    projection.per_task.clone()
                }
            })
    }

    fn begin_scan(&self, indexed_image_ids: BTreeSet<ImageId>) -> CompletionScanGuard<'_> {
        let mut inner = self.inner.lock();
        inner.next_attempt_id = inner.next_attempt_id.wrapping_add(1);
        let attempt_id = inner.next_attempt_id;
        let membership_generation = inner.membership_generation;
        inner.active_scan = Some(ActiveScan {
            attempt_id,
            membership_generation,
            indexed_image_ids,
            pending: BTreeMap::new(),
        });
        CompletionScanGuard {
            cache: self,
            attempt_id,
            membership_generation,
            armed: true,
        }
    }

    fn publish_scan(
        &self,
        attempt_id: u64,
        membership_generation: u64,
        mut scanned: BTreeMap<ImageId, ImageCompletion>,
    ) -> Option<BTreeMap<TaskId, usize>> {
        let mut inner = self.inner.lock();
        let active = inner.active_scan.take()?;
        if active.attempt_id != attempt_id
            || active.membership_generation != membership_generation
            || inner.membership_generation != membership_generation
        {
            tracing::debug!(
                event = "completion_projection.scan.discarded",
                attempt_id,
                membership_generation,
                current_generation = inner.membership_generation,
                "task completion scan was invalidated before publication"
            );
            return None;
        }
        let pending_count = active.pending.len();
        for (image_id, observation) in active.pending {
            if !active.indexed_image_ids.contains(&image_id) {
                continue;
            }
            let replace = scanned
                .get(&image_id)
                .is_none_or(|current| observation.sequence >= current.sequence);
            if replace {
                scanned.insert(image_id, observation);
            }
        }
        if scanned.keys().ne(active.indexed_image_ids.iter()) {
            tracing::warn!(
                event = "completion_projection.scan.membership_mismatch",
                attempt_id,
                membership_generation,
                "task completion scan did not produce the captured image membership"
            );
            return None;
        }
        let projection = TaskCompletionProjection::from_images(membership_generation, scanned);
        let counts = projection.per_task.clone();
        inner.projection = Some(projection);
        tracing::debug!(
            event = "completion_projection.scan.published",
            attempt_id,
            membership_generation,
            pending_observations = pending_count,
            "task completion projection published"
        );
        Some(counts)
    }

    fn abort_scan(&self, attempt_id: u64, membership_generation: u64) {
        let mut inner = self.inner.lock();
        if inner.active_scan.as_ref().is_some_and(|active| {
            active.attempt_id == attempt_id && active.membership_generation == membership_generation
        }) {
            inner.active_scan = None;
            tracing::debug!(
                event = "completion_projection.scan.cancelled",
                attempt_id,
                membership_generation,
                "task completion scan attempt was cancelled"
            );
        }
    }

    pub(crate) fn observe_transition(
        &self,
        image_id: &ImageId,
        previous: ImageCompletion,
        next: ImageCompletion,
    ) {
        if previous.has_same_progress(&next) {
            return;
        }
        self.observe(image_id, next, false);
    }

    pub(crate) fn observe_authoritative(&self, state: &ImageState) {
        self.observe(&state.image_id, ImageCompletion::from_state(state), true);
    }

    fn observe(&self, image_id: &ImageId, observation: ImageCompletion, authoritative: bool) {
        let mut inner = self.inner.lock();
        if let Some(mut projection) = inner.projection.take() {
            match projection.observe(image_id, observation.clone(), authoritative) {
                Ok(()) => inner.projection = Some(projection),
                Err(invariant) => {
                    tracing::warn!(
                        event = "completion_projection.invalidated",
                        reason = ?invariant,
                        "task completion projection invariant failed"
                    );
                }
            }
            return;
        }

        let Some(active) = inner.active_scan.as_mut() else {
            return;
        };
        if !active.indexed_image_ids.contains(image_id) {
            return;
        }
        match active.pending.get(image_id) {
            Some(current) => match observation.sequence.cmp(&current.sequence) {
                Ordering::Less => {}
                Ordering::Equal if observation.has_same_progress(current) => {}
                Ordering::Equal if authoritative => {
                    active.pending.insert(image_id.clone(), observation);
                }
                Ordering::Equal => {
                    tracing::warn!(
                        event = "completion_projection.scan.invalidated",
                        reason = "equal_sequence_mismatch",
                        "task completion scan observation invariant failed"
                    );
                    inner.active_scan = None;
                }
                Ordering::Greater => {
                    active.pending.insert(image_id.clone(), observation);
                }
            },
            None => {
                active.pending.insert(image_id.clone(), observation);
            }
        }
    }

    pub(crate) fn invalidate_membership(&self, reason: &'static str) {
        let mut inner = self.inner.lock();
        inner.membership_generation = inner.membership_generation.wrapping_add(1);
        inner.projection = None;
        inner.active_scan = None;
        tracing::debug!(
            event = "completion_projection.invalidated",
            reason,
            membership_generation = inner.membership_generation,
            "task completion projection membership changed"
        );
    }

    pub(crate) fn invalidate(&self, reason: &'static str) {
        let mut inner = self.inner.lock();
        inner.projection = None;
        inner.active_scan = None;
        tracing::debug!(
            event = "completion_projection.invalidated",
            reason,
            membership_generation = inner.membership_generation,
            "task completion projection invalidated"
        );
    }

    #[cfg(test)]
    pub(crate) fn scan_count(&self) -> u64 {
        self.scans.load(std::sync::atomic::Ordering::Relaxed)
    }
}

struct CompletionScanGuard<'a> {
    cache: &'a TaskCompletionCache,
    attempt_id: u64,
    membership_generation: u64,
    armed: bool,
}

impl CompletionScanGuard<'_> {
    fn publish(
        mut self,
        scanned: BTreeMap<ImageId, ImageCompletion>,
    ) -> Option<BTreeMap<TaskId, usize>> {
        let result = self
            .cache
            .publish_scan(self.attempt_id, self.membership_generation, scanned);
        self.armed = false;
        result
    }
}

impl Drop for CompletionScanGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.cache
                .abort_scan(self.attempt_id, self.membership_generation);
        }
    }
}

impl DatasetRepository {
    pub(crate) async fn task_completion_counts(&self) -> StorageResult<BTreeMap<TaskId, usize>> {
        self.task_progress_counts(false).await
    }

    pub(crate) async fn task_annotation_counts(&self) -> StorageResult<BTreeMap<TaskId, usize>> {
        self.task_progress_counts(true).await
    }

    async fn task_progress_counts(
        &self,
        include_submitted: bool,
    ) -> StorageResult<BTreeMap<TaskId, usize>> {
        if let Some(counts) = self.task_completion_cache.counts(include_submitted) {
            return Ok(counts);
        }
        let _refresh = self.task_completion_cache.refresh.lock().await;
        loop {
            if let Some(counts) = self.task_completion_cache.counts(include_submitted) {
                return Ok(counts);
            }

            self.load_images_index_shared().await?;
            let cached_index = self.images_index_cache.read().await;
            let indexed_image_ids = cached_index
                .as_ref()
                .expect("image index cache was populated")
                .images_by_hash
                .values()
                .map(|record| record.image_id.clone())
                .collect::<BTreeSet<_>>();
            let attempt = self
                .task_completion_cache
                .begin_scan(indexed_image_ids.clone());
            let attempt_id = attempt.attempt_id;
            let membership_generation = attempt.membership_generation;
            drop(cached_index);

            #[cfg(test)]
            self.task_completion_cache
                .scans
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let started = Instant::now();
            tracing::debug!(
                event = "completion_projection.scan.started",
                attempt_id,
                membership_generation,
                image_count = indexed_image_ids.len(),
                "task completion projection scan started"
            );

            let mut image_ids = indexed_image_ids.into_iter();
            let mut workers = tokio::task::JoinSet::new();
            for image_id in image_ids.by_ref().take(MAX_COMPLETION_SCAN_WORKERS) {
                let repository = self.clone();
                workers.spawn(async move {
                    let state = repository.load_image_state(&image_id).await?;
                    Ok::<_, StorageError>((image_id, ImageCompletion::from_state(&state)))
                });
            }
            let mut scanned = BTreeMap::new();
            while let Some(result) = workers.join_next().await {
                let (image_id, completion) = result.map_err(|error| {
                    StorageError::BackgroundTask(format!("task completion worker failed: {error}"))
                })??;
                scanned.insert(image_id, completion);
                if let Some(image_id) = image_ids.next() {
                    let repository = self.clone();
                    workers.spawn(async move {
                        let state = repository.load_image_state(&image_id).await?;
                        Ok::<_, StorageError>((image_id, ImageCompletion::from_state(&state)))
                    });
                }
            }
            if let Some(counts) = attempt.publish(scanned) {
                tracing::debug!(
                    event = "completion_projection.scan.completed",
                    attempt_id,
                    membership_generation,
                    elapsed_ms = started.elapsed().as_millis(),
                    "task completion projection scan completed"
                );
                return Ok(if include_submitted {
                    self.task_completion_cache
                        .counts(true)
                        .expect("the published projection must remain available")
                } else {
                    counts
                });
            }
        }
    }

    pub(crate) fn completion_observation(&self, state: &ImageState) -> ImageCompletion {
        ImageCompletion::from_state(state)
    }

    pub(crate) fn observe_completion_transition(
        &self,
        image_id: &ImageId,
        previous: ImageCompletion,
        next: &ImageState,
    ) {
        self.task_completion_cache.observe_transition(
            image_id,
            previous,
            ImageCompletion::from_state(next),
        );
    }

    pub(crate) fn observe_authoritative_completion(&self, state: &ImageState) {
        self.task_completion_cache.observe_authoritative(state);
    }

    #[cfg(test)]
    pub(crate) async fn pause_after_next_completion_observation(
        &self,
    ) -> std::sync::Arc<crate::repository::CompletionCommitPause> {
        let pause = std::sync::Arc::new(crate::repository::CompletionCommitPause::default());
        *self.completion_commit_pause.lock().await = Some(pause.clone());
        pause
    }

    #[cfg(test)]
    pub(crate) fn fail_next_state_cache_write_after_completion(&self) {
        self.fail_state_cache_write_after_completion
            .store(true, std::sync::atomic::Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) async fn completion_post_observation_test_hook(&self) -> StorageResult<()> {
        if let Some(pause) = self.completion_commit_pause.lock().await.take() {
            pause.started.notify_one();
            pause.resume.notified().await;
        }
        if self
            .fail_state_cache_write_after_completion
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            return Err(StorageError::BackgroundTask(
                "injected state cache publication failure".to_string(),
            ));
        }
        Ok(())
    }
}

pub(crate) fn validated_max_ratio(config: &ImbalanceConfig) -> StorageResult<f64> {
    if config.max_ratio.is_finite() && config.max_ratio >= 1.0 {
        Ok(f64::from(config.max_ratio))
    } else {
        Err(StorageError::InvalidAssignment(
            "imbalance maxRatio must be finite and at least 1.0".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use labello_domain::{
        Actor, AnnotationGeometry, AnnotationId, AnnotationType, BoundingBox, ClassId, DatasetId,
        DatasetMetadata, DatasetRole, DatasetRoleAssignment, ImageRecord, ImagesIndex,
        ImportCoverage, LabelClass, OfflineAnnotationSource, OfflineMutation,
        OfflineMutationFragment, OfflineSyncRequest, ReviewConfig, SCHEMA_VERSION, TaskDefinition,
        TaskOutcome, TaskState, TutorialContent, UserId, now,
    };

    use super::*;

    fn state_with_task(
        image_id: &str,
        task_id: &str,
        sequence: u64,
        status: TaskStatus,
    ) -> ImageState {
        let mut state = ImageState::new(ImageId::from(image_id));
        state.current_sequence = sequence;
        let task_id = TaskId::from(task_id);
        state.task_states.insert(
            task_id.clone(),
            TaskState {
                task_id,
                status,
                outcome: None,
                assigned_to: None,
                completed_by: None,
                completed_at: None,
                updated_at: now(),
            },
        );
        state
    }

    async fn repository_with_image()
    -> (tempfile::TempDir, DatasetRepository, ImageId, TaskId, Actor) {
        let temp = tempfile::tempdir().unwrap();
        let repository = DatasetRepository::new(temp.path());
        let task_id = TaskId::from("boxes");
        let class_id = ClassId::from("person");
        let mut metadata = DatasetMetadata::new(DatasetId::from("ds"), "Dataset", now());
        metadata.label_classes.push(LabelClass {
            class_id: class_id.clone(),
            name: "Person".to_string(),
            color: "#ffffff".to_string(),
            description: None,
        });
        metadata.tasks.push(TaskDefinition {
            task_id: task_id.clone(),
            name: "Boxes".to_string(),
            annotation_type: AnnotationType::BoundingBox,
            class_ids: vec![class_id],
            instructions: TutorialContent {
                title: "Boxes".to_string(),
                example_text: "Draw boxes".to_string(),
                example_images: Vec::new(),
            },
            skeleton: None,
            review: ReviewConfig::default(),
            prelabel_config_ids: Vec::new(),
            manual_box_guide_migration: None,
            enabled: true,
        });
        metadata.role_assignments.push(DatasetRoleAssignment {
            dataset_id: metadata.dataset_id.clone(),
            user_id: UserId::from("annotator"),
            roles: BTreeSet::from([DatasetRole::Annotator]),
            assigned_at: now(),
            assigned_by: None,
        });
        repository.initialize(metadata).await.unwrap();
        let image_id = ImageId::from("img");
        repository
            .save_images_index(&ImagesIndex {
                schema_version: SCHEMA_VERSION,
                image_count: 1,
                images_by_hash: BTreeMap::from([(
                    "hash".to_string(),
                    ImageRecord {
                        image_id: image_id.clone(),
                        blake3: "hash".to_string(),
                        canonical_path: "images/image.png".to_string(),
                        known_paths: vec!["images/image.png".to_string()],
                        duplicate_paths: Vec::new(),
                        file_name: "image.png".to_string(),
                        byte_size: 1,
                        width: 1,
                        height: 1,
                        media_type: "image/png".to_string(),
                        source_memberships: None,
                    },
                )]),
            })
            .await
            .unwrap();
        let actor = Actor {
            user_id: UserId::from("annotator"),
            role: DatasetRole::Annotator,
        };
        (temp, repository, image_id, task_id, actor)
    }

    fn task_state_payload(task_id: &TaskId, status: TaskStatus) -> labello_domain::EventPayload {
        labello_domain::EventPayload::TaskStateChanged {
            task_state: TaskState {
                task_id: task_id.clone(),
                status,
                outcome: Some(TaskOutcome::AnnotationCompleted),
                assigned_to: None,
                completed_by: Some(UserId::from("annotator")),
                completed_at: Some(now()),
                updated_at: now(),
            },
        }
    }

    #[test]
    fn contribution_tracks_annotation_and_review_progress_separately() {
        let completed = state_with_task("img", "completed", 1, TaskStatus::Completed);
        let completed = ImageCompletion::from_state(&completed);
        let completed_task = BTreeSet::from([TaskId::from("completed")]);
        assert_eq!(completed.annotation_completed_tasks, completed_task);
        assert_eq!(completed.completed_tasks, completed_task);

        for status in [
            TaskStatus::Pending,
            TaskStatus::InProgress,
            TaskStatus::NeedsCorrection,
        ] {
            let state = state_with_task("img", "task", 1, status);
            let progress = ImageCompletion::from_state(&state);
            assert!(progress.annotation_completed_tasks.is_empty());
            assert!(progress.completed_tasks.is_empty());
        }

        let submitted =
            ImageCompletion::from_state(&state_with_task("img", "task", 1, TaskStatus::Submitted));
        assert_eq!(
            submitted.annotation_completed_tasks,
            BTreeSet::from([TaskId::from("task")])
        );
        assert!(submitted.completed_tasks.is_empty());

        let mut excluded = state_with_task("img", "excluded", 1, TaskStatus::Completed);
        excluded
            .import_coverage
            .insert(TaskId::from("excluded"), ImportCoverage::Excluded);
        let excluded_progress = ImageCompletion::from_state(&excluded);
        assert!(excluded_progress.annotation_completed_tasks.is_empty());
        assert!(excluded_progress.completed_tasks.is_empty());
        excluded
            .included_import_tasks
            .insert(TaskId::from("excluded"));
        let included_progress = ImageCompletion::from_state(&excluded);
        let included_task = BTreeSet::from([TaskId::from("excluded")]);
        assert_eq!(included_progress.annotation_completed_tasks, included_task);
        assert_eq!(included_progress.completed_tasks, included_task);
    }

    #[test]
    fn warm_projection_applies_newer_deltas_and_repairs_authoritative_mismatch() {
        let image_id = ImageId::from("img");
        let pending =
            ImageCompletion::from_state(&state_with_task("img", "task", 1, TaskStatus::Pending));
        let completed =
            ImageCompletion::from_state(&state_with_task("img", "task", 2, TaskStatus::Completed));
        let mut projection = TaskCompletionProjection::from_images(
            0,
            BTreeMap::from([(image_id.clone(), pending.clone())]),
        );

        projection
            .observe(&image_id, completed.clone(), false)
            .unwrap();
        assert_eq!(projection.per_task[&TaskId::from("task")], 1);
        projection
            .observe(&image_id, pending.clone(), false)
            .unwrap();
        assert_eq!(projection.per_task[&TaskId::from("task")], 1);

        let equal_sequence_pending = ImageCompletion {
            sequence: completed.sequence,
            annotation_completed_tasks: BTreeSet::new(),
            completed_tasks: BTreeSet::new(),
        };
        assert!(matches!(
            projection.observe(&image_id, equal_sequence_pending.clone(), false),
            Err(ProjectionInvariant::EqualSequenceMismatch)
        ));
        projection
            .observe(&image_id, equal_sequence_pending, true)
            .unwrap();
        assert!(projection.per_task.is_empty());
    }

    #[test]
    fn membership_change_discards_attempt_pending_observations() {
        let cache = TaskCompletionCache::default();
        let image_id = ImageId::from("removed");
        let attempt = cache.begin_scan(BTreeSet::from([image_id.clone()]));
        let pending = ImageCompletion::from_state(&state_with_task(
            "removed",
            "task",
            1,
            TaskStatus::Pending,
        ));
        let completed = ImageCompletion::from_state(&state_with_task(
            "removed",
            "task",
            2,
            TaskStatus::Completed,
        ));
        cache.observe_transition(&image_id, pending.clone(), completed);
        cache.invalidate_membership("test");

        assert!(
            attempt
                .publish(BTreeMap::from([(image_id, pending)]))
                .is_none()
        );
        let next = cache.begin_scan(BTreeSet::new());
        assert_eq!(next.publish(BTreeMap::new()), Some(BTreeMap::new()));
    }

    #[test]
    fn invalid_max_ratios_are_rejected() {
        for max_ratio in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0, 0.999] {
            assert!(
                validated_max_ratio(&ImbalanceConfig {
                    max_ratio,
                    enforce: true,
                })
                .is_err()
            );
        }
        for max_ratio in [1.0, 2.0, f32::MAX] {
            assert!(
                validated_max_ratio(&ImbalanceConfig {
                    max_ratio,
                    enforce: true,
                })
                .is_ok()
            );
        }
    }

    #[tokio::test]
    async fn repository_rejects_invalid_imbalance_configuration() {
        for max_ratio in [f32::NAN, f32::INFINITY, -1.0, 0.999] {
            let temp = tempfile::tempdir().unwrap();
            let repository = DatasetRepository::new(temp.path());
            let mut metadata = DatasetMetadata::new(DatasetId::from("ds"), "Dataset", now());
            metadata.imbalance = Some(ImbalanceConfig {
                max_ratio,
                enforce: true,
            });
            assert!(repository.initialize(metadata).await.is_err());
            assert!(!repository.dataset_path().exists());
        }

        let temp = tempfile::tempdir().unwrap();
        let repository = DatasetRepository::new(temp.path());
        let mut metadata = DatasetMetadata::new(DatasetId::from("ds"), "Dataset", now());
        repository.initialize(metadata.clone()).await.unwrap();
        metadata.imbalance = Some(ImbalanceConfig {
            max_ratio: f32::NEG_INFINITY,
            enforce: true,
        });
        assert!(repository.save_dataset(&metadata).await.is_err());
    }

    #[tokio::test]
    async fn repository_scan_is_shared_and_stage_progress_is_incremental() {
        let (_temp, repository, image_id, task_id, actor) = repository_with_image().await;
        let (annotation, review) = tokio::join!(
            repository.task_annotation_counts(),
            repository.task_completion_counts(),
        );
        assert_eq!(annotation.unwrap(), BTreeMap::new());
        assert_eq!(review.unwrap(), BTreeMap::new());
        assert_eq!(repository.task_completion_cache.scan_count(), 1);

        repository
            .append_payload(
                &image_id,
                &actor,
                task_state_payload(&task_id, TaskStatus::Submitted),
            )
            .await
            .unwrap();
        assert_eq!(
            repository.task_annotation_counts().await.unwrap(),
            BTreeMap::from([(task_id.clone(), 1)])
        );
        assert_eq!(
            repository.task_completion_counts().await.unwrap(),
            BTreeMap::new()
        );

        repository
            .append_payload(
                &image_id,
                &actor,
                task_state_payload(&task_id, TaskStatus::Completed),
            )
            .await
            .unwrap();
        assert_eq!(
            repository.task_completion_counts().await.unwrap(),
            BTreeMap::from([(task_id.clone(), 1)])
        );
        assert_eq!(
            repository.task_annotation_counts().await.unwrap(),
            BTreeMap::from([(task_id, 1)])
        );
        assert_eq!(repository.task_completion_cache.scan_count(), 1);
    }

    #[tokio::test]
    async fn identical_index_and_configuration_saves_keep_projection_warm() {
        let (_temp, repository, _image_id, _task_id, _actor) = repository_with_image().await;
        repository.task_completion_counts().await.unwrap();
        let index = repository.load_images_index().await.unwrap();
        repository.save_images_index(&index).await.unwrap();
        let mut metadata = repository.load_dataset_config().await.unwrap();
        metadata.name = "Renamed".to_string();
        repository.save_dataset(&metadata).await.unwrap();

        repository.task_completion_counts().await.unwrap();
        assert_eq!(repository.task_completion_cache.scan_count(), 1);
    }

    #[tokio::test]
    async fn completed_disabled_task_is_ready_when_enabled_without_reconstruction() {
        let (_temp, repository, image_id, task_id, actor) = repository_with_image().await;
        let mut metadata = repository.load_dataset_config().await.unwrap();
        metadata.tasks[0].enabled = false;
        repository.save_dataset(&metadata).await.unwrap();
        repository.task_completion_counts().await.unwrap();

        repository
            .append_payload(
                &image_id,
                &actor,
                task_state_payload(&task_id, TaskStatus::Completed),
            )
            .await
            .unwrap();
        metadata.tasks[0].enabled = true;
        repository.save_dataset(&metadata).await.unwrap();

        assert_eq!(
            repository.task_completion_counts().await.unwrap(),
            BTreeMap::from([(task_id, 1)])
        );
        assert_eq!(repository.task_completion_cache.scan_count(), 1);
    }

    #[tokio::test]
    async fn image_membership_change_reconstructs_and_excludes_removed_completion() {
        let (_temp, repository, image_id, task_id, actor) = repository_with_image().await;
        repository.task_completion_counts().await.unwrap();
        repository
            .append_payload(
                &image_id,
                &actor,
                task_state_payload(&task_id, TaskStatus::Completed),
            )
            .await
            .unwrap();
        repository
            .save_images_index(&ImagesIndex::default())
            .await
            .unwrap();

        assert_eq!(
            repository.task_completion_counts().await.unwrap(),
            BTreeMap::new()
        );
        assert_eq!(repository.task_completion_cache.scan_count(), 2);
    }

    #[tokio::test]
    async fn cancellation_after_event_publication_cannot_skip_projection_update() {
        let (_temp, repository, image_id, task_id, actor) = repository_with_image().await;
        repository.task_completion_counts().await.unwrap();
        let pause = repository.pause_after_next_completion_observation().await;
        let writing_repository = repository.clone();
        let writing_image_id = image_id.clone();
        let writing_task_id = task_id.clone();
        let writer = tokio::spawn(async move {
            writing_repository
                .append_payload(
                    &writing_image_id,
                    &actor,
                    task_state_payload(&writing_task_id, TaskStatus::Completed),
                )
                .await
        });
        pause.started.notified().await;
        writer.abort();
        assert!(writer.await.unwrap_err().is_cancelled());

        assert_eq!(
            repository.task_completion_counts().await.unwrap(),
            BTreeMap::from([(task_id.clone(), 1)])
        );
        assert_eq!(repository.task_completion_cache.scan_count(), 1);
    }

    #[tokio::test]
    async fn state_cache_failure_after_event_publication_leaves_projection_correct() {
        let (_temp, repository, image_id, task_id, actor) = repository_with_image().await;
        repository.task_completion_counts().await.unwrap();
        repository.fail_next_state_cache_write_after_completion();

        assert!(
            repository
                .append_payload(
                    &image_id,
                    &actor,
                    task_state_payload(&task_id, TaskStatus::Completed),
                )
                .await
                .is_err()
        );
        assert_eq!(
            repository.task_completion_counts().await.unwrap(),
            BTreeMap::from([(task_id.clone(), 1)])
        );
        assert_eq!(
            repository
                .load_image_state(&image_id)
                .await
                .unwrap()
                .task_states[&task_id]
                .status,
            TaskStatus::Completed
        );
    }

    #[tokio::test]
    async fn accepted_offline_fragment_uses_shared_transaction_and_keeps_counts_warm() {
        let (_temp, repository, image_id, task_id, _actor) = repository_with_image().await;
        let mut metadata = repository.load_dataset_config().await.unwrap();
        metadata.imbalance = Some(ImbalanceConfig {
            max_ratio: 2.0,
            enforce: true,
        });
        repository.save_dataset(&metadata).await.unwrap();
        repository.dataset_stats().await.unwrap();
        repository.task_completion_counts().await.unwrap();
        let result = repository
            .sync_offline_events(OfflineSyncRequest::new(
                DatasetId::from("ds"),
                UserId::from("annotator"),
                vec![OfflineMutationFragment {
                    image_id,
                    base_sequence: 0,
                    mutations: vec![OfflineMutation::AnnotationUpsert {
                        annotation_id: AnnotationId::from("offline"),
                        expected_version: None,
                        task_id,
                        class_id: ClassId::from("person"),
                        annotation_type: AnnotationType::BoundingBox,
                        source: OfflineAnnotationSource::Human,
                        geometry: AnnotationGeometry::BoundingBox(BoundingBox {
                            x: 0.1,
                            y: 0.1,
                            width: 0.2,
                            height: 0.2,
                        }),
                        reason: None,
                    }],
                }],
            ))
            .await
            .unwrap();

        assert_eq!(result.merged_events, 1);
        assert!(result.conflicts.is_empty());
        assert_eq!(
            repository.task_completion_counts().await.unwrap(),
            BTreeMap::new()
        );
        assert!(
            repository
                .assign_next_image(
                    &UserId::from("annotator"),
                    &TaskId::from("boxes"),
                    labello_domain::AssignmentKind::Annotation,
                )
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(repository.task_completion_cache.scan_count(), 1);
        assert_eq!(
            repository.stats_scan_count(),
            1,
            "assignment balancing must not refresh the invalidated UI statistics cache"
        );
    }
}
