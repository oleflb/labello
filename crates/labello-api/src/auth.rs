use axum::http::HeaderMap;
use labello_domain::{Actor, DatasetMetadata, DatasetRole, UserAccount, UserId};

use crate::{
    ApiState,
    error::{ApiError, ApiResult},
};

pub fn actor_from_headers(state: &ApiState, headers: &HeaderMap) -> ApiResult<Actor> {
    let account = current_account(state, headers)?;
    Ok(Actor {
        user_id: account.user_id,
        role: DatasetRole::Annotator,
    })
}

pub fn current_account(state: &ApiState, headers: &HeaderMap) -> ApiResult<UserAccount> {
    session_account(state, headers)?.ok_or_else(|| {
        tracing::debug!(
            event = "auth.denied",
            auth_mode = "session",
            reason = "login_required",
            "authentication required"
        );
        ApiError::Unauthorized("login required".to_string())
    })
}

pub(crate) fn session_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("cookie")
        .and_then(|value| value.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|cookie| {
                let (name, value) = cookie.trim().split_once('=')?;
                (name == crate::session::SESSION_COOKIE).then(|| value.to_string())
            })
        })
}

fn session_account(state: &ApiState, headers: &HeaderMap) -> ApiResult<Option<UserAccount>> {
    match session_token(headers) {
        Some(token) => Ok(state
            .server_store
            .session(&token)?
            .map(|session| session.account)),
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
        role.clone(),
    )
    .map_err(|error| {
        tracing::warn!(
            event = "authorization.denied",
            user_id = %actor.user_id,
            dataset_id = %metadata.dataset_id,
            required_role = %role,
            "dataset role required"
        );
        ApiError::Unauthorized(error.to_string())
    })
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
        tracing::warn!(
            event = "authorization.denied",
            user_id = %actor.user_id,
            dataset_id = %metadata.dataset_id,
            reason = "no_dataset_role",
            "dataset access denied"
        );
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
        tracing::warn!(
            event = "authorization.denied",
            user_id = %actor.user_id,
            reason = "bootstrap_admin_required",
            action,
            "bootstrap administrator access denied"
        );
        Err(ApiError::Unauthorized(format!(
            "user {} cannot {action}",
            actor.user_id
        )))
    }
}
