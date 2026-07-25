use serde_json::{Value, json};

use crate::*;

const V2_EVENT_LOG: &str = r#"
[
  {
    "schemaVersion": 2,
    "eventSequence": 1,
    "eventId": "evt_1",
    "imageId": "img_1",
    "type": "annotation_version_created",
    "actorUserId": "annotator",
    "actorRole": "annotator",
    "timestamp": "2026-01-02T03:04:05Z",
    "payload": {
      "kind": "annotation_version_created",
      "annotation": {
        "annotationId": "ann_1",
        "version": 1,
        "taskId": "bounding_box:person",
        "classId": "person",
        "type": "bounding_box",
        "source": { "source": "human" },
        "geometry": {
          "type": "bounding_box",
          "geometry": {
            "x": 0.10000000149011612,
            "y": 0.10000000149011612,
            "width": 0.20000000298023224,
            "height": 0.20000000298023224
          }
        },
        "authorUserId": "annotator",
        "createdAt": "2026-01-02T03:04:05Z",
        "updatedAt": "2026-01-02T03:04:05Z",
        "deleted": false
      },
      "previous_version": null,
      "reason": null
    }
  },
  {
    "schemaVersion": 2,
    "eventSequence": 2,
    "eventId": "evt_2",
    "imageId": "img_1",
    "type": "annotation_version_created",
    "actorUserId": "annotator",
    "actorRole": "annotator",
    "timestamp": "2026-01-02T03:04:05Z",
    "payload": {
      "kind": "annotation_version_created",
      "annotation": {
        "annotationId": "ann_1",
        "version": 2,
        "taskId": "bounding_box:person",
        "classId": "person",
        "type": "bounding_box",
        "source": { "source": "human" },
        "geometry": {
          "type": "bounding_box",
          "geometry": {
            "x": 0.20000000298023224,
            "y": 0.20000000298023224,
            "width": 0.30000001192092896,
            "height": 0.30000001192092896
          }
        },
        "authorUserId": "annotator",
        "createdAt": "2026-01-02T03:04:05Z",
        "updatedAt": "2026-01-02T03:04:05Z",
        "deleted": false
      },
      "previous_version": 1,
      "reason": "move"
    }
  },
  {
    "schemaVersion": 2,
    "eventSequence": 3,
    "eventId": "evt_3",
    "imageId": "img_1",
    "type": "annotation_version_created",
    "actorUserId": "annotator",
    "actorRole": "annotator",
    "timestamp": "2026-01-02T03:04:05Z",
    "payload": {
      "kind": "annotation_version_created",
      "annotation": {
        "annotationId": "ann_deleted",
        "version": 1,
        "taskId": "bounding_box:person",
        "classId": "person",
        "type": "bounding_box",
        "source": { "source": "human" },
        "geometry": {
          "type": "bounding_box",
          "geometry": {
            "x": 0.6000000238418579,
            "y": 0.6000000238418579,
            "width": 0.10000000149011612,
            "height": 0.10000000149011612
          }
        },
        "authorUserId": "annotator",
        "createdAt": "2026-01-02T03:04:05Z",
        "updatedAt": "2026-01-02T03:04:05Z",
        "deleted": false
      },
      "previous_version": null,
      "reason": null
    }
  },
  {
    "schemaVersion": 2,
    "eventSequence": 4,
    "eventId": "evt_4",
    "imageId": "img_1",
    "type": "annotation_deleted",
    "actorUserId": "annotator",
    "actorRole": "annotator",
    "timestamp": "2026-01-02T03:04:05Z",
    "payload": {
      "kind": "annotation_deleted",
      "annotation_id": "ann_deleted",
      "version": 1,
      "reason": "duplicate"
    }
  },
  {
    "schemaVersion": 2,
    "eventSequence": 5,
    "eventId": "evt_5",
    "imageId": "img_1",
    "type": "task_state_changed",
    "actorUserId": "annotator",
    "actorRole": "annotator",
    "timestamp": "2026-01-02T03:04:05Z",
    "payload": {
      "kind": "task_state_changed",
      "task_state": {
        "taskId": "bounding_box:person",
        "status": "submitted",
        "outcome": "annotation_completed",
        "assignedTo": "annotator",
        "completedBy": "annotator",
        "completedAt": "2026-01-02T03:04:05Z",
        "updatedAt": "2026-01-02T03:04:05Z"
      }
    }
  },
  {
    "schemaVersion": 2,
    "eventSequence": 6,
    "eventId": "evt_6",
    "imageId": "img_1",
    "type": "assignment_updated",
    "actorUserId": "reviewer",
    "actorRole": "reviewer",
    "timestamp": "2026-01-02T03:04:05Z",
    "payload": {
      "kind": "assignment_updated",
      "assignment": {
        "assignmentId": "asg_1",
        "imageId": "img_1",
        "taskId": "bounding_box:person",
        "assignedTo": "reviewer",
        "kind": "review",
        "status": "active",
        "expiresAt": "2026-01-02T04:04:05Z",
        "createdAt": "2026-01-02T03:04:05Z",
        "updatedAt": "2026-01-02T03:04:05Z"
      }
    }
  },
  {
    "schemaVersion": 2,
    "eventSequence": 7,
    "eventId": "evt_7",
    "imageId": "img_1",
    "type": "review_recorded",
    "actorUserId": "reviewer",
    "actorRole": "reviewer",
    "timestamp": "2026-01-02T03:04:05Z",
    "payload": {
      "kind": "review_recorded",
      "review": {
        "reviewId": "rev_object",
        "target": {
          "targetType": "annotation_version",
          "annotation_id": "ann_1",
          "version": 2
        },
        "reviewerUserId": "reviewer",
        "decision": "approved",
        "timestamp": "2026-01-02T03:04:05Z",
        "comment": "object approved"
      }
    }
  },
  {
    "schemaVersion": 2,
    "eventSequence": 8,
    "eventId": "evt_8",
    "imageId": "img_1",
    "type": "review_recorded",
    "actorUserId": "reviewer",
    "actorRole": "reviewer",
    "timestamp": "2026-01-02T03:04:05Z",
    "payload": {
      "kind": "review_recorded",
      "review": {
        "reviewId": "rev_task",
        "target": {
          "targetType": "task",
          "task_id": "bounding_box:person"
        },
        "reviewerUserId": "reviewer",
        "decision": "approved",
        "timestamp": "2026-01-02T03:04:05Z",
        "comment": null
      }
    }
  },
  {
    "schemaVersion": 2,
    "eventSequence": 9,
    "eventId": "evt_9",
    "imageId": "img_1",
    "type": "reviewer_correction_recorded",
    "actorUserId": "reviewer",
    "actorRole": "reviewer",
    "timestamp": "2026-01-02T03:04:05Z",
    "payload": {
      "kind": "reviewer_correction_recorded",
      "correction": {
        "correctionId": "cor_1",
        "assignmentId": "asg_1",
        "annotationId": "ann_1",
        "previousVersion": 2,
        "correctedVersion": 3,
        "taskId": "bounding_box:person",
        "reviewerUserId": "reviewer",
        "timestamp": "2026-01-02T03:04:05Z",
        "reason": "tighten box"
      },
      "annotation": {
        "annotationId": "ann_1",
        "version": 3,
        "taskId": "bounding_box:person",
        "classId": "person",
        "type": "bounding_box",
        "source": { "source": "reviewer_correction", "correction_id": "cor_1" },
        "geometry": {
          "type": "bounding_box",
          "geometry": {
            "x": 0.25,
            "y": 0.25,
            "width": 0.20000000298023224,
            "height": 0.20000000298023224
          }
        },
        "authorUserId": "reviewer",
        "createdAt": "2026-01-02T03:04:05Z",
        "updatedAt": "2026-01-02T03:04:05Z",
        "deleted": false
      },
      "review": {
        "reviewId": "rev_correction",
        "target": {
          "targetType": "annotation_version",
          "annotation_id": "ann_1",
          "version": 2
        },
        "reviewerUserId": "reviewer",
        "decision": "rejected",
        "timestamp": "2026-01-02T03:04:05Z",
        "comment": "tighten box"
      },
      "task_state": {
        "taskId": "bounding_box:person",
        "status": "completed",
        "outcome": "reviewer_corrected",
        "assignedTo": null,
        "completedBy": "reviewer",
        "completedAt": "2026-01-02T03:04:05Z",
        "updatedAt": "2026-01-02T03:04:05Z"
      },
      "assignments": [
        {
          "assignmentId": "asg_1",
          "imageId": "img_1",
          "taskId": "bounding_box:person",
          "assignedTo": "reviewer",
          "kind": "review",
          "status": "completed",
          "expiresAt": "2026-01-02T04:04:05Z",
          "createdAt": "2026-01-02T03:04:05Z",
          "updatedAt": "2026-01-02T03:04:05Z"
        }
      ]
    }
  },
  {
    "schemaVersion": 2,
    "eventSequence": 10,
    "eventId": "evt_10",
    "imageId": "img_1",
    "type": "adjudication_recorded",
    "actorUserId": "adjudicator",
    "actorRole": "adjudicator",
    "timestamp": "2026-01-02T03:04:05Z",
    "payload": {
      "kind": "adjudication_recorded",
      "adjudication": {
        "adjudicationId": "adj_1",
        "taskId": "bounding_box:person",
        "annotationIds": ["ann_1"],
        "adjudicatorUserId": "adjudicator",
        "decision": "accept_annotation",
        "resolution": "use corrected box",
        "timestamp": "2026-01-02T03:04:05Z"
      }
    }
  }
]
"#;

#[test]
fn v2_event_names_shapes_and_replay_match_the_golden_log() {
    let golden: Value = serde_json::from_str(V2_EVENT_LOG).unwrap();
    let events: Vec<EventLogEntry> = serde_json::from_value(golden.clone()).unwrap();

    assert_eq!(serde_json::to_value(&events).unwrap(), golden);
    assert_eq!(events.len(), 10);
    for event in &events {
        assert_eq!(event.payload.event_type(), event.event_type);
        event.validate_shape().unwrap();

        let mut mismatched = event.clone();
        mismatched.event_type = match event.event_type {
            EventType::AnnotationVersionCreated => EventType::AnnotationDeleted,
            _ => EventType::AnnotationVersionCreated,
        };
        assert!(mismatched.validate_shape().is_err());
    }

    let state = rebuild_state(ImageId::from("img_1"), &events).unwrap();
    assert_eq!(state.schema_version, 2);
    assert_eq!(state.current_sequence, 10);
    assert_eq!(
        state
            .current_annotation(&AnnotationId::from("ann_1"))
            .unwrap()
            .version,
        3
    );
    assert!(
        state
            .current_annotation(&AnnotationId::from("ann_deleted"))
            .unwrap()
            .deleted
    );
    assert_eq!(state.reviews.len(), 3);
    assert_eq!(state.reviewer_corrections.len(), 1);
    assert_eq!(state.adjudications.len(), 1);
    assert_eq!(state.assignments[0].status, AssignmentStatus::Completed);
    assert_eq!(
        state.task_states[&TaskId::from("bounding_box:person")].outcome,
        Some(TaskOutcome::ReviewerCorrected)
    );

    let event_names = events
        .iter()
        .map(|event| event.event_type.to_string())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        event_names,
        [
            "adjudication_recorded",
            "annotation_deleted",
            "annotation_version_created",
            "assignment_updated",
            "review_recorded",
            "reviewer_correction_recorded",
            "task_state_changed",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    );
}

#[test]
fn task_workflow_and_review_target_v2_json_is_stable() {
    let timestamp: Timestamp = "2026-01-02T03:04:05Z".parse().unwrap();
    let state = TaskState {
        task_id: TaskId::from("bounding_box:person"),
        status: TaskStatus::NeedsCorrection,
        outcome: Some(TaskOutcome::ReviewerCorrected),
        assigned_to: Some(UserId::from("annotator")),
        completed_by: Some(UserId::from("reviewer")),
        completed_at: Some(timestamp),
        updated_at: timestamp,
    };
    assert_eq!(
        serde_json::to_value(state).unwrap(),
        json!({
            "taskId": "bounding_box:person",
            "status": "needs_correction",
            "outcome": "reviewer_corrected",
            "assignedTo": "annotator",
            "completedBy": "reviewer",
            "completedAt": "2026-01-02T03:04:05Z",
            "updatedAt": "2026-01-02T03:04:05Z"
        })
    );
    assert_eq!(
        [
            TaskStatus::Pending,
            TaskStatus::InProgress,
            TaskStatus::Submitted,
            TaskStatus::Completed,
            TaskStatus::NeedsCorrection,
            TaskStatus::AdjudicationRequired,
        ]
        .map(|status| serde_json::to_value(status).unwrap()),
        [
            json!("pending"),
            json!("in_progress"),
            json!("submitted"),
            json!("completed"),
            json!("needs_correction"),
            json!("adjudication_required"),
        ]
    );
    assert_eq!(
        [
            TaskOutcome::AnnotationCompleted,
            TaskOutcome::Approved,
            TaskOutcome::ReviewerCorrected,
            TaskOutcome::Adjudicated,
        ]
        .map(|outcome| serde_json::to_value(outcome).unwrap()),
        [
            json!("annotation_completed"),
            json!("approved"),
            json!("reviewer_corrected"),
            json!("adjudicated"),
        ]
    );
    assert_eq!(
        [
            ReviewTarget::AnnotationVersion {
                annotation_id: AnnotationId::from("ann_1"),
                version: 2,
            },
            ReviewTarget::Task {
                task_id: TaskId::from("task_1"),
            },
            ReviewTarget::Image {
                image_id: ImageId::from("img_1"),
            },
        ]
        .map(|target| serde_json::to_value(target).unwrap()),
        [
            json!({
                "targetType": "annotation_version",
                "annotation_id": "ann_1",
                "version": 2
            }),
            json!({ "targetType": "task", "task_id": "task_1" }),
            json!({ "targetType": "image", "image_id": "img_1" }),
        ]
    );

    let new_state = TaskState::new(TaskId::from("task_1"), timestamp);
    assert_eq!(new_state.status, TaskStatus::Pending);
    assert_eq!(new_state.outcome, None);
    assert_eq!(new_state.assigned_to, None);
    assert_eq!(new_state.completed_by, None);
    assert_eq!(new_state.completed_at, None);

    let _: crate::annotation::TaskState = new_state;
    let _: crate::annotation::TaskStatus = TaskStatus::Pending;
    let _: crate::annotation::TaskOutcome = TaskOutcome::Approved;
}

#[test]
fn v2_schema_version_fields_are_present_at_persistence_boundaries() {
    let timestamp: Timestamp = "2026-01-02T03:04:05Z".parse().unwrap();
    let metadata = DatasetMetadata::new(DatasetId::from("ds_1"), "Dataset", timestamp);
    let config = DatasetConfig::from_metadata(&metadata);
    let index = ImagesIndex::default();
    let state = ImageState::new(ImageId::from("img_1"));
    let keybindings = KeybindingSet::defaults_for(UserId::from("user_1"));
    let snapshot = DatasetSnapshot {
        schema_version: SCHEMA_VERSION,
        snapshot_id: "snapshot_1".to_string(),
        dataset_id: DatasetId::from("ds_1"),
        created_at: timestamp,
        includes_image_bytes: false,
        total_bytes: 0,
        files: Vec::new(),
    };
    let offline_bundle = OfflineBundle {
        schema_version: SCHEMA_VERSION,
        dataset_id: DatasetId::from("ds_1"),
        user_id: UserId::from("user_1"),
        created_at: timestamp,
        expires_at: None,
        roles: vec![DatasetRole::Annotator],
        tasks: Vec::new(),
        images: Vec::new(),
    };
    let offline_sync =
        OfflineSyncRequest::new(DatasetId::from("ds_1"), UserId::from("user_1"), Vec::new());

    for value in [
        serde_json::to_value(config).unwrap(),
        serde_json::to_value(index).unwrap(),
        serde_json::to_value(state).unwrap(),
        serde_json::to_value(keybindings).unwrap(),
        serde_json::to_value(snapshot).unwrap(),
        serde_json::to_value(offline_bundle).unwrap(),
        serde_json::to_value(offline_sync).unwrap(),
    ] {
        assert_eq!(value.get("schemaVersion"), Some(&json!(2)));
        assert!(value.get("schema_version").is_none());
    }
}
