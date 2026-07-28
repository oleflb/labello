mod builder;
mod control_store;
mod formats;
mod image_validation;
mod ir;
mod jobs;
mod planning;
mod publication;
mod recovery;
mod source;
mod source_service;
mod types;

#[cfg(test)]
mod tests;

pub use control_store::ImportControlStore;
pub use types::*;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use labello_domain::{DatasetId, ImportId, SCHEMA_VERSION, UserId, now};
use parking_lot::Mutex;

use crate::{
    error::{PathIo, StorageError, StorageResult},
    fsjson::{read_json, write_json_atomic},
};

use self::source::{SourceIndex, import_error};

const SERVER_STATE_DIR: &str = ".labello-server";
const IMPORT_STAGING_DIR: &str = "imports";
const JOBS_DIR: &str = "jobs";
const RESERVATIONS_DIR: &str = "reservations";
const JOB_FILE: &str = "job.json";
const PREFLIGHT_FILE: &str = "spool/preflight-artifacts.json";
const PREFLIGHT_TIME_LIMIT: std::time::Duration = std::time::Duration::from_secs(30 * 60);
const DESCRIPTOR_INSPECTION_TIME_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);
const MAX_CONCURRENT_DESCRIPTOR_INSPECTIONS: usize = 2;

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreflightArtifacts {
    generation: String,
    plan: ImportPlan,
    ir: ir::ImportIr,
}

#[derive(Clone)]
pub struct ImportService {
    datasets_root: Arc<PathBuf>,
    datasets_display_root: Arc<PathBuf>,
    config: Arc<ImportConfig>,
    datasets_root_handle: Arc<File>,
    import_root_handles: Arc<BTreeMap<String, Arc<File>>>,
    mutation_lock: Arc<tokio::sync::Mutex<()>>,
    active_builds: Arc<Mutex<BTreeSet<ImportId>>>,
    descriptor_inspection_workers: Arc<tokio::sync::Semaphore>,
    decoded_image_memory: Arc<image_validation::DecodedImageMemoryLimiter>,
    capabilities: ImportCapabilities,
    control_store: ImportControlStore,
    #[cfg(test)]
    fail_create_after_reservation: Arc<std::sync::atomic::AtomicBool>,
}

struct ActiveBuildGuard {
    active: Arc<Mutex<BTreeSet<ImportId>>>,
    import_id: ImportId,
}

struct ParserCancellationGuard(Arc<AtomicBool>);

impl Drop for ParserCancellationGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

impl Drop for ActiveBuildGuard {
    fn drop(&mut self) {
        self.active.lock().remove(&self.import_id);
    }
}

impl ImportService {
    pub async fn new(
        datasets_root: impl Into<PathBuf>,
        config: ImportConfig,
    ) -> StorageResult<Self> {
        let datasets_root = datasets_root.into();
        tokio::fs::create_dir_all(&datasets_root)
            .await
            .with_path(&datasets_root)?;
        validate_config(&datasets_root, &config)?;
        let datasets_root = std::fs::canonicalize(&datasets_root).with_path(&datasets_root)?;
        let staging = datasets_root
            .join(SERVER_STATE_DIR)
            .join(IMPORT_STAGING_DIR);
        let control_store = ImportControlStore::new(&staging);
        for directory in [staging.join(JOBS_DIR), staging.join(RESERVATIONS_DIR)] {
            tokio::fs::create_dir_all(&directory)
                .await
                .with_path(&directory)?;
            set_private_permissions(&directory)?;
        }
        let (atomic_publication, secure_server_open, reason) = probe_platform(&datasets_root)?;
        let datasets_display_root = datasets_root.clone();
        let datasets_root_handle = Arc::new(File::open(&datasets_root).with_path(&datasets_root)?);
        let datasets_root = pinned_directory_path(&datasets_root_handle, &datasets_root);
        let import_root_handles = config
            .import_roots
            .iter()
            .map(|root| {
                let path = std::fs::canonicalize(&root.path).with_path(&root.path)?;
                Ok((
                    root.root_id.clone(),
                    Arc::new(File::open(&path).with_path(&path)?),
                ))
            })
            .collect::<StorageResult<BTreeMap<_, _>>>()?;
        let available = config.enabled && atomic_publication && secure_server_open;
        let capabilities = ImportCapabilities {
            available,
            unavailable_reason: if available {
                None
            } else if !config.enabled {
                Some("dataset import is disabled by configuration".to_string())
            } else {
                reason
            },
            profiles: if available {
                config.allowed_profiles.clone()
            } else {
                Vec::new()
            },
            browser_upload: available,
            server_directory_roots: if available {
                config
                    .import_roots
                    .iter()
                    .map(|root| root.root_id.clone())
                    .collect()
            } else {
                Vec::new()
            },
            limits: config.limits.clone(),
            schema_version: SCHEMA_VERSION,
            parser_version: IMPORT_PARSER_VERSION.to_string(),
            atomic_publication,
            secure_server_open,
        };
        let decoded_image_memory = Arc::new(image_validation::DecodedImageMemoryLimiter::new(
            config.limits.decoded_image_memory_bytes,
        ));
        Ok(Self {
            datasets_root: Arc::new(datasets_root),
            datasets_display_root: Arc::new(datasets_display_root),
            config: Arc::new(config),
            datasets_root_handle,
            import_root_handles: Arc::new(import_root_handles),
            mutation_lock: Arc::new(tokio::sync::Mutex::new(())),
            active_builds: Arc::new(Mutex::new(BTreeSet::new())),
            descriptor_inspection_workers: Arc::new(tokio::sync::Semaphore::new(
                MAX_CONCURRENT_DESCRIPTOR_INSPECTIONS,
            )),
            decoded_image_memory,
            capabilities,
            control_store,
            #[cfg(test)]
            fail_create_after_reservation: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }

    pub fn capabilities(&self) -> &ImportCapabilities {
        &self.capabilities
    }

    pub fn control_store(&self) -> &ImportControlStore {
        &self.control_store
    }
}

impl ImportService {
    fn require_available(&self) -> StorageResult<()> {
        if self.capabilities.available {
            Ok(())
        } else {
            Err(import_error(
                "import_unavailable",
                self.capabilities
                    .unavailable_reason
                    .clone()
                    .unwrap_or_else(|| "dataset import is unavailable".to_string()),
            ))
        }
    }

    fn staging_root(&self) -> PathBuf {
        self.datasets_root
            .join(SERVER_STATE_DIR)
            .join(IMPORT_STAGING_DIR)
    }

    fn job_dir(&self, import_id: &ImportId) -> PathBuf {
        self.staging_root().join(JOBS_DIR).join(import_id.as_str())
    }

    fn reservation_path(&self, dataset_id: &DatasetId) -> PathBuf {
        self.staging_root()
            .join(RESERVATIONS_DIR)
            .join(format!("{}.json", dataset_id.as_str()))
    }

    async fn load_job(&self, import_id: &ImportId) -> StorageResult<ImportJob> {
        import_id
            .validate_path_segment()
            .map_err(|_| import_error("import_id_invalid", "import ID is invalid"))?;
        read_json(&self.job_dir(import_id).join(JOB_FILE)).await
    }

    async fn load_owned_job(
        &self,
        import_id: &ImportId,
        owner: &UserId,
    ) -> StorageResult<ImportJob> {
        let job = self.load_job(import_id).await?;
        require_owner(&job, owner)?;
        Ok(job)
    }

    async fn save_job(&self, job: &ImportJob) -> StorageResult<()> {
        let mut job = job.clone();
        job.updated_at = now();
        write_json_atomic(&self.job_dir(&job.import_id).join(JOB_FILE), &job).await
    }

    async fn load_artifacts(&self, job: &ImportJob) -> StorageResult<PreflightArtifacts> {
        let artifacts: PreflightArtifacts =
            read_json(&self.job_dir(&job.import_id).join(PREFLIGHT_FILE)).await?;
        let generation_matches =
            job.preflight_generation.as_deref() == Some(artifacts.generation.as_str());
        let identity_matches = artifacts.plan.import_id == job.import_id
            && artifacts.plan.destination_dataset_id == job.destination_dataset_id
            && job.source_fingerprint.as_deref()
                == Some(artifacts.plan.source_fingerprint.as_str());
        let hash_matches = job
            .plan_hash
            .as_deref()
            .is_none_or(|hash| hash == artifacts.plan.plan_hash);
        let (class_ids, task_ids) = formats::planned_ids(&artifacts.ir, &artifacts.plan.request);
        if !generation_matches
            || !identity_matches
            || !hash_matches
            || class_ids != artifacts.plan.class_ids
            || task_ids != artifacts.plan.task_ids
        {
            return Err(import_error(
                "preflight_artifacts_stale",
                "preflight plan and IR generation do not match the import job",
            ));
        }
        Ok(artifacts)
    }

    async fn list_jobs_internal(&self) -> StorageResult<Vec<ImportJob>> {
        let directory = self.staging_root().join(JOBS_DIR);
        let mut reader = tokio::fs::read_dir(&directory)
            .await
            .with_path(&directory)?;
        let mut jobs = Vec::new();
        while let Some(entry) = reader.next_entry().await.with_path(&directory)? {
            if entry.file_type().await.with_path(entry.path())?.is_dir() {
                let path = entry.path().join(JOB_FILE);
                if tokio::fs::try_exists(&path).await.with_path(&path)? {
                    jobs.push(read_json(&path).await?);
                }
            }
        }
        Ok(jobs)
    }

    async fn create_reservation(
        &self,
        dataset_id: &DatasetId,
        owner: &UserId,
    ) -> StorageResult<()> {
        let path = self.reservation_path(dataset_id);
        let bytes = serde_json::to_vec_pretty(&serde_json::json!({
            "schemaVersion": SCHEMA_VERSION,
            "datasetId": dataset_id,
            "ownerUserId": owner,
            "createdAt": now(),
        }))
        .map_err(|source| StorageError::Json {
            path: path.clone(),
            source,
        })?;
        let mut file = match tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .await
        {
            Ok(file) => file,
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(import_error(
                    "destination_reserved",
                    "destination dataset is already reserved",
                ));
            }
            Err(source) => return Err(StorageError::Io { path, source }),
        };
        use tokio::io::AsyncWriteExt;
        file.write_all(&bytes).await.with_path(&path)?;
        file.write_all(b"\n").await.with_path(&path)?;
        file.sync_all().await.with_path(&path)?;
        sync_parent(&path)?;
        Ok(())
    }

    async fn verify_reservation(&self, job: &ImportJob) -> StorageResult<()> {
        let value: serde_json::Value =
            read_json(&self.reservation_path(&job.destination_dataset_id)).await?;
        if value.get("ownerUserId").and_then(serde_json::Value::as_str)
            != Some(job.owner_user_id.as_str())
        {
            return Err(import_error(
                "reservation_lost",
                "destination reservation does not match the import owner",
            ));
        }
        if tokio::fs::try_exists(self.datasets_root.join(job.destination_dataset_id.as_str()))
            .await
            .with_path(&*self.datasets_root)?
        {
            return Err(import_error(
                "destination_exists",
                "destination dataset appeared before publication",
            ));
        }
        Ok(())
    }

    async fn release_reservation(&self, dataset_id: &DatasetId) -> StorageResult<()> {
        let path = self.reservation_path(dataset_id);
        if tokio::fs::try_exists(&path).await.with_path(&path)? {
            tokio::fs::remove_file(&path).await.with_path(&path)?;
            sync_parent(&path)?;
        }
        Ok(())
    }

    async fn committed_result(
        &self,
        job: &ImportJob,
        recovered: bool,
    ) -> StorageResult<ImportCommitResult> {
        let path = self.datasets_root.join(job.destination_dataset_id.as_str());
        if !builder::published_matches(&path, job).await? {
            return Err(import_error(
                "published_dataset_mismatch",
                "published dataset does not match the import job",
            ));
        }
        Ok(ImportCommitResult {
            import_id: job.import_id.clone(),
            dataset_id: job.destination_dataset_id.clone(),
            dataset_path: self
                .datasets_display_root
                .join(job.destination_dataset_id.as_str()),
            recovered,
        })
    }
}

#[cfg(target_os = "linux")]
fn pinned_directory_path(handle: &File, _fallback: &Path) -> PathBuf {
    use std::os::fd::AsRawFd;
    PathBuf::from(format!("/proc/self/fd/{}", handle.as_raw_fd()))
}

#[cfg(not(target_os = "linux"))]
fn pinned_directory_path(_handle: &File, fallback: &Path) -> PathBuf {
    fallback.to_path_buf()
}

fn update_job_counts(job: &mut ImportJob, index: &SourceIndex) {
    job.accepted_files = index.files.values().filter(|file| file.complete).count();
    job.accepted_bytes = index.files.values().map(|file| file.accepted_bytes).sum();
}

fn require_owner(job: &ImportJob, owner: &UserId) -> StorageResult<()> {
    if &job.owner_user_id == owner {
        Ok(())
    } else {
        Err(import_error(
            "import_owner_mismatch",
            "import job belongs to another owner",
        ))
    }
}

fn validate_destination_id(dataset_id: &DatasetId) -> StorageResult<()> {
    dataset_id.validate_path_segment().map_err(|_| {
        import_error(
            "destination_id_invalid",
            "destination dataset ID is invalid",
        )
    })?;
    let value = dataset_id.as_str();
    if value.starts_with('.')
        || value.starts_with("tmp-")
        || value.starts_with("import-")
        || value == SERVER_STATE_DIR
    {
        return Err(import_error(
            "destination_id_reserved",
            "destination dataset ID is reserved",
        ));
    }
    Ok(())
}

fn validate_sealed_source_anchor(job: &ImportJob, index: &SourceIndex) -> StorageResult<bool> {
    if !index.sealed {
        return Err(import_error(
            "source_changed",
            "sealed import job has an unsealed source index",
        ));
    }
    if job.source_fingerprint == index.source_fingerprint {
        return Ok(index
            .parser_version
            .as_deref()
            .unwrap_or("labello-storage-import-v1")
            != IMPORT_PARSER_VERSION);
    }
    if index.parser_version.as_deref() == Some(IMPORT_PARSER_VERSION) {
        let mut ordered = index.files.values().collect::<Vec<_>>();
        ordered.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        let legacy =
            source::source_fingerprint(&ordered, job.profile.id(), "labello-storage-import-v1")?;
        if job.source_fingerprint.as_deref() == Some(&legacy) {
            return Ok(true);
        }
    }
    Err(import_error(
        "source_changed",
        "sealed source no longer matches the import job",
    ))
}

fn validate_config(datasets_root: &Path, config: &ImportConfig) -> StorageResult<()> {
    if config.limits.image_validation_workers == 0
        || config.limits.image_validation_workers > MAX_IMAGE_VALIDATION_WORKERS
    {
        return Err(import_error(
            "import_limit_invalid",
            "image validation workers must be within the supported range",
        ));
    }
    let minimum_image_memory = config
        .limits
        .decoded_image_bytes
        .checked_mul(2)
        .and_then(|decoded| decoded.checked_add(config.limits.single_source_file_bytes));
    if minimum_image_memory.is_none_or(|minimum| config.limits.decoded_image_memory_bytes < minimum)
    {
        return Err(import_error(
            "import_limit_invalid",
            "decoded image memory budget must cover one maximum-size animated image",
        ));
    }
    let datasets = std::fs::canonicalize(datasets_root).with_path(datasets_root)?;
    let server_state = datasets.join(SERVER_STATE_DIR);
    let mut roots = Vec::new();
    let mut ids = BTreeSet::new();
    for root in &config.import_roots {
        if root.root_id.is_empty()
            || !root
                .root_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            || !ids.insert(root.root_id.clone())
        {
            return Err(import_error(
                "import_root_invalid",
                "import root IDs must be unique safe opaque IDs",
            ));
        }
        let canonical = std::fs::canonicalize(&root.path).with_path(&root.path)?;
        if !canonical.is_dir()
            || canonical.starts_with(&datasets)
            || datasets.starts_with(&canonical)
            || canonical.starts_with(&server_state)
        {
            return Err(import_error(
                "import_root_overlap",
                "import roots cannot overlap datasets or server state",
            ));
        }
        if roots.iter().any(|existing: &PathBuf| {
            canonical.starts_with(existing) || existing.starts_with(&canonical)
        }) {
            return Err(import_error(
                "import_root_overlap",
                "configured import roots cannot overlap one another",
            ));
        }
        roots.push(canonical);
    }
    if config.allowed_profiles.is_empty()
        || config
            .allowed_profiles
            .iter()
            .collect::<BTreeSet<_>>()
            .len()
            != config.allowed_profiles.len()
    {
        return Err(import_error(
            "profile_config_invalid",
            "allowed import profiles must be unique and nonempty",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> StorageResult<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).with_path(path)
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> StorageResult<()> {
    Ok(())
}

fn sync_parent(path: &Path) -> StorageResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| import_error("filesystem_sync", "path has no parent directory"))?;
    source::sync_directory(parent)
}

async fn remove_if_exists(path: &Path) -> StorageResult<()> {
    if tokio::fs::try_exists(path).await.with_path(path)? {
        let metadata = tokio::fs::symlink_metadata(path).await.with_path(path)?;
        if metadata.is_dir() {
            tokio::fs::remove_dir_all(path).await.with_path(path)?;
        } else {
            tokio::fs::remove_file(path).await.with_path(path)?;
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn probe_platform(datasets_root: &Path) -> StorageResult<(bool, bool, Option<String>)> {
    use std::io::Write;
    let probe = datasets_root
        .join(SERVER_STATE_DIR)
        .join(IMPORT_STAGING_DIR)
        .join(format!(".probe-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir(&probe).with_path(&probe)?;
    let source = probe.join("source");
    std::fs::create_dir(&source).with_path(&source)?;
    let mut regular =
        std::fs::File::create(source.join("regular")).with_path(source.join("regular"))?;
    regular
        .write_all(b"probe")
        .with_path(source.join("regular"))?;
    regular.sync_all().with_path(source.join("regular"))?;
    let root_handle = std::fs::File::open(&probe).with_path(&probe)?;
    use rustix::fs::{Mode, OFlags, RenameFlags, ResolveFlags, openat2, renameat_with};
    let secure = openat2(
        &root_handle,
        "source/regular",
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .is_ok();
    let atomic = renameat_with(
        &root_handle,
        "source",
        &root_handle,
        "destination",
        RenameFlags::NOREPLACE,
    )
    .is_ok();
    source::sync_directory(&probe)?;
    std::fs::remove_dir_all(&probe).with_path(&probe)?;
    Ok((atomic, secure, (!atomic || !secure).then(|| "filesystem does not support required Linux beneath-open and no-replace publication primitives".to_string())))
}

#[cfg(not(target_os = "linux"))]
fn probe_platform(_datasets_root: &Path) -> StorageResult<(bool, bool, Option<String>)> {
    Ok((
        false,
        false,
        Some("first-release import requires Linux openat2 and renameat2 support".to_string()),
    ))
}

#[cfg(target_os = "linux")]
fn publish_no_replace(
    root: &File,
    datasets_root: &Path,
    output: &Path,
    dataset_id: &DatasetId,
) -> StorageResult<()> {
    use rustix::fs::{RenameFlags, renameat_with};
    let relative = output.strip_prefix(datasets_root).map_err(|_| {
        import_error(
            "publication_cross_filesystem",
            "staged output is not below datasets root",
        )
    })?;
    renameat_with(
        root,
        relative,
        root,
        dataset_id.as_str(),
        RenameFlags::NOREPLACE,
    )
    .map_err(|source| {
        if source == rustix::io::Errno::EXIST {
            import_error("destination_exists", "destination dataset already exists")
        } else {
            StorageError::Io {
                path: output.to_path_buf(),
                source: std::io::Error::from_raw_os_error(source.raw_os_error()),
            }
        }
    })?;
    source::sync_directory(datasets_root)
}

#[cfg(not(target_os = "linux"))]
fn publish_no_replace(
    _root: &File,
    _datasets_root: &Path,
    _output: &Path,
    _dataset_id: &DatasetId,
) -> StorageResult<()> {
    Err(import_error(
        "publication_unsupported",
        "atomic no-replace publication is unavailable",
    ))
}
