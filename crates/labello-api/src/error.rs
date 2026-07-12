use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid id: {0}")]
    InvalidId(#[from] labello_domain::IdValidationError),

    #[error("storage error: {0}")]
    Storage(#[from] labello_storage::StorageError),

    #[error("http client error: {0}")]
    Http(#[from] reqwest::Error),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorBody {
    error: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::InvalidId(_) => StatusCode::BAD_REQUEST,
            ApiError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            ApiError::NotFound(_) => StatusCode::NOT_FOUND,
            ApiError::Storage(labello_storage::StorageError::NotFound(_)) => StatusCode::NOT_FOUND,
            ApiError::Storage(labello_storage::StorageError::AlreadyExists(_)) => {
                StatusCode::CONFLICT
            }
            ApiError::Storage(labello_storage::StorageError::OutsideDatasetRoot(_)) => {
                StatusCode::BAD_REQUEST
            }
            ApiError::Storage(labello_storage::StorageError::Unauthorized(_)) => {
                StatusCode::UNAUTHORIZED
            }
            ApiError::Storage(labello_storage::StorageError::InvalidAssignment(_)) => {
                StatusCode::BAD_REQUEST
            }
            ApiError::Storage(labello_storage::StorageError::AssignmentConflict(_)) => {
                StatusCode::CONFLICT
            }
            ApiError::Storage(_) | ApiError::Http(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (
            status,
            Json(ErrorBody {
                error: self.to_string(),
            }),
        )
            .into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
