use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    DatasetId, DatasetRoleAssignment, ImageDimensions, ImageId, LabelClass, MigrationRecord,
    PrelabelConfig, SCHEMA_VERSION, TaskDefinition, Timestamp,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DatasetMetadata {
    pub schema_version: u32,
    pub dataset_id: DatasetId,
    pub name: String,
    pub dataset_root: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub migration_history: Vec<MigrationRecord>,
    pub label_classes: Vec<LabelClass>,
    pub tasks: Vec<TaskDefinition>,
    pub images: BTreeMap<ImageId, ImageRecord>,
    pub role_assignments: Vec<DatasetRoleAssignment>,
    pub imbalance: Option<ImbalanceConfig>,
    pub prelabel_configs: Vec<PrelabelConfig>,
}

impl DatasetMetadata {
    pub fn new(dataset_id: DatasetId, name: impl Into<String>, timestamp: Timestamp) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            dataset_id,
            name: name.into(),
            dataset_root: ".".to_string(),
            created_at: timestamp,
            updated_at: timestamp,
            migration_history: Vec::new(),
            label_classes: Vec::new(),
            tasks: Vec::new(),
            images: BTreeMap::new(),
            role_assignments: Vec::new(),
            imbalance: None,
            prelabel_configs: Vec::new(),
        }
    }

    pub fn task(&self, task_id: &crate::TaskId) -> Option<&TaskDefinition> {
        self.tasks.iter().find(|task| &task.task_id == task_id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImageRecord {
    pub image_id: ImageId,
    pub blake3: String,
    pub canonical_path: String,
    pub known_paths: Vec<String>,
    pub duplicate_paths: Vec<String>,
    pub file_name: String,
    pub byte_size: u64,
    pub width: u32,
    pub height: u32,
    pub media_type: String,
}

impl ImageRecord {
    pub fn dimensions(&self) -> ImageDimensions {
        ImageDimensions {
            width: self.width,
            height: self.height,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImagesIndex {
    pub schema_version: u32,
    pub images_by_hash: BTreeMap<String, ImageRecord>,
}

impl Default for ImagesIndex {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            images_by_hash: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImbalanceConfig {
    pub max_ratio: f32,
    pub enforce: bool,
}

impl Default for ImbalanceConfig {
    fn default() -> Self {
        Self {
            max_ratio: 2.0,
            enforce: false,
        }
    }
}
