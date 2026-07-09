use axum::http::HeaderMap;
use labello_domain::{Actor, DatasetMetadata, DatasetRole, UserId};

use crate::error::{ApiError, ApiResult};

pub fn actor_from_headers(headers: &HeaderMap) -> ApiResult<Actor> {
    let user_id = header(headers, "x-user-id")
        .ok_or_else(|| ApiError::Unauthorized("missing x-user-id header".to_string()))?;
    let role = header(headers, "x-user-role")
        .map(parse_role)
        .transpose()?
        .unwrap_or(DatasetRole::Annotator);
    Ok(Actor {
        user_id: UserId::from(user_id),
        role,
    })
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
