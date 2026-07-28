use super::*;

impl ImportService {
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

    pub async fn registered_files(
        &self,
        import_id: &ImportId,
        owner: &UserId,
    ) -> StorageResult<Vec<RegisteredFile>> {
        self.load_owned_job(import_id, owner).await?;
        Ok(source::load_source_index(&self.job_dir(import_id))
            .await?
            .files
            .into_values()
            .collect())
    }

    pub async fn find_matching_job(
        &self,
        owner: &UserId,
        request: &CreateImportRequest,
    ) -> StorageResult<Option<ImportJob>> {
        Ok(self.list_jobs_internal().await?.into_iter().find(|job| {
            &job.owner_user_id == owner
                && job.destination_dataset_id == request.destination_dataset_id
                && job.destination_name == request.destination_name
                && job.profile == request.profile
                && job.transport == request.transport
                && !matches!(
                    job.phase,
                    ImportJobPhase::Failed | ImportJobPhase::Cancelled | ImportJobPhase::Expired
                )
        }))
    }
}
