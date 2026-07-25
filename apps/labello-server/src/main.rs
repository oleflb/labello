use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
    path::PathBuf,
    time::Duration,
};

use anyhow::{Context, bail};
use labello_api::{ApiState, GithubOAuthConfig, router};
use labello_domain::UserId;
use labello_storage::{ImportConfig, ImportLimits, ImportRoot, ImportService};
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
    #[serde(default)]
    import: Option<ImportFileConfig>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DevelopmentAuthConfig {
    local_admin_login: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GithubOAuthFileConfig {
    client_id: String,
    client_secret: String,
    redirect_uri: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ImportFileConfig {
    enabled: bool,
    server_roots: Vec<ImportRootFileConfig>,
    retain_raw_source: bool,
    failed_retention_hours: u64,
    successful_metadata_retention_days: u64,
    #[serde(default)]
    limits: ImportLimitsFileConfig,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct ImportLimitsFileConfig {
    concurrent_build_jobs: u64,
    concurrent_browser_upload_jobs: u64,
    active_reservations_per_owner: u64,
    browser_source_files: u64,
    browser_source_bytes: u64,
    server_source_files: u64,
    total_source_bytes: u64,
    selected_images: u64,
    single_source_file_bytes: u64,
    descriptor_bytes: u64,
    upload_chunk_bytes: u64,
    source_path_bytes: u64,
    source_path_depth: u64,
    source_component_bytes: u64,
    selected_categories: u64,
    selected_tasks: u64,
    coverage_entries: u64,
    annotations_total: u64,
    annotations_per_image: u64,
    generated_file_bytes_per_image: u64,
    keypoints_per_skeleton: u64,
    yolo_line_bytes: u64,
    yolo_columns: u64,
    structured_data_nesting: u64,
    decoded_image_pixels: u64,
    decoded_image_bytes: u64,
    staged_bytes: u64,
    diagnostic_examples_per_code: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ImportRootFileConfig {
    id: String,
    path: String,
    allowed_owners: Vec<String>,
}

impl Default for ImportLimitsFileConfig {
    fn default() -> Self {
        let limits = ImportLimits::default();
        Self {
            concurrent_build_jobs: u64::try_from(limits.concurrent_build_jobs)
                .expect("default concurrent_build_jobs fits in u64"),
            concurrent_browser_upload_jobs: u64::try_from(limits.concurrent_browser_upload_jobs)
                .expect("default concurrent_browser_upload_jobs fits in u64"),
            active_reservations_per_owner: u64::try_from(limits.active_reservations_per_owner)
                .expect("default active_reservations_per_owner fits in u64"),
            browser_source_files: u64::try_from(limits.browser_source_files)
                .expect("default browser_source_files fits in u64"),
            browser_source_bytes: limits.browser_source_bytes,
            server_source_files: u64::try_from(limits.server_source_files)
                .expect("default server_source_files fits in u64"),
            total_source_bytes: limits.total_source_bytes,
            selected_images: u64::try_from(limits.selected_images)
                .expect("default selected_images fits in u64"),
            single_source_file_bytes: limits.single_source_file_bytes,
            descriptor_bytes: limits.descriptor_bytes,
            upload_chunk_bytes: u64::try_from(limits.upload_chunk_bytes)
                .expect("default upload_chunk_bytes fits in u64"),
            source_path_bytes: u64::try_from(limits.source_path_bytes)
                .expect("default source_path_bytes fits in u64"),
            source_path_depth: u64::try_from(limits.source_path_depth)
                .expect("default source_path_depth fits in u64"),
            source_component_bytes: u64::try_from(limits.source_component_bytes)
                .expect("default source_component_bytes fits in u64"),
            selected_categories: u64::try_from(limits.selected_categories)
                .expect("default selected_categories fits in u64"),
            selected_tasks: u64::try_from(limits.selected_tasks)
                .expect("default selected_tasks fits in u64"),
            coverage_entries: u64::try_from(limits.coverage_entries)
                .expect("default coverage_entries fits in u64"),
            annotations_total: u64::try_from(limits.annotations_total)
                .expect("default annotations_total fits in u64"),
            annotations_per_image: u64::try_from(limits.annotations_per_image)
                .expect("default annotations_per_image fits in u64"),
            generated_file_bytes_per_image: limits.generated_file_bytes_per_image,
            keypoints_per_skeleton: u64::try_from(limits.keypoints_per_skeleton)
                .expect("default keypoints_per_skeleton fits in u64"),
            yolo_line_bytes: u64::try_from(limits.yolo_line_bytes)
                .expect("default yolo_line_bytes fits in u64"),
            yolo_columns: u64::try_from(limits.yolo_columns)
                .expect("default yolo_columns fits in u64"),
            structured_data_nesting: u64::try_from(limits.structured_data_nesting)
                .expect("default structured_data_nesting fits in u64"),
            decoded_image_pixels: limits.decoded_image_pixels,
            decoded_image_bytes: limits.decoded_image_bytes,
            staged_bytes: limits.staged_bytes,
            diagnostic_examples_per_code: u64::try_from(limits.diagnostic_examples_per_code)
                .expect("default diagnostic_examples_per_code fits in u64"),
        }
    }
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
                local_admin_login: true,
            },
            github_oauth: None,
            import: None,
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
    let local_admin_login_enabled = config.development_auth.local_admin_login;
    let github_oauth_enabled = config.github_oauth.is_some();
    let session_cookie_secure = config.session_cookie_secure;
    let browser_origin_count = config.browser_origins.len();
    let bootstrap_admin_count = config.bootstrap_admins.len();
    let state = build_state(config, bind).await?;

    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(
        event = "server.started",
        version = env!("CARGO_PKG_VERSION"),
        %bind,
        local_admin_login_enabled,
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

async fn build_state(config: ServerConfig, bind: SocketAddr) -> anyhow::Result<ApiState> {
    let datasets_root = PathBuf::from(&config.datasets_root);
    let bootstrap_admins: Vec<_> = config
        .bootstrap_admins
        .into_iter()
        .map(UserId::from)
        .collect();
    let local_admin_user_id = if config.development_auth.local_admin_login {
        if !bind.ip().is_loopback() {
            bail!("developmentAuth.localAdminLogin requires a loopback bind address");
        }
        let user_id = bootstrap_admins.first().cloned().ok_or_else(|| {
            anyhow::anyhow!(
                "developmentAuth.localAdminLogin requires at least one valid bootstrap admin"
            )
        })?;
        user_id.validate_path_segment().map_err(|error| {
            anyhow::anyhow!(
                "developmentAuth.localAdminLogin requires a valid first bootstrap admin: {error}"
            )
        })?;
        Some(user_id)
    } else {
        None
    };
    let mut state = ApiState::new(&datasets_root)
        .with_browser_origins(config.browser_origins)
        .context("invalid browserOrigins")?
        .with_session_cookie_secure(config.session_cookie_secure)
        .with_bootstrap_admins(bootstrap_admins)
        .with_local_admin_login(local_admin_user_id)
        .with_import_root_owners(import_root_owners(config.import.as_ref())?);
    if let Some(github) = config.github_oauth {
        state = state.with_github_oauth(GithubOAuthConfig {
            client_id: github.client_id,
            client_secret: github.client_secret,
            redirect_uri: github.redirect_uri,
        });
    }
    let import_service = ImportService::new(
        &datasets_root,
        storage_import_config(config.import.as_ref())?,
    )
    .await
    .context("cannot initialize dataset import service")?;
    let recovery = import_service
        .recover()
        .await
        .context("cannot recover dataset import jobs")?;
    tracing::info!(
        event = "import.recovery.completed",
        recovered_successes = recovery.recovered_successes,
        resumed_jobs = recovery.resumed_to_awaiting_decision,
        failed_commits = recovery.failed_incomplete_commits,
        released_reservations = recovery.released_reservations,
        "dataset import recovery completed"
    );
    state = state.with_import_service(import_service);
    Ok(state)
}

fn import_root_owners(
    config: Option<&ImportFileConfig>,
) -> anyhow::Result<BTreeMap<String, BTreeSet<UserId>>> {
    config
        .into_iter()
        .flat_map(|config| &config.server_roots)
        .map(|root| {
            let owners = root
                .allowed_owners
                .iter()
                .map(|owner| {
                    let owner = UserId::from(owner.clone());
                    owner.validate_path_segment().map_err(|error| {
                        anyhow::anyhow!(
                            "import root {} has invalid allowedOwners entry: {error}",
                            root.id
                        )
                    })?;
                    Ok(owner)
                })
                .collect::<anyhow::Result<BTreeSet<_>>>()?;
            Ok((root.id.clone(), owners))
        })
        .collect()
}

fn storage_import_config(config: Option<&ImportFileConfig>) -> anyhow::Result<ImportConfig> {
    let Some(config) = config else {
        return Ok(ImportConfig::default());
    };
    Ok(ImportConfig {
        enabled: config.enabled,
        import_roots: config
            .server_roots
            .iter()
            .map(|root| ImportRoot {
                root_id: root.id.clone(),
                path: PathBuf::from(&root.path),
                allowed_owners: root
                    .allowed_owners
                    .iter()
                    .cloned()
                    .map(UserId::from)
                    .collect(),
            })
            .collect(),
        allowed_profiles: labello_storage::ImportProfile::ALL.to_vec(),
        retain_raw_source: config.retain_raw_source,
        failed_retention: Duration::from_secs(
            config.failed_retention_hours.saturating_mul(60 * 60),
        ),
        successful_metadata_retention: Duration::from_secs(
            config
                .successful_metadata_retention_days
                .saturating_mul(24 * 60 * 60),
        ),
        limits: storage_import_limits(&config.limits)?,
    })
}

fn storage_import_limits(config: &ImportLimitsFileConfig) -> anyhow::Result<ImportLimits> {
    let values = [
        ("concurrentBuildJobs", config.concurrent_build_jobs),
        (
            "concurrentBrowserUploadJobs",
            config.concurrent_browser_upload_jobs,
        ),
        (
            "activeReservationsPerOwner",
            config.active_reservations_per_owner,
        ),
        ("browserSourceFiles", config.browser_source_files),
        ("browserSourceBytes", config.browser_source_bytes),
        ("serverSourceFiles", config.server_source_files),
        ("totalSourceBytes", config.total_source_bytes),
        ("selectedImages", config.selected_images),
        ("singleSourceFileBytes", config.single_source_file_bytes),
        ("descriptorBytes", config.descriptor_bytes),
        ("uploadChunkBytes", config.upload_chunk_bytes),
        ("sourcePathBytes", config.source_path_bytes),
        ("sourcePathDepth", config.source_path_depth),
        ("sourceComponentBytes", config.source_component_bytes),
        ("selectedCategories", config.selected_categories),
        ("selectedTasks", config.selected_tasks),
        ("coverageEntries", config.coverage_entries),
        ("annotationsTotal", config.annotations_total),
        ("annotationsPerImage", config.annotations_per_image),
        (
            "generatedFileBytesPerImage",
            config.generated_file_bytes_per_image,
        ),
        ("keypointsPerSkeleton", config.keypoints_per_skeleton),
        ("yoloLineBytes", config.yolo_line_bytes),
        ("yoloColumns", config.yolo_columns),
        ("structuredDataNesting", config.structured_data_nesting),
        ("decodedImagePixels", config.decoded_image_pixels),
        ("decodedImageBytes", config.decoded_image_bytes),
        ("stagedBytes", config.staged_bytes),
        (
            "diagnosticExamplesPerCode",
            config.diagnostic_examples_per_code,
        ),
    ];
    if let Some((name, _)) = values.into_iter().find(|(_, value)| *value == 0) {
        bail!("import.limits.{name} must be greater than zero");
    }

    validate_limit_order(
        config.browser_source_bytes,
        "browserSourceBytes",
        config.total_source_bytes,
        "totalSourceBytes",
    )?;
    validate_limit_order(
        config.single_source_file_bytes,
        "singleSourceFileBytes",
        config.total_source_bytes,
        "totalSourceBytes",
    )?;
    validate_limit_order(
        config.descriptor_bytes,
        "descriptorBytes",
        config.single_source_file_bytes,
        "singleSourceFileBytes",
    )?;
    validate_limit_order(
        config.upload_chunk_bytes,
        "uploadChunkBytes",
        config.single_source_file_bytes,
        "singleSourceFileBytes",
    )?;
    validate_limit_order(
        config.source_component_bytes,
        "sourceComponentBytes",
        config.source_path_bytes,
        "sourcePathBytes",
    )?;
    validate_limit_order(
        config.source_path_depth,
        "sourcePathDepth",
        config.source_path_bytes,
        "sourcePathBytes",
    )?;
    validate_limit_order(
        config.annotations_per_image,
        "annotationsPerImage",
        config.annotations_total,
        "annotationsTotal",
    )?;
    validate_limit_order(
        config.generated_file_bytes_per_image,
        "generatedFileBytesPerImage",
        config.staged_bytes,
        "stagedBytes",
    )?;
    validate_limit_order(
        config.yolo_columns,
        "yoloColumns",
        config.yolo_line_bytes,
        "yoloLineBytes",
    )?;
    validate_limit_order(
        config.total_source_bytes,
        "totalSourceBytes",
        config.staged_bytes,
        "stagedBytes",
    )?;

    for (name, value) in [
        ("selectedCategories", config.selected_categories),
        ("selectedTasks", config.selected_tasks),
        ("annotationsPerImage", config.annotations_per_image),
        ("keypointsPerSkeleton", config.keypoints_per_skeleton),
    ] {
        if value > u64::from(u32::MAX) {
            bail!("import.limits.{name} exceeds the import capability range");
        }
    }

    Ok(ImportLimits {
        concurrent_build_jobs: usize_import_limit(
            config.concurrent_build_jobs,
            "concurrentBuildJobs",
        )?,
        concurrent_browser_upload_jobs: usize_import_limit(
            config.concurrent_browser_upload_jobs,
            "concurrentBrowserUploadJobs",
        )?,
        active_reservations_per_owner: usize_import_limit(
            config.active_reservations_per_owner,
            "activeReservationsPerOwner",
        )?,
        browser_source_files: usize_import_limit(
            config.browser_source_files,
            "browserSourceFiles",
        )?,
        browser_source_bytes: config.browser_source_bytes,
        server_source_files: usize_import_limit(config.server_source_files, "serverSourceFiles")?,
        total_source_bytes: config.total_source_bytes,
        selected_images: usize_import_limit(config.selected_images, "selectedImages")?,
        single_source_file_bytes: config.single_source_file_bytes,
        descriptor_bytes: config.descriptor_bytes,
        upload_chunk_bytes: usize_import_limit(config.upload_chunk_bytes, "uploadChunkBytes")?,
        source_path_bytes: usize_import_limit(config.source_path_bytes, "sourcePathBytes")?,
        source_path_depth: usize_import_limit(config.source_path_depth, "sourcePathDepth")?,
        source_component_bytes: usize_import_limit(
            config.source_component_bytes,
            "sourceComponentBytes",
        )?,
        selected_categories: usize_import_limit(config.selected_categories, "selectedCategories")?,
        selected_tasks: usize_import_limit(config.selected_tasks, "selectedTasks")?,
        coverage_entries: usize_import_limit(config.coverage_entries, "coverageEntries")?,
        annotations_total: usize_import_limit(config.annotations_total, "annotationsTotal")?,
        annotations_per_image: usize_import_limit(
            config.annotations_per_image,
            "annotationsPerImage",
        )?,
        generated_file_bytes_per_image: config.generated_file_bytes_per_image,
        keypoints_per_skeleton: usize_import_limit(
            config.keypoints_per_skeleton,
            "keypointsPerSkeleton",
        )?,
        yolo_line_bytes: usize_import_limit(config.yolo_line_bytes, "yoloLineBytes")?,
        yolo_columns: usize_import_limit(config.yolo_columns, "yoloColumns")?,
        structured_data_nesting: usize_import_limit(
            config.structured_data_nesting,
            "structuredDataNesting",
        )?,
        decoded_image_pixels: config.decoded_image_pixels,
        decoded_image_bytes: config.decoded_image_bytes,
        staged_bytes: config.staged_bytes,
        diagnostic_examples_per_code: usize_import_limit(
            config.diagnostic_examples_per_code,
            "diagnosticExamplesPerCode",
        )?,
    })
}

fn validate_limit_order(
    lower: u64,
    lower_name: &str,
    upper: u64,
    upper_name: &str,
) -> anyhow::Result<()> {
    if lower > upper {
        bail!("import.limits.{lower_name} cannot exceed import.limits.{upper_name}");
    }
    Ok(())
}

fn usize_import_limit(value: u64, name: &str) -> anyhow::Result<usize> {
    usize::try_from(value)
        .with_context(|| format!("import.limits.{name} exceeds this platform's usize range"))
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
localAdminLogin = false
"#;

    #[test]
    fn parses_required_server_schema() {
        let config: ServerConfig = toml::from_str(CONFIG).unwrap();
        assert_eq!(config.browser_origins, ["https://app.example.com"]);
        assert!(!config.development_auth.local_admin_login);
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
    fn import_configuration_is_optional_strict_and_converts_to_storage() {
        let configured = format!(
            "{CONFIG}\n[import]\nenabled = true\nserverRoots = []\nretainRawSource = true\nfailedRetentionHours = 12\nsuccessfulMetadataRetentionDays = 7\n"
        );
        let config: ServerConfig = toml::from_str(&configured).unwrap();
        let storage = storage_import_config(config.import.as_ref()).unwrap();
        assert!(storage.enabled);
        assert!(storage.retain_raw_source);
        assert_eq!(storage.limits, ImportLimits::default());
        assert_eq!(
            storage.allowed_profiles,
            labello_storage::ImportProfile::ALL
        );
        assert_eq!(storage.failed_retention, Duration::from_secs(12 * 60 * 60));
        assert_eq!(
            storage.successful_metadata_retention,
            Duration::from_secs(7 * 24 * 60 * 60)
        );

        let invalid = configured.replace("retainRawSource", "keepRawSource");
        let error = toml::from_str::<ServerConfig>(&invalid)
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown field"), "{error}");

        let missing = configured.replace("failedRetentionHours = 12\n", "");
        let error = toml::from_str::<ServerConfig>(&missing)
            .unwrap_err()
            .to_string();
        assert!(error.contains("failedRetentionHours"), "{error}");
    }

    #[test]
    fn import_limits_default_individually_and_convert_exactly() {
        let configured = format!(
            "{CONFIG}\n[import]\nenabled = true\nserverRoots = []\nretainRawSource = false\nfailedRetentionHours = 24\nsuccessfulMetadataRetentionDays = 30\n\n[import.limits]\nconcurrentBuildJobs = 3\n"
        );
        let config: ServerConfig = toml::from_str(&configured).unwrap();
        let expected = ImportLimits {
            concurrent_build_jobs: 3,
            ..ImportLimits::default()
        };
        assert_eq!(
            storage_import_config(config.import.as_ref())
                .unwrap()
                .limits,
            expected
        );

        let configured = configured.replace(
            "concurrentBuildJobs = 3\n",
            r#"concurrentBuildJobs = 3
concurrentBrowserUploadJobs = 4
activeReservationsPerOwner = 5
browserSourceFiles = 600
browserSourceBytes = 6000
serverSourceFiles = 700
totalSourceBytes = 10000
selectedImages = 500
singleSourceFileBytes = 1000
descriptorBytes = 500
uploadChunkBytes = 250
sourcePathBytes = 200
sourcePathDepth = 20
sourceComponentBytes = 100
selectedCategories = 30
selectedTasks = 40
coverageEntries = 1000
annotationsTotal = 2000
annotationsPerImage = 20
generatedFileBytesPerImage = 500
keypointsPerSkeleton = 10
yoloLineBytes = 100
yoloColumns = 20
structuredDataNesting = 8
decodedImagePixels = 10000
decodedImageBytes = 40000
stagedBytes = 20000
diagnosticExamplesPerCode = 7
"#,
        );
        let config: ServerConfig = toml::from_str(&configured).unwrap();
        assert_eq!(
            storage_import_config(config.import.as_ref())
                .unwrap()
                .limits,
            ImportLimits {
                concurrent_build_jobs: 3,
                concurrent_browser_upload_jobs: 4,
                active_reservations_per_owner: 5,
                browser_source_files: 600,
                browser_source_bytes: 6000,
                server_source_files: 700,
                total_source_bytes: 10000,
                selected_images: 500,
                single_source_file_bytes: 1000,
                descriptor_bytes: 500,
                upload_chunk_bytes: 250,
                source_path_bytes: 200,
                source_path_depth: 20,
                source_component_bytes: 100,
                selected_categories: 30,
                selected_tasks: 40,
                coverage_entries: 1000,
                annotations_total: 2000,
                annotations_per_image: 20,
                generated_file_bytes_per_image: 500,
                keypoints_per_skeleton: 10,
                yolo_line_bytes: 100,
                yolo_columns: 20,
                structured_data_nesting: 8,
                decoded_image_pixels: 10000,
                decoded_image_bytes: 40000,
                staged_bytes: 20000,
                diagnostic_examples_per_code: 7,
            }
        );
    }

    #[test]
    fn rejects_unknown_or_unsafe_import_limits() {
        let configured = format!(
            "{CONFIG}\n[import]\nenabled = true\nserverRoots = []\nretainRawSource = false\nfailedRetentionHours = 24\nsuccessfulMetadataRetentionDays = 30\n\n[import.limits]\n"
        );

        let unknown = format!("{configured}browserFiles = 10\n");
        let error = toml::from_str::<ServerConfig>(&unknown)
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown field"), "{error}");

        for (field, value, expected) in [
            ("concurrentBuildJobs", 0, "must be greater than zero"),
            (
                "descriptorBytes",
                ImportLimits::default().single_source_file_bytes + 1,
                "descriptorBytes cannot exceed import.limits.singleSourceFileBytes",
            ),
            (
                "stagedBytes",
                ImportLimits::default().total_source_bytes - 1,
                "totalSourceBytes cannot exceed import.limits.stagedBytes",
            ),
            (
                "selectedCategories",
                u64::from(u32::MAX) + 1,
                "exceeds the import capability range",
            ),
        ] {
            let invalid = format!("{configured}{field} = {value}\n");
            let config: ServerConfig = toml::from_str(&invalid).unwrap();
            let error = storage_import_config(config.import.as_ref())
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected), "{field}: {error}");
        }
    }

    #[tokio::test]
    async fn configured_import_root_owners_reach_api_capability_policy() {
        let base = std::env::temp_dir().join(format!(
            "labello-server-config-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let datasets = base.join("datasets");
        let source = base.join("source");
        std::fs::create_dir_all(&source).unwrap();
        let mut config = ServerConfig {
            datasets_root: datasets.to_string_lossy().to_string(),
            bootstrap_admins: vec!["admin".to_string(), "other".to_string()],
            ..ServerConfig::default()
        };
        config.import = Some(ImportFileConfig {
            enabled: false,
            server_roots: vec![ImportRootFileConfig {
                id: "curated".to_string(),
                path: source.to_string_lossy().to_string(),
                allowed_owners: vec!["admin".to_string()],
            }],
            retain_raw_source: false,
            failed_retention_hours: 24,
            successful_metadata_retention_days: 30,
            limits: ImportLimitsFileConfig::default(),
        });

        let state = build_state(config, "127.0.0.1:8080".parse().unwrap())
            .await
            .unwrap();
        assert!(state.import_root_visible_to("curated", &UserId::from("admin")));
        assert!(!state.import_root_visible_to("curated", &UserId::from("other")));
        assert!(!state.import_root_visible_to("unconfigured", &UserId::from("admin")));
        drop(state);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn import_root_owner_ids_are_validated() {
        let config = ImportFileConfig {
            enabled: true,
            server_roots: vec![ImportRootFileConfig {
                id: "curated".to_string(),
                path: "/tmp/source".to_string(),
                allowed_owners: vec!["../escape".to_string()],
            }],
            retain_raw_source: false,
            failed_retention_hours: 24,
            successful_metadata_retention_days: 30,
            limits: ImportLimitsFileConfig::default(),
        };
        assert!(import_root_owners(Some(&config)).is_err());
    }

    #[test]
    fn rejects_removed_development_auth_fields() {
        let old = CONFIG.replace(
            "localAdminLogin = false",
            "enabled = true\ntoken = \"dev-local-token\"\nlocalAdminLogin = false",
        );
        let error = toml::from_str::<ServerConfig>(&old)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("unknown field"), "{error}");
    }

    #[tokio::test]
    async fn startup_rejects_invalid_origins() {
        let mut config: ServerConfig = toml::from_str(CONFIG).unwrap();
        config.browser_origins = vec!["https://app.example.com/path".to_string()];
        assert!(
            build_state(config, "127.0.0.1:8080".parse().unwrap())
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("browserOrigins")
        );
    }

    #[tokio::test]
    async fn local_admin_login_requires_safe_development_configuration() {
        let config = ServerConfig {
            bind: "0.0.0.0:8080".to_string(),
            ..Default::default()
        };
        assert!(
            build_state(config, "0.0.0.0:8080".parse().unwrap())
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("loopback")
        );

        let mut config = ServerConfig::default();
        config.bootstrap_admins.clear();
        assert!(
            build_state(config, "127.0.0.1:8080".parse().unwrap())
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("bootstrap admin")
        );

        let mut config = ServerConfig::default();
        config.bootstrap_admins[0] = "../admin".to_string();
        assert!(
            build_state(config, "127.0.0.1:8080".parse().unwrap())
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("valid first bootstrap admin")
        );
    }
}
