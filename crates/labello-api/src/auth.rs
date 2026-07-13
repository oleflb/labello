use axum::http::HeaderMap;
use labello_domain::{Actor, DatasetMetadata, DatasetRole, UserAccount, UserId, now};

use crate::{
    ApiState,
    error::{ApiError, ApiResult},
};

pub fn actor_from_headers(state: &ApiState, headers: &HeaderMap) -> ApiResult<Actor> {
    if let Some(account) = session_account(state, headers)? {
        return Ok(Actor {
            user_id: account.user_id,
            role: DatasetRole::Annotator,
        });
    }
    if !state.dev_auth_enabled() {
        return Err(ApiError::Unauthorized("login required".to_string()));
    }
    if let Some(expected) = state.dev_auth_token() {
        let provided = header(headers, "x-dev-token")
            .ok_or_else(|| ApiError::Unauthorized("missing x-dev-token header".to_string()))?;
        if provided != expected {
            return Err(ApiError::Unauthorized(
                "invalid x-dev-token header".to_string(),
            ));
        }
    }
    let user_id = header(headers, "x-user-id")
        .ok_or_else(|| ApiError::Unauthorized("missing x-user-id header".to_string()))?;
    let user_id = UserId::from(user_id);
    user_id.validate_path_segment()?;
    let role = header(headers, "x-user-role")
        .map(parse_role)
        .transpose()?
        .unwrap_or(DatasetRole::Annotator);
    state.server_store.upsert_user(UserAccount {
        user_id: user_id.clone(),
        display_name: user_id.to_string(),
        github_user_id: None,
        github_login: None,
        created_at: now(),
        updated_at: now(),
    })?;
    Ok(Actor { user_id, role })
}

pub fn current_account(state: &ApiState, headers: &HeaderMap) -> ApiResult<UserAccount> {
    if let Some(account) = session_account(state, headers)? {
        return Ok(account);
    }
    let actor = actor_from_headers(state, headers)?;
    state
        .server_store
        .user(&actor.user_id)?
        .ok_or_else(|| ApiError::Unauthorized("login required".to_string()))
}

pub(crate) fn session_token(headers: &HeaderMap) -> Option<String> {
    header(headers, "cookie").and_then(|cookies| {
        cookies.split(';').find_map(|cookie| {
            let (name, value) = cookie.trim().split_once('=')?;
            (name == crate::session::SESSION_COOKIE).then(|| value.to_string())
        })
    })
}

fn session_account(state: &ApiState, headers: &HeaderMap) -> ApiResult<Option<UserAccount>> {
    match session_token(headers) {
        Some(token) => state.server_store.session_user(&token),
        None => Ok(None),
    }
}

pub fn ensure_dataset_role(
    metadata: &DatasetMetadata,
    actor: &Actor,
    role: DatasetRole,
) -> ApiResult<()> {
    labello_domain::require_role(
        &metadata.role_assignments,
        &metadata.dataset_id,
        &actor.user_id,
        role,
    )
    .map_err(|error| ApiError::Unauthorized(error.to_string()))
}

pub fn has_dataset_role(metadata: &DatasetMetadata, user_id: &UserId, role: &DatasetRole) -> bool {
    metadata.role_assignments.iter().any(|assignment| {
        assignment.dataset_id == metadata.dataset_id
            && &assignment.user_id == user_id
            && assignment.has_role(role)
    })
}

pub fn ensure_any_dataset_role(metadata: &DatasetMetadata, actor: &Actor) -> ApiResult<()> {
    let allowed = metadata.role_assignments.iter().any(|assignment| {
        assignment.dataset_id == metadata.dataset_id
            && assignment.user_id == actor.user_id
            && !assignment.roles.is_empty()
    });
    if allowed {
        Ok(())
    } else {
        Err(ApiError::Unauthorized(format!(
            "user {} has no role for dataset {}",
            actor.user_id, metadata.dataset_id
        )))
    }
}

pub fn ensure_bootstrap_admin(
    state: &crate::ApiState,
    actor: &Actor,
    action: &str,
) -> ApiResult<()> {
    if state.is_bootstrap_admin(&actor.user_id) {
        Ok(())
    } else {
        Err(ApiError::Unauthorized(format!(
            "user {} cannot {action}",
            actor.user_id
        )))
    }
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

fn parse_role(value: String) -> ApiResult<DatasetRole> {
    match value.as_str() {
        "annotator" => Ok(DatasetRole::Annotator),
        "reviewer" => Ok(DatasetRole::Reviewer),
        "adjudicator" => Ok(DatasetRole::Adjudicator),
        "data_admin" => Ok(DatasetRole::DataAdmin),
        _ => Err(ApiError::BadRequest(format!(
            "unknown dataset role {value}"
        ))),
    }
}
