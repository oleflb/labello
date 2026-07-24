use std::collections::BTreeSet;

use eframe::egui;
use labello_client::{DatasetSummary, DatasetUser};
use labello_domain::{
    AnnotationGeometry, AnnotationId, AnnotationSource, AnnotationType, AnnotationVersion,
    Assignment, AssignmentId, AssignmentKind, AssignmentStatus, BoundingBox, ClassId, ClassStats,
    CorrectionId, DatasetMetadata, DatasetRole, DatasetRoleAssignment, DatasetStats, ImageState,
    TaskId, TaskStats, ThroughputPoint, UserAccount, UserId,
};

use crate::app::{AppView, CorrectionDraft, LabelloApp, PendingTransition};

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
}

impl InspectorPreset {
    pub const ALL: [Self; 15] = [
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
            app.tasks[0].review.allow_reviewer_corrections = true;
            let annotation = app.annotations[0].clone();
            app.correction_draft = Some(CorrectionDraft {
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
            app.pending_transition = Some(PendingTransition::View(AppView::Review));
            app
        }
        InspectorPreset::DialogAdminDiscard => {
            let mut app = admin_preset();
            app.datasets.admin_config.as_mut().unwrap().name = "Staged dataset name".to_string();
            app.admin_tools.confirm_discard = true;
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
            app.admin_tools.load_error = Some("Admin configuration is unavailable".to_string());
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
            app.assignment = None;
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
    let image_id = app.current.as_ref().unwrap().image.image_id.clone();
    app.current_state = Some(ImageState::new(image_id.clone()));
    app.current_texture = Some(ctx.load_texture(
        "inspector-preview",
        preview_image(),
        egui::TextureOptions::LINEAR,
    ));
    app.assignment = Some(Assignment {
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
    app.persisted_annotations
        .insert(annotation.annotation_id.clone());
    app.selected_annotation = Some(annotation.annotation_id.clone());
    app.annotations = vec![annotation];
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
            },
        )]
        .into(),
        per_class: [(
            ClassId::from("person"),
            ClassStats {
                annotations: 31,
                completed_tasks: 18,
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
    metadata.label_classes = app.classes.clone();
    metadata.tasks = app.tasks.clone();
    let image = app.current.as_ref().unwrap().image.clone();
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
        task_id: TaskId::from("bounding_box:person"),
        class_id: ClassId::from("person"),
        annotation_type: AnnotationType::BoundingBox,
        source: AnnotationSource::Human,
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
    app.current = None;
    app.current_state = None;
    app.current_texture = None;
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
