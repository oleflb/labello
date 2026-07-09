use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    DatasetId, DatasetRole, EventLogEntry, ImageRecord, ImageState, SCHEMA_VERSION, TaskDefinition,
    Timestamp, UserId,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OfflineBundle {
    pub schema_version: u32,
    pub dataset_id: DatasetId,
    pub user_id: UserId,
    pub created_at: Timestamp,
    pub expires_at: Option<Timestamp>,
    pub roles: Vec<DatasetRole>,
    pub tasks: Vec<TaskDefinition>,
    pub images: Vec<OfflineImageBundle>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OfflineImageBundle {
    pub image: ImageRecord,
    pub state: ImageState,
    pub event_log_fragment: EventLogFragment,
    pub image_bytes_base64: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EventLogFragment {
    pub image_id: crate::ImageId,
    pub base_sequence: u64,
    pub events: Vec<EventLogEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OfflineSyncRequest {
    pub schema_version: u32,
    pub dataset_id: DatasetId,
    pub user_id: UserId,
    pub fragments: Vec<EventLogFragment>,
}

impl OfflineSyncRequest {
    pub fn new(dataset_id: DatasetId, user_id: UserId, fragments: Vec<EventLogFragment>) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            dataset_id,
            user_id,
            fragments,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SyncConflict {
    pub image_id: crate::ImageId,
    pub reason: String,
    pub server_sequence: u64,
    pub client_base_sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OfflineSyncResult {
    pub merged_events: usize,
    pub conflicts: Vec<SyncConflict>,
}
