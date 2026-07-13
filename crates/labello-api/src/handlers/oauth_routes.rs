use axum::{
    Json,
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
) -> ApiResult<Redirect> {
    let config = state
        .github_oauth
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("github oauth is not configured".to_string()))?;
    let _ = query;
    let oauth_state = state.server_store.create_oauth_state()?;
    Ok(Redirect::temporary(
        &config.authorization_url(&oauth_state)?,
    ))
}

pub(crate) async fn github_callback(
    State(state): State<ApiState>,
    Query(query): Query<OAuthCallbackRequest>,
) -> ApiResult<Response> {
    let config = state
        .github_oauth
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("github oauth is not configured".to_string()))?;
    if !state.server_store.consume_oauth_state(&query.state)? {
        return Err(ApiError::Unauthorized(
            "invalid or expired oauth state".to_string(),
        ));
    }
    let account = state
        .server_store
        .upsert_user(oauth::exchange_code(&state.http, config, &query.code).await?)?;
    let token = state.create_session(account.user_id.clone())?;
    let mut headers = HeaderMap::new();
    headers.insert(
        SET_COOKIE,
        HeaderValue::from_str(&crate::session::session_cookie(
            &token,
            state.session_cookie_secure(),
        ))
        .map_err(|error| ApiError::Internal(error.to_string()))?,
    );
    Ok((headers, Json(account)).into_response())
}
