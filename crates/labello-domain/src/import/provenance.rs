use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ImportId;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceProfile {
    pub profile_id: String,
    pub profile_version: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "provenance", rename_all = "snake_case")]
pub enum ImportGeometryProvenance {
    Direct,
    Derived { transform: ImportTransform },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportTransform {
    pub transform_id: String,
    pub version: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parameters: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportedOrigin {
    pub import_id: ImportId,
    pub source_profile: SourceProfile,
    pub source_namespace: String,
    pub source_object_key: String,
    pub geometry_provenance: ImportGeometryProvenance,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ImportCoverage {
    Complete,
    VerifiedEmpty,
    Incomplete,
    Excluded,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceMembership {
    pub source_namespace: String,
    pub split: String,
    pub source_image_key: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportCoverageCounts {
    pub complete: usize,
    pub verified_empty: usize,
    pub incomplete: usize,
    pub excluded: usize,
}

impl ImportCoverageCounts {
    pub fn total(&self) -> usize {
        self.complete + self.verified_empty + self.incomplete + self.excluded
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportCoverageTotals {
    pub bounding_boxes: ImportCoverageCounts,
    pub skeletons: ImportCoverageCounts,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportMigrationTotals {
    pub expected_targets: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportOutputTotals {
    pub images: usize,
    pub classes: usize,
    pub tasks: usize,
    pub annotations: usize,
    pub events: usize,
    pub states: usize,
    pub estimated_bytes: u64,
}
