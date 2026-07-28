#[cfg(test)]
mod tests {
    use super::*;
    use labello_domain::{DatasetId, SCHEMA_VERSION, now};
    use serde_json::json;

    #[test]
    fn parser_time_limit_is_an_actionable_client_error() {
        let error = map_storage(storage::StorageError::Import {
            code: "parser_time_limit".to_string(),
            message: "import parsing exceeded the parser time budget".to_string(),
        });

        assert!(matches!(error, ApiError::Unprocessable(_)));
    }

    fn attestations() -> client::ImportAttestations {
        client::ImportAttestations {
            ground_truth: true,
            exhaustive: true,
            coverage_scope: Vec::new(),
            provenance: "fixture".to_string(),
        }
    }

    fn job(profile: storage::ImportProfile) -> storage::ImportJob {
        let timestamp = now();
        storage::ImportJob {
            schema_version: SCHEMA_VERSION,
            import_id: ImportId::from("imp_test"),
            owner_user_id: UserId::from("admin"),
            destination_dataset_id: DatasetId::from("imported"),
            destination_name: "Imported".to_string(),
            profile,
            transport: storage::ImportTransport::Browser,
            phase: storage::ImportJobPhase::Uploading,
            source_fingerprint: None,
            plan_hash: None,
            preflight_generation: None,
            accepted_files: 2,
            accepted_bytes: 10,
            created_at: timestamp,
            updated_at: timestamp,
            failure_code: None,
        }
    }

    fn control(profile: client::ImportProfile) -> JobControl {
        JobControl {
            import_id: ImportId::from("imp_test"),
            owner_user_id: UserId::from("admin"),
            create_request: client::CreateImportRequest {
                destination_dataset_id: DatasetId::from("imported"),
                destination_name: "Imported".to_string(),
                profile,
                source: client::ImportSourceSelection::BrowserFolder,
                attestations: attestations(),
            },
            seal_request: None,
            files: BTreeMap::from([
                (
                    "descriptor".to_string(),
                    FileControl {
                        client_file_id: None,
                        relative_path: "annotations/keypoints.json".to_string(),
                        byte_size: 5,
                        blake3: "a".repeat(64),
                        accepted_bytes: 5,
                        complete: true,
                    },
                ),
                (
                    "instances".to_string(),
                    FileControl {
                        client_file_id: None,
                        relative_path: "annotations/instances.json".to_string(),
                        byte_size: 5,
                        blake3: "c".repeat(64),
                        accepted_bytes: 5,
                        complete: true,
                    },
                ),
                (
                    "image".to_string(),
                    FileControl {
                        client_file_id: None,
                        relative_path: "images/a.png".to_string(),
                        byte_size: 5,
                        blake3: "b".repeat(64),
                        accepted_bytes: 5,
                        complete: true,
                    },
                ),
            ]),
            plan: None,
            accepted_plan_request: None,
            pending_plan_request: None,
        }
    }

    #[test]
    fn import_control_persistence_contract_is_stable() {
        let control = control(client::ImportProfile::CocoInstancesGtV1);
        assert_eq!(
            serde_json::to_value(&control).unwrap(),
            json!({
                "importId": "imp_test",
                "ownerUserId": "admin",
                "createRequest": {
                    "destinationDatasetId": "imported",
                    "destinationName": "Imported",
                    "profile": "coco_instances_gt_v1",
                    "source": {
                        "transport": "browser_folder"
                    },
                    "attestations": {
                        "groundTruth": true,
                        "exhaustive": true,
                        "coverageScope": [],
                        "provenance": "fixture"
                    }
                },
                "sealRequest": null,
                "files": {
                    "descriptor": {
                        "clientFileId": null,
                        "relativePath": "annotations/keypoints.json",
                        "byteSize": 5,
                        "blake3": "a".repeat(64),
                        "acceptedBytes": 5,
                        "complete": true
                    },
                    "image": {
                        "clientFileId": null,
                        "relativePath": "images/a.png",
                        "byteSize": 5,
                        "blake3": "b".repeat(64),
                        "acceptedBytes": 5,
                        "complete": true
                    },
                    "instances": {
                        "clientFileId": null,
                        "relativePath": "annotations/instances.json",
                        "byteSize": 5,
                        "blake3": "c".repeat(64),
                        "acceptedBytes": 5,
                        "complete": true
                    }
                },
                "plan": null,
                "acceptedPlanRequest": null,
                "pendingPlanRequest": null
            })
        );
        let mut legacy = serde_json::to_value(&control).unwrap();
        let object = legacy.as_object_mut().unwrap();
        object.remove("acceptedPlanRequest");
        object.remove("pendingPlanRequest");
        let descriptor = object["files"]["descriptor"].as_object_mut().unwrap();
        descriptor.remove("acceptedBytes");
        descriptor.remove("complete");
        let restored: JobControl = serde_json::from_value(legacy).unwrap();
        assert!(restored.accepted_plan_request.is_none());
        assert!(restored.pending_plan_request.is_none());
        assert_eq!(restored.files["descriptor"].accepted_bytes, 0);
        assert!(!restored.files["descriptor"].complete);

        let pending = IdempotencyRecord::Pending {
            operation: "create".to_string(),
            request_hash: "request-hash".to_string(),
        };
        assert_eq!(
            serde_json::to_value(pending).unwrap(),
            json!({
                "status": "pending",
                "operation": "create",
                "request_hash": "request-hash"
            })
        );
        let complete = IdempotencyRecord::Complete {
            operation: "create".to_string(),
            request_hash: "request-hash".to_string(),
            response: json!({"importId": "imp_test"}),
        };
        assert_eq!(
            serde_json::to_value(complete).unwrap(),
            json!({
                "status": "complete",
                "operation": "create",
                "request_hash": "request-hash",
                "response": {"importId": "imp_test"}
            })
        );
    }

    #[test]
    fn coco_selection_preserves_identity_and_rejects_unsupported_inputs() {
        let job = job(storage::ImportProfile::CocoKeypointsGtV1);
        let control = control(client::ImportProfile::CocoKeypointsGtV1);
        let mut seal: client::SealImportRequest = serde_json::from_value(json!({
            "source": {
                "sourceNamespace": "release_set",
                "descriptors": [
                    {
                        "descriptorFileId": "instances",
                        "kind": "coco_instances",
                        "release": "v2",
                        "split": "train",
                        "imageRootFileId": "image",
                        "pairingGroup": "people"
                    },
                    {
                        "descriptorFileId": "descriptor",
                        "kind": "coco_keypoints",
                        "release": "v2",
                        "split": "train",
                        "imageRootFileId": "image",
                        "pairingGroup": "people"
                    }
                ],
                "selectedSplits": ["train"],
                "selectedCategoryKeys": []
            },
            "attestations": {
                "groundTruth": true,
                "exhaustive": true,
                "coverageScope": [],
                "provenance": "fixture"
            }
        }))
        .unwrap();

        let converted = convert_preflight(&job, &control, &seal).unwrap();
        assert_eq!(
            converted.intent,
            storage::ImportIntent::AuthoritativeGroundTruth
        );
        assert_eq!(converted.coco_descriptors.len(), 2);
        let descriptor = &converted.coco_descriptors[1];
        assert_eq!(descriptor.descriptor_path, "annotations/keypoints.json");
        assert_eq!(descriptor.image_root, "images");
        assert_eq!(descriptor.split, "train");
        assert_eq!(descriptor.source_namespace, "release_set");
        assert_eq!(descriptor.release, "v2");
        assert_eq!(
            descriptor.kind,
            labello_domain::ImportDescriptorKind::CocoKeypoints
        );
        assert_eq!(descriptor.pairing_group.as_deref(), Some("people"));

        let mut non_exhaustive_control = control.clone();
        non_exhaustive_control
            .create_request
            .attestations
            .exhaustive = false;
        let mut non_exhaustive_seal = seal.clone();
        non_exhaustive_seal.attestations.exhaustive = false;
        let converted =
            convert_preflight(&job, &non_exhaustive_control, &non_exhaustive_seal).unwrap();
        assert_eq!(converted.intent, storage::ImportIntent::RequireApproval);

        seal.source.descriptors[0].kind = client::ImportDescriptorKind::YoloDataset;
        assert!(convert_preflight(&job, &control, &seal).is_err());
        seal.source.descriptors[0].kind = client::ImportDescriptorKind::CocoInstances;
        seal.source.selected_category_keys = vec!["release_set:v2:7".to_string()];
        assert!(convert_preflight(&job, &control, &seal).is_err());
    }

    fn current_preflight() -> storage::PreflightRequest {
        storage::PreflightRequest {
            descriptor_paths: vec!["dataset.yaml".to_string()],
            selected_splits: vec!["train".to_string()],
            coco_descriptors: Vec::new(),
            ground_truth_attested: true,
            exhaustive_attested: true,
            source_namespace: "fixture".to_string(),
            source_release: "v1".to_string(),
            coverage_scope: vec!["person".to_string()],
            attestation_provenance: "fixture".to_string(),
            intent: storage::ImportIntent::AuthoritativeGroundTruth,
            policies: storage::CompatibilityPolicies::default(),
            output: storage::OutputPolicy::defaults_for(
                storage::ImportProfile::UltralyticsYoloDetectV1,
            ),
            acknowledged_warning_codes: Vec::new(),
            category_mappings: Vec::new(),
            task_mappings: Vec::new(),
            geometry_mappings: Vec::new(),
        }
    }

    fn valid_mapping_json() -> serde_json::Value {
        json!({
            "categoryMappings": [{
                "sourceCategoryKey": "0", "sourceCategoryId": "0",
                "classId": "person", "className": "Person", "color": "#123456",
                "selected": true
            }],
            "geometryMappings": [{
                "sourceCategoryKey": "0", "sourceGeometry": "bounding_box",
                "targetGeometry": "bounding_box", "policy": "direct", "parameters": []
            }],
            "taskMappings": [{
                "sourceCategoryKey": "0",
                "task": {
                    "taskId": "person-box", "name": "Person boxes",
                    "annotationType": "bounding_box", "classIds": ["person"],
                    "instructions": {"title": "Boxes", "exampleText": "Draw", "exampleImages": []},
                    "skeleton": null,
                    "review": {"requiredReviews": 0, "workflow": "none", "allowReviewerCorrections": false, "agreementThreshold": null},
                    "prelabelConfigIds": [], "manualBoxGuideMigration": null, "enabled": true
                },
                "workflowIntent": "authoritative_ground_truth"
            }],
            "skeletonMappings": [], "compatibility": {}, "acknowledgements": []
        })
    }

    #[test]
    fn plan_mapping_validation_rejects_malicious_or_unrepresentable_shapes() {
        let valid: client::UpdateImportPlanRequest =
            serde_json::from_value(valid_mapping_json()).unwrap();
        assert!(convert_plan_update(current_preflight(), valid).is_ok());

        let mut wrong_class = valid_mapping_json();
        wrong_class["taskMappings"][0]["task"]["classIds"] = json!(["other"]);
        let request = serde_json::from_value(wrong_class).unwrap();
        assert!(convert_plan_update(current_preflight(), request).is_err());

        let mut duplicate_category = valid_mapping_json();
        let duplicate = duplicate_category["categoryMappings"][0].clone();
        duplicate_category["categoryMappings"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        let request = serde_json::from_value(duplicate_category).unwrap();
        assert!(convert_plan_update(current_preflight(), request).is_err());

        let mut wrong_workflow = valid_mapping_json();
        wrong_workflow["taskMappings"][0]["task"]["review"]["workflow"] = json!("approval");
        wrong_workflow["taskMappings"][0]["task"]["review"]["requiredReviews"] = json!(1);
        let request = serde_json::from_value(wrong_workflow).unwrap();
        assert!(convert_plan_update(current_preflight(), request).is_err());

        let mut envelope = valid_mapping_json();
        envelope["geometryMappings"][0]["sourceGeometry"] = json!("skeleton");
        envelope["geometryMappings"][0]["policy"] = json!("keypoint_envelope_v1");
        let request = serde_json::from_value(envelope).unwrap();
        assert!(convert_plan_update(current_preflight(), request).is_err());

        let mut template = valid_mapping_json();
        template["geometryMappings"][0]["targetGeometry"] = json!("skeleton");
        template["geometryMappings"][0]["policy"] = json!("box_relative_template_v1");
        let request = serde_json::from_value(template).unwrap();
        assert!(convert_plan_update(current_preflight(), request).is_err());

        let mut manual_without_guide = valid_mapping_json();
        manual_without_guide["geometryMappings"][0]["targetGeometry"] = json!("skeleton");
        manual_without_guide["geometryMappings"][0]["policy"] = json!("manual_box_guide_v1");
        let request = serde_json::from_value(manual_without_guide).unwrap();
        assert!(convert_plan_update(current_preflight(), request).is_err());
    }

    #[test]
    fn plan_mapping_converts_envelope_and_exact_named_template_parameters() {
        let mut envelope = valid_mapping_json();
        envelope["geometryMappings"][0]["sourceGeometry"] = json!("skeleton");
        envelope["geometryMappings"][0]["policy"] = json!("keypoint_envelope_v1");
        envelope["geometryMappings"][0]["parameters"] = json!([
            {"name": "padding", "value": 0.05},
            {"name": "minimumPixels", "value": 1.0},
            {"name": "includeHidden", "value": true}
        ]);
        let converted = convert_plan_update(
            current_preflight(),
            serde_json::from_value(envelope).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            converted.geometry_mappings[0].policy,
            labello_domain::ImportGeometryPolicy::KeypointEnvelopeV1 {
                padding_ratio: 0.05,
                minimum_pixels: 1,
                include_hidden: true
            }
        ));

        let mut template = valid_mapping_json();
        template["geometryMappings"][0]["targetGeometry"] = json!("skeleton");
        template["geometryMappings"][0]["policy"] = json!("box_relative_template_v1");
        template["geometryMappings"][0]["parameters"] = json!([
            {"name": "nose", "x": 0.5, "y": 0.25, "state": "visible"},
            {"name": "tail", "x": 0.5, "y": 0.75, "state": "hidden"}
        ]);
        template["taskMappings"][0]["task"]["taskId"] = json!("person-skeleton");
        template["taskMappings"][0]["task"]["name"] = json!("Person skeleton");
        template["taskMappings"][0]["task"]["annotationType"] = json!("skeleton");
        template["taskMappings"][0]["task"]["skeleton"] = json!({
            "keypoints": [
                {"name": "nose", "required": false},
                {"name": "tail", "required": false}
            ],
            "edges": [{"from": "nose", "to": "tail"}],
            "allowHidden": true,
            "allowAbsent": true
        });
        template["skeletonMappings"] = json!([{
            "sourceCategoryKey": "0",
            "targetTaskId": "person-skeleton",
            "skeleton": template["taskMappings"][0]["task"]["skeleton"].clone(),
            "sourceKeypointNames": [],
            "namesConfirmed": true
        }]);
        let converted = convert_plan_update(
            current_preflight(),
            serde_json::from_value(template).unwrap(),
        )
        .unwrap();
        let labello_domain::ImportGeometryPolicy::BoxRelativeTemplateV1 { keypoints } =
            &converted.geometry_mappings[0].policy
        else {
            panic!("expected template policy");
        };
        assert_eq!(
            keypoints
                .iter()
                .map(|point| point.name.as_str())
                .collect::<Vec<_>>(),
            ["nose", "tail"]
        );

        let mut invalid = valid_mapping_json();
        invalid["geometryMappings"][0]["sourceGeometry"] = json!("skeleton");
        invalid["geometryMappings"][0]["policy"] = json!("keypoint_envelope_v1");
        invalid["geometryMappings"][0]["parameters"] = json!([
            {"name": "padding", "value": "NaN"},
            {"name": "minimumPixels", "value": 0.5},
            {"name": "includeHidden", "value": true}
        ]);
        assert!(
            serde_json::from_value::<client::UpdateImportPlanRequest>(invalid).is_err(),
            "non-numeric public parameters must be rejected during decoding"
        );
    }

    #[test]
    fn plan_mapping_allows_independent_manual_categories() {
        let mut request = valid_mapping_json();
        request["categoryMappings"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "sourceCategoryKey": "1", "sourceCategoryId": "1",
                "classId": "car", "className": "Car", "color": "#654321",
                "selected": true
            }));
        request["geometryMappings"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "sourceCategoryKey": "0", "sourceGeometry": "bounding_box",
                "targetGeometry": "skeleton", "policy": "manual_box_guide_v1",
                "parameters": []
            }));
        request["geometryMappings"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "sourceCategoryKey": "1", "sourceGeometry": "bounding_box",
                "targetGeometry": "bounding_box", "policy": "direct",
                "parameters": []
            }));
        request["taskMappings"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "sourceCategoryKey": "0",
                "task": {
                    "taskId": "person-skeleton", "name": "Person skeleton",
                    "annotationType": "skeleton", "classIds": ["person"],
                    "instructions": {"title": "Skeleton", "exampleText": "Draw", "exampleImages": []},
                    "skeleton": {
                        "keypoints": [{"name": "center", "required": false}],
                        "edges": [], "allowHidden": false, "allowAbsent": true
                    },
                    "review": {"requiredReviews": 0, "workflow": "none", "allowReviewerCorrections": false, "agreementThreshold": null},
                    "prelabelConfigIds": [],
                    "manualBoxGuideMigration": {
                        "guideTaskId": "person-box", "cardinality": "exactly_one",
                        "allowExclusion": true, "sequence": "imported_spatial_order_v1"
                    },
                    "enabled": true
                },
                "workflowIntent": "authoritative_ground_truth"
            }));
        request["taskMappings"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "sourceCategoryKey": "1",
                "task": {
                    "taskId": "car-box", "name": "Car boxes",
                    "annotationType": "bounding_box", "classIds": ["car"],
                    "instructions": {"title": "Boxes", "exampleText": "Draw", "exampleImages": []},
                    "skeleton": null,
                    "review": {"requiredReviews": 0, "workflow": "none", "allowReviewerCorrections": false, "agreementThreshold": null},
                    "prelabelConfigIds": [], "manualBoxGuideMigration": null, "enabled": true
                },
                "workflowIntent": "authoritative_ground_truth"
            }));
        request["skeletonMappings"] = json!([{
            "sourceCategoryKey": "0", "targetTaskId": "person-skeleton",
            "sourceKeypointNames": [], "namesConfirmed": true,
            "skeleton": {
                "keypoints": [{"name": "center", "required": false}],
                "edges": [], "allowHidden": false, "allowAbsent": true
            }
        }]);
        let converted = convert_plan_update(
            current_preflight(),
            serde_json::from_value(request.clone()).unwrap(),
        )
        .unwrap();
        assert_eq!(converted.geometry_mappings.len(), 3);
        assert!(converted.geometry_mappings.iter().any(|mapping| {
            mapping.source_category_key == "0"
                && matches!(
                    mapping.policy,
                    labello_domain::ImportGeometryPolicy::ManualBoxGuideV1
                )
        }));
        assert!(converted.geometry_mappings.iter().any(|mapping| {
            mapping.source_category_key == "1"
                && matches!(mapping.policy, labello_domain::ImportGeometryPolicy::Direct)
        }));

        let mut second_manual = request;
        second_manual["geometryMappings"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "sourceCategoryKey": "1", "sourceGeometry": "bounding_box",
                "targetGeometry": "skeleton", "policy": "manual_box_guide_v1",
                "parameters": []
            }));
        let mut skeleton_task = second_manual["taskMappings"][1].clone();
        skeleton_task["sourceCategoryKey"] = json!("1");
        skeleton_task["task"]["taskId"] = json!("car-skeleton");
        skeleton_task["task"]["name"] = json!("Car skeleton");
        skeleton_task["task"]["classIds"] = json!(["car"]);
        skeleton_task["task"]["manualBoxGuideMigration"]["guideTaskId"] = json!("car-box");
        second_manual["taskMappings"]
            .as_array_mut()
            .unwrap()
            .push(skeleton_task);
        let mut skeleton_mapping = second_manual["skeletonMappings"][0].clone();
        skeleton_mapping["sourceCategoryKey"] = json!("1");
        skeleton_mapping["targetTaskId"] = json!("car-skeleton");
        second_manual["skeletonMappings"]
            .as_array_mut()
            .unwrap()
            .push(skeleton_mapping);
        let shared_schema = convert_plan_update(
            current_preflight(),
            serde_json::from_value(second_manual.clone()).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            shared_schema.output.box_to_skeleton,
            storage::BoxToSkeletonPolicy::ManualBoxGuide { .. }
        ));

        second_manual["taskMappings"][3]["task"]["skeleton"]["keypoints"] =
            json!([{"name": "wheel", "required": false}]);
        second_manual["skeletonMappings"][1]["skeleton"]["keypoints"] =
            json!([{"name": "wheel", "required": false}]);
        let converted = convert_plan_update(
            current_preflight(),
            serde_json::from_value(second_manual).unwrap(),
        )
        .unwrap();
        assert_eq!(converted.geometry_mappings.len(), 4);
        assert_eq!(converted.task_mappings.len(), 4);
        assert!(matches!(
            converted.output.box_to_skeleton,
            storage::BoxToSkeletonPolicy::None
        ));
        for (category, guide, target) in [
            ("0", "person-box", "person-skeleton"),
            ("1", "car-box", "car-skeleton"),
        ] {
            let target = converted
                .task_mappings
                .iter()
                .find(|mapping| {
                    mapping.source_category_key == category
                        && mapping.task.task_id == labello_domain::TaskId::from(target)
                })
                .unwrap();
            assert_eq!(
                target
                    .task
                    .manual_box_guide_migration
                    .as_ref()
                    .unwrap()
                    .guide_task_id,
                labello_domain::TaskId::from(guide)
            );
        }
    }

    fn report_plan() -> storage::ImportPlan {
        storage::ImportPlan {
            schema_version: SCHEMA_VERSION,
            import_id: ImportId::from("imp_report"),
            destination_dataset_id: DatasetId::from("report"),
            source_fingerprint: "source".to_string(),
            plan_hash: "plan".to_string(),
            request: current_preflight(),
            totals: storage::ImportTotals {
                source_files: 3,
                source_bytes: 30,
                descriptors: 1,
                images: 2,
                categories: 1,
                source_objects: 7,
                keypoints: 0,
                direct_boxes: 7,
                direct_skeletons: 0,
                derived_geometry: 0,
                clipped_geometry: 0,
                envelope_derived: 0,
                template_derived: 0,
                output_tasks: 1,
                output_annotations: 7,
                estimated_output_bytes: 1_000,
            },
            coverage: labello_domain::ImportCoverageTotals {
                bounding_boxes: labello_domain::ImportCoverageCounts {
                    complete: 1,
                    verified_empty: 1,
                    incomplete: 2,
                    excluded: 3,
                },
                skeletons: Default::default(),
            },
            diagnostics: vec![storage::ImportDiagnostic {
                code: "bad_rows".to_string(),
                severity: storage::DiagnosticSeverity::Error,
                profile: storage::ImportProfile::UltralyticsYoloDetectV1,
                count: 7,
                summary: "bad rows".to_string(),
                blocks_commit: true,
                requires_acknowledgement: true,
                changes_coverage: false,
                examples: Vec::new(),
            }],
            source_categories: BTreeMap::from([(
                "0".to_string(),
                storage::ImportSourceCategory {
                    source_namespace: "fixture".to_string(),
                    source_category_id: "0".to_string(),
                    source_name: "Person".to_string(),
                    source_supercategory: None,
                    direct_bounding_boxes: true,
                    direct_skeletons: false,
                    keypoint_names: Vec::new(),
                    edges: Vec::new(),
                    allow_hidden: false,
                },
            )]),
            class_ids: BTreeMap::from([("0".to_string(), "person".to_string())]),
            task_ids: BTreeMap::from([("0".to_string(), vec!["person-box".to_string()])]),
        }
    }

    #[test]
    fn reports_and_diagnostic_pages_use_storage_occurrence_counts() {
        let plan = report_plan();
        let report = convert_report(&plan);
        assert_eq!(report.blocking_diagnostics, 7);
        assert_eq!(report.required_acknowledgements, 7);
        assert_eq!(report.output.events, 2);
        assert_eq!(report.output.temporary_bytes, 1_000);
        assert_eq!(report.source.objects, 7);
        assert_eq!(report.coverage.complete, 1);
        assert_eq!(report.coverage.verified_empty, 1);
        assert_eq!(report.coverage.incomplete, 2);
        assert_eq!(report.coverage.excluded, 3);
        assert_eq!(report.coverage_by_geometry.bounding_boxes, report.coverage);
        assert_eq!(
            report.coverage_by_geometry.skeletons,
            client::ImportCoverageCounts::default()
        );
        assert_eq!(report.output.required_free_bytes, 64 * 1024 * 1024 + 1_100);

        let query = client::ImportDiagnosticsQuery {
            cursor: Some("2".to_string()),
            limit: 3,
            code: None,
            severity: None,
        };
        let page = diagnostic_page(&plan, &query, 2);
        assert_eq!(page.total, 7);
        assert_eq!(page.diagnostics.len(), 3);
        assert_eq!(page.next_cursor.as_deref(), Some("5"));
        assert_eq!(page.diagnostics[0].diagnostic_id, "bad_rows:2");

        let mut bad_source_id = valid_mapping_json();
        bad_source_id["categoryMappings"][0]["sourceCategoryId"] = json!("forged");
        let request = serde_json::from_value(bad_source_id).unwrap();
        assert!(validate_plan_update_against_current(&plan, &request).is_err());

        let mut bad_acknowledgement = valid_mapping_json();
        bad_acknowledgement["acknowledgements"] = json!([{
            "diagnosticCode": "bad_rows",
            "policy": "accept",
            "affectedCount": 6,
            "acknowledged": true
        }]);
        let request = serde_json::from_value(bad_acknowledgement).unwrap();
        assert!(validate_plan_update_against_current(&plan, &request).is_err());
    }

    #[tokio::test]
    async fn capabilities_filter_server_roots_for_each_actor() {
        let datasets = tempfile::tempdir().unwrap();
        let admin_root = tempfile::tempdir().unwrap();
        let other_root = tempfile::tempdir().unwrap();
        let public_root = tempfile::tempdir().unwrap();
        let service = storage::ImportService::new(
            datasets.path(),
            storage::ImportConfig {
                enabled: true,
                import_roots: vec![
                    storage::ImportRoot {
                        root_id: "admin".to_string(),
                        path: admin_root.path().to_path_buf(),
                        allowed_owners: vec![UserId::from("admin")],
                    },
                    storage::ImportRoot {
                        root_id: "other".to_string(),
                        path: other_root.path().to_path_buf(),
                        allowed_owners: vec![UserId::from("other")],
                    },
                    storage::ImportRoot {
                        root_id: "public".to_string(),
                        path: public_root.path().to_path_buf(),
                        allowed_owners: Vec::new(),
                    },
                ],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let fail_closed = ApiState::new(datasets.path()).with_import_service(service.clone());
        assert!(
            convert_capabilities(&fail_closed, &UserId::from("admin"))
                .server_roots
                .is_empty()
        );
        let state = ApiState::new(datasets.path())
            .with_import_service(service)
            .with_import_root_owners([
                ("admin".to_string(), BTreeSet::from([UserId::from("admin")])),
                ("other".to_string(), BTreeSet::from([UserId::from("other")])),
                ("public".to_string(), BTreeSet::new()),
            ]);

        let admin = convert_capabilities(&state, &UserId::from("admin"));
        assert_eq!(
            admin
                .server_roots
                .iter()
                .map(|root| root.root_id.as_str())
                .collect::<Vec<_>>(),
            ["admin", "public"]
        );
        let other = convert_capabilities(&state, &UserId::from("other"));
        assert_eq!(
            other
                .server_roots
                .iter()
                .map(|root| root.root_id.as_str())
                .collect::<Vec<_>>(),
            ["other", "public"]
        );
    }
}
