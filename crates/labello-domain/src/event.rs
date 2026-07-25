use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

use crate::{
    AdjudicationRecord, AnnotationId, AnnotationVersion, Assignment, DatasetId, DatasetRole,
    EventId, ImageId, ImportCoverage, ImportId, MigrationConfirmation, MigrationDependencyMarker,
    MigrationDisposition, MigrationHash, MigrationPass, MigrationPassItem, MigrationTarget,
    ObjectGroupId, ReviewRecord, ReviewerCorrectionRecord, SCHEMA_VERSION, TaskId, TaskState,
    Timestamp, UserId,
};

pub const MAX_IMPORT_ANNOTATIONS_PER_EVENT: usize = 10_000;
pub const MAX_IMPORT_TASKS_PER_EVENT: usize = 1_000;
pub const MAX_MIGRATION_TARGETS_PER_EVENT: usize = 10_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportTaskInitialization {
    pub task_id: TaskId,
    pub coverage: ImportCoverage,
    pub initial_state: TaskState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MigrationTargetSetInitialization {
    pub dataset_id: DatasetId,
    pub guide_task_id: TaskId,
    pub target_task_id: TaskId,
    pub target_set_hash: MigrationHash,
    pub targets: Vec<MigrationTarget>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    AnnotationVersionCreated,
    AnnotationDeleted,
    TaskStateChanged,
    ReviewRecorded,
    ReviewerCorrectionRecorded,
    AdjudicationRecorded,
    AssignmentUpdated,
    ImportInitialized,
    ImportedTaskReopened,
    ImportCoverageIncluded,
    MigrationDispositionChanged,
    MigrationDispositionReopened,
    MigrationDependencyMarked,
    MigrationDependencyCleared,
    MigrationPassStarted,
    MigrationPassItemRecorded,
    MigrationFullImageConfirmed,
}

impl std::fmt::Display for EventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::AnnotationVersionCreated => "annotation_version_created",
            Self::AnnotationDeleted => "annotation_deleted",
            Self::TaskStateChanged => "task_state_changed",
            Self::ReviewRecorded => "review_recorded",
            Self::ReviewerCorrectionRecorded => "reviewer_correction_recorded",
            Self::AdjudicationRecorded => "adjudication_recorded",
            Self::AssignmentUpdated => "assignment_updated",
            Self::ImportInitialized => "import_initialized",
            Self::ImportedTaskReopened => "imported_task_reopened",
            Self::ImportCoverageIncluded => "import_coverage_included",
            Self::MigrationDispositionChanged => "migration_disposition_changed",
            Self::MigrationDispositionReopened => "migration_disposition_reopened",
            Self::MigrationDependencyMarked => "migration_dependency_marked",
            Self::MigrationDependencyCleared => "migration_dependency_cleared",
            Self::MigrationPassStarted => "migration_pass_started",
            Self::MigrationPassItemRecorded => "migration_pass_item_recorded",
            Self::MigrationFullImageConfirmed => "migration_full_image_confirmed",
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
    ReviewerCorrectionRecorded {
        correction: ReviewerCorrectionRecord,
        annotation: Box<AnnotationVersion>,
        review: ReviewRecord,
        task_state: TaskState,
        assignments: Vec<Assignment>,
    },
    AdjudicationRecorded {
        adjudication: AdjudicationRecord,
    },
    AssignmentUpdated {
        assignment: Assignment,
    },
    ImportInitialized {
        import_id: ImportId,
        annotations: Vec<AnnotationVersion>,
        task_initializations: Vec<ImportTaskInitialization>,
        migration_target_sets: Vec<MigrationTargetSetInitialization>,
    },
    ImportedTaskReopened {
        task_state: TaskState,
        reason: String,
    },
    ImportCoverageIncluded {
        task_state: TaskState,
        reason: String,
    },
    MigrationDispositionChanged {
        task_id: TaskId,
        object_group_id: ObjectGroupId,
        disposition: MigrationDisposition,
    },
    MigrationDispositionReopened {
        task_id: TaskId,
        object_group_id: ObjectGroupId,
        disposition: MigrationDisposition,
    },
    MigrationDependencyMarked {
        task_id: TaskId,
        object_group_id: ObjectGroupId,
        marker: MigrationDependencyMarker,
    },
    MigrationDependencyCleared {
        task_id: TaskId,
        object_group_id: ObjectGroupId,
        marker_version: u32,
    },
    MigrationPassStarted {
        pass: MigrationPass,
    },
    MigrationPassItemRecorded {
        pass_id: crate::MigrationPassId,
        item: MigrationPassItem,
    },
    MigrationFullImageConfirmed {
        confirmation: MigrationConfirmation,
    },
}

impl EventPayload {
    pub fn event_type(&self) -> EventType {
        match self {
            Self::AnnotationVersionCreated { .. } => EventType::AnnotationVersionCreated,
            Self::AnnotationDeleted { .. } => EventType::AnnotationDeleted,
            Self::TaskStateChanged { .. } => EventType::TaskStateChanged,
            Self::ReviewRecorded { .. } => EventType::ReviewRecorded,
            Self::ReviewerCorrectionRecorded { .. } => EventType::ReviewerCorrectionRecorded,
            Self::AdjudicationRecorded { .. } => EventType::AdjudicationRecorded,
            Self::AssignmentUpdated { .. } => EventType::AssignmentUpdated,
            Self::ImportInitialized { .. } => EventType::ImportInitialized,
            Self::ImportedTaskReopened { .. } => EventType::ImportedTaskReopened,
            Self::ImportCoverageIncluded { .. } => EventType::ImportCoverageIncluded,
            Self::MigrationDispositionChanged { .. } => EventType::MigrationDispositionChanged,
            Self::MigrationDispositionReopened { .. } => EventType::MigrationDispositionReopened,
            Self::MigrationDependencyMarked { .. } => EventType::MigrationDependencyMarked,
            Self::MigrationDependencyCleared { .. } => EventType::MigrationDependencyCleared,
            Self::MigrationPassStarted { .. } => EventType::MigrationPassStarted,
            Self::MigrationPassItemRecorded { .. } => EventType::MigrationPassItemRecorded,
            Self::MigrationFullImageConfirmed { .. } => EventType::MigrationFullImageConfirmed,
        }
    }
}

#[derive(Clone, Debug, PartialEq, JsonSchema)]
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

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventLogEntryWire {
    schema_version: u32,
    event_sequence: u64,
    event_id: EventId,
    image_id: ImageId,
    #[serde(rename = "type")]
    event_type: EventType,
    actor_user_id: UserId,
    actor_role: DatasetRole,
    timestamp: Timestamp,
    payload: EventPayload,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EventLogEntryV2WireSchema {
    pub schema_version: u32,
    pub event_sequence: u64,
    pub event_id: EventId,
    pub image_id: ImageId,
    #[serde(rename = "type")]
    pub event_type: EventTypeV2WireSchema,
    pub actor_user_id: UserId,
    pub actor_role: DatasetRole,
    pub timestamp: Timestamp,
    pub payload: EventPayloadV2WireSchema,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EventLogEntryV3WireSchema {
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

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventTypeV2WireSchema {
    AnnotationVersionCreated,
    AnnotationDeleted,
    TaskStateChanged,
    ReviewRecorded,
    ReviewerCorrectionRecorded,
    AdjudicationRecorded,
    AssignmentUpdated,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventPayloadV2WireSchema {
    AnnotationVersionCreated {
        annotation: AnnotationVersionV2WireSchema,
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
    ReviewerCorrectionRecorded {
        correction: Box<ReviewerCorrectionRecord>,
        annotation: Box<AnnotationVersionV2WireSchema>,
        review: ReviewRecord,
        task_state: TaskState,
        assignments: Vec<Assignment>,
    },
    AdjudicationRecorded {
        adjudication: AdjudicationRecord,
    },
    AssignmentUpdated {
        assignment: Assignment,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationVersionV2WireSchema {
    pub annotation_id: AnnotationId,
    pub version: u32,
    pub task_id: TaskId,
    pub class_id: crate::ClassId,
    #[serde(rename = "type")]
    pub annotation_type: crate::AnnotationType,
    pub source: crate::AnnotationSource,
    pub geometry: crate::AnnotationGeometry,
    pub author_user_id: UserId,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub deleted: bool,
}

impl From<EventLogEntryWire> for EventLogEntry {
    fn from(wire: EventLogEntryWire) -> Self {
        Self {
            schema_version: wire.schema_version,
            event_sequence: wire.event_sequence,
            event_id: wire.event_id,
            image_id: wire.image_id,
            event_type: wire.event_type,
            actor_user_id: wire.actor_user_id,
            actor_role: wire.actor_role,
            timestamp: wire.timestamp,
            payload: wire.payload,
        }
    }
}

impl From<&EventLogEntry> for EventLogEntryWire {
    fn from(entry: &EventLogEntry) -> Self {
        Self {
            schema_version: entry.schema_version,
            event_sequence: entry.event_sequence,
            event_id: entry.event_id.clone(),
            image_id: entry.image_id.clone(),
            event_type: entry.event_type.clone(),
            actor_user_id: entry.actor_user_id.clone(),
            actor_role: entry.actor_role.clone(),
            timestamp: entry.timestamp,
            payload: entry.payload.clone(),
        }
    }
}

impl Serialize for EventLogEntry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut value = serde_json::to_value(EventLogEntryWire::from(self))
            .map_err(serde::ser::Error::custom)?;
        if self.schema_version == crate::LEGACY_SCHEMA_VERSION {
            transform_event_annotations(&mut value, false).map_err(serde::ser::Error::custom)?;
        }
        value.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for EventLogEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut value = serde_json::Value::deserialize(deserializer)?;
        let schema_version = value
            .get("schemaVersion")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| D::Error::custom("event schemaVersion is missing"))?;
        if schema_version == u64::from(crate::LEGACY_SCHEMA_VERSION) {
            transform_event_annotations(&mut value, true).map_err(D::Error::custom)?;
        }
        serde_json::from_value::<EventLogEntryWire>(value)
            .map(EventLogEntry::from)
            .map_err(D::Error::custom)
    }
}

fn transform_event_annotations(
    event: &mut serde_json::Value,
    upcast: bool,
) -> Result<(), &'static str> {
    let payload = event
        .get_mut("payload")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("event payload must be an object")?;
    match payload.get("kind").and_then(serde_json::Value::as_str) {
        Some("annotation_version_created") => {
            if let Some(annotation) = payload.get_mut("annotation") {
                transform_annotation(annotation, upcast)?;
            }
        }
        Some("reviewer_correction_recorded") => {
            if let Some(annotation) = payload.get_mut("annotation") {
                transform_annotation(annotation, upcast)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn transform_annotation(
    annotation: &mut serde_json::Value,
    upcast: bool,
) -> Result<(), &'static str> {
    let object = annotation
        .as_object_mut()
        .ok_or("annotation must be an object")?;
    if upcast {
        let mut source = object
            .remove("source")
            .ok_or("v2 annotation source is missing")?;
        if source.get("source").and_then(serde_json::Value::as_str) == Some("human") {
            source
                .as_object_mut()
                .ok_or("annotation source must be an object")?
                .insert(
                    "action".to_string(),
                    serde_json::Value::String("authored".to_string()),
                );
        }
        object.insert(
            "origin".to_string(),
            serde_json::json!({ "origin": "native", "legacyV2": true }),
        );
        object.insert("objectGroupId".to_string(), serde_json::Value::Null);
        object.insert("revisionSource".to_string(), source);
    } else {
        object.remove("origin");
        object.remove("objectGroupId");
        let mut source = object
            .remove("revisionSource")
            .ok_or("v3 annotation revisionSource is missing")?;
        source
            .as_object_mut()
            .ok_or("annotation revisionSource must be an object")?
            .remove("action");
        object.insert("source".to_string(), source);
    }
    Ok(())
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
        crate::validate_supported_schema_version(self.schema_version)?;
        if self.schema_version == crate::LEGACY_SCHEMA_VERSION
            && !matches!(
                self.event_type,
                EventType::AnnotationVersionCreated
                    | EventType::AnnotationDeleted
                    | EventType::TaskStateChanged
                    | EventType::ReviewRecorded
                    | EventType::ReviewerCorrectionRecorded
                    | EventType::AdjudicationRecorded
                    | EventType::AssignmentUpdated
            )
        {
            return Err(crate::DomainError::EventPayloadMismatch(
                self.event_type.to_string(),
            ));
        }
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
            EventPayload::ReviewerCorrectionRecorded { correction, .. } => {
                Some(&correction.task_id)
            }
            EventPayload::ImportInitialized {
                task_initializations,
                ..
            } => task_initializations.first().map(|task| &task.task_id),
            EventPayload::ImportedTaskReopened { task_state, .. }
            | EventPayload::ImportCoverageIncluded { task_state, .. } => Some(&task_state.task_id),
            EventPayload::MigrationDispositionChanged { task_id, .. }
            | EventPayload::MigrationDispositionReopened { task_id, .. }
            | EventPayload::MigrationDependencyMarked { task_id, .. }
            | EventPayload::MigrationDependencyCleared { task_id, .. } => Some(task_id),
            EventPayload::MigrationPassStarted { pass } => Some(&pass.task_id),
            EventPayload::MigrationFullImageConfirmed { confirmation } => {
                Some(&confirmation.task_id)
            }
            EventPayload::AnnotationDeleted { .. }
            | EventPayload::ReviewRecorded { .. }
            | EventPayload::MigrationPassItemRecorded { .. } => None,
        }
    }
}
