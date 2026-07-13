use std::{future::Future, pin::Pin};

use labello_domain::{
    AdjudicationRecord, Assignment, DatasetId, DatasetMetadata, DatasetSnapshot, DatasetStats,
    EventLogEntry, EventPayload, ImageExplorerPage, ImageId, ImageRecord, ImageState,
    KeybindingSet, OfflineBundle, OfflineSyncRequest, OfflineSyncResult, PrelabelConfig,
    PrelabelSuggestion, ReviewRecord, TaskDefinition, UserAccount, UserId,
};

use crate::{
    AppendEventRequest, AssignNextRequest, AssignmentActionRequest, ClientError, ClientResult,
    CorrectionRequest, CreateDatasetRequest, DatasetSummary, DatasetUser, ImageExplorerQuery,
    ImageFile, ImagePreview, IngestJob, IngestReport, OAuthCallbackRequest, OAuthLoginRequest,
    OfflineBundleRequest, PrelabelSuggestionRequest, SetDatasetRolesRequest,
    UpdateDatasetConfigRequest,
};

pub type ApiFuture<'a, T> = Pin<Box<dyn Future<Output = ClientResult<T>> + 'a>>;

pub trait DatasetApi {
    fn list_datasets<'a>(&'a self) -> ApiFuture<'a, Vec<DatasetSummary>>;
    fn create_dataset<'a>(
        &'a self,
        request: CreateDatasetRequest,
    ) -> ApiFuture<'a, DatasetMetadata>;
    fn get_dataset<'a>(&'a self, dataset_id: &'a DatasetId) -> ApiFuture<'a, DatasetMetadata>;
    fn get_admin_dataset<'a>(&'a self, dataset_id: &'a DatasetId)
    -> ApiFuture<'a, DatasetMetadata>;
    fn update_dataset_config<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: UpdateDatasetConfigRequest,
    ) -> ApiFuture<'a, DatasetMetadata>;
    fn ingest_dataset<'a>(&'a self, dataset_id: &'a DatasetId) -> ApiFuture<'a, IngestReport>;
    fn start_ingest_job<'a>(&'a self, dataset_id: &'a DatasetId) -> ApiFuture<'a, IngestJob>;
    fn get_ingest_job<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        job_id: &'a str,
    ) -> ApiFuture<'a, IngestJob>;
    fn create_snapshot<'a>(&'a self, _dataset_id: &'a DatasetId) -> ApiFuture<'a, DatasetSnapshot> {
        Box::pin(async {
            Err(ClientError::Demo(
                "snapshots are not implemented by this client".to_string(),
            ))
        })
    }
    fn list_snapshots<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
    ) -> ApiFuture<'a, Vec<DatasetSnapshot>> {
        Box::pin(async {
            Err(ClientError::Demo(
                "snapshots are not implemented by this client".to_string(),
            ))
        })
    }
    fn get_snapshot_file<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _snapshot_id: &'a str,
        _path: &'a str,
    ) -> ApiFuture<'a, crate::SnapshotFile> {
        Box::pin(async {
            Err(ClientError::Demo(
                "snapshot downloads are not implemented by this client".to_string(),
            ))
        })
    }
}

pub trait TaskApi {
    fn list_tasks<'a>(&'a self, dataset_id: &'a DatasetId) -> ApiFuture<'a, Vec<TaskDefinition>>;
    fn add_task<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        task: TaskDefinition,
    ) -> ApiFuture<'a, TaskDefinition>;
}

pub trait ImageApi {
    fn list_images<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _query: ImageExplorerQuery,
    ) -> ApiFuture<'a, ImageExplorerPage> {
        Box::pin(async {
            Err(ClientError::Demo(
                "image explorer is not implemented by this client".to_string(),
            ))
        })
    }
    fn assign_next_image<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: AssignNextRequest,
    ) -> ApiFuture<'a, Option<Assignment>>;
    fn release_assignment<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _request: AssignmentActionRequest,
    ) -> ApiFuture<'a, Assignment> {
        Box::pin(async {
            Err(ClientError::Demo(
                "assignment release is not implemented by this client".to_string(),
            ))
        })
    }
    fn complete_assignment<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _request: AssignmentActionRequest,
    ) -> ApiFuture<'a, Assignment> {
        Box::pin(async {
            Err(ClientError::Demo(
                "assignment completion is not implemented by this client".to_string(),
            ))
        })
    }
    fn get_image_state<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> ApiFuture<'a, ImageState>;
    fn get_image_record<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> ApiFuture<'a, ImageRecord>;
    fn get_image_file<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> ApiFuture<'a, ImageFile>;
    fn get_image_preview<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        max_dimension: u32,
    ) -> ApiFuture<'a, ImagePreview>;
    fn rebuild_image<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> ApiFuture<'a, ImageState>;
}

pub trait AnnotationApi {
    fn append_event<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: AppendEventRequest,
    ) -> ApiFuture<'a, EventLogEntry>;

    fn append_payload<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        payload: EventPayload,
    ) -> ApiFuture<'a, EventLogEntry> {
        self.append_event(dataset_id, image_id, AppendEventRequest { payload })
    }

    fn append_assigned_event<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        request: AppendEventRequest,
    ) -> ApiFuture<'a, EventLogEntry> {
        Box::pin(async move {
            self.append_event(dataset_id, &assignment.image_id, request)
                .await
        })
    }
}

pub trait ReviewApi {
    fn record_review<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        review: ReviewRecord,
    ) -> ApiFuture<'a, EventLogEntry>;

    fn record_assigned_review<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        review: ReviewRecord,
    ) -> ApiFuture<'a, EventLogEntry> {
        Box::pin(async move {
            self.record_review(dataset_id, &assignment.image_id, review)
                .await
        })
    }

    fn record_correction<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: CorrectionRequest,
    ) -> ApiFuture<'a, EventLogEntry>;

    fn record_assigned_correction<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        request: CorrectionRequest,
    ) -> ApiFuture<'a, EventLogEntry> {
        Box::pin(async move {
            self.record_correction(dataset_id, &assignment.image_id, request)
                .await
        })
    }
}

pub trait AdjudicationApi {
    fn record_adjudication<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        adjudication: AdjudicationRecord,
    ) -> ApiFuture<'a, EventLogEntry>;

    fn record_assigned_adjudication<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        adjudication: AdjudicationRecord,
    ) -> ApiFuture<'a, EventLogEntry> {
        Box::pin(async move {
            self.record_adjudication(dataset_id, &assignment.image_id, adjudication)
                .await
        })
    }
}

pub trait OfflineApi {
    fn offline_bundle<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: OfflineBundleRequest,
    ) -> ApiFuture<'a, OfflineBundle>;

    fn sync_offline_events<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: OfflineSyncRequest,
    ) -> ApiFuture<'a, OfflineSyncResult>;
}

pub trait StatsApi {
    fn dataset_stats<'a>(&'a self, dataset_id: &'a DatasetId) -> ApiFuture<'a, DatasetStats>;
}

pub trait KeybindingApi {
    fn get_keybindings<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        user_id: &'a UserId,
    ) -> ApiFuture<'a, KeybindingSet>;

    fn save_keybindings<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        keybindings: KeybindingSet,
    ) -> ApiFuture<'a, KeybindingSet>;
}

pub trait PrelabelApi {
    fn list_prelabel_configs<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> ApiFuture<'a, Vec<PrelabelConfig>>;

    fn add_prelabel_config<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        config: PrelabelConfig,
    ) -> ApiFuture<'a, PrelabelConfig>;

    fn prelabel_suggestions<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: PrelabelSuggestionRequest,
    ) -> ApiFuture<'a, Vec<PrelabelSuggestion>>;
}

pub trait AuthApi {
    fn github_login_url<'a>(&'a self, request: OAuthLoginRequest) -> ApiFuture<'a, String>;
    fn github_callback<'a>(&'a self, request: OAuthCallbackRequest) -> ApiFuture<'a, UserAccount>;
    fn me<'a>(&'a self) -> ApiFuture<'a, UserAccount> {
        Box::pin(async {
            Err(ClientError::Demo(
                "current session lookup is not implemented by this client".to_string(),
            ))
        })
    }
    fn logout<'a>(&'a self) -> ApiFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

pub trait UserApi {
    fn list_dataset_users<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> ApiFuture<'a, Vec<DatasetUser>>;
    fn set_dataset_roles<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: SetDatasetRolesRequest,
    ) -> ApiFuture<'a, DatasetUser>;
}

pub trait LabelloApi:
    DatasetApi
    + TaskApi
    + ImageApi
    + AnnotationApi
    + ReviewApi
    + AdjudicationApi
    + OfflineApi
    + StatsApi
    + KeybindingApi
    + PrelabelApi
    + AuthApi
    + UserApi
{
}

impl<T> LabelloApi for T where
    T: DatasetApi
        + TaskApi
        + ImageApi
        + AnnotationApi
        + ReviewApi
        + AdjudicationApi
        + OfflineApi
        + StatsApi
        + KeybindingApi
        + PrelabelApi
        + AuthApi
        + UserApi
{
}
