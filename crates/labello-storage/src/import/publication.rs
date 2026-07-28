use super::*;

impl ImportService {
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
}
