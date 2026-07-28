#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_idempotency_keys_remain_unique_when_request_counters_restart() {
        let first = import_key("plan", 1);
        let second = import_key("plan", 1);

        assert_ne!(first, second);
        for key in [first, second] {
            assert!(key.starts_with("ui-plan-"));
            assert!(key.ends_with("-1"));
            assert!(!key.is_empty() && key.len() <= 200);
            assert!(key.bytes().all(|byte| byte.is_ascii_graphic()));
        }
    }

    #[test]
    fn import_flow_defaults_to_yolo_detect_with_a_matching_descriptor() {
        let flow = ImportFlowState::default();

        assert_eq!(flow.profile, ImportProfile::UltralyticsYoloDetectV1);
        assert_eq!(flow.descriptors.len(), 1);
        assert_eq!(flow.descriptors[0].kind, ImportDescriptorKind::YoloDataset);
    }

    #[test]
    fn diagnostic_overview_keeps_blocking_severity_visible_when_collapsed() {
        let diagnostics = vec![
            labello_client::ImportDiagnosticSummary {
                severity: ImportDiagnosticSeverity::Error,
                count: 3,
                impact: labello_client::ImportDiagnosticImpact {
                    blocks_commit: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            labello_client::ImportDiagnosticSummary {
                code: "warning".to_string(),
                severity: ImportDiagnosticSeverity::WarningRequiresAck,
                count: 2,
                impact: labello_client::ImportDiagnosticImpact {
                    requires_acknowledgement: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            labello_client::ImportDiagnosticSummary {
                severity: ImportDiagnosticSeverity::Info,
                count: 1,
                ..Default::default()
            },
        ];
        let overview =
            ImportDiagnosticOverview::from_diagnostics(&diagnostics, &Default::default());

        assert_eq!(
            overview.disclosure_label(false),
            "Diagnostics — 1 error, 1 warning, 1 info · 6 affected · 1 blocking diagnostic · 1 acknowledgement required"
        );
        assert_eq!(
            overview.disclosure_label(true),
            "Diagnostics (1 error, 1 warning, 1 info) · commit blocked"
        );
        assert_eq!(overview.color(), theme::DANGER);

        let acknowledged = std::collections::BTreeSet::from([diagnostics[1].code.clone()]);
        let acknowledged_overview =
            ImportDiagnosticOverview::from_diagnostics(&diagnostics, &acknowledged);
        assert!(
            !acknowledged_overview
                .disclosure_label(false)
                .contains("acknowledgement required")
        );
    }

    #[test]
    fn progress_overview_projects_each_screen_to_one_current_stage() {
        for (screen, current) in [
            (ImportScreen::Source, ImportStage::Source),
            (ImportScreen::Configure, ImportStage::Configure),
            (ImportScreen::Preflight, ImportStage::Preflight),
            (ImportScreen::Ready, ImportStage::Ready),
            (ImportScreen::Running, ImportStage::Import),
        ] {
            let flow = ImportFlowState {
                screen,
                ..Default::default()
            };
            for stage in ImportStage::ALL {
                let expected = match stage.index().cmp(&current.index()) {
                    std::cmp::Ordering::Less => ImportStageStatus::Complete,
                    std::cmp::Ordering::Equal => ImportStageStatus::Active,
                    std::cmp::Ordering::Greater => ImportStageStatus::Pending,
                };
                assert_eq!(import_stage_status(&flow, stage), expected);
            }
        }

        let success = ImportFlowState {
            screen: ImportScreen::Success,
            ..Default::default()
        };
        assert!(
            ImportStage::ALL.into_iter().all(|stage| {
                import_stage_status(&success, stage) == ImportStageStatus::Complete
            })
        );

        let failure = ImportFlowState {
            screen: ImportScreen::Failure,
            ..Default::default()
        };
        assert_eq!(
            import_stage_status(&failure, ImportStage::Source),
            ImportStageStatus::Failed
        );
    }

    #[test]
    fn activity_descriptions_use_redacted_route_templates() {
        assert_eq!(
            ImportActivity::Commit.operation(),
            "POST /imports/{import_id}/commit"
        );
        assert_eq!(
            ImportActivity::UploadChunk.operation(),
            "POST /imports/{import_id}/files/{file_id}/chunks"
        );
        assert!(!ImportActivity::Commit.operation().contains("imp_"));
    }

    #[test]
    fn queued_import_activity_is_cleared_with_the_import_epoch() {
        let mut app = LabelloApp::default();
        let request = app.import_request_identity(None);
        let request_id = request.request_id;

        assert!(app.queue_command(UiCommand::ImportCapabilities { request }));
        assert_eq!(
            app.import.active_operations.get(&request_id),
            Some(&ImportActivity::CheckCapabilities)
        );

        app.begin_import_epoch();
        assert!(app.import.active_operations.is_empty());
        assert!(app.runtime.commands.is_empty());
    }

    fn category(key: &str, source_id: &str, class_id: &str) -> ImportCategoryDraft {
        ImportCategoryDraft {
            selected: true,
            source_category_key: key.to_string(),
            source_category_id: source_id.to_string(),
            source_name: "Person".to_string(),
            class_id: class_id.to_string(),
            class_name: "Person".to_string(),
            class_color: "#5eead4".to_string(),
            bounding_box_task_id: format!("bounding_box:{class_id}"),
            bounding_box_task_name: "Person bounding boxes".to_string(),
            skeleton_task_id: format!("skeleton:{class_id}"),
            skeleton_task_name: "Person skeletons".to_string(),
            source_skeleton: Some(SkeletonSpec {
                keypoints: vec![labello_domain::KeypointSpec {
                    name: "nose".to_string(),
                    required: false,
                }],
                edges: Vec::new(),
                allow_hidden: true,
                allow_absent: true,
            }),
            direct_geometry: vec![
                ImportGeometryKind::BoundingBox,
                ImportGeometryKind::Skeleton,
            ],
            geometry_mappings: vec![
                ImportGeometryMappingRequest {
                    source_category_key: key.to_string(),
                    source_geometry: ImportGeometryKind::BoundingBox,
                    target_geometry: ImportGeometryKind::BoundingBox,
                    policy: ImportGeometryPolicy::Direct,
                    parameters: Vec::new(),
                },
                ImportGeometryMappingRequest {
                    source_category_key: key.to_string(),
                    source_geometry: ImportGeometryKind::Skeleton,
                    target_geometry: ImportGeometryKind::Skeleton,
                    policy: ImportGeometryPolicy::Direct,
                    parameters: Vec::new(),
                },
            ],
            task_mappings: Vec::new(),
            skeleton_mappings: Vec::new(),
            workflow_intent: ImportWorkflowIntent::AuthoritativeGroundTruth,
            target_keypoint_names: "nose".to_string(),
        }
    }

    fn manual_category(
        key: &str,
        source_id: &str,
        class_id: &str,
        keypoints: &str,
    ) -> ImportCategoryDraft {
        let mut category = category(key, source_id, class_id);
        category.direct_geometry = vec![ImportGeometryKind::BoundingBox];
        category.source_skeleton = None;
        category.target_keypoint_names = keypoints.to_string();
        category.geometry_mappings = vec![
            ImportGeometryMappingRequest {
                source_category_key: category.source_category_key.clone(),
                source_geometry: ImportGeometryKind::BoundingBox,
                target_geometry: ImportGeometryKind::BoundingBox,
                policy: ImportGeometryPolicy::Direct,
                parameters: Vec::new(),
            },
            ImportGeometryMappingRequest {
                source_category_key: category.source_category_key.clone(),
                source_geometry: ImportGeometryKind::BoundingBox,
                target_geometry: ImportGeometryKind::Skeleton,
                policy: ImportGeometryPolicy::ManualBoxGuideV1,
                parameters: Vec::new(),
            },
        ];
        category
    }

    fn validation_app(categories: Vec<ImportCategoryDraft>) -> LabelloApp {
        let report = labello_client::ImportPreflightReport {
            source: labello_client::ImportSourceCounts {
                categories: categories.len() as u64,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut app = LabelloApp::default();
        app.import.profile = ImportProfile::CocoInstancesGtV1;
        app.import.exhaustive = true;
        app.import.categories = categories;
        app.import.job = Some(ImportJob {
            import_id: labello_domain::ImportId::from("imp-validation"),
            owner_user_id: labello_domain::UserId::from("admin"),
            destination_dataset_id: DatasetId::from("imported"),
            destination_name: "Imported".to_string(),
            profile: app.import.profile,
            transport: ImportTransport::ServerDirectory,
            lifecycle: ImportLifecycle::AwaitingDecision,
            progress: Default::default(),
            failure: None,
            source_fingerprint: Some("source".to_string()),
            plan_hash: None,
            preflight_report: Some(report),
            can_cancel: true,
            created_at: labello_domain::now(),
            updated_at: labello_domain::now(),
            expires_at: None,
            recovery: None,
        });
        app
    }

    fn has_mapping_issue(
        validation: &ImportMappingValidation,
        category_index: Option<usize>,
        field: ImportMappingField,
        message: &str,
    ) -> bool {
        validation
            .for_field(category_index, field)
            .any(|issue| issue.message.contains(message))
    }

    #[test]
    fn mapping_validation_reports_each_invalid_field_at_its_input() {
        let mut invalid = category("release:person:17", "17", "person");
        invalid.class_id = "bad/class".to_string();
        invalid.class_name.clear();
        invalid.class_color = "teal".to_string();
        invalid.bounding_box_task_id = "bad/task".to_string();
        let app = validation_app(vec![invalid]);

        let validation = app.import_mapping_validation();

        assert!(has_mapping_issue(
            &validation,
            Some(0),
            ImportMappingField::ClassId,
            "safe path segment"
        ));
        assert!(has_mapping_issue(
            &validation,
            Some(0),
            ImportMappingField::ClassName,
            "cannot be empty"
        ));
        assert!(has_mapping_issue(
            &validation,
            Some(0),
            ImportMappingField::ClassColor,
            "#RRGGBB"
        ));
        assert!(has_mapping_issue(
            &validation,
            Some(0),
            ImportMappingField::BoundingBoxTaskId,
            "safe path segment"
        ));
    }

    #[test]
    fn mapping_validation_marks_every_duplicate_owner() {
        let first = category("release:person:17", "17", "person");
        let mut second = category("release:vehicle:18", "18", "person");
        second.bounding_box_task_id = first.bounding_box_task_id.clone();
        let app = validation_app(vec![first, second]);

        let validation = app.import_mapping_validation();

        for index in 0..2 {
            assert!(has_mapping_issue(
                &validation,
                Some(index),
                ImportMappingField::ClassId,
                "unique class IDs"
            ));
            assert!(has_mapping_issue(
                &validation,
                Some(index),
                ImportMappingField::BoundingBoxTaskId,
                "unique task ID"
            ));
        }
    }

    #[test]
    fn mapping_validation_reports_parameter_and_template_schema_errors() {
        let mut envelope = category("release:person:17", "17", "person");
        envelope.geometry_mappings = vec![ImportGeometryMappingRequest {
            source_category_key: envelope.source_category_key.clone(),
            source_geometry: ImportGeometryKind::Skeleton,
            target_geometry: ImportGeometryKind::BoundingBox,
            policy: ImportGeometryPolicy::KeypointEnvelopeV1,
            parameters: vec![
                ImportMappingParameter::Scalar {
                    name: "paddingRatio".to_string(),
                    value: 1.5,
                },
                ImportMappingParameter::Scalar {
                    name: "minimumPixels".to_string(),
                    value: 0.5,
                },
            ],
        }];
        let app = validation_app(vec![envelope]);

        let validation = app.import_mapping_validation();
        let field = ImportMappingField::Geometry(ImportGeometryKind::BoundingBox);

        assert!(has_mapping_issue(
            &validation,
            Some(0),
            field,
            "padding must be between 0 and 1"
        ));
        assert!(has_mapping_issue(
            &validation,
            Some(0),
            field,
            "minimum pixels must be a whole number"
        ));
        assert!(has_mapping_issue(
            &validation,
            Some(0),
            field,
            "hidden keypoints"
        ));

        let mut template = category("release:person:17", "17", "person");
        template.direct_geometry = vec![ImportGeometryKind::BoundingBox];
        template.source_skeleton = None;
        template.target_keypoint_names = "nose".to_string();
        template.geometry_mappings = vec![
            ImportGeometryMappingRequest {
                source_category_key: template.source_category_key.clone(),
                source_geometry: ImportGeometryKind::BoundingBox,
                target_geometry: ImportGeometryKind::BoundingBox,
                policy: ImportGeometryPolicy::Direct,
                parameters: Vec::new(),
            },
            ImportGeometryMappingRequest {
                source_category_key: template.source_category_key.clone(),
                source_geometry: ImportGeometryKind::BoundingBox,
                target_geometry: ImportGeometryKind::Skeleton,
                policy: ImportGeometryPolicy::BoxRelativeTemplateV1,
                parameters: vec![ImportMappingParameter::Point {
                    name: "nose".to_string(),
                    x: 0.5,
                    y: 0.5,
                    state: labello_domain::KeypointState::Absent,
                }],
            },
        ];
        let validation = validation_app(vec![template]).import_mapping_validation();
        assert!(has_mapping_issue(
            &validation,
            Some(0),
            ImportMappingField::Geometry(ImportGeometryKind::Skeleton),
            "At least one template keypoint must be present"
        ));
    }

    #[test]
    fn compatibility_warnings_are_profile_specific() {
        let mut app = validation_app(vec![category("release:person:17", "17", "person")]);
        app.import.coco_crowds = labello_client::CocoCrowdPolicy::Incomplete;
        app.import.geometry_bounds = labello_client::GeometryBoundsPolicy::Clip;
        app.import.yolo_missing_labels =
            labello_client::YoloMissingLabelPolicy::MissingIsBackground;

        let validation = app.import_mapping_validation();

        assert!(has_mapping_issue(
            &validation,
            None,
            ImportMappingField::Compatibility(ImportCompatibilityField::CocoCrowds),
            "coverage incomplete"
        ));
        assert!(has_mapping_issue(
            &validation,
            None,
            ImportMappingField::Compatibility(ImportCompatibilityField::GeometryBounds),
            "clipped"
        ));
        assert!(
            validation
                .for_field(
                    None,
                    ImportMappingField::Compatibility(ImportCompatibilityField::YoloMissingLabels)
                )
                .next()
                .is_none()
        );
    }

    #[test]
    fn seed_confirmation_is_reset_when_its_mapping_scope_changes() {
        let mut flow =
            validation_app(vec![category("release:person:17", "17", "person")]).import;
        flow.categories[0].workflow_intent = ImportWorkflowIntent::SeedFutureAnnotation;
        flow.seed_workflow_confirmation_scope = flow.seed_workflow_scope();
        flow.seed_workflow_confirmed = true;

        flow.categories[0].workflow_intent = ImportWorkflowIntent::RequireApproval;
        flow.sync_seed_workflow_confirmation_scope();

        assert!(!flow.seed_workflow_confirmed);
    }

    #[test]
    fn browser_limits_are_checked_before_file_contents_are_needed() {
        let limits = labello_client::ImportLimits {
            max_browser_files: 2,
            max_browser_bytes: 10,
            max_single_file_bytes: 6,
            ..Default::default()
        };
        assert!(validate_browser_selection_limits(2, 10, [4, 6], &limits).is_ok());
        assert!(validate_browser_selection_limits(3, 10, [3, 3, 4], &limits).is_err());
        assert!(validate_browser_selection_limits(2, 11, [5, 6], &limits).is_err());
        assert!(validate_browser_selection_limits(1, 7, [7], &limits).is_err());
    }

    #[test]
    fn manual_mapping_submits_guide_and_target_tasks_for_every_category() {
        let mut app = LabelloApp::default();
        app.import.capabilities = Some(ImportCapabilities {
            manual_box_guide_migration: true,
            ..Default::default()
        });
        app.import.categories = vec![
            manual_category("release:v2:17", "17", "person", "nose,left_eye"),
            manual_category("release:v2:18", "18", "vehicle", "nose,left_eye"),
        ];

        let request = app.import_plan_request();

        assert_eq!(request.task_mappings.len(), 4);
        let guide = request
            .task_mappings
            .iter()
            .find(|mapping| mapping.task.task_id == TaskId::from("bounding_box:person"))
            .unwrap();
        assert!(guide.task.manual_box_guide_migration.is_none());
        assert_eq!(
            guide.task.review.workflow,
            labello_domain::ReviewWorkflow::None
        );
        let target = request
            .task_mappings
            .iter()
            .find(|mapping| mapping.task.task_id == TaskId::from("skeleton:person"))
            .unwrap();
        assert!(target
            .task
            .manual_box_guide_migration
            .as_ref()
            .is_some_and(|migration| {
                migration.guide_task_id == TaskId::from("bounding_box:person")
            }));
        assert_eq!(
            target.task.review.workflow,
            labello_domain::ReviewWorkflow::Approval
        );
        assert_eq!(target.task.review.required_reviews, 1);
        assert!(request.task_mappings.iter().any(|mapping| {
            mapping.task.task_id == TaskId::from("skeleton:vehicle")
                && mapping
                    .task
                    .manual_box_guide_migration
                    .as_ref()
                    .is_some_and(|migration| {
                        migration.guide_task_id == TaskId::from("bounding_box:vehicle")
                    })
        }));
        assert_eq!(request.geometry_mappings.len(), 4);
        assert_eq!(request.skeleton_mappings.len(), 2);
        assert_eq!(
            request.category_mappings[0].source_category_key,
            "release:v2:17"
        );
        assert!(request.category_mappings[0].selected);
        assert!(
            request.skeleton_mappings[0]
                .source_keypoint_names
                .is_empty()
        );
    }

    #[test]
    fn category_specific_manual_mapping_allows_multiple_categories() {
        let mut app = LabelloApp::default();
        app.import.capabilities = Some(ImportCapabilities {
            manual_box_guide_migration: true,
            ..Default::default()
        });
        let report = labello_client::ImportPreflightReport {
            source: labello_client::ImportSourceCounts {
                categories: 2,
                ..Default::default()
            },
            ..Default::default()
        };
        app.import.plan = Some(ImportPlan {
            report: report.clone(),
            ..Default::default()
        });
        app.import.job = Some(ImportJob {
            import_id: labello_domain::ImportId::from("imp-test"),
            owner_user_id: labello_domain::UserId::from("admin"),
            destination_dataset_id: DatasetId::from("imported"),
            destination_name: "Imported".to_string(),
            profile: ImportProfile::UltralyticsYoloDetectV1,
            transport: ImportTransport::BrowserFolder,
            lifecycle: ImportLifecycle::AwaitingDecision,
            progress: Default::default(),
            failure: None,
            source_fingerprint: Some("source".to_string()),
            plan_hash: Some("plan".to_string()),
            preflight_report: Some(report),
            can_cancel: true,
            created_at: labello_domain::now(),
            updated_at: labello_domain::now(),
            expires_at: None,
            recovery: None,
        });
        let person = manual_category("source:0", "0", "person", "nose");
        let vehicle = manual_category("source:1", "1", "vehicle", "wheel, axle");
        app.import.categories = vec![person, vehicle];
        app.import.exhaustive = true;

        assert!(app.import_mappings_complete());
        let request = app.import_plan_request();
        assert_eq!(request.task_mappings.len(), 4);
        assert_eq!(request.skeleton_mappings.len(), 2);
        for (category, guide, target) in [
            ("source:0", "bounding_box:person", "skeleton:person"),
            ("source:1", "bounding_box:vehicle", "skeleton:vehicle"),
        ] {
            let target = request
                .task_mappings
                .iter()
                .find(|mapping| {
                    mapping.source_category_key == category
                        && mapping.task.task_id == TaskId::from(target)
                })
                .unwrap();
            assert_eq!(
                target
                    .task
                    .manual_box_guide_migration
                    .as_ref()
                    .unwrap()
                    .guide_task_id,
                TaskId::from(guide)
            );
        }

        app.import.diagnostics = vec![labello_client::ImportDiagnostic::default()];
        app.import.diagnostics_cursor = Some("old".to_string());
        app.request_update_import_plan();
        assert!(app.import.plan.is_none());
        assert!(app.import.pending_plan_request.is_some());
        assert!(app.import.diagnostics.is_empty());
        assert!(app.import.diagnostics_cursor.is_none());
    }

    #[test]
    fn recovery_restores_each_manual_category_target_schema() {
        let mut planned = LabelloApp::default();
        planned.import.categories = vec![
            manual_category("source:0", "0", "person", "nose, left_eye"),
            manual_category("source:1", "1", "vehicle", "wheel, axle"),
        ];
        let accepted = planned.import_plan_request();
        let source_categories = planned
            .import
            .categories
            .iter()
            .map(|category| {
                let category_mapping = accepted
                    .category_mappings
                    .iter()
                    .find(|mapping| mapping.source_category_key == category.source_category_key)
                    .unwrap()
                    .clone();
                labello_client::ImportSourceCategory {
                    source_category_key: category.source_category_key.clone(),
                    source_category_id: category.source_category_id.clone(),
                    source_name: category.source_name.clone(),
                    source_supercategory: None,
                    source_namespace: "source".to_string(),
                    direct_geometry: category.direct_geometry.clone(),
                    keypoint_schema: None,
                    generated_category_mapping: category_mapping.clone(),
                    generated_task_mappings: Vec::new(),
                    current_category_mapping: category_mapping,
                    current_geometry_mappings: accepted
                        .geometry_mappings
                        .iter()
                        .filter(|mapping| {
                            mapping.source_category_key == category.source_category_key
                        })
                        .cloned()
                        .collect(),
                    current_task_mappings: accepted
                        .task_mappings
                        .iter()
                        .filter(|mapping| {
                            mapping.source_category_key == category.source_category_key
                        })
                        .cloned()
                        .collect(),
                    current_skeleton_mappings: accepted
                        .skeleton_mappings
                        .iter()
                        .filter(|mapping| {
                            mapping.source_category_key == category.source_category_key
                        })
                        .cloned()
                        .collect(),
                }
            })
            .collect();
        let report = labello_client::ImportPreflightReport {
            source: labello_client::ImportSourceCounts {
                categories: 2,
                ..Default::default()
            },
            ..Default::default()
        };
        let plan = ImportPlan {
            import_id: labello_domain::ImportId::from("imp-recovery"),
            report: report.clone(),
            source_categories,
            accepted_request: Some(accepted.clone()),
            ..Default::default()
        };
        let now = labello_domain::now();
        let job = ImportJob {
            import_id: plan.import_id.clone(),
            owner_user_id: labello_domain::UserId::from("admin"),
            destination_dataset_id: DatasetId::from("recovered"),
            destination_name: "Recovered".to_string(),
            profile: ImportProfile::UltralyticsYoloDetectV1,
            transport: ImportTransport::ServerDirectory,
            lifecycle: ImportLifecycle::AwaitingDecision,
            progress: Default::default(),
            failure: None,
            source_fingerprint: Some("source".to_string()),
            plan_hash: Some("plan".to_string()),
            preflight_report: Some(report),
            can_cancel: true,
            created_at: now,
            updated_at: now,
            expires_at: None,
            recovery: Some(labello_client::ImportRecoveryState {
                attestations: ImportAttestations {
                    ground_truth: true,
                    exhaustive: true,
                    coverage_scope: Vec::new(),
                    provenance: "fixture".to_string(),
                },
                accepted_plan: Some(plan),
                ..Default::default()
            }),
        };

        let mut recovered = LabelloApp::default();
        recovered.import.capabilities = Some(ImportCapabilities {
            manual_box_guide_migration: true,
            ..Default::default()
        });
        recovered.import.hydrate_job_contract(&job);

        assert_eq!(recovered.import.categories.len(), 2);
        assert_eq!(
            recovered
                .import
                .categories
                .iter()
                .map(|category| (
                    category.class_id.as_str(),
                    category.target_keypoint_names.as_str()
                ))
                .collect::<Vec<_>>(),
            vec![("person", "nose, left_eye"), ("vehicle", "wheel, axle")]
        );
        assert_eq!(recovered.import.accepted_plan_request, Some(accepted));
        assert!(recovered.import_mappings_complete());
    }

    #[test]
    fn manual_policy_is_not_offered_without_server_capability() {
        let policies = policies_for_mapping(
            ImportGeometryKind::BoundingBox,
            ImportGeometryKind::Skeleton,
            &[ImportGeometryKind::BoundingBox],
            false,
        );

        assert!(!policies.contains(&ImportGeometryPolicy::ManualBoxGuideV1));
    }

    #[test]
    fn mapped_tasks_match_api_review_validation_for_every_intent() {
        for (intent, workflow, required_reviews) in [
            (
                ImportWorkflowIntent::AuthoritativeGroundTruth,
                labello_domain::ReviewWorkflow::None,
                0,
            ),
            (
                ImportWorkflowIntent::RequireApproval,
                labello_domain::ReviewWorkflow::Approval,
                1,
            ),
            (
                ImportWorkflowIntent::SeedFutureAnnotation,
                labello_domain::ReviewWorkflow::Approval,
                1,
            ),
        ] {
            let task = mapped_task(
                TaskId::from("bounding_box:person"),
                "Person boxes",
                AnnotationType::BoundingBox,
                ClassId::from("person"),
                None,
                None,
                intent,
            );
            assert_eq!(task.review.workflow, workflow);
            assert_eq!(task.review.required_reviews, required_reviews);
            assert!(!task.review.allow_reviewer_corrections);
            assert!(task.review.agreement_threshold.is_none());
        }

        let manual = mapped_task(
            TaskId::from("skeleton:person"),
            "Person skeletons",
            AnnotationType::Skeleton,
            ClassId::from("person"),
            Some(labello_domain::SkeletonSpec {
                keypoints: Vec::new(),
                edges: Vec::new(),
                allow_hidden: true,
                allow_absent: true,
            }),
            Some(labello_domain::ManualBoxGuideMigration {
                guide_task_id: TaskId::from("bounding_box:person"),
                cardinality: labello_domain::MigrationCardinality::ExactlyOne,
                allow_exclusion: true,
                sequence: labello_domain::MigrationSequence::ImportedSpatialOrderV1,
            }),
            ImportWorkflowIntent::AuthoritativeGroundTruth,
        );
        assert_eq!(
            manual.review.workflow,
            labello_domain::ReviewWorkflow::Approval
        );
        assert_eq!(manual.review.required_reviews, 1);
        assert!(!manual.review.allow_reviewer_corrections);
    }

    #[test]
    fn omit_geometry_emits_no_tasks() {
        let mut app = LabelloApp::default();
        app.import.categories = vec![category("source:3", "3", "person")];
        for mapping in &mut app.import.categories[0].geometry_mappings {
            mapping.policy = ImportGeometryPolicy::Omit;
        }

        let request = app.import_plan_request();

        assert!(request.task_mappings.is_empty());
        assert!(request.skeleton_mappings.is_empty());
        assert!(!request.geometry_mappings.is_empty());
        assert!(
            request
                .geometry_mappings
                .iter()
                .all(|mapping| mapping.policy == ImportGeometryPolicy::Omit)
        );
    }

    #[test]
    fn pose_direct_box_and_skeleton_mappings_are_independent() {
        let mut app = LabelloApp::default();
        app.import.profile = ImportProfile::CocoKeypointsGtV1;
        app.import.categories = vec![category("paired:person:17", "17", "person")];

        let both = app.import_plan_request();
        assert_eq!(both.geometry_mappings.len(), 2);
        assert_eq!(both.task_mappings.len(), 2);
        assert_eq!(both.skeleton_mappings.len(), 1);

        app.import.categories[0].geometry_mappings[0].policy = ImportGeometryPolicy::Omit;
        let skeleton_only = app.import_plan_request();
        assert_eq!(skeleton_only.geometry_mappings.len(), 2);
        assert_eq!(skeleton_only.task_mappings.len(), 1);
        assert_eq!(
            skeleton_only.task_mappings[0].task.annotation_type,
            AnnotationType::Skeleton
        );
    }

    #[test]
    fn manual_mapping_uses_only_the_selected_real_category() {
        let mut app = LabelloApp::default();
        app.import.capabilities = Some(ImportCapabilities {
            manual_box_guide_migration: true,
            ..Default::default()
        });
        let selected = manual_category("release:person:17", "17", "person", "nose");
        let mut omitted = manual_category("release:vehicle:91", "91", "vehicle", "nose");
        omitted.selected = false;
        app.import.categories = vec![selected, omitted];

        let request = app.import_plan_request();

        assert_eq!(request.category_mappings.len(), 2);
        assert_eq!(
            request
                .category_mappings
                .iter()
                .filter(|row| row.selected)
                .count(),
            1
        );
        assert_eq!(request.task_mappings.len(), 2);
        assert!(
            request
                .task_mappings
                .iter()
                .all(|mapping| mapping.source_category_key == "release:person:17")
        );
        assert!(
            request.skeleton_mappings[0]
                .source_keypoint_names
                .is_empty()
        );
    }

    #[test]
    fn paired_coco_descriptor_kinds_are_preserved_and_api_validated() {
        let mut app = LabelloApp::default();
        app.import.profile = ImportProfile::CocoKeypointsGtV1;
        app.import.transport = ImportTransport::ServerDirectory;
        app.import.descriptors = vec![
            ImportDescriptorDraft {
                descriptor_file_id: "annotations/instances.json".to_string(),
                kind: ImportDescriptorKind::CocoInstances,
                image_root_file_id: "images/example.jpg".to_string(),
                pairing_group: "people_train".to_string(),
                ..Default::default()
            },
            ImportDescriptorDraft {
                descriptor_file_id: "annotations/keypoints.json".to_string(),
                kind: ImportDescriptorKind::CocoKeypoints,
                image_root_file_id: "images/example.jpg".to_string(),
                pairing_group: "people_train".to_string(),
                ..Default::default()
            },
        ];
        app.import.job = Some(ImportJob {
            import_id: labello_domain::ImportId::from("imp-test"),
            owner_user_id: labello_domain::UserId::from("admin"),
            destination_dataset_id: DatasetId::from("imported"),
            destination_name: "Imported".to_string(),
            profile: app.import.profile,
            transport: app.import.transport,
            lifecycle: ImportLifecycle::Uploading,
            progress: Default::default(),
            failure: None,
            source_fingerprint: None,
            plan_hash: None,
            preflight_report: None,
            can_cancel: true,
            created_at: labello_domain::now(),
            updated_at: labello_domain::now(),
            expires_at: None,
            recovery: None,
        });

        assert!(app.import_descriptors_valid());
        app.request_seal_import();
        assert!(app.import.error.is_none());
        let UiCommand::SealImport { body, .. } = app.runtime.commands.pop_back().unwrap() else {
            panic!("seal command was not queued");
        };
        assert_eq!(
            body.source
                .descriptors
                .iter()
                .map(|descriptor| descriptor.kind)
                .collect::<Vec<_>>(),
            vec![
                ImportDescriptorKind::CocoInstances,
                ImportDescriptorKind::CocoKeypoints
            ]
        );
    }

    #[test]
    fn yolo_seal_uses_one_descriptor_and_all_checked_discovered_splits() {
        let mut app = LabelloApp::default();
        app.import.profile = ImportProfile::UltralyticsYoloDetectV1;
        app.import.transport = ImportTransport::ServerDirectory;
        app.import.descriptors = vec![ImportDescriptorDraft {
            descriptor_file_id: "dataset.yaml".to_string(),
            kind: ImportDescriptorKind::YoloDataset,
            release: "v1".to_string(),
            ..Default::default()
        }];
        app.import.yolo_inspected_descriptor_file_id = Some("dataset.yaml".to_string());
        app.import.yolo_splits = vec![
            ImportYoloSplitDraft {
                name: "train".to_string(),
                usable: true,
                selected: true,
                issue: None,
            },
            ImportYoloSplitDraft {
                name: "val".to_string(),
                usable: true,
                selected: true,
                issue: None,
            },
            ImportYoloSplitDraft {
                name: "test".to_string(),
                usable: false,
                selected: false,
                issue: Some("invalid split".to_string()),
            },
        ];
        app.import.job = Some(ImportJob {
            import_id: labello_domain::ImportId::from("imp-yolo"),
            owner_user_id: labello_domain::UserId::from("admin"),
            destination_dataset_id: DatasetId::from("imported-yolo"),
            destination_name: "Imported YOLO".to_string(),
            profile: app.import.profile,
            transport: app.import.transport,
            lifecycle: ImportLifecycle::Uploading,
            progress: Default::default(),
            failure: None,
            source_fingerprint: None,
            plan_hash: None,
            preflight_report: None,
            can_cancel: true,
            created_at: labello_domain::now(),
            updated_at: labello_domain::now(),
            expires_at: None,
            recovery: None,
        });

        assert!(app.import_descriptors_valid());
        app.request_seal_import();

        let UiCommand::SealImport { body, .. } = app.runtime.commands.pop_back().unwrap() else {
            panic!("seal command was not queued");
        };
        assert_eq!(body.source.descriptors.len(), 1);
        assert_eq!(body.source.descriptors[0].split, "train");
        assert_eq!(body.source.selected_splits, vec!["train", "val"]);
    }

    #[test]
    fn capability_normalization_rejects_unadvertised_profile_and_transport() {
        let mut flow = ImportFlowState {
            profile: ImportProfile::CocoKeypointsGtV1,
            transport: ImportTransport::ServerDirectory,
            server_root_id: "missing".to_string(),
            ..Default::default()
        };
        let capabilities = ImportCapabilities {
            profiles: vec![labello_client::ImportProfileCapability {
                profile: ImportProfile::CocoInstancesGtV1,
                enabled: true,
                ..Default::default()
            }],
            transports: vec![labello_client::ImportTransportCapability {
                transport: ImportTransport::BrowserFolder,
                enabled: true,
                ..Default::default()
            }],
            manual_box_guide_migration: false,
            ..Default::default()
        };

        flow.normalize_capability_selection(&capabilities);

        assert_eq!(flow.profile, ImportProfile::CocoInstancesGtV1);
        assert_eq!(flow.transport, ImportTransport::BrowserFolder);
    }
}
