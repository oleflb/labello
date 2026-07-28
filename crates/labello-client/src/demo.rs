use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use labello_domain::{
    AdjudicationRecord, Assignment, DatasetId, DatasetMetadata, DatasetStats, EventLogEntry,
    EventPayload, ImageId, ImageRecord, ImageState, ImportId, KeybindingSet, OfflineBundle,
    OfflineSyncRequest, OfflineSyncResult, PrelabelConfig, PrelabelSuggestion, ReviewRecord,
    TaskDefinition, UserAccount, UserId,
};

use crate::{
    AdjudicationApi, AnnotationApi, AnnotationBatchRequest, AppendEventRequest, AssignNextRequest,
    AssignmentActionRequest, AuthApi, AuthOptions, ClientError, CorrectionRequest,
    CreateDatasetRequest, DatasetApi, DatasetSummary, DatasetUser, ImageApi, ImageFile,
    ImagePreview, ImportApi, IngestJob, IngestJobStatus, IngestReport, KeybindingApi,
    OAuthCallbackRequest, OAuthLoginRequest, OfflineApi, OfflineBundleRequest, PrelabelApi,
    PrelabelSuggestionRequest, ReviewApi, SessionInfo, SetDatasetRolesRequest, StatsApi, TaskApi,
    UpdateDatasetConfigRequest, UserApi,
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

include!("demo/datasets.rs");
include!("demo/imports.rs");
include!("demo/workflow.rs");
include!("demo/administration.rs");
include!("demo/auth.rs");

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

    #[tokio::test]
    async fn dataset_import_is_capability_gated_and_unavailable() {
        let api = DemoLabelloApi::new();

        let capabilities = api.import_capabilities().await.unwrap();
        assert!(!capabilities.available);
        assert!(capabilities.unavailable_reason.is_some());
        assert!(matches!(
            api.get_import(&ImportId::from("imp_1")).await,
            Err(ClientError::Demo(message)) if message.contains("unavailable")
        ));
    }
}
