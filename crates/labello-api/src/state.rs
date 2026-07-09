use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};

use labello_client::IngestJob;
use labello_domain::UserId;
use labello_storage::DatasetRepository;
use tokio::sync::RwLock;

use crate::GithubOAuthConfig;

#[derive(Clone)]
pub struct ApiState {
    datasets_root: Arc<PathBuf>,
    bootstrap_admins: Arc<BTreeSet<UserId>>,
    dev_auth_token: Option<Arc<String>>,
    ingest_jobs: Arc<RwLock<BTreeMap<String, IngestJob>>>,
    pub github_oauth: Option<GithubOAuthConfig>,
    pub http: reqwest::Client,
}

impl ApiState {
    pub fn new(datasets_root: impl Into<PathBuf>) -> Self {
        Self {
            datasets_root: Arc::new(datasets_root.into()),
            bootstrap_admins: Arc::new(BTreeSet::from([UserId::from("admin")])),
            dev_auth_token: None,
            ingest_jobs: Arc::new(RwLock::new(BTreeMap::new())),
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
        self.dev_auth_token = token.filter(|token| !token.is_empty()).map(Arc::new);
        self
    }

    pub fn dev_auth_token(&self) -> Option<&str> {
        self.dev_auth_token.as_deref().map(String::as_str)
    }

    pub fn is_bootstrap_admin(&self, user_id: &UserId) -> bool {
        self.bootstrap_admins.contains(user_id)
    }

    pub fn with_github_oauth(mut self, config: GithubOAuthConfig) -> Self {
        self.github_oauth = Some(config);
        self
    }

    pub fn repo(&self, dataset_id: &labello_domain::DatasetId) -> DatasetRepository {
        DatasetRepository::new(self.datasets_root.join(dataset_id.as_str()))
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
