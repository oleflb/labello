use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    AnnotationGeometry, AnnotationId, AnnotationType, ClassId, DatasetId, DatasetRole,
    EventLogEntry, ImageRecord, ImageState, ImportManifest, PrelabelConfigId, SCHEMA_VERSION,
    TaskDefinition, TaskId, Timestamp, UserId,
};

pub const MAX_OFFLINE_FRAGMENTS: usize = 1_000;
pub const MAX_OFFLINE_MUTATIONS: usize = 10_000;
pub const MAX_OFFLINE_MUTATIONS_PER_FRAGMENT: usize = 10_000;
pub const MAX_OFFLINE_REASON_BYTES: usize = 2_000;

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
    #[serde(default)]
    pub import_manifests: Vec<ImportManifest>,
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OfflineSyncRequest {
    pub schema_version: u32,
    pub dataset_id: DatasetId,
    pub user_id: UserId,
    pub fragments: Vec<OfflineMutationFragment>,
}

impl OfflineSyncRequest {
    pub fn new(
        dataset_id: DatasetId,
        user_id: UserId,
        fragments: Vec<OfflineMutationFragment>,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            dataset_id,
            user_id,
            fragments,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OfflineMutationFragment {
    pub image_id: crate::ImageId,
    pub base_sequence: u64,
    pub mutations: Vec<OfflineMutation>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OfflineMutation {
    AnnotationUpsert {
        annotation_id: AnnotationId,
        expected_version: Option<u32>,
        task_id: TaskId,
        class_id: ClassId,
        annotation_type: AnnotationType,
        source: OfflineAnnotationSource,
        geometry: AnnotationGeometry,
        reason: Option<String>,
    },
    AnnotationDelete {
        annotation_id: AnnotationId,
        expected_version: u32,
        reason: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum OfflineAnnotationSource {
    Human,
    PrelabelSuggestion {
        config_id: PrelabelConfigId,
        model_id: String,
        confidence: f32,
    },
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
