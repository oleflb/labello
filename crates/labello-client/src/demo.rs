use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use labello_domain::{
    AdjudicationRecord, Assignment, DatasetId, DatasetMetadata, DatasetStats, EventLogEntry,
    EventPayload, ImageId, ImageRecord, ImageState, KeybindingSet, OfflineBundle,
    OfflineSyncRequest, OfflineSyncResult, PrelabelConfig, PrelabelSuggestion, ReviewRecord,
    TaskDefinition, UserAccount, UserId,
};

use crate::{
    AdjudicationApi, AnnotationApi, AnnotationBatchRequest, AppendEventRequest, AssignNextRequest,
    AssignmentActionRequest, AuthApi, AuthOptions, ClientError, CorrectionRequest,
    CreateDatasetRequest, DatasetApi, DatasetSummary, DatasetUser, ImageApi, ImageFile,
    ImagePreview, IngestJob, IngestJobStatus, IngestReport, KeybindingApi, OAuthCallbackRequest,
    OAuthLoginRequest, OfflineApi, OfflineBundleRequest, PrelabelApi, PrelabelSuggestionRequest,
    ReviewApi, SessionInfo, SetDatasetRolesRequest, StatsApi, TaskApi, UpdateDatasetConfigRequest,
    UserApi,
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

    fn start_ingest_job<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, IngestJob> {
        Box::pin(async move {
            Ok(IngestJob {
                job_id: "demo-ingest".to_string(),
                dataset_id: dataset_id.clone(),
                status: IngestJobStatus::Completed,
                report: Some(IngestReport::default()),
                error: None,
            })
        })
    }

    fn get_ingest_job<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        job_id: &'a str,
    ) -> crate::ApiFuture<'a, IngestJob> {
        Box::pin(async move {
            Ok(IngestJob {
                job_id: job_id.to_string(),
                dataset_id: dataset_id.clone(),
                status: IngestJobStatus::Completed,
                report: Some(IngestReport::default()),
                error: None,
            })
        })
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

    fn release_assignment<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _request: AssignmentActionRequest,
    ) -> crate::ApiFuture<'a, Assignment> {
        Box::pin(async move {
            Err(ClientError::Demo(
                "the demo backend does not create assignments".to_string(),
            ))
        })
    }

    fn complete_assignment<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _request: AssignmentActionRequest,
    ) -> crate::ApiFuture<'a, Assignment> {
        Box::pin(async move {
            Err(ClientError::Demo(
                "the demo backend does not create assignments".to_string(),
            ))
        })
    }

    fn reopen_assignment<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _request: AssignmentActionRequest,
    ) -> crate::ApiFuture<'a, Assignment> {
        Box::pin(async move {
            Err(ClientError::Demo(
                "the demo backend does not create assignments".to_string(),
            ))
        })
    }

    fn get_image_state<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> crate::ApiFuture<'a, ImageState> {
        Box::pin(async move { Ok(ImageState::new(image_id.clone())) })
    }

    fn get_image_record<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
    ) -> crate::ApiFuture<'a, ImageRecord> {
        Box::pin(async move {
            Ok(ImageRecord {
                image_id: image_id.clone(),
                blake3: image_id.to_string(),
                canonical_path: format!("images/{image_id}.png"),
                known_paths: vec![],
                duplicate_paths: vec![],
                file_name: format!("{image_id}.png"),
                byte_size: 4,
                width: 1,
                height: 1,
                media_type: "image/png".to_string(),
            })
        })
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

    fn get_image_preview<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        _max_dimension: u32,
    ) -> crate::ApiFuture<'a, ImagePreview> {
        Box::pin(async move {
            Ok(ImagePreview {
                image_id: image_id.clone(),
                width: 1,
                height: 1,
                rgba: vec![18, 23, 34, 255],
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

    fn apply_annotation_batch<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        assignment: AssignmentActionRequest,
        request: AnnotationBatchRequest,
    ) -> crate::ApiFuture<'a, ImageState> {
        Box::pin(async move {
            let mut state = ImageState::new(assignment.image_id.clone());
            for payload in request.payloads {
                let event = EventLogEntry::new(
                    state.current_sequence + 1,
                    assignment.image_id.clone(),
                    UserId::from("demo_user"),
                    labello_domain::DatasetRole::Annotator,
                    labello_domain::now(),
                    payload,
                );
                state
                    .apply_event(&event)
                    .map_err(|error| ClientError::Demo(error.to_string()))?;
            }
            Ok(state)
        })
    }
}

impl ReviewApi for DemoLabelloApi {
    fn record_review<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        image_id: &'a ImageId,
        review: ReviewRecord,
    ) -> crate::ApiFuture<'a, ImageState> {
        let _ = dataset_id;
        Box::pin(async move {
            let mut state = ImageState::new(image_id.clone());
            let event = EventLogEntry::new(
                1,
                image_id.clone(),
                UserId::from("demo_user"),
                labello_domain::DatasetRole::Reviewer,
                labello_domain::now(),
                EventPayload::ReviewRecorded { review },
            );
            state
                .apply_event(&event)
                .map_err(|error| ClientError::Demo(error.to_string()))?;
            Ok(state)
        })
    }

    fn record_correction<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        _image_id: &'a ImageId,
        _request: CorrectionRequest,
    ) -> crate::ApiFuture<'a, EventLogEntry> {
        Box::pin(async move {
            Err(ClientError::Demo(
                "the demo backend does not create review assignments".to_string(),
            ))
        })
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
            let mut keybindings = self
                .state
                .borrow()
                .keybindings
                .get(user_id)
                .cloned()
                .unwrap_or_else(|| KeybindingSet::defaults_for(user_id.clone()));
            keybindings.normalize();
            Ok(keybindings)
        })
    }

    fn save_keybindings<'a>(
        &'a self,
        _dataset_id: &'a DatasetId,
        mut keybindings: KeybindingSet,
    ) -> crate::ApiFuture<'a, KeybindingSet> {
        Box::pin(async move {
            keybindings
                .validate()
                .map_err(|error| ClientError::Demo(error.to_string()))?;
            keybindings.normalize();
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
    fn auth_options<'a>(&'a self) -> crate::ApiFuture<'a, AuthOptions> {
        Box::pin(async move {
            Ok(AuthOptions {
                github_oauth: true,
                local_admin_login: false,
            })
        })
    }

    fn local_admin_login<'a>(&'a self) -> crate::ApiFuture<'a, SessionInfo> {
        Box::pin(async move {
            Err(ClientError::Api {
                status: 401,
                message: "local administrator login is not available in demo mode".to_string(),
            })
        })
    }

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

    fn me<'a>(&'a self) -> crate::ApiFuture<'a, SessionInfo> {
        Box::pin(async move {
            let account = self
                .github_callback(OAuthCallbackRequest {
                    code: String::new(),
                    state: String::new(),
                })
                .await?;
            Ok(SessionInfo {
                account,
                can_create_datasets: true,
            })
        })
    }

    fn logout<'a>(&'a self) -> crate::ApiFuture<'a, ()> {
        Box::pin(async move { Ok(()) })
    }
}

impl UserApi for DemoLabelloApi {
    fn list_dataset_users<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
    ) -> crate::ApiFuture<'a, Vec<DatasetUser>> {
        Box::pin(async move {
            let metadata = self.dataset(dataset_id)?;
            Ok(metadata
                .role_assignments
                .into_iter()
                .map(|assignment| DatasetUser {
                    account: UserAccount {
                        user_id: assignment.user_id.clone(),
                        display_name: assignment.user_id.to_string(),
                        github_user_id: None,
                        github_login: None,
                        created_at: assignment.assigned_at,
                        updated_at: assignment.assigned_at,
                    },
                    roles: assignment.roles.into_iter().collect(),
                })
                .collect())
        })
    }

    fn set_dataset_roles<'a>(
        &'a self,
        dataset_id: &'a DatasetId,
        request: SetDatasetRolesRequest,
    ) -> crate::ApiFuture<'a, DatasetUser> {
        Box::pin(async move {
            let mut state = self.state.borrow_mut();
            let metadata = state
                .datasets
                .get_mut(dataset_id)
                .ok_or_else(|| ClientError::Demo(format!("dataset {dataset_id} does not exist")))?;
            metadata
                .role_assignments
                .retain(|assignment| assignment.user_id != request.user_id);
            if !request.roles.is_empty() {
                metadata
                    .role_assignments
                    .push(labello_domain::DatasetRoleAssignment {
                        dataset_id: dataset_id.clone(),
                        user_id: request.user_id.clone(),
                        roles: request.roles.iter().cloned().collect(),
                        assigned_at: labello_domain::now(),
                        assigned_by: Some(UserId::from("demo_user")),
                    });
            }
            let timestamp = labello_domain::now();
            Ok(DatasetUser {
                account: UserAccount {
                    user_id: request.user_id.clone(),
                    display_name: request.user_id.to_string(),
                    github_user_id: None,
                    github_login: None,
                    created_at: timestamp,
                    updated_at: timestamp,
                },
                roles: request.roles,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_admin_login_is_not_available() {
        let api = DemoLabelloApi::new();

        assert_eq!(
            api.auth_options().await.unwrap(),
            AuthOptions {
                github_oauth: true,
                local_admin_login: false,
            }
        );
        assert!(matches!(
            api.local_admin_login().await,
            Err(ClientError::Api { status: 401, .. })
        ));
    }
}
