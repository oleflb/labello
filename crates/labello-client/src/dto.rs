use labello_domain::{
    AnnotationVersion, AssignmentKind, ClassId, DatasetId, DatasetRole, DatasetRoleAssignment,
    EventPayload, ImageId, ImageRecord, ImbalanceConfig, LabelClass, OfflineSyncRequest,
    PrelabelConfig, PrelabelConfigId, TaskDefinition, TaskId, UserId,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDatasetRequest {
    pub dataset_id: DatasetId,
    pub name: String,
    pub admin_user_id: UserId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetSummary {
    pub dataset_id: DatasetId,
    pub name: String,
    pub roles: Vec<DatasetRole>,
    pub total_images: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDatasetConfigRequest {
    pub name: String,
    pub image_roots: Vec<String>,
    pub label_classes: Vec<LabelClass>,
    pub tasks: Vec<TaskDefinition>,
    pub role_assignments: Vec<DatasetRoleAssignment>,
    pub imbalance: Option<ImbalanceConfig>,
    pub prelabel_configs: Vec<PrelabelConfig>,
}

impl UpdateDatasetConfigRequest {
    pub fn from_metadata(metadata: &labello_domain::DatasetMetadata) -> Self {
        Self {
            name: metadata.name.clone(),
            image_roots: metadata.image_roots.clone(),
            label_classes: metadata.label_classes.clone(),
            tasks: metadata.tasks.clone(),
            role_assignments: metadata.role_assignments.clone(),
            imbalance: metadata.imbalance.clone(),
            prelabel_configs: metadata.prelabel_configs.clone(),
        }
    }

    pub fn class_ids(&self) -> Vec<ClassId> {
        self.label_classes
            .iter()
            .map(|class| class.class_id.clone())
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignNextRequest {
    pub task_id: TaskId,
    pub kind: Option<AssignmentKind>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppendEventRequest {
    pub payload: EventPayload,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionRequest {
    pub annotation: AnnotationVersion,
    pub previous_version: u32,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfflineBundleRequest {
    #[serde(default = "default_offline_limit")]
    pub limit: usize,
    #[serde(default)]
    pub include_image_bytes: bool,
}

impl Default for OfflineBundleRequest {
    fn default() -> Self {
        Self {
            limit: 25,
            include_image_bytes: false,
        }
    }
}

fn default_offline_limit() -> usize {
    25
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrelabelSuggestionRequest {
    pub config_id: PrelabelConfigId,
    pub task_id: TaskId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthLoginRequest {
    pub state: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthCallbackRequest {
    pub code: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageFile {
    pub image_id: ImageId,
    pub media_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagePreview {
    pub image_id: ImageId,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestReport {
    pub discovered_files: usize,
    pub new_images: usize,
    pub duplicate_files: Vec<DuplicateImage>,
    pub changed_paths: Vec<ChangedPath>,
    pub unreadable_files: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateImage {
    pub image_id: ImageId,
    pub canonical_path: String,
    pub duplicate_path: String,
    pub blake3: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedPath {
    pub relative_path: String,
    pub previous_blake3: String,
    pub current_blake3: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageSummary {
    pub image: ImageRecord,
    pub event_sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfflineSyncEnvelope {
    pub request: OfflineSyncRequest,
}
