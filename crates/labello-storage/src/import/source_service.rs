use super::*;

impl ImportService {
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
}
