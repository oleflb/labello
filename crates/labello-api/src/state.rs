use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use labello_client::IngestJob;
use labello_domain::{DatasetId, IdValidationError, UserId};
use labello_storage::DatasetRepository;
use tokio::sync::RwLock;

use crate::{GithubOAuthConfig, error::ApiResult, session::ServerStore};

#[derive(Clone)]
pub struct ApiState {
    datasets_root: Arc<PathBuf>,
    bootstrap_admins: Arc<BTreeSet<UserId>>,
    dev_auth_token: Option<Arc<String>>,
    dev_auth_enabled: bool,
    allowed_origins: Arc<Vec<String>>,
    session_cookie_secure: bool,
    pub(crate) server_store: ServerStore,
    ingest_jobs: Arc<RwLock<BTreeMap<String, IngestJob>>>,
    repositories: Arc<Mutex<BTreeMap<DatasetId, Arc<DatasetRepository>>>>,
    pub github_oauth: Option<GithubOAuthConfig>,
    pub http: reqwest::Client,
}

impl ApiState {
    pub fn new(datasets_root: impl Into<PathBuf>) -> Self {
        let datasets_root = datasets_root.into();
        Self {
            server_store: ServerStore::new(&datasets_root),
            datasets_root: Arc::new(datasets_root),
            bootstrap_admins: Arc::new(BTreeSet::from([UserId::from("admin")])),
            dev_auth_token: None,
            dev_auth_enabled: cfg!(test),
            allowed_origins: Arc::new(Vec::new()),
            session_cookie_secure: true,
            ingest_jobs: Arc::new(RwLock::new(BTreeMap::new())),
            repositories: Arc::new(Mutex::new(BTreeMap::new())),
            github_oauth: None,
            http: reqwest::Client::new(),
        }
    }

    pub fn datasets_root(&self) -> &std::path::Path {
        &self.datasets_root
    }

    pub fn with_bootstrap_admins(mut self, admins: impl IntoIterator<Item = UserId>) -> Self {
        self.bootstrap_admins = Arc::new(admins.into_iter().collect());
        self
    }

    pub fn with_dev_auth_token(mut self, token: Option<String>) -> Self {
        self.dev_auth_enabled = token.is_some();
        self.dev_auth_token = token.filter(|token| !token.is_empty()).map(Arc::new);
        self
    }

    pub fn dev_auth_enabled(&self) -> bool {
        self.dev_auth_enabled
    }

    pub fn dev_auth_token(&self) -> Option<&str> {
        self.dev_auth_token.as_deref().map(String::as_str)
    }

    pub fn is_bootstrap_admin(&self, user_id: &UserId) -> bool {
        self.bootstrap_admins.contains(user_id)
    }

    pub fn with_allowed_origins(mut self, origins: Vec<String>) -> Self {
        self.allowed_origins = Arc::new(origins);
        self
    }

    pub fn allowed_origins(&self) -> &[String] {
        &self.allowed_origins
    }

    pub fn with_session_cookie_secure(mut self, secure: bool) -> Self {
        self.session_cookie_secure = secure;
        self
    }

    pub fn session_cookie_secure(&self) -> bool {
        self.session_cookie_secure
    }

    pub(crate) fn create_session(&self, user_id: UserId) -> ApiResult<String> {
        self.server_store.create_session(user_id)
    }

    pub fn with_github_oauth(mut self, config: GithubOAuthConfig) -> Self {
        self.github_oauth = Some(config);
        self
    }

    pub fn repo(
        &self,
        dataset_id: &DatasetId,
    ) -> Result<Arc<DatasetRepository>, IdValidationError> {
        dataset_id.validate_path_segment()?;
        let mut repositories = self
            .repositories
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(repositories
            .entry(dataset_id.clone())
            .or_insert_with(|| {
                Arc::new(DatasetRepository::new(
                    self.datasets_root.join(dataset_id.as_str()),
                ))
            })
            .clone())
    }

    pub async fn put_ingest_job(&self, job: IngestJob) {
        self.ingest_jobs
            .write()
            .await
            .insert(job.job_id.clone(), job);
    }

    pub async fn get_ingest_job(&self, job_id: &str) -> Option<IngestJob> {
        self.ingest_jobs.read().await.get(job_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caches_repository_instances_by_dataset() {
        let state = ApiState::new("/tmp/labello-datasets");
        let first = state.repo(&DatasetId::from("ds")).unwrap();
        let second = state.clone().repo(&DatasetId::from("ds")).unwrap();
        let other = state.repo(&DatasetId::from("other")).unwrap();

        assert!(Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(&first, &other));
    }

    #[test]
    fn rejects_unsafe_dataset_ids_before_building_paths() {
        let state = ApiState::new("/tmp/labello-datasets");
        assert!(state.repo(&DatasetId::from("../escape")).is_err());
    }
}
