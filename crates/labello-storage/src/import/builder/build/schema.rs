fn build_classes(
    ir: &ImportIr,
    class_ids: &BTreeMap<String, String>,
    request: &PreflightRequest,
) -> Vec<LabelClass> {
    ir.categories
        .iter()
        .filter_map(|(key, category)| {
            let id = class_ids.get(key)?;
            if let Some(mapping) = request
                .category_mappings
                .iter()
                .find(|mapping| mapping.selected && mapping.source_category_key == *key)
            {
                return Some(LabelClass {
                    class_id: mapping.class_id.clone(),
                    name: mapping.class_name.clone(),
                    color: mapping.color.clone(),
                    description: category.supercategory.clone(),
                });
            }
            let digest = blake3::hash(id.as_bytes()).to_hex().to_string();
            Some(LabelClass {
                class_id: ClassId::from(id.clone()),
                name: category.name.clone(),
                color: format!("#{}", &digest[..6]),
                description: category.supercategory.clone(),
            })
        })
        .collect()
}

fn build_tasks(
    ir: &ImportIr,
    plan: &ImportPlan,
    class_ids: &BTreeMap<String, String>,
    task_ids: &BTreeMap<String, Vec<String>>,
) -> StorageResult<Vec<TaskDefinition>> {
    if !plan.request.task_mappings.is_empty() {
        let planned = task_ids
            .values()
            .flatten()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let mut tasks = plan
            .request
            .task_mappings
            .iter()
            .filter(|mapping| planned.contains(mapping.task.task_id.as_str()))
            .map(|mapping| mapping.task.clone())
            .collect::<Vec<_>>();
        tasks.sort_by(|left, right| left.task_id.cmp(&right.task_id));
        if tasks
            .windows(2)
            .any(|pair| pair[0].task_id == pair[1].task_id)
        {
            return Err(import_error(
                "task_mapping_invalid",
                "mapped task IDs must be unique",
            ));
        }
        validate_manual_tasks(&tasks)?;
        return Ok(tasks);
    }
    let mut tasks = Vec::new();
    for (category_key, ids) in task_ids {
        let category = &ir.categories[category_key];
        let class_id = ClassId::from(class_ids[category_key].clone());
        for id in ids {
            let annotation_type = if id.starts_with("bounding_box:") {
                AnnotationType::BoundingBox
            } else {
                AnnotationType::Skeleton
            };
            let skeleton = if annotation_type == AnnotationType::Skeleton {
                Some(skeleton_spec(category, &plan.request.output)?)
            } else {
                None
            };
            let review = match plan.request.intent {
                ImportIntent::AuthoritativeGroundTruth => ReviewConfig {
                    required_reviews: 0,
                    workflow: ReviewWorkflow::None,
                    allow_reviewer_corrections: false,
                    agreement_threshold: None,
                },
                ImportIntent::RequireApproval | ImportIntent::SeedFutureAnnotation => {
                    ReviewConfig {
                        required_reviews: 1,
                        workflow: ReviewWorkflow::Approval,
                        allow_reviewer_corrections: false,
                        agreement_threshold: None,
                    }
                }
            };
            let manual_box_guide_migration = if annotation_type == AnnotationType::Skeleton
                && matches!(
                    plan.request.output.box_to_skeleton,
                    BoxToSkeletonPolicy::ManualBoxGuide { .. }
                ) {
                Some(labello_domain::ManualBoxGuideMigration {
                    guide_task_id: TaskId::from(format!("bounding_box:{class_id}")),
                    cardinality: labello_domain::MigrationCardinality::ExactlyOne,
                    allow_exclusion: true,
                    sequence: labello_domain::MigrationSequence::ImportedSpatialOrderV1,
                })
            } else {
                None
            };
            tasks.push(TaskDefinition {
                task_id: TaskId::from(id.clone()),
                name: format!(
                    "{} {}",
                    category.name,
                    if annotation_type == AnnotationType::BoundingBox {
                        "boxes"
                    } else {
                        "skeletons"
                    }
                ),
                annotation_type,
                class_ids: vec![class_id.clone()],
                instructions: TutorialContent {
                    title: format!("Annotate {}", category.name),
                    example_text:
                        "Imported source geometry and coverage are recorded in the audit history."
                            .to_string(),
                    example_images: Vec::new(),
                },
                skeleton,
                review,
                prelabel_config_ids: Vec::new(),
                manual_box_guide_migration,
                enabled: true,
            });
        }
    }
    tasks.sort_by(|left, right| left.task_id.cmp(&right.task_id));
    validate_manual_tasks(&tasks)?;
    Ok(tasks)
}

fn validate_manual_tasks(tasks: &[TaskDefinition]) -> StorageResult<()> {
    for task in tasks {
        if let Some(config) = &task.manual_box_guide_migration {
            let guide = tasks
                .iter()
                .find(|candidate| candidate.task_id == config.guide_task_id)
                .ok_or_else(|| {
                    import_error(
                        "manual_migration_guide_missing",
                        "manual migration guide task was not generated",
                    )
                })?;
            task.validate_manual_migration(guide)?;
        }
    }
    Ok(())
}

fn planned_task_id(
    request: &PreflightRequest,
    category_key: &str,
    annotation_type: AnnotationType,
    class_id: &ClassId,
) -> Option<TaskId> {
    if request.task_mappings.is_empty() {
        return Some(TaskId::from(format!("{annotation_type}:{class_id}")));
    }
    request
        .task_mappings
        .iter()
        .find(|mapping| {
            mapping.source_category_key == category_key
                && mapping.task.annotation_type == annotation_type
        })
        .map(|mapping| mapping.task.task_id.clone())
}

fn task_intent(request: &PreflightRequest, task_id: &TaskId) -> ImportIntent {
    request
        .task_mappings
        .iter()
        .find(|mapping| mapping.task.task_id == *task_id)
        .map_or(request.intent, |mapping| mapping.intent)
}

fn manifest_intent(intent: ImportIntent) -> labello_domain::ImportTaskIntent {
    match intent {
        ImportIntent::AuthoritativeGroundTruth => {
            labello_domain::ImportTaskIntent::AuthoritativeGroundTruth
        }
        ImportIntent::RequireApproval => labello_domain::ImportTaskIntent::RequireApproval,
        ImportIntent::SeedFutureAnnotation => {
            labello_domain::ImportTaskIntent::SeedFutureAnnotation
        }
    }
}

fn manifest_compatibility(
    policies: &CompatibilityPolicies,
) -> labello_domain::ImportCompatibilityPolicies {
    labello_domain::ImportCompatibilityPolicies {
        yolo_missing_labels: match policies.yolo_missing_labels {
            YoloMissingLabelPolicy::Block => labello_domain::ImportYoloMissingLabelPolicy::Block,
            YoloMissingLabelPolicy::MissingIsBackground => {
                labello_domain::ImportYoloMissingLabelPolicy::MissingIsBackground
            }
            YoloMissingLabelPolicy::RetainIncomplete => {
                labello_domain::ImportYoloMissingLabelPolicy::RetainIncomplete
            }
        },
        yolo_duplicate_rows: match policies.yolo_duplicate_rows {
            DuplicateRowPolicy::Block => labello_domain::ImportDuplicateRowPolicy::Block,
            DuplicateRowPolicy::Deduplicate => {
                labello_domain::ImportDuplicateRowPolicy::Deduplicate
            }
        },
        coco_crowds: match policies.coco_crowds {
            CocoCrowdPolicy::Block => labello_domain::ImportCocoCrowdPolicy::Block,
            CocoCrowdPolicy::Incomplete => labello_domain::ImportCocoCrowdPolicy::Incomplete,
            CocoCrowdPolicy::ExcludeImageTask => {
                labello_domain::ImportCocoCrowdPolicy::ExcludeImageTask
            }
        },
        coco_bbox_only: policies.coco_bbox_only,
        geometry_bounds: match policies.geometry_bounds {
            GeometryBoundsPolicy::Block => labello_domain::ImportGeometryBoundsPolicy::Block,
            GeometryBoundsPolicy::ClipDerived => {
                labello_domain::ImportGeometryBoundsPolicy::ClipDerived
            }
        },
        cross_split_duplicates: match policies.cross_split_duplicates {
            CrossSplitDuplicatePolicy::Block => {
                labello_domain::ImportCrossSplitDuplicatePolicy::Block
            }
            CrossSplitDuplicatePolicy::MultipleMemberships => {
                labello_domain::ImportCrossSplitDuplicatePolicy::MultipleMemberships
            }
        },
        yolo_keypoint_names: match policies.yolo_keypoint_names {
            YoloKeypointNamePolicy::RequireSourceNames => {
                labello_domain::ImportKeypointNamePolicy::RequireSourceNames
            }
            YoloKeypointNamePolicy::GenerateIndexed => {
                labello_domain::ImportKeypointNamePolicy::GenerateIndexed
            }
        },
    }
}

fn manifest_transforms(output: &OutputPolicy) -> labello_domain::ImportTransformPolicies {
    let box_to_skeleton = match &output.box_to_skeleton {
        BoxToSkeletonPolicy::None => labello_domain::ImportBoxToSkeletonPolicy::None,
        BoxToSkeletonPolicy::Template { keypoints } => {
            labello_domain::ImportBoxToSkeletonPolicy::Template {
                keypoints: keypoints
                    .iter()
                    .map(|point| labello_domain::ImportTemplateKeypoint {
                        name: point.name.clone(),
                        x: point.x,
                        y: point.y,
                        state: point.state.clone(),
                    })
                    .collect(),
            }
        }
        BoxToSkeletonPolicy::ManualBoxGuide {
            keypoint_names,
            edges,
        } => labello_domain::ImportBoxToSkeletonPolicy::ManualBoxGuide {
            keypoint_names: keypoint_names.clone(),
            edges: edges.clone(),
        },
    };
    labello_domain::ImportTransformPolicies {
        bounding_boxes: output.bounding_boxes,
        skeletons: output.skeletons,
        box_to_skeleton,
    }
}

fn skeleton_spec(category: &IrCategory, output: &OutputPolicy) -> StorageResult<SkeletonSpec> {
    let names = if !category.keypoint_names.is_empty() {
        category.keypoint_names.clone()
    } else {
        match &output.box_to_skeleton {
            BoxToSkeletonPolicy::Template { keypoints } => {
                keypoints.iter().map(|point| point.name.clone()).collect()
            }
            BoxToSkeletonPolicy::ManualBoxGuide { keypoint_names, .. } => keypoint_names.clone(),
            BoxToSkeletonPolicy::None => {
                return Err(import_error(
                    "skeleton_schema_missing",
                    "skeleton output requires a source, template, or manual schema",
                ));
            }
        }
    };
    if names.iter().collect::<BTreeSet<_>>().len() != names.len()
        || names.iter().any(|name| name.is_empty())
    {
        return Err(import_error(
            "skeleton_schema_invalid",
            "skeleton keypoint names must be unique and nonempty",
        ));
    }
    Ok(SkeletonSpec {
        keypoints: names
            .into_iter()
            .map(|name| KeypointSpec {
                name,
                required: false,
            })
            .collect(),
        edges: if category.edges.is_empty() {
            match &output.box_to_skeleton {
                BoxToSkeletonPolicy::ManualBoxGuide { edges, .. } => edges
                    .iter()
                    .map(|(from, to)| SkeletonEdge {
                        from: from.clone(),
                        to: to.clone(),
                    })
                    .collect(),
                _ => Vec::new(),
            }
        } else {
            category
                .edges
                .iter()
                .map(|(from, to)| SkeletonEdge {
                    from: from.clone(),
                    to: to.clone(),
                })
                .collect()
        },
        allow_hidden: category.allow_hidden
            || matches!(output.box_to_skeleton, BoxToSkeletonPolicy::Template { ref keypoints } if keypoints.iter().any(|point| point.state == labello_domain::KeypointState::Hidden)),
        allow_absent: true,
    })
}
