use axum::{
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, header::SET_COOKIE},
    response::{IntoResponse, Redirect, Response},
};
use labello_client::{OAuthCallbackRequest, OAuthLoginRequest};
use labello_domain::{DatasetId, DatasetRole, DatasetRoleAssignment, UserId};

use crate::{
    ApiState,
    auth::session_token,
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
    tracing::info!(event = "auth.oauth.started", "GitHub OAuth flow started");
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
    let account = oauth::exchange_code(
        &state.http,
        config,
        &state.github_oauth_endpoints,
        &query.code,
    )
    .await?;
    if state.server_store.user(&account.user_id)?.is_none() {
        assign_initial_annotator_role(&state, &account.user_id).await?;
    }
    let account = state.server_store.upsert_user(account)?;
    if let Some(token) = session_token(&headers) {
        state.server_store.delete_session(&token)?;
    }
    let session = state.create_session(account.user_id.clone())?;
    tracing::info!(
        event = "auth.oauth.completed",
        user_id = %account.user_id,
        "GitHub OAuth login completed"
    );
    let mut response = Redirect::to(&return_to).into_response();
    response.headers_mut().append(
        SET_COOKIE,
        HeaderValue::from_str(&crate::session::session_cookie(
            &session.cookie,
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

async fn assign_initial_annotator_role(state: &ApiState, user_id: &UserId) -> ApiResult<()> {
    let _mutation = state.lock_datasets_root_mutation().await;
    tokio::fs::create_dir_all(state.datasets_root())
        .await
        .map_err(|source| labello_storage::StorageError::Io {
            path: state.datasets_root().to_path_buf(),
            source,
        })?;
    let mut entries = tokio::fs::read_dir(state.datasets_root())
        .await
        .map_err(|source| labello_storage::StorageError::Io {
            path: state.datasets_root().to_path_buf(),
            source,
        })?;
    while let Some(entry) =
        entries
            .next_entry()
            .await
            .map_err(|source| labello_storage::StorageError::Io {
                path: state.datasets_root().to_path_buf(),
                source,
            })?
    {
        let file_type = match entry.file_type().await {
            Ok(file_type) => file_type,
            Err(error) => {
                tracing::warn!(
                    event = "auth.oauth.default_role.skipped",
                    error_kind = %error.kind(),
                    "could not inspect dataset entry"
                );
                continue;
            }
        };
        if !file_type.is_dir() || entry.file_name() == ".labello-server" {
            continue;
        }
        let dataset_id = DatasetId::from(entry.file_name().to_string_lossy().to_string());
        let repo = match state.repo(&dataset_id) {
            Ok(repo) => repo,
            Err(_) => {
                tracing::warn!(
                    event = "auth.oauth.default_role.skipped",
                    dataset_id = %dataset_id,
                    error_kind = "invalid_id",
                    "invalid dataset directory ignored"
                );
                continue;
            }
        };
        let mut metadata = match repo.load_dataset_config().await {
            Ok(metadata) => metadata,
            Err(error) => {
                tracing::warn!(
                    event = "auth.oauth.default_role.skipped",
                    dataset_id = %dataset_id,
                    error_kind = error.kind(),
                    diagnostic = error.safe_diagnostic().as_deref().unwrap_or("redacted"),
                    "unreadable dataset ignored"
                );
                continue;
            }
        };
        if let Some(assignment) = metadata
            .role_assignments
            .iter_mut()
            .find(|assignment| assignment.user_id == *user_id)
        {
            if !assignment.roles.is_empty() {
                continue;
            }
            assignment.roles.insert(DatasetRole::Annotator);
        } else {
            metadata.role_assignments.push(DatasetRoleAssignment {
                dataset_id: dataset_id.clone(),
                user_id: user_id.clone(),
                roles: [DatasetRole::Annotator].into_iter().collect(),
                assigned_at: labello_domain::now(),
                assigned_by: None,
            });
        }
        metadata.updated_at = labello_domain::now();
        repo.save_dataset(&metadata).await?;
        tracing::info!(
            event = "auth.oauth.default_role.assigned",
            dataset_id = %dataset_id,
            user_id = %user_id,
            role = %DatasetRole::Annotator,
            "default dataset role assigned"
        );
    }
    Ok(())
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
