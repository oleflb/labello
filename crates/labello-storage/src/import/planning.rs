use super::*;

impl ImportService {
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
        let decoded_image_memory = self.decoded_image_memory.clone();
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
                &decoded_image_memory,
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
}
