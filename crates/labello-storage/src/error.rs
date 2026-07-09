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

    #[error("image error at {path:?}: {source}")]
    Image {
        path: PathBuf,
        source: image::ImageError,
    },

    #[error("domain error: {0}")]
    Domain(#[from] labello_domain::DomainError),

    #[error("required file does not exist: {0:?}")]
    NotFound(PathBuf),

    #[error("path is outside dataset root: {0:?}")]
    OutsideDatasetRoot(PathBuf),

    #[error("unauthorized: {0}")]
    Unauthorized(String),
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
