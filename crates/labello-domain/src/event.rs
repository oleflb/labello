use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    AdjudicationRecord, AnnotationId, AnnotationVersion, Assignment, DatasetRole, EventId, ImageId,
    ReviewRecord, SCHEMA_VERSION, TaskId, TaskState, Timestamp, UserId,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    AnnotationVersionCreated,
    AnnotationDeleted,
    TaskStateChanged,
    ReviewRecorded,
    AdjudicationRecorded,
    AssignmentUpdated,
}

impl std::fmt::Display for EventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::AnnotationVersionCreated => "annotation_version_created",
            Self::AnnotationDeleted => "annotation_deleted",
            Self::TaskStateChanged => "task_state_changed",
            Self::ReviewRecorded => "review_recorded",
            Self::AdjudicationRecorded => "adjudication_recorded",
            Self::AssignmentUpdated => "assignment_updated",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventPayload {
    AnnotationVersionCreated {
        annotation: AnnotationVersion,
        previous_version: Option<u32>,
        reason: Option<String>,
    },
    AnnotationDeleted {
        annotation_id: AnnotationId,
        version: u32,
        reason: Option<String>,
    },
    TaskStateChanged {
        task_state: TaskState,
    },
    ReviewRecorded {
        review: ReviewRecord,
    },
    AdjudicationRecorded {
        adjudication: AdjudicationRecord,
    },
    AssignmentUpdated {
        assignment: Assignment,
    },
}

impl EventPayload {
    pub fn event_type(&self) -> EventType {
        match self {
            Self::AnnotationVersionCreated { .. } => EventType::AnnotationVersionCreated,
            Self::AnnotationDeleted { .. } => EventType::AnnotationDeleted,
            Self::TaskStateChanged { .. } => EventType::TaskStateChanged,
            Self::ReviewRecorded { .. } => EventType::ReviewRecorded,
            Self::AdjudicationRecorded { .. } => EventType::AdjudicationRecorded,
            Self::AssignmentUpdated { .. } => EventType::AssignmentUpdated,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EventLogEntry {
    pub schema_version: u32,
    pub event_sequence: u64,
    pub event_id: EventId,
    pub image_id: ImageId,
    #[serde(rename = "type")]
    pub event_type: EventType,
    pub actor_user_id: UserId,
    pub actor_role: DatasetRole,
    pub timestamp: Timestamp,
    pub payload: EventPayload,
}

impl EventLogEntry {
    pub fn new(
        event_sequence: u64,
        image_id: ImageId,
        actor_user_id: UserId,
        actor_role: DatasetRole,
        timestamp: Timestamp,
        payload: EventPayload,
    ) -> Self {
        let event_type = payload.event_type();
        Self {
            schema_version: SCHEMA_VERSION,
            event_sequence,
            event_id: EventId::generate(),
            image_id,
            event_type,
            actor_user_id,
            actor_role,
            timestamp,
            payload,
        }
    }

    pub fn validate_shape(&self) -> crate::DomainResult<()> {
        let actual = self.payload.event_type();
        if actual == self.event_type {
            Ok(())
        } else {
            Err(crate::DomainError::EventPayloadMismatch(
                self.event_type.to_string(),
            ))
        }
    }

    pub fn task_id(&self) -> Option<&TaskId> {
        match &self.payload {
            EventPayload::AnnotationVersionCreated { annotation, .. } => Some(&annotation.task_id),
            EventPayload::TaskStateChanged { task_state } => Some(&task_state.task_id),
            EventPayload::AssignmentUpdated { assignment } => Some(&assignment.task_id),
            EventPayload::AdjudicationRecorded { adjudication } => Some(&adjudication.task_id),
            EventPayload::AnnotationDeleted { .. } | EventPayload::ReviewRecorded { .. } => None,
        }
    }
}
