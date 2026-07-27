use eframe::egui::{self, RichText};
use labello_client::{
    CommitImportRequest, CreateImportRequest, ImportAcknowledgementRequest, ImportAttestations,
    ImportCapabilities, ImportCategoryMappingRequest, ImportDescriptorKind,
    ImportDescriptorSelection, ImportDiagnosticSeverity, ImportGeometryKind,
    ImportGeometryMappingRequest, ImportGeometryPolicy, ImportJob, ImportLifecycle,
    ImportMappingParameter, ImportPlan, ImportProfile, ImportSourceConfiguration,
    ImportSourceSelection, ImportTaskMappingRequest, ImportTransport, ImportWorkflowIntent,
    SealImportRequest, StartImportPreflightRequest, UpdateImportPlanRequest,
};
use labello_domain::{
    AnnotationType, ClassId, DatasetId, ReviewConfig, SkeletonSpec, TaskDefinition, TaskId,
    TutorialContent,
};

use crate::{
    app::{AppView, ImportActivity, LabelloApp, UiCommand},
    theme,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImportDescriptorDraft {
    pub descriptor_file_id: String,
    pub kind: ImportDescriptorKind,
    pub release: String,
    pub split: String,
    pub image_root_file_id: String,
    pub pairing_group: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImportYoloSplitDraft {
    pub name: String,
    pub usable: bool,
    pub selected: bool,
    pub issue: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImportSourcePickerTarget {
    DatasetFolder,
    Descriptor(usize),
    CocoImageRoot(usize),
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ImportSourcePickerState {
    pub target: Option<ImportSourcePickerTarget>,
    pub relative_path: String,
    pub page: Option<labello_client::ImportBrowsePage>,
    pub loading: bool,
    pub error: Option<String>,
    pub pending_request_id: Option<u64>,
    pub pending_append: bool,
}

impl Default for ImportDescriptorDraft {
    fn default() -> Self {
        Self {
            descriptor_file_id: String::new(),
            kind: ImportDescriptorKind::CocoInstances,
            release: "default".to_string(),
            split: "train".to_string(),
            image_root_file_id: String::new(),
            pairing_group: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ImportCategoryDraft {
    pub selected: bool,
    pub source_category_key: String,
    pub source_category_id: String,
    pub source_name: String,
    pub class_id: String,
    pub class_name: String,
    pub class_color: String,
    pub bounding_box_task_id: String,
    pub bounding_box_task_name: String,
    pub skeleton_task_id: String,
    pub skeleton_task_name: String,
    pub source_skeleton: Option<SkeletonSpec>,
    pub direct_geometry: Vec<ImportGeometryKind>,
    pub geometry_mappings: Vec<ImportGeometryMappingRequest>,
    pub task_mappings: Vec<ImportTaskMappingRequest>,
    pub skeleton_mappings: Vec<labello_client::ImportSkeletonMappingRequest>,
    pub workflow_intent: ImportWorkflowIntent,
    pub target_keypoint_names: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RegisteredImportPath {
    pub client_file_id: String,
    pub file_id: String,
    pub relative_path: String,
}

pub struct RawImportChunkRequest {
    pub api_base_url: String,
    pub import_id: String,
    pub file_id: String,
    pub offset: u64,
    pub length: u64,
    pub digest: String,
    pub bytes: Vec<u8>,
    pub csrf_token: String,
    pub idempotency_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawImportChunkResponse {
    pub accepted_offset: u64,
    pub complete: bool,
}

pub type RawImportChunkUploader = std::rc::Rc<
    dyn Fn(
        RawImportChunkRequest,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<RawImportChunkResponse, String>>>,
    >,
>;

#[cfg(target_arch = "wasm32")]
pub(crate) struct BrowserImportFile {
    pub client_file_id: String,
    pub relative_path: String,
    pub byte_size: u64,
    pub blake3: String,
    pub file: web_sys::File,
}

#[cfg(target_arch = "wasm32")]
impl std::fmt::Debug for BrowserImportFile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrowserImportFile")
            .field("client_file_id", &self.client_file_id)
            .field("relative_path", &"<redacted>")
            .field("byte_size", &self.byte_size)
            .field("blake3", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ImportScreen {
    #[default]
    Source,
    Configure,
    Preflight,
    Ready,
    Running,
    Failure,
    Success,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportStage {
    Source,
    Configure,
    Preflight,
    Ready,
    Import,
}

impl ImportStage {
    const ALL: [Self; 5] = [
        Self::Source,
        Self::Configure,
        Self::Preflight,
        Self::Ready,
        Self::Import,
    ];

    fn index(self) -> usize {
        match self {
            Self::Source => 0,
            Self::Configure => 1,
            Self::Preflight => 2,
            Self::Ready => 3,
            Self::Import => 4,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Source => "Source",
            Self::Configure => "Configure",
            Self::Preflight => "Preflight",
            Self::Ready => "Ready",
            Self::Import => "Import",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportStageStatus {
    Pending,
    Active,
    Complete,
    Failed,
}

struct ActiveStageProgress {
    label: String,
    fraction: Option<f32>,
}

pub(crate) struct ImportFlowState {
    pub capabilities: Option<ImportCapabilities>,
    pub capabilities_loading: bool,
    pub capabilities_error: Option<String>,
    pub open: bool,
    pub screen: ImportScreen,
    pub busy: bool,
    pub active_operations: std::collections::BTreeMap<u64, ImportActivity>,
    pub error: Option<String>,
    pub job: Option<ImportJob>,
    pub plan: Option<ImportPlan>,
    pub destination_id: String,
    pub destination_name: String,
    pub profile: ImportProfile,
    pub transport: ImportTransport,
    pub server_root_id: String,
    pub server_relative_path: String,
    pub source_picker: ImportSourcePickerState,
    pub ground_truth: bool,
    pub exhaustive: bool,
    pub coverage_scope: String,
    pub provenance: String,
    pub source_namespace: String,
    pub descriptors: Vec<ImportDescriptorDraft>,
    pub yolo_splits: Vec<ImportYoloSplitDraft>,
    pub yolo_inspection_loading: bool,
    pub yolo_inspection_error: Option<String>,
    pub yolo_inspected_descriptor_file_id: Option<String>,
    pub pending_yolo_inspection_request_id: Option<u64>,
    pub yolo_inspection_retry_after_current: bool,
    pub registered_paths: Vec<RegisteredImportPath>,
    pub categories: Vec<ImportCategoryDraft>,
    pub direct_bounding_boxes: bool,
    pub direct_skeletons: bool,
    pub target_geometry: ImportGeometryKind,
    pub geometry_policy: ImportGeometryPolicy,
    pub workflow_intent: ImportWorkflowIntent,
    pub yolo_missing_labels: labello_client::YoloMissingLabelPolicy,
    pub yolo_duplicate_rows: labello_client::YoloDuplicateRowPolicy,
    pub coco_crowds: labello_client::CocoCrowdPolicy,
    pub coco_structure: labello_client::CocoStructurePolicy,
    pub geometry_bounds: labello_client::GeometryBoundsPolicy,
    pub cross_split_duplicates: labello_client::CrossSplitDuplicatePolicy,
    pub missing_keypoint_names: labello_client::MissingKeypointNamesPolicy,
    pub keypoint_names: String,
    pub seed_workflow_confirmed: bool,
    pub acknowledgements: std::collections::BTreeSet<String>,
    pub pending_plan_request: Option<UpdateImportPlanRequest>,
    pub accepted_plan_request: Option<UpdateImportPlanRequest>,
    pub recovery_contract_gap: bool,
    pub recovery_import_id: String,
    pub poll_after: Option<web_time::Instant>,
    pub diagnostics: Vec<labello_client::ImportDiagnostic>,
    pub diagnostics_cursor: Option<String>,
    #[cfg(target_arch = "wasm32")]
    pub browser_files: std::collections::BTreeMap<String, web_sys::File>,
    #[cfg(target_arch = "wasm32")]
    pub browser_uploads: Vec<labello_client::RegisteredImportFile>,
}

impl Default for ImportFlowState {
    fn default() -> Self {
        Self {
            capabilities: None,
            capabilities_loading: false,
            capabilities_error: None,
            open: false,
            screen: ImportScreen::Source,
            busy: false,
            active_operations: Default::default(),
            error: None,
            job: None,
            plan: None,
            destination_id: String::new(),
            destination_name: String::new(),
            profile: ImportProfile::UltralyticsYoloDetectV1,
            transport: ImportTransport::BrowserFolder,
            server_root_id: String::new(),
            server_relative_path: String::new(),
            source_picker: Default::default(),
            ground_truth: false,
            exhaustive: false,
            coverage_scope: String::new(),
            provenance: String::new(),
            source_namespace: "source".to_string(),
            descriptors: vec![descriptor_draft(ImportProfile::UltralyticsYoloDetectV1)],
            yolo_splits: Vec::new(),
            yolo_inspection_loading: false,
            yolo_inspection_error: None,
            yolo_inspected_descriptor_file_id: None,
            pending_yolo_inspection_request_id: None,
            yolo_inspection_retry_after_current: false,
            registered_paths: Vec::new(),
            categories: Vec::new(),
            direct_bounding_boxes: true,
            direct_skeletons: false,
            target_geometry: ImportGeometryKind::BoundingBox,
            geometry_policy: ImportGeometryPolicy::Direct,
            workflow_intent: ImportWorkflowIntent::AuthoritativeGroundTruth,
            yolo_missing_labels: Default::default(),
            yolo_duplicate_rows: Default::default(),
            coco_crowds: Default::default(),
            coco_structure: Default::default(),
            geometry_bounds: Default::default(),
            cross_split_duplicates: Default::default(),
            missing_keypoint_names: Default::default(),
            keypoint_names: String::new(),
            seed_workflow_confirmed: false,
            acknowledgements: Default::default(),
            pending_plan_request: None,
            accepted_plan_request: None,
            recovery_contract_gap: false,
            recovery_import_id: String::new(),
            poll_after: None,
            diagnostics: Vec::new(),
            diagnostics_cursor: None,
            #[cfg(target_arch = "wasm32")]
            browser_files: Default::default(),
            #[cfg(target_arch = "wasm32")]
            browser_uploads: Vec::new(),
        }
    }
}

impl ImportFlowState {
    pub(crate) fn reset_job(&mut self) {
        let capabilities = self.capabilities.take();
        let open = self.open;
        *self = Self::default();
        self.capabilities = capabilities;
        self.open = open;
    }

    pub(crate) fn normalize_capability_selection(&mut self, capabilities: &ImportCapabilities) {
        let previous_profile = self.profile;
        if !capabilities
            .profiles
            .iter()
            .any(|entry| entry.enabled && entry.profile == self.profile)
            && let Some(profile) = capabilities.profiles.iter().find(|entry| entry.enabled)
        {
            self.profile = profile.profile;
        }
        if !capabilities
            .transports
            .iter()
            .any(|entry| entry.enabled && entry.transport == self.transport)
            && let Some(transport) = capabilities.transports.iter().find(|entry| entry.enabled)
        {
            self.transport = transport.transport;
        }
        if self.transport == ImportTransport::ServerDirectory
            && !capabilities
                .server_roots
                .iter()
                .any(|root| root.root_id == self.server_root_id)
        {
            self.server_root_id = capabilities
                .server_roots
                .first()
                .map(|root| root.root_id.clone())
                .unwrap_or_default();
        }
        if self.geometry_policy == ImportGeometryPolicy::ManualBoxGuideV1
            && !capabilities.manual_box_guide_migration
        {
            self.geometry_policy = ImportGeometryPolicy::Direct;
        }
        if self.profile != previous_profile {
            self.descriptors = vec![descriptor_draft(self.profile)];
            self.invalidate_yolo_inspection();
            self.categories.clear();
            self.direct_bounding_boxes = true;
            self.direct_skeletons = profile_has_skeletons(self.profile);
        }
    }

    pub(crate) fn hydrate_job_contract(&mut self, job: &ImportJob) {
        self.profile = job.profile;
        self.transport = job.transport;
        self.destination_id = job.destination_dataset_id.to_string();
        self.destination_name = job.destination_name.clone();
        let Some(recovery) = job.recovery.as_ref() else {
            return;
        };
        self.recovery_contract_gap = false;
        self.ground_truth = recovery.attestations.ground_truth;
        self.exhaustive = recovery.attestations.exhaustive;
        self.coverage_scope = recovery.attestations.coverage_scope.join(", ");
        self.provenance = recovery.attestations.provenance.clone();
        self.server_root_id = recovery.server_root_id.clone().unwrap_or_default();
        self.registered_paths = recovery
            .registered_files
            .iter()
            .map(|file| RegisteredImportPath {
                client_file_id: file.client_file_id.clone(),
                file_id: file.file_id.clone(),
                relative_path: if file.client_file_id.is_empty() {
                    file.file_id.clone()
                } else {
                    file.client_file_id.clone()
                },
            })
            .collect();
        if recovery.source.is_none() {
            self.descriptors = vec![descriptor_draft(self.profile)];
            self.invalidate_yolo_inspection();
        }
        if let Some(source) = recovery.source.as_ref() {
            self.source_namespace = source.source_namespace.clone();
            self.descriptors = source
                .descriptors
                .iter()
                .map(|descriptor| ImportDescriptorDraft {
                    descriptor_file_id: descriptor.descriptor_file_id.clone(),
                    kind: descriptor.kind,
                    release: descriptor.release.clone(),
                    split: descriptor.split.clone(),
                    image_root_file_id: descriptor.image_root_file_id.clone().unwrap_or_default(),
                    pairing_group: descriptor.pairing_group.clone().unwrap_or_default(),
                })
                .collect();
            if !is_coco_profile(self.profile) {
                self.yolo_splits = source
                    .selected_splits
                    .iter()
                    .map(|name| ImportYoloSplitDraft {
                        name: name.clone(),
                        usable: true,
                        selected: true,
                        issue: None,
                    })
                    .collect();
                self.yolo_inspected_descriptor_file_id = self
                    .descriptors
                    .first()
                    .map(|descriptor| descriptor.descriptor_file_id.clone());
                self.yolo_inspection_loading = false;
                self.yolo_inspection_error = None;
                self.pending_yolo_inspection_request_id = None;
                self.yolo_inspection_retry_after_current = false;
            }
        }
        let Some(plan) = recovery.accepted_plan.as_ref() else {
            self.plan = None;
            self.accepted_plan_request = None;
            self.categories.clear();
            return;
        };
        let accepted = plan.accepted_request.clone();
        self.categories =
            plan.source_categories
                .iter()
                .map(|source| {
                    let category = &source.current_category_mapping;
                    let box_task = source.current_task_mappings.iter().find(|mapping| {
                        mapping.task.annotation_type == AnnotationType::BoundingBox
                    });
                    let skeleton_task = source
                        .current_task_mappings
                        .iter()
                        .find(|mapping| mapping.task.annotation_type == AnnotationType::Skeleton);
                    let target_skeleton = skeleton_task
                        .and_then(|task| {
                            source
                                .current_skeleton_mappings
                                .iter()
                                .find(|mapping| mapping.target_task_id == task.task.task_id)
                        })
                        .map(|mapping| &mapping.skeleton)
                        .or(source.keypoint_schema.as_ref());
                    ImportCategoryDraft {
                        selected: category.selected,
                        source_category_key: source.source_category_key.clone(),
                        source_category_id: source.source_category_id.clone(),
                        source_name: source.source_name.clone(),
                        class_id: category.class_id.to_string(),
                        class_name: category.class_name.clone(),
                        class_color: category.color.clone(),
                        bounding_box_task_id: box_task
                            .map(|mapping| mapping.task.task_id.to_string())
                            .unwrap_or_else(|| format!("bounding_box:{}", category.class_id)),
                        bounding_box_task_name: box_task
                            .map(|mapping| mapping.task.name.clone())
                            .unwrap_or_else(|| format!("{} boxes", category.class_name)),
                        skeleton_task_id: skeleton_task
                            .map(|mapping| mapping.task.task_id.to_string())
                            .unwrap_or_else(|| format!("skeleton:{}", category.class_id)),
                        skeleton_task_name: skeleton_task
                            .map(|mapping| mapping.task.name.clone())
                            .unwrap_or_else(|| format!("{} skeletons", category.class_name)),
                        source_skeleton: source.keypoint_schema.clone(),
                        direct_geometry: source.direct_geometry.clone(),
                        geometry_mappings: source.current_geometry_mappings.clone(),
                        task_mappings: source.current_task_mappings.clone(),
                        skeleton_mappings: source.current_skeleton_mappings.clone(),
                        workflow_intent: source
                            .current_task_mappings
                            .first()
                            .map(|mapping| mapping.workflow_intent)
                            .unwrap_or(ImportWorkflowIntent::AuthoritativeGroundTruth),
                        target_keypoint_names: target_skeleton
                            .map(|schema| {
                                schema
                                    .keypoints
                                    .iter()
                                    .map(|point| point.name.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_default(),
                    }
                })
                .collect();
        if let Some(request) = accepted.as_ref() {
            self.yolo_missing_labels = request.compatibility.yolo_missing_labels;
            self.yolo_duplicate_rows = request.compatibility.yolo_duplicate_rows;
            self.coco_crowds = request.compatibility.coco_crowds;
            self.coco_structure = request.compatibility.coco_structure;
            self.geometry_bounds = request.compatibility.geometry_bounds;
            self.cross_split_duplicates = request.compatibility.cross_split_duplicates;
            self.missing_keypoint_names = request.compatibility.missing_keypoint_names;
            self.acknowledgements = request
                .acknowledgements
                .iter()
                .filter(|acknowledgement| acknowledgement.acknowledged)
                .map(|acknowledgement| acknowledgement.diagnostic_code.clone())
                .collect();
        }
        self.accepted_plan_request = accepted;
        self.plan = Some(plan.clone());
    }

    pub(crate) fn invalidate_yolo_inspection(&mut self) {
        self.yolo_splits.clear();
        self.yolo_inspection_loading = false;
        self.yolo_inspection_error = None;
        self.yolo_inspected_descriptor_file_id = None;
        self.pending_yolo_inspection_request_id = None;
        self.yolo_inspection_retry_after_current = false;
    }
}

impl LabelloApp {
    pub(crate) fn request_import_capabilities(&mut self) {
        if self.runtime.api.is_none()
            || self.auth.account.is_none()
            || !self.auth.can_create_datasets
            || self.import_flow.capabilities_loading
            || self.import_flow.capabilities.is_some()
        {
            return;
        }
        self.import_flow.capabilities_loading = true;
        self.import_flow.capabilities_error = None;
        let request = self.import_request_identity(None);
        self.queue_command(UiCommand::ImportCapabilities { request });
    }

    pub(crate) fn refresh_import_if_due(&mut self) {
        let should_poll = self.import_flow.open
            && !self
                .import_flow
                .active_operations
                .values()
                .any(|activity| *activity == ImportActivity::LoadStatus)
            && self.import_flow.job.as_ref().is_some_and(|job| {
                matches!(
                    job.lifecycle,
                    ImportLifecycle::Preflighting
                        | ImportLifecycle::Building
                        | ImportLifecycle::Verifying
                        | ImportLifecycle::Committing
                )
            })
            && self
                .import_flow
                .poll_after
                .is_none_or(|deadline| web_time::Instant::now() >= deadline);
        if should_poll {
            self.request_import_poll();
        }
    }

    pub(crate) fn import_setup_section(&mut self, ui: &mut egui::Ui) {
        if !self.import_flow.open {
            self.import_flow.open = true;
            if self.import_flow.destination_id.is_empty() {
                self.import_flow.destination_id = "imported-dataset".to_string();
                self.import_flow.destination_name = "Imported dataset".to_string();
            }
        }
        ui.heading("Import dataset");
        ui.label(
            RichText::new("Register, validate, and import an existing dataset.")
                .color(theme::TEXT_MUTED),
        );
        ui.add_space(theme::SPACE_2);
        self.request_import_capabilities();
        theme::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            self.import_progress_overview(ui);
            ui.separator();
            let Some(capabilities) = self.import_flow.capabilities.clone() else {
                if self.import_flow.capabilities_loading {
                    ui.small("Checking dataset import capability...");
                } else if let Some(error) = self.import_flow.capabilities_error.clone() {
                    theme::inline_message(ui, theme::Intent::Warning, error);
                }
                return;
            };
            if !capabilities.available {
                if let Some(reason) = capabilities.unavailable_reason {
                    ui.small(format!("Dataset import unavailable: {reason}"));
                }
                return;
            }
            self.import_flow_contents(ui, &capabilities);
        });
        self.import_source_picker_modal(ui.ctx());
    }

    fn import_flow_contents(&mut self, ui: &mut egui::Ui, capabilities: &ImportCapabilities) {
        if let Some(error) = self.import_flow.error.clone() {
            theme::inline_message(ui, theme::Intent::Error, error);
        }
        if self.import_flow.recovery_contract_gap
            && matches!(
                self.import_flow.screen,
                ImportScreen::Configure
                    | ImportScreen::Preflight
                    | ImportScreen::Ready
                    | ImportScreen::Failure
            )
        {
            theme::inline_message(
                ui,
                theme::Intent::Error,
                "This recovered job does not include its attestations, source descriptors, category identities/schema, or accepted mapping request in the current API contract. Unsafe continuation is disabled.",
            );
            if theme::primary_button(
                ui,
                !self.import_flow.busy,
                egui::Button::new("Restart import setup"),
            )
            .clicked()
            {
                self.restart_import_setup();
            }
            return;
        }
        match self.import_flow.screen {
            ImportScreen::Source => self.import_source_step(ui, capabilities),
            ImportScreen::Configure => self.import_transport_step(ui),
            ImportScreen::Preflight | ImportScreen::Ready => self.import_preflight_step(ui),
            ImportScreen::Running => self.import_running_step(ui),
            ImportScreen::Failure => self.import_failure_step(ui),
            ImportScreen::Success => self.import_success_step(ui),
        }
        ui.separator();
        ui.collapsing("Recover an import", |ui| {
            theme::labeled_text_field(
                ui,
                "Import ID",
                &mut self.import_flow.recovery_import_id,
                theme::COMPACT_TEXT_FIELD_HEIGHT,
            );
            if ui
                .add_enabled(
                    !self.import_flow.busy
                        && !self.import_flow.recovery_import_id.trim().is_empty(),
                    egui::Button::new("Resume import"),
                )
                .clicked()
            {
                self.request_import_recovery();
            }
        });
    }

    fn import_progress_overview(&self, ui: &mut egui::Ui) {
        let activity = self.current_import_activity();
        let show_activity_label = ui.available_width() >= 520.0;
        ui.horizontal(|ui| {
            ui.heading("Import progress");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(activity) = activity {
                    let status = format!("{} | {}", activity.label(), activity.operation());
                    let response = ui.spinner().on_hover_text(&status);
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Other, true, status.clone())
                    });
                    if show_activity_label {
                        ui.label(
                            RichText::new(activity.label())
                                .small()
                                .color(theme::TEXT_MUTED),
                        );
                    }
                }
            });
        });

        let active_stage = current_import_stage(&self.import_flow);
        let active_progress = self.active_stage_progress(active_stage, activity);
        let pill_width = 98.0;
        let columns = (((ui.available_width() + theme::SPACE_2) / (pill_width + theme::SPACE_2))
            .floor() as usize)
            .clamp(1, ImportStage::ALL.len());
        for row in ImportStage::ALL.chunks(columns) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = theme::SPACE_2;
                for &stage in row {
                    let status = import_stage_status(&self.import_flow, stage);
                    let fraction = match status {
                        ImportStageStatus::Complete | ImportStageStatus::Failed => Some(1.0),
                        ImportStageStatus::Pending => Some(0.0),
                        ImportStageStatus::Active => active_progress.fraction,
                    };
                    import_stage_pill(ui, stage, status, fraction);
                }
            });
        }
        ui.add_space(theme::SPACE_2);
        ui.label(
            RichText::new(import_step_label(self.import_flow.screen)).color(theme::TEXT_MUTED),
        );
        let progress_color = if self.import_flow.screen == ImportScreen::Failure {
            theme::DANGER
        } else if self.import_flow.screen == ImportScreen::Success {
            theme::SUCCESS
        } else {
            theme::ACCENT
        };
        ui.label(RichText::new(&active_progress.label).strong());
        if let Some(fraction) = active_progress.fraction {
            let progress = egui::ProgressBar::new(fraction)
                .desired_height(18.0)
                .fill(progress_color);
            let show_value = self.import_flow.screen != ImportScreen::Failure;
            let response = ui.add(if show_value {
                progress.show_percentage()
            } else {
                progress.text("Blocked")
            });
            response.widget_info(|| {
                let mut info = egui::WidgetInfo::labeled(
                    egui::WidgetType::ProgressIndicator,
                    true,
                    active_progress.label.clone(),
                );
                if show_value {
                    info.value = Some((fraction.clamp(0.0, 1.0) * 100.0).floor() as f64);
                }
                info
            });
        } else {
            indeterminate_import_progress(ui, &active_progress.label, progress_color);
        }
    }

    fn current_import_activity(&self) -> Option<ImportActivity> {
        self.import_flow
            .active_operations
            .values()
            .copied()
            .max_by_key(|activity| activity.priority())
            .or_else(|| {
                self.import_flow
                    .capabilities_loading
                    .then_some(ImportActivity::CheckCapabilities)
            })
            .or_else(|| {
                self.import_flow
                    .source_picker
                    .loading
                    .then_some(ImportActivity::BrowseSource)
            })
            .or_else(|| {
                self.import_flow
                    .yolo_inspection_loading
                    .then_some(ImportActivity::InspectDescriptor)
            })
            .or_else(|| {
                (self.import_flow.screen == ImportScreen::Success && self.loading.datasets)
                    .then_some(ImportActivity::RefreshDatasets)
            })
            .or_else(|| {
                self.import_flow
                    .busy
                    .then_some(match self.import_flow.screen {
                        ImportScreen::Source => ImportActivity::Create,
                        ImportScreen::Configure => ImportActivity::Seal,
                        ImportScreen::Preflight => ImportActivity::Preflight,
                        ImportScreen::Ready => ImportActivity::UpdatePlan,
                        ImportScreen::Running => ImportActivity::Commit,
                        ImportScreen::Failure => ImportActivity::LoadStatus,
                        ImportScreen::Success => ImportActivity::RefreshDatasets,
                    })
            })
    }

    fn active_stage_progress(
        &self,
        stage: ImportStage,
        activity: Option<ImportActivity>,
    ) -> ActiveStageProgress {
        if self.import_flow.screen == ImportScreen::Failure {
            return ActiveStageProgress {
                label: format!("{} needs attention", stage.label()),
                fraction: Some(1.0),
            };
        }
        if self.import_flow.screen == ImportScreen::Success {
            return ActiveStageProgress {
                label: "Import complete".to_string(),
                fraction: Some(1.0),
            };
        }

        match stage {
            ImportStage::Source => {
                if matches!(activity, Some(ImportActivity::Create)) {
                    return ActiveStageProgress {
                        label: activity.unwrap().label().to_string(),
                        fraction: None,
                    };
                }
                let dataset_id = DatasetId::from(self.import_flow.destination_id.trim());
                let source_selected = self.import_flow.transport == ImportTransport::BrowserFolder
                    || (!self.import_flow.server_root_id.is_empty()
                        && !self.import_flow.server_relative_path.trim().is_empty());
                let complete = [
                    dataset_id.validate_path_segment().is_ok(),
                    !self.import_flow.destination_name.trim().is_empty(),
                    source_selected,
                    self.import_flow.ground_truth,
                    !self.import_flow.provenance.trim().is_empty(),
                ]
                .into_iter()
                .filter(|ready| *ready)
                .count();
                ActiveStageProgress {
                    label: format!("Source setup: {complete} of 5 requirements complete"),
                    fraction: Some(complete as f32 / 5.0),
                }
            }
            ImportStage::Configure => {
                if let Some(job) = &self.import_flow.job
                    && job.transport == ImportTransport::BrowserFolder
                    && job.progress.total_bytes > 0
                    && job.progress.accepted_bytes < job.progress.total_bytes
                {
                    return ActiveStageProgress {
                        label: format!(
                            "Uploading source: {} of {} files, {} of {}",
                            job.progress.uploaded_files,
                            job.progress.total_files,
                            import_human_bytes(job.progress.accepted_bytes),
                            import_human_bytes(job.progress.total_bytes),
                        ),
                        fraction: Some(
                            job.progress.accepted_bytes as f32 / job.progress.total_bytes as f32,
                        ),
                    };
                }
                if let Some(activity) = activity {
                    return ActiveStageProgress {
                        label: activity.label().to_string(),
                        fraction: None,
                    };
                }
                let upload_ready = self.import_flow.transport == ImportTransport::ServerDirectory
                    || self.import_flow.job.as_ref().is_some_and(|job| {
                        job.progress.total_files > 0
                            && job.progress.uploaded_files == job.progress.total_files
                            && job.progress.accepted_bytes == job.progress.total_bytes
                    });
                let complete = [
                    upload_ready,
                    !self.import_flow.source_namespace.trim().is_empty(),
                    self.import_descriptor_error().is_none(),
                ]
                .into_iter()
                .filter(|ready| *ready)
                .count();
                ActiveStageProgress {
                    label: format!("Source configuration: {complete} of 3 requirements complete"),
                    fraction: Some(complete as f32 / 3.0),
                }
            }
            ImportStage::Preflight => {
                if let Some(activity) = activity {
                    return ActiveStageProgress {
                        label: activity.label().to_string(),
                        fraction: None,
                    };
                }
                let report = self
                    .import_flow
                    .plan
                    .as_ref()
                    .map(|plan| &plan.report)
                    .or_else(|| {
                        self.import_flow
                            .job
                            .as_ref()
                            .and_then(|job| job.preflight_report.as_ref())
                    });
                let acknowledgements_complete = report.is_some_and(|report| {
                    report.diagnostics.iter().all(|diagnostic| {
                        !diagnostic.impact.requires_acknowledgement
                            || self.import_flow.acknowledgements.contains(&diagnostic.code)
                    })
                });
                let complete = [
                    report.is_some(),
                    self.import_mappings_complete(),
                    acknowledgements_complete,
                ]
                .into_iter()
                .filter(|ready| *ready)
                .count();
                ActiveStageProgress {
                    label: format!("Preflight review: {complete} of 3 requirements complete"),
                    fraction: Some(complete as f32 / 3.0),
                }
            }
            ImportStage::Ready => ActiveStageProgress {
                label: "Preflight accepted; ready to import".to_string(),
                fraction: Some(1.0),
            },
            ImportStage::Import => {
                let counters = self.import_flow.job.as_ref().and_then(|job| {
                    let total = job
                        .progress
                        .total_images
                        .saturating_add(job.progress.total_objects);
                    let complete = job
                        .progress
                        .processed_images
                        .saturating_add(job.progress.processed_objects);
                    (total > 0 && complete < total).then_some((complete, total))
                });
                match counters {
                    Some((complete, total)) => ActiveStageProgress {
                        label: format!("Building dataset: {complete} of {total} records processed"),
                        fraction: Some(complete as f32 / total as f32),
                    },
                    None => ActiveStageProgress {
                        label: activity.map_or_else(
                            || {
                                self.import_flow.job.as_ref().map_or_else(
                                    || "Building and publishing dataset".to_string(),
                                    |job| lifecycle_label(job.lifecycle).to_string(),
                                )
                            },
                            |activity| activity.label().to_string(),
                        ),
                        fraction: None,
                    },
                }
            }
        }
    }

    fn import_source_step(&mut self, ui: &mut egui::Ui, capabilities: &ImportCapabilities) {
        self.import_flow
            .normalize_capability_selection(capabilities);
        ui.label(RichText::new("Destination").strong());
        theme::labeled_text_field(
            ui,
            "Dataset ID",
            &mut self.import_flow.destination_id,
            theme::COMPACT_TEXT_FIELD_HEIGHT,
        );
        theme::labeled_text_field(
            ui,
            "Dataset name",
            &mut self.import_flow.destination_name,
            theme::COMPACT_TEXT_FIELD_HEIGHT,
        );
        ui.label(RichText::new("Source profile").strong());
        let previous_profile = self.import_flow.profile;
        egui::ComboBox::from_label("Import profile")
            .selected_text(profile_label(self.import_flow.profile))
            .show_ui(ui, |ui| {
                for profile in capabilities
                    .profiles
                    .iter()
                    .filter(|profile| profile.enabled)
                {
                    ui.selectable_value(
                        &mut self.import_flow.profile,
                        profile.profile,
                        if profile.display_name.is_empty() {
                            profile_label(profile.profile)
                        } else {
                            &profile.display_name
                        },
                    );
                }
            });
        if self.import_flow.profile != previous_profile {
            self.import_flow.plan = None;
            self.import_flow.accepted_plan_request = None;
            self.import_flow.geometry_policy = ImportGeometryPolicy::Direct;
            self.import_flow.categories.clear();
            self.import_flow.descriptors = vec![descriptor_draft(self.import_flow.profile)];
            self.import_flow.invalidate_yolo_inspection();
            self.import_flow.direct_bounding_boxes = true;
            self.import_flow.direct_skeletons = profile_has_skeletons(self.import_flow.profile);
            self.import_flow.target_geometry = match self.import_flow.profile {
                ImportProfile::UltralyticsYoloPoseV1 | ImportProfile::CocoKeypointsGtV1 => {
                    ImportGeometryKind::Skeleton
                }
                _ => ImportGeometryKind::BoundingBox,
            };
        }
        ui.label(RichText::new("Transport").strong());
        let previous_transport = self.import_flow.transport;
        for transport in capabilities
            .transports
            .iter()
            .filter(|transport| transport.enabled)
        {
            ui.radio_value(
                &mut self.import_flow.transport,
                transport.transport,
                transport_label(transport.transport),
            );
        }
        if self.import_flow.transport != previous_transport {
            self.import_flow.registered_paths.clear();
            self.import_flow.descriptors = vec![descriptor_draft(self.import_flow.profile)];
            self.import_flow.invalidate_yolo_inspection();
        }
        if self.import_flow.transport == ImportTransport::ServerDirectory {
            let previous_root = self.import_flow.server_root_id.clone();
            egui::ComboBox::from_label("Server import root")
                .selected_text(
                    capabilities
                        .server_roots
                        .iter()
                        .find(|root| root.root_id == self.import_flow.server_root_id)
                        .map(|root| root.display_name.as_str())
                        .unwrap_or("Choose a root"),
                )
                .show_ui(ui, |ui| {
                    for root in &capabilities.server_roots {
                        ui.selectable_value(
                            &mut self.import_flow.server_root_id,
                            root.root_id.clone(),
                            &root.display_name,
                        );
                    }
                });
            if self.import_flow.server_root_id != previous_root {
                self.import_flow.server_relative_path.clear();
                self.import_flow.source_picker = Default::default();
            }
            status_row(
                ui,
                "Dataset folder",
                match self.import_flow.server_relative_path.as_str() {
                    "" => "Not selected".to_string(),
                    "." => "/".to_string(),
                    path => path.to_string(),
                },
            );
            if ui
                .add_enabled(
                    !self.import_flow.server_root_id.is_empty(),
                    egui::Button::new(if self.import_flow.server_relative_path.is_empty() {
                        "Choose dataset folder"
                    } else {
                        "Change dataset folder"
                    }),
                )
                .clicked()
            {
                self.open_import_source_picker(ImportSourcePickerTarget::DatasetFolder);
            }
        } else {
            theme::inline_message(
                ui,
                theme::Intent::Info,
                "The browser folder is selected after the import is registered. Reselect the same folder to resume an interrupted upload.",
            );
        }
        ui.label(RichText::new("Attestations").strong());
        ui.checkbox(
            &mut self.import_flow.ground_truth,
            "I attest that these labels are ground truth",
        );
        ui.checkbox(
            &mut self.import_flow.exhaustive,
            "I attest that labels are exhaustive for the stated coverage",
        );
        theme::labeled_text_field(
            ui,
            "Coverage scope (comma separated)",
            &mut self.import_flow.coverage_scope,
            theme::COMPACT_TEXT_FIELD_HEIGHT,
        );
        theme::labeled_text_field(
            ui,
            "Provenance",
            &mut self.import_flow.provenance,
            theme::COMPACT_TEXT_FIELD_HEIGHT,
        );
        let dataset_id = DatasetId::from(self.import_flow.destination_id.trim());
        let valid = dataset_id.validate_path_segment().is_ok()
            && !self.import_flow.destination_name.trim().is_empty()
            && self.import_flow.ground_truth
            && !self.import_flow.provenance.trim().is_empty()
            && (self.import_flow.transport == ImportTransport::BrowserFolder
                || (!self.import_flow.server_root_id.is_empty()
                    && !self.import_flow.server_relative_path.trim().is_empty()));
        if theme::primary_button(
            ui,
            valid && !self.import_flow.busy,
            egui::Button::new("Register import"),
        )
        .on_disabled_hover_text("Complete the destination, source, and required attestations.")
        .clicked()
        {
            self.request_create_import();
        }
    }

    fn import_transport_step(&mut self, ui: &mut egui::Ui) {
        if let Some(job) = &self.import_flow.job {
            status_row(ui, "Import", job.import_id.to_string());
            status_row(ui, "Status", lifecycle_label(job.lifecycle));
            if job.transport == ImportTransport::BrowserFolder {
                status_row(
                    ui,
                    "Upload progress",
                    format!(
                        "{} of {} files, {} of {} bytes",
                        job.progress.uploaded_files,
                        job.progress.total_files,
                        job.progress.accepted_bytes,
                        job.progress.total_bytes
                    ),
                );
                if ui
                    .add_enabled(
                        !self.import_flow.busy,
                        egui::Button::new(if job.progress.registered_files == 0 {
                            "Choose folder and upload"
                        } else {
                            "Reselect folder and resume"
                        }),
                    )
                    .clicked()
                {
                    self.request_import_folder_selection();
                }
            }
        }
        ui.separator();
        ui.label(RichText::new("Source configuration").strong());
        theme::labeled_text_field(
            ui,
            "Source namespace",
            &mut self.import_flow.source_namespace,
            theme::COMPACT_TEXT_FIELD_HEIGHT,
        );
        let browser_paths = self.import_flow.registered_paths.clone();
        let browser_transport = self.import_flow.transport == ImportTransport::BrowserFolder;
        let profile = self.import_flow.profile;
        let coco = is_coco_profile(profile);
        let mut open_picker = None;
        if coco {
            let descriptor_count = self.import_flow.descriptors.len();
            let mut remove = None;
            for (index, descriptor) in self.import_flow.descriptors.iter_mut().enumerate() {
                ui.push_id(("import-descriptor", index), |ui| {
                    ui.label(RichText::new(format!("Descriptor {}", index + 1)).strong());
                    egui::ComboBox::from_label("Descriptor kind")
                        .selected_text(descriptor_kind_label(descriptor.kind))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut descriptor.kind,
                                ImportDescriptorKind::CocoInstances,
                                descriptor_kind_label(ImportDescriptorKind::CocoInstances),
                            );
                            if profile == ImportProfile::CocoKeypointsGtV1 {
                                ui.selectable_value(
                                    &mut descriptor.kind,
                                    ImportDescriptorKind::CocoKeypoints,
                                    descriptor_kind_label(ImportDescriptorKind::CocoKeypoints),
                                );
                            }
                        });
                    if browser_transport {
                        source_file_selector(
                            ui,
                            "Descriptor file",
                            &mut descriptor.descriptor_file_id,
                            &browser_paths,
                            |path| descriptor_path_matches(profile, path),
                        );
                    } else if server_source_file_picker(
                        ui,
                        "Descriptor file",
                        &descriptor.descriptor_file_id,
                        &browser_paths,
                        "Choose descriptor file",
                    ) {
                        open_picker = Some(ImportSourcePickerTarget::Descriptor(index));
                    }
                    theme::labeled_text_field(
                        ui,
                        "Release",
                        &mut descriptor.release,
                        theme::COMPACT_TEXT_FIELD_HEIGHT,
                    );
                    theme::labeled_text_field(
                        ui,
                        "Split",
                        &mut descriptor.split,
                        theme::COMPACT_TEXT_FIELD_HEIGHT,
                    );
                    theme::labeled_text_field(
                        ui,
                        "Pairing group (optional)",
                        &mut descriptor.pairing_group,
                        theme::COMPACT_TEXT_FIELD_HEIGHT,
                    );
                    if browser_transport {
                        source_file_selector(
                            ui,
                            "Exact COCO image root",
                            &mut descriptor.image_root_file_id,
                            &browser_paths,
                            is_image_path,
                        );
                    } else if server_source_file_picker(
                        ui,
                        "Exact COCO image root",
                        &descriptor.image_root_file_id,
                        &browser_paths,
                        "Choose image in root",
                    ) {
                        open_picker = Some(ImportSourcePickerTarget::CocoImageRoot(index));
                    }
                    ui.small(
                        "Select a registered image directly inside the exact root referenced by COCO file_name values.",
                    );
                    if descriptor_count > 1 && ui.button("Remove descriptor").clicked() {
                        remove = Some(index);
                    }
                    ui.separator();
                });
            }
            if let Some(index) = remove {
                self.import_flow.descriptors.remove(index);
                self.import_flow.source_picker = Default::default();
            }
            if ui.button("Add COCO descriptor").clicked() {
                self.import_flow
                    .descriptors
                    .push(descriptor_draft(self.import_flow.profile));
            }
        } else {
            if self.import_flow.descriptors.len() != 1 {
                self.import_flow.descriptors = vec![descriptor_draft(profile)];
                self.import_flow.invalidate_yolo_inspection();
            }
            let mut descriptor_changed = false;
            let mut inspect_after_edit = false;
            if let Some(descriptor) = self.import_flow.descriptors.first_mut() {
                ui.label(RichText::new("YOLO source").strong());
                let previous = descriptor.descriptor_file_id.clone();
                if browser_transport {
                    inspect_after_edit = source_file_selector(
                        ui,
                        "Dataset YAML",
                        &mut descriptor.descriptor_file_id,
                        &browser_paths,
                        |path| descriptor_path_matches(profile, path),
                    );
                } else if server_source_file_picker(
                    ui,
                    "Dataset YAML",
                    &descriptor.descriptor_file_id,
                    &browser_paths,
                    "Choose descriptor file",
                ) {
                    open_picker = Some(ImportSourcePickerTarget::Descriptor(0));
                }
                descriptor_changed = previous != descriptor.descriptor_file_id;
                theme::labeled_text_field(
                    ui,
                    "Release",
                    &mut descriptor.release,
                    theme::COMPACT_TEXT_FIELD_HEIGHT,
                );
            }
            if descriptor_changed {
                self.import_flow.invalidate_yolo_inspection();
            }
            if inspect_after_edit {
                self.request_yolo_descriptor_inspection();
            }
            let descriptor_selected = self
                .import_flow
                .descriptors
                .first()
                .is_some_and(|descriptor| !descriptor.descriptor_file_id.trim().is_empty());
            let inspect_label = if self.import_flow.yolo_inspection_error.is_some() {
                "Retry split inspection"
            } else if self.import_flow.yolo_inspected_descriptor_file_id.is_some() {
                "Refresh splits"
            } else {
                "Inspect YAML splits"
            };
            if ui
                .add_enabled(
                    descriptor_selected && !self.import_flow.yolo_inspection_loading,
                    egui::Button::new(inspect_label),
                )
                .clicked()
            {
                self.request_yolo_descriptor_inspection();
            }
            ui.label(RichText::new("Splits to import").strong());
            if self.import_flow.yolo_inspection_loading {
                ui.small("Descriptor inspection is in progress.");
            }
            for split in &mut self.import_flow.yolo_splits {
                ui.add_enabled(
                    split.usable,
                    egui::Checkbox::new(&mut split.selected, &split.name),
                );
                if let Some(issue) = &split.issue {
                    ui.small(issue);
                }
            }
            if descriptor_selected
                && !self.import_flow.yolo_inspection_loading
                && self.import_flow.yolo_splits.is_empty()
                && self.import_flow.yolo_inspection_error.is_none()
            {
                ui.small("Inspect the YAML to discover its train, val, and test splits.");
            }
            ui.separator();
        }
        if let Some(target) = open_picker {
            self.open_import_source_picker(target);
        }
        let descriptor_error = self.import_descriptor_error();
        if let Some(error) = &descriptor_error {
            theme::inline_message(ui, theme::Intent::Warning, error);
        }
        if theme::primary_button(
            ui,
            !self.import_flow.busy && descriptor_error.is_none(),
            egui::Button::new("Seal source and run preflight"),
        )
        .clicked()
        {
            self.request_seal_import();
        }
        if ui
            .add_enabled(!self.import_flow.busy, egui::Button::new("Cancel import"))
            .clicked()
        {
            self.request_cancel_import();
        }
    }

    fn import_preflight_step(&mut self, ui: &mut egui::Ui) {
        let previous_report = self.import_flow.plan.is_none()
            && self.import_flow.pending_plan_request.is_none()
            && self
                .import_flow
                .job
                .as_ref()
                .is_some_and(|job| job.preflight_report.is_some());
        let report = self
            .import_flow
            .pending_plan_request
            .is_none()
            .then(|| {
                self.import_flow
                    .plan
                    .as_ref()
                    .map(|plan| &plan.report)
                    .or_else(|| {
                        self.import_flow
                            .job
                            .as_ref()
                            .and_then(|job| job.preflight_report.as_ref())
                    })
            })
            .flatten()
            .cloned();
        if let Some(report) = report {
            if previous_report {
                theme::inline_message(
                    ui,
                    theme::Intent::Warning,
                    "This is the last accepted preflight and does not include the current unsaved mappings.",
                );
            }
            ui.label(RichText::new("Preflight summary").strong());
            status_row(ui, "Images", report.source.images.to_string());
            status_row(ui, "Objects", report.source.objects.to_string());
            status_row(
                ui,
                "Output annotations",
                report.output.annotations.to_string(),
            );
            status_row(
                ui,
                "Geometry",
                format!(
                    "{} direct, {} clipped, {} skipped",
                    report.geometry.direct, report.geometry.clipped, report.geometry.skipped
                ),
            );
            status_row(
                ui,
                "Coverage",
                format!(
                    "{} complete, {} empty, {} incomplete, {} excluded",
                    report.coverage.complete,
                    report.coverage.verified_empty,
                    report.coverage.incomplete,
                    report.coverage.excluded
                ),
            );
            ui.separator();
            self.import_diagnostics_disclosure(ui, &report.diagnostics);
        } else {
            ui.small("Deterministic preflight checks are in progress.");
        }
        ui.separator();
        self.import_mapping_editor(ui);
        let mappings_complete = self.import_mappings_complete();
        let plan_covers_source = self.import_plan_covers_all_categories();
        if self.import_flow.plan.is_some() && !plan_covers_source {
            theme::inline_message(
                ui,
                theme::Intent::Error,
                "The accepted plan does not output every selected category and required workflow. Correct the exact source keys; commit remains disabled.",
            );
        }
        let commit_ready = self
            .import_flow
            .plan
            .as_ref()
            .is_some_and(|plan| plan.commit_ready)
            && self.import_plan_is_current()
            && plan_covers_source
            && mappings_complete;
        if theme::primary_button(
            ui,
            !self.import_flow.busy && mappings_complete,
            egui::Button::new("Save mappings and re-run preflight"),
        )
        .on_disabled_hover_text(
            "Represent every discovered category and complete the selected workflow before saving.",
        )
        .clicked()
        {
            self.request_update_import_plan();
        }
        if theme::primary_button(
            ui,
            !self.import_flow.busy && commit_ready,
            egui::Button::new("Commit import"),
        )
        .on_disabled_hover_text(
            "Save these exact mappings again after every edit, resolve diagnostics, and acknowledge warnings.",
        )
        .clicked()
        {
            self.request_commit_import();
        }
        if ui
            .add_enabled(!self.import_flow.busy, egui::Button::new("Cancel import"))
            .clicked()
        {
            self.request_cancel_import();
        }
    }

    fn import_diagnostics_disclosure(
        &mut self,
        ui: &mut egui::Ui,
        diagnostics: &[labello_client::ImportDiagnosticSummary],
    ) {
        let overview = ImportDiagnosticOverview::from_diagnostics(
            diagnostics,
            &self.import_flow.acknowledgements,
        );
        let compact = ui.available_width() < 480.0;
        let label = overview.disclosure_label(compact);
        let color = overview.color();

        let disclosure = egui::CollapsingHeader::new(RichText::new(label).strong().color(color))
            .id_salt("import-preflight-diagnostics")
            .default_open(true)
            .show_background(true)
            .show(ui, |ui| {
                if diagnostics.is_empty() {
                    theme::inline_message(ui, theme::Intent::Success, "No diagnostics reported.");
                }
                for diagnostic in diagnostics {
                    let intent = match diagnostic.severity {
                        ImportDiagnosticSeverity::Error => theme::Intent::Error,
                        ImportDiagnosticSeverity::WarningRequiresAck
                        | ImportDiagnosticSeverity::Warning => theme::Intent::Warning,
                        ImportDiagnosticSeverity::Info | ImportDiagnosticSeverity::Unknown => {
                            theme::Intent::Info
                        }
                    };
                    theme::inline_message(
                        ui,
                        intent,
                        format!(
                            "{} diagnostic {}: {} ({} affected)",
                            diagnostic_severity_label(diagnostic.severity),
                            diagnostic.code,
                            diagnostic.safe_summary,
                            diagnostic.count
                        ),
                    );
                    if diagnostic.impact.requires_acknowledgement {
                        let mut acknowledged =
                            self.import_flow.acknowledgements.contains(&diagnostic.code);
                        if ui
                            .checkbox(
                                &mut acknowledged,
                                format!("Acknowledge {}", diagnostic.code),
                            )
                            .changed()
                        {
                            if acknowledged {
                                self.import_flow
                                    .acknowledgements
                                    .insert(diagnostic.code.clone());
                            } else {
                                self.import_flow.acknowledgements.remove(&diagnostic.code);
                            }
                        }
                    }
                }
                if !self.import_flow.diagnostics.is_empty() {
                    ui.label(RichText::new("Diagnostic details").strong());
                    for diagnostic in &self.import_flow.diagnostics {
                        ui.label(format!(
                            "{} diagnostic {}: {}",
                            diagnostic_severity_label(diagnostic.severity),
                            diagnostic.code,
                            diagnostic.safe_summary
                        ));
                    }
                }
                if self.import_flow.diagnostics_cursor.is_some()
                    && ui
                        .add_enabled(
                            !self.import_flow.busy,
                            egui::Button::new("Load more diagnostics"),
                        )
                        .clicked()
                {
                    self.request_import_diagnostics(false);
                }
            });
        let expanded =
            egui::collapsing_header::CollapsingState::load(ui.ctx(), disclosure.header_response.id)
                .is_some_and(|state| state.is_open());
        ui.ctx()
            .accesskit_node_builder(disclosure.header_response.id, |node| {
                node.set_expanded(expanded);
            });
    }

    fn import_mapping_editor(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Category and task mapping").strong());
        let discovered = self
            .import_flow
            .plan
            .as_ref()
            .map(|plan| plan.report.source.categories)
            .or_else(|| {
                self.import_flow
                    .job
                    .as_ref()
                    .and_then(|job| job.preflight_report.as_ref())
                    .map(|report| report.source.categories)
            })
            .unwrap_or(0);
        if discovered > 0 && self.import_flow.categories.len() != discovered as usize {
            theme::inline_message(
                ui,
                theme::Intent::Error,
                "This API contract reports only a category count, not the discovered category keys, IDs, names, or skeleton schemas required for a valid plan. Mapping and commit are disabled; Labello will not guess sparse source IDs.",
            );
            if ui
                .add_enabled(
                    !self.import_flow.busy,
                    egui::Button::new("Restart import setup"),
                )
                .clicked()
            {
                self.restart_import_setup();
            }
            return;
        }
        ui.label(format!(
            "{} mapping rows for {discovered} discovered categories",
            self.import_flow.categories.len()
        ));
        for (index, category) in self.import_flow.categories.iter_mut().enumerate() {
            ui.push_id(("import-category", index), |ui| {
                ui.label(RichText::new(format!("Category {}", index + 1)).strong());
                ui.checkbox(&mut category.selected, "Include this source category");
                status_row(ui, "Source category key", &category.source_category_key);
                status_row(ui, "Source category ID", &category.source_category_id);
                status_row(ui, "Source category name", &category.source_name);
                status_row(
                    ui,
                    "Direct geometry",
                    category
                        .direct_geometry
                        .iter()
                        .map(|kind| match kind {
                            ImportGeometryKind::BoundingBox => "bounding box",
                            ImportGeometryKind::Skeleton => "skeleton",
                        })
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                for (label, value) in [
                    ("Class ID", &mut category.class_id),
                    ("Class name", &mut category.class_name),
                    ("Class color", &mut category.class_color),
                ] {
                    theme::labeled_text_field(ui, label, value, theme::COMPACT_TEXT_FIELD_HEIGHT);
                }
                let category_specific = !category.geometry_mappings.is_empty();
                let active_target = |target| {
                    category.geometry_mappings.iter().any(|mapping| {
                        mapping.target_geometry == target
                            && mapping.policy != ImportGeometryPolicy::Omit
                    })
                };
                let bounding_box_task = if category_specific {
                    active_target(ImportGeometryKind::BoundingBox)
                } else {
                    (self.import_flow.geometry_policy == ImportGeometryPolicy::Direct
                        && self.import_flow.direct_bounding_boxes)
                        || self.import_flow.geometry_policy
                            == ImportGeometryPolicy::ManualBoxGuideV1
                };
                let skeleton_task = if category_specific {
                    active_target(ImportGeometryKind::Skeleton)
                } else {
                    (self.import_flow.geometry_policy == ImportGeometryPolicy::Direct
                        && self.import_flow.direct_skeletons)
                        || self.import_flow.geometry_policy
                            == ImportGeometryPolicy::ManualBoxGuideV1
                };
                if bounding_box_task {
                    theme::labeled_text_field(
                        ui,
                        "Bounding-box task ID",
                        &mut category.bounding_box_task_id,
                        theme::COMPACT_TEXT_FIELD_HEIGHT,
                    );
                    theme::labeled_text_field(
                        ui,
                        "Bounding-box task name",
                        &mut category.bounding_box_task_name,
                        theme::COMPACT_TEXT_FIELD_HEIGHT,
                    );
                }
                if skeleton_task {
                    theme::labeled_text_field(
                        ui,
                        "Skeleton task ID",
                        &mut category.skeleton_task_id,
                        theme::COMPACT_TEXT_FIELD_HEIGHT,
                    );
                    theme::labeled_text_field(
                        ui,
                        "Skeleton task name",
                        &mut category.skeleton_task_name,
                        theme::COMPACT_TEXT_FIELD_HEIGHT,
                    );
                }
                if !category.geometry_mappings.is_empty() {
                    ui.label(RichText::new("Category geometry outputs").strong());
                    let direct_geometry = category.direct_geometry.clone();
                    let source_skeleton = category.source_skeleton.clone();
                    let target_keypoint_names = category.target_keypoint_names.clone();
                    for (mapping_index, mapping) in
                        category.geometry_mappings.iter_mut().enumerate()
                    {
                        let target = match mapping.target_geometry {
                            ImportGeometryKind::BoundingBox => "bounding box",
                            ImportGeometryKind::Skeleton => "skeleton",
                        };
                        egui::ComboBox::from_label(format!("{target} source geometry"))
                            .selected_text(match mapping.source_geometry {
                                ImportGeometryKind::BoundingBox => "Bounding box",
                                ImportGeometryKind::Skeleton => "Skeleton",
                            })
                            .show_ui(ui, |ui| {
                                for source in &direct_geometry {
                                    ui.selectable_value(
                                        &mut mapping.source_geometry,
                                        *source,
                                        match source {
                                            ImportGeometryKind::BoundingBox => "Bounding box",
                                            ImportGeometryKind::Skeleton => "Skeleton",
                                        },
                                    );
                                }
                            });
                        egui::ComboBox::from_label(format!("{target} policy"))
                            .selected_text(policy_label(mapping.policy))
                            .show_ui(ui, |ui| {
                                for policy in policies_for_mapping(
                                    mapping.source_geometry,
                                    mapping.target_geometry,
                                    &direct_geometry,
                                    self.import_flow.capabilities.as_ref().is_some_and(
                                        |capabilities| capabilities.manual_box_guide_migration,
                                    ),
                                ) {
                                    ui.selectable_value(
                                        &mut mapping.policy,
                                        policy,
                                        policy_label(policy),
                                    );
                                }
                            });
                        ui.push_id(("mapping-parameters", mapping_index), |ui| {
                            mapping_parameter_editor(
                                ui,
                                mapping,
                                source_skeleton.as_ref(),
                                &target_keypoint_names,
                            );
                        });
                    }
                    if category
                        .direct_geometry
                        .contains(&ImportGeometryKind::BoundingBox)
                        && !category
                            .geometry_mappings
                            .iter()
                            .any(|mapping| mapping.target_geometry == ImportGeometryKind::Skeleton)
                        && ui.button("Add skeleton output").clicked()
                    {
                        category
                            .geometry_mappings
                            .push(ImportGeometryMappingRequest {
                                source_category_key: category.source_category_key.clone(),
                                source_geometry: ImportGeometryKind::BoundingBox,
                                target_geometry: ImportGeometryKind::Skeleton,
                                policy: ImportGeometryPolicy::BoxRelativeTemplateV1,
                                parameters: Vec::new(),
                            });
                    }
                    if category.geometry_mappings.iter().any(|mapping| {
                        mapping.target_geometry == ImportGeometryKind::Skeleton
                            && mapping.source_geometry == ImportGeometryKind::BoundingBox
                    }) {
                        theme::labeled_text_field(
                            ui,
                            "Target keypoint names (comma separated)",
                            &mut category.target_keypoint_names,
                            theme::COMPACT_TEXT_FIELD_HEIGHT,
                        );
                    }
                    egui::ComboBox::from_label("Category workflow intent")
                        .selected_text(intent_label(category.workflow_intent))
                        .show_ui(ui, |ui| {
                            for intent in [
                                ImportWorkflowIntent::AuthoritativeGroundTruth,
                                ImportWorkflowIntent::RequireApproval,
                                ImportWorkflowIntent::SeedFutureAnnotation,
                            ] {
                                ui.selectable_value(
                                    &mut category.workflow_intent,
                                    intent,
                                    intent_label(intent),
                                );
                            }
                        });
                }
                ui.separator();
            });
        }
        let category_specific = self
            .import_flow
            .categories
            .iter()
            .any(|category| !category.geometry_mappings.is_empty());
        if category_specific
            && self.import_flow.categories.iter().any(|category| {
                category.selected
                    && category.workflow_intent == ImportWorkflowIntent::SeedFutureAnnotation
            })
        {
            theme::inline_message(
                ui,
                theme::Intent::Warning,
                "Seed workflow keeps imported geometry pending for future human annotation instead of completing it as ground truth.",
            );
            ui.checkbox(
                &mut self.import_flow.seed_workflow_confirmed,
                "Create the selected pending seed workflows",
            );
        }
        if !category_specific {
            egui::ComboBox::from_label("Geometry policy")
                .selected_text(policy_label(self.import_flow.geometry_policy))
                .show_ui(ui, |ui| {
                    let mut policies =
                        vec![ImportGeometryPolicy::Direct, ImportGeometryPolicy::Omit];
                    if self
                        .import_flow
                        .capabilities
                        .as_ref()
                        .is_some_and(|capabilities| capabilities.manual_box_guide_migration)
                    {
                        policies.insert(1, ImportGeometryPolicy::ManualBoxGuideV1);
                    }
                    for policy in policies {
                        ui.selectable_value(
                            &mut self.import_flow.geometry_policy,
                            policy,
                            policy_label(policy),
                        );
                    }
                });
            if self.import_flow.geometry_policy == ImportGeometryPolicy::Direct {
                ui.checkbox(
                    &mut self.import_flow.direct_bounding_boxes,
                    "Import direct bounding boxes",
                );
                if profile_has_skeletons(self.import_flow.profile) {
                    ui.checkbox(
                        &mut self.import_flow.direct_skeletons,
                        "Import direct skeletons",
                    );
                } else {
                    self.import_flow.direct_skeletons = false;
                }
                self.import_flow.target_geometry = if self.import_flow.direct_skeletons {
                    ImportGeometryKind::Skeleton
                } else {
                    ImportGeometryKind::BoundingBox
                };
            } else if self.import_flow.geometry_policy == ImportGeometryPolicy::ManualBoxGuideV1 {
                self.import_flow.target_geometry = ImportGeometryKind::Skeleton;
                theme::inline_message(
                    ui,
                    theme::Intent::Info,
                    "Manual migration creates a separate read-only bounding-box guide and skeleton target for each selected source category.",
                );
            }
            egui::ComboBox::from_label("Workflow intent")
                .selected_text(intent_label(self.import_flow.workflow_intent))
                .show_ui(ui, |ui| {
                    for intent in [
                        ImportWorkflowIntent::AuthoritativeGroundTruth,
                        ImportWorkflowIntent::RequireApproval,
                        ImportWorkflowIntent::SeedFutureAnnotation,
                    ] {
                        ui.selectable_value(
                            &mut self.import_flow.workflow_intent,
                            intent,
                            intent_label(intent),
                        );
                    }
                });
            if self.import_flow.workflow_intent == ImportWorkflowIntent::SeedFutureAnnotation {
                theme::inline_message(
                    ui,
                    theme::Intent::Warning,
                    "Seed workflow keeps imported geometry pending for future human annotation instead of completing it as ground truth.",
                );
                ui.checkbox(
                    &mut self.import_flow.seed_workflow_confirmed,
                    "Create pending seed workflows for every mapped category",
                );
            } else {
                self.import_flow.seed_workflow_confirmed = false;
            }
            if self.import_flow.geometry_policy == ImportGeometryPolicy::ManualBoxGuideV1 {
                theme::labeled_text_field(
                    ui,
                    "Manual target keypoint names (comma separated)",
                    &mut self.import_flow.keypoint_names,
                    theme::COMPACT_TEXT_FIELD_HEIGHT,
                );
            }
        }
        ui.separator();
        ui.label(RichText::new("Compatibility policies").strong());
        egui::ComboBox::from_label("YOLO missing labels")
            .selected_text(format!("{:?}", self.import_flow.yolo_missing_labels))
            .show_ui(ui, |ui| {
                for policy in [
                    labello_client::YoloMissingLabelPolicy::Block,
                    labello_client::YoloMissingLabelPolicy::Incomplete,
                    labello_client::YoloMissingLabelPolicy::MissingIsBackground,
                ] {
                    ui.selectable_value(
                        &mut self.import_flow.yolo_missing_labels,
                        policy,
                        format!("{policy:?}"),
                    );
                }
            });
        egui::ComboBox::from_label("YOLO duplicate rows")
            .selected_text(format!("{:?}", self.import_flow.yolo_duplicate_rows))
            .show_ui(ui, |ui| {
                for policy in [
                    labello_client::YoloDuplicateRowPolicy::Block,
                    labello_client::YoloDuplicateRowPolicy::Deduplicate,
                ] {
                    ui.selectable_value(
                        &mut self.import_flow.yolo_duplicate_rows,
                        policy,
                        format!("{policy:?}"),
                    );
                }
            });
        egui::ComboBox::from_label("COCO crowd objects")
            .selected_text(format!("{:?}", self.import_flow.coco_crowds))
            .show_ui(ui, |ui| {
                for policy in [
                    labello_client::CocoCrowdPolicy::Block,
                    labello_client::CocoCrowdPolicy::Incomplete,
                    labello_client::CocoCrowdPolicy::ExcludeImageTask,
                ] {
                    ui.selectable_value(
                        &mut self.import_flow.coco_crowds,
                        policy,
                        format!("{policy:?}"),
                    );
                }
            });
        egui::ComboBox::from_label("COCO structure")
            .selected_text(format!("{:?}", self.import_flow.coco_structure))
            .show_ui(ui, |ui| {
                for policy in [
                    labello_client::CocoStructurePolicy::Canonical,
                    labello_client::CocoStructurePolicy::BboxCompatibility,
                ] {
                    ui.selectable_value(
                        &mut self.import_flow.coco_structure,
                        policy,
                        format!("{policy:?}"),
                    );
                }
            });
        egui::ComboBox::from_label("Out-of-bounds geometry")
            .selected_text(format!("{:?}", self.import_flow.geometry_bounds))
            .show_ui(ui, |ui| {
                for policy in [
                    labello_client::GeometryBoundsPolicy::Reject,
                    labello_client::GeometryBoundsPolicy::Clip,
                ] {
                    ui.selectable_value(
                        &mut self.import_flow.geometry_bounds,
                        policy,
                        format!("{policy:?}"),
                    );
                }
            });
        egui::ComboBox::from_label("Cross-split duplicates")
            .selected_text(format!("{:?}", self.import_flow.cross_split_duplicates))
            .show_ui(ui, |ui| {
                for policy in [
                    labello_client::CrossSplitDuplicatePolicy::Block,
                    labello_client::CrossSplitDuplicatePolicy::MergeMemberships,
                ] {
                    ui.selectable_value(
                        &mut self.import_flow.cross_split_duplicates,
                        policy,
                        format!("{policy:?}"),
                    );
                }
            });
        egui::ComboBox::from_label("Missing keypoint names")
            .selected_text(format!("{:?}", self.import_flow.missing_keypoint_names))
            .show_ui(ui, |ui| {
                for policy in [
                    labello_client::MissingKeypointNamesPolicy::Block,
                    labello_client::MissingKeypointNamesPolicy::GenerateIndexed,
                ] {
                    ui.selectable_value(
                        &mut self.import_flow.missing_keypoint_names,
                        policy,
                        format!("{policy:?}"),
                    );
                }
            });
    }

    fn import_running_step(&mut self, ui: &mut egui::Ui) {
        if let Some(job) = &self.import_flow.job {
            status_row(ui, "Status", lifecycle_label(job.lifecycle));
            status_row(
                ui,
                "Images",
                format!(
                    "{} of {}",
                    job.progress.processed_images, job.progress.total_images
                ),
            );
            status_row(
                ui,
                "Objects",
                format!(
                    "{} of {}",
                    job.progress.processed_objects, job.progress.total_objects
                ),
            );
            if job.can_cancel
                && ui
                    .add_enabled(!self.import_flow.busy, egui::Button::new("Cancel import"))
                    .clicked()
            {
                self.request_cancel_import();
            }
        }
    }

    fn import_failure_step(&mut self, ui: &mut egui::Ui) {
        if let Some(failure) = self
            .import_flow
            .job
            .as_ref()
            .and_then(|job| job.failure.as_ref())
        {
            status_row(ui, "Failure code", failure.code.clone());
            theme::inline_message(ui, theme::Intent::Error, &failure.safe_summary);
            status_row(
                ui,
                "Retry",
                if failure.retryable {
                    "Available"
                } else {
                    "Not available"
                },
            );
            if failure.retryable
                && theme::primary_button(
                    ui,
                    !self.import_flow.busy,
                    egui::Button::new("Retry import"),
                )
                .clicked()
            {
                self.request_retry_import();
            }
        }
        if ui
            .add_enabled(!self.import_flow.busy, egui::Button::new("Cancel import"))
            .clicked()
        {
            self.request_cancel_import();
        }
        if ui.button("Start another import").clicked() {
            self.begin_import_epoch();
            self.import_flow.reset_job();
        }
    }

    fn import_success_step(&mut self, ui: &mut egui::Ui) {
        theme::inline_message(
            ui,
            theme::Intent::Success,
            "Import committed and verified successfully.",
        );
        if let Some(job) = &self.import_flow.job {
            status_row(ui, "Dataset ID", job.destination_dataset_id.to_string());
            status_row(ui, "Plan hash", job.plan_hash.clone().unwrap_or_default());
        }
        let dataset_ready = self.import_flow.job.as_ref().is_some_and(|job| {
            self.datasets
                .summaries
                .iter()
                .any(|dataset| dataset.dataset_id == job.destination_dataset_id)
        }) && !self.loading.datasets;
        if self.loading.datasets {
            ui.label("Refreshing the dataset catalog before navigation...");
        }
        if theme::primary_button(
            ui,
            dataset_ready,
            egui::Button::new("Open imported dataset Admin"),
        )
        .on_disabled_hover_text("Wait for the imported dataset to appear in the refreshed catalog.")
        .clicked()
            && let Some(job) = &self.import_flow.job
        {
            let dataset_id = job.destination_dataset_id.clone();
            self.import_flow.open = false;
            self.open_dataset(dataset_id, AppView::Admin);
        }
        if ui.button("Import another dataset").clicked() {
            self.begin_import_epoch();
            self.import_flow.reset_job();
        }
    }

    pub(crate) fn request_create_import(&mut self) {
        let Some(capabilities) = self.import_flow.capabilities.as_ref() else {
            return;
        };
        if !capabilities
            .profiles
            .iter()
            .any(|entry| entry.enabled && entry.profile == self.import_flow.profile)
            || !capabilities
                .transports
                .iter()
                .any(|entry| entry.enabled && entry.transport == self.import_flow.transport)
        {
            self.import_flow.error = Some(
                "The selected profile or transport is not advertised by the server.".to_string(),
            );
            return;
        }
        self.begin_import_epoch();
        self.import_flow.busy = true;
        self.import_flow.error = None;
        let source = match self.import_flow.transport {
            ImportTransport::BrowserFolder => ImportSourceSelection::BrowserFolder,
            ImportTransport::ServerDirectory => ImportSourceSelection::ServerDirectory {
                import_root_id: self.import_flow.server_root_id.clone(),
                relative_path: self.import_flow.server_relative_path.trim().to_string(),
            },
            ImportTransport::Unknown => return,
        };
        let request = self.import_request_identity(None);
        let key = import_key("create", request.request_id);
        self.queue_command(UiCommand::CreateImport {
            request,
            body: CreateImportRequest {
                destination_dataset_id: DatasetId::from(self.import_flow.destination_id.trim()),
                destination_name: self.import_flow.destination_name.trim().to_string(),
                profile: self.import_flow.profile,
                source,
                attestations: self.import_attestations(),
            },
            idempotency_key: key,
        });
    }

    pub(crate) fn request_seal_import(&mut self) {
        let Some(import_id) = self
            .import_flow
            .job
            .as_ref()
            .map(|job| job.import_id.clone())
        else {
            return;
        };
        if let Some(error) = self.import_descriptor_error() {
            self.import_flow.error = Some(error);
            return;
        }
        self.import_flow.busy = true;
        self.import_flow.error = None;
        let request = self.import_request_identity(Some(import_id.clone()));
        let key = import_key("seal", request.request_id);
        let yolo = !is_coco_profile(self.import_flow.profile);
        let selected_splits = if yolo {
            self.import_flow
                .yolo_splits
                .iter()
                .filter(|split| split.usable && split.selected)
                .map(|split| split.name.clone())
                .collect::<Vec<_>>()
        } else {
            self.import_flow
                .descriptors
                .iter()
                .map(|descriptor| descriptor.split.trim().to_string())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect()
        };
        let yolo_descriptor_split = selected_splits.first().cloned().unwrap_or_default();
        let descriptors = self
            .import_flow
            .descriptors
            .iter()
            .map(|descriptor| ImportDescriptorSelection {
                descriptor_file_id: descriptor.descriptor_file_id.trim().to_string(),
                kind: descriptor.kind,
                release: descriptor.release.trim().to_string(),
                split: if yolo {
                    yolo_descriptor_split.clone()
                } else {
                    descriptor.split.trim().to_string()
                },
                image_root_file_id: (!descriptor.image_root_file_id.trim().is_empty())
                    .then(|| descriptor.image_root_file_id.trim().to_string()),
                pairing_group: (!descriptor.pairing_group.trim().is_empty())
                    .then(|| descriptor.pairing_group.trim().to_string()),
            })
            .collect::<Vec<_>>();
        self.queue_command(UiCommand::SealImport {
            request,
            import_id,
            body: SealImportRequest {
                source: ImportSourceConfiguration {
                    source_namespace: self.import_flow.source_namespace.trim().to_string(),
                    descriptors,
                    selected_splits,
                    selected_category_keys: Vec::new(),
                },
                attestations: self.import_attestations(),
            },
            idempotency_key: key,
        });
    }

    pub(crate) fn request_yolo_descriptor_inspection(&mut self) {
        let Some(import_id) = self
            .import_flow
            .job
            .as_ref()
            .map(|job| job.import_id.clone())
        else {
            return;
        };
        let Some(descriptor_file_id) = self
            .import_flow
            .descriptors
            .first()
            .map(|descriptor| descriptor.descriptor_file_id.trim().to_string())
            .filter(|reference| !reference.is_empty())
        else {
            return;
        };
        if self.import_flow.yolo_inspection_loading {
            return;
        }
        self.import_flow.invalidate_yolo_inspection();
        self.import_flow.yolo_inspection_loading = true;
        let request = self.import_request_identity(Some(import_id.clone()));
        self.import_flow.pending_yolo_inspection_request_id = Some(request.request_id);
        self.queue_command(UiCommand::InspectYoloDescriptor {
            request,
            import_id,
            descriptor_file_id: descriptor_file_id.clone(),
            body: labello_client::InspectYoloDescriptorRequest { descriptor_file_id },
        });
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn request_yolo_descriptor_inspection_after_upload(&mut self) {
        if self.import_flow.yolo_inspection_loading {
            self.import_flow.yolo_inspection_retry_after_current = true;
        } else {
            self.request_yolo_descriptor_inspection();
        }
    }

    pub(crate) fn open_import_source_picker(&mut self, target: ImportSourcePickerTarget) {
        self.import_flow.source_picker = ImportSourcePickerState {
            target: Some(target),
            ..Default::default()
        };
        let initial_path = match target {
            ImportSourcePickerTarget::DatasetFolder => String::new(),
            ImportSourcePickerTarget::Descriptor(_)
            | ImportSourcePickerTarget::CocoImageRoot(_) => {
                match self.import_flow.server_relative_path.as_str() {
                    "" | "." => String::new(),
                    path => path.to_string(),
                }
            }
        };
        self.request_import_source_browse(initial_path, 0);
    }

    fn request_import_source_browse(&mut self, relative_path: String, offset: u32) {
        let Some(target) = self.import_flow.source_picker.target else {
            return;
        };
        if self.import_flow.source_picker.loading {
            return;
        }
        let request = match target {
            ImportSourcePickerTarget::DatasetFolder => self.import_request_identity(None),
            ImportSourcePickerTarget::Descriptor(_)
            | ImportSourcePickerTarget::CocoImageRoot(_) => {
                let import_id = self
                    .import_flow
                    .job
                    .as_ref()
                    .map(|job| job.import_id.clone());
                self.import_request_identity(import_id)
            }
        };
        self.import_flow.source_picker.loading = true;
        self.import_flow.source_picker.error = None;
        self.import_flow.source_picker.pending_request_id = Some(request.request_id);
        self.import_flow.source_picker.pending_append = offset > 0;
        if offset == 0 {
            self.import_flow.source_picker.relative_path = relative_path.clone();
        }
        match target {
            ImportSourcePickerTarget::DatasetFolder => {
                let root_id = self.import_flow.server_root_id.clone();
                if root_id.is_empty() {
                    self.import_flow.source_picker.loading = false;
                    self.import_flow.source_picker.error =
                        Some("Choose a server import root first.".to_string());
                    return;
                }
                self.queue_command(UiCommand::BrowseImportRoot {
                    request,
                    root_id,
                    body: labello_client::BrowseServerImportRootRequest {
                        relative_path,
                        offset,
                    },
                });
            }
            ImportSourcePickerTarget::Descriptor(_)
            | ImportSourcePickerTarget::CocoImageRoot(_) => {
                let Some(import_id) = self
                    .import_flow
                    .job
                    .as_ref()
                    .map(|job| job.import_id.clone())
                else {
                    self.import_flow.source_picker.loading = false;
                    return;
                };
                let mode = match target {
                    ImportSourcePickerTarget::Descriptor(_) => {
                        labello_client::ImportSourceBrowseMode::Descriptors
                    }
                    ImportSourcePickerTarget::CocoImageRoot(_) => {
                        labello_client::ImportSourceBrowseMode::Images
                    }
                    ImportSourcePickerTarget::DatasetFolder => unreachable!(),
                };
                self.queue_command(UiCommand::BrowseImportSource {
                    request,
                    import_id,
                    body: labello_client::BrowseImportSourceRequest {
                        relative_path,
                        offset,
                        mode,
                    },
                });
            }
        }
    }

    fn import_source_picker_modal(&mut self, ctx: &egui::Context) {
        let Some(target) = self.import_flow.source_picker.target else {
            return;
        };
        let screen = ctx.content_rect();
        let width = (screen.width() - 32.0).clamp(1.0, 680.0);
        let max_height = (screen.height() - 32.0).max(1.0);
        let page = self.import_flow.source_picker.page.clone();
        let requested_path = self.import_flow.source_picker.relative_path.clone();
        let loading = self.import_flow.source_picker.loading;
        let error = self.import_flow.source_picker.error.clone();
        let mut navigate = None;
        let mut select_folder = false;
        let mut selected_folder = None;
        let mut selected_file = None;
        let mut load_more = None;
        let mut close = false;
        let response = theme::modal(ctx, egui::Id::new("import-source-picker")).show(ctx, |ui| {
            ui.set_width(width);
            ui.set_max_height(max_height);
            ui.heading(match target {
                ImportSourcePickerTarget::DatasetFolder => "Choose dataset folder",
                ImportSourcePickerTarget::Descriptor(_) => "Choose descriptor file",
                ImportSourcePickerTarget::CocoImageRoot(_) => "Choose an image in the COCO root",
            });
            let current = page
                .as_ref()
                .map(|page| page.relative_path.as_str())
                .unwrap_or(&requested_path);
            ui.label(format!(
                "Current folder: {}",
                if current.is_empty() { "/" } else { current }
            ));
            ui.horizontal_wrapped(|ui| {
                if !current.is_empty() && ui.button("Up one folder").clicked() {
                    navigate = Some(parent_source_directory(current));
                }
                if target == ImportSourcePickerTarget::DatasetFolder
                    && theme::primary_button(ui, true, egui::Button::new("Select this folder"))
                        .clicked()
                {
                    select_folder = true;
                }
            });
            if let Some(error) = &error {
                theme::inline_message(ui, theme::Intent::Warning, error);
                if ui.button("Retry").clicked() {
                    navigate = Some(current.to_string());
                }
            }
            if loading && page.is_none() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Loading server source...");
                });
            }
            let entries = page
                .as_ref()
                .map(|page| page.entries.as_slice())
                .unwrap_or(&[]);
            egui::ScrollArea::vertical()
                .id_salt("import-source-picker-entries")
                .max_height((max_height - 180.0).max(1.0))
                .show(ui, |ui| {
                    if entries.is_empty() && !loading && error.is_none() {
                        ui.label("This folder has no matching entries.");
                    }
                    for entry in entries {
                        match entry.kind {
                            labello_client::ImportBrowseEntryKind::Directory => {
                                if target == ImportSourcePickerTarget::DatasetFolder {
                                    ui.horizontal_wrapped(|ui| {
                                        if ui
                                            .button(format!("Open folder {}", entry.name))
                                            .on_hover_text(&entry.relative_path)
                                            .clicked()
                                        {
                                            navigate = Some(entry.relative_path.clone());
                                        }
                                        if ui
                                            .button(format!("Select folder {}", entry.name))
                                            .on_hover_text(&entry.relative_path)
                                            .clicked()
                                        {
                                            selected_folder = Some(entry.relative_path.clone());
                                        }
                                    });
                                } else if ui
                                    .add_sized(
                                        [ui.available_width(), 44.0],
                                        egui::Button::new(format!("Open folder {}", entry.name)),
                                    )
                                    .on_hover_text(&entry.relative_path)
                                    .clicked()
                                {
                                    navigate = Some(entry.relative_path.clone());
                                }
                            }
                            labello_client::ImportBrowseEntryKind::File => {
                                if ui
                                    .add_sized(
                                        [ui.available_width(), 44.0],
                                        egui::Button::new(format!("Select {}", entry.name)),
                                    )
                                    .on_hover_text(&entry.relative_path)
                                    .clicked()
                                {
                                    selected_file = Some(entry.clone());
                                }
                            }
                        }
                    }
                });
            if let Some(offset) = page.as_ref().and_then(|page| page.next_offset)
                && ui
                    .add_enabled(!loading, egui::Button::new("Load more"))
                    .clicked()
            {
                load_more = Some((current.to_string(), offset));
            }
            if ui.button("Close picker").clicked() {
                close = true;
            }
        });
        response.response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Window, true, "Server source picker")
        });
        if select_folder {
            let selected = page
                .as_ref()
                .map(|page| page.relative_path.clone())
                .unwrap_or(requested_path);
            self.import_flow.server_relative_path = if selected.is_empty() {
                ".".to_string()
            } else {
                selected
            };
            close = true;
        }
        if let Some(selected) = selected_folder {
            self.import_flow.server_relative_path = selected;
            close = true;
        }
        if let Some(entry) = selected_file
            && let Some(file_id) = entry.file_id
        {
            let selected_path = RegisteredImportPath {
                client_file_id: String::new(),
                file_id: file_id.clone(),
                relative_path: entry.relative_path,
            };
            if let Some(existing) = self
                .import_flow
                .registered_paths
                .iter_mut()
                .find(|path| path.file_id == file_id)
            {
                *existing = selected_path;
            } else {
                self.import_flow.registered_paths.push(selected_path);
            }
            match target {
                ImportSourcePickerTarget::DatasetFolder => {}
                ImportSourcePickerTarget::Descriptor(index) => {
                    if let Some(descriptor) = self.import_flow.descriptors.get_mut(index) {
                        descriptor.descriptor_file_id = file_id;
                    }
                    if !is_coco_profile(self.import_flow.profile) {
                        self.import_flow.invalidate_yolo_inspection();
                        self.request_yolo_descriptor_inspection();
                    }
                }
                ImportSourcePickerTarget::CocoImageRoot(index) => {
                    if let Some(descriptor) = self.import_flow.descriptors.get_mut(index) {
                        descriptor.image_root_file_id = file_id;
                    }
                }
            }
            close = true;
        }
        if let Some(path) = navigate {
            self.import_flow.source_picker.page = None;
            self.request_import_source_browse(path, 0);
        } else if let Some((path, offset)) = load_more {
            self.request_import_source_browse(path, offset);
        }
        if close || response.should_close() {
            self.import_flow.source_picker = Default::default();
        }
    }

    pub(crate) fn request_preflight_import(&mut self, restart: bool) {
        let Some(import_id) = self
            .import_flow
            .job
            .as_ref()
            .map(|job| job.import_id.clone())
        else {
            return;
        };
        self.import_flow.busy = true;
        self.import_flow.screen = ImportScreen::Preflight;
        let request = self.import_request_identity(Some(import_id.clone()));
        let key = import_key("preflight", request.request_id);
        self.queue_command(UiCommand::PreflightImport {
            request,
            import_id,
            body: StartImportPreflightRequest { restart },
            idempotency_key: key,
        });
    }

    pub(crate) fn request_update_import_plan(&mut self) {
        let Some(import_id) = self
            .import_flow
            .job
            .as_ref()
            .map(|job| job.import_id.clone())
        else {
            return;
        };
        if !self.import_mappings_complete() {
            self.import_flow.error = Some(
                "Every discovered category needs one complete, uniquely keyed mapping.".to_string(),
            );
            return;
        }
        let body = self.import_plan_request();
        self.import_flow.busy = true;
        self.import_flow.plan = None;
        self.import_flow.accepted_plan_request = None;
        self.import_flow.diagnostics.clear();
        self.import_flow.diagnostics_cursor = None;
        self.import_flow.pending_plan_request = Some(body.clone());
        let request = self.import_request_identity(Some(import_id.clone()));
        let key = import_key("plan", request.request_id);
        self.queue_command(UiCommand::UpdateImportPlan {
            request,
            import_id,
            body,
            idempotency_key: key,
        });
    }

    pub(crate) fn request_commit_import(&mut self) {
        if !self.import_plan_is_current() || !self.import_plan_covers_all_categories() {
            self.import_flow.error = Some(
                "Mappings changed or the accepted plan omits discovered categories/tasks. Save exact source mappings and wait for a complete matching plan before committing."
                    .to_string(),
            );
            return;
        }
        let Some((import_id, plan_hash)) = self
            .import_flow
            .plan
            .as_ref()
            .map(|plan| (plan.import_id.clone(), plan.plan_hash.clone()))
        else {
            return;
        };
        self.import_flow.busy = true;
        self.import_flow.screen = ImportScreen::Running;
        if let Some(job) = self.import_flow.job.as_mut() {
            job.lifecycle = ImportLifecycle::Building;
            job.progress.phase = labello_client::ImportProgressPhase::Build;
        }
        self.import_flow.poll_after =
            Some(web_time::Instant::now() + web_time::Duration::from_millis(500));
        let request = self.import_request_identity(Some(import_id.clone()));
        let key = import_key("commit", request.request_id);
        self.queue_command(UiCommand::CommitImport {
            request,
            import_id,
            body: CommitImportRequest { plan_hash },
            idempotency_key: key,
        });
    }

    pub(crate) fn request_cancel_import(&mut self) {
        let Some(import_id) = self
            .import_flow
            .job
            .as_ref()
            .map(|job| job.import_id.clone())
        else {
            self.begin_import_epoch();
            self.import_flow.reset_job();
            return;
        };
        self.import_flow.busy = true;
        let request = self.import_request_identity(Some(import_id.clone()));
        let key = import_key("cancel", request.request_id);
        self.queue_command(UiCommand::CancelImport {
            request,
            import_id,
            idempotency_key: key,
        });
    }

    pub(crate) fn request_import_poll(&mut self) {
        let Some(import_id) = self
            .import_flow
            .job
            .as_ref()
            .map(|job| job.import_id.clone())
        else {
            return;
        };
        self.import_flow.poll_after = None;
        let request = self.import_request_identity(Some(import_id.clone()));
        self.queue_command(UiCommand::GetImport { request, import_id });
    }

    pub(crate) fn request_import_diagnostics(&mut self, restart: bool) {
        let Some(import_id) = self
            .import_flow
            .job
            .as_ref()
            .map(|job| job.import_id.clone())
        else {
            return;
        };
        if restart {
            self.import_flow.diagnostics.clear();
            self.import_flow.diagnostics_cursor = None;
        }
        self.import_flow.busy = true;
        let request = self.import_request_identity(Some(import_id.clone()));
        self.queue_command(UiCommand::ImportDiagnostics {
            request,
            import_id,
            query: labello_client::ImportDiagnosticsQuery {
                cursor: self.import_flow.diagnostics_cursor.clone(),
                limit: self
                    .import_flow
                    .capabilities
                    .as_ref()
                    .map(|capabilities| capabilities.limits.max_diagnostic_page_size.min(100))
                    .unwrap_or(100),
                code: None,
                severity: None,
            },
        });
    }

    pub(crate) fn request_import_recovery(&mut self) {
        self.begin_import_epoch();
        let recovery_import_id = self.import_flow.recovery_import_id.trim().to_string();
        self.import_flow.reset_job();
        self.import_flow.recovery_import_id = recovery_import_id.clone();
        let import_id = labello_domain::ImportId::from(recovery_import_id);
        self.import_flow.busy = true;
        let request = self.import_request_identity(Some(import_id.clone()));
        self.queue_command(UiCommand::GetImport { request, import_id });
    }

    fn request_retry_import(&mut self) {
        let phase = self
            .import_flow
            .job
            .as_ref()
            .and_then(|job| job.failure.as_ref())
            .map(|failure| failure.phase);
        if matches!(
            phase,
            Some(
                labello_client::ImportProgressPhase::Build
                    | labello_client::ImportProgressPhase::Verification
                    | labello_client::ImportProgressPhase::Commit
            )
        ) && self.import_flow.plan.is_some()
        {
            self.request_commit_import();
        } else {
            self.request_preflight_import(true);
        }
    }

    pub(crate) fn request_import_folder_selection(&mut self) {
        #[cfg(target_arch = "wasm32")]
        {
            let Some(import_id) = self
                .import_flow
                .job
                .as_ref()
                .map(|job| job.import_id.clone())
            else {
                return;
            };
            let request = self.import_request_identity(Some(import_id));
            self.runtime.active_requests.insert(request.request_id);
            self.import_flow
                .active_operations
                .insert(request.request_id, ImportActivity::SelectFolder);
            self.import_flow.busy = true;
            let limits = self
                .import_flow
                .capabilities
                .as_ref()
                .map(|capabilities| capabilities.limits.clone())
                .unwrap_or_default();
            if let Err(error) =
                crate::import_flow::browser::pick_import_folder(self, request.clone(), limits)
            {
                self.runtime.active_requests.remove(&request.request_id);
                self.import_flow
                    .active_operations
                    .remove(&request.request_id);
                self.import_flow.busy = false;
                self.import_flow.error = Some(error);
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.import_flow.error =
                Some("Browser folder selection is available in the WebAssembly build.".to_string());
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn register_selected_import_files(&mut self, files: Vec<BrowserImportFile>) {
        let Some(import_id) = self
            .import_flow
            .job
            .as_ref()
            .map(|job| job.import_id.clone())
        else {
            return;
        };
        let limits = self
            .import_flow
            .capabilities
            .as_ref()
            .map(|capabilities| capabilities.limits.clone())
            .unwrap_or_default();
        let total_bytes = files.iter().map(|file| file.byte_size).sum::<u64>();
        if files.is_empty()
            || files.len() as u64 > limits.max_browser_files
            || total_bytes > limits.max_browser_bytes
            || files
                .iter()
                .any(|file| file.byte_size > limits.max_single_file_bytes)
        {
            self.import_flow.busy = false;
            self.import_flow.error = Some(
                "Selected folder is empty or exceeds the advertised browser import limits."
                    .to_string(),
            );
            return;
        }
        self.import_flow.browser_files = files
            .iter()
            .map(|file| (file.client_file_id.clone(), file.file.clone()))
            .collect();
        self.import_flow.registered_paths = files
            .iter()
            .map(|file| RegisteredImportPath {
                client_file_id: file.client_file_id.clone(),
                file_id: String::new(),
                relative_path: file.relative_path.clone(),
            })
            .collect();
        let body = labello_client::RegisterImportFilesRequest {
            files: files
                .into_iter()
                .map(|file| labello_client::ImportFileRegistration {
                    client_file_id: file.client_file_id,
                    relative_path: file.relative_path,
                    byte_size: file.byte_size,
                    blake3: Some(file.blake3),
                })
                .collect(),
        };
        let request = self.import_request_identity(Some(import_id.clone()));
        let key = import_key("register", request.request_id);
        self.import_flow.busy = true;
        self.queue_command(UiCommand::RegisterImportFiles {
            request,
            import_id,
            body,
            idempotency_key: key,
        });
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn upload_next_import_chunk(&mut self) {
        let Some(import_id_value) = self
            .import_flow
            .job
            .as_ref()
            .map(|job| job.import_id.clone())
        else {
            return;
        };
        let Some(file) = self
            .import_flow
            .browser_uploads
            .iter()
            .find(|file| !file.complete)
            .cloned()
        else {
            self.import_flow.busy = false;
            return;
        };
        let Some(source) = self
            .import_flow
            .browser_files
            .get(&file.client_file_id)
            .cloned()
        else {
            self.import_flow.busy = false;
            self.import_flow.error = Some(
                "Upload source is no longer selected. Reselect the same folder to continue."
                    .to_string(),
            );
            return;
        };
        let Some(uploader) = self.runtime.import_chunk_uploader.clone() else {
            self.import_flow.busy = false;
            self.import_flow.error =
                Some("Raw browser import transport is unavailable.".to_string());
            return;
        };
        let Some(csrf_token) = self.runtime.api.as_ref().and_then(|api| api.csrf_token()) else {
            self.import_flow.busy = false;
            self.import_flow.error =
                Some("Import upload requires an authenticated session.".to_string());
            return;
        };
        let chunk_bytes = self
            .import_flow
            .capabilities
            .as_ref()
            .map(|capabilities| capabilities.limits.upload_chunk_bytes)
            .unwrap_or(8 * 1024 * 1024)
            .max(1);
        let offset = file.accepted_bytes;
        let length = (file.byte_size - offset).min(chunk_bytes);
        let request = self.import_request_identity(Some(import_id_value.clone()));
        self.runtime.active_requests.insert(request.request_id);
        self.import_flow
            .active_operations
            .insert(request.request_id, ImportActivity::UploadChunk);
        self.import_flow.busy = true;
        let api_base_url = self.config.api_base_url.clone();
        let import_id = import_id_value.to_string();
        let file_id = file.file_id.clone();
        let idempotency_key = import_key("chunk", request.request_id);
        self.spawn_import_message(async move {
            let result = async {
                let bytes = browser::read_file_range(&source, offset, length).await?;
                let digest = blake3::hash(&bytes).to_hex().to_string();
                uploader(RawImportChunkRequest {
                    api_base_url,
                    import_id,
                    file_id: file_id.clone(),
                    offset,
                    length,
                    digest,
                    bytes,
                    csrf_token,
                    idempotency_key,
                })
                .await
            }
            .await;
            crate::app::UiMessage::ImportChunkUploaded {
                request,
                file_id,
                result,
            }
        });
    }

    fn import_attestations(&self) -> ImportAttestations {
        ImportAttestations {
            ground_truth: self.import_flow.ground_truth,
            exhaustive: self.import_flow.exhaustive,
            coverage_scope: split_csv(&self.import_flow.coverage_scope),
            provenance: self.import_flow.provenance.trim().to_string(),
        }
    }

    fn import_plan_request(&self) -> UpdateImportPlanRequest {
        let manual = self.import_flow.geometry_policy == ImportGeometryPolicy::ManualBoxGuideV1;
        let manual_skeleton = manual.then(|| SkeletonSpec {
            keypoints: split_csv(&self.import_flow.keypoint_names)
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
                policy: policy_label(self.import_flow.geometry_policy).to_string(),
                affected_count: diagnostic.count,
                acknowledged: true,
            })
            .collect();
        if self
            .import_flow
            .categories
            .iter()
            .any(|category| !category.geometry_mappings.is_empty())
        {
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
                            mapping.task.task_id =
                                TaskId::from(category.bounding_box_task_id.trim());
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
                                    guide_task_id: TaskId::from(
                                        category.bounding_box_task_id.trim(),
                                    ),
                                    cardinality: labello_domain::MigrationCardinality::ExactlyOne,
                                    allow_exclusion: true,
                                    sequence:
                                        labello_domain::MigrationSequence::ImportedSpatialOrderV1,
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
                    let skeleton =
                        category
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
                        source_keypoint_names: if category
                            .direct_geometry
                            .contains(&ImportGeometryKind::Skeleton)
                        {
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
                    skeleton_mappings.push(mapping);
                }
            }
            return UpdateImportPlanRequest {
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
                    .flat_map(|category| category.geometry_mappings.iter().cloned())
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
            };
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
                    let key = category.source_category_key.trim().to_string();
                    let mapping =
                        |source_geometry, target_geometry, policy| ImportGeometryMappingRequest {
                            source_category_key: key.clone(),
                            source_geometry,
                            target_geometry,
                            policy,
                            parameters: Vec::<ImportMappingParameter>::new(),
                        };
                    match self.import_flow.geometry_policy {
                        ImportGeometryPolicy::Direct => {
                            let mut mappings = Vec::new();
                            if self.import_flow.direct_bounding_boxes {
                                mappings.push(mapping(
                                    ImportGeometryKind::BoundingBox,
                                    ImportGeometryKind::BoundingBox,
                                    ImportGeometryPolicy::Direct,
                                ));
                            }
                            if self.import_flow.direct_skeletons {
                                mappings.push(mapping(
                                    ImportGeometryKind::Skeleton,
                                    ImportGeometryKind::Skeleton,
                                    ImportGeometryPolicy::Direct,
                                ));
                            }
                            mappings
                        }
                        ImportGeometryPolicy::ManualBoxGuideV1 => vec![
                            mapping(
                                ImportGeometryKind::BoundingBox,
                                ImportGeometryKind::BoundingBox,
                                ImportGeometryPolicy::Direct,
                            ),
                            mapping(
                                ImportGeometryKind::BoundingBox,
                                ImportGeometryKind::Skeleton,
                                ImportGeometryPolicy::ManualBoxGuideV1,
                            ),
                        ],
                        ImportGeometryPolicy::Omit => {
                            let mut mappings = vec![mapping(
                                ImportGeometryKind::BoundingBox,
                                ImportGeometryKind::BoundingBox,
                                ImportGeometryPolicy::Omit,
                            )];
                            if profile_has_skeletons(self.import_flow.profile) {
                                mappings.push(mapping(
                                    ImportGeometryKind::Skeleton,
                                    ImportGeometryKind::Skeleton,
                                    ImportGeometryPolicy::Omit,
                                ));
                            }
                            mappings
                        }
                        ImportGeometryPolicy::KeypointEnvelopeV1
                        | ImportGeometryPolicy::BoxRelativeTemplateV1 => Vec::new(),
                    }
                })
                .collect(),
            task_mappings: self
                .import_flow
                .categories
                .iter()
                .filter(|category| category.selected)
                .flat_map(|category| {
                    let class_id = ClassId::from(category.class_id.trim());
                    let key = category.source_category_key.trim().to_string();
                    let mut mappings = Vec::new();
                    let include_box = manual
                        || (self.import_flow.geometry_policy == ImportGeometryPolicy::Direct
                            && self.import_flow.direct_bounding_boxes);
                    if include_box {
                        mappings.push(ImportTaskMappingRequest {
                            source_category_key: key.clone(),
                            task: mapped_task(
                                TaskId::from(category.bounding_box_task_id.trim()),
                                category.bounding_box_task_name.trim(),
                                AnnotationType::BoundingBox,
                                class_id.clone(),
                                None,
                                None,
                                self.import_flow.workflow_intent,
                            ),
                            workflow_intent: self.import_flow.workflow_intent,
                        });
                    }
                    let include_skeleton = manual
                        || (self.import_flow.geometry_policy == ImportGeometryPolicy::Direct
                            && self.import_flow.direct_skeletons);
                    if include_skeleton {
                        let guide_task_id = TaskId::from(category.bounding_box_task_id.trim());
                        let migration = manual.then_some(labello_domain::ManualBoxGuideMigration {
                            guide_task_id,
                            cardinality: labello_domain::MigrationCardinality::ExactlyOne,
                            allow_exclusion: true,
                            sequence: labello_domain::MigrationSequence::ImportedSpatialOrderV1,
                        });
                        mappings.push(ImportTaskMappingRequest {
                            source_category_key: key,
                            task: mapped_task(
                                TaskId::from(category.skeleton_task_id.trim()),
                                category.skeleton_task_name.trim(),
                                AnnotationType::Skeleton,
                                class_id,
                                if manual {
                                    manual_skeleton.clone()
                                } else {
                                    category.source_skeleton.clone()
                                },
                                migration,
                                self.import_flow.workflow_intent,
                            ),
                            workflow_intent: self.import_flow.workflow_intent,
                        });
                    }
                    mappings
                })
                .collect(),
            skeleton_mappings: self
                .import_flow
                .categories
                .iter()
                .filter(|category| {
                    category.selected
                        && (manual
                            || (self.import_flow.geometry_policy == ImportGeometryPolicy::Direct
                                && self.import_flow.direct_skeletons))
                })
                .filter_map(|category| {
                    let skeleton = if manual {
                        manual_skeleton.clone()
                    } else {
                        category.source_skeleton.clone()
                    }?;
                    Some(labello_client::ImportSkeletonMappingRequest {
                        source_category_key: category.source_category_key.trim().to_string(),
                        target_task_id: TaskId::from(category.skeleton_task_id.trim()),
                        source_keypoint_names: if manual {
                            Vec::new()
                        } else {
                            skeleton
                                .keypoints
                                .iter()
                                .map(|keypoint| keypoint.name.clone())
                                .collect()
                        },
                        skeleton: skeleton.clone(),
                        names_confirmed: true,
                    })
                })
                .collect(),
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

    #[cfg(test)]
    fn import_descriptors_valid(&self) -> bool {
        self.import_descriptor_error().is_none()
    }

    fn import_descriptor_error(&self) -> Option<String> {
        let coco = is_coco_profile(self.import_flow.profile);
        if !valid_identity_component(&self.import_flow.source_namespace) {
            return Some(
                "Source namespace must use only letters, numbers, '.', '_', or '-'.".to_string(),
            );
        }
        let reference_valid =
            |reference: &str| {
                !reference.trim().is_empty()
                    && (self.import_flow.transport == ImportTransport::ServerDirectory
                        || self.import_flow.registered_paths.iter().any(|path| {
                            path.file_id == reference || path.client_file_id == reference
                        }))
            };
        if !coco {
            let Some(descriptor) = self.import_flow.descriptors.first() else {
                return Some("Select one Dataset YAML.".to_string());
            };
            if self.import_flow.descriptors.len() != 1
                || descriptor.kind != ImportDescriptorKind::YoloDataset
            {
                return Some("YOLO imports require exactly one Dataset YAML.".to_string());
            }
            if !reference_valid(&descriptor.descriptor_file_id) {
                return Some("Select a registered Dataset YAML.".to_string());
            }
            if !valid_identity_component(&descriptor.release) {
                return Some(
                    "Release must use only letters, numbers, '.', '_', or '-'.".to_string(),
                );
            }
            if self.import_flow.yolo_inspection_loading {
                return Some("Wait for YAML split inspection to finish.".to_string());
            }
            if let Some(error) = &self.import_flow.yolo_inspection_error {
                return Some(error.clone());
            }
            if self
                .import_flow
                .yolo_inspected_descriptor_file_id
                .as_deref()
                .map(str::trim)
                != Some(descriptor.descriptor_file_id.trim())
            {
                return Some("Inspect the selected YAML before sealing the source.".to_string());
            }
            if !self
                .import_flow
                .yolo_splits
                .iter()
                .any(|split| split.usable && split.selected)
            {
                return Some("Select at least one usable YAML split.".to_string());
            }
            return None;
        }
        let mut descriptor_references = std::collections::BTreeSet::new();
        let mut descriptor_identities = std::collections::BTreeSet::new();
        let valid = !self.import_flow.descriptors.is_empty()
            && self.import_flow.descriptors.iter().all(|descriptor| {
                descriptor_kind_allowed(self.import_flow.profile, descriptor.kind)
                    && reference_valid(&descriptor.descriptor_file_id)
                    && descriptor_references.insert(descriptor.descriptor_file_id.trim())
                    && valid_identity_component(&descriptor.release)
                    && valid_identity_component(&descriptor.split)
                    && (descriptor.pairing_group.trim().is_empty()
                        || valid_identity_component(&descriptor.pairing_group))
                    && descriptor_identities.insert((
                        descriptor_kind_label(descriptor.kind),
                        descriptor.release.trim(),
                        descriptor.split.trim(),
                        descriptor.pairing_group.trim(),
                    ))
                    && (!coco || reference_valid(&descriptor.image_root_file_id))
            })
            && valid_identity_component(&self.import_flow.source_namespace);
        (!valid).then(|| {
            "Every COCO descriptor needs a unique registered JSON file, valid release and split, and an exact registered image root."
                .to_string()
        })
    }

    fn import_mappings_complete(&self) -> bool {
        let discovered = self
            .import_flow
            .job
            .as_ref()
            .and_then(|job| job.preflight_report.as_ref())
            .map(|report| report.source.categories as usize)
            .or_else(|| {
                self.import_flow
                    .plan
                    .as_ref()
                    .map(|plan| plan.report.source.categories as usize)
            })
            .unwrap_or(0);
        let mut source_keys = std::collections::BTreeSet::new();
        let mut class_ids = std::collections::BTreeSet::new();
        let mut task_ids = std::collections::BTreeSet::new();
        let selected = self
            .import_flow
            .categories
            .iter()
            .filter(|category| category.selected)
            .count();
        let direct = self.import_flow.geometry_policy == ImportGeometryPolicy::Direct;
        let manual = self.import_flow.geometry_policy == ImportGeometryPolicy::ManualBoxGuideV1;
        let category_specific = self
            .import_flow
            .categories
            .iter()
            .any(|category| !category.geometry_mappings.is_empty());
        let manual_categories = if category_specific {
            self.import_flow
                .categories
                .iter()
                .filter(|category| {
                    category.selected
                        && category
                            .geometry_mappings
                            .iter()
                            .any(|mapping| mapping.policy == ImportGeometryPolicy::ManualBoxGuideV1)
                })
                .count()
        } else if manual {
            selected
        } else {
            0
        };
        let manual_available = self
            .import_flow
            .capabilities
            .as_ref()
            .is_some_and(|capabilities| capabilities.manual_box_guide_migration);
        let has_seed_workflow = if category_specific {
            self.import_flow.categories.iter().any(|category| {
                category.selected
                    && category.workflow_intent == ImportWorkflowIntent::SeedFutureAnnotation
            })
        } else {
            self.import_flow.workflow_intent == ImportWorkflowIntent::SeedFutureAnnotation
        };
        discovered > 0
            && self.import_flow.categories.len() == discovered
            && selected > 0
            && self.import_flow.categories.iter().all(|category| {
                let active_targets = category
                    .geometry_mappings
                    .iter()
                    .filter(|mapping| mapping.policy != ImportGeometryPolicy::Omit)
                    .map(|mapping| mapping.target_geometry)
                    .collect::<std::collections::BTreeSet<_>>();
                let category_manual = category
                    .geometry_mappings
                    .iter()
                    .any(|mapping| mapping.policy == ImportGeometryPolicy::ManualBoxGuideV1);
                let bounding_boxes = if category_specific {
                    active_targets.contains(&ImportGeometryKind::BoundingBox)
                } else {
                    (direct && self.import_flow.direct_bounding_boxes) || manual
                };
                let skeletons = if category_specific {
                    active_targets.contains(&ImportGeometryKind::Skeleton)
                } else {
                    (direct && self.import_flow.direct_skeletons) || manual
                };
                let skeleton_schema_valid = if category_specific {
                    category.geometry_mappings.iter().all(|mapping| {
                        mapping.policy == ImportGeometryPolicy::Omit
                            || mapping.target_geometry != ImportGeometryKind::Skeleton
                            || match mapping.source_geometry {
                                ImportGeometryKind::Skeleton => category.source_skeleton.is_some(),
                                ImportGeometryKind::BoundingBox => {
                                    !split_csv(&category.target_keypoint_names).is_empty()
                                }
                            }
                    })
                } else if manual {
                    !split_csv(&self.import_flow.keypoint_names).is_empty()
                } else {
                    !skeletons || category.source_skeleton.is_some()
                };
                !category.source_category_key.trim().is_empty()
                    && !category.source_category_id.trim().is_empty()
                    && ClassId::from(category.class_id.trim())
                        .validate_path_segment()
                        .is_ok()
                    && !category.class_name.trim().is_empty()
                    && valid_color(&category.class_color)
                    && source_keys.insert(category.source_category_key.trim())
                    && (!category.selected || class_ids.insert(category.class_id.trim()))
                    && (!category.selected || !category_manual || bounding_boxes)
                    && (!category.selected || !category_specific || bounding_boxes || skeletons)
                    && (!category.selected
                        || !bounding_boxes
                        || (!category.bounding_box_task_name.trim().is_empty()
                            && TaskId::from(category.bounding_box_task_id.trim())
                                .validate_path_segment()
                                .is_ok()
                            && task_ids.insert(category.bounding_box_task_id.trim())))
                    && (!category.selected
                        || !skeletons
                        || (!category.skeleton_task_name.trim().is_empty()
                            && TaskId::from(category.skeleton_task_id.trim())
                                .validate_path_segment()
                                .is_ok()
                            && task_ids.insert(category.skeleton_task_id.trim())
                            && skeleton_schema_valid))
            })
            && (category_specific
                || !direct
                || self.import_flow.direct_bounding_boxes
                || self.import_flow.direct_skeletons)
            && (category_specific || self.import_flow.geometry_policy != ImportGeometryPolicy::Omit)
            && (!has_seed_workflow || self.import_flow.seed_workflow_confirmed)
            && (manual_categories == 0 || manual_available)
    }

    fn import_plan_is_current(&self) -> bool {
        let Some(plan) = self.import_flow.plan.as_ref() else {
            return false;
        };
        if let Some(accepted) = self.import_flow.accepted_plan_request.as_ref() {
            return accepted == &self.import_plan_request();
        }
        self.import_flow.job.as_ref().is_some_and(|job| {
            job.plan_hash.as_deref() == Some(plan.plan_hash.as_str())
                && job.source_fingerprint.as_deref() == Some(plan.source_fingerprint.as_str())
        })
    }

    fn import_plan_covers_all_categories(&self) -> bool {
        let Some(plan) = self.import_flow.plan.as_ref() else {
            return false;
        };
        let selected = self
            .import_flow
            .categories
            .iter()
            .filter(|category| category.selected)
            .count() as u64;
        let category_specific = self
            .import_flow
            .categories
            .iter()
            .any(|category| !category.geometry_mappings.is_empty());
        let required_tasks = if category_specific {
            self.import_flow
                .categories
                .iter()
                .filter(|category| category.selected)
                .map(|category| {
                    category
                        .geometry_mappings
                        .iter()
                        .filter(|mapping| mapping.policy != ImportGeometryPolicy::Omit)
                        .map(|mapping| mapping.target_geometry)
                        .collect::<std::collections::BTreeSet<_>>()
                        .len() as u64
                })
                .sum()
        } else {
            let tasks_per_category =
                if self.import_flow.geometry_policy == ImportGeometryPolicy::ManualBoxGuideV1 {
                    2
                } else {
                    u64::from(self.import_flow.direct_bounding_boxes)
                        + u64::from(self.import_flow.direct_skeletons)
                };
            selected.saturating_mul(tasks_per_category)
        };
        selected > 0
            && plan.report.output.classes == selected
            && plan.report.output.tasks >= required_tasks
    }

    fn restart_import_setup(&mut self) {
        self.begin_import_epoch();
        self.import_flow.reset_job();
        self.import_flow.open = true;
    }
}

fn mapped_task(
    task_id: TaskId,
    name: &str,
    annotation_type: AnnotationType,
    class_id: ClassId,
    skeleton: Option<SkeletonSpec>,
    manual_box_guide_migration: Option<labello_domain::ManualBoxGuideMigration>,
    workflow_intent: ImportWorkflowIntent,
) -> TaskDefinition {
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
        review: review_config(workflow_intent),
        prelabel_config_ids: Vec::new(),
        manual_box_guide_migration,
        enabled: true,
    }
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

fn source_file_selector(
    ui: &mut egui::Ui,
    label: &str,
    selected: &mut String,
    paths: &[RegisteredImportPath],
    include: impl Fn(&str) -> bool,
) -> bool {
    let previous = selected.clone();
    egui::ComboBox::from_label(label)
        .selected_text(
            paths
                .iter()
                .find(|path| path.file_id == *selected || path.client_file_id == *selected)
                .map(|path| path.relative_path.as_str())
                .unwrap_or("Choose a registered file"),
        )
        .show_ui(ui, |ui| {
            for path in paths.iter().filter(|path| include(&path.relative_path)) {
                let reference = if path.file_id.is_empty() {
                    &path.client_file_id
                } else {
                    &path.file_id
                };
                ui.selectable_value(selected, reference.clone(), &path.relative_path);
            }
        });
    *selected != previous
}

fn server_source_file_picker(
    ui: &mut egui::Ui,
    label: &str,
    selected: &str,
    paths: &[RegisteredImportPath],
    button_label: &str,
) -> bool {
    let display = paths
        .iter()
        .find(|path| path.file_id == selected)
        .map(|path| path.relative_path.as_str())
        .unwrap_or(if selected.is_empty() {
            "Not selected"
        } else {
            "Selected staged file"
        });
    status_row(ui, label, display);
    ui.button(button_label).clicked()
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

fn profile_has_skeletons(profile: ImportProfile) -> bool {
    matches!(
        profile,
        ImportProfile::UltralyticsYoloPoseV1 | ImportProfile::CocoKeypointsGtV1
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

fn current_import_stage(flow: &ImportFlowState) -> ImportStage {
    match flow.screen {
        ImportScreen::Source => ImportStage::Source,
        ImportScreen::Configure => ImportStage::Configure,
        ImportScreen::Preflight => ImportStage::Preflight,
        ImportScreen::Ready => ImportStage::Ready,
        ImportScreen::Running | ImportScreen::Success => ImportStage::Import,
        ImportScreen::Failure => failure_import_stage(flow),
    }
}

fn failure_import_stage(flow: &ImportFlowState) -> ImportStage {
    let phase = flow
        .job
        .as_ref()
        .and_then(|job| job.failure.as_ref().map(|failure| failure.phase))
        .or_else(|| {
            (flow.plan.is_none())
                .then(|| flow.job.as_ref().map(|job| job.progress.phase))
                .flatten()
        });
    match phase {
        Some(labello_client::ImportProgressPhase::Registration) => ImportStage::Source,
        Some(
            labello_client::ImportProgressPhase::Upload
            | labello_client::ImportProgressPhase::Sealing,
        ) => ImportStage::Configure,
        Some(labello_client::ImportProgressPhase::Preflight) => ImportStage::Preflight,
        Some(
            labello_client::ImportProgressPhase::Build
            | labello_client::ImportProgressPhase::Verification
            | labello_client::ImportProgressPhase::Commit,
        ) => ImportStage::Import,
        Some(labello_client::ImportProgressPhase::Cleanup) | None => {
            if flow.plan.is_some() {
                ImportStage::Import
            } else if flow
                .job
                .as_ref()
                .is_some_and(|job| job.preflight_report.is_some())
            {
                ImportStage::Preflight
            } else if flow.job.is_some() {
                ImportStage::Configure
            } else {
                ImportStage::Source
            }
        }
        Some(labello_client::ImportProgressPhase::Unknown) => ImportStage::Configure,
    }
}

fn import_stage_status(flow: &ImportFlowState, stage: ImportStage) -> ImportStageStatus {
    if flow.screen == ImportScreen::Success {
        return ImportStageStatus::Complete;
    }
    let current = current_import_stage(flow);
    if flow.screen == ImportScreen::Failure && stage == current {
        return ImportStageStatus::Failed;
    }
    match stage.index().cmp(&current.index()) {
        std::cmp::Ordering::Less => ImportStageStatus::Complete,
        std::cmp::Ordering::Equal => ImportStageStatus::Active,
        std::cmp::Ordering::Greater => ImportStageStatus::Pending,
    }
}

fn import_stage_pill(
    ui: &mut egui::Ui,
    stage: ImportStage,
    status: ImportStageStatus,
    fraction: Option<f32>,
) {
    let (color, fill, stroke) = match status {
        ImportStageStatus::Pending => (theme::TEXT_DISABLED, theme::SURFACE, theme::BORDER),
        ImportStageStatus::Active => (
            theme::ACCENT,
            egui::Color32::from_rgba_unmultiplied(
                theme::ACCENT.r(),
                theme::ACCENT.g(),
                theme::ACCENT.b(),
                60,
            ),
            theme::ACCENT,
        ),
        ImportStageStatus::Complete => (
            theme::SUCCESS,
            egui::Color32::from_rgba_unmultiplied(
                theme::SUCCESS.r(),
                theme::SUCCESS.g(),
                theme::SUCCESS.b(),
                24,
            ),
            theme::SUCCESS.gamma_multiply(0.6),
        ),
        ImportStageStatus::Failed => (
            theme::DANGER,
            egui::Color32::from_rgba_unmultiplied(
                theme::DANGER.r(),
                theme::DANGER.g(),
                theme::DANGER.b(),
                36,
            ),
            theme::DANGER,
        ),
    };
    let response = egui::Frame::new()
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, stroke))
        .corner_radius(egui::CornerRadius::same(theme::BADGE_RADIUS))
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(78.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(format!("{} {}", stage.index() + 1, stage.label()))
                        .color(color)
                        .small()
                        .strong(),
                );
                let (track, _) =
                    ui.allocate_exact_size(egui::vec2(78.0, 3.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(track, 2.0, theme::BORDER_STRONG.gamma_multiply(0.55));
                match fraction {
                    Some(fraction) => {
                        let width = track.width() * fraction.clamp(0.0, 1.0);
                        if width > 0.0 {
                            ui.painter().rect_filled(
                                egui::Rect::from_min_size(
                                    track.min,
                                    egui::vec2(width, track.height()),
                                ),
                                2.0,
                                color,
                            );
                        }
                    }
                    None => {
                        ui.ctx().request_repaint();
                        let phase = ((ui.input(|input| input.time) as f32 * 0.7) % 1.35) - 0.35;
                        let segment = egui::Rect::from_min_size(
                            egui::pos2(track.left() + track.width() * phase, track.top()),
                            egui::vec2(track.width() * 0.35, track.height()),
                        );
                        ui.painter()
                            .with_clip_rect(track)
                            .rect_filled(segment, 2.0, color);
                    }
                }
            });
        })
        .response;
    let status_label = match status {
        ImportStageStatus::Pending => "pending",
        ImportStageStatus::Active => "current",
        ImportStageStatus::Complete => "complete",
        ImportStageStatus::Failed => "failed",
    };
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::ProgressIndicator,
            true,
            format!(
                "Step {} of 5: {}, {status_label}",
                stage.index() + 1,
                stage.label()
            ),
        )
    });
}

fn indeterminate_import_progress(ui: &mut egui::Ui, label: &str, color: egui::Color32) {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().max(96.0), 18.0),
        egui::Sense::hover(),
    );
    ui.ctx().request_repaint();
    ui.painter()
        .rect_filled(rect, 12.0, ui.visuals().extreme_bg_color);
    let phase = ((ui.input(|input| input.time) as f32 * 0.55) % 1.4) - 0.4;
    let segment = egui::Rect::from_min_size(
        egui::pos2(rect.left() + rect.width() * phase, rect.top()),
        egui::vec2(rect.width() * 0.4, rect.height()),
    );
    ui.painter()
        .with_clip_rect(rect)
        .rect_filled(segment, 12.0, color.gamma_multiply(0.8));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::ProgressIndicator, true, label.to_string())
    });
}

fn import_human_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

pub(crate) fn import_screen(job: &ImportJob, plan: Option<&ImportPlan>) -> ImportScreen {
    match job.lifecycle {
        ImportLifecycle::Registering | ImportLifecycle::Uploading | ImportLifecycle::Sealed => {
            ImportScreen::Configure
        }
        ImportLifecycle::Preflighting => ImportScreen::Preflight,
        ImportLifecycle::AwaitingDecision if plan.is_some_and(|plan| plan.commit_ready) => {
            ImportScreen::Ready
        }
        ImportLifecycle::AwaitingDecision => ImportScreen::Preflight,
        ImportLifecycle::Building | ImportLifecycle::Verifying | ImportLifecycle::Committing => {
            ImportScreen::Running
        }
        ImportLifecycle::Succeeded => ImportScreen::Success,
        ImportLifecycle::Failed | ImportLifecycle::Cancelled | ImportLifecycle::Expired => {
            ImportScreen::Failure
        }
        ImportLifecycle::Unknown => ImportScreen::Failure,
    }
}

fn import_step_label(screen: ImportScreen) -> &'static str {
    match screen {
        ImportScreen::Source => "1. Source, profile, transport, and attestations",
        ImportScreen::Configure => "2. Register files and configure the source",
        ImportScreen::Preflight => "3. Preflight diagnostics and mappings",
        ImportScreen::Ready => "4. Ready to commit",
        ImportScreen::Running => "5. Building and verifying the dataset",
        ImportScreen::Failure => "Import needs attention",
        ImportScreen::Success => "Import complete",
    }
}

fn profile_label(profile: ImportProfile) -> &'static str {
    match profile {
        ImportProfile::UltralyticsYoloDetectV1 => "Ultralytics YOLO detect v1",
        ImportProfile::UltralyticsYoloPoseV1 => "Ultralytics YOLO pose v1",
        ImportProfile::CocoInstancesGtV1 => "COCO instances ground truth v1",
        ImportProfile::CocoKeypointsGtV1 => "COCO keypoints ground truth v1",
        ImportProfile::Unknown => "Unknown profile",
    }
}

fn transport_label(transport: ImportTransport) -> &'static str {
    match transport {
        ImportTransport::BrowserFolder => "Browser folder upload",
        ImportTransport::ServerDirectory => "Server directory",
        ImportTransport::Unknown => "Unknown transport",
    }
}

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

fn policy_label(policy: ImportGeometryPolicy) -> &'static str {
    match policy {
        ImportGeometryPolicy::Direct => "Direct",
        ImportGeometryPolicy::KeypointEnvelopeV1 => "Keypoint envelope",
        ImportGeometryPolicy::ManualBoxGuideV1 => "Manual box guide",
        ImportGeometryPolicy::BoxRelativeTemplateV1 => "Box-relative template",
        ImportGeometryPolicy::Omit => "Omit",
    }
}

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

fn mapping_parameter_editor(
    ui: &mut egui::Ui,
    mapping: &mut ImportGeometryMappingRequest,
    skeleton: Option<&SkeletonSpec>,
    target_keypoint_names: &str,
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
                            ui.add(egui::DragValue::new(value).speed(0.01));
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
            if !mapping
                .parameters
                .iter()
                .all(|parameter| matches!(parameter, ImportMappingParameter::Point { .. }))
                || mapping.parameters.is_empty()
            {
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
                mapping.parameters = names
                    .into_iter()
                    .map(|name| ImportMappingParameter::Point {
                        name,
                        x: 0.5,
                        y: 0.5,
                        state: labello_domain::KeypointState::Visible,
                    })
                    .collect();
            }
            for parameter in &mut mapping.parameters {
                if let ImportMappingParameter::Point { name, x, y, state } = parameter {
                    ui.horizontal(|ui| {
                        ui.label(name.as_str());
                        ui.label("x");
                        ui.add(egui::DragValue::new(x).range(0.0..=1.0).speed(0.01));
                        ui.label("y");
                        ui.add(egui::DragValue::new(y).range(0.0..=1.0).speed(0.01));
                        egui::ComboBox::from_id_salt("state")
                            .selected_text(format!("{state:?}"))
                            .show_ui(ui, |ui| {
                                for candidate in [
                                    labello_domain::KeypointState::Visible,
                                    labello_domain::KeypointState::Hidden,
                                    labello_domain::KeypointState::Absent,
                                ] {
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

fn intent_label(intent: ImportWorkflowIntent) -> &'static str {
    match intent {
        ImportWorkflowIntent::AuthoritativeGroundTruth => "Authoritative ground truth",
        ImportWorkflowIntent::RequireApproval => "Require approval",
        ImportWorkflowIntent::SeedFutureAnnotation => "Seed future annotation",
    }
}

fn lifecycle_label(lifecycle: ImportLifecycle) -> &'static str {
    match lifecycle {
        ImportLifecycle::Registering => "Registering files",
        ImportLifecycle::Uploading => "Uploading files",
        ImportLifecycle::Sealed => "Source sealed",
        ImportLifecycle::Preflighting => "Running preflight",
        ImportLifecycle::AwaitingDecision => "Awaiting decision",
        ImportLifecycle::Building => "Building dataset",
        ImportLifecycle::Verifying => "Verifying dataset",
        ImportLifecycle::Committing => "Committing dataset",
        ImportLifecycle::Succeeded => "Succeeded",
        ImportLifecycle::Failed => "Failed",
        ImportLifecycle::Cancelled => "Cancelled",
        ImportLifecycle::Expired => "Expired",
        ImportLifecycle::Unknown => "Unknown",
    }
}

#[derive(Default)]
struct ImportDiagnosticOverview {
    errors: usize,
    warnings: usize,
    information: usize,
    affected: u64,
    blocking: usize,
    unacknowledged: usize,
}

impl ImportDiagnosticOverview {
    fn from_diagnostics(
        diagnostics: &[labello_client::ImportDiagnosticSummary],
        acknowledgements: &std::collections::BTreeSet<String>,
    ) -> ImportDiagnosticOverview {
        let mut overview = Self::default();
        for diagnostic in diagnostics {
            match diagnostic.severity {
                ImportDiagnosticSeverity::Error => overview.errors += 1,
                ImportDiagnosticSeverity::WarningRequiresAck
                | ImportDiagnosticSeverity::Warning => overview.warnings += 1,
                ImportDiagnosticSeverity::Info | ImportDiagnosticSeverity::Unknown => {
                    overview.information += 1;
                }
            }
            overview.affected = overview.affected.saturating_add(diagnostic.count);
            overview.blocking += usize::from(diagnostic.impact.blocks_commit);
            overview.unacknowledged += usize::from(
                diagnostic.impact.requires_acknowledgement
                    && !acknowledgements.contains(&diagnostic.code),
            );
        }
        overview
    }

    fn disclosure_label(&self, compact: bool) -> String {
        let mut severities = Vec::new();
        if self.errors > 0 {
            severities.push(counted(self.errors, "error"));
        }
        if self.warnings > 0 {
            severities.push(counted(self.warnings, "warning"));
        }
        if self.information > 0 {
            severities.push(format!("{} info", self.information));
        }
        if severities.is_empty() {
            return "Diagnostics (none)".to_string();
        }

        if compact {
            let action = if self.blocking > 0 {
                " · commit blocked"
            } else if self.unacknowledged > 0 {
                " · action required"
            } else {
                ""
            };
            format!("Diagnostics ({}){action}", severities.join(", "))
        } else {
            let mut parts = vec![severities.join(", ")];
            if self.affected > 0 {
                parts.push(format!("{} affected", self.affected));
            }
            if self.blocking > 0 {
                parts.push(counted(self.blocking, "blocking diagnostic"));
            }
            if self.unacknowledged > 0 {
                parts.push(format!(
                    "{} acknowledgement{} required",
                    self.unacknowledged,
                    if self.unacknowledged == 1 { "" } else { "s" }
                ));
            }
            format!("Diagnostics — {}", parts.join(" · "))
        }
    }

    fn color(&self) -> egui::Color32 {
        if self.errors > 0 || self.blocking > 0 {
            theme::DANGER
        } else if self.warnings > 0 || self.unacknowledged > 0 {
            theme::WARNING
        } else if self.information > 0 {
            theme::INFO
        } else {
            theme::SUCCESS
        }
    }
}

fn counted(count: usize, singular: &str) -> String {
    format!("{count} {singular}{}", if count == 1 { "" } else { "s" })
}

fn diagnostic_severity_label(severity: ImportDiagnosticSeverity) -> &'static str {
    match severity {
        ImportDiagnosticSeverity::Error => "Error",
        ImportDiagnosticSeverity::WarningRequiresAck => "Warning requiring acknowledgement",
        ImportDiagnosticSeverity::Warning => "Warning",
        ImportDiagnosticSeverity::Info => "Information",
        ImportDiagnosticSeverity::Unknown => "Unknown-severity",
    }
}

fn status_row(ui: &mut egui::Ui, label: &str, value: impl ToString) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("{label}:")).strong());
        ui.label(value.to_string());
    });
}

pub(crate) fn import_key(action: &str, request_id: u64) -> String {
    format!("ui-{action}-{request_id}")
}

#[cfg(target_arch = "wasm32")]
pub(crate) mod browser {
    use super::*;

    use wasm_bindgen::{JsCast, closure::Closure};

    pub(crate) fn pick_import_folder(
        app: &LabelloApp,
        request: crate::app::ImportRequestIdentity,
        limits: labello_client::ImportLimits,
    ) -> Result<(), String> {
        let window = web_sys::window().ok_or_else(|| "missing browser window".to_string())?;
        let document = window
            .document()
            .ok_or_else(|| "missing browser document".to_string())?;
        let input = document
            .create_element("input")
            .map_err(js_error)?
            .dyn_into::<web_sys::HtmlInputElement>()
            .map_err(|_| "failed to create import folder input".to_string())?;
        input.set_type("file");
        input.set_multiple(true);
        input
            .set_attribute("webkitdirectory", "")
            .map_err(js_error)?;
        input.set_attribute("directory", "").map_err(js_error)?;
        input
            .set_attribute("style", "display:none")
            .map_err(js_error)?;
        document
            .body()
            .ok_or_else(|| "missing browser document body".to_string())?
            .append_child(&input)
            .map_err(js_error)?;
        let tx = app.runtime.tx.clone();
        let repaint = app
            .runtime
            .repaint_ctx
            .clone()
            .ok_or_else(|| "folder picker opened before egui was ready".to_string())?;
        let input_for_callback = input.clone();
        let finished = std::rc::Rc::new(std::cell::Cell::new(false));
        let finished_for_callback = finished.clone();
        let callback = Closure::<dyn FnMut(_)>::new(move |event: web_sys::Event| {
            if finished_for_callback.replace(true) {
                return;
            }
            let files = (event.type_() == "change")
                .then(|| input_for_callback.files())
                .flatten();
            input_for_callback.remove();
            let tx = tx.clone();
            let repaint = repaint.clone();
            let request = request.clone();
            let limits = limits.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let result = collect_files(files, &limits).await;
                let _ =
                    tx.send(crate::app::UiMessage::ImportBrowserFilesSelected { request, result });
                repaint.request_repaint();
            });
        });
        let function = callback.as_ref().unchecked_ref();
        input.set_onchange(Some(function));
        input
            .unchecked_ref::<web_sys::HtmlElement>()
            .set_oncancel(Some(function));
        callback.forget();
        input.click();
        Ok(())
    }

    async fn collect_files(
        files: Option<web_sys::FileList>,
        limits: &labello_client::ImportLimits,
    ) -> Result<Vec<BrowserImportFile>, String> {
        let files = files.ok_or_else(|| "folder selection cancelled".to_string())?;
        if files.length() == 0 {
            return Err("selected folder contains no files".to_string());
        }
        let mut pending = Vec::with_capacity(files.length() as usize);
        let mut total_bytes = 0_u64;
        for index in 0..files.length() {
            let file = files
                .item(index)
                .ok_or_else(|| "selected folder contains an unreadable file".to_string())?;
            let byte_size = file.unchecked_ref::<web_sys::Blob>().size() as u64;
            let relative_path = relative_file_path(&file);
            total_bytes = total_bytes.saturating_add(byte_size);
            pending.push((index, file, relative_path, byte_size));
        }
        validate_browser_selection_limits(
            pending.len(),
            total_bytes,
            pending.iter().map(|(_, _, _, size)| *size),
            limits,
        )?;
        let mut selected = Vec::with_capacity(pending.len());
        for (index, file, relative_path, byte_size) in pending {
            let blake3 = hash_file(&file, byte_size).await?;
            selected.push(BrowserImportFile {
                client_file_id: format!("browser-{index}"),
                relative_path,
                byte_size,
                blake3,
                file,
            });
        }
        Ok(selected)
    }

    async fn hash_file(file: &web_sys::File, size: u64) -> Result<String, String> {
        const HASH_CHUNK: u64 = 8 * 1024 * 1024;
        let mut hasher = blake3::Hasher::new();
        let mut offset = 0;
        while offset < size {
            let length = (size - offset).min(HASH_CHUNK);
            hasher.update(&read_file_range(file, offset, length).await?);
            offset += length;
        }
        Ok(hasher.finalize().to_hex().to_string())
    }

    pub(crate) async fn read_file_range(
        file: &web_sys::File,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, String> {
        let blob = file
            .unchecked_ref::<web_sys::Blob>()
            .slice_with_f64_and_f64(offset as f64, (offset + length) as f64)
            .map_err(js_error)?;
        let buffer = wasm_bindgen_futures::JsFuture::from(blob.array_buffer())
            .await
            .map_err(js_error)?;
        Ok(js_sys::Uint8Array::new(&buffer).to_vec())
    }

    fn relative_file_path(file: &web_sys::File) -> String {
        js_sys::Reflect::get(
            file.as_ref(),
            &wasm_bindgen::JsValue::from_str("webkitRelativePath"),
        )
        .ok()
        .and_then(|path| path.as_string())
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| file.name())
    }

    fn js_error(error: wasm_bindgen::JsValue) -> String {
        error
            .as_string()
            .unwrap_or_else(|| "browser import operation failed".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            app.import_flow.active_operations.get(&request_id),
            Some(&ImportActivity::CheckCapabilities)
        );

        app.begin_import_epoch();
        assert!(app.import_flow.active_operations.is_empty());
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
            geometry_mappings: Vec::new(),
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
        app.import_flow.capabilities = Some(ImportCapabilities {
            manual_box_guide_migration: true,
            ..Default::default()
        });
        app.import_flow.geometry_policy = ImportGeometryPolicy::ManualBoxGuideV1;
        app.import_flow.target_geometry = ImportGeometryKind::Skeleton;
        app.import_flow.keypoint_names = "nose,left_eye".to_string();
        app.import_flow.categories = vec![
            category("release:v2:17", "17", "person"),
            category("release:v2:18", "18", "vehicle"),
        ];

        let request = app.import_plan_request();

        assert_eq!(request.task_mappings.len(), 4);
        assert!(request.task_mappings.iter().any(|mapping| {
            mapping.task.task_id == TaskId::from("bounding_box:person")
                && mapping.task.manual_box_guide_migration.is_none()
        }));
        assert!(request.task_mappings.iter().any(|mapping| {
            mapping.task.task_id == TaskId::from("skeleton:person")
                && mapping
                    .task
                    .manual_box_guide_migration
                    .as_ref()
                    .is_some_and(|migration| {
                        migration.guide_task_id == TaskId::from("bounding_box:person")
                    })
        }));
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
        app.import_flow.capabilities = Some(ImportCapabilities {
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
        app.import_flow.plan = Some(ImportPlan {
            report: report.clone(),
            ..Default::default()
        });
        app.import_flow.job = Some(ImportJob {
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
        app.import_flow.categories = vec![person, vehicle];

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

        app.import_flow.diagnostics = vec![labello_client::ImportDiagnostic::default()];
        app.import_flow.diagnostics_cursor = Some("old".to_string());
        app.request_update_import_plan();
        assert!(app.import_flow.plan.is_none());
        assert!(app.import_flow.pending_plan_request.is_some());
        assert!(app.import_flow.diagnostics.is_empty());
        assert!(app.import_flow.diagnostics_cursor.is_none());
    }

    #[test]
    fn recovery_restores_each_manual_category_target_schema() {
        let mut planned = LabelloApp::default();
        planned.import_flow.categories = vec![
            manual_category("source:0", "0", "person", "nose, left_eye"),
            manual_category("source:1", "1", "vehicle", "wheel, axle"),
        ];
        let accepted = planned.import_plan_request();
        let source_categories = planned
            .import_flow
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
                accepted_plan: Some(plan),
                ..Default::default()
            }),
        };

        let mut recovered = LabelloApp::default();
        recovered.import_flow.capabilities = Some(ImportCapabilities {
            manual_box_guide_migration: true,
            ..Default::default()
        });
        recovered.import_flow.hydrate_job_contract(&job);

        assert_eq!(recovered.import_flow.categories.len(), 2);
        assert_eq!(
            recovered
                .import_flow
                .categories
                .iter()
                .map(|category| (
                    category.class_id.as_str(),
                    category.target_keypoint_names.as_str()
                ))
                .collect::<Vec<_>>(),
            vec![("person", "nose, left_eye"), ("vehicle", "wheel, axle")]
        );
        assert_eq!(recovered.import_flow.accepted_plan_request, Some(accepted));
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
    }

    #[test]
    fn omit_geometry_emits_no_tasks() {
        let mut app = LabelloApp::default();
        app.import_flow.categories = vec![category("source:3", "3", "person")];
        app.import_flow.geometry_policy = ImportGeometryPolicy::Omit;

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
        app.import_flow.profile = ImportProfile::CocoKeypointsGtV1;
        app.import_flow.categories = vec![category("paired:person:17", "17", "person")];
        app.import_flow.direct_bounding_boxes = true;
        app.import_flow.direct_skeletons = true;

        let both = app.import_plan_request();
        assert_eq!(both.geometry_mappings.len(), 2);
        assert_eq!(both.task_mappings.len(), 2);
        assert_eq!(both.skeleton_mappings.len(), 1);

        app.import_flow.direct_bounding_boxes = false;
        let skeleton_only = app.import_plan_request();
        assert_eq!(skeleton_only.geometry_mappings.len(), 1);
        assert_eq!(skeleton_only.task_mappings.len(), 1);
        assert_eq!(
            skeleton_only.task_mappings[0].task.annotation_type,
            AnnotationType::Skeleton
        );
    }

    #[test]
    fn manual_mapping_uses_only_the_selected_real_category() {
        let mut app = LabelloApp::default();
        app.import_flow.capabilities = Some(ImportCapabilities {
            manual_box_guide_migration: true,
            ..Default::default()
        });
        app.import_flow.geometry_policy = ImportGeometryPolicy::ManualBoxGuideV1;
        app.import_flow.keypoint_names = "nose".to_string();
        let selected = category("release:person:17", "17", "person");
        let mut omitted = category("release:vehicle:91", "91", "vehicle");
        omitted.selected = false;
        app.import_flow.categories = vec![selected, omitted];

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
        app.import_flow.profile = ImportProfile::CocoKeypointsGtV1;
        app.import_flow.transport = ImportTransport::ServerDirectory;
        app.import_flow.descriptors = vec![
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
        app.import_flow.job = Some(ImportJob {
            import_id: labello_domain::ImportId::from("imp-test"),
            owner_user_id: labello_domain::UserId::from("admin"),
            destination_dataset_id: DatasetId::from("imported"),
            destination_name: "Imported".to_string(),
            profile: app.import_flow.profile,
            transport: app.import_flow.transport,
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
        assert!(app.import_flow.error.is_none());
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
        app.import_flow.profile = ImportProfile::UltralyticsYoloDetectV1;
        app.import_flow.transport = ImportTransport::ServerDirectory;
        app.import_flow.descriptors = vec![ImportDescriptorDraft {
            descriptor_file_id: "dataset.yaml".to_string(),
            kind: ImportDescriptorKind::YoloDataset,
            release: "v1".to_string(),
            ..Default::default()
        }];
        app.import_flow.yolo_inspected_descriptor_file_id = Some("dataset.yaml".to_string());
        app.import_flow.yolo_splits = vec![
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
        app.import_flow.job = Some(ImportJob {
            import_id: labello_domain::ImportId::from("imp-yolo"),
            owner_user_id: labello_domain::UserId::from("admin"),
            destination_dataset_id: DatasetId::from("imported-yolo"),
            destination_name: "Imported YOLO".to_string(),
            profile: app.import_flow.profile,
            transport: app.import_flow.transport,
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
    fn capability_normalization_rejects_unadvertised_profile_transport_and_manual_mode() {
        let mut flow = ImportFlowState {
            profile: ImportProfile::CocoKeypointsGtV1,
            transport: ImportTransport::ServerDirectory,
            server_root_id: "missing".to_string(),
            geometry_policy: ImportGeometryPolicy::ManualBoxGuideV1,
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
        assert_eq!(flow.geometry_policy, ImportGeometryPolicy::Direct);
    }
}
