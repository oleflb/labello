use std::{net::SocketAddr, path::PathBuf};

use anyhow::{Context, bail};
use labello_api::{ApiState, GithubOAuthConfig, router};
use labello_domain::UserId;
use serde::{Deserialize, Serialize};

mod logging;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ServerConfig {
    bind: String,
    datasets_root: String,
    bootstrap_admins: Vec<String>,
    browser_origins: Vec<String>,
    session_cookie_secure: bool,
    development_auth: DevelopmentAuthConfig,
    github_oauth: Option<GithubOAuthFileConfig>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DevelopmentAuthConfig {
    enabled: bool,
    token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GithubOAuthFileConfig {
    client_id: String,
    client_secret: String,
    redirect_uri: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8080".to_string(),
            datasets_root: "datasets".to_string(),
            bootstrap_admins: vec!["admin".to_string()],
            browser_origins: vec![
                "http://127.0.0.1:8081".to_string(),
                "http://localhost:8081".to_string(),
            ],
            session_cookie_secure: false,
            development_auth: DevelopmentAuthConfig {
                enabled: true,
                token: "dev-local-token".to_string(),
            },
            github_oauth: None,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    logging::init()?;

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

    let bind: SocketAddr = config.bind.parse().context("invalid server bind address")?;
    let development_auth_enabled = config.development_auth.enabled;
    let github_oauth_enabled = config.github_oauth.is_some();
    let session_cookie_secure = config.session_cookie_secure;
    let browser_origin_count = config.browser_origins.len();
    let bootstrap_admin_count = config.bootstrap_admins.len();
    let state = build_state(config)?;

    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(
        event = "server.started",
        version = env!("CARGO_PKG_VERSION"),
        %bind,
        development_auth_enabled,
        github_oauth_enabled,
        session_cookie_secure,
        browser_origin_count,
        bootstrap_admin_count,
        "labello server listening"
    );
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    tracing::info!(event = "server.stopped", "labello server stopped");
    Ok(())
}

async fn shutdown_signal() {
    match tokio::signal::ctrl_c().await {
        Ok(()) => tracing::info!(event = "server.shutdown.started", "shutdown requested"),
        Err(error) => tracing::error!(
            event = "server.shutdown.signal_failed",
            error = %error,
            "could not install shutdown signal handler"
        ),
    }
}

fn load_or_create_config() -> anyhow::Result<ServerConfig> {
    let path = std::env::var_os("LABELLO_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("labello.server.toml"));
    if path.exists() {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read server config {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("invalid server config {}", path.display()))
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

fn build_state(config: ServerConfig) -> anyhow::Result<ApiState> {
    if config.development_auth.enabled && config.development_auth.token.trim().is_empty() {
        bail!("developmentAuth.token must be nonempty when developmentAuth.enabled is true");
    }
    let dev_auth_token = config
        .development_auth
        .enabled
        .then_some(config.development_auth.token);
    let mut state = ApiState::new(PathBuf::from(config.datasets_root))
        .with_dev_auth_token(dev_auth_token)
        .with_browser_origins(config.browser_origins)
        .context("invalid browserOrigins")?
        .with_session_cookie_secure(config.session_cookie_secure)
        .with_bootstrap_admins(config.bootstrap_admins.into_iter().map(UserId::from));
    if let Some(github) = config.github_oauth {
        state = state.with_github_oauth(GithubOAuthConfig {
            client_id: github.client_id,
            client_secret: github.client_secret,
            redirect_uri: github.redirect_uri,
        });
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"
bind = "127.0.0.1:8080"
datasetsRoot = "datasets"
bootstrapAdmins = ["admin"]
browserOrigins = ["https://app.example.com"]
sessionCookieSecure = true

[developmentAuth]
enabled = false
token = "unused"
"#;

    #[test]
    fn parses_required_server_schema() {
        let config: ServerConfig = toml::from_str(CONFIG).unwrap();
        assert_eq!(config.browser_origins, ["https://app.example.com"]);
        assert!(!config.development_auth.enabled);
    }

    #[test]
    fn example_config_matches_defaults_and_documents_oauth() {
        const EXAMPLE: &str = include_str!("../../../labello.server.example.toml");

        let config: ServerConfig = toml::from_str(EXAMPLE).unwrap();
        assert_eq!(config, ServerConfig::default());

        let with_oauth = EXAMPLE
            .replace("# [githubOauth]", "[githubOauth]")
            .replace("# clientId", "clientId")
            .replace("# clientSecret", "clientSecret")
            .replace("# redirectUri", "redirectUri");
        let config: ServerConfig = toml::from_str(&with_oauth).unwrap();
        let github = config.github_oauth.unwrap();
        assert_eq!(github.client_id, "your-github-client-id");
        assert_eq!(github.client_secret, "your-github-client-secret");
        assert_eq!(
            github.redirect_uri,
            "https://api.example.com/auth/github/callback"
        );
    }

    #[test]
    fn rejects_old_and_missing_config_fields() {
        let old = CONFIG
            .replace("browserOrigins", "allowedOrigins")
            .replace("developmentAuth", "devAuth");
        let error = toml::from_str::<ServerConfig>(&old)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("unknown field"), "{error}");

        let missing = CONFIG.replace("sessionCookieSecure = true\n", "");
        let error = toml::from_str::<ServerConfig>(&missing)
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("missing field `sessionCookieSecure`"),
            "{error}"
        );
    }

    #[test]
    fn rejects_removed_development_auth_defaults() {
        let old = CONFIG.replace(
            "token = \"unused\"",
            "token = \"unused\"\ndefaultUserId = \"admin\"\ndefaultRole = \"data_admin\"",
        );
        let error = toml::from_str::<ServerConfig>(&old)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("unknown field"), "{error}");
    }

    #[test]
    fn startup_rejects_invalid_origins_and_empty_enabled_dev_token() {
        let mut config: ServerConfig = toml::from_str(CONFIG).unwrap();
        config.browser_origins = vec!["https://app.example.com/path".to_string()];
        assert!(
            build_state(config)
                .err()
                .unwrap()
                .to_string()
                .contains("browserOrigins")
        );

        let mut config: ServerConfig = toml::from_str(CONFIG).unwrap();
        config.development_auth.enabled = true;
        config.development_auth.token.clear();
        assert!(
            build_state(config)
                .err()
                .unwrap()
                .to_string()
                .contains("developmentAuth.token")
        );
    }
}
