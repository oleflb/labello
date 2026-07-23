use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use labello_client::IngestJob;
use labello_domain::{DatasetId, IdValidationError, UserId};
use labello_storage::DatasetRepository;
use tokio::sync::RwLock;
use url::Url;

use crate::{GithubOAuthConfig, error::ApiResult, session::ServerStore};

#[derive(Clone)]
pub struct ApiState {
    datasets_root: Arc<PathBuf>,
    bootstrap_admins: Arc<BTreeSet<UserId>>,
    local_admin_user_id: Option<Arc<UserId>>,
    browser_origins: Arc<Vec<String>>,
    session_cookie_secure: bool,
    pub(crate) server_store: ServerStore,
    ingest_jobs: Arc<RwLock<BTreeMap<String, IngestJob>>>,
    repositories: Arc<Mutex<BTreeMap<DatasetId, Arc<DatasetRepository>>>>,
    pub github_oauth: Option<GithubOAuthConfig>,
    pub(crate) github_oauth_endpoints: crate::oauth::GithubOAuthEndpoints,
    pub http: reqwest::Client,
}

impl ApiState {
    pub fn new(datasets_root: impl Into<PathBuf>) -> Self {
        let datasets_root = datasets_root.into();
        Self {
            server_store: ServerStore::new(&datasets_root),
            datasets_root: Arc::new(datasets_root),
            bootstrap_admins: Arc::new(BTreeSet::from([UserId::from("admin")])),
            local_admin_user_id: None,
            browser_origins: Arc::new(Vec::new()),
            session_cookie_secure: true,
            ingest_jobs: Arc::new(RwLock::new(BTreeMap::new())),
            repositories: Arc::new(Mutex::new(BTreeMap::new())),
            github_oauth: None,
            github_oauth_endpoints: crate::oauth::GithubOAuthEndpoints::default(),
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

    pub fn with_local_admin_login(mut self, user_id: Option<UserId>) -> Self {
        self.local_admin_user_id = user_id.map(Arc::new);
        self
    }

    pub fn local_admin_login_enabled(&self) -> bool {
        self.local_admin_user_id.is_some()
    }

    pub(crate) fn local_admin_user_id(&self) -> Option<&UserId> {
        self.local_admin_user_id.as_deref()
    }

    pub fn is_bootstrap_admin(&self, user_id: &UserId) -> bool {
        self.bootstrap_admins.contains(user_id)
    }

    pub fn with_browser_origins(mut self, origins: Vec<String>) -> ApiResult<Self> {
        self.browser_origins = Arc::new(validate_browser_origins(origins)?);
        Ok(self)
    }

    pub fn browser_origins(&self) -> &[String] {
        &self.browser_origins
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

    #[cfg(test)]
    pub(crate) fn with_github_oauth_endpoints(
        mut self,
        endpoints: crate::oauth::GithubOAuthEndpoints,
    ) -> Self {
        self.github_oauth_endpoints = endpoints;
        self
    }

    pub fn repo(
        &self,
        dataset_id: &DatasetId,
    ) -> Result<Arc<DatasetRepository>, IdValidationError> {
        dataset_id.validate_path_segment()?;
        let mut repositories = self.repositories.lock().unwrap_or_else(|poisoned| {
            tracing::error!(
                event = "repository.lock_poisoned",
                "repository cache lock recovered after panic"
            );
            poisoned.into_inner()
        });
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

fn validate_browser_origins(origins: Vec<String>) -> ApiResult<Vec<String>> {
    if origins.is_empty() {
        return Err(crate::error::ApiError::BadRequest(
            "browserOrigins must contain at least one origin".to_string(),
        ));
    }
    origins
        .into_iter()
        .map(|origin| {
            let url = Url::parse(&origin).map_err(|error| {
                crate::error::ApiError::BadRequest(format!(
                    "invalid browser origin {origin:?}: {error}"
                ))
            })?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.path() != "/"
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(crate::error::ApiError::BadRequest(format!(
                    "invalid browser origin {origin:?}: expected an http(s) origin without credentials, path, query, or fragment"
                )));
            }
            Ok(url.origin().ascii_serialization())
        })
        .collect()
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
