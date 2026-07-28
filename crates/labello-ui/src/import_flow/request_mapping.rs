impl LabelloApp {
    fn import_attestations(&self) -> ImportAttestations {
        ImportAttestations {
            ground_truth: self.import_flow.ground_truth,
            exhaustive: self.import_flow.exhaustive,
            coverage_scope: split_csv(&self.import_flow.coverage_scope),
            provenance: self.import_flow.provenance.trim().to_string(),
        }
    }

    pub(crate) fn import_plan_request(&self) -> UpdateImportPlanRequest {
        let acknowledgements = self
            .import_flow
            .job
            .as_ref()
            .and_then(|job| job.preflight_report.as_ref())
            .into_iter()
            .flat_map(|report| &report.diagnostics)
            .filter(|diagnostic| self.import_flow.acknowledgements.contains(&diagnostic.code))
            .map(|diagnostic| ImportAcknowledgementRequest {
                diagnostic_code: diagnostic.code.clone(),
                policy: self
                    .import_flow
                    .accepted_plan_request
                    .as_ref()
                    .and_then(|request| {
                        request.acknowledgements.iter().find(|acknowledgement| {
                            acknowledgement.diagnostic_code == diagnostic.code
                        })
                    })
                    .map(|acknowledgement| acknowledgement.policy.clone())
                    .unwrap_or_else(|| "mapping selection".to_string()),
                affected_count: diagnostic.count,
                acknowledged: true,
            })
            .collect();
        let mut task_mappings = Vec::new();
        let mut skeleton_mappings = Vec::new();
        for category in self
            .import_flow
            .categories
            .iter()
            .filter(|category| category.selected)
        {
            let class_id = ClassId::from(category.class_id.trim());
            let active_targets = category
                .geometry_mappings
                .iter()
                .filter(|mapping| mapping.policy != ImportGeometryPolicy::Omit)
                .map(|mapping| mapping.target_geometry)
                .collect::<std::collections::BTreeSet<_>>();
            for stored in category.task_mappings.iter().filter(|mapping| {
                active_targets.contains(&match mapping.task.annotation_type {
                    AnnotationType::BoundingBox => ImportGeometryKind::BoundingBox,
                    AnnotationType::Skeleton => ImportGeometryKind::Skeleton,
                })
            }) {
                let mut mapping = stored.clone();
                mapping.source_category_key = category.source_category_key.trim().to_string();
                mapping.workflow_intent = category.workflow_intent;
                mapping.task.class_ids = vec![class_id.clone()];
                mapping.task.review = review_config(category.workflow_intent);
                match mapping.task.annotation_type {
                    AnnotationType::BoundingBox => {
                        mapping.task.task_id = TaskId::from(category.bounding_box_task_id.trim());
                        mapping.task.name = category.bounding_box_task_name.trim().to_string();
                    }
                    AnnotationType::Skeleton => {
                        mapping.task.task_id = TaskId::from(category.skeleton_task_id.trim());
                        mapping.task.name = category.skeleton_task_name.trim().to_string();
                        let manual = category.geometry_mappings.iter().any(|geometry| {
                            geometry.target_geometry == ImportGeometryKind::Skeleton
                                && geometry.policy == ImportGeometryPolicy::ManualBoxGuideV1
                        });
                        mapping.task.manual_box_guide_migration =
                            manual.then_some(labello_domain::ManualBoxGuideMigration {
                                guide_task_id: TaskId::from(category.bounding_box_task_id.trim()),
                                cardinality: labello_domain::MigrationCardinality::ExactlyOne,
                                allow_exclusion: true,
                                sequence: labello_domain::MigrationSequence::ImportedSpatialOrderV1,
                            });
                    }
                }
                task_mappings.push(mapping);
            }
            if active_targets.contains(&ImportGeometryKind::BoundingBox)
                && !task_mappings.iter().any(|mapping| {
                    mapping.source_category_key == category.source_category_key
                        && mapping.task.annotation_type == AnnotationType::BoundingBox
                })
            {
                task_mappings.push(ImportTaskMappingRequest {
                    source_category_key: category.source_category_key.clone(),
                    task: mapped_task(
                        TaskId::from(category.bounding_box_task_id.trim()),
                        category.bounding_box_task_name.trim(),
                        AnnotationType::BoundingBox,
                        class_id.clone(),
                        None,
                        None,
                        category.workflow_intent,
                    ),
                    workflow_intent: category.workflow_intent,
                });
            }
            if active_targets.contains(&ImportGeometryKind::Skeleton)
                && !task_mappings.iter().any(|mapping| {
                    mapping.source_category_key == category.source_category_key
                        && mapping.task.annotation_type == AnnotationType::Skeleton
                })
            {
                let skeleton = category
                    .source_skeleton
                    .clone()
                    .unwrap_or_else(|| SkeletonSpec {
                        keypoints: split_csv(&category.target_keypoint_names)
                            .into_iter()
                            .map(|name| labello_domain::KeypointSpec {
                                name,
                                required: false,
                            })
                            .collect(),
                        edges: Vec::new(),
                        allow_hidden: true,
                        allow_absent: true,
                    });
                let manual = category.geometry_mappings.iter().any(|mapping| {
                    mapping.target_geometry == ImportGeometryKind::Skeleton
                        && mapping.policy == ImportGeometryPolicy::ManualBoxGuideV1
                });
                task_mappings.push(ImportTaskMappingRequest {
                    source_category_key: category.source_category_key.clone(),
                    task: mapped_task(
                        TaskId::from(category.skeleton_task_id.trim()),
                        category.skeleton_task_name.trim(),
                        AnnotationType::Skeleton,
                        class_id.clone(),
                        Some(skeleton.clone()),
                        manual.then_some(labello_domain::ManualBoxGuideMigration {
                            guide_task_id: TaskId::from(category.bounding_box_task_id.trim()),
                            cardinality: labello_domain::MigrationCardinality::ExactlyOne,
                            allow_exclusion: true,
                            sequence: labello_domain::MigrationSequence::ImportedSpatialOrderV1,
                        }),
                        category.workflow_intent,
                    ),
                    workflow_intent: category.workflow_intent,
                });
                skeleton_mappings.push(labello_client::ImportSkeletonMappingRequest {
                    source_category_key: category.source_category_key.clone(),
                    target_task_id: TaskId::from(category.skeleton_task_id.trim()),
                    source_keypoint_names: if category.geometry_mappings.iter().any(|mapping| {
                        mapping.target_geometry == ImportGeometryKind::Skeleton
                            && mapping.policy == ImportGeometryPolicy::Direct
                    }) {
                        skeleton
                            .keypoints
                            .iter()
                            .map(|point| point.name.clone())
                            .collect()
                    } else {
                        Vec::new()
                    },
                    skeleton,
                    names_confirmed: true,
                });
            }
            for stored in category
                .skeleton_mappings
                .iter()
                .filter(|_| active_targets.contains(&ImportGeometryKind::Skeleton))
            {
                let mut mapping = stored.clone();
                mapping.source_category_key = category.source_category_key.trim().to_string();
                mapping.target_task_id = TaskId::from(category.skeleton_task_id.trim());
                if let Some(task) = task_mappings.iter().find(|task| {
                    task.source_category_key == mapping.source_category_key
                        && task.task.annotation_type == AnnotationType::Skeleton
                }) && let Some(skeleton) = task.task.skeleton.clone()
                {
                    mapping.skeleton = skeleton;
                }
                mapping.source_keypoint_names =
                    if category.geometry_mappings.iter().any(|geometry| {
                        geometry.target_geometry == ImportGeometryKind::Skeleton
                            && geometry.policy == ImportGeometryPolicy::Direct
                    }) {
                        mapping
                            .skeleton
                            .keypoints
                            .iter()
                            .map(|point| point.name.clone())
                            .collect()
                    } else {
                        Vec::new()
                    };
                skeleton_mappings.push(mapping);
            }
        }
        UpdateImportPlanRequest {
            category_mappings: self
                .import_flow
                .categories
                .iter()
                .map(|category| ImportCategoryMappingRequest {
                    source_category_key: category.source_category_key.trim().to_string(),
                    source_category_id: category.source_category_id.trim().to_string(),
                    class_id: ClassId::from(category.class_id.trim()),
                    class_name: category.class_name.trim().to_string(),
                    color: category.class_color.trim().to_string(),
                    selected: category.selected,
                })
                .collect(),
            geometry_mappings: self
                .import_flow
                .categories
                .iter()
                .filter(|category| category.selected)
                .flat_map(|category| {
                    category
                        .geometry_mappings
                        .iter()
                        .cloned()
                        .map(|mut mapping| {
                            mapping.source_category_key =
                                category.source_category_key.trim().to_string();
                            mapping
                        })
                })
                .collect(),
            task_mappings,
            skeleton_mappings,
            compatibility: labello_client::ImportCompatibilityPolicies {
                yolo_missing_labels: self.import_flow.yolo_missing_labels,
                yolo_duplicate_rows: self.import_flow.yolo_duplicate_rows,
                coco_crowds: self.import_flow.coco_crowds,
                coco_structure: self.import_flow.coco_structure,
                geometry_bounds: self.import_flow.geometry_bounds,
                cross_split_duplicates: self.import_flow.cross_split_duplicates,
                missing_keypoint_names: self.import_flow.missing_keypoint_names,
            },
            acknowledgements,
        }
    }
}
