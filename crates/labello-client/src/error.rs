use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("HTTP request failed")]
    Http(reqwest::Error),

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

impl From<reqwest::Error> for ClientError {
    fn from(error: reqwest::Error) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let connect = error.is_connect();
        #[cfg(target_arch = "wasm32")]
        let connect = false;
        tracing::error!(
            event = "http.client.failed",
            outcome = "transport_error",
            timeout = error.is_timeout(),
            connect,
            request = error.is_request(),
            body = error.is_body(),
            decode = error.is_decode(),
            status = error.status().map(|status| status.as_u16()),
            "HTTP request failed"
        );
        Self::Http(error)
    }
}
