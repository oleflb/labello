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

include!("dto/access.rs");
include!("dto/workflow.rs");
include!("dto/offline.rs");
include!("dto/media.rs");

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
