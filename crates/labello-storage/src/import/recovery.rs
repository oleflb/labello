use super::*;

impl ImportService {
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
}
