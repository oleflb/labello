use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

pub const MAX_BROWSER_CONFIG_BYTES: usize = 16 * 1024;
pub const MAX_IMAGE_VALIDATION_WORKERS: usize = 32;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserConfig {
    api_base_url: Option<String>,
}

impl BrowserConfig {
    pub fn parse(text: &str) -> Result<Self, ConfigParseError> {
        if text.len() > MAX_BROWSER_CONFIG_BYTES {
            return Err(ConfigParseError::new("configuration exceeds 16 KiB", None));
        }
        let mut config: Self = serde_json::from_str(text).map_err(|error| {
            ConfigParseError::new(
                "invalid JSON syntax or schema",
                Some((error.line(), error.column())),
            )
        })?;
        config.api_base_url = config
            .api_base_url
            .as_deref()
            .map(validate_api_base_url)
            .transpose()
            .map_err(|category| ConfigParseError::new(category, None))?;
        Ok(config)
    }

    pub fn api_base_url(&self) -> Option<&str> {
        self.api_base_url.as_deref()
    }
}

fn validate_api_base_url(value: &str) -> Result<String, &'static str> {
    let authority = value
        .split_once("://")
        .map(|(_, authority)| authority)
        .unwrap_or_default();
    if authority.is_empty() || authority.starts_with('/') {
        return Err("apiBaseUrl must use HTTP or HTTPS and include a host");
    }
    let url = url::Url::parse(value).map_err(|_| "apiBaseUrl must be an absolute URL")?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("apiBaseUrl must use HTTP or HTTPS and include a host");
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("apiBaseUrl must not contain credentials");
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("apiBaseUrl must not contain a query or fragment");
    }
    if url.path() != "/" && !url.path().ends_with('/') {
        return Err("apiBaseUrl path prefixes must end with a slash");
    }
    Ok(url.to_string())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServerConfig {
    pub bind: String,
    pub datasets_root: String,
    pub bootstrap_admins: Vec<String>,
    pub browser_origins: Vec<String>,
    pub session_cookie_secure: bool,
    pub development_auth: DevelopmentAuthConfig,
    pub github_oauth: Option<GithubOAuthFileConfig>,
    #[serde(default)]
    pub import: Option<ImportFileConfig>,
}

impl ServerConfig {
    pub fn parse(text: &str) -> Result<Self, ConfigParseError> {
        let config: Self = toml::from_str(text).map_err(|error| {
            ConfigParseError::new(
                "invalid TOML syntax or schema",
                error.span().map(|span| line_column(text, span.start)),
            )
        })?;
        if config
            .bootstrap_admins
            .iter()
            .any(|admin| !valid_path_segment_id(admin))
            || config.import.iter().any(|import| {
                import.server_roots.iter().any(|root| {
                    root.allowed_owners
                        .iter()
                        .any(|owner| !valid_path_segment_id(owner))
                })
            })
        {
            return Err(ConfigParseError::new(
                "invalid identifier in server configuration",
                None,
            ));
        }
        config
            .validate_semantics()
            .map_err(|error| ConfigParseError::new(error.to_string(), None))?;
        Ok(config)
    }

    pub fn validate_semantics(&self) -> Result<(), ConfigValidationError> {
        if let Some(import) = &self.import {
            validate_import_root_ids(import.server_roots.iter().map(|root| root.id.as_str()))?;
            import.limits.validate()?;
        }
        Ok(())
    }
}

fn valid_path_segment_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && !matches!(value, "." | "..")
        && !value
            .bytes()
            .any(|byte| byte.is_ascii_control() || matches!(byte, b'/' | b'\\'))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DevelopmentAuthConfig {
    pub local_admin_login: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GithubOAuthFileConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportFileConfig {
    pub enabled: bool,
    pub server_roots: Vec<ImportRootFileConfig>,
    pub retain_raw_source: bool,
    pub failed_retention_hours: u64,
    pub successful_metadata_retention_days: u64,
    #[serde(default)]
    pub limits: ImportLimitsFileConfig,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportRootFileConfig {
    pub id: String,
    pub path: String,
    pub allowed_owners: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportLimitsFileConfig {
    pub concurrent_build_jobs: u64,
    pub image_validation_workers: u64,
    pub decoded_image_memory_bytes: u64,
    pub concurrent_browser_upload_jobs: u64,
    pub active_reservations_per_owner: u64,
    pub browser_source_files: u64,
    pub browser_source_bytes: u64,
    pub server_source_files: u64,
    pub total_source_bytes: u64,
    pub selected_images: u64,
    pub single_source_file_bytes: u64,
    pub descriptor_bytes: u64,
    pub upload_chunk_bytes: u64,
    pub source_path_bytes: u64,
    pub source_path_depth: u64,
    pub source_component_bytes: u64,
    pub selected_categories: u64,
    pub selected_tasks: u64,
    pub coverage_entries: u64,
    pub annotations_total: u64,
    pub annotations_per_image: u64,
    pub generated_file_bytes_per_image: u64,
    pub keypoints_per_skeleton: u64,
    pub yolo_line_bytes: u64,
    pub yolo_columns: u64,
    pub structured_data_nesting: u64,
    pub decoded_image_pixels: u64,
    pub decoded_image_bytes: u64,
    pub staged_bytes: u64,
    pub diagnostic_examples_per_code: u64,
}

impl Default for ImportLimitsFileConfig {
    fn default() -> Self {
        Self {
            concurrent_build_jobs: 1,
            image_validation_workers: 8,
            decoded_image_memory_bytes: 5 * 1024 * 1024 * 1024,
            concurrent_browser_upload_jobs: 2,
            active_reservations_per_owner: 2,
            browser_source_files: 25_000,
            browser_source_bytes: 20 * 1024 * 1024 * 1024,
            server_source_files: 50_000,
            total_source_bytes: 100 * 1024 * 1024 * 1024,
            selected_images: 10_000,
            single_source_file_bytes: 4 * 1024 * 1024 * 1024,
            descriptor_bytes: 16 * 1024 * 1024,
            upload_chunk_bytes: 8 * 1024 * 1024,
            source_path_bytes: 1024,
            source_path_depth: 32,
            source_component_bytes: 255,
            selected_categories: 100,
            selected_tasks: 200,
            coverage_entries: 2_000_000,
            annotations_total: 1_000_000,
            annotations_per_image: 10_000,
            generated_file_bytes_per_image: 64 * 1024 * 1024,
            keypoints_per_skeleton: 512,
            yolo_line_bytes: 1024 * 1024,
            yolo_columns: 4096,
            structured_data_nesting: 64,
            decoded_image_pixels: 50_000_000,
            decoded_image_bytes: 512 * 1024 * 1024,
            staged_bytes: 250 * 1024 * 1024 * 1024,
            diagnostic_examples_per_code: 100,
        }
    }
}

impl ImportLimitsFileConfig {
    pub fn validate(&self) -> Result<(), ConfigValidationError> {
        let values = [
            ("concurrentBuildJobs", self.concurrent_build_jobs),
            ("imageValidationWorkers", self.image_validation_workers),
            ("decodedImageMemoryBytes", self.decoded_image_memory_bytes),
            (
                "concurrentBrowserUploadJobs",
                self.concurrent_browser_upload_jobs,
            ),
            (
                "activeReservationsPerOwner",
                self.active_reservations_per_owner,
            ),
            ("browserSourceFiles", self.browser_source_files),
            ("browserSourceBytes", self.browser_source_bytes),
            ("serverSourceFiles", self.server_source_files),
            ("totalSourceBytes", self.total_source_bytes),
            ("selectedImages", self.selected_images),
            ("singleSourceFileBytes", self.single_source_file_bytes),
            ("descriptorBytes", self.descriptor_bytes),
            ("uploadChunkBytes", self.upload_chunk_bytes),
            ("sourcePathBytes", self.source_path_bytes),
            ("sourcePathDepth", self.source_path_depth),
            ("sourceComponentBytes", self.source_component_bytes),
            ("selectedCategories", self.selected_categories),
            ("selectedTasks", self.selected_tasks),
            ("coverageEntries", self.coverage_entries),
            ("annotationsTotal", self.annotations_total),
            ("annotationsPerImage", self.annotations_per_image),
            (
                "generatedFileBytesPerImage",
                self.generated_file_bytes_per_image,
            ),
            ("keypointsPerSkeleton", self.keypoints_per_skeleton),
            ("yoloLineBytes", self.yolo_line_bytes),
            ("yoloColumns", self.yolo_columns),
            ("structuredDataNesting", self.structured_data_nesting),
            ("decodedImagePixels", self.decoded_image_pixels),
            ("decodedImageBytes", self.decoded_image_bytes),
            ("stagedBytes", self.staged_bytes),
            (
                "diagnosticExamplesPerCode",
                self.diagnostic_examples_per_code,
            ),
        ];
        if let Some((name, _)) = values.into_iter().find(|(_, value)| *value == 0) {
            return Err(ConfigValidationError::new(format!(
                "import.limits.{name} must be greater than zero"
            )));
        }
        if self.image_validation_workers
            > u64::try_from(MAX_IMAGE_VALIDATION_WORKERS)
                .expect("maximum image validation workers fits in u64")
        {
            return Err(ConfigValidationError::new(
                "import.limits.imageValidationWorkers exceeds the supported maximum",
            ));
        }

        validate_limit_order(
            self.browser_source_bytes,
            "browserSourceBytes",
            self.total_source_bytes,
            "totalSourceBytes",
        )?;
        validate_limit_order(
            self.single_source_file_bytes,
            "singleSourceFileBytes",
            self.total_source_bytes,
            "totalSourceBytes",
        )?;
        validate_limit_order(
            self.descriptor_bytes,
            "descriptorBytes",
            self.single_source_file_bytes,
            "singleSourceFileBytes",
        )?;
        validate_limit_order(
            self.upload_chunk_bytes,
            "uploadChunkBytes",
            self.single_source_file_bytes,
            "singleSourceFileBytes",
        )?;
        validate_limit_order(
            self.source_component_bytes,
            "sourceComponentBytes",
            self.source_path_bytes,
            "sourcePathBytes",
        )?;
        validate_limit_order(
            self.source_path_depth,
            "sourcePathDepth",
            self.source_path_bytes,
            "sourcePathBytes",
        )?;
        validate_limit_order(
            self.annotations_per_image,
            "annotationsPerImage",
            self.annotations_total,
            "annotationsTotal",
        )?;
        validate_limit_order(
            self.generated_file_bytes_per_image,
            "generatedFileBytesPerImage",
            self.staged_bytes,
            "stagedBytes",
        )?;
        validate_limit_order(
            self.yolo_columns,
            "yoloColumns",
            self.yolo_line_bytes,
            "yoloLineBytes",
        )?;
        let minimum_image_memory = self
            .decoded_image_bytes
            .checked_mul(2)
            .and_then(|decoded| decoded.checked_add(self.single_source_file_bytes))
            .ok_or_else(|| {
                ConfigValidationError::new("import image validation memory limits overflow")
            })?;
        validate_limit_order(
            minimum_image_memory,
            "singleSourceFileBytes plus twice decodedImageBytes",
            self.decoded_image_memory_bytes,
            "decodedImageMemoryBytes",
        )?;
        validate_limit_order(
            self.total_source_bytes,
            "totalSourceBytes",
            self.staged_bytes,
            "stagedBytes",
        )?;

        for (name, value) in [
            ("selectedCategories", self.selected_categories),
            ("selectedTasks", self.selected_tasks),
            ("annotationsPerImage", self.annotations_per_image),
            ("keypointsPerSkeleton", self.keypoints_per_skeleton),
        ] {
            if value > u64::from(u32::MAX) {
                return Err(ConfigValidationError::new(format!(
                    "import.limits.{name} exceeds the import capability range"
                )));
            }
        }

        for (name, value) in [
            ("concurrentBuildJobs", self.concurrent_build_jobs),
            ("imageValidationWorkers", self.image_validation_workers),
            (
                "concurrentBrowserUploadJobs",
                self.concurrent_browser_upload_jobs,
            ),
            (
                "activeReservationsPerOwner",
                self.active_reservations_per_owner,
            ),
            ("browserSourceFiles", self.browser_source_files),
            ("serverSourceFiles", self.server_source_files),
            ("selectedImages", self.selected_images),
            ("uploadChunkBytes", self.upload_chunk_bytes),
            ("sourcePathBytes", self.source_path_bytes),
            ("sourcePathDepth", self.source_path_depth),
            ("sourceComponentBytes", self.source_component_bytes),
            ("selectedCategories", self.selected_categories),
            ("selectedTasks", self.selected_tasks),
            ("coverageEntries", self.coverage_entries),
            ("annotationsTotal", self.annotations_total),
            ("annotationsPerImage", self.annotations_per_image),
            ("keypointsPerSkeleton", self.keypoints_per_skeleton),
            ("yoloLineBytes", self.yolo_line_bytes),
            ("yoloColumns", self.yolo_columns),
            ("structuredDataNesting", self.structured_data_nesting),
            (
                "diagnosticExamplesPerCode",
                self.diagnostic_examples_per_code,
            ),
        ] {
            if usize::try_from(value).is_err() {
                return Err(ConfigValidationError::new(format!(
                    "import.limits.{name} exceeds this platform's usize range"
                )));
            }
        }
        Ok(())
    }
}

fn validate_limit_order(
    lower: u64,
    lower_name: &str,
    upper: u64,
    upper_name: &str,
) -> Result<(), ConfigValidationError> {
    if lower > upper {
        return Err(ConfigValidationError::new(format!(
            "import.limits.{lower_name} cannot exceed import.limits.{upper_name}"
        )));
    }
    Ok(())
}

pub fn validate_import_root_ids<'a>(
    ids: impl IntoIterator<Item = &'a str>,
) -> Result<(), ConfigValidationError> {
    let mut unique = BTreeSet::new();
    for id in ids {
        if id.is_empty()
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            || !unique.insert(id)
        {
            return Err(ConfigValidationError::new(
                "import root IDs must be unique safe opaque IDs",
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigValidationError(String);

impl ConfigValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for ConfigValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ConfigValidationError {}

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigParseError {
    category: String,
    location: Option<(usize, usize)>,
}

impl ConfigParseError {
    fn new(category: impl Into<String>, location: Option<(usize, usize)>) -> Self {
        Self {
            category: category.into(),
            location,
        }
    }
}

impl std::fmt::Display for ConfigParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.category)?;
        if let Some((line, column)) = self.location {
            write!(formatter, " at line {line}, column {column}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ConfigParseError {}

fn line_column(source: &str, byte_offset: usize) -> (usize, usize) {
    let prefix = &source[..byte_offset.min(source.len())];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix.len(), |(_, tail)| tail.len())
        + 1;
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_schema_rejects_unknown_fields_and_unsafe_paths() {
        assert!(BrowserConfig::parse(r#"{"apiBaseUrl":"https://example.com/api/"}"#).is_ok());
        for text in [
            r#"{"apiBaseUrl":"../admin"}"#,
            r#"{"apiBaseUrl":"https://example.com/api"}"#,
            r#"{"apiBaseUrl":"https://example.com/?secret=value"}"#,
            r#"{"apiUrl":"https://example.com/"}"#,
        ] {
            assert!(BrowserConfig::parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn server_errors_never_include_source_text() {
        let secret = "do-not-print-this-secret";
        let source = format!("clientSecret = {secret}\n");
        let error = ServerConfig::parse(&source).unwrap_err().to_string();
        assert!(!error.contains(secret));
        assert!(error.contains("line 1"), "{error}");
    }

    #[test]
    fn server_schema_rejects_unsafe_bootstrap_admins() {
        let source = toml::to_string(&ServerConfig {
            bootstrap_admins: vec!["../admin".to_string()],
            ..ServerConfig::default()
        })
        .unwrap();
        assert!(ServerConfig::parse(&source).is_err());
    }

    #[test]
    fn import_root_ids_are_opaque_and_unique() {
        assert!(validate_import_root_ids(["root-1", "root_2"]).is_ok());
        for ids in [vec!["bad.root"], vec!["duplicate", "duplicate"]] {
            assert!(validate_import_root_ids(ids).is_err());
        }

        let config = ServerConfig {
            import: Some(ImportFileConfig {
                enabled: true,
                server_roots: vec![ImportRootFileConfig {
                    id: "bad.root".to_owned(),
                    path: "/srv/import".to_owned(),
                    allowed_owners: Vec::new(),
                }],
                retain_raw_source: false,
                failed_retention_hours: 24,
                successful_metadata_retention_days: 30,
                limits: ImportLimitsFileConfig::default(),
            }),
            ..ServerConfig::default()
        };
        let source = toml::to_string(&config).unwrap();
        assert!(ServerConfig::parse(&source).is_err());
    }

    #[test]
    fn server_parse_rejects_invalid_import_limit_semantics() {
        let config = ServerConfig {
            import: Some(ImportFileConfig {
                enabled: true,
                server_roots: Vec::new(),
                retain_raw_source: false,
                failed_retention_hours: 24,
                successful_metadata_retention_days: 30,
                limits: ImportLimitsFileConfig {
                    image_validation_workers: 0,
                    ..ImportLimitsFileConfig::default()
                },
            }),
            ..ServerConfig::default()
        };
        let source = toml::to_string(&config).unwrap();
        let error = ServerConfig::parse(&source).unwrap_err().to_string();
        assert_eq!(
            error,
            "import.limits.imageValidationWorkers must be greater than zero"
        );
    }
}
