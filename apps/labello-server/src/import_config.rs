use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::Duration,
};

use anyhow::Context;
use labello_config::{ImportFileConfig, ImportLimitsFileConfig};
use labello_domain::UserId;
use labello_storage::{ImportConfig, ImportLimits, ImportRoot};

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
    config.validate()?;

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

fn usize_import_limit(value: u64, name: &str) -> anyhow::Result<usize> {
    usize::try_from(value)
        .with_context(|| format!("import.limits.{name} exceeds this platform's usize range"))
}
