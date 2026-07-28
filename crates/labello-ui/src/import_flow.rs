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

include!("import_flow/state.rs");
include!("import_flow/orchestration.rs");
include!("import_flow/views.rs");
include!("import_flow/request_mapping.rs");
include!("import_flow/validation.rs");
include!("import_flow/request_support.rs");
include!("import_flow/view_support.rs");
include!("import_flow/validation_support.rs");
include!("import_flow/browser_upload.rs");
include!("import_flow/tests.rs");
