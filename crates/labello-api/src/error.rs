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

    #[error("internal error: {0}")]
    Internal(String),

    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorBody {
    error: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let error_kind = self.kind();
        if status.is_server_error() {
            match &self {
                ApiError::Storage(error) => tracing::error!(
                    event = "api.error",
                    error_kind,
                    diagnostic = error.safe_diagnostic().as_deref().unwrap_or("redacted"),
                    "request failed"
                ),
                ApiError::Http(error) => tracing::error!(
                    event = "api.error",
                    error_kind,
                    timeout = error.is_timeout(),
                    connect = error.is_connect(),
                    upstream_status = error.status().map(|status| status.as_u16()),
                    "request failed"
                ),
                ApiError::Json(error) => tracing::error!(
                    event = "api.error",
                    error_kind,
                    line = error.line(),
                    column = error.column(),
                    "request failed"
                ),
                _ => tracing::error!(event = "api.error", error_kind, "request failed"),
            }
        } else {
            tracing::debug!(
                event = "api.request.rejected",
                error_kind,
                status = status.as_u16(),
                "request rejected"
            );
        }
        (
            status,
            Json(ErrorBody {
                error: self.public_message(),
            }),
        )
            .into_response()
    }
}

impl ApiError {
    fn status(&self) -> StatusCode {
        match self {
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
            ApiError::Storage(labello_storage::StorageError::InvalidCorrection(_)) => {
                StatusCode::BAD_REQUEST
            }
            ApiError::Storage(labello_storage::StorageError::AssignmentConflict(_)) => {
                StatusCode::CONFLICT
            }
            ApiError::Storage(_)
            | ApiError::Http(_)
            | ApiError::Internal(_)
            | ApiError::Json(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "bad_request",
            Self::Unauthorized(_) => "unauthorized",
            Self::NotFound(_) => "not_found",
            Self::InvalidId(_) => "invalid_id",
            Self::Storage(error) => error.kind(),
            Self::Http(_) => "http_client",
            Self::Internal(_) => "internal",
            Self::Json(_) => "serialization",
        }
    }

    fn public_message(&self) -> String {
        match self {
            Self::Storage(labello_storage::StorageError::NotFound(_)) => "not found".to_string(),
            Self::Storage(labello_storage::StorageError::AlreadyExists(_)) => {
                "already exists".to_string()
            }
            Self::Storage(labello_storage::StorageError::OutsideDatasetRoot(_)) => {
                "path is outside the dataset root".to_string()
            }
            Self::Storage(labello_storage::StorageError::Unauthorized(_)) => {
                "unauthorized".to_string()
            }
            Self::Storage(labello_storage::StorageError::InvalidAssignment(message))
            | Self::Storage(labello_storage::StorageError::InvalidCorrection(message))
            | Self::Storage(labello_storage::StorageError::AssignmentConflict(message)) => {
                message.clone()
            }
            error if error.status().is_server_error() => "internal server error".to_string(),
            error => error.to_string(),
        }
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
