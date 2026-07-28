pub(super) fn seal_output(output: &Path, job: &ImportJob, plan: &ImportPlan) -> StorageResult<()> {
    sync_tree(output)?;
    let sentinel = CompletionSentinel {
        schema_version: SCHEMA_VERSION,
        import_id: job.import_id.clone(),
        dataset_id: job.destination_dataset_id.clone(),
        source_fingerprint: plan.source_fingerprint.clone(),
        plan_hash: plan.plan_hash.clone(),
    };
    let path = output.join(COMPLETION_SENTINEL);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_path(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(&sentinel).map_err(|source| StorageError::Json {
        path: path.clone(),
        source,
    })?;
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .with_path(&path)?;
    file.write_all(&bytes).with_path(&path)?;
    file.write_all(b"\n").with_path(&path)?;
    file.sync_all().with_path(&path)?;
    sync_directory(path.parent().expect("sentinel parent"))?;
    sync_directory(output)
}

pub(super) async fn published_matches(destination: &Path, job: &ImportJob) -> StorageResult<bool> {
    let sentinel_path = destination.join(COMPLETION_SENTINEL);
    if !tokio::fs::try_exists(&sentinel_path)
        .await
        .with_path(&sentinel_path)?
    {
        return Ok(false);
    }
    let sentinel: CompletionSentinel = read_json(&sentinel_path).await?;
    if sentinel.import_id != job.import_id
        || sentinel.dataset_id != job.destination_dataset_id
        || Some(sentinel.source_fingerprint.as_str()) != job.source_fingerprint.as_deref()
        || Some(sentinel.plan_hash.as_str()) != job.plan_hash.as_deref()
    {
        return Ok(false);
    }
    let manifest_path = destination
        .join(paths::IMPORTS_DIR)
        .join(job.import_id.as_str())
        .join(paths::IMPORT_MANIFEST_FILE);
    let manifest: ImportManifest = read_json(&manifest_path).await?;
    Ok(manifest.import_id == job.import_id
        && manifest.dataset_id == job.destination_dataset_id
        && Some(manifest.plan_hash.as_str()) == job.plan_hash.as_deref())
}

pub(super) async fn sealed_output_matches(output: &Path, job: &ImportJob) -> StorageResult<bool> {
    let path = output.join(COMPLETION_SENTINEL);
    if !tokio::fs::try_exists(&path).await.with_path(&path)? {
        return Ok(false);
    }
    let sentinel: CompletionSentinel = read_json(&path).await?;
    Ok(sentinel.import_id == job.import_id
        && sentinel.dataset_id == job.destination_dataset_id
        && Some(sentinel.source_fingerprint.as_str()) == job.source_fingerprint.as_deref()
        && Some(sentinel.plan_hash.as_str()) == job.plan_hash.as_deref())
}
