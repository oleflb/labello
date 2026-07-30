use std::{future::Future, pin::Pin};

use labello_domain::{
    AdjudicationRecord, Assignment, DatasetId, DatasetMetadata, DatasetSnapshot, DatasetStats,
    EventLogEntry, EventPayload, ImageExplorerPage, ImageId, ImageRecord, ImageState, ImportId,
    KeybindingSet, OfflineBundle, OfflineSyncRequest, OfflineSyncResult, PrelabelConfig,
    PrelabelSuggestion, ReviewRecord, TaskDefinition, UserAccount, UserId,
};

use crate::{
    AnnotationBatchRequest, AppendEventRequest, AssignNextRequest, AssignmentActionRequest,
    AssignmentAvailability, AssignmentAvailabilityRequest, AssignmentRevalidation, AuthOptions,
    ClientError, ClientResult, CorrectionRequest, CreateDatasetRequest, DatasetSummary,
    DatasetUser, ImageExplorerQuery, ImageFile, ImagePreview, IngestJob, IngestReport,
    OAuthCallbackRequest, OAuthLoginRequest, OfflineBundleRequest, PrelabelSuggestionRequest,
    SessionInfo, SetDatasetRolesRequest, UpdateDatasetConfigRequest,
};

pub type ApiFuture<'a, T> = Pin<Box<dyn Future<Output = ClientResult<T>> + 'a>>;

include!("traits/datasets_imports.rs");
include!("traits/workflow.rs");
include!("traits/administration.rs");
include!("traits/facade.rs");
