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

mod validation;
mod wire;

pub(crate) use wire::transform_annotation;
pub use wire::{
    AnnotationVersionV2WireSchema, EventLogEntryV2WireSchema, EventLogEntryV3WireSchema,
    EventPayloadV2WireSchema, EventTypeV2WireSchema,
};
