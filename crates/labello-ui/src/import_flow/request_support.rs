fn mapped_task(
    task_id: TaskId,
    name: &str,
    annotation_type: AnnotationType,
    class_id: ClassId,
    skeleton: Option<SkeletonSpec>,
    manual_box_guide_migration: Option<labello_domain::ManualBoxGuideMigration>,
    workflow_intent: ImportWorkflowIntent,
) -> TaskDefinition {
    let review = review_config_for_task(
        workflow_intent,
        manual_box_guide_migration.is_some(),
    );
    TaskDefinition {
        task_id,
        name: name.to_string(),
        annotation_type,
        class_ids: vec![class_id],
        instructions: TutorialContent {
            title: name.to_string(),
            example_text: "Follow the imported dataset definition and project guidance."
                .to_string(),
            example_images: Vec::new(),
        },
        skeleton,
        review,
        prelabel_config_ids: Vec::new(),
        manual_box_guide_migration,
        enabled: true,
    }
}

fn review_config_for_task(
    intent: ImportWorkflowIntent,
    manual_box_guide_migration: bool,
) -> ReviewConfig {
    if manual_box_guide_migration {
        return ReviewConfig {
            required_reviews: 1,
            workflow: labello_domain::ReviewWorkflow::Approval,
            allow_reviewer_corrections: false,
            agreement_threshold: None,
        };
    }
    review_config(intent)
}

fn review_config(intent: ImportWorkflowIntent) -> ReviewConfig {
    match intent {
        ImportWorkflowIntent::AuthoritativeGroundTruth => ReviewConfig {
            required_reviews: 0,
            workflow: labello_domain::ReviewWorkflow::None,
            allow_reviewer_corrections: false,
            agreement_threshold: None,
        },
        ImportWorkflowIntent::RequireApproval | ImportWorkflowIntent::SeedFutureAnnotation => {
            ReviewConfig {
                required_reviews: 1,
                workflow: labello_domain::ReviewWorkflow::Approval,
                allow_reviewer_corrections: false,
                agreement_threshold: None,
            }
        }
    }
}


fn parent_source_directory(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(parent, _)| parent.to_string())
        .unwrap_or_default()
}

fn descriptor_path_matches(profile: ImportProfile, path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    match profile {
        ImportProfile::UltralyticsYoloDetectV1 | ImportProfile::UltralyticsYoloPoseV1 => {
            lower.ends_with(".yaml") || lower.ends_with(".yml")
        }
        ImportProfile::CocoInstancesGtV1 | ImportProfile::CocoKeypointsGtV1 => {
            lower.ends_with(".json")
        }
        ImportProfile::Unknown => false,
    }
}

fn is_coco_profile(profile: ImportProfile) -> bool {
    matches!(
        profile,
        ImportProfile::CocoInstancesGtV1 | ImportProfile::CocoKeypointsGtV1
    )
}

fn descriptor_draft(profile: ImportProfile) -> ImportDescriptorDraft {
    ImportDescriptorDraft {
        kind: descriptor_kind(profile),
        ..Default::default()
    }
}

fn descriptor_kind_allowed(profile: ImportProfile, kind: ImportDescriptorKind) -> bool {
    match profile {
        ImportProfile::UltralyticsYoloDetectV1 | ImportProfile::UltralyticsYoloPoseV1 => {
            kind == ImportDescriptorKind::YoloDataset
        }
        ImportProfile::CocoInstancesGtV1 => kind == ImportDescriptorKind::CocoInstances,
        ImportProfile::CocoKeypointsGtV1 => matches!(
            kind,
            ImportDescriptorKind::CocoInstances | ImportDescriptorKind::CocoKeypoints
        ),
        ImportProfile::Unknown => false,
    }
}

fn descriptor_kind_label(kind: ImportDescriptorKind) -> &'static str {
    match kind {
        ImportDescriptorKind::YoloDataset => "YOLO dataset",
        ImportDescriptorKind::CocoInstances => "COCO instances",
        ImportDescriptorKind::CocoKeypoints => "COCO keypoints",
    }
}

fn is_image_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".jpg", ".jpeg", ".png", ".webp", ".bmp", ".gif"]
        .iter()
        .any(|extension| lower.ends_with(extension))
}

fn valid_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_identity_component(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 255
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

// Browser selection invokes this in WASM; native tests exercise the same pure
// limit check without making browser-only code part of the native runtime.
#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
fn validate_browser_selection_limits(
    file_count: usize,
    total_bytes: u64,
    file_sizes: impl IntoIterator<Item = u64>,
    limits: &labello_client::ImportLimits,
) -> Result<(), String> {
    if file_count == 0 {
        return Err("selected folder contains no files".to_string());
    }
    if file_count as u64 > limits.max_browser_files {
        return Err(format!(
            "selected folder has {file_count} files; the server limit is {}",
            limits.max_browser_files
        ));
    }
    if total_bytes > limits.max_browser_bytes {
        return Err(format!(
            "selected folder has {total_bytes} bytes; the server limit is {}",
            limits.max_browser_bytes
        ));
    }
    if file_sizes
        .into_iter()
        .any(|size| size > limits.max_single_file_bytes)
    {
        return Err(format!(
            "a selected file exceeds the {} byte per-file limit",
            limits.max_single_file_bytes
        ));
    }
    Ok(())
}

fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}
