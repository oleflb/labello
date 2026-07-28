use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
};

#[cfg(test)]
use std::sync::Arc;

use labello_domain::{
    ClassStats, DatasetStats, ImageState, ImportCoverage, MigrationDispositionStatus,
    ReviewDecision, ReviewTarget, RevisionSource, TaskId, TaskOutcome, TaskStats, TaskStatus,
};

use tokio::sync::Mutex;

#[cfg(test)]
use tokio::sync::Notify;

use crate::{DatasetRepository, StorageResult};

mod aggregation;
mod cache;
mod scan;

use aggregation::StatsAggregation;
#[cfg(test)]
use aggregation::current_task_review_decision;
pub(crate) use cache::StatsCache;

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
        Actor, AnnotationGeometry, AnnotationId, AnnotationOrigin, AnnotationType, Assignment,
        AssignmentId, AssignmentKind, AssignmentStatus, BoundingBox, ClassId, DatasetId,
        DatasetMetadata, DatasetRole, EventPayload, HumanRevisionKind, ImageId, ImageRecord,
        ImageState, ImagesIndex, ImportCoverage, ImportGeometryProvenance, ImportId,
        ImportTaskInitialization, ImportedOrigin, ReviewConfig, ReviewId, ReviewRecord,
        ReviewWorkflow, RevisionSource, SCHEMA_VERSION, SourceProfile, TaskDefinition, TaskId,
        TaskOutcome, TaskState, TutorialContent, UserId, now,
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
            source_memberships: None,
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
                        object_group_id: None,
                        origin: AnnotationOrigin::native(),
                        task_id: TaskId::from("boxes"),
                        class_id: ClassId::from("person"),
                        annotation_type: AnnotationType::BoundingBox,
                        revision_source: RevisionSource::Human {
                            action: HumanRevisionKind::Authored,
                        },
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
        repository
            .append_payload(
                &ImageId::from("img_1"),
                &Actor {
                    user_id: UserId::from("admin"),
                    role: DatasetRole::DataAdmin,
                },
                EventPayload::AnnotationVersionCreated {
                    annotation: labello_domain::AnnotationVersion {
                        annotation_id: AnnotationId::from("ann_imported"),
                        version: 1,
                        object_group_id: None,
                        origin: AnnotationOrigin::Imported {
                            imported: ImportedOrigin {
                                import_id: ImportId::from("imp_1"),
                                source_profile: SourceProfile {
                                    profile_id: "fixture".to_string(),
                                    profile_version: 1,
                                },
                                source_namespace: "fixture".to_string(),
                                source_object_key: "fixture:1".to_string(),
                                geometry_provenance: ImportGeometryProvenance::Direct,
                            },
                        },
                        task_id: TaskId::from("boxes"),
                        class_id: ClassId::from("person"),
                        annotation_type: AnnotationType::BoundingBox,
                        revision_source: RevisionSource::Import {
                            import_id: ImportId::from("imp_1"),
                        },
                        geometry: AnnotationGeometry::BoundingBox(BoundingBox {
                            x: 0.4,
                            y: 0.4,
                            width: 0.2,
                            height: 0.2,
                        }),
                        author_user_id: UserId::from("admin"),
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
        assert_eq!(stats.per_class[&ClassId::from("person")].annotations, 2);
        assert_eq!(stats.provenance.human_authored_annotations, 1);
        assert_eq!(
            stats.per_task[&TaskId::from("boxes")]
                .provenance
                .human_authored_annotations,
            1
        );
        assert_eq!(
            stats
                .throughput
                .iter()
                .map(|point| point.annotations)
                .sum::<usize>(),
            1
        );
        assert_eq!(
            stats.per_class[&ClassId::from("person")]
                .provenance
                .human_authored_annotations,
            1
        );
        assert_eq!(repository.stats_scan_count(), 3);
    }

    #[tokio::test]
    async fn excluded_import_coverage_is_reported_but_omitted_from_completion_counts() {
        let temp = tempfile::tempdir().unwrap();
        let repository = DatasetRepository::new(temp.path());
        let task_id = TaskId::from("boxes");
        let mut metadata = DatasetMetadata::new(DatasetId::from("ds"), "Dataset", now());
        metadata.tasks.push(TaskDefinition {
            task_id: task_id.clone(),
            name: "Boxes".to_string(),
            annotation_type: AnnotationType::BoundingBox,
            class_ids: vec![ClassId::from("person")],
            instructions: TutorialContent {
                title: "Boxes".to_string(),
                example_text: "Draw boxes".to_string(),
                example_images: Vec::new(),
            },
            skeleton: None,
            review: ReviewConfig {
                required_reviews: 1,
                workflow: ReviewWorkflow::None,
                allow_reviewer_corrections: false,
                agreement_threshold: None,
            },
            prelabel_config_ids: Vec::new(),
            manual_box_guide_migration: None,
            enabled: true,
        });
        repository.initialize(metadata).await.unwrap();
        let image_id = ImageId::from("img_1");
        repository
            .save_images_index(&ImagesIndex {
                schema_version: SCHEMA_VERSION,
                image_count: 1,
                images_by_hash: BTreeMap::from([(
                    "hash".to_string(),
                    ImageRecord {
                        image_id: image_id.clone(),
                        blake3: "hash".to_string(),
                        canonical_path: "images/one.png".to_string(),
                        known_paths: vec!["images/one.png".to_string()],
                        duplicate_paths: Vec::new(),
                        file_name: "one.png".to_string(),
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
        repository
            .append_payload(
                &image_id,
                &Actor {
                    user_id: UserId::from("admin"),
                    role: DatasetRole::DataAdmin,
                },
                EventPayload::ImportInitialized {
                    import_id: ImportId::from("imp_1"),
                    annotations: Vec::new(),
                    task_initializations: vec![ImportTaskInitialization {
                        task_id: task_id.clone(),
                        coverage: ImportCoverage::Excluded,
                        initial_state: TaskState::new(task_id.clone(), now()),
                    }],
                    migration_target_sets: Vec::new(),
                },
            )
            .await
            .unwrap();

        let stats = repository.dataset_stats().await.unwrap();
        assert_eq!(stats.import_coverage.excluded, 1);
        assert_eq!(stats.pending_tasks, 0);
        assert_eq!(stats.per_task[&task_id].pending, 0);
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
                    source_memberships: None,
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
            source_memberships: None,
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
