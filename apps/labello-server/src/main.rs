use std::{net::SocketAddr, path::PathBuf};

use labello_api::{ApiState, GithubOAuthConfig, router};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let datasets_root = std::env::var_os("LABELLO_DATASETS_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("datasets"));
    let bind: SocketAddr = std::env::var("LABELLO_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
        .parse()?;
    let mut state = ApiState::new(datasets_root);
    if let (Ok(client_id), Ok(client_secret), Ok(redirect_uri)) = (
        std::env::var("GITHUB_CLIENT_ID"),
        std::env::var("GITHUB_CLIENT_SECRET"),
        std::env::var("GITHUB_REDIRECT_URI"),
    ) {
        state = state.with_github_oauth(GithubOAuthConfig {
            client_id,
            client_secret,
            redirect_uri,
        });
    }

    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, "labello server listening");
    axum::serve(listener, router(state)).await?;
    Ok(())
}
