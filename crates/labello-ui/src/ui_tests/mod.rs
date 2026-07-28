use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};
#[cfg(not(target_arch = "wasm32"))]
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

use eframe::egui;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use labello_client::{
    AdjudicationApi, AnnotationApi, AnnotationBatchRequest, ApiFuture, AppendEventRequest,
    AssignNextRequest, AssignmentActionRequest, AuthApi, AuthOptions, ClientError, ClientResult,
    CorrectionRequest, CreateDatasetRequest, DatasetApi, DatasetSummary, DatasetUser, ImageApi,
    ImageExplorerQuery, ImageFile, ImagePreview, ImportApi, IngestJob, IngestJobStatus,
    IngestReport, KeybindingApi, OAuthCallbackRequest, OAuthLoginRequest, OfflineApi,
    OfflineBundleRequest, PrelabelApi, PrelabelSuggestionRequest, ReviewApi, SessionInfo,
    SetDatasetRolesRequest, SnapshotFile, StatsApi, TaskApi, UpdateDatasetConfigRequest, UserApi,
};
use labello_domain::{
    AdjudicationRecord, AnnotationGeometry, AnnotationOrigin, AnnotationType, Assignment,
    AssignmentId, AssignmentKind, AssignmentStatus, BoundingBox, BrowserAcceleration, ClassId,
    DatasetId, DatasetMetadata, DatasetRole, DatasetRoleAssignment, DatasetSnapshot, DatasetStats,
    EventId, EventLogEntry, EventPayload, HumanRevisionKind, ImageExplorerItem, ImageExplorerPage,
    ImageId, ImageRecord, ImageState, ImportId, KeybindingSet, KeypointAnnotation, KeypointSpec,
    KeypointState, LabelClass, MigrationDispositionStatus, MigrationExclusion, ModelSpec,
    NormalizedPoint, OfflineBundle, OfflineSyncRequest, OfflineSyncResult, OutputProcessing,
    PrelabelConfig, PrelabelConfigId, PrelabelExecution, PrelabelSuggestion, ReviewConfig,
    ReviewId, ReviewRecord, ReviewTarget, RevisionSource, SCHEMA_VERSION, SkeletonGeometry,
    SkeletonSpec, SnapshotFileEntry, TaskDefinition, TaskId, TaskStatus, TutorialContent,
    UserAccount, UserId,
};
use web_time::{Duration, Instant};

mod support;

use support::*;

use crate::app::{
    AdminSection, AppConfig, AppView, Drawer, FolderUploadProgress, IMAGE_QUEUE_SIZE, LabelloApp,
    LayoutMode, RequestIdentity, SaveStatus, SetupSection, UiCommand, UiMessage,
};
use crate::canvas::BoundingBoxEdit;
use crate::persistence::{
    StoredAssignmentAvailability, StoredCanvasTransform, StoredView, WorkspacePreference,
};
use crate::theme;

#[cfg(not(target_arch = "wasm32"))]
fn poll_ready_task(mut future: Pin<Box<dyn Future<Output = ()> + 'static>>) {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    assert!(matches!(
        future.as_mut().poll(&mut context),
        Poll::Ready(())
    ));
}

// Keep feature suites in this module so every test exercises the same assembled harness.
include!("suites/setup.rs");
include!("suites/admin.rs");
include!("suites/workspace.rs");
include!("suites/import.rs");
include!("suites/migration.rs");
include!("suites/persistence.rs");
include!("suites/accessibility.rs");
include!("suites/responsive.rs");
