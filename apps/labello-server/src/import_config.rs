use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::Duration,
};

use anyhow::{Context, bail};
use labello_domain::UserId;
use labello_storage::{ImportConfig, ImportLimits, ImportRoot};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ImportFileConfig {
    pub(crate) enabled: bool,
    pub(crate) server_roots: Vec<ImportRootFileConfig>,
    pub(crate) retain_raw_source: bool,
    pub(crate) failed_retention_hours: u64,
    pub(crate) successful_metadata_retention_days: u64,
    #[serde(default)]
    pub(crate) limits: ImportLimitsFileConfig,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ImportLimitsFileConfig {
    pub(crate) concurrent_build_jobs: u64,
    pub(crate) image_validation_workers: u64,
    pub(crate) decoded_image_memory_bytes: u64,
    pub(crate) concurrent_browser_upload_jobs: u64,
    pub(crate) active_reservations_per_owner: u64,
    pub(crate) browser_source_files: u64,
    pub(crate) browser_source_bytes: u64,
    pub(crate) server_source_files: u64,
    pub(crate) total_source_bytes: u64,
    pub(crate) selected_images: u64,
    pub(crate) single_source_file_bytes: u64,
    pub(crate) descriptor_bytes: u64,
    pub(crate) upload_chunk_bytes: u64,
    pub(crate) source_path_bytes: u64,
    pub(crate) source_path_depth: u64,
    pub(crate) source_component_bytes: u64,
    pub(crate) selected_categories: u64,
    pub(crate) selected_tasks: u64,
    pub(crate) coverage_entries: u64,
    pub(crate) annotations_total: u64,
    pub(crate) annotations_per_image: u64,
    pub(crate) generated_file_bytes_per_image: u64,
    pub(crate) keypoints_per_skeleton: u64,
    pub(crate) yolo_line_bytes: u64,
    pub(crate) yolo_columns: u64,
    pub(crate) structured_data_nesting: u64,
    pub(crate) decoded_image_pixels: u64,
    pub(crate) decoded_image_bytes: u64,
    pub(crate) staged_bytes: u64,
    pub(crate) diagnostic_examples_per_code: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ImportRootFileConfig {
    pub(crate) id: String,
    pub(crate) path: String,
    pub(crate) allowed_owners: Vec<String>,
}

impl Default for ImportLimitsFileConfig {
    fn default() -> Self {
        let limits = ImportLimits::default();
        Self {
            concurrent_build_jobs: u64::try_from(limits.concurrent_build_jobs)
                .expect("default concurrent_build_jobs fits in u64"),
            image_validation_workers: u64::try_from(limits.image_validation_workers)
                .expect("default image_validation_workers fits in u64"),
            decoded_image_memory_bytes: limits.decoded_image_memory_bytes,
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

pub(crate) fn import_root_owners(
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

pub(crate) fn storage_import_config(
    config: Option<&ImportFileConfig>,
) -> anyhow::Result<ImportConfig> {
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

pub(crate) fn storage_import_limits(
    config: &ImportLimitsFileConfig,
) -> anyhow::Result<ImportLimits> {
    let values = [
        ("concurrentBuildJobs", config.concurrent_build_jobs),
        ("imageValidationWorkers", config.image_validation_workers),
        ("decodedImageMemoryBytes", config.decoded_image_memory_bytes),
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
    if config.image_validation_workers
        > u64::try_from(labello_storage::MAX_IMAGE_VALIDATION_WORKERS)
            .expect("maximum image validation workers fits in u64")
    {
        bail!("import.limits.imageValidationWorkers exceeds the supported maximum");
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
    let minimum_image_memory = config
        .decoded_image_bytes
        .checked_mul(2)
        .and_then(|decoded| decoded.checked_add(config.single_source_file_bytes))
        .ok_or_else(|| anyhow::anyhow!("import image validation memory limits overflow"))?;
    validate_limit_order(
        minimum_image_memory,
        "singleSourceFileBytes plus twice decodedImageBytes",
        config.decoded_image_memory_bytes,
        "decodedImageMemoryBytes",
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
        image_validation_workers: usize_import_limit(
            config.image_validation_workers,
            "imageValidationWorkers",
        )?,
        decoded_image_memory_bytes: config.decoded_image_memory_bytes,
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
