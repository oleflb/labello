use axum::{
    Json,
    extract::{Query, State},
    response::Redirect,
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
    Ok(Redirect::temporary(&config.authorization_url(
        query.state.as_deref().unwrap_or("labello"),
    )?))
}

pub(crate) async fn github_callback(
    State(state): State<ApiState>,
    Query(query): Query<OAuthCallbackRequest>,
) -> ApiResult<Json<labello_domain::UserAccount>> {
    let config = state
        .github_oauth
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("github oauth is not configured".to_string()))?;
    Ok(Json(
        oauth::exchange_code(&state.http, config, &query.code).await?,
    ))
}
