pub(super) async fn verify(output: &Path, job: &ImportJob, plan: &ImportPlan) -> StorageResult<()> {
    let repository = DatasetRepository::new(output);
    let metadata = repository.load_dataset().await?;
    if metadata.dataset_id != job.destination_dataset_id
        || metadata.schema_version != SCHEMA_VERSION
    {
        return Err(import_error(
            "output_identity_mismatch",
            "generated dataset identity is invalid",
        ));
    }
    let index = repository.load_images_index().await?;
    if index.images_by_hash.len() != plan.totals.images {
        return Err(import_error(
            "output_count_mismatch",
            "generated image count differs from the plan",
        ));
    }
    let mut annotations = 0_usize;
    for record in index.images_by_hash.values() {
        let events = repository.load_events(&record.image_id).await?;
        let replayed = rebuild_state(record.image_id.clone(), &events)?;
        let stored: labello_domain::ImageState =
            read_json(&repository.state_path(&record.image_id)).await?;
        if replayed != stored {
            return Err(import_error(
                "state_replay_mismatch",
                "generated state does not equal event replay",
            ));
        }
        for annotation in replayed.active_annotations() {
            let task = metadata.task(&annotation.task_id).ok_or_else(|| {
                import_error(
                    "output_task_reference",
                    "annotation references a missing task",
                )
            })?;
            annotation.validate_for_task(task, record.dimensions())?;
            annotations += 1;
        }
    }
    if annotations != plan.totals.output_annotations {
        return Err(import_error(
            "output_count_mismatch",
            "verified annotation count differs from the plan",
        ));
    }
    let manifests = repository.load_import_manifests().await?;
    if manifests.len() != 1
        || manifests[0].import_id != job.import_id
        || manifests[0].plan_hash != plan.plan_hash
    {
        return Err(import_error(
            "output_manifest_mismatch",
            "generated import manifest is invalid",
        ));
    }
    let manifest_path = output
        .join(paths::IMPORTS_DIR)
        .join(job.import_id.as_str())
        .join(paths::IMPORT_MANIFEST_FILE);
    let manifest_value: serde_json::Value = read_json(&manifest_path).await?;
    let integrity = manifest_value
        .get("outputIntegrity")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| {
            import_error(
                "output_manifest_mismatch",
                "import manifest has no output integrity map",
            )
        })?;
    for (relative, digest) in integrity {
        let relative_path = Path::new(relative);
        if relative_path.is_absolute()
            || relative_path
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err(import_error(
                "output_manifest_mismatch",
                "output integrity path is not a safe relative path",
            ));
        }
        let digest = digest.as_str().ok_or_else(|| {
            import_error(
                "output_manifest_mismatch",
                "output integrity digest is invalid",
            )
        })?;
        if super::source::hash_file(&output.join(relative_path))? != digest {
            return Err(import_error(
                "output_integrity_mismatch",
                "generated output file does not match its manifest digest",
            ));
        }
    }
    Ok(())
}
