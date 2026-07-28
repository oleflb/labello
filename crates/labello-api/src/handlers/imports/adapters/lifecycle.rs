fn storage_profile(profile: client::ImportProfile) -> ApiResult<storage::ImportProfile> {
    match profile {
        client::ImportProfile::UltralyticsYoloDetectV1 => {
            Ok(storage::ImportProfile::UltralyticsYoloDetectV1)
        }
        client::ImportProfile::UltralyticsYoloPoseV1 => {
            Ok(storage::ImportProfile::UltralyticsYoloPoseV1)
        }
        client::ImportProfile::CocoInstancesGtV1 => Ok(storage::ImportProfile::CocoInstancesGtV1),
        client::ImportProfile::CocoKeypointsGtV1 => Ok(storage::ImportProfile::CocoKeypointsGtV1),
        client::ImportProfile::Unknown => Err(ApiError::Unprocessable(
            "unsupported import profile".to_string(),
        )),
    }
}

fn client_profile(profile: storage::ImportProfile) -> client::ImportProfile {
    match profile {
        storage::ImportProfile::UltralyticsYoloDetectV1 => {
            client::ImportProfile::UltralyticsYoloDetectV1
        }
        storage::ImportProfile::UltralyticsYoloPoseV1 => {
            client::ImportProfile::UltralyticsYoloPoseV1
        }
        storage::ImportProfile::CocoInstancesGtV1 => client::ImportProfile::CocoInstancesGtV1,
        storage::ImportProfile::CocoKeypointsGtV1 => client::ImportProfile::CocoKeypointsGtV1,
    }
}

fn client_phase(phase: storage::ImportJobPhase) -> client::ImportLifecycle {
    match phase {
        storage::ImportJobPhase::Registering => client::ImportLifecycle::Registering,
        storage::ImportJobPhase::Uploading => client::ImportLifecycle::Uploading,
        storage::ImportJobPhase::Sealed => client::ImportLifecycle::Sealed,
        storage::ImportJobPhase::Preflighting => client::ImportLifecycle::Preflighting,
        storage::ImportJobPhase::AwaitingDecision => client::ImportLifecycle::AwaitingDecision,
        storage::ImportJobPhase::Building => client::ImportLifecycle::Building,
        storage::ImportJobPhase::Verifying => client::ImportLifecycle::Verifying,
        storage::ImportJobPhase::Committing => client::ImportLifecycle::Committing,
        storage::ImportJobPhase::Succeeded => client::ImportLifecycle::Succeeded,
        storage::ImportJobPhase::Failed => client::ImportLifecycle::Failed,
        storage::ImportJobPhase::Cancelled => client::ImportLifecycle::Cancelled,
        storage::ImportJobPhase::Expired => client::ImportLifecycle::Expired,
    }
}

fn progress_phase(phase: storage::ImportJobPhase) -> client::ImportProgressPhase {
    match phase {
        storage::ImportJobPhase::Registering => client::ImportProgressPhase::Registration,
        storage::ImportJobPhase::Uploading => client::ImportProgressPhase::Upload,
        storage::ImportJobPhase::Sealed => client::ImportProgressPhase::Sealing,
        storage::ImportJobPhase::Preflighting | storage::ImportJobPhase::AwaitingDecision => {
            client::ImportProgressPhase::Preflight
        }
        storage::ImportJobPhase::Building => client::ImportProgressPhase::Build,
        storage::ImportJobPhase::Verifying => client::ImportProgressPhase::Verification,
        storage::ImportJobPhase::Committing | storage::ImportJobPhase::Succeeded => {
            client::ImportProgressPhase::Commit
        }
        storage::ImportJobPhase::Failed
        | storage::ImportJobPhase::Cancelled
        | storage::ImportJobPhase::Expired => client::ImportProgressPhase::Cleanup,
    }
}

fn convert_job(job: storage::ImportJob, control: Option<&JobControl>) -> client::ImportJob {
    let plan = control.and_then(|control| control.plan.as_ref());
    let phase = job.phase.clone();
    let report = plan.map(convert_report);
    client::ImportJob {
        import_id: job.import_id,
        owner_user_id: job.owner_user_id,
        destination_dataset_id: job.destination_dataset_id,
        destination_name: job.destination_name,
        profile: client_profile(job.profile),
        transport: match job.transport {
            storage::ImportTransport::Browser => client::ImportTransport::BrowserFolder,
            storage::ImportTransport::ServerDirectory => client::ImportTransport::ServerDirectory,
        },
        lifecycle: client_phase(phase.clone()),
        progress: client::ImportProgress {
            phase: progress_phase(phase.clone()),
            registered_files: job.accepted_files as u64,
            uploaded_files: job.accepted_files as u64,
            total_files: job.accepted_files as u64,
            accepted_bytes: job.accepted_bytes,
            total_bytes: job.accepted_bytes,
            processed_images: plan.map_or(0, |plan| plan.totals.images as u64),
            total_images: plan.map_or(0, |plan| plan.totals.images as u64),
            processed_objects: plan.map_or(0, |plan| plan.totals.source_objects as u64),
            total_objects: plan.map_or(0, |plan| plan.totals.source_objects as u64),
        },
        failure: job.failure_code.map(|code| client::ImportFailure {
            safe_summary: format!("import failed ({code})"),
            code,
            phase: progress_phase(phase.clone()),
            retryable: false,
        }),
        source_fingerprint: job.source_fingerprint,
        plan_hash: job.plan_hash,
        preflight_report: report,
        can_cancel: !matches!(
            phase,
            storage::ImportJobPhase::Committing
                | storage::ImportJobPhase::Succeeded
                | storage::ImportJobPhase::Cancelled
                | storage::ImportJobPhase::Expired
        ),
        created_at: job.created_at,
        updated_at: job.updated_at,
        expires_at: None,
        recovery: control.map(|control| client::ImportRecoveryState {
            attestations: control.create_request.attestations.clone(),
            server_root_id: match &control.create_request.source {
                client::ImportSourceSelection::ServerDirectory { import_root_id, .. } => {
                    Some(import_root_id.clone())
                }
                client::ImportSourceSelection::BrowserFolder => None,
            },
            source: control
                .seal_request
                .as_ref()
                .map(|seal| safe_source_configuration(control, &seal.source)),
            registered_files: control
                .files
                .iter()
                .map(|(file_id, file)| client::RegisteredImportFile {
                    client_file_id: file.client_file_id.clone().unwrap_or_default(),
                    file_id: file_id.clone(),
                    byte_size: file.byte_size,
                    accepted_bytes: file.accepted_bytes,
                    complete: file.complete,
                })
                .collect(),
            accepted_plan: plan
                .map(|plan| convert_plan(plan, control.accepted_plan_request.as_ref())),
        }),
    }
}

fn safe_source_configuration(
    control: &JobControl,
    source: &client::ImportSourceConfiguration,
) -> client::ImportSourceConfiguration {
    let opaque_id = |reference: &str| {
        control
            .files
            .iter()
            .find(|(file_id, file)| {
                file_id.as_str() == reference
                    || file.client_file_id.as_deref() == Some(reference)
                    || file.relative_path == reference
            })
            .map(|(file_id, _)| file_id.clone())
            .unwrap_or_else(|| reference.to_string())
    };
    client::ImportSourceConfiguration {
        source_namespace: source.source_namespace.clone(),
        descriptors: source
            .descriptors
            .iter()
            .map(|descriptor| client::ImportDescriptorSelection {
                descriptor_file_id: opaque_id(&descriptor.descriptor_file_id),
                kind: descriptor.kind,
                release: descriptor.release.clone(),
                split: descriptor.split.clone(),
                image_root_file_id: descriptor.image_root_file_id.as_deref().map(&opaque_id),
                pairing_group: descriptor.pairing_group.clone(),
            })
            .collect(),
        selected_splits: source.selected_splits.clone(),
        selected_category_keys: source.selected_category_keys.clone(),
    }
}

fn convert_capabilities(state: &ApiState, actor: &UserId) -> client::ImportCapabilities {
    let Some(service) = state.import_service() else {
        return client::ImportCapabilities {
            available: false,
            unavailable_reason: Some("dataset import is not configured".to_string()),
            ..Default::default()
        };
    };
    let capabilities = service.capabilities();
    let available = capabilities.available
        && capabilities.atomic_publication
        && capabilities.secure_server_open
        && capabilities.browser_upload
        && !capabilities.profiles.is_empty();
    let unavailable_reason = if available {
        None
    } else {
        capabilities.unavailable_reason.clone()
    };
    let visible_roots = state.visible_import_roots(actor);
    let server_roots = capabilities
        .server_directory_roots
        .iter()
        .filter(|root_id| visible_roots.contains(root_id.as_str()))
        .map(|root_id| client::ServerImportRoot {
            root_id: root_id.clone(),
            display_name: root_id.clone(),
        })
        .collect::<Vec<_>>();
    client::ImportCapabilities {
        available,
        unavailable_reason,
        profiles: storage::ImportProfile::ALL
            .into_iter()
            .map(|profile| client::ImportProfileCapability {
                profile: client_profile(profile),
                enabled: available && capabilities.profiles.contains(&profile),
                display_name: match profile {
                    storage::ImportProfile::UltralyticsYoloDetectV1 => "Ultralytics YOLO detection",
                    storage::ImportProfile::UltralyticsYoloPoseV1 => "Ultralytics YOLO pose",
                    storage::ImportProfile::CocoInstancesGtV1 => "COCO instances ground truth",
                    storage::ImportProfile::CocoKeypointsGtV1 => "COCO keypoints ground truth",
                }
                .to_string(),
                profile_version: 1,
            })
            .collect(),
        transports: vec![
            client::ImportTransportCapability {
                transport: client::ImportTransport::BrowserFolder,
                enabled: available && capabilities.browser_upload,
                resumable: true,
            },
            client::ImportTransportCapability {
                transport: client::ImportTransport::ServerDirectory,
                enabled: available && !server_roots.is_empty(),
                resumable: false,
            },
        ],
        server_roots,
        limits: client::ImportLimits {
            max_browser_files: capabilities.limits.browser_source_files as u64,
            max_browser_bytes: capabilities.limits.browser_source_bytes,
            max_server_files: capabilities.limits.server_source_files as u64,
            max_source_bytes: capabilities.limits.total_source_bytes,
            max_selected_images: capabilities.limits.selected_images as u64,
            max_single_file_bytes: capabilities.limits.single_source_file_bytes,
            upload_chunk_bytes: capabilities.limits.upload_chunk_bytes as u64,
            max_selected_categories: capabilities.limits.selected_categories as u32,
            max_generated_tasks: capabilities.limits.selected_tasks as u32,
            max_annotations: capabilities.limits.annotations_total as u64,
            max_annotations_per_image: capabilities.limits.annotations_per_image as u32,
            max_keypoints_per_skeleton: capabilities.limits.keypoints_per_skeleton as u32,
            max_diagnostic_page_size: 100,
        },
        schema_version: capabilities.schema_version,
        parser_version: capabilities.parser_version.clone(),
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        manual_box_guide_migration: available,
    }
}
