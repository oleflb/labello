use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum DomainError {
    #[error("unsupported schema version {found}; supported version is {supported}")]
    UnsupportedSchemaVersion { found: u32, supported: u32 },

    #[error("invalid geometry: {0}")]
    InvalidGeometry(String),

    #[error("annotation type {annotation_type} is not allowed for task {task_id}")]
    AnnotationTypeMismatch {
        task_id: String,
        annotation_type: String,
    },

    #[error("class {class_id} is not allowed for task {task_id}")]
    ClassNotAllowed { task_id: String, class_id: String },

    #[error("event sequence {found} is invalid; expected {expected}")]
    InvalidEventSequence { expected: u64, found: u64 },

    #[error("event image {found} does not match state image {expected}")]
    ImageMismatch { expected: String, found: String },

    #[error("event payload does not match event type {0}")]
    EventPayloadMismatch(String),

    #[error("annotation {0} does not exist")]
    MissingAnnotation(String),

    #[error("annotation {annotation_id} version {version} does not exist")]
    MissingAnnotationVersion { annotation_id: String, version: u32 },

    #[error("reviewer correction {0} is internally inconsistent")]
    InvalidReviewerCorrection(String),

    #[error("reviewer correction {0} already exists")]
    DuplicateReviewerCorrection(String),

    #[error("task {0} does not exist")]
    MissingTask(String),

    #[error("user {user_id} lacks dataset role {role}")]
    MissingRole { user_id: String, role: String },

    #[error("keybinding conflict for {chord}: {actions:?}")]
    KeybindingConflict { chord: String, actions: Vec<String> },

    #[error("offline sync conflict for image {image_id}: {reason}")]
    SyncConflict { image_id: String, reason: String },
}

pub type DomainResult<T> = Result<T, DomainError>;
