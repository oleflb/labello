use labello_domain::{
    AnnotationGeometry, AnnotationId, AssignmentId, AssignmentKind, ClassId, CorrectionId,
    DatasetId, DatasetRole, DatasetRoleAssignment, EventPayload, ImageId, ImageRecord,
    ImbalanceConfig, LabelClass, OfflineSyncRequest, PrelabelConfig, PrelabelConfigId,
    TaskDefinition, TaskId, TaskStatus, UserAccount, UserId,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignment_id: Option<AssignmentId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded_image_ids: Vec<ImageId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentActionRequest {
    pub assignment_id: AssignmentId,
    pub image_id: ImageId,
    pub task_id: TaskId,
    pub kind: AssignmentKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppendEventRequest {
    pub payload: EventPayload,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationBatchRequest {
    pub payloads: Vec<EventPayload>,
    #[serde(default)]
    pub complete: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionRequest {
    pub correction_id: CorrectionId,
    pub annotation_id: AnnotationId,
    pub expected_version: u32,
    pub geometry: AnnotationGeometry,
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
    pub return_to: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthCallbackRequest {
    pub code: String,
    pub state: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetUser {
    pub account: UserAccount,
    pub roles: Vec<DatasetRole>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetDatasetRolesRequest {
    pub user_id: UserId,
    pub roles: Vec<DatasetRole>,
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
#[serde(rename_all = "snake_case")]
pub enum IngestJobStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestJob {
    pub job_id: String,
    pub dataset_id: DatasetId,
    pub status: IngestJobStatus,
    pub report: Option<IngestReport>,
    pub error: Option<String>,
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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageExplorerQuery {
    #[serde(default = "default_page")]
    pub page: usize,
    #[serde(default = "default_page_size")]
    pub page_size: usize,
    pub search: Option<String>,
    pub status: Option<TaskStatus>,
    pub task_id: Option<TaskId>,
    pub class_id: Option<ClassId>,
}

fn default_page() -> usize {
    1
}

fn default_page_size() -> usize {
    25
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotFile {
    pub file_name: String,
    pub media_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfflineSyncEnvelope {
    pub request: OfflineSyncRequest,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assign_next_request_uses_camel_case_json() {
        let request = AssignNextRequest {
            task_id: TaskId::from("bounding_box:person"),
            kind: Some(AssignmentKind::Annotation),
            assignment_id: Some(AssignmentId::from("asn_1")),
            excluded_image_ids: vec![ImageId::from("img_1")],
        };

        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::json!({
                "taskId": "bounding_box:person",
                "kind": "annotation",
                "assignmentId": "asn_1",
                "excludedImageIds": ["img_1"]
            })
        );
    }

    #[test]
    fn assign_next_request_defaults_to_no_exclusions() {
        let request: AssignNextRequest = serde_json::from_value(serde_json::json!({
            "taskId": "bounding_box:person",
            "kind": "annotation"
        }))
        .unwrap();

        assert!(request.excluded_image_ids.is_empty());
    }
}
