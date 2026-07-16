use axum::{
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, header::SET_COOKIE},
    response::{IntoResponse, Redirect, Response},
};
use labello_client::{OAuthCallbackRequest, OAuthLoginRequest};

use crate::{
    ApiState,
    error::{ApiError, ApiResult},
    oauth,
};

pub(crate) async fn github_login(
    State(state): State<ApiState>,
    Query(query): Query<OAuthLoginRequest>,
) -> ApiResult<Response> {
    let config = state
        .github_oauth
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("github oauth is not configured".to_string()))?;
    let return_to = validate_return_to(&state, query.return_to.as_deref())?;
    let flow = state.server_store.create_oauth_flow(return_to)?;
    let mut response = Redirect::temporary(&config.authorization_url(&flow.state)?).into_response();
    response.headers_mut().append(
        SET_COOKIE,
        HeaderValue::from_str(&crate::session::oauth_flow_cookie(
            &flow.cookie_token,
            state.session_cookie_secure(),
        ))
        .map_err(|error| ApiError::Internal(error.to_string()))?,
    );
    Ok(response)
}

pub(crate) async fn github_callback(
    State(state): State<ApiState>,
    Query(query): Query<OAuthCallbackRequest>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let config = state
        .github_oauth
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("github oauth is not configured".to_string()))?;
    let cookie_token = cookie(&headers, crate::session::OAUTH_FLOW_COOKIE)
        .ok_or_else(|| ApiError::Unauthorized("missing oauth flow cookie".to_string()))?;
    let return_to = state
        .server_store
        .consume_oauth_flow(&query.state, &cookie_token)?
        .ok_or_else(|| ApiError::Unauthorized("invalid or expired oauth flow".to_string()))?;
    let account = state.server_store.upsert_user(
        oauth::exchange_code(
            &state.http,
            config,
            &state.github_oauth_endpoints,
            &query.code,
        )
        .await?,
    )?;
    let token = state.create_session(account.user_id.clone())?;
    let mut response = Redirect::to(&return_to).into_response();
    response.headers_mut().append(
        SET_COOKIE,
        HeaderValue::from_str(&crate::session::session_cookie(
            &token,
            state.session_cookie_secure(),
        ))
        .map_err(|error| ApiError::Internal(error.to_string()))?,
    );
    response.headers_mut().append(
        SET_COOKIE,
        HeaderValue::from_str(&crate::session::expired_oauth_flow_cookie(
            state.session_cookie_secure(),
        ))
        .map_err(|error| ApiError::Internal(error.to_string()))?,
    );
    Ok(response)
}

fn validate_return_to(state: &ApiState, return_to: Option<&str>) -> ApiResult<String> {
    let return_to = return_to
        .or_else(|| state.browser_origins().first().map(String::as_str))
        .ok_or_else(|| ApiError::BadRequest("no browser origins are configured".to_string()))?;
    let url = url::Url::parse(return_to)
        .map_err(|error| ApiError::BadRequest(format!("invalid returnTo URL: {error}")))?;
    let origin = url.origin().ascii_serialization();
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || !state.browser_origins().contains(&origin)
    {
        return Err(ApiError::BadRequest(
            "returnTo must use a configured browser origin".to_string(),
        ));
    }
    Ok(url.to_string())
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(axum::http::header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|cookie| {
                let (candidate, value) = cookie.trim().split_once('=')?;
                (candidate == name).then(|| value.to_string())
            })
        })
}
