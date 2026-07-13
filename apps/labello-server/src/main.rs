use std::{net::SocketAddr, path::PathBuf};

use labello_api::{ApiState, GithubOAuthConfig, router};
use labello_domain::UserId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServerConfig {
    bind: String,
    datasets_root: String,
    bootstrap_admins: Vec<String>,
    #[serde(default)]
    allowed_origins: Vec<String>,
    #[serde(default = "default_cookie_secure")]
    session_cookie_secure: bool,
    dev_auth: DevAuthConfig,
    github_oauth: Option<GithubOAuthFileConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DevAuthConfig {
    enabled: bool,
    #[serde(default = "default_dev_token")]
    token: String,
    default_user_id: String,
    default_role: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GithubOAuthFileConfig {
    client_id: String,
    client_secret: String,
    redirect_uri: String,
}

fn default_dev_token() -> String {
    "dev-local-token".to_string()
}

fn default_cookie_secure() -> bool {
    true
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8080".to_string(),
            datasets_root: "datasets".to_string(),
            bootstrap_admins: vec!["admin".to_string()],
            allowed_origins: vec![
                "http://127.0.0.1:8081".to_string(),
                "http://localhost:8081".to_string(),
            ],
            session_cookie_secure: false,
            dev_auth: DevAuthConfig {
                enabled: true,
                token: "dev-local-token".to_string(),
                default_user_id: "admin".to_string(),
                default_role: "data_admin".to_string(),
            },
            github_oauth: None,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let mut config = load_or_create_config()?;
    if let Some(root) = std::env::var_os("LABELLO_DATASETS_ROOT") {
        config.datasets_root = root.to_string_lossy().to_string();
    }
    if let Ok(bind) = std::env::var("LABELLO_BIND") {
        config.bind = bind;
    }
    if let (Ok(client_id), Ok(client_secret), Ok(redirect_uri)) = (
        std::env::var("GITHUB_CLIENT_ID"),
        std::env::var("GITHUB_CLIENT_SECRET"),
        std::env::var("GITHUB_REDIRECT_URI"),
    ) {
        config.github_oauth = Some(GithubOAuthFileConfig {
            client_id,
            client_secret,
            redirect_uri,
        });
    }

    let datasets_root = PathBuf::from(&config.datasets_root);
    let bind: SocketAddr = config.bind.parse()?;
    let dev_auth_token = config
        .dev_auth
        .enabled
        .then(|| config.dev_auth.token.clone())
        .filter(|token| !token.is_empty());
    let mut state = ApiState::new(datasets_root)
        .with_dev_auth_token(dev_auth_token)
        .with_allowed_origins(config.allowed_origins.clone())
        .with_session_cookie_secure(config.session_cookie_secure)
        .with_bootstrap_admins(config.bootstrap_admins.iter().cloned().map(UserId::from));
    if let Some(github) = config.github_oauth {
        state = state.with_github_oauth(GithubOAuthConfig {
            client_id: github.client_id,
            client_secret: github.client_secret,
            redirect_uri: github.redirect_uri,
        });
    }

    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, "labello server listening");
    axum::serve(listener, router(state)).await?;
    Ok(())
}

fn load_or_create_config() -> anyhow::Result<ServerConfig> {
    let path = std::env::var_os("LABELLO_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("labello.server.toml"));
    if path.exists() {
        let text = std::fs::read_to_string(&path)?;
        Ok(toml::from_str(&text)?)
    } else {
        let config = ServerConfig::default();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, toml::to_string_pretty(&config)?)?;
        Ok(config)
    }
}
