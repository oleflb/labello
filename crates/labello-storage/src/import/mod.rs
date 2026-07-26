mod builder;
mod formats;
mod image_validation;
mod ir;
mod source;
mod types;

#[cfg(test)]
mod tests;

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
    capabilities: ImportCapabilities,
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
            capabilities,
            #[cfg(test)]
            fail_create_after_reservation: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }

    pub fn capabilities(&self) -> &ImportCapabilities {
        &self.capabilities
    }

    pub async fn create_job(
        &self,
        owner: UserId,
        request: CreateImportRequest,
    ) -> StorageResult<ImportJob> {
        self.require_available()?;
        validate_destination_id(&request.destination_dataset_id)?;
        if request.destination_name.trim().is_empty() || request.destination_name.len() > 512 {
            return Err(import_error(
                "destination_name_invalid",
                "destination name is invalid",
            ));
        }
        if !self.config.allowed_profiles.contains(&request.profile) {
            return Err(import_error(
                "profile_disabled",
                "source profile is not enabled",
            ));
        }
        let _guard = self.mutation_lock.lock().await;
        if tokio::fs::try_exists(
            self.datasets_root
                .join(request.destination_dataset_id.as_str()),
        )
        .await
        .with_path(&*self.datasets_root)?
        {
            return Err(import_error(
                "destination_exists",
                "destination dataset already exists",
            ));
        }
        let jobs = self.list_jobs_internal().await?;
        let active_for_owner = jobs
            .iter()
            .filter(|job| job.owner_user_id == owner && !job.phase.terminal())
            .count();
        if active_for_owner >= self.config.limits.active_reservations_per_owner {
            return Err(import_error(
                "reservation_limit",
                "owner has too many active reservations",
            ));
        }
        if request.transport == ImportTransport::Browser
            && jobs
                .iter()
                .filter(|job| {
                    job.transport == ImportTransport::Browser
                        && matches!(
                            job.phase,
                            ImportJobPhase::Registering | ImportJobPhase::Uploading
                        )
                })
                .count()
                >= self.config.limits.concurrent_browser_upload_jobs
        {
            return Err(import_error(
                "upload_concurrency_limit",
                "too many browser upload jobs are active",
            ));
        }
        self.create_reservation(&request.destination_dataset_id, &owner)
            .await?;
        let destination_dataset_id = request.destination_dataset_id.clone();
        #[cfg(test)]
        if self
            .fail_create_after_reservation
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            self.release_reservation(&destination_dataset_id).await?;
            return Err(import_error(
                "injected_create_failure",
                "injected failure after reservation creation",
            ));
        }
        let import_id = ImportId::from(format!("imp_{}", uuid::Uuid::new_v4().simple()));
        let timestamp = now();
        let job = ImportJob {
            schema_version: SCHEMA_VERSION,
            import_id: import_id.clone(),
            owner_user_id: owner,
            destination_dataset_id: request.destination_dataset_id,
            destination_name: request.destination_name,
            profile: request.profile,
            transport: request.transport,
            phase: ImportJobPhase::Registering,
            source_fingerprint: None,
            plan_hash: None,
            preflight_generation: None,
            accepted_files: 0,
            accepted_bytes: 0,
            created_at: timestamp,
            updated_at: timestamp,
            failure_code: None,
        };
        let job_dir = self.job_dir(&import_id);
        let create_result = async {
            tokio::fs::create_dir_all(job_dir.join(source::SOURCE_DIR))
                .await
                .with_path(&job_dir)?;
            tokio::fs::create_dir_all(job_dir.join("spool"))
                .await
                .with_path(&job_dir)?;
            tokio::fs::create_dir_all(job_dir.join("diagnostics"))
                .await
                .with_path(&job_dir)?;
            set_private_permissions(&job_dir)?;
            write_json_atomic(&job_dir.join(JOB_FILE), &job).await?;
            source::save_source_index(&job_dir, &SourceIndex::default()).await?;
            Ok(job.clone())
        }
        .await;
        if create_result.is_err() {
            let _ = remove_if_exists(&job_dir).await;
            let _ = self.release_reservation(&destination_dataset_id).await;
        }
        create_result
    }

    pub async fn job(&self, import_id: &ImportId, owner: &UserId) -> StorageResult<ImportJob> {
        let job = self.load_job(import_id).await?;
        require_owner(&job, owner)?;
        Ok(job)
    }

    pub async fn register_browser_files(
        &self,
        import_id: &ImportId,
        owner: &UserId,
        registrations: Vec<BrowserFileRegistration>,
    ) -> StorageResult<Vec<RegisteredFile>> {
        let mut job = self.load_owned_job(import_id, owner).await?;
        if job.transport != ImportTransport::Browser
            || !matches!(
                job.phase,
                ImportJobPhase::Registering | ImportJobPhase::Uploading
            )
        {
            return Err(import_error(
                "job_phase_invalid",
                "job is not accepting browser files",
            ));
        }
        let job_dir = self.job_dir(import_id);
        let mut index = source::load_source_index(&job_dir).await?;
        let files = source::register_browser_files(
            &job_dir,
            &mut index,
            registrations,
            &self.config.limits,
        )
        .await?;
        job.phase = ImportJobPhase::Uploading;
        update_job_counts(&mut job, &index);
        self.save_job(&job).await?;
        Ok(files)
    }

    pub async fn upload_chunk(
        &self,
        import_id: &ImportId,
        owner: &UserId,
        file_id: &str,
        offset: u64,
        bytes: &[u8],
        blake3: &str,
    ) -> StorageResult<RegisteredFile> {
        let mut job = self.load_owned_job(import_id, owner).await?;
        if job.transport != ImportTransport::Browser || job.phase != ImportJobPhase::Uploading {
            return Err(import_error(
                "job_phase_invalid",
                "job is not accepting upload chunks",
            ));
        }
        let job_dir = self.job_dir(import_id);
        let mut index = source::load_source_index(&job_dir).await?;
        let file = source::upload_chunk(
            &job_dir,
            &mut index,
            file_id,
            offset,
            bytes,
            blake3,
            &self.config.limits,
        )
        .await?;
        update_job_counts(&mut job, &index);
        self.save_job(&job).await?;
        Ok(file)
    }

    pub async fn copy_server_directory(
        &self,
        import_id: &ImportId,
        owner: &UserId,
        selection: ServerDirectorySelection,
    ) -> StorageResult<ImportJob> {
        let mut job = self.load_owned_job(import_id, owner).await?;
        if job.transport != ImportTransport::ServerDirectory
            || job.phase != ImportJobPhase::Registering
        {
            return Err(import_error(
                "job_phase_invalid",
                "job is not accepting a server directory",
            ));
        }
        let root = self
            .config
            .import_roots
            .iter()
            .find(|root| root.root_id == selection.root_id)
            .ok_or_else(|| {
                import_error(
                    "import_root_missing",
                    "configured import root does not exist",
                )
            })?;
        let root_handle = self
            .import_root_handles
            .get(&selection.root_id)
            .ok_or_else(|| {
                import_error(
                    "import_root_missing",
                    "configured import root handle does not exist",
                )
            })?;
        if !root.allowed_owners.is_empty() && !root.allowed_owners.contains(owner) {
            return Err(import_error(
                "import_root_forbidden",
                "owner cannot use this import root",
            ));
        }
        let job_dir = self.job_dir(import_id);
        let index = source::copy_server_directory(
            &job_dir,
            &root.path,
            root_handle,
            &selection,
            &self.config.limits,
        )?;
        source::save_source_index(&job_dir, &index).await?;
        job.phase = ImportJobPhase::Uploading;
        update_job_counts(&mut job, &index);
        self.save_job(&job).await?;
        Ok(job)
    }

    pub async fn browse_server_root(
        &self,
        root_id: &str,
        owner: &UserId,
        relative_directory: &str,
        offset: usize,
    ) -> StorageResult<ImportBrowsePage> {
        self.require_available()?;
        let root = self
            .config
            .import_roots
            .iter()
            .find(|root| root.root_id == root_id)
            .ok_or_else(|| {
                import_error(
                    "import_root_missing",
                    "configured import root does not exist",
                )
            })?;
        if !root.allowed_owners.is_empty() && !root.allowed_owners.contains(owner) {
            return Err(import_error(
                "import_root_forbidden",
                "owner cannot use this import root",
            ));
        }
        let root_handle = self
            .import_root_handles
            .get(root_id)
            .cloned()
            .ok_or_else(|| {
                import_error(
                    "import_root_missing",
                    "configured import root does not exist",
                )
            })?;
        let relative_directory = relative_directory.to_string();
        let limits = self.config.limits.clone();
        tokio::task::spawn_blocking(move || {
            source::browse_server_directory(&root_handle, &relative_directory, offset, &limits)
        })
        .await
        .map_err(|_| {
            import_error(
                "server_source_browse_failed",
                "server source browser failed",
            )
        })?
    }

    pub async fn browse_staged_source(
        &self,
        import_id: &ImportId,
        owner: &UserId,
        relative_directory: &str,
        offset: usize,
        mode: ImportBrowseMode,
    ) -> StorageResult<ImportBrowsePage> {
        let job = self.load_owned_job(import_id, owner).await?;
        if !matches!(
            job.phase,
            ImportJobPhase::Uploading | ImportJobPhase::Sealed
        ) {
            return Err(import_error(
                "job_phase_invalid",
                "job source cannot be browsed in this phase",
            ));
        }
        let index = source::load_source_index(&self.job_dir(import_id)).await?;
        source::browse_staged_source(
            &index,
            job.profile,
            relative_directory,
            offset,
            mode,
            &self.config.limits,
        )
    }

    pub async fn inspect_yolo_descriptor(
        &self,
        import_id: &ImportId,
        owner: &UserId,
        descriptor_path: &str,
    ) -> StorageResult<YoloDescriptorInspection> {
        let job = self.load_owned_job(import_id, owner).await?;
        if !matches!(
            job.phase,
            ImportJobPhase::Uploading | ImportJobPhase::Sealed
        ) {
            return Err(import_error(
                "job_phase_invalid",
                "job is not accepting descriptor inspection",
            ));
        }
        if !matches!(
            job.profile,
            ImportProfile::UltralyticsYoloDetectV1 | ImportProfile::UltralyticsYoloPoseV1
        ) {
            return Err(import_error(
                "yolo_profile_mismatch",
                "descriptor inspection requires a YOLO import profile",
            ));
        }
        let job_dir = self.job_dir(import_id);
        let index = source::load_source_index(&job_dir).await?;
        let limits = self.config.limits.clone();
        let descriptor_path = descriptor_path.to_string();
        let worker_permit = self
            .descriptor_inspection_workers
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                import_error(
                    "descriptor_inspection_busy",
                    "too many descriptor inspections are already running",
                )
            })?;
        match tokio::time::timeout(
            DESCRIPTOR_INSPECTION_TIME_LIMIT,
            tokio::task::spawn_blocking(move || {
                let _worker_permit = worker_permit;
                let source = source::SourceAccess::new(&job_dir, &index);
                formats::inspect_yolo_descriptor(&source, &descriptor_path, &limits)
            }),
        )
        .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(import_error(
                "parser_worker_failed",
                "descriptor parser worker terminated unexpectedly",
            )),
            Err(_) => Err(import_error(
                "parser_time_limit",
                "descriptor parsing exceeded the parser time budget",
            )),
        }
    }

    pub async fn seal(&self, import_id: &ImportId, owner: &UserId) -> StorageResult<ImportJob> {
        let mut job = self.load_owned_job(import_id, owner).await?;
        if !matches!(
            job.phase,
            ImportJobPhase::Uploading | ImportJobPhase::Sealed
        ) {
            return Err(import_error(
                "job_phase_invalid",
                "job source cannot be sealed in this phase",
            ));
        }
        let job_dir = self.job_dir(import_id);
        let mut index = source::load_source_index(&job_dir).await?;
        let parser_version_migration = if job.phase == ImportJobPhase::Sealed {
            validate_sealed_source_anchor(&job, &index)?
        } else {
            false
        };
        let fingerprint = source::seal_source(&job_dir, &mut index, job.profile.id()).await?;
        if job.phase == ImportJobPhase::Sealed
            && job.source_fingerprint.as_deref() != Some(&fingerprint)
            && !parser_version_migration
        {
            return Err(import_error(
                "source_changed",
                "sealed source no longer matches the import job",
            ));
        }
        job.phase = ImportJobPhase::Sealed;
        job.source_fingerprint = Some(fingerprint);
        update_job_counts(&mut job, &index);
        self.save_job(&job).await?;
        Ok(job)
    }

    pub async fn preflight(
        &self,
        import_id: &ImportId,
        owner: &UserId,
        request: PreflightRequest,
    ) -> StorageResult<ImportPlan> {
        let preflight_started = Instant::now();
        let mut job = self.load_owned_job(import_id, owner).await?;
        if !matches!(
            job.phase,
            ImportJobPhase::Sealed | ImportJobPhase::AwaitingDecision
        ) {
            return Err(import_error(
                "job_phase_invalid",
                "job is not ready for preflight",
            ));
        }
        if !request.ground_truth_attested {
            return Err(import_error(
                "ground_truth_attestation_required",
                "ground-truth attestation is required",
            ));
        }
        let source_verification_started = Instant::now();
        let mut source_index = source::load_source_index(&self.job_dir(import_id)).await?;
        let parser_version_migration = validate_sealed_source_anchor(&job, &source_index)?;
        let verified_fingerprint = source::seal_source(
            &self.job_dir(import_id),
            &mut source_index,
            job.profile.id(),
        )
        .await?;
        if job.source_fingerprint.as_deref() != Some(&verified_fingerprint) {
            if !parser_version_migration {
                return Err(import_error(
                    "source_changed",
                    "sealed source no longer matches the import job",
                ));
            }
            job.source_fingerprint = Some(verified_fingerprint);
            job.plan_hash = None;
            job.preflight_generation = None;
        }
        let source_verification_ms = elapsed_ms(source_verification_started);
        let source_file_count = source_index.files.len();
        let source_byte_count = source_index
            .files
            .values()
            .map(|file| file.byte_size)
            .sum::<u64>();
        let generation = uuid::Uuid::new_v4().simple().to_string();
        job.phase = ImportJobPhase::Preflighting;
        job.plan_hash = None;
        job.preflight_generation = Some(generation.clone());
        job.failure_code = None;
        self.save_job(&job).await?;
        let parse_job_dir = self.job_dir(import_id);
        let parse_job = job.clone();
        let parse_limits = self.config.limits.clone();
        let parser_cancelled = Arc::new(AtomicBool::new(false));
        let _cancellation_guard = ParserCancellationGuard(parser_cancelled.clone());
        let worker_cancelled = parser_cancelled.clone();
        let mut parser_worker = tokio::task::spawn_blocking(move || {
            formats::preflight(
                &parse_job_dir,
                &source_index,
                &parse_job,
                request,
                &parse_limits,
                &worker_cancelled,
            )
        });
        let result = match tokio::time::timeout(PREFLIGHT_TIME_LIMIT, &mut parser_worker).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(import_error(
                "parser_worker_failed",
                "import parser worker terminated unexpectedly",
            )),
            Err(_) => {
                parser_cancelled.store(true, Ordering::Relaxed);
                let _ = parser_worker.await;
                Err(import_error(
                    "parser_time_limit",
                    "import parsing exceeded the parser time budget",
                ))
            }
        };
        match result {
            Ok(output) => {
                let formats::PreflightOutput { plan, ir, timings } = output;
                let artifact_persistence_started = Instant::now();
                write_json_atomic(
                    &self.job_dir(import_id).join(PREFLIGHT_FILE),
                    &PreflightArtifacts {
                        generation,
                        plan: plan.clone(),
                        ir,
                    },
                )
                .await?;
                job.phase = ImportJobPhase::AwaitingDecision;
                job.plan_hash = Some(plan.plan_hash.clone());
                self.save_job(&job).await?;
                let artifact_persistence_ms = elapsed_ms(artifact_persistence_started);
                tracing::info!(
                    event = "import.preflight.phases",
                    import_id = %job.import_id,
                    profile = job.profile.id(),
                    elapsed_ms = elapsed_ms(preflight_started),
                    source_verification_ms,
                    parse_ms = timings.parse_ms,
                    semantic_validation_ms = timings.semantic_validation_ms,
                    plan_assembly_ms = timings.plan_assembly_ms,
                    plan_hash_ms = timings.plan_hash_ms,
                    artifact_persistence_ms,
                    source_file_count,
                    source_byte_count,
                    image_count = plan.totals.images,
                    source_object_count = plan.totals.source_objects,
                    output_annotation_count = plan.totals.output_annotations,
                    "import preflight phases completed"
                );
                Ok(plan)
            }
            Err(error) => {
                job.phase = ImportJobPhase::Sealed;
                job.plan_hash = None;
                job.preflight_generation = None;
                job.failure_code = Some(match &error {
                    StorageError::Import { code, .. } => code.clone(),
                    _ => error.kind().to_string(),
                });
                self.save_job(&job).await?;
                Err(error)
            }
        }
    }

    pub async fn plan(&self, import_id: &ImportId, owner: &UserId) -> StorageResult<ImportPlan> {
        let job = self.load_owned_job(import_id, owner).await?;
        if job.phase != ImportJobPhase::AwaitingDecision {
            return Err(import_error(
                "job_phase_invalid",
                "job does not have a current preflight plan",
            ));
        }
        let plan = self.load_artifacts(&job).await?.plan;
        if job.source_fingerprint.as_deref() != Some(plan.source_fingerprint.as_str())
            || job.plan_hash.as_deref() != Some(plan.plan_hash.as_str())
        {
            return Err(import_error(
                "plan_stale",
                "stored plan does not match the import job",
            ));
        }
        Ok(plan)
    }

    pub async fn update_plan(
        &self,
        import_id: &ImportId,
        owner: &UserId,
        request: PreflightRequest,
    ) -> StorageResult<ImportPlan> {
        self.preflight(import_id, owner, request).await
    }

    pub async fn diagnostics(
        &self,
        import_id: &ImportId,
        owner: &UserId,
        offset: usize,
        limit: usize,
    ) -> StorageResult<Vec<ImportDiagnostic>> {
        self.load_owned_job(import_id, owner).await?;
        let plan = self
            .load_artifacts(&self.load_owned_job(import_id, owner).await?)
            .await?
            .plan;
        Ok(plan
            .diagnostics
            .into_iter()
            .skip(offset)
            .take(limit.min(1000))
            .collect())
    }

    pub async fn commit(
        &self,
        import_id: &ImportId,
        owner: &UserId,
        plan_hash: &str,
    ) -> StorageResult<ImportCommitResult> {
        let mut job = self.load_owned_job(import_id, owner).await?;
        if job.phase == ImportJobPhase::Succeeded {
            if job.plan_hash.as_deref() != Some(plan_hash) {
                return Err(import_error(
                    "plan_stale",
                    "commit does not match the published preflight plan",
                ));
            }
            return self.committed_result(&job, true).await;
        }
        if job.phase != ImportJobPhase::AwaitingDecision
            || job.plan_hash.as_deref() != Some(plan_hash)
        {
            return Err(import_error(
                "plan_stale",
                "commit does not match the current preflight plan",
            ));
        }
        let plan = self.load_artifacts(&job).await?.plan;
        if plan.plan_hash != plan_hash || !plan.committable() {
            return Err(import_error(
                "plan_not_committable",
                "preflight plan has unresolved diagnostics",
            ));
        }
        let mut source_index = source::load_source_index(&self.job_dir(import_id)).await?;
        let verified_fingerprint = source::seal_source(
            &self.job_dir(import_id),
            &mut source_index,
            job.profile.id(),
        )
        .await?;
        if source_index.source_fingerprint.as_deref() != job.source_fingerprint.as_deref()
            || plan.source_fingerprint != job.source_fingerprint.as_deref().unwrap_or_default()
            || job.source_fingerprint.as_deref() != Some(&verified_fingerprint)
        {
            return Err(import_error(
                "source_changed",
                "sealed source fingerprint no longer matches the plan",
            ));
        }
        let _active_guard = {
            let mut active = self.active_builds.lock();
            if active.contains(import_id)
                || active.len() >= self.config.limits.concurrent_build_jobs
            {
                return Err(import_error(
                    "build_concurrency_limit",
                    "another import build is active",
                ));
            }
            active.insert(import_id.clone());
            ActiveBuildGuard {
                active: self.active_builds.clone(),
                import_id: import_id.clone(),
            }
        };
        let result = self.commit_inner(&mut job, &plan, owner).await;
        match result {
            Ok(result) => Ok(result),
            Err(error) => {
                let destination = self.datasets_root.join(job.destination_dataset_id.as_str());
                match builder::published_matches(&destination, &job).await {
                    Ok(true) if source::sync_directory(&self.datasets_root).is_ok() => {
                        job.phase = ImportJobPhase::Succeeded;
                        self.save_job(&job).await?;
                        self.release_reservation(&job.destination_dataset_id)
                            .await?;
                        return self.committed_result(&job, true).await;
                    }
                    Ok(true) | Err(_) => {
                        // Leave the persisted committing phase and reservation for startup recovery.
                        return Err(error);
                    }
                    Ok(false) if job.phase == ImportJobPhase::Committing => {
                        // Publication can be retried from the verified, sealed output.
                        return Err(error);
                    }
                    Ok(false) => {}
                }
                job.phase = ImportJobPhase::Failed;
                job.failure_code = Some(match &error {
                    StorageError::Import { code, .. } => code.clone(),
                    _ => error.kind().to_string(),
                });
                self.save_job(&job).await?;
                self.release_reservation(&job.destination_dataset_id)
                    .await?;
                Err(error)
            }
        }
    }

    async fn commit_inner(
        &self,
        job: &mut ImportJob,
        plan: &ImportPlan,
        owner: &UserId,
    ) -> StorageResult<ImportCommitResult> {
        job.phase = ImportJobPhase::Building;
        self.save_job(job).await?;
        let ir = self.load_artifacts(job).await?.ir;
        builder::build(
            &self.job_dir(&job.import_id),
            job,
            plan,
            &ir,
            owner,
            &self.config.limits,
        )
        .await?;
        job.phase = ImportJobPhase::Verifying;
        self.save_job(job).await?;
        builder::verify(&self.job_dir(&job.import_id).join("output"), job, plan).await?;
        builder::seal_output(&self.job_dir(&job.import_id).join("output"), job, plan)?;
        job.phase = ImportJobPhase::Committing;
        self.save_job(job).await?;
        let _guard = self.mutation_lock.lock().await;
        self.verify_reservation(job).await?;
        publish_no_replace(
            &self.datasets_root_handle,
            &self.datasets_root,
            &self.job_dir(&job.import_id).join("output"),
            &job.destination_dataset_id,
        )?;
        job.phase = ImportJobPhase::Succeeded;
        self.save_job(job).await?;
        self.release_reservation(&job.destination_dataset_id)
            .await?;
        if !self.config.retain_raw_source {
            remove_if_exists(&self.job_dir(&job.import_id).join(source::SOURCE_DIR)).await?;
            remove_if_exists(&self.job_dir(&job.import_id).join("spool")).await?;
        }
        self.committed_result(job, false).await
    }

    pub async fn cancel(&self, import_id: &ImportId, owner: &UserId) -> StorageResult<ImportJob> {
        let mut job = self.load_owned_job(import_id, owner).await?;
        if matches!(
            job.phase,
            ImportJobPhase::Committing | ImportJobPhase::Succeeded
        ) {
            return Err(import_error(
                "job_not_cancellable",
                "commit has begun and cannot be cancelled",
            ));
        }
        if job.phase != ImportJobPhase::Cancelled {
            job.phase = ImportJobPhase::Cancelled;
            self.save_job(&job).await?;
            for child in [source::SOURCE_DIR, "spool", "diagnostics", "output"] {
                remove_if_exists(&self.job_dir(import_id).join(child)).await?;
            }
            self.release_reservation(&job.destination_dataset_id)
                .await?;
        }
        Ok(job)
    }

    pub async fn recover(&self) -> StorageResult<RecoveryReport> {
        let _guard = self.mutation_lock.lock().await;
        let mut report = RecoveryReport::default();
        for mut job in self.list_jobs_internal().await? {
            let destination = self.datasets_root.join(job.destination_dataset_id.as_str());
            let publish_phase = matches!(
                job.phase,
                ImportJobPhase::Committing | ImportJobPhase::Verifying | ImportJobPhase::Building
            );
            if publish_phase && builder::published_matches(&destination, &job).await? {
                source::sync_directory(&self.datasets_root)?;
                job.phase = ImportJobPhase::Succeeded;
                self.save_job(&job).await?;
                self.release_reservation(&job.destination_dataset_id)
                    .await?;
                report.recovered_successes += 1;
                continue;
            }
            if job.phase == ImportJobPhase::Committing {
                let artifacts = self.load_artifacts(&job).await?;
                let output = self.job_dir(&job.import_id).join("output");
                builder::verify(&output, &job, &artifacts.plan).await?;
                if !builder::sealed_output_matches(&output, &job).await? {
                    return Err(import_error(
                        "incomplete_commit_recovery",
                        "committing output is not sealed for the current import plan",
                    ));
                }
                self.verify_reservation(&job).await?;
                publish_no_replace(
                    &self.datasets_root_handle,
                    &self.datasets_root,
                    &output,
                    &job.destination_dataset_id,
                )?;
                job.phase = ImportJobPhase::Succeeded;
                self.save_job(&job).await?;
                self.release_reservation(&job.destination_dataset_id)
                    .await?;
                report.recovered_successes += 1;
                continue;
            }
            let protected_build_phase = matches!(
                job.phase,
                ImportJobPhase::Building | ImportJobPhase::Verifying | ImportJobPhase::Committing
            );
            if !job.phase.terminal()
                && !protected_build_phase
                && now()
                    .signed_duration_since(job.updated_at)
                    .to_std()
                    .unwrap_or_default()
                    >= self.config.failed_retention
            {
                job.phase = ImportJobPhase::Expired;
                self.save_job(&job).await?;
                self.active_builds.lock().remove(&job.import_id);
                self.release_reservation(&job.destination_dataset_id)
                    .await?;
                remove_if_exists(&self.job_dir(&job.import_id)).await?;
                report.expired_abandoned_jobs += 1;
            } else if matches!(
                job.phase,
                ImportJobPhase::Preflighting | ImportJobPhase::Building | ImportJobPhase::Verifying
            ) {
                match self.load_artifacts(&job).await {
                    Ok(artifacts) => {
                        job.plan_hash = Some(artifacts.plan.plan_hash);
                        job.phase = ImportJobPhase::AwaitingDecision;
                        report.resumed_to_awaiting_decision += 1;
                    }
                    Err(_) => {
                        job.phase = ImportJobPhase::Sealed;
                        job.plan_hash = None;
                        job.preflight_generation = None;
                    }
                }
                remove_if_exists(&self.job_dir(&job.import_id).join("output")).await?;
                self.save_job(&job).await?;
            }
        }
        let active = self
            .list_jobs_internal()
            .await?
            .into_iter()
            .filter(|job| !job.phase.terminal())
            .map(|job| job.destination_dataset_id)
            .collect::<BTreeSet<_>>();
        let reservations = self.staging_root().join(RESERVATIONS_DIR);
        let mut entries = tokio::fs::read_dir(&reservations)
            .await
            .with_path(&reservations)?;
        while let Some(entry) = entries.next_entry().await.with_path(&reservations)? {
            if !entry.file_type().await.with_path(entry.path())?.is_file() {
                continue;
            }
            let value: serde_json::Value = read_json(&entry.path()).await?;
            let Some(dataset_id) = value
                .get("datasetId")
                .and_then(serde_json::Value::as_str)
                .map(DatasetId::from)
            else {
                continue;
            };
            if !active.contains(&dataset_id) {
                self.release_reservation(&dataset_id).await?;
                report.released_reservations += 1;
            }
        }
        Ok(report)
    }

    pub async fn cleanup_expired(
        &self,
        timestamp: labello_domain::Timestamp,
    ) -> StorageResult<usize> {
        let _guard = self.mutation_lock.lock().await;
        let mut cleaned = 0;
        for mut job in self.list_jobs_internal().await? {
            let retention = if job.phase == ImportJobPhase::Succeeded {
                self.config.successful_metadata_retention
            } else {
                self.config.failed_retention
            };
            let age = timestamp
                .signed_duration_since(job.updated_at)
                .to_std()
                .unwrap_or_default();
            if matches!(
                job.phase,
                ImportJobPhase::Building | ImportJobPhase::Verifying | ImportJobPhase::Committing
            ) {
                continue;
            }
            if age >= retention {
                if !job.phase.terminal() && self.active_builds.lock().contains(&job.import_id) {
                    continue;
                }
                job.phase = ImportJobPhase::Expired;
                self.save_job(&job).await?;
                self.active_builds.lock().remove(&job.import_id);
                self.release_reservation(&job.destination_dataset_id)
                    .await?;
                remove_if_exists(&self.job_dir(&job.import_id)).await?;
                cleaned += 1;
            }
        }
        Ok(cleaned)
    }

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
