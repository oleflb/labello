pub(super) async fn seal_source(
    job_dir: &Path,
    index: &mut SourceIndex,
    profile_id: &str,
) -> StorageResult<String> {
    let previous_fingerprint = index.source_fingerprint.clone();
    let previous_parser_version = index
        .parser_version
        .as_deref()
        .unwrap_or("labello-storage-import-v1");
    if index.files.is_empty() || index.files.values().any(|file| !file.complete) {
        return Err(import_error(
            "source_incomplete",
            "every registered source file must be complete before sealing",
        ));
    }
    let mut ordered = index.files.values().collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    for file in &ordered {
        let path = job_dir.join(SOURCE_DIR).join(&file.file_id);
        if std::fs::metadata(&path).with_path(&path)?.len() != file.byte_size
            || hash_file(&path)? != file.blake3
        {
            return Err(import_error(
                "source_file_digest_mismatch",
                "staged source changed before sealing",
            ));
        }
    }
    if index.sealed {
        let expected_previous = source_fingerprint(&ordered, profile_id, previous_parser_version)?;
        if previous_fingerprint.as_deref() != Some(&expected_previous) {
            return Err(import_error(
                "source_changed",
                "sealed source fingerprint changed",
            ));
        }
    }
    let fingerprint =
        source_fingerprint(&ordered, profile_id, super::types::IMPORT_PARSER_VERSION)?;
    index.sealed = true;
    index.source_fingerprint = Some(fingerprint.clone());
    index.parser_version = Some(super::types::IMPORT_PARSER_VERSION.to_string());
    save_source_index(job_dir, index).await?;
    Ok(fingerprint)
}

pub(super) fn source_fingerprint(
    ordered: &[&RegisteredFile],
    profile_id: &str,
    parser_version: &str,
) -> StorageResult<String> {
    let mut hasher = Hasher::new();
    hasher.update(b"labello:import-source:v1\0");
    hash_string(&mut hasher, profile_id);
    hash_string(&mut hasher, parser_version);
    for file in ordered {
        hash_string(&mut hasher, &file.relative_path);
        hasher.update(&file.byte_size.to_be_bytes());
        let digest = blake3::Hash::from_hex(&file.blake3)
            .map_err(|_| import_error("digest_invalid", "stored source digest is invalid"))?;
        hasher.update(digest.as_bytes());
    }
    Ok(hasher.finalize().to_hex().to_string())
}

pub(super) async fn load_source_index(job_dir: &Path) -> StorageResult<SourceIndex> {
    let path = job_dir.join(SOURCE_INDEX_FILE);
    if !tokio::fs::try_exists(&path).await.with_path(&path)? {
        return Ok(SourceIndex::default());
    }
    crate::fsjson::read_json(&path).await
}

pub(super) async fn save_source_index(job_dir: &Path, index: &SourceIndex) -> StorageResult<()> {
    write_json_atomic(&job_dir.join(SOURCE_INDEX_FILE), index).await
}
