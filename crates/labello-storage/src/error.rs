use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("io error at {path:?}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("json error at {path:?}: {source}")]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },

    #[error("toml decode error at {path:?}: {source}")]
    TomlDecode {
        path: PathBuf,
        source: toml::de::Error,
    },

    #[error("toml encode error at {path:?}: {source}")]
    TomlEncode {
        path: PathBuf,
        source: toml::ser::Error,
    },

    #[error("image error at {path:?}: {source}")]
    Image {
        path: PathBuf,
        source: image::ImageError,
    },

    #[error("domain error: {0}")]
    Domain(#[from] labello_domain::DomainError),

    #[error("required file does not exist: {0:?}")]
    NotFound(PathBuf),

    #[error("dataset is already initialized: {0:?}")]
    AlreadyExists(PathBuf),

    #[error("path is outside dataset root: {0:?}")]
    OutsideDatasetRoot(PathBuf),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("invalid assignment: {0}")]
    InvalidAssignment(String),

    #[error("invalid reviewer correction: {0}")]
    InvalidCorrection(String),

    #[error("assignment conflict: {0}")]
    AssignmentConflict(String),

    #[error("background storage task failed: {0}")]
    BackgroundTask(String),
}

impl StorageError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Io { .. } => "storage_io",
            Self::Json { .. } => "storage_json",
            Self::TomlDecode { .. } => "storage_toml_decode",
            Self::TomlEncode { .. } => "storage_toml_encode",
            Self::Image { .. } => "storage_image",
            Self::Domain(_) => "storage_domain",
            Self::NotFound(_) => "storage_not_found",
            Self::AlreadyExists(_) => "storage_already_exists",
            Self::OutsideDatasetRoot(_) => "storage_outside_dataset_root",
            Self::Unauthorized(_) => "storage_unauthorized",
            Self::InvalidAssignment(_) => "storage_invalid_assignment",
            Self::InvalidCorrection(_) => "storage_invalid_correction",
            Self::AssignmentConflict(_) => "storage_assignment_conflict",
            Self::BackgroundTask(_) => "storage_background_task",
        }
    }

    pub fn safe_diagnostic(&self) -> Option<String> {
        match self {
            Self::Io { source, .. } => Some(format!("{}: {source}", source.kind())),
            Self::Json { source, .. } => Some(source.to_string()),
            Self::TomlDecode { source, .. } => Some(source.message().to_string()),
            Self::TomlEncode { source, .. } => Some(source.to_string()),
            Self::Image { source, .. } => Some(source.to_string()),
            _ => None,
        }
    }
}

pub type StorageResult<T> = Result<T, StorageError>;

pub(crate) trait PathIo<T> {
    fn with_path(self, path: impl Into<PathBuf>) -> StorageResult<T>;
}

impl<T> PathIo<T> for Result<T, std::io::Error> {
    fn with_path(self, path: impl Into<PathBuf>) -> StorageResult<T> {
        self.map_err(|source| StorageError::Io {
            path: path.into(),
            source,
        })
    }
}

pub(crate) trait PathJson<T> {
    fn with_json_path(self, path: impl Into<PathBuf>) -> StorageResult<T>;
}

impl<T> PathJson<T> for Result<T, serde_json::Error> {
    fn with_json_path(self, path: impl Into<PathBuf>) -> StorageResult<T> {
        self.map_err(|source| StorageError::Json {
            path: path.into(),
            source,
        })
    }
}

pub(crate) trait PathTomlDecode<T> {
    fn with_toml_decode_path(self, path: impl Into<PathBuf>) -> StorageResult<T>;
}

impl<T> PathTomlDecode<T> for Result<T, toml::de::Error> {
    fn with_toml_decode_path(self, path: impl Into<PathBuf>) -> StorageResult<T> {
        self.map_err(|source| StorageError::TomlDecode {
            path: path.into(),
            source,
        })
    }
}

pub(crate) trait PathTomlEncode<T> {
    fn with_toml_encode_path(self, path: impl Into<PathBuf>) -> StorageResult<T>;
}

impl<T> PathTomlEncode<T> for Result<T, toml::ser::Error> {
    fn with_toml_encode_path(self, path: impl Into<PathBuf>) -> StorageResult<T> {
        self.map_err(|source| StorageError::TomlEncode {
            path: path.into(),
            source,
        })
    }
}
