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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportMappingIssueSeverity {
    Error,
    Warning,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportMappingField {
    Form,
    CategorySelection,
    ClassId,
    ClassName,
    ClassColor,
    BoundingBoxTaskId,
    BoundingBoxTaskName,
    SkeletonTaskId,
    SkeletonTaskName,
    Geometry(ImportGeometryKind),
    TargetKeypointNames,
    WorkflowIntent,
    SeedConfirmation,
    Compatibility(ImportCompatibilityField),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportCompatibilityField {
    YoloMissingLabels,
    YoloDuplicateRows,
    CocoCrowds,
    CocoStructure,
    GeometryBounds,
    CrossSplitDuplicates,
    MissingKeypointNames,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ImportMappingIssue {
    severity: ImportMappingIssueSeverity,
    category_index: Option<usize>,
    field: ImportMappingField,
    message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ImportMappingValidation {
    issues: Vec<ImportMappingIssue>,
}

impl ImportMappingValidation {
    fn error_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|issue| issue.severity == ImportMappingIssueSeverity::Error)
            .count()
    }

    fn warning_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|issue| issue.severity == ImportMappingIssueSeverity::Warning)
            .count()
    }

    fn is_valid(&self) -> bool {
        self.error_count() == 0
    }

    fn for_field(
        &self,
        category_index: Option<usize>,
        field: ImportMappingField,
    ) -> impl Iterator<Item = &ImportMappingIssue> {
        self.issues
            .iter()
            .filter(move |issue| issue.category_index == category_index && issue.field == field)
    }

    fn category_counts(&self, category_index: usize) -> (usize, usize) {
        self.issues
            .iter()
            .filter(|issue| issue.category_index == Some(category_index))
            .fold((0, 0), |(errors, warnings), issue| match issue.severity {
                ImportMappingIssueSeverity::Error => (errors + 1, warnings),
                ImportMappingIssueSeverity::Warning => (errors, warnings + 1),
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ImportGeometryChoice {
    source: ImportGeometryKind,
    policy: ImportGeometryPolicy,
    label: &'static str,
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
    pub yolo_missing_labels: labello_client::YoloMissingLabelPolicy,
    pub yolo_duplicate_rows: labello_client::YoloDuplicateRowPolicy,
    pub coco_crowds: labello_client::CocoCrowdPolicy,
    pub coco_structure: labello_client::CocoStructurePolicy,
    pub geometry_bounds: labello_client::GeometryBoundsPolicy,
    pub cross_split_duplicates: labello_client::CrossSplitDuplicatePolicy,
    pub missing_keypoint_names: labello_client::MissingKeypointNamesPolicy,
    pub seed_workflow_confirmed: bool,
    pub seed_workflow_confirmation_scope: Option<String>,
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
            yolo_missing_labels: Default::default(),
            yolo_duplicate_rows: Default::default(),
            coco_crowds: Default::default(),
            coco_structure: Default::default(),
            geometry_bounds: Default::default(),
            cross_split_duplicates: Default::default(),
            missing_keypoint_names: Default::default(),
            seed_workflow_confirmed: false,
            seed_workflow_confirmation_scope: None,
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
        if self.profile != previous_profile {
            self.descriptors = vec![descriptor_draft(self.profile)];
            self.invalidate_yolo_inspection();
            self.categories.clear();
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
            self.seed_workflow_confirmed = request.task_mappings.iter().any(|mapping| {
                mapping.workflow_intent == ImportWorkflowIntent::SeedFutureAnnotation
            });
        }
        self.accepted_plan_request = accepted;
        self.plan = Some(plan.clone());
        self.normalize_mapping_draft();
        self.seed_workflow_confirmation_scope = self.seed_workflow_scope();
    }

    pub(crate) fn invalidate_yolo_inspection(&mut self) {
        self.yolo_splits.clear();
        self.yolo_inspection_loading = false;
        self.yolo_inspection_error = None;
        self.yolo_inspected_descriptor_file_id = None;
        self.pending_yolo_inspection_request_id = None;
        self.yolo_inspection_retry_after_current = false;
    }

    pub(crate) fn normalize_mapping_draft(&mut self) {
        for category in &mut self.categories {
            if !category.geometry_mappings.is_empty() {
                continue;
            }
            let key = category.source_category_key.clone();
            category.geometry_mappings = category
                .direct_geometry
                .iter()
                .map(|geometry| ImportGeometryMappingRequest {
                    source_category_key: key.clone(),
                    source_geometry: *geometry,
                    target_geometry: *geometry,
                    policy: ImportGeometryPolicy::Direct,
                    parameters: Vec::new(),
                })
                .collect();
        }
    }

    fn sync_seed_workflow_confirmation_scope(&mut self) {
        let scope = self.seed_workflow_scope();
        if self.seed_workflow_confirmation_scope != scope {
            self.seed_workflow_confirmation_scope = scope;
            self.seed_workflow_confirmed = false;
        }
    }

    fn seed_workflow_scope(&self) -> Option<String> {
        let values = self
            .categories
            .iter()
            .filter(|category| {
                category.selected
                    && category.workflow_intent == ImportWorkflowIntent::SeedFutureAnnotation
            })
            .map(|category| {
                let outputs = category
                    .geometry_mappings
                    .iter()
                    .filter(|mapping| mapping.policy != ImportGeometryPolicy::Omit)
                    .map(|mapping| {
                        format!(
                            "{:?}:{:?}:{:?}",
                            mapping.source_geometry, mapping.target_geometry, mapping.policy
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                format!("{}={outputs}", category.source_category_key)
            })
            .collect::<Vec<_>>();
        (!values.is_empty()).then(|| values.join("|"))
    }
}
