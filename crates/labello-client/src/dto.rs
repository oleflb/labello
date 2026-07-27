use std::collections::BTreeMap;

use labello_domain::{
    AnnotationGeometry, AnnotationId, AssignmentId, AssignmentKind, ClassId, CorrectionId,
    DatasetId, DatasetRole, DatasetRoleAssignment, EventLogEntry, EventPayload, ImageId,
    ImageRecord, ImbalanceConfig, LabelClass, PrelabelConfig, PrelabelConfigId, TaskDefinition,
    TaskId, TaskStatus, UserAccount, UserId,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

pub use labello_domain::{
    OfflineAnnotationSource, OfflineMutation, OfflineMutationFragment, OfflineSyncRequest,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthOptions {
    pub github_oauth: bool,
    pub local_admin_login: bool,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub account: UserAccount,
    pub can_create_datasets: bool,
    pub csrf_token: String,
}

impl std::fmt::Debug for SessionInfo {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionInfo")
            .field("account", &self.account)
            .field("can_create_datasets", &self.can_create_datasets)
            .field("csrf_token", &"<redacted>")
            .finish()
    }
}

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
pub struct AssignmentAvailabilityRequest {
    pub kind: AssignmentKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentAvailability {
    pub kind: AssignmentKind,
    pub tasks: BTreeMap<TaskId, bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentActionRequest {
    pub assignment_id: AssignmentId,
    pub image_id: ImageId,
    pub task_id: TaskId,
    pub kind: AssignmentKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppendEventRequest {
    pub payload: EventPayload,
}

impl Serialize for AppendEventRequest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire<'a> {
            schema_version: u32,
            payload: &'a EventPayload,
        }

        Wire {
            schema_version: labello_domain::SCHEMA_VERSION,
            payload: &self.payload,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AppendEventRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let (schema_version, mut object) =
            mutation_request_parts(value).map_err(D::Error::custom)?;
        let payload = object
            .remove("payload")
            .ok_or_else(|| D::Error::custom("event payload is missing"))?;
        Ok(Self {
            payload: deserialize_versioned_payload(payload, schema_version)
                .map_err(D::Error::custom)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnnotationBatchRequest {
    pub payloads: Vec<EventPayload>,
    pub complete: bool,
}

impl Serialize for AnnotationBatchRequest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire<'a> {
            schema_version: u32,
            payloads: &'a [EventPayload],
            complete: bool,
        }

        Wire {
            schema_version: labello_domain::SCHEMA_VERSION,
            payloads: &self.payloads,
            complete: self.complete,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AnnotationBatchRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let (schema_version, mut object) =
            mutation_request_parts(value).map_err(D::Error::custom)?;
        let payloads = object
            .remove("payloads")
            .ok_or_else(|| D::Error::custom("annotation payloads are missing"))?
            .as_array()
            .ok_or_else(|| D::Error::custom("annotation payloads must be an array"))?
            .iter()
            .cloned()
            .map(|payload| deserialize_versioned_payload(payload, schema_version))
            .collect::<Result<Vec<_>, _>>()
            .map_err(D::Error::custom)?;
        let complete = object
            .remove("complete")
            .map(|value| {
                value
                    .as_bool()
                    .ok_or("annotation batch complete must be a boolean")
            })
            .transpose()
            .map_err(D::Error::custom)?
            .unwrap_or(false);
        Ok(Self { payloads, complete })
    }
}

fn mutation_request_parts(
    value: serde_json::Value,
) -> Result<(Option<u32>, serde_json::Map<String, serde_json::Value>), String> {
    let mut object = value
        .as_object()
        .cloned()
        .ok_or_else(|| "mutation request must be an object".to_string())?;
    let schema_version = object
        .remove("schemaVersion")
        .map(|value| {
            value
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| "schemaVersion must be an unsigned 32-bit integer".to_string())
        })
        .transpose()?;
    if let Some(schema_version) = schema_version {
        labello_domain::validate_supported_schema_version(schema_version)
            .map_err(|error| error.to_string())?;
    }
    Ok((schema_version, object))
}

fn deserialize_versioned_payload(
    payload: serde_json::Value,
    schema_version: Option<u32>,
) -> Result<EventPayload, String> {
    let schema_version = schema_version.unwrap_or_else(|| {
        if payload
            .get("annotation")
            .and_then(|annotation| annotation.get("source"))
            .is_some()
        {
            labello_domain::LEGACY_SCHEMA_VERSION
        } else {
            labello_domain::SCHEMA_VERSION
        }
    });
    let event_type = payload
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "event payload kind is missing".to_string())?;
    let event: EventLogEntry = serde_json::from_value(serde_json::json!({
        "schemaVersion": schema_version,
        "eventSequence": 0,
        "eventId": "dto_event",
        "imageId": "dto_image",
        "type": event_type,
        "actorUserId": "dto_user",
        "actorRole": "annotator",
        "timestamp": "1970-01-01T00:00:00Z",
        "payload": payload,
    }))
    .map_err(|error| error.to_string())?;
    event.validate_shape().map_err(|error| error.to_string())?;
    Ok(event.payload)
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
    use labello_domain::{
        AnnotationOrigin, DatasetId, EventPayload, OfflineSyncRequest, RevisionSource,
        SCHEMA_VERSION, UserId,
    };

    use super::*;

    #[test]
    fn auth_options_use_camel_case_json() {
        let options = AuthOptions {
            github_oauth: true,
            local_admin_login: false,
        };

        assert_eq!(
            serde_json::to_value(options).unwrap(),
            serde_json::json!({
                "githubOauth": true,
                "localAdminLogin": false
            })
        );
    }

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

    #[test]
    fn offline_bundle_request_preserves_defaults_and_casing() {
        let request: OfflineBundleRequest = serde_json::from_value(serde_json::json!({})).unwrap();

        assert_eq!(request, OfflineBundleRequest::default());
        assert_eq!(request.limit, 25);
        assert!(!request.include_image_bytes);
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::json!({
                "limit": 25,
                "includeImageBytes": false
            })
        );
    }

    #[test]
    fn image_explorer_query_preserves_defaults() {
        let query: ImageExplorerQuery = serde_json::from_value(serde_json::json!({})).unwrap();

        assert_eq!(query.page, 1);
        assert_eq!(query.page_size, 25);
        assert_eq!(query.search, None);
        assert_eq!(query.status, None);
        assert_eq!(query.task_id, None);
        assert_eq!(query.class_id, None);
        assert_eq!(
            serde_json::to_value(query).unwrap(),
            serde_json::json!({
                "page": 1,
                "pageSize": 25,
                "search": null,
                "status": null,
                "taskId": null,
                "classId": null
            })
        );
    }

    #[test]
    fn offline_sync_envelope_keeps_current_schema_version_and_casing() {
        let envelope = OfflineSyncEnvelope {
            request: OfflineSyncRequest::new(
                DatasetId::from("ds_1"),
                UserId::from("user_1"),
                Vec::new(),
            ),
        };

        assert_eq!(
            serde_json::to_value(envelope).unwrap(),
            serde_json::json!({
                "request": {
                    "schemaVersion": SCHEMA_VERSION,
                    "datasetId": "ds_1",
                    "userId": "user_1",
                    "fragments": []
                }
            })
        );
    }

    #[test]
    fn offline_sync_uses_bounded_mutations_without_event_authority_fields() {
        let request = OfflineSyncRequest::new(
            DatasetId::from("ds_1"),
            UserId::from("user_1"),
            vec![OfflineMutationFragment {
                image_id: ImageId::from("img_1"),
                base_sequence: 7,
                mutations: vec![OfflineMutation::AnnotationDelete {
                    annotation_id: AnnotationId::from("ann_1"),
                    expected_version: 3,
                    reason: None,
                }],
            }],
        );

        let value = serde_json::to_value(request).unwrap();
        assert_eq!(
            value["fragments"][0]["mutations"][0]["kind"],
            "annotation_delete"
        );
        assert!(value["fragments"][0].get("events").is_none());
        assert!(
            value["fragments"][0]["mutations"][0]
                .get("actorUserId")
                .is_none()
        );
        assert!(
            value["fragments"][0]["mutations"][0]
                .get("timestamp")
                .is_none()
        );
    }

    #[test]
    fn mutation_dtos_emit_current_schema_version() {
        let request = AnnotationBatchRequest {
            payloads: Vec::new(),
            complete: true,
        };

        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::json!({
                "schemaVersion": SCHEMA_VERSION,
                "payloads": [],
                "complete": true,
            })
        );
    }

    #[test]
    fn mutation_dtos_upcast_legacy_v2_annotations() {
        let request: AppendEventRequest = serde_json::from_value(serde_json::json!({
            "schemaVersion": 2,
            "payload": {
                "kind": "annotation_version_created",
                "annotation": {
                    "annotationId": "ann_1",
                    "version": 1,
                    "taskId": "bounding_box:person",
                    "classId": "person",
                    "type": "bounding_box",
                    "source": { "source": "human" },
                    "geometry": {
                        "type": "bounding_box",
                        "geometry": { "x": 0.1, "y": 0.1, "width": 0.2, "height": 0.2 }
                    },
                    "authorUserId": "annotator",
                    "createdAt": "2026-01-02T03:04:05Z",
                    "updatedAt": "2026-01-02T03:04:05Z",
                    "deleted": false
                },
                "previous_version": null,
                "reason": null
            }
        }))
        .unwrap();

        let EventPayload::AnnotationVersionCreated { annotation, .. } = request.payload else {
            panic!("unexpected payload")
        };
        assert!(matches!(
            annotation.origin,
            AnnotationOrigin::Native { legacy_v2: true }
        ));
        assert!(matches!(
            annotation.revision_source,
            RevisionSource::Human { .. }
        ));
    }
}
