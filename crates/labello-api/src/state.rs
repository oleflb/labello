use std::{path::PathBuf, sync::Arc};

use labello_storage::DatasetRepository;

use crate::GithubOAuthConfig;

#[derive(Clone)]
pub struct ApiState {
    datasets_root: Arc<PathBuf>,
    pub github_oauth: Option<GithubOAuthConfig>,
    pub http: reqwest::Client,
}

impl ApiState {
    pub fn new(datasets_root: impl Into<PathBuf>) -> Self {
        Self {
            datasets_root: Arc::new(datasets_root.into()),
            github_oauth: None,
            http: reqwest::Client::new(),
        }
    }

    pub fn with_github_oauth(mut self, config: GithubOAuthConfig) -> Self {
        self.github_oauth = Some(config);
        self
    }

    pub fn repo(&self, dataset_id: &labello_domain::DatasetId) -> DatasetRepository {
        DatasetRepository::new(self.datasets_root.join(dataset_id.as_str()))
    }
}
