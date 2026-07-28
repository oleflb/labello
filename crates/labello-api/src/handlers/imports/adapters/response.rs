fn client_severity(severity: storage::DiagnosticSeverity) -> client::ImportDiagnosticSeverity {
    match severity {
        storage::DiagnosticSeverity::Error => client::ImportDiagnosticSeverity::Error,
        storage::DiagnosticSeverity::WarningRequiresAck => {
            client::ImportDiagnosticSeverity::WarningRequiresAck
        }
        storage::DiagnosticSeverity::Warning => client::ImportDiagnosticSeverity::Warning,
        storage::DiagnosticSeverity::Info => client::ImportDiagnosticSeverity::Info,
    }
}

fn convert_report(plan: &storage::ImportPlan) -> client::ImportPreflightReport {
    let diagnostics = plan
        .diagnostics
        .iter()
        .map(|diagnostic| client::ImportDiagnosticSummary {
            code: diagnostic.code.clone(),
            severity: client_severity(diagnostic.severity),
            source_profile: client_profile(diagnostic.profile),
            count: diagnostic.count,
            safe_summary: diagnostic.summary.clone(),
            impact: client::ImportDiagnosticImpact {
                blocks_commit: diagnostic.blocks_commit,
                requires_acknowledgement: diagnostic.requires_acknowledgement,
                changes_coverage: diagnostic.changes_coverage,
                discards_metadata: false,
            },
            examples: diagnostic
                .examples
                .iter()
                .map(|example| client::ImportDiagnosticExample {
                    source: Some(convert_source_reference(example)),
                    safe_summary: diagnostic.summary.clone(),
                })
                .collect(),
        })
        .collect();
    client::ImportPreflightReport {
        source_fingerprint: plan.source_fingerprint.clone(),
        plan_hash: Some(plan.plan_hash.clone()),
        source: client::ImportSourceCounts {
            files: plan.totals.source_files as u64,
            bytes: plan.totals.source_bytes,
            descriptors: plan.totals.descriptors as u64,
            splits: plan.request.selected_splits.len() as u64,
            images: plan.totals.images as u64,
            categories: plan.totals.categories as u64,
            objects: plan.totals.source_objects as u64,
            keypoints: plan.totals.keypoints as u64,
        },
        geometry: {
            let source_direct = (plan.totals.direct_boxes + plan.totals.direct_skeletons) as u64;
            client::ImportGeometryCounts {
                direct: source_direct.saturating_sub(plan.totals.clipped_geometry as u64),
                clipped: plan.totals.clipped_geometry as u64,
                template_derived: plan.totals.template_derived as u64,
                envelope_derived: plan.totals.envelope_derived as u64,
                ..Default::default()
            }
        },
        coverage: {
            let boxes = &plan.coverage.bounding_boxes;
            let skeletons = &plan.coverage.skeletons;
            client::ImportCoverageCounts {
                complete: (boxes.complete + skeletons.complete) as u64,
                verified_empty: (boxes.verified_empty + skeletons.verified_empty) as u64,
                incomplete: (boxes.incomplete + skeletons.incomplete) as u64,
                excluded: (boxes.excluded + skeletons.excluded) as u64,
            }
        },
        coverage_by_geometry: client::ImportCoverageByGeometry {
            bounding_boxes: client_coverage_counts(&plan.coverage.bounding_boxes),
            skeletons: client_coverage_counts(&plan.coverage.skeletons),
        },
        output: client::ImportOutputEstimate {
            classes: plan.class_ids.len() as u64,
            tasks: plan.totals.output_tasks as u64,
            annotations: plan.totals.output_annotations as u64,
            events: plan.totals.images as u64,
            output_bytes: plan.totals.estimated_output_bytes,
            temporary_bytes: plan.totals.estimated_output_bytes,
            required_free_bytes: plan
                .totals
                .estimated_output_bytes
                .saturating_add(plan.totals.estimated_output_bytes / 10)
                .saturating_add(64 * 1024 * 1024),
        },
        blocking_diagnostics: plan
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.blocks_commit)
            .map(|diagnostic| diagnostic.count)
            .sum(),
        required_acknowledgements: plan
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.requires_acknowledgement)
            .map(|diagnostic| diagnostic.count)
            .sum(),
        diagnostics,
    }
}

fn client_coverage_counts(
    counts: &labello_domain::ImportCoverageCounts,
) -> client::ImportCoverageCounts {
    client::ImportCoverageCounts {
        complete: counts.complete as u64,
        verified_empty: counts.verified_empty as u64,
        incomplete: counts.incomplete as u64,
        excluded: counts.excluded as u64,
    }
}

fn convert_plan(
    plan: &storage::ImportPlan,
    accepted_request: Option<&client::UpdateImportPlanRequest>,
) -> client::ImportPlan {
    let generated_request = generated_plan_request(plan);
    let current_request = accepted_request
        .cloned()
        .unwrap_or_else(|| current_plan_request(plan, &generated_request));
    let source_categories = plan
        .source_categories
        .iter()
        .map(|(key, source)| {
            let generated_category_mapping = generated_request
                .category_mappings
                .iter()
                .find(|mapping| mapping.source_category_key == *key)
                .expect("generated category mapping")
                .clone();
            let current_category_mapping = current_request
                .category_mappings
                .iter()
                .find(|mapping| mapping.source_category_key == *key)
                .cloned()
                .unwrap_or_else(|| generated_category_mapping.clone());
            client::ImportSourceCategory {
                source_category_key: key.clone(),
                source_category_id: source.source_category_id.clone(),
                source_name: source.source_name.clone(),
                source_supercategory: source.source_supercategory.clone(),
                source_namespace: source.source_namespace.clone(),
                direct_geometry: [
                    source
                        .direct_bounding_boxes
                        .then_some(client::ImportGeometryKind::BoundingBox),
                    source
                        .direct_skeletons
                        .then_some(client::ImportGeometryKind::Skeleton),
                ]
                .into_iter()
                .flatten()
                .collect(),
                keypoint_schema: source_skeleton(source),
                generated_category_mapping,
                generated_task_mappings: generated_request
                    .task_mappings
                    .iter()
                    .filter(|mapping| mapping.source_category_key == *key)
                    .cloned()
                    .collect(),
                current_category_mapping,
                current_geometry_mappings: current_request
                    .geometry_mappings
                    .iter()
                    .filter(|mapping| mapping.source_category_key == *key)
                    .cloned()
                    .collect(),
                current_task_mappings: current_request
                    .task_mappings
                    .iter()
                    .filter(|mapping| mapping.source_category_key == *key)
                    .cloned()
                    .collect(),
                current_skeleton_mappings: current_request
                    .skeleton_mappings
                    .iter()
                    .filter(|mapping| mapping.source_category_key == *key)
                    .cloned()
                    .collect(),
            }
        })
        .collect();
    client::ImportPlan {
        import_id: plan.import_id.clone(),
        source_fingerprint: plan.source_fingerprint.clone(),
        plan_hash: plan.plan_hash.clone(),
        commit_ready: plan.committable(),
        blocking_diagnostic_codes: plan
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.blocks_commit)
            .map(|diagnostic| diagnostic.code.clone())
            .collect(),
        required_acknowledgement_codes: plan
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.requires_acknowledgement
                    && !plan
                        .request
                        .acknowledged_warning_codes
                        .contains(&diagnostic.code)
            })
            .map(|diagnostic| diagnostic.code.clone())
            .collect(),
        report: convert_report(plan),
        source_categories,
        accepted_request: Some(current_request),
    }
}

fn generated_plan_request(plan: &storage::ImportPlan) -> client::UpdateImportPlanRequest {
    let category_mappings = plan
        .source_categories
        .iter()
        .map(|(key, source)| client::ImportCategoryMappingRequest {
            source_category_key: key.clone(),
            source_category_id: source.source_category_id.clone(),
            class_id: labello_domain::ClassId::from(plan.class_ids[key].clone()),
            class_name: source.source_name.clone(),
            color: generated_color(&plan.class_ids[key]),
            selected: plan.class_ids.contains_key(key),
        })
        .collect::<Vec<_>>();
    let task_mappings = plan
        .task_ids
        .iter()
        .flat_map(|(key, task_ids)| {
            task_ids
                .iter()
                .map(move |task_id| client::ImportTaskMappingRequest {
                    source_category_key: key.clone(),
                    task: generated_task(plan, key, task_id),
                    workflow_intent: client_intent(plan.request.intent),
                })
        })
        .collect::<Vec<_>>();
    let geometry_mappings = task_mappings
        .iter()
        .map(|mapping| {
            let kind = geometry_kind(mapping.task.annotation_type.clone());
            client::ImportGeometryMappingRequest {
                source_category_key: mapping.source_category_key.clone(),
                source_geometry: kind,
                target_geometry: kind,
                policy: client::ImportGeometryPolicy::Direct,
                parameters: Vec::new(),
            }
        })
        .collect();
    let skeleton_mappings = task_mappings
        .iter()
        .filter_map(|mapping| {
            let skeleton = mapping.task.skeleton.clone()?;
            Some(client::ImportSkeletonMappingRequest {
                source_category_key: mapping.source_category_key.clone(),
                target_task_id: mapping.task.task_id.clone(),
                source_keypoint_names: plan.source_categories[&mapping.source_category_key]
                    .keypoint_names
                    .clone(),
                skeleton,
                names_confirmed: true,
            })
        })
        .collect();
    client::UpdateImportPlanRequest {
        category_mappings,
        geometry_mappings,
        task_mappings,
        skeleton_mappings,
        compatibility: client_compatibility(&plan.request.policies),
        acknowledgements: Vec::new(),
    }
}

fn current_plan_request(
    plan: &storage::ImportPlan,
    generated: &client::UpdateImportPlanRequest,
) -> client::UpdateImportPlanRequest {
    if plan.request.category_mappings.is_empty() {
        return generated.clone();
    }
    let mut request = generated.clone();
    request.category_mappings = plan
        .request
        .category_mappings
        .iter()
        .map(|mapping| client::ImportCategoryMappingRequest {
            source_category_key: mapping.source_category_key.clone(),
            source_category_id: mapping.source_category_id.clone(),
            class_id: mapping.class_id.clone(),
            class_name: mapping.class_name.clone(),
            color: mapping.color.clone(),
            selected: mapping.selected,
        })
        .collect();
    request.task_mappings = plan
        .request
        .task_mappings
        .iter()
        .map(|mapping| client::ImportTaskMappingRequest {
            source_category_key: mapping.source_category_key.clone(),
            task: mapping.task.clone(),
            workflow_intent: client_intent(mapping.intent),
        })
        .collect();
    request.geometry_mappings = plan
        .request
        .geometry_mappings
        .iter()
        .map(client_geometry_mapping)
        .collect();
    request.skeleton_mappings = request
        .task_mappings
        .iter()
        .filter_map(|mapping| {
            let skeleton = mapping.task.skeleton.clone()?;
            let source_names = if request.geometry_mappings.iter().any(|geometry| {
                geometry.source_category_key == mapping.source_category_key
                    && geometry.policy == client::ImportGeometryPolicy::Direct
                    && geometry.target_geometry == client::ImportGeometryKind::Skeleton
            }) {
                plan.source_categories[&mapping.source_category_key]
                    .keypoint_names
                    .clone()
            } else {
                Vec::new()
            };
            Some(client::ImportSkeletonMappingRequest {
                source_category_key: mapping.source_category_key.clone(),
                target_task_id: mapping.task.task_id.clone(),
                skeleton,
                source_keypoint_names: source_names,
                names_confirmed: true,
            })
        })
        .collect();
    request.compatibility = client_compatibility(&plan.request.policies);
    request.acknowledgements = plan
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            plan.request
                .acknowledged_warning_codes
                .contains(&diagnostic.code)
        })
        .map(|diagnostic| client::ImportAcknowledgementRequest {
            diagnostic_code: diagnostic.code.clone(),
            policy: "accepted".to_string(),
            affected_count: diagnostic.count,
            acknowledged: true,
        })
        .collect();
    request
}

fn generated_task(
    plan: &storage::ImportPlan,
    key: &str,
    task_id: &str,
) -> labello_domain::TaskDefinition {
    let source = &plan.source_categories[key];
    let annotation_type = if task_id.starts_with("bounding_box:") {
        labello_domain::AnnotationType::BoundingBox
    } else {
        labello_domain::AnnotationType::Skeleton
    };
    let skeleton = (annotation_type == labello_domain::AnnotationType::Skeleton)
        .then(|| source_skeleton(source))
        .flatten();
    labello_domain::TaskDefinition {
        task_id: labello_domain::TaskId::from(task_id),
        name: format!(
            "{} {}",
            source.source_name,
            if annotation_type == labello_domain::AnnotationType::BoundingBox {
                "boxes"
            } else {
                "skeletons"
            }
        ),
        annotation_type,
        class_ids: vec![labello_domain::ClassId::from(plan.class_ids[key].clone())],
        instructions: labello_domain::TutorialContent {
            title: format!("Annotate {}", source.source_name),
            example_text:
                "Imported source geometry and coverage are recorded in the audit history."
                    .to_string(),
            example_images: Vec::new(),
        },
        skeleton,
        review: review_for_intent(plan.request.intent),
        prelabel_config_ids: Vec::new(),
        manual_box_guide_migration: None,
        enabled: true,
    }
}

fn source_skeleton(source: &storage::ImportSourceCategory) -> Option<labello_domain::SkeletonSpec> {
    (!source.keypoint_names.is_empty()).then(|| labello_domain::SkeletonSpec {
        keypoints: source
            .keypoint_names
            .iter()
            .map(|name| labello_domain::KeypointSpec {
                name: name.clone(),
                required: false,
            })
            .collect(),
        edges: source
            .edges
            .iter()
            .map(|(from, to)| labello_domain::SkeletonEdge {
                from: from.clone(),
                to: to.clone(),
            })
            .collect(),
        allow_hidden: source.allow_hidden,
        allow_absent: true,
    })
}

fn generated_color(class_id: &str) -> String {
    let digest = blake3::hash(class_id.as_bytes()).to_hex().to_string();
    format!("#{}", &digest[..6])
}

fn client_intent(intent: storage::ImportIntent) -> client::ImportWorkflowIntent {
    match intent {
        storage::ImportIntent::AuthoritativeGroundTruth => {
            client::ImportWorkflowIntent::AuthoritativeGroundTruth
        }
        storage::ImportIntent::RequireApproval => client::ImportWorkflowIntent::RequireApproval,
        storage::ImportIntent::SeedFutureAnnotation => {
            client::ImportWorkflowIntent::SeedFutureAnnotation
        }
    }
}

fn review_for_intent(intent: storage::ImportIntent) -> labello_domain::ReviewConfig {
    match intent {
        storage::ImportIntent::AuthoritativeGroundTruth => labello_domain::ReviewConfig {
            required_reviews: 0,
            workflow: labello_domain::ReviewWorkflow::None,
            allow_reviewer_corrections: false,
            agreement_threshold: None,
        },
        storage::ImportIntent::RequireApproval | storage::ImportIntent::SeedFutureAnnotation => {
            labello_domain::ReviewConfig {
                required_reviews: 1,
                workflow: labello_domain::ReviewWorkflow::Approval,
                allow_reviewer_corrections: false,
                agreement_threshold: None,
            }
        }
    }
}

fn client_compatibility(
    policies: &storage::CompatibilityPolicies,
) -> client::ImportCompatibilityPolicies {
    client::ImportCompatibilityPolicies {
        yolo_missing_labels: match policies.yolo_missing_labels {
            storage::YoloMissingLabelPolicy::Block => client::YoloMissingLabelPolicy::Block,
            storage::YoloMissingLabelPolicy::MissingIsBackground => {
                client::YoloMissingLabelPolicy::MissingIsBackground
            }
            storage::YoloMissingLabelPolicy::RetainIncomplete => {
                client::YoloMissingLabelPolicy::Incomplete
            }
        },
        yolo_duplicate_rows: match policies.yolo_duplicate_rows {
            storage::DuplicateRowPolicy::Block => client::YoloDuplicateRowPolicy::Block,
            storage::DuplicateRowPolicy::Deduplicate => client::YoloDuplicateRowPolicy::Deduplicate,
        },
        coco_crowds: match policies.coco_crowds {
            storage::CocoCrowdPolicy::Block => client::CocoCrowdPolicy::Block,
            storage::CocoCrowdPolicy::Incomplete => client::CocoCrowdPolicy::Incomplete,
            storage::CocoCrowdPolicy::ExcludeImageTask => client::CocoCrowdPolicy::ExcludeImageTask,
        },
        coco_structure: if policies.coco_bbox_only {
            client::CocoStructurePolicy::BboxCompatibility
        } else {
            client::CocoStructurePolicy::Canonical
        },
        geometry_bounds: match policies.geometry_bounds {
            storage::GeometryBoundsPolicy::Block => client::GeometryBoundsPolicy::Reject,
            storage::GeometryBoundsPolicy::ClipDerived => client::GeometryBoundsPolicy::Clip,
        },
        cross_split_duplicates: match policies.cross_split_duplicates {
            storage::CrossSplitDuplicatePolicy::Block => client::CrossSplitDuplicatePolicy::Block,
            storage::CrossSplitDuplicatePolicy::MultipleMemberships => {
                client::CrossSplitDuplicatePolicy::MergeMemberships
            }
        },
        missing_keypoint_names: match policies.yolo_keypoint_names {
            storage::YoloKeypointNamePolicy::RequireSourceNames => {
                client::MissingKeypointNamesPolicy::Block
            }
            storage::YoloKeypointNamePolicy::GenerateIndexed => {
                client::MissingKeypointNamesPolicy::GenerateIndexed
            }
        },
    }
}

fn client_geometry_mapping(
    mapping: &labello_domain::ImportGeometryMapping,
) -> client::ImportGeometryMappingRequest {
    let (policy, parameters) = match &mapping.policy {
        labello_domain::ImportGeometryPolicy::Direct => {
            (client::ImportGeometryPolicy::Direct, Vec::new())
        }
        labello_domain::ImportGeometryPolicy::KeypointEnvelopeV1 {
            padding_ratio,
            minimum_pixels,
            include_hidden,
        } => (
            client::ImportGeometryPolicy::KeypointEnvelopeV1,
            vec![
                client::ImportMappingParameter::Scalar {
                    name: "paddingRatio".to_string(),
                    value: *padding_ratio,
                },
                client::ImportMappingParameter::Scalar {
                    name: "minimumPixels".to_string(),
                    value: f64::from(*minimum_pixels),
                },
                client::ImportMappingParameter::Boolean {
                    name: "includeHidden".to_string(),
                    value: *include_hidden,
                },
            ],
        ),
        labello_domain::ImportGeometryPolicy::ManualBoxGuideV1 => {
            (client::ImportGeometryPolicy::ManualBoxGuideV1, Vec::new())
        }
        labello_domain::ImportGeometryPolicy::BoxRelativeTemplateV1 { keypoints } => (
            client::ImportGeometryPolicy::BoxRelativeTemplateV1,
            keypoints
                .iter()
                .map(|point| client::ImportMappingParameter::Point {
                    name: point.name.clone(),
                    x: point.x,
                    y: point.y,
                    state: point.state.clone(),
                })
                .collect(),
        ),
        labello_domain::ImportGeometryPolicy::Omit => {
            (client::ImportGeometryPolicy::Omit, Vec::new())
        }
    };
    client::ImportGeometryMappingRequest {
        source_category_key: mapping.source_category_key.clone(),
        source_geometry: match mapping.source_geometry {
            labello_domain::ImportGeometryKind::BoundingBox => {
                client::ImportGeometryKind::BoundingBox
            }
            labello_domain::ImportGeometryKind::Skeleton => client::ImportGeometryKind::Skeleton,
        },
        target_geometry: match mapping.target_geometry {
            labello_domain::ImportGeometryKind::BoundingBox => {
                client::ImportGeometryKind::BoundingBox
            }
            labello_domain::ImportGeometryKind::Skeleton => client::ImportGeometryKind::Skeleton,
        },
        policy,
        parameters,
    }
}

fn convert_source_reference(
    example: &storage::DiagnosticExample,
) -> client::ImportDiagnosticSourceReference {
    client::ImportDiagnosticSourceReference {
        relative_path: example.source_path.clone(),
        source_image_id: example.source_image_key.clone(),
        category_id: None,
        annotation_id: example.source_object_key.clone(),
        line: example.line,
    }
}

fn convert_diagnostic(
    diagnostic: &storage::ImportDiagnostic,
    index: u64,
    occurrence: u64,
) -> client::ImportDiagnostic {
    client::ImportDiagnostic {
        diagnostic_id: format!("{}:{index}", diagnostic.code),
        code: diagnostic.code.clone(),
        severity: client_severity(diagnostic.severity),
        source_profile: client_profile(diagnostic.profile),
        safe_summary: diagnostic.summary.clone(),
        impact: client::ImportDiagnosticImpact {
            blocks_commit: diagnostic.blocks_commit,
            requires_acknowledgement: diagnostic.requires_acknowledgement,
            changes_coverage: diagnostic.changes_coverage,
            discards_metadata: false,
        },
        source: usize::try_from(occurrence)
            .ok()
            .and_then(|occurrence| diagnostic.examples.get(occurrence))
            .map(convert_source_reference),
    }
}
