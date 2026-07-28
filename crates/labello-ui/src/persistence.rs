use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    future::Future,
    pin::Pin,
    rc::Rc,
};

use labello_domain::{
    AnnotationGeometry, AnnotationId, AnnotationVersion, Assignment, AssignmentId, AssignmentKind,
    AssignmentStatus, DatasetId, DatasetMetadata, ImageId, ImageState, TaskId, Timestamp, UserId,
};
use serde::{Deserialize, Serialize};
use web_time::{Duration, Instant};

const PREFERENCE_VERSION: u32 = 2;
const DRAFT_VERSION: u32 = 2;
const LOCAL_PREFIX: &str = "labello:workspace:v2";
#[cfg(target_arch = "wasm32")]
const DATABASE_NAME: &str = "labello-workspace-v2";
#[cfg(target_arch = "wasm32")]
const DRAFT_STORE: &str = "drafts";
pub(crate) const DRAFT_TTL_SECONDS: i64 = 7 * 24 * 60 * 60;
const MAX_DRAFT_BYTES: usize = 8 * 1024 * 1024;
const MAX_ADMIN_DRAFT_BYTES: usize = 256 * 1024;
const STORAGE_RETRY_BASE: Duration = Duration::from_millis(100);
const STORAGE_RETRY_MAX: Duration = Duration::from_secs(5);

pub(crate) type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + 'a>>;

include!("persistence/identity.rs");
include!("persistence/records.rs");
include!("persistence/queue.rs");
include!("persistence/memory.rs");
include!("persistence/restore.rs");
include!("persistence/retry.rs");
include!("persistence/mapping.rs");
include!("persistence/indexed_db.rs");
include!("persistence/local_storage.rs");

#[cfg(test)]
mod tests {
    use super::*;
    use labello_domain::{ClassId, LabelClass};

    fn assignment() -> Assignment {
        let now = labello_domain::now();
        Assignment {
            assignment_id: AssignmentId::from("assignment-a"),
            image_id: ImageId::from("image-a"),
            task_id: TaskId::from("task-a"),
            assigned_to: UserId::from("user-a"),
            kind: AssignmentKind::Annotation,
            status: AssignmentStatus::Active,
            expires_at: Some(now + chrono::Duration::minutes(30)),
            created_at: now,
            updated_at: now,
        }
    }

    fn identity() -> StorageIdentity {
        StorageIdentity::new("HTTPS://Example.COM:443/api/", UserId::from("user-a")).unwrap()
    }

    fn work_draft() -> WorkDraft {
        WorkDraft::new(
            &identity(),
            DatasetId::from("data-a"),
            &assignment(),
            7,
            3,
            WorkDraftPayload::Annotation(AnnotationDraft {
                annotations: Vec::new(),
                accepted_prelabels: Vec::new(),
                selected_annotation: None,
                active_skeleton: None,
                skeleton_keypoint_index: 0,
                next_keypoint_hidden: false,
            }),
        )
    }

    #[test]
    fn normalizes_server_and_namespaces_every_identity_dimension() {
        assert_eq!(identity().server, "https://example.com/api");
        let first = assignment();
        let mut second = first.clone();
        second.assignment_id = AssignmentId::from("assignment-b");
        assert_ne!(
            work_draft_key(&identity(), &DatasetId::from("data-a"), &first),
            work_draft_key(&identity(), &DatasetId::from("data-a"), &second)
        );
        assert_ne!(
            work_draft_key(&identity(), &DatasetId::from("data-a"), &first),
            work_draft_key(&identity(), &DatasetId::from("data-b"), &first)
        );
        let other_user =
            StorageIdentity::new("https://example.com/api", UserId::from("user-b")).unwrap();
        assert_ne!(
            work_draft_key(&identity(), &DatasetId::from("data-a"), &first),
            work_draft_key(&other_user, &DatasetId::from("data-a"), &first)
        );
    }

    #[test]
    fn canvas_preferences_clamp_non_finite_and_extreme_values() {
        assert_eq!(
            StoredCanvasTransform {
                zoom: f32::NAN,
                pan_x: f32::INFINITY,
                pan_y: -200_000.0,
            }
            .clamped(),
            StoredCanvasTransform {
                zoom: 1.0,
                pan_x: 0.0,
                pan_y: -100_000.0,
            }
        );
    }

    #[test]
    fn validates_exact_assignment_sequence_and_expiration() {
        let draft = work_draft();
        let assignment = assignment();
        let mut state = ImageState::new(assignment.image_id.clone());
        state.current_sequence = 7;
        assert_eq!(
            validate_work_draft(
                &draft,
                &identity(),
                &DatasetId::from("data-a"),
                &assignment,
                &state,
                labello_domain::now(),
            ),
            DraftValidation::Valid
        );
        state.current_sequence = 8;
        assert!(matches!(
            validate_work_draft(
                &draft,
                &identity(),
                &DatasetId::from("data-a"),
                &assignment,
                &state,
                labello_domain::now(),
            ),
            DraftValidation::Conflict(_)
        ));
        assert!(matches!(
            validate_work_draft(
                &draft,
                &identity(),
                &DatasetId::from("data-a"),
                &assignment,
                &state,
                assignment.expires_at.unwrap() + chrono::Duration::seconds(1),
            ),
            DraftValidation::Expired(_)
        ));
    }

    #[test]
    fn memory_store_is_async_bounded_isolated_and_garbage_collected() {
        let store = MemoryDraftStore::default();
        let mut draft = work_draft();
        poll(store.put(DraftRecord::Work(Box::new(draft.clone())))).unwrap();
        assert_eq!(
            poll(store.get(&draft.key)).unwrap(),
            Some(DraftRecord::Work(Box::new(draft.clone())))
        );
        draft.updated_at = labello_domain::now() - chrono::Duration::seconds(DRAFT_TTL_SECONDS + 1);
        poll(store.put(DraftRecord::Work(Box::new(draft.clone())))).unwrap();
        assert_eq!(
            poll(store.garbage_collect(labello_domain::now())).unwrap(),
            1
        );
        assert_eq!(poll(store.get(&draft.key)).unwrap(), None);

        let huge = AdminDraft::new(
            &identity(),
            DatasetId::from("data-a"),
            &metadata("baseline"),
            &metadata(&"x".repeat(MAX_ADMIN_DRAFT_BYTES)),
        );
        assert!(poll(store.put(DraftRecord::Admin(Box::new(huge)))).is_err());
    }

    #[test]
    fn memory_store_surfaces_failures() {
        let store = MemoryDraftStore::default();
        store.fail_with("quota denied");
        assert_eq!(
            poll(store.get("key")).unwrap_err(),
            "quota denied".to_string()
        );
    }

    #[test]
    fn failed_put_retries_the_unchanged_record_and_advances_marker_only_on_success() {
        let store = Rc::new(MemoryDraftStore::default());
        store.fail_next(1, "quota denied");
        let mut app = crate::app::LabelloApp::default();
        app.runtime.persistence.identity = Some(identity());
        app.runtime.persistence.store = store.clone();
        let record = DraftRecord::Work(Box::new(work_draft()));
        let expected = match &record {
            DraftRecord::Work(draft) => (**draft).clone(),
            DraftRecord::Admin(_) => unreachable!(),
        };
        app.runtime.persistence.desired_work_draft = match &record {
            DraftRecord::Work(draft) => Some((**draft).clone()),
            DraftRecord::Admin(_) => None,
        };
        app.queue_persistence(PersistenceCommand::Save(Box::new(record.clone())));

        let command = app.runtime.persistence.commands.pop_front().unwrap();
        let completion = poll(execute_persistence_command(store.clone(), command));
        app.handle_persistence_completion(completion);

        assert!(app.runtime.storage_error.is_some());
        assert!(app.runtime.persistence.last_work_draft.is_none());
        let retry = app.runtime.persistence.commands.front().unwrap();
        assert_eq!(retry.attempt, 1);
        assert!(matches!(
            &retry.command,
            PersistenceCommand::Save(queued) if queued.as_ref() == &record
        ));

        let mut retry = app.runtime.persistence.commands.pop_front().unwrap();
        retry.ready_at = Instant::now();
        let completion = poll(execute_persistence_command(store.clone(), retry));
        app.handle_persistence_completion(completion);
        assert_eq!(app.runtime.persistence.last_work_draft, Some(expected));
        assert_eq!(poll(store.get(record.key())).unwrap(), Some(record));
        assert_eq!(retry_delay(u8::MAX), STORAGE_RETRY_MAX);
    }

    #[test]
    fn edit_during_save_keeps_the_new_generation_queued_and_rebases_only_the_saved_one() {
        let store = Rc::new(MemoryDraftStore::default());
        let mut app = crate::app::LabelloApp::default();
        app.runtime.persistence.identity = Some(identity());
        app.runtime.persistence.store = store.clone();
        let saved = work_draft();
        let mut newer = saved.clone();
        newer.edit_generation += 1;
        if let WorkDraftPayload::Annotation(payload) = &mut newer.payload {
            payload.accepted_prelabels.push("later-edit".to_string());
        }

        app.runtime.persistence.desired_work_draft = Some(saved.clone());
        app.queue_persistence(PersistenceCommand::Save(Box::new(DraftRecord::Work(
            Box::new(saved.clone()),
        ))));
        let in_flight = app.runtime.persistence.commands.pop_front().unwrap();
        app.runtime.persistence.desired_work_draft = Some(newer.clone());
        app.queue_persistence(PersistenceCommand::Save(Box::new(DraftRecord::Work(
            Box::new(newer.clone()),
        ))));

        let completion = poll(execute_persistence_command(store.clone(), in_flight));
        app.handle_persistence_completion(completion);
        assert_eq!(app.runtime.persistence.last_work_draft, Some(saved.clone()));
        app.rebase_work_draft_after_save(saved.edit_generation);
        assert!(app.runtime.persistence.last_work_draft.is_none());
        assert_eq!(
            app.runtime.persistence.desired_work_draft,
            Some(newer.clone())
        );
        assert!(app.runtime.persistence.commands.iter().any(|queued| matches!(
            &queued.command,
            PersistenceCommand::Save(record)
                if matches!(record.as_ref(), DraftRecord::Work(draft) if draft.as_ref() == &newer)
        )));

        let mut rebased = newer;
        rebased.base_event_sequence += 1;
        app.runtime.persistence.desired_work_draft = Some(rebased.clone());
        app.queue_persistence(PersistenceCommand::Save(Box::new(DraftRecord::Work(
            Box::new(rebased.clone()),
        ))));
        let restarted = app.runtime.persistence.commands.pop_front().unwrap();
        let completion = poll(execute_persistence_command(store.clone(), restarted));
        app.handle_persistence_completion(completion);
        assert_eq!(
            poll(store.get(&rebased.key)).unwrap(),
            Some(DraftRecord::Work(Box::new(rebased)))
        );
    }

    #[test]
    fn completion_identity_requires_the_exact_namespace_not_a_prefix_collision() {
        let current = StorageIdentity::new("https://example.test", UserId::from("user")).unwrap();
        let colliding =
            StorageIdentity::new("https://example.test", UserId::from("user2")).unwrap();
        let mut assignment = assignment();
        assignment.assigned_to = colliding.user_id.clone();
        let key = work_draft_key(&colliding, &DatasetId::from("data-a"), &assignment);
        assert!(key.starts_with(&current.prefix()));
        assert!(!current.owns_key(&key));
        assert!(colliding.owns_key(&key));

        let mut app = crate::app::LabelloApp::default();
        app.runtime.persistence.identity = Some(current);
        app.handle_persistence_completion(PersistenceCompletion::Saved {
            command: QueuedPersistenceCommand {
                identity: colliding,
                command: PersistenceCommand::Save(Box::new(DraftRecord::Work(Box::new(
                    work_draft(),
                )))),
                attempt: 0,
                ready_at: Instant::now(),
            },
            result: Ok(()),
        });
        assert!(app.runtime.persistence.last_work_draft.is_none());
    }

    #[test]
    fn admin_drafts_exclude_the_image_index() {
        let baseline = metadata("baseline");
        let mut config = baseline.clone();
        config.images.insert(
            ImageId::from("image-a"),
            labello_domain::ImageRecord {
                image_id: ImageId::from("image-a"),
                blake3: "hash".to_string(),
                canonical_path: "x".repeat(MAX_ADMIN_DRAFT_BYTES),
                known_paths: Vec::new(),
                duplicate_paths: Vec::new(),
                source_memberships: None,
                file_name: "image.png".to_string(),
                byte_size: 1,
                width: 1,
                height: 1,
                media_type: "image/png".to_string(),
            },
        );
        let draft = AdminDraft::new(&identity(), DatasetId::from("data-a"), &baseline, &config);
        assert!(draft.config.images.is_empty());
        assert!(DraftRecord::Admin(Box::new(draft)).validate_size().is_ok());
    }

    fn metadata(name: &str) -> DatasetMetadata {
        let mut metadata =
            DatasetMetadata::new(DatasetId::from("data-a"), name, labello_domain::now());
        metadata.image_roots.clear();
        metadata.label_classes = vec![LabelClass {
            class_id: ClassId::from("class"),
            name: "Class".to_string(),
            color: "#ffffff".to_string(),
            description: None,
        }];
        metadata
    }

    fn poll<T>(future: impl Future<Output = T>) -> T {
        use std::task::{Context, Poll, Waker};
        let mut future = Box::pin(future);
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("memory store future was unexpectedly pending"),
        }
    }
}
