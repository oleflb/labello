use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("url error: {0}")]
    Url(#[from] url::ParseError),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("api error {status}: {message}")]
    Api { status: u16, message: String },

    #[error("demo api error: {0}")]
    Demo(String),
}

pub type ClientResult<T> = Result<T, ClientError>;
