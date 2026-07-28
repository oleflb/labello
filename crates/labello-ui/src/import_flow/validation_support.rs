fn descriptor_kind(profile: ImportProfile) -> ImportDescriptorKind {
    match profile {
        ImportProfile::UltralyticsYoloDetectV1 | ImportProfile::UltralyticsYoloPoseV1 => {
            ImportDescriptorKind::YoloDataset
        }
        ImportProfile::CocoKeypointsGtV1 => ImportDescriptorKind::CocoKeypoints,
        ImportProfile::CocoInstancesGtV1 | ImportProfile::Unknown => {
            ImportDescriptorKind::CocoInstances
        }
    }
}

#[cfg(test)]
fn policies_for_mapping(
    source: ImportGeometryKind,
    target: ImportGeometryKind,
    available: &[ImportGeometryKind],
    manual_box_guide_available: bool,
) -> Vec<ImportGeometryPolicy> {
    let mut policies = Vec::new();
    if available.contains(&source) {
        match (source, target) {
            (ImportGeometryKind::BoundingBox, ImportGeometryKind::BoundingBox)
            | (ImportGeometryKind::Skeleton, ImportGeometryKind::Skeleton) => {
                policies.push(ImportGeometryPolicy::Direct);
            }
            (ImportGeometryKind::Skeleton, ImportGeometryKind::BoundingBox) => {
                policies.push(ImportGeometryPolicy::KeypointEnvelopeV1);
            }
            (ImportGeometryKind::BoundingBox, ImportGeometryKind::Skeleton) => {
                policies.push(ImportGeometryPolicy::BoxRelativeTemplateV1);
                if manual_box_guide_available {
                    policies.push(ImportGeometryPolicy::ManualBoxGuideV1);
                }
            }
        }
    }
    policies.push(ImportGeometryPolicy::Omit);
    policies
}

fn geometry_choices_for_target(
    target: ImportGeometryKind,
    available: &[ImportGeometryKind],
    manual_box_guide_available: bool,
    has_source_skeleton: bool,
) -> Vec<ImportGeometryChoice> {
    let mut choices = Vec::new();
    match target {
        ImportGeometryKind::BoundingBox => {
            if available.contains(&ImportGeometryKind::BoundingBox) {
                choices.push(ImportGeometryChoice {
                    source: ImportGeometryKind::BoundingBox,
                    policy: ImportGeometryPolicy::Direct,
                    label: "Direct bounding boxes",
                });
            }
            if available.contains(&ImportGeometryKind::Skeleton) {
                choices.push(ImportGeometryChoice {
                    source: ImportGeometryKind::Skeleton,
                    policy: ImportGeometryPolicy::KeypointEnvelopeV1,
                    label: "Derive boxes from skeletons",
                });
            }
        }
        ImportGeometryKind::Skeleton => {
            if available.contains(&ImportGeometryKind::Skeleton) {
                choices.push(ImportGeometryChoice {
                    source: ImportGeometryKind::Skeleton,
                    policy: ImportGeometryPolicy::Direct,
                    label: "Direct skeletons",
                });
            }
            if available.contains(&ImportGeometryKind::BoundingBox) {
                choices.push(ImportGeometryChoice {
                    source: ImportGeometryKind::BoundingBox,
                    policy: ImportGeometryPolicy::BoxRelativeTemplateV1,
                    label: "Template skeletons from boxes",
                });
                if manual_box_guide_available && !has_source_skeleton {
                    choices.push(ImportGeometryChoice {
                        source: ImportGeometryKind::BoundingBox,
                        policy: ImportGeometryPolicy::ManualBoxGuideV1,
                        label: "Manual box guide",
                    });
                }
            }
        }
    }
    choices.push(ImportGeometryChoice {
        source: available.first().copied().unwrap_or(target),
        policy: ImportGeometryPolicy::Omit,
        label: "Do not import this output",
    });
    choices
}

fn mapping_parameter_errors(
    mapping: &ImportGeometryMappingRequest,
    skeleton: Option<&SkeletonSpec>,
) -> Vec<String> {
    let mut errors = Vec::new();
    match mapping.policy {
        ImportGeometryPolicy::Direct
        | ImportGeometryPolicy::ManualBoxGuideV1
        | ImportGeometryPolicy::Omit => {
            if !mapping.parameters.is_empty() {
                errors.push("This output policy does not accept parameters.".to_string());
            }
        }
        ImportGeometryPolicy::KeypointEnvelopeV1 => {
            let mut padding = None;
            let mut minimum_pixels = None;
            let mut include_hidden = None;
            for parameter in &mapping.parameters {
                match parameter {
                    ImportMappingParameter::Scalar { name, value }
                        if matches!(
                            name.as_str(),
                            "padding" | "padding_ratio" | "paddingRatio"
                        ) && padding.replace(*value).is_none() => {}
                    ImportMappingParameter::Scalar { name, value }
                        if matches!(
                            name.as_str(),
                            "minimum_pixels" | "minimumPixels" | "min_pixels" | "minPixels"
                        ) && minimum_pixels.replace(*value).is_none() => {}
                    ImportMappingParameter::Boolean { name, value }
                        if matches!(
                            name.as_str(),
                            "include_hidden" | "includeHidden" | "hidden"
                        ) && include_hidden.replace(*value).is_none() => {}
                    _ => errors.push(
                        "Envelope parameters must contain one padding, minimum-pixels, and hidden value."
                            .to_string(),
                    ),
                }
            }
            if padding.is_none_or(|value| !value.is_finite() || !(0.0..=1.0).contains(&value)) {
                errors.push("Envelope padding must be between 0 and 1.".to_string());
            }
            if minimum_pixels.is_none_or(|value| {
                !value.is_finite()
                    || value.fract() != 0.0
                    || value < 1.0
                    || value > f64::from(u32::MAX)
            }) {
                errors.push(
                    "Envelope minimum pixels must be a whole number from 1 through 4294967295."
                        .to_string(),
                );
            }
            if include_hidden.is_none() {
                errors.push("Choose whether hidden keypoints contribute to envelopes.".to_string());
            }
        }
        ImportGeometryPolicy::BoxRelativeTemplateV1 => {
            let Some(skeleton) = skeleton else {
                errors.push("Define a target skeleton before editing template points.".to_string());
                return errors;
            };
            if mapping.parameters.len() != skeleton.keypoints.len() || mapping.parameters.is_empty()
            {
                errors.push(
                    "Template points must define every target keypoint exactly once.".to_string(),
                );
                return errors;
            }
            let mut any_present = false;
            for (parameter, spec) in mapping.parameters.iter().zip(&skeleton.keypoints) {
                let ImportMappingParameter::Point { name, x, y, state } = parameter else {
                    errors.push("Template parameters must be named points.".to_string());
                    continue;
                };
                if name != &spec.name {
                    errors.push(
                        "Template point names and order must match the target skeleton."
                            .to_string(),
                    );
                }
                if !x.is_finite()
                    || !y.is_finite()
                    || !(0.0..=1.0).contains(x)
                    || !(0.0..=1.0).contains(y)
                {
                    errors.push("Template point coordinates must be between 0 and 1.".to_string());
                }
                match state {
                    labello_domain::KeypointState::Visible => any_present = true,
                    labello_domain::KeypointState::Hidden => {
                        any_present = true;
                        if !skeleton.allow_hidden {
                            errors.push(
                                "This target skeleton does not allow hidden keypoints.".to_string(),
                            );
                        }
                    }
                    labello_domain::KeypointState::Absent => {
                        if !skeleton.allow_absent || spec.required {
                            errors.push(
                                "A required target keypoint cannot be marked absent.".to_string(),
                            );
                        }
                    }
                }
            }
            if !any_present {
                errors.push("At least one template keypoint must be present.".to_string());
            }
        }
    }
    errors.sort();
    errors.dedup();
    errors
}

fn push_mapping_issue(
    validation: &mut ImportMappingValidation,
    severity: ImportMappingIssueSeverity,
    category_index: Option<usize>,
    field: ImportMappingField,
    message: impl Into<String>,
) {
    let issue = ImportMappingIssue {
        severity,
        category_index,
        field,
        message: message.into(),
    };
    if !validation.issues.contains(&issue) {
        validation.issues.push(issue);
    }
}

fn mapping_parameter_editor(
    ui: &mut egui::Ui,
    mapping: &mut ImportGeometryMappingRequest,
    skeleton: Option<&SkeletonSpec>,
    target_keypoint_names: &str,
    show_template_points: bool,
) {
    match mapping.policy {
        ImportGeometryPolicy::KeypointEnvelopeV1 => {
            if !matches!(
                mapping.parameters.as_slice(),
                [
                    ImportMappingParameter::Scalar { .. },
                    ImportMappingParameter::Scalar { .. },
                    ImportMappingParameter::Boolean { .. }
                ]
            ) {
                mapping.parameters = vec![
                    ImportMappingParameter::Scalar {
                        name: "paddingRatio".to_string(),
                        value: 0.05,
                    },
                    ImportMappingParameter::Scalar {
                        name: "minimumPixels".to_string(),
                        value: 1.0,
                    },
                    ImportMappingParameter::Boolean {
                        name: "includeHidden".to_string(),
                        value: true,
                    },
                ];
            }
            for parameter in &mut mapping.parameters {
                match parameter {
                    ImportMappingParameter::Scalar { name, value } => {
                        ui.horizontal(|ui| {
                            ui.label(name.as_str());
                            let editor = egui::DragValue::new(value);
                            if matches!(name.as_str(), "padding" | "padding_ratio" | "paddingRatio")
                            {
                                ui.add(editor.range(0.0..=1.0).speed(0.01));
                            } else {
                                ui.add(
                                    editor
                                        .range(1.0..=f64::from(u32::MAX))
                                        .speed(1.0)
                                        .fixed_decimals(0),
                                );
                            }
                        });
                    }
                    ImportMappingParameter::Boolean { name, value } => {
                        ui.checkbox(value, name.as_str());
                    }
                    ImportMappingParameter::Point { .. } => {}
                }
            }
        }
        ImportGeometryPolicy::BoxRelativeTemplateV1 => {
            let names = skeleton.map_or_else(
                || split_csv(target_keypoint_names),
                |skeleton| {
                    skeleton
                        .keypoints
                        .iter()
                        .map(|point| point.name.clone())
                        .collect()
                },
            );
            let current_names = mapping
                .parameters
                .iter()
                .filter_map(|parameter| {
                    let ImportMappingParameter::Point { name, .. } = parameter else {
                        return None;
                    };
                    Some(name.as_str())
                })
                .collect::<Vec<_>>();
            if current_names != names.iter().map(String::as_str).collect::<Vec<_>>() {
                let previous = mapping
                    .parameters
                    .iter()
                    .filter_map(|parameter| {
                        let ImportMappingParameter::Point { name, x, y, state } = parameter else {
                            return None;
                        };
                        Some((name.clone(), (*x, *y, state.clone())))
                    })
                    .collect::<std::collections::BTreeMap<_, _>>();
                mapping.parameters = names
                    .into_iter()
                    .map(|name| {
                        let (x, y, state) = previous.get(&name).cloned().unwrap_or((
                            0.5,
                            0.5,
                            labello_domain::KeypointState::Visible,
                        ));
                        ImportMappingParameter::Point { name, x, y, state }
                    })
                    .collect();
            }
            if !show_template_points {
                ui.small(
                    "Template-point controls will update after you finish editing keypoint names.",
                );
                return;
            }
            ui.label(RichText::new("Template point positions").strong());
            for (point_index, parameter) in mapping.parameters.iter_mut().enumerate() {
                if let ImportMappingParameter::Point { name, x, y, state } = parameter {
                    ui.horizontal(|ui| {
                        ui.label(name.as_str());
                        ui.label("x");
                        ui.add(egui::DragValue::new(x).range(0.0..=1.0).speed(0.01));
                        ui.label("y");
                        ui.add(egui::DragValue::new(y).range(0.0..=1.0).speed(0.01));
                        egui::ComboBox::from_id_salt(("state", point_index))
                            .selected_text(format!("{state:?}"))
                            .show_ui(ui, |ui| {
                                let spec = skeleton
                                    .and_then(|skeleton| skeleton.keypoints.get(point_index));
                                let mut candidates = vec![labello_domain::KeypointState::Visible];
                                if skeleton.is_none_or(|skeleton| skeleton.allow_hidden) {
                                    candidates.push(labello_domain::KeypointState::Hidden);
                                }
                                if skeleton.is_none_or(|skeleton| skeleton.allow_absent)
                                    && spec.is_none_or(|spec| !spec.required)
                                {
                                    candidates.push(labello_domain::KeypointState::Absent);
                                }
                                for candidate in candidates {
                                    ui.selectable_value(
                                        state,
                                        candidate.clone(),
                                        format!("{candidate:?}"),
                                    );
                                }
                            });
                    });
                }
            }
        }
        ImportGeometryPolicy::Direct
        | ImportGeometryPolicy::ManualBoxGuideV1
        | ImportGeometryPolicy::Omit => mapping.parameters.clear(),
    }
}
