use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use labello_domain::{
    AdjudicationRecord, Assignment, DatasetId, DatasetMetadata, DatasetStats, EventLogEntry,
    EventPayload, ImageId, ImageState, KeybindingSet, OfflineBundle, OfflineSyncRequest,
    OfflineSyncResult, PrelabelConfig, PrelabelSuggestion, ReviewRecord, TaskDefinition,
    UserAccount, UserId,
};

use crate::{
    AdjudicationApi, AnnotationApi, AppendEventRequest, AssignNextRequest, AuthApi, ClientError,
    CorrectionRequest, CreateDatasetRequest, DatasetApi, DatasetSummary, ImageApi, ImageFile,
    IngestReport, KeybindingApi, OAuthCallbackRequest, OAuthLoginRequest, OfflineApi,
    OfflineBundleRequest, PrelabelApi, PrelabelSuggestionRequest, ReviewApi, StatsApi, TaskApi,
    UpdateDatasetConfigRequest,
};

#[derive(Clone, Default)]
pub struct DemoLabelloApi {
    state: Rc<RefCell<DemoState>>,
}

#[derive(Default)]
struct DemoState {
    datasets: BTreeMap<DatasetId, DatasetMetadata>,
    keybindings: BTreeMap<UserId, KeybindingSet>,
}

impl DemoLabelloApi {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_dataset(self, metadata: DatasetMetadata) -> Self {
        self.state
            .borrow_mut()
            .datasets
            .insert(metadata.dataset_id.clone(), metadata);
        self
    }

    fn dataset(&self, dataset_id: &DatasetId) -> crate::ClientResult<DatasetMetadata> {
        self.state
            .borrow()
            .datasets
            .get(dataset_id)
            .cloned()
            .ok_or_else(|| ClientError::Demo(format!("dataset {dataset_id} does not exist")))
    }
}

impl DatasetApi for DemoLabelloApi {
    fn list_datasets<'a>(&'a self) -> crate::ApiFuture<'a, Vec<DatasetSummary>> {
        Box::pin(async move {
            Ok(self
                .state
                .borrow()
                .datasets
                .values()
                .map(|metadata| DatasetSummary {
                    dataset_id: metadata.dataset_id.clone(),
                    name: metadata.name.clone(),
                    roles: Vec::new(),
                    total_images: metadata.images.len(),
                })
                .collect())
        })
    }

    fn create_dataset<'a>(
        &'a self,
        request: CreateDatasetRequest,
    ) -> crate::ApiFuture<'a, DatasetMetadata> {
        Box::pin(async move {
            let metadata = DatasetMetadata::new(
                request.dataset_id.clone(),
                request.name,
                labello_domain::now(),
            );
            self.state
                .borrow_mut()
                .datasets
                .insert(request.dataset_id, metadata.clone());
            Ok(metadata)
        })
    }

    fn get_dataset<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, DatasetMetadata> {
        Box::pin(async move { self.dataset(dataset_id) })
    }

    fn get_admin_dataset<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, DatasetMetadata> {
        self.get_dataset(dataset_id)
    }

    fn update_dataset_config<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: UpdateDatasetConfigRequest,
    ) -> crate::ApiFuture<'a, DatasetMetadata> {
        Box::pin(async move {
            let mut state = self.state.borrow_mut();
            let metadata = state
                .datasets
                .get_mut(dataset_id)
                .ok_or_else(|| ClientError::Demo(format!("dataset {dataset_id} does not exist")))?;
            metadata.name = request.name;
            metadata.image_roots = request.image_roots;
            metadata.label_classes = request.label_classes;
            metadata.tasks = request.tasks;
            metadata.role_assignments = request.role_assignments;
            metadata.imbalance = request.imbalance;
            metadata.prelabel_configs = request.prelabel_configs;
            metadata.updated_at = labello_domain::now();
            Ok(metadata.clone())
        })
    }

    fn ingest_dataset<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, IngestReport> {
        Box::pin(async move { Ok(IngestReport::default()) })
    }
}

impl TaskApi for DemoLabelloApi {
    fn list_tasks<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, Vec<TaskDefinition>> {
        Box::pin(async move { Ok(self.dataset(dataset_id)?.tasks) })
    }

    fn add_task<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        task: TaskDefinition,
    ) -> crate::ApiFuture<'a, TaskDefinition> {
        Box::pin(async move {
            let mut state = self.state.borrow_mut();
            let dataset = state
                .datasets
                .get_mut(dataset_id)
                .ok_or_else(|| ClientError::Demo(format!("dataset {dataset_id} does not exist")))?;
            dataset
                .tasks
                .retain(|existing| existing.task_id != task.task_id);
            dataset.tasks.push(task.clone());
            Ok(task)
        })
    }
}

impl ImageApi for DemoLabelloApi {
    fn assign_next_image<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _request: AssignNextRequest,
    ) -> crate::ApiFuture<'a, Option<Assignment>> {
        Box::pin(async move { Ok(None) })
    }

    fn get_image_state<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> crate::ApiFuture<'a, ImageState> {
        Box::pin(async move { Ok(ImageState::new(image_id.clone())) })
    }

    fn get_image_file<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> crate::ApiFuture<'a, ImageFile> {
        Box::pin(async move {
            Ok(ImageFile {
                image_id: image_id.clone(),
                media_type: "application/octet-stream".to_string(),
                bytes: Vec::new(),
            })
        })
    }

    fn rebuild_image<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> crate::ApiFuture<'a, ImageState> {
        self.get_image_state(dataset_id, image_id)
    }
}

impl AnnotationApi for DemoLabelloApi {
    fn append_event<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: AppendEventRequest,
    ) -> crate::ApiFuture<'a, EventLogEntry> {
        Box::pin(async move {
            Ok(EventLogEntry::new(
                1,
                image_id.clone(),
                UserId::from("demo_user"),
                labello_domain::DatasetRole::Annotator,
                labello_domain::now(),
                request.payload,
            ))
        })
    }
}

impl ReviewApi for DemoLabelloApi {
    fn record_review<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        review: ReviewRecord,
    ) -> crate::ApiFuture<'a, EventLogEntry> {
        self.append_payload(
            dataset_id,
            image_id,
            EventPayload::ReviewRecorded { review },
        )
    }

    fn record_correction<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        request: CorrectionRequest,
    ) -> crate::ApiFuture<'a, EventLogEntry> {
        self.append_payload(
            dataset_id,
            image_id,
            EventPayload::AnnotationVersionCreated {
                annotation: request.annotation,
                previous_version: Some(request.previous_version),
                reason: request.reason,
            },
        )
    }
}

impl AdjudicationApi for DemoLabelloApi {
    fn record_adjudication<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        adjudication: AdjudicationRecord,
    ) -> crate::ApiFuture<'a, EventLogEntry> {
        self.append_payload(
            dataset_id,
            image_id,
            EventPayload::AdjudicationRecorded { adjudication },
        )
    }
}

impl OfflineApi for DemoLabelloApi {
    fn offline_bundle<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        _request: OfflineBundleRequest,
    ) -> crate::ApiFuture<'a, OfflineBundle> {
        Box::pin(async move {
            let dataset = self.dataset(dataset_id)?;
            Ok(OfflineBundle {
                schema_version: labello_domain::SCHEMA_VERSION,
                dataset_id: dataset.dataset_id,
                user_id: UserId::from("demo_user"),
                created_at: labello_domain::now(),
                expires_at: None,
                roles: Vec::new(),
                tasks: dataset.tasks,
                images: Vec::new(),
            })
        })
    }

    fn sync_offline_events<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _request: OfflineSyncRequest,
    ) -> crate::ApiFuture<'a, OfflineSyncResult> {
        Box::pin(async move {
            Ok(OfflineSyncResult {
                merged_events: 0,
                conflicts: Vec::new(),
            })
        })
    }
}

impl StatsApi for DemoLabelloApi {
    fn dataset_stats<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, DatasetStats> {
        Box::pin(async move { Ok(DatasetStats::default()) })
    }
}

impl KeybindingApi for DemoLabelloApi {
    fn get_keybindings<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        user_id: &'a UserId,
    ) -> crate::ApiFuture<'a, KeybindingSet> {
        Box::pin(async move {
            Ok(self
                .state
                .borrow()
                .keybindings
                .get(user_id)
                .cloned()
                .unwrap_or_else(|| KeybindingSet::defaults_for(user_id.clone())))
        })
    }

    fn save_keybindings<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        keybindings: KeybindingSet,
    ) -> crate::ApiFuture<'a, KeybindingSet> {
        Box::pin(async move {
            keybindings
                .validate_conflicts()
                .map_err(|error| ClientError::Demo(error.to_string()))?;
            self.state
                .borrow_mut()
                .keybindings
                .insert(keybindings.user_id.clone(), keybindings.clone());
            Ok(keybindings)
        })
    }
}

impl PrelabelApi for DemoLabelloApi {
    fn list_prelabel_configs<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, Vec<PrelabelConfig>> {
        Box::pin(async move { Ok(self.dataset(dataset_id)?.prelabel_configs) })
    }

    fn add_prelabel_config<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        config: PrelabelConfig,
    ) -> crate::ApiFuture<'a, PrelabelConfig> {
        Box::pin(async move {
            let mut state = self.state.borrow_mut();
            let dataset = state
                .datasets
                .get_mut(dataset_id)
                .ok_or_else(|| ClientError::Demo(format!("dataset {dataset_id} does not exist")))?;
            dataset
                .prelabel_configs
                .retain(|existing| existing.config_id != config.config_id);
            dataset.prelabel_configs.push(config.clone());
            Ok(config)
        })
    }

    fn prelabel_suggestions<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _request: PrelabelSuggestionRequest,
    ) -> crate::ApiFuture<'a, Vec<PrelabelSuggestion>> {
        Box::pin(async move { Ok(Vec::new()) })
    }
}

impl AuthApi for DemoLabelloApi {
    fn github_login_url<'a>(&'a self, _request: OAuthLoginRequest) -> crate::ApiFuture<'a, String> {
        Box::pin(async move { Ok("https://github.com/login/oauth/authorize".to_string()) })
    }

    fn github_callback<'a>(
        &'a self,
        _request: OAuthCallbackRequest,
    ) -> crate::ApiFuture<'a, UserAccount> {
        Box::pin(async move {
            let timestamp = labello_domain::now();
            Ok(UserAccount {
                user_id: UserId::from("demo_user"),
                display_name: "Demo User".to_string(),
                github_user_id: None,
                github_login: None,
                created_at: timestamp,
                updated_at: timestamp,
            })
        })
    }
}
