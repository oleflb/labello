pub(super) async fn register_browser_files(
    job_dir: &Path,
    index: &mut SourceIndex,
    registrations: Vec<BrowserFileRegistration>,
    limits: &ImportLimits,
) -> StorageResult<Vec<RegisteredFile>> {
    if index.sealed {
        return Err(import_error(
            "source_sealed",
            "sealed source cannot be changed",
        ));
    }
    if index.files.len().saturating_add(registrations.len()) > limits.browser_source_files {
        return Err(import_error(
            "source_file_limit",
            "browser source file count exceeds the configured limit",
        ));
    }
    let mut path_keys = index
        .files
        .values()
        .map(|file| collision_key(&file.relative_path))
        .collect::<BTreeSet<_>>();
    let mut total = index.files.values().map(|file| file.byte_size).sum::<u64>();
    let mut registered = Vec::with_capacity(registrations.len());
    for registration in registrations {
        validate_source_path(&registration.relative_path, limits)?;
        validate_digest(&registration.blake3)?;
        if registration.byte_size > limits.single_source_file_bytes {
            return Err(import_error(
                "source_file_too_large",
                "registered source file exceeds the configured limit",
            ));
        }
        total = total.checked_add(registration.byte_size).ok_or_else(|| {
            import_error(
                "source_byte_limit",
                "registered source byte count overflowed",
            )
        })?;
        if total > limits.browser_source_bytes
            || total > limits.total_source_bytes
            || total > limits.staged_bytes
        {
            return Err(import_error(
                "source_byte_limit",
                "browser source bytes exceed the configured limit",
            ));
        }
        if !path_keys.insert(collision_key(&registration.relative_path)) {
            return Err(import_error(
                "source_path_collision",
                "source paths collide after case and Unicode normalization",
            ));
        }
        let file = RegisteredFile {
            file_id: format!("file_{}", uuid::Uuid::new_v4().simple()),
            relative_path: registration.relative_path,
            byte_size: registration.byte_size,
            blake3: registration.blake3.to_ascii_lowercase(),
            accepted_bytes: 0,
            complete: false,
            accepted_chunks: BTreeMap::new(),
        };
        index.files.insert(file.file_id.clone(), file.clone());
        registered.push(file);
    }
    save_source_index(job_dir, index).await?;
    Ok(registered)
}

pub(super) async fn upload_chunk(
    job_dir: &Path,
    index: &mut SourceIndex,
    file_id: &str,
    offset: u64,
    bytes: &[u8],
    digest: &str,
    limits: &ImportLimits,
) -> StorageResult<RegisteredFile> {
    if index.sealed {
        return Err(import_error(
            "source_sealed",
            "sealed source cannot be changed",
        ));
    }
    if bytes.is_empty() || bytes.len() > limits.upload_chunk_bytes {
        return Err(import_error(
            "upload_chunk_limit",
            "upload chunk length is outside the configured bounds",
        ));
    }
    validate_digest(digest)?;
    let actual = blake3::hash(bytes).to_hex().to_string();
    if actual != digest.to_ascii_lowercase() {
        return Err(import_error(
            "upload_chunk_digest_mismatch",
            "upload chunk digest does not match its bytes",
        ));
    }
    let file = index.files.get_mut(file_id).ok_or_else(|| {
        import_error(
            "source_file_missing",
            "registered source file does not exist",
        )
    })?;
    let source_dir = job_dir.join(SOURCE_DIR);
    tokio::fs::create_dir_all(&source_dir)
        .await
        .with_path(&source_dir)?;
    let path = source_dir.join(file_id);
    recover_staged_file(&path, file.accepted_bytes)?;
    if let Some(accepted) = file.accepted_chunks.get(&offset) {
        if accepted.length != bytes.len() || accepted.blake3 != actual {
            return Err(import_error(
                "upload_chunk_retry_mismatch",
                "an accepted upload chunk retry differs from the original",
            ));
        }
        let mut existing = vec![0; bytes.len()];
        let mut handle = File::open(&path).with_path(&path)?;
        handle.seek(SeekFrom::Start(offset)).with_path(&path)?;
        handle.read_exact(&mut existing).with_path(&path)?;
        if blake3::hash(&existing) != blake3::hash(bytes) {
            return Err(import_error(
                "upload_chunk_staging_corrupt",
                "accepted staged bytes no longer match their digest",
            ));
        }
        return Ok(file.clone());
    }
    if offset != file.accepted_bytes {
        return Err(import_error(
            "upload_chunk_not_sequential",
            "upload chunks must begin at the next accepted offset",
        ));
    }
    let new_length = offset
        .checked_add(bytes.len() as u64)
        .ok_or_else(|| import_error("upload_chunk_limit", "upload offset overflowed"))?;
    if new_length > file.byte_size {
        return Err(import_error(
            "upload_chunk_limit",
            "upload chunk exceeds the registered file size",
        ));
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true);
    let mut handle = options.open(&path).with_path(&path)?;
    let current_length = handle.metadata().with_path(&path)?.len();
    if current_length != offset {
        return Err(import_error(
            "upload_chunk_staging_corrupt",
            "staged file length does not equal its accepted offset",
        ));
    }
    handle.seek(SeekFrom::Start(offset)).with_path(&path)?;
    handle.write_all(bytes).with_path(&path)?;
    handle.sync_data().with_path(&path)?;
    file.accepted_chunks.insert(
        offset,
        AcceptedChunk {
            length: bytes.len(),
            blake3: actual,
        },
    );
    file.accepted_bytes = new_length;
    if new_length == file.byte_size {
        let full_digest = hash_file(&path)?;
        if full_digest != file.blake3 {
            handle.set_len(offset).with_path(&path)?;
            handle.sync_data().with_path(&path)?;
            return Err(import_error(
                "source_file_digest_mismatch",
                "completed source file does not match its registered digest",
            ));
        }
        file.complete = true;
    }
    let result = file.clone();
    save_source_index(job_dir, index).await?;
    Ok(result)
}

fn recover_staged_file(path: &Path, durable_offset: u64) -> StorageResult<()> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    let file = options.open(path).with_path(path)?;
    let length = file.metadata().with_path(path)?.len();
    if length < durable_offset {
        return Err(import_error(
            "upload_chunk_staging_corrupt",
            "staged file is shorter than its durable accepted offset",
        ));
    }
    if length > durable_offset {
        file.set_len(durable_offset).with_path(path)?;
        file.sync_data().with_path(path)?;
    }
    Ok(())
}
