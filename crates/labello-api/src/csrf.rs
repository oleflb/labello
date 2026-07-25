use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Method, Request, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::{
    ApiState,
    auth::session_token,
    error::{ApiError, ApiResult},
};

pub(crate) const HEADER: &str = "x-csrf-token";

pub(crate) async fn enforce(
    State(state): State<ApiState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if is_unsafe(request.method())
        && request.uri().path() != "/auth/local-admin"
        && let Err(error) = validate_cookie_request(&state, request.headers())
    {
        return error.into_response();
    }
    next.run(request).await
}

pub(crate) fn require_login_origin(state: &ApiState, headers: &HeaderMap) -> ApiResult<()> {
    validate_origin(state, headers, true)
}

fn validate_cookie_request(state: &ApiState, headers: &HeaderMap) -> ApiResult<()> {
    let Some(cookie_token) = session_token(headers) else {
        return Ok(());
    };
    validate_origin(state, headers, false)?;
    let session = state
        .server_store
        .session(&cookie_token)?
        .ok_or_else(|| ApiError::Unauthorized("invalid or expired session".to_string()))?;
    let mut values = headers.get_all(HEADER).iter();
    let supplied = values
        .next()
        .and_then(|value| value.to_str().ok())
        .filter(|_| values.next().is_none())
        .ok_or_else(|| ApiError::Unauthorized("valid CSRF token required".to_string()))?;
    if blake3::hash(supplied.as_bytes()) != blake3::hash(session.csrf_token.as_bytes()) {
        return Err(ApiError::Unauthorized(
            "valid CSRF token required".to_string(),
        ));
    }
    Ok(())
}

fn validate_origin(state: &ApiState, headers: &HeaderMap, required: bool) -> ApiResult<()> {
    let mut origins = headers.get_all(header::ORIGIN).iter();
    let Some(origin) = origins.next() else {
        return if required {
            Err(ApiError::Unauthorized(
                "configured origin required".to_string(),
            ))
        } else {
            Ok(())
        };
    };
    if origins.next().is_some()
        || origin.to_str().map_or(true, |origin| {
            !state
                .browser_origins()
                .iter()
                .any(|allowed| allowed == origin)
        })
    {
        return Err(ApiError::Unauthorized("origin is not allowed".to_string()));
    }
    Ok(())
}

fn is_unsafe(method: &Method) -> bool {
    !matches!(method.as_str(), "GET" | "HEAD" | "OPTIONS")
}
