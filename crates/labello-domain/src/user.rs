use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{DatasetId, DomainError, DomainResult, Timestamp, UserId};

#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum DatasetRole {
    Annotator,
    Reviewer,
    Adjudicator,
    DataAdmin,
}

impl std::fmt::Display for DatasetRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Annotator => "annotator",
            Self::Reviewer => "reviewer",
            Self::Adjudicator => "adjudicator",
            Self::DataAdmin => "data_admin",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Actor {
    pub user_id: UserId,
    pub role: DatasetRole,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DatasetRoleAssignment {
    pub dataset_id: DatasetId,
    pub user_id: UserId,
    pub roles: BTreeSet<DatasetRole>,
    pub assigned_at: Timestamp,
    pub assigned_by: Option<UserId>,
}

impl DatasetRoleAssignment {
    pub fn has_role(&self, role: &DatasetRole) -> bool {
        self.roles.contains(role)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserAccount {
    pub user_id: UserId,
    pub display_name: String,
    pub github_user_id: Option<String>,
    pub github_login: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub fn require_role(
    assignments: &[DatasetRoleAssignment],
    dataset_id: &DatasetId,
    user_id: &UserId,
    role: DatasetRole,
) -> DomainResult<()> {
    let allowed = assignments.iter().any(|assignment| {
        &assignment.dataset_id == dataset_id
            && &assignment.user_id == user_id
            && assignment.roles.contains(&role)
    });
    if allowed {
        Ok(())
    } else {
        Err(DomainError::MissingRole {
            user_id: user_id.to_string(),
            role: role.to_string(),
        })
    }
}
