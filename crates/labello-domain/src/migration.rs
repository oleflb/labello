use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{DomainError, DomainResult, SCHEMA_VERSION, Timestamp};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MigrationRecord {
    pub from_version: u32,
    pub to_version: u32,
    pub name: String,
    pub applied_at: Timestamp,
}

pub fn validate_schema_version(schema_version: u32) -> DomainResult<()> {
    if schema_version == SCHEMA_VERSION {
        Ok(())
    } else {
        Err(DomainError::UnsupportedSchemaVersion {
            found: schema_version,
            supported: SCHEMA_VERSION,
        })
    }
}

pub trait SequentialMigration<T> {
    fn source_version(&self) -> u32;
    fn target_version(&self) -> u32;
    fn name(&self) -> &'static str;
    fn migrate(&self, value: T) -> DomainResult<T>;
}

pub fn migrate_sequential<T>(
    mut value: T,
    mut version: u32,
    target: u32,
    migrations: &[&dyn SequentialMigration<T>],
) -> DomainResult<T> {
    while version < target {
        let next = migrations
            .iter()
            .find(|migration| {
                migration.source_version() == version && migration.target_version() == version + 1
            })
            .ok_or(DomainError::UnsupportedSchemaVersion {
                found: version,
                supported: target,
            })?;
        value = next.migrate(value)?;
        version += 1;
    }
    Ok(value)
}
