use std::collections::BTreeSet;

use eframe::egui;
use labello_client::{
    DatasetSummary, DatasetUser, ImportCapabilities, ImportFailure, ImportJob, ImportLifecycle,
    ImportPlan, ImportPreflightReport, ImportProfile, ImportProfileCapability, ImportProgress,
    ImportProgressPhase, ImportSourceCounts, ImportTransport, ImportTransportCapability,
};
use labello_domain::{
    AnnotationGeometry, AnnotationId, AnnotationOrigin, AnnotationType, AnnotationVersion,
    Assignment, AssignmentId, AssignmentKind, AssignmentStatus, BoundingBox, ClassId, ClassStats,
    CorrectionId, DatasetMetadata, DatasetRole, DatasetRoleAssignment, DatasetStats,
    HumanRevisionKind, ImageState, KeypointSpec, ManualBoxGuideMigration, MigrationCardinality,
    MigrationDisposition, MigrationDispositionStatus, MigrationHashContext, MigrationPass,
    MigrationPassId, MigrationSequence, MigrationTarget, MigrationTargetSetInitialization,
    ObjectGroupId, RevisionSource, SkeletonSpec, TaskId, TaskStats, ThroughputPoint, UserAccount,
    UserId, migration_target_set_hash,
};

use crate::app::{AppView, CorrectionDraft, LabelloApp, PendingTransition, SetupSection};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InspectorPreset {
    Annotation,
    Setup,
    Review,
    ReviewCorrection,
    Adjudication,
    Admin,
    Statistics,
    DialogSettings,
    DialogTransition,
    DialogAdminDiscard,
    SetupFailure,
    AdminFailure,
    StatisticsFailure,
    AssignmentFailure,
    ImageFailure,
    ImportSource,
    ImportPreflight,
    ImportReady,
    ImportRunning,
    ImportFailure,
    ImportSuccess,
    ImportMultipleDescriptors,
    ImportYoloSplits,
    ImportServerFolderPicker,
    ImportServerDescriptorPicker,
    ImportPartialCategories,
    ImportRecoveryBlocked,
    MigrationObject,
    MigrationExclusion,
    MigrationPass,
    MigrationFullImage,
    MigrationReview,
    MigrationAnnotatedEdit,
    MigrationGuideDeleted,
}

impl InspectorPreset {
    pub const ALL: [Self; 34] = [
        Self::Annotation,
        Self::Setup,
        Self::Review,
        Self::ReviewCorrection,
        Self::Adjudication,
        Self::Admin,
        Self::Statistics,
        Self::DialogSettings,
        Self::DialogTransition,
        Self::DialogAdminDiscard,
        Self::SetupFailure,
        Self::AdminFailure,
        Self::StatisticsFailure,
        Self::AssignmentFailure,
        Self::ImageFailure,
        Self::ImportSource,
        Self::ImportPreflight,
        Self::ImportReady,
        Self::ImportRunning,
        Self::ImportFailure,
        Self::ImportSuccess,
        Self::ImportMultipleDescriptors,
        Self::ImportYoloSplits,
        Self::ImportServerFolderPicker,
        Self::ImportServerDescriptorPicker,
        Self::ImportPartialCategories,
        Self::ImportRecoveryBlocked,
        Self::MigrationObject,
        Self::MigrationExclusion,
        Self::MigrationPass,
        Self::MigrationFullImage,
        Self::MigrationReview,
        Self::MigrationAnnotatedEdit,
        Self::MigrationGuideDeleted,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Annotation => "annotation",
            Self::Setup => "setup",
            Self::Review => "review",
            Self::ReviewCorrection => "review-correction",
            Self::Adjudication => "adjudication",
            Self::Admin => "admin",
            Self::Statistics => "statistics",
            Self::DialogSettings => "dialog-settings",
            Self::DialogTransition => "dialog-transition",
            Self::DialogAdminDiscard => "dialog-admin-discard",
            Self::SetupFailure => "setup-failure",
            Self::AdminFailure => "admin-failure",
            Self::StatisticsFailure => "statistics-failure",
            Self::AssignmentFailure => "assignment-failure",
            Self::ImageFailure => "image-failure",
            Self::ImportSource => "import-source",
            Self::ImportPreflight => "import-preflight",
            Self::ImportReady => "import-ready",
            Self::ImportRunning => "import-running",
            Self::ImportFailure => "import-failure",
            Self::ImportSuccess => "import-success",
            Self::ImportMultipleDescriptors => "import-multiple-descriptors",
            Self::ImportYoloSplits => "import-yolo-splits",
            Self::ImportServerFolderPicker => "import-server-folder-picker",
            Self::ImportServerDescriptorPicker => "import-server-descriptor-picker",
            Self::ImportPartialCategories => "import-partial-categories",
            Self::ImportRecoveryBlocked => "import-recovery-blocked",
            Self::MigrationObject => "migration-object",
            Self::MigrationExclusion => "migration-exclusion",
            Self::MigrationPass => "migration-pass",
            Self::MigrationFullImage => "migration-full-image",
            Self::MigrationReview => "migration-review",
            Self::MigrationAnnotatedEdit => "migration-annotated-edit",
            Self::MigrationGuideDeleted => "migration-guide-deleted",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|preset| preset.name() == name)
    }
}

pub fn build(preset: InspectorPreset, ctx: &egui::Context) -> LabelloApp {
    match preset {
        InspectorPreset::Annotation => work_preset(AssignmentKind::Annotation, ctx),
        InspectorPreset::Setup => setup_preset(),
        InspectorPreset::Review => work_preset(AssignmentKind::Review, ctx),
        InspectorPreset::ReviewCorrection => {
            let mut app = work_preset(AssignmentKind::Review, ctx);
            app.work.tasks[0].review.allow_reviewer_corrections = true;
            let annotation = app.work.annotations[0].clone();
            app.work.correction_draft = Some(CorrectionDraft {
                correction_id: CorrectionId::from("cor_inspector"),
                annotation_id: annotation.annotation_id,
                expected_version: annotation.version,
                original_geometry: annotation.geometry,
                edited_geometry: AnnotationGeometry::BoundingBox(BoundingBox {
                    x: 0.24,
                    y: 0.18,
                    width: 0.34,
                    height: 0.58,
                }),
                reason: "Tightened the box around the visible person.".to_string(),
                geometry_history: Vec::new(),
                selected_keypoint: None,
            });
            app
        }
        InspectorPreset::Adjudication => work_preset(AssignmentKind::Adjudication, ctx),
        InspectorPreset::Admin => admin_preset(),
        InspectorPreset::Statistics => statistics_preset(),
        InspectorPreset::DialogSettings => {
            let mut app = work_preset(AssignmentKind::Annotation, ctx);
            app.open_shortcut_settings();
            app
        }
        InspectorPreset::DialogTransition => {
            let mut app = work_preset(AssignmentKind::Annotation, ctx);
            app.work.pending_transition = Some(PendingTransition::View(AppView::Review));
            app
        }
        InspectorPreset::DialogAdminDiscard => {
            let mut app = admin_preset();
            app.datasets.admin_config.as_mut().unwrap().name = "Staged dataset name".to_string();
            app.admin.confirm_discard = true;
            app
        }
        InspectorPreset::SetupFailure => {
            let mut app = setup_preset();
            app.datasets.summaries.clear();
            app.datasets.summaries_error = Some("Dataset catalog is unavailable".to_string());
            app
        }
        InspectorPreset::AdminFailure => {
            let mut app = admin_preset();
            app.datasets.admin_config = None;
            app.datasets.admin_baseline = None;
            app.admin.load_error = Some("Admin configuration is unavailable".to_string());
            app
        }
        InspectorPreset::StatisticsFailure => {
            let mut app = setup_preset();
            app.view = AppView::Stats;
            app.datasets.stats_error = Some("Statistics service is unavailable".to_string());
            app
        }
        InspectorPreset::AssignmentFailure => {
            let mut app = work_preset(AssignmentKind::Annotation, ctx);
            app.work.assignment = None;
            app.loading.saving = true;
            clear_image(&mut app);
            app.runtime.error = Some("Could not claim an assignment".to_string());
            app
        }
        InspectorPreset::ImageFailure => {
            let mut app = work_preset(AssignmentKind::Annotation, ctx);
            clear_image(&mut app);
            app.runtime.error = Some("The assignment preview could not be decoded".to_string());
            app
        }
        InspectorPreset::ImportSource => import_preset(crate::import_flow::ImportScreen::Source),
        InspectorPreset::ImportPreflight => {
            import_preset(crate::import_flow::ImportScreen::Preflight)
        }
        InspectorPreset::ImportReady => import_preset(crate::import_flow::ImportScreen::Ready),
        InspectorPreset::ImportRunning => import_preset(crate::import_flow::ImportScreen::Running),
        InspectorPreset::ImportFailure => import_preset(crate::import_flow::ImportScreen::Failure),
        InspectorPreset::ImportSuccess => import_preset(crate::import_flow::ImportScreen::Success),
        InspectorPreset::ImportMultipleDescriptors => import_multiple_descriptors_preset(),
        InspectorPreset::ImportYoloSplits => import_yolo_splits_preset(),
        InspectorPreset::ImportServerFolderPicker => import_server_folder_picker_preset(),
        InspectorPreset::ImportServerDescriptorPicker => import_server_descriptor_picker_preset(),
        InspectorPreset::ImportPartialCategories => import_partial_categories_preset(),
        InspectorPreset::ImportRecoveryBlocked => import_recovery_blocked_preset(),
        InspectorPreset::MigrationObject => migration_preset(ctx, MigrationPreset::Object),
        InspectorPreset::MigrationExclusion => migration_preset(ctx, MigrationPreset::Exclusion),
        InspectorPreset::MigrationPass => migration_preset(ctx, MigrationPreset::Pass),
        InspectorPreset::MigrationFullImage => migration_preset(ctx, MigrationPreset::FullImage),
        InspectorPreset::MigrationReview => migration_preset(ctx, MigrationPreset::Review),
        InspectorPreset::MigrationAnnotatedEdit => migration_preset(ctx, MigrationPreset::Pass),
        InspectorPreset::MigrationGuideDeleted => migration_deleted_guide_preset(ctx),
    }
}

fn import_multiple_descriptors_preset() -> LabelloApp {
    let mut app = import_preset(crate::import_flow::ImportScreen::Configure);
    app.import.profile = ImportProfile::CocoKeypointsGtV1;
    app.import.registered_paths = vec![
        crate::import_flow::RegisteredImportPath {
            client_file_id: "browser-instances".to_string(),
            file_id: "file-instances".to_string(),
            relative_path: "release/annotations/instances_train.json".to_string(),
        },
        crate::import_flow::RegisteredImportPath {
            client_file_id: "browser-keypoints".to_string(),
            file_id: "file-keypoints".to_string(),
            relative_path: "release/annotations/person_keypoints_train.json".to_string(),
        },
        crate::import_flow::RegisteredImportPath {
            client_file_id: "browser-image".to_string(),
            file_id: "file-image-root".to_string(),
            relative_path: "release/train/frame-001.jpg".to_string(),
        },
    ];
    app.import.descriptors = vec![
        crate::import_flow::ImportDescriptorDraft {
            descriptor_file_id: "file-instances".to_string(),
            kind: labello_client::ImportDescriptorKind::CocoInstances,
            image_root_file_id: "file-image-root".to_string(),
            pairing_group: "people-train".to_string(),
            ..Default::default()
        },
        crate::import_flow::ImportDescriptorDraft {
            descriptor_file_id: "file-keypoints".to_string(),
            kind: labello_client::ImportDescriptorKind::CocoKeypoints,
            image_root_file_id: "file-image-root".to_string(),
            pairing_group: "people-train".to_string(),
            ..Default::default()
        },
    ];
    app
}

fn import_yolo_splits_preset() -> LabelloApp {
    let mut app = import_preset(crate::import_flow::ImportScreen::Configure);
    app.import.profile = ImportProfile::UltralyticsYoloDetectV1;
    app.import.registered_paths = vec![crate::import_flow::RegisteredImportPath {
        client_file_id: "browser-yaml".to_string(),
        file_id: "file-yaml".to_string(),
        relative_path: "release/dataset.yaml".to_string(),
    }];
    app.import.descriptors = vec![crate::import_flow::ImportDescriptorDraft {
        descriptor_file_id: "file-yaml".to_string(),
        kind: labello_client::ImportDescriptorKind::YoloDataset,
        release: "v1".to_string(),
        ..Default::default()
    }];
    app.import.yolo_inspected_descriptor_file_id = Some("file-yaml".to_string());
    app.import.yolo_splits = ["train", "val", "test"]
        .into_iter()
        .map(|name| crate::import_flow::ImportYoloSplitDraft {
            name: name.to_string(),
            usable: true,
            selected: true,
            issue: None,
        })
        .collect();
    app
}

fn import_server_folder_picker_preset() -> LabelloApp {
    let mut app = import_preset(crate::import_flow::ImportScreen::Source);
    app.import.transport = ImportTransport::ServerDirectory;
    app.import.server_root_id = "staging".to_string();
    app.import.source_picker = crate::import_flow::ImportSourcePickerState {
        target: Some(crate::import_flow::ImportSourcePickerTarget::DatasetFolder),
        page: Some(labello_client::ImportBrowsePage {
            relative_path: String::new(),
            entries: ["release-2025", "release-2026"]
                .into_iter()
                .map(|name| labello_client::ImportBrowseEntry {
                    name: name.to_string(),
                    relative_path: name.to_string(),
                    kind: labello_client::ImportBrowseEntryKind::Directory,
                    file_id: None,
                })
                .collect(),
            next_offset: None,
        }),
        ..Default::default()
    };
    app
}

fn import_server_descriptor_picker_preset() -> LabelloApp {
    let mut app = import_yolo_splits_preset();
    app.import.transport = ImportTransport::ServerDirectory;
    app.import.server_root_id = "staging".to_string();
    app.import.server_relative_path = "release-2026".to_string();
    app.import.source_picker = crate::import_flow::ImportSourcePickerState {
        target: Some(crate::import_flow::ImportSourcePickerTarget::Descriptor(0)),
        page: Some(labello_client::ImportBrowsePage {
            relative_path: "release-2026".to_string(),
            entries: vec![labello_client::ImportBrowseEntry {
                name: "dataset.yaml".to_string(),
                relative_path: "release-2026/dataset.yaml".to_string(),
                kind: labello_client::ImportBrowseEntryKind::File,
                file_id: Some("file-yaml".to_string()),
            }],
            next_offset: None,
        }),
        ..Default::default()
    };
    app
}

fn import_recovery_blocked_preset() -> LabelloApp {
    let mut app = import_preset(crate::import_flow::ImportScreen::Preflight);
    app.import.recovery_contract_gap = true;
    app.import.recovery_import_id = "imp_inspector".to_string();
    app.import.categories.clear();
    app.import.plan = None;
    app
}

fn import_partial_categories_preset() -> LabelloApp {
    let mut app = import_preset(crate::import_flow::ImportScreen::Preflight);
    app.import
        .job
        .as_mut()
        .unwrap()
        .preflight_report
        .as_mut()
        .unwrap()
        .source
        .categories = 3;
    app.import.pending_plan_request = Some(labello_client::UpdateImportPlanRequest {
        category_mappings: Vec::new(),
        geometry_mappings: Vec::new(),
        task_mappings: Vec::new(),
        skeleton_mappings: Vec::new(),
        compatibility: Default::default(),
        acknowledgements: Vec::new(),
    });
    app
}

fn migration_deleted_guide_preset(ctx: &egui::Context) -> LabelloApp {
    let mut app = migration_preset(ctx, MigrationPreset::Object);
    let state = app.work.current_state.as_mut().unwrap();
    state
        .annotations
        .get_mut(&AnnotationId::from("guide-left"))
        .unwrap()
        .last_mut()
        .unwrap()
        .deleted = true;
    app.work.annotations = state.active_annotations().cloned().collect();
    app
}

fn import_preset(screen: crate::import_flow::ImportScreen) -> LabelloApp {
    let mut app = setup_preset();
    app.setup.section = SetupSection::Import;
    app.import.capabilities = Some(import_capabilities());
    app.import.open = true;
    app.import.screen = screen;
    app.import.destination_id = "wildlife-2026".to_string();
    app.import.destination_name = "Wildlife 2026".to_string();
    app.import.ground_truth = true;
    app.import.exhaustive = true;
    app.import.coverage_scope = "person".to_string();
    app.import.provenance = "Curated benchmark release".to_string();
    if screen != crate::import_flow::ImportScreen::Source {
        app.import.profile = ImportProfile::CocoInstancesGtV1;
        app.import.descriptors = vec![crate::import_flow::ImportDescriptorDraft::default()];
    }
    app.import.descriptors[0].descriptor_file_id = "file-annotations".to_string();
    app.import.descriptors[0].image_root_file_id = "file-image-root".to_string();
    if screen != crate::import_flow::ImportScreen::Source {
        app.import.job = Some(import_job(screen));
    }
    if screen == crate::import_flow::ImportScreen::Running {
        app.import.busy = true;
        app.import.poll_after =
            Some(web_time::Instant::now() + web_time::Duration::from_secs(3600));
        app.import
            .active_operations
            .insert(u64::MAX, crate::app::ImportActivity::Commit);
    }
    if matches!(
        screen,
        crate::import_flow::ImportScreen::Preflight
            | crate::import_flow::ImportScreen::Ready
            | crate::import_flow::ImportScreen::Running
            | crate::import_flow::ImportScreen::Failure
            | crate::import_flow::ImportScreen::Success
    ) {
        app.import.categories = vec![crate::import_flow::ImportCategoryDraft {
            selected: true,
            source_category_key: "wildlife:v1:17".to_string(),
            source_category_id: "17".to_string(),
            source_name: "Person".to_string(),
            class_id: "person".to_string(),
            class_name: "Person".to_string(),
            class_color: "#5eead4".to_string(),
            bounding_box_task_id: "bounding_box:person".to_string(),
            bounding_box_task_name: "Person bounding boxes".to_string(),
            skeleton_task_id: "skeleton:person".to_string(),
            skeleton_task_name: "Person skeletons".to_string(),
            source_skeleton: None,
            direct_geometry: vec![labello_client::ImportGeometryKind::BoundingBox],
            geometry_mappings: Vec::new(),
            task_mappings: Vec::new(),
            skeleton_mappings: Vec::new(),
            workflow_intent: labello_client::ImportWorkflowIntent::AuthoritativeGroundTruth,
            target_keypoint_names: String::new(),
        }];
    }
    if matches!(
        screen,
        crate::import_flow::ImportScreen::Ready | crate::import_flow::ImportScreen::Success
    ) {
        app.import.normalize_mapping_draft();
        let accepted_request = app.import_plan_request();
        app.import.accepted_plan_request = Some(accepted_request.clone());
        app.import.plan = Some(ImportPlan {
            import_id: labello_domain::ImportId::from("imp_inspector"),
            source_fingerprint: "source-inspector".to_string(),
            plan_hash: "plan-inspector".to_string(),
            commit_ready: true,
            blocking_diagnostic_codes: Vec::new(),
            required_acknowledgement_codes: Vec::new(),
            report: import_report(),
            source_categories: Vec::new(),
            accepted_request: Some(accepted_request),
        });
    }
    app
}

fn import_capabilities() -> ImportCapabilities {
    ImportCapabilities {
        available: true,
        profiles: vec![
            ImportProfileCapability {
                profile: ImportProfile::CocoInstancesGtV1,
                enabled: true,
                display_name: "COCO instances".to_string(),
                profile_version: 1,
            },
            ImportProfileCapability {
                profile: ImportProfile::CocoKeypointsGtV1,
                enabled: true,
                display_name: "COCO keypoints".to_string(),
                profile_version: 1,
            },
        ],
        transports: vec![
            ImportTransportCapability {
                transport: ImportTransport::BrowserFolder,
                enabled: true,
                resumable: true,
            },
            ImportTransportCapability {
                transport: ImportTransport::ServerDirectory,
                enabled: true,
                resumable: true,
            },
        ],
        server_roots: vec![labello_client::ServerImportRoot {
            root_id: "staging".to_string(),
            display_name: "Staging datasets".to_string(),
        }],
        schema_version: labello_domain::SCHEMA_VERSION,
        parser_version: "import-parser-v1".to_string(),
        tool_version: "inspector".to_string(),
        manual_box_guide_migration: true,
        ..Default::default()
    }
}

fn import_job(screen: crate::import_flow::ImportScreen) -> ImportJob {
    let lifecycle = match screen {
        crate::import_flow::ImportScreen::Preflight | crate::import_flow::ImportScreen::Ready => {
            ImportLifecycle::AwaitingDecision
        }
        crate::import_flow::ImportScreen::Running => ImportLifecycle::Building,
        crate::import_flow::ImportScreen::Failure => ImportLifecycle::Failed,
        crate::import_flow::ImportScreen::Success => ImportLifecycle::Succeeded,
        _ => ImportLifecycle::Uploading,
    };
    ImportJob {
        import_id: labello_domain::ImportId::from("imp_inspector"),
        owner_user_id: UserId::from("demo_user"),
        destination_dataset_id: labello_domain::DatasetId::from("wildlife-2026"),
        destination_name: "Wildlife 2026".to_string(),
        profile: ImportProfile::CocoInstancesGtV1,
        transport: ImportTransport::BrowserFolder,
        lifecycle,
        progress: ImportProgress {
            phase: if lifecycle == ImportLifecycle::Building {
                ImportProgressPhase::Build
            } else {
                ImportProgressPhase::Preflight
            },
            registered_files: 126,
            uploaded_files: 126,
            total_files: 126,
            accepted_bytes: 48_000_000,
            total_bytes: 48_000_000,
            processed_images: 64,
            total_images: 120,
            processed_objects: 418,
            total_objects: 900,
        },
        failure: (screen == crate::import_flow::ImportScreen::Failure).then(|| ImportFailure {
            code: "source_descriptor_invalid".to_string(),
            phase: ImportProgressPhase::Preflight,
            safe_summary: "The selected descriptor did not match the chosen profile.".to_string(),
            retryable: true,
        }),
        source_fingerprint: Some("source-inspector".to_string()),
        plan_hash: (screen != crate::import_flow::ImportScreen::Preflight)
            .then(|| "plan-inspector".to_string()),
        preflight_report: Some(import_report()),
        can_cancel: !matches!(screen, crate::import_flow::ImportScreen::Success),
        created_at: timestamp(),
        updated_at: timestamp(),
        expires_at: None,
        recovery: None,
    }
}

fn import_report() -> ImportPreflightReport {
    ImportPreflightReport {
        source_fingerprint: "source-inspector".to_string(),
        source: ImportSourceCounts {
            files: 126,
            bytes: 48_000_000,
            descriptors: 1,
            splits: 2,
            images: 120,
            categories: 1,
            objects: 900,
            keypoints: 0,
        },
        geometry: labello_client::ImportGeometryCounts {
            direct: 894,
            clipped: 6,
            skipped: 0,
            ..Default::default()
        },
        output: labello_client::ImportOutputEstimate {
            classes: 1,
            tasks: 1,
            annotations: 900,
            events: 120,
            ..Default::default()
        },
        diagnostics: vec![labello_client::ImportDiagnosticSummary {
            code: "geometry_clipped".to_string(),
            severity: labello_client::ImportDiagnosticSeverity::WarningRequiresAck,
            source_profile: ImportProfile::CocoInstancesGtV1,
            count: 6,
            safe_summary: "Some boxes extend beyond image bounds and will be clipped.".to_string(),
            impact: labello_client::ImportDiagnosticImpact {
                requires_acknowledgement: true,
                ..Default::default()
            },
            examples: Vec::new(),
        }],
        required_acknowledgements: 1,
        ..Default::default()
    }
}

#[derive(Clone, Copy)]
enum MigrationPreset {
    Object,
    Exclusion,
    Pass,
    FullImage,
    Review,
}

fn migration_preset(ctx: &egui::Context, preset: MigrationPreset) -> LabelloApp {
    let mut app = work_preset(
        if matches!(preset, MigrationPreset::Review) {
            AssignmentKind::Review
        } else {
            AssignmentKind::Annotation
        },
        ctx,
    );
    let guide_task_id = TaskId::from("bounding_box:person");
    let target_task_id = TaskId::from("skeleton:person");
    app.work.tasks.push(labello_domain::TaskDefinition {
        task_id: target_task_id.clone(),
        name: "Person skeleton migration".to_string(),
        annotation_type: AnnotationType::Skeleton,
        class_ids: vec![ClassId::from("person")],
        instructions: labello_domain::TutorialContent {
            title: "Place a skeleton for each imported box".to_string(),
            example_text: "Use the canonical box as a guide.".to_string(),
            example_images: Vec::new(),
        },
        skeleton: Some(SkeletonSpec {
            keypoints: ["head", "left_hand", "right_hand", "left_foot", "right_foot"]
                .into_iter()
                .map(|name| KeypointSpec {
                    name: name.to_string(),
                    required: name == "head",
                })
                .collect(),
            edges: Vec::new(),
            allow_hidden: true,
            allow_absent: true,
        }),
        review: Default::default(),
        prelabel_config_ids: Vec::new(),
        manual_box_guide_migration: Some(ManualBoxGuideMigration {
            guide_task_id: guide_task_id.clone(),
            cardinality: MigrationCardinality::ExactlyOne,
            allow_exclusion: true,
            sequence: MigrationSequence::ImportedSpatialOrderV1,
        }),
        enabled: true,
    });
    app.work.selected_task_id = Some(target_task_id.clone());
    app.work.tool = crate::app::Tool::Keypoints;
    app.work.assignment.as_mut().unwrap().task_id = target_task_id.clone();
    let image_id = app.work.current.as_ref().unwrap().image.image_id.clone();
    let targets = vec![
        MigrationTarget {
            object_group_id: ObjectGroupId::from("group-left"),
            guide_annotation_id: AnnotationId::from("guide-left"),
            reserved_skeleton_annotation_id: AnnotationId::from("skeleton-left"),
            sequence_index: 0,
        },
        MigrationTarget {
            object_group_id: ObjectGroupId::from("group-right"),
            guide_annotation_id: AnnotationId::from("guide-right"),
            reserved_skeleton_annotation_id: AnnotationId::from("skeleton-right"),
            sequence_index: 1,
        },
    ];
    let target_hash = migration_target_set_hash(
        &MigrationHashContext {
            dataset_id: &app.config.dataset_id,
            image_id: &image_id,
            guide_task_id: &guide_task_id,
            target_task_id: &target_task_id,
        },
        &targets,
    )
    .unwrap();
    let mut state = ImageState::new(image_id.clone());
    for (index, target) in targets.iter().enumerate() {
        let guide = guide_annotation(target, index);
        state
            .annotations
            .insert(guide.annotation_id.clone(), vec![guide]);
    }
    state.migration_target_sets.insert(
        target_task_id.clone(),
        MigrationTargetSetInitialization {
            dataset_id: app.config.dataset_id.clone(),
            guide_task_id,
            target_task_id: target_task_id.clone(),
            target_set_hash: target_hash.clone(),
            targets: targets.clone(),
        },
    );
    let resolved = matches!(
        preset,
        MigrationPreset::Pass | MigrationPreset::FullImage | MigrationPreset::Review
    );
    state.migration_dispositions.insert(
        target_task_id.clone(),
        targets
            .iter()
            .enumerate()
            .map(|(index, target)| {
                let status = if resolved && index == 0 {
                    let skeleton = skeleton_annotation(target, &target_task_id);
                    state
                        .annotations
                        .insert(skeleton.annotation_id.clone(), vec![skeleton]);
                    MigrationDispositionStatus::Annotated {
                        skeleton_annotation_id: target.reserved_skeleton_annotation_id.clone(),
                        skeleton_version: 1,
                    }
                } else if resolved {
                    MigrationDispositionStatus::Excluded {
                        exclusion: labello_domain::MigrationExclusion {
                            reason: labello_domain::MigrationExclusionReason::ObjectNotPresent,
                            event_id: labello_domain::EventId::from("evt-exclusion"),
                            actor_user_id: app.config.user_id.clone(),
                            timestamp: timestamp(),
                            note: Some("The source box covers a reflection.".to_string()),
                        },
                    }
                } else {
                    MigrationDispositionStatus::Pending
                };
                (
                    target.object_group_id.clone(),
                    MigrationDisposition {
                        disposition_version: 1,
                        status,
                    },
                )
            })
            .collect(),
    );
    if matches!(preset, MigrationPreset::Pass) {
        let state_hash = state.current_migration_state_hash(&target_task_id).unwrap();
        let pass_id = MigrationPassId::from("pass-inspector");
        state.migration_passes.insert(
            pass_id.clone(),
            MigrationPass {
                pass_id: pass_id.clone(),
                assignment_id: app.work.assignment.as_ref().unwrap().assignment_id.clone(),
                task_id: target_task_id.clone(),
                expected_target_set_hash: target_hash,
                starting_state_hash: state_hash,
                actor_user_id: app.config.user_id.clone(),
                started_at: timestamp(),
                items: Vec::new(),
            },
        );
        app.work.migration.active_pass_id = Some(pass_id);
    }
    app.work.current_state = Some(state.clone());
    app.work.annotations = state.active_annotations().cloned().collect();
    app.work.migration.cursor = state
        .migration_cursor(&target_task_id, app.work.migration.active_pass_id.as_ref())
        .ok();
    if matches!(preset, MigrationPreset::Exclusion) {
        app.work.migration.exclusion_reason =
            labello_domain::MigrationExclusionReason::InsufficientVisibleFeatures;
        app.work.migration.exclusion_note = "Only a small occluded region is visible.".to_string();
    }
    if matches!(preset, MigrationPreset::Review) {
        app.work.migration.review_index = 1;
    }
    app
}

fn guide_annotation(target: &MigrationTarget, index: usize) -> AnnotationVersion {
    AnnotationVersion {
        annotation_id: target.guide_annotation_id.clone(),
        version: 1,
        object_group_id: Some(target.object_group_id.clone()),
        origin: AnnotationOrigin::native(),
        task_id: TaskId::from("bounding_box:person"),
        class_id: ClassId::from("person"),
        annotation_type: AnnotationType::BoundingBox,
        revision_source: RevisionSource::Human {
            action: HumanRevisionKind::Authored,
        },
        geometry: AnnotationGeometry::BoundingBox(BoundingBox {
            x: 0.12 + index as f32 * 0.48,
            y: 0.18,
            width: 0.26,
            height: 0.62,
        }),
        author_user_id: UserId::from("import"),
        created_at: timestamp(),
        updated_at: timestamp(),
        deleted: false,
    }
}

fn skeleton_annotation(target: &MigrationTarget, task_id: &TaskId) -> AnnotationVersion {
    AnnotationVersion {
        annotation_id: target.reserved_skeleton_annotation_id.clone(),
        version: 1,
        object_group_id: Some(target.object_group_id.clone()),
        origin: AnnotationOrigin::native(),
        task_id: task_id.clone(),
        class_id: ClassId::from("person"),
        annotation_type: AnnotationType::Skeleton,
        revision_source: RevisionSource::Human {
            action: HumanRevisionKind::Authored,
        },
        geometry: AnnotationGeometry::Skeleton(labello_domain::SkeletonGeometry {
            keypoints: vec![labello_domain::KeypointAnnotation {
                name: "head".to_string(),
                state: labello_domain::KeypointState::Visible,
                point: Some(labello_domain::NormalizedPoint { x: 0.25, y: 0.25 }),
            }],
        }),
        author_user_id: UserId::from("demo_user"),
        created_at: timestamp(),
        updated_at: timestamp(),
        deleted: false,
    }
}

fn work_preset(kind: AssignmentKind, ctx: &egui::Context) -> LabelloApp {
    let view = match kind {
        AssignmentKind::Annotation => AppView::Annotate,
        AssignmentKind::Review => AppView::Review,
        AssignmentKind::Adjudication => AppView::Adjudicate,
    };
    let mut app = LabelloApp {
        view,
        ..Default::default()
    };
    seed_dataset(&mut app);
    let image_id = app.work.current.as_ref().unwrap().image.image_id.clone();
    app.work.current_state = Some(ImageState::new(image_id.clone()));
    app.work.current_texture = Some(ctx.load_texture(
        "inspector-preview",
        preview_image(),
        egui::TextureOptions::LINEAR,
    ));
    app.work.assignment = Some(Assignment {
        assignment_id: AssignmentId::from("asg_inspector"),
        image_id,
        task_id: TaskId::from("bounding_box:person"),
        assigned_to: app.config.user_id.clone(),
        kind,
        status: AssignmentStatus::Active,
        expires_at: None,
        created_at: timestamp(),
        updated_at: timestamp(),
    });
    let annotation = sample_annotation();
    app.work
        .persisted_annotations
        .insert(annotation.annotation_id.clone());
    app.work.selected_annotation = Some(annotation.annotation_id.clone());
    app.work.annotations = vec![annotation];
    app
}

fn setup_preset() -> LabelloApp {
    let mut app = LabelloApp::default();
    seed_dataset(&mut app);
    app.view = AppView::Setup;
    app
}

fn admin_preset() -> LabelloApp {
    let mut app = setup_preset();
    app.view = AppView::Admin;
    app
}

fn statistics_preset() -> LabelloApp {
    let mut app = setup_preset();
    app.view = AppView::Stats;
    app.datasets.stats = DatasetStats {
        total_images: 24,
        completed_tasks: 18,
        pending_tasks: 6,
        reviewed_tasks: 14,
        unreviewed_tasks: 4,
        approved_tasks: 12,
        rejected_tasks: 2,
        reviewer_corrected_tasks: 3,
        finalized_tasks: 14,
        per_task: [(
            TaskId::from("bounding_box:person"),
            TaskStats {
                completed: 18,
                pending: 6,
                reviewed: 14,
                unreviewed: 4,
                approved: 12,
                rejected: 2,
                reviewer_corrected: 3,
                finalized: 14,
                provenance: Default::default(),
                migration: Default::default(),
            },
        )]
        .into(),
        per_class: [(
            ClassId::from("person"),
            ClassStats {
                annotations: 31,
                completed_tasks: 18,
                provenance: Default::default(),
            },
        )]
        .into(),
        throughput: vec![
            ThroughputPoint {
                day: "2026-07-22".to_string(),
                annotations: 9,
                reviews: 5,
            },
            ThroughputPoint {
                day: "2026-07-23".to_string(),
                annotations: 13,
                reviews: 8,
            },
            ThroughputPoint {
                day: "2026-07-24".to_string(),
                annotations: 11,
                reviews: 10,
            },
        ],
        provenance: Default::default(),
        migration: Default::default(),
        import_coverage: Default::default(),
    };
    app.datasets.last_stats_completion =
        Some(web_time::Instant::now() + web_time::Duration::from_secs(100 * 365 * 24 * 60 * 60));
    app
}

fn seed_dataset(app: &mut LabelloApp) {
    let roles = vec![
        DatasetRole::Annotator,
        DatasetRole::Reviewer,
        DatasetRole::Adjudicator,
        DatasetRole::DataAdmin,
    ];
    let account = sample_account(app.config.user_id.clone());
    let mut metadata =
        DatasetMetadata::new(app.config.dataset_id.clone(), "Demo Dataset", timestamp());
    metadata.label_classes = app.work.classes.clone();
    metadata.tasks = app.work.tasks.clone();
    let image = app.work.current.as_ref().unwrap().image.clone();
    metadata.images.insert(image.image_id.clone(), image);
    metadata.role_assignments = vec![DatasetRoleAssignment {
        dataset_id: app.config.dataset_id.clone(),
        user_id: account.user_id.clone(),
        roles: roles.iter().cloned().collect::<BTreeSet<_>>(),
        assigned_at: timestamp(),
        assigned_by: Some(account.user_id.clone()),
    }];
    app.auth.account = Some(account.clone());
    app.auth.can_create_datasets = true;
    app.auth.checked = true;
    app.datasets.summaries = vec![DatasetSummary {
        dataset_id: app.config.dataset_id.clone(),
        name: metadata.name.clone(),
        roles: roles.clone(),
        total_images: metadata.images.len(),
    }];
    app.datasets.metadata = Some(metadata.clone());
    app.datasets.admin_config = Some(metadata.clone());
    app.datasets.admin_baseline = Some(metadata);
    app.datasets.users = vec![DatasetUser {
        account,
        roles: roles.clone(),
    }];
    app.datasets.users_baseline = app.datasets.users.clone();
}

fn sample_account(user_id: UserId) -> UserAccount {
    UserAccount {
        user_id,
        display_name: "Demo Annotator".to_string(),
        github_user_id: None,
        github_login: Some("demo-annotator".to_string()),
        created_at: timestamp(),
        updated_at: timestamp(),
    }
}

fn sample_annotation() -> AnnotationVersion {
    AnnotationVersion {
        annotation_id: AnnotationId::from("ann_inspector"),
        version: 1,
        object_group_id: None,
        origin: AnnotationOrigin::native(),
        task_id: TaskId::from("bounding_box:person"),
        class_id: ClassId::from("person"),
        annotation_type: AnnotationType::BoundingBox,
        revision_source: RevisionSource::Human {
            action: HumanRevisionKind::Authored,
        },
        geometry: AnnotationGeometry::BoundingBox(BoundingBox {
            x: 0.28,
            y: 0.2,
            width: 0.3,
            height: 0.54,
        }),
        author_user_id: UserId::from("demo_user"),
        created_at: timestamp(),
        updated_at: timestamp(),
        deleted: false,
    }
}

fn preview_image() -> egui::ColorImage {
    let size = [16, 10];
    let mut image = egui::ColorImage::filled(size, egui::Color32::from_rgb(38, 54, 74));
    for y in 0..size[1] {
        for x in 0..size[0] {
            image.pixels[y * size[0] + x] = if y > 6 {
                egui::Color32::from_rgb(56, 74, 66)
            } else if (x + y) % 5 == 0 {
                egui::Color32::from_rgb(72, 94, 118)
            } else {
                egui::Color32::from_rgb(40, 58, 80)
            };
        }
    }
    image
}

fn clear_image(app: &mut LabelloApp) {
    app.work.current = None;
    app.work.current_state = None;
    app.work.current_texture = None;
}

fn timestamp() -> labello_domain::Timestamp {
    "2026-07-24T12:00:00Z".parse().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_names_are_unique_and_round_trip() {
        let names = InspectorPreset::ALL
            .into_iter()
            .map(InspectorPreset::name)
            .collect::<BTreeSet<_>>();
        assert_eq!(names.len(), InspectorPreset::ALL.len());
        for preset in InspectorPreset::ALL {
            assert_eq!(InspectorPreset::from_name(preset.name()), Some(preset));
        }
    }

    #[test]
    fn every_preset_builds() {
        let ctx = egui::Context::default();
        for preset in InspectorPreset::ALL {
            let _ = build(preset, &ctx);
        }
    }

    #[test]
    fn specialized_presets_keep_their_required_state() {
        let ctx = egui::Context::default();
        let correction = build(InspectorPreset::ReviewCorrection, &ctx);
        assert!(correction.tasks[0].review.allow_reviewer_corrections);
        assert!(correction.correction_draft.is_some());

        let statistics = build(InspectorPreset::Statistics, &ctx);
        assert!(
            statistics
                .datasets
                .last_stats_completion
                .unwrap()
                .checked_duration_since(web_time::Instant::now())
                > Some(web_time::Duration::from_secs(90 * 365 * 24 * 60 * 60))
        );
    }
}
