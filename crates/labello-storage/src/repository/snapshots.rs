use super::*;

impl DatasetRepository {
    pub async fn create_snapshot(&self) -> StorageResult<DatasetSnapshot> {
        let config = self.load_dataset_config().await?;
        let index = self.load_images_index().await?;
        let snapshot_id = format!(
            "{}-{}",
            now().format("%Y%m%dT%H%M%S%.3fZ"),
            uuid::Uuid::new_v4().simple()
        );
        let snapshots_dir = self.snapshots_dir();
        let temporary = snapshots_dir.join(format!(".{snapshot_id}.tmp"));
        let destination = snapshots_dir.join(&snapshot_id);
        tokio::fs::create_dir_all(&temporary)
            .await
            .with_path(&temporary)?;

        let result = async {
            let mut files = Vec::new();
            self.snapshot_copy_file(paths::DATASET_FILE, &temporary, &mut files)
                .await?;
            self.snapshot_copy_file(paths::IMAGES_INDEX_FILE, &temporary, &mut files)
                .await?;
            if tokio::fs::try_exists(self.schema_path())
                .await
                .with_path(self.schema_path())?
            {
                self.snapshot_copy_file(paths::SCHEMA_FILE, &temporary, &mut files)
                    .await?;
            }
            let imports = self
                .snapshot_import_records(&config.dataset_id, &temporary, &mut files)
                .await?;
            for image in index.images_by_hash.values() {
                let events = self.load_events(&image.image_id).await?;
                let events_path = self.events_path(&image.image_id);
                let event_bytes = if tokio::fs::try_exists(&events_path)
                    .await
                    .with_path(&events_path)?
                {
                    tokio::fs::read(&events_path)
                        .await
                        .with_path(&events_path)?
                } else {
                    Vec::new()
                };
                let relative = format!(
                    "{}/{}/{}",
                    paths::ANNOTATIONS_DIR,
                    image.image_id,
                    paths::EVENTS_FILE
                );
                write_snapshot_bytes(&temporary, &relative, &event_bytes, &mut files).await?;

                let state = rebuild_state(image.image_id.clone(), &events)?;
                let state_bytes = serde_json::to_vec_pretty(&state)
                    .with_json_path(self.state_path(&image.image_id))?;
                let relative = format!(
                    "{}/{}/{}",
                    paths::ANNOTATIONS_DIR,
                    image.image_id,
                    paths::STATE_FILE
                );
                write_snapshot_bytes(&temporary, &relative, &state_bytes, &mut files).await?;
            }
            files.sort_by(|left, right| left.path.cmp(&right.path));
            let manifest = DatasetSnapshot {
                schema_version: SCHEMA_VERSION,
                snapshot_id: snapshot_id.clone(),
                dataset_id: config.dataset_id,
                created_at: now(),
                includes_image_bytes: false,
                total_bytes: files.iter().map(|file| file.byte_size).sum(),
                files,
                imports,
            };
            let manifest_bytes = serde_json::to_vec_pretty(&manifest)
                .with_json_path(temporary.join(paths::SNAPSHOT_MANIFEST_FILE))?;
            write_snapshot_bytes(
                &temporary,
                paths::SNAPSHOT_MANIFEST_FILE,
                &manifest_bytes,
                &mut Vec::new(),
            )
            .await?;
            tokio::fs::rename(&temporary, &destination)
                .await
                .with_path(&destination)?;
            Ok(manifest)
        }
        .await;
        if result.is_err()
            && let Err(error) = tokio::fs::remove_dir_all(&temporary).await
        {
            tracing::warn!(
                event = "snapshot.cleanup.failed",
                error_kind = %error.kind(),
                "could not remove incomplete snapshot"
            );
        }
        result
    }

    pub async fn list_snapshots(&self) -> StorageResult<Vec<DatasetSnapshot>> {
        let directory = self.snapshots_dir();
        if !tokio::fs::try_exists(&directory)
            .await
            .with_path(&directory)?
        {
            return Ok(Vec::new());
        }
        let mut entries = tokio::fs::read_dir(&directory)
            .await
            .with_path(&directory)?;
        let mut snapshots: Vec<DatasetSnapshot> = Vec::new();
        while let Some(entry) = entries.next_entry().await.with_path(&directory)? {
            if !entry.file_type().await.with_path(entry.path())?.is_dir()
                || entry.file_name().to_string_lossy().starts_with('.')
            {
                continue;
            }
            let manifest_path = entry.path().join(paths::SNAPSHOT_MANIFEST_FILE);
            if tokio::fs::try_exists(&manifest_path)
                .await
                .with_path(&manifest_path)?
            {
                snapshots.push(read_current_json(&manifest_path).await?);
            }
        }
        snapshots.sort_by_key(|snapshot| std::cmp::Reverse(snapshot.created_at));
        Ok(snapshots)
    }

    pub async fn snapshot_file(
        &self,
        snapshot_id: &str,
        relative_path: &str,
    ) -> StorageResult<Vec<u8>> {
        validate_snapshot_segment(snapshot_id)?;
        let relative = Path::new(relative_path);
        if relative.as_os_str().is_empty()
            || relative.is_absolute()
            || relative.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(StorageError::OutsideDatasetRoot(relative.to_path_buf()));
        }
        let snapshot = self.snapshots_dir().join(snapshot_id);
        let manifest: DatasetSnapshot =
            read_current_json(&snapshot.join(paths::SNAPSHOT_MANIFEST_FILE)).await?;
        if relative_path != paths::SNAPSHOT_MANIFEST_FILE
            && !manifest.files.iter().any(|file| file.path == relative_path)
        {
            return Err(StorageError::NotFound(snapshot.join(relative)));
        }
        let path = snapshot.join(relative);
        tokio::fs::read(&path).await.with_path(path)
    }

    async fn snapshot_copy_file(
        &self,
        relative_path: &str,
        destination: &Path,
        files: &mut Vec<SnapshotFileEntry>,
    ) -> StorageResult<()> {
        let source = self.root.join(relative_path);
        let bytes = tokio::fs::read(&source).await.with_path(&source)?;
        write_snapshot_bytes(destination, relative_path, &bytes, files).await
    }

    async fn snapshot_import_records(
        &self,
        dataset_id: &labello_domain::DatasetId,
        destination: &Path,
        files: &mut Vec<SnapshotFileEntry>,
    ) -> StorageResult<Vec<SnapshotImportEntry>> {
        let records = self.import_records(dataset_id).await?;
        let mut imports = Vec::with_capacity(records.len());
        for (manifest, manifest_path, source_objects_path) in records {
            let relative_root = format!("{}/{}", paths::IMPORTS_DIR, manifest.import_id);
            let manifest_relative = format!("{relative_root}/{}", paths::IMPORT_MANIFEST_FILE);
            let source_objects_relative =
                format!("{relative_root}/{}", paths::IMPORT_SOURCE_OBJECTS_FILE);
            let manifest_bytes = tokio::fs::read(&manifest_path)
                .await
                .with_path(&manifest_path)?;
            write_snapshot_bytes(destination, &manifest_relative, &manifest_bytes, files).await?;
            let source_objects_bytes = tokio::fs::read(&source_objects_path)
                .await
                .with_path(&source_objects_path)?;
            write_snapshot_bytes(
                destination,
                &source_objects_relative,
                &source_objects_bytes,
                files,
            )
            .await?;
            imports.push(SnapshotImportEntry {
                import_id: manifest.import_id,
                manifest_path: manifest_relative,
                source_objects_path: source_objects_relative,
            });
        }
        Ok(imports)
    }

    pub async fn load_import_manifests(&self) -> StorageResult<Vec<ImportManifest>> {
        let dataset_id = self.load_dataset_config().await?.dataset_id;
        Ok(self
            .import_records(&dataset_id)
            .await?
            .into_iter()
            .map(|(manifest, _, _)| manifest)
            .collect())
    }

    async fn import_records(
        &self,
        dataset_id: &labello_domain::DatasetId,
    ) -> StorageResult<Vec<(ImportManifest, PathBuf, PathBuf)>> {
        let directory = self.imports_dir();
        if !tokio::fs::try_exists(&directory)
            .await
            .with_path(&directory)?
        {
            return Ok(Vec::new());
        }
        let mut reader = tokio::fs::read_dir(&directory)
            .await
            .with_path(&directory)?;
        let mut import_directories = Vec::new();
        while let Some(entry) = reader.next_entry().await.with_path(&directory)? {
            if entry.file_type().await.with_path(entry.path())?.is_dir()
                && !entry.file_name().to_string_lossy().starts_with('.')
            {
                import_directories.push(entry.path());
            }
        }
        import_directories.sort();

        let mut records = Vec::with_capacity(import_directories.len());
        for import_directory in import_directories {
            let directory_name = import_directory
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| StorageError::OutsideDatasetRoot(import_directory.clone()))?;
            let directory_import_id = ImportId::from(directory_name);
            directory_import_id
                .validate_path_segment()
                .map_err(|_| StorageError::OutsideDatasetRoot(import_directory.clone()))?;
            let manifest_path = import_directory.join(paths::IMPORT_MANIFEST_FILE);
            let source_objects_path = import_directory.join(paths::IMPORT_SOURCE_OBJECTS_FILE);
            let manifest: ImportManifest = read_json(&manifest_path).await?;
            labello_domain::validate_schema_version(manifest.schema_version)?;
            if manifest.import_id != directory_import_id || manifest.dataset_id != *dataset_id {
                return Err(StorageError::Domain(
                    labello_domain::DomainError::InvalidImport(
                        "import manifest identity does not match its repository path".to_string(),
                    ),
                ));
            }
            if !tokio::fs::try_exists(&source_objects_path)
                .await
                .with_path(&source_objects_path)?
            {
                return Err(StorageError::NotFound(source_objects_path));
            }
            records.push((manifest, manifest_path, source_objects_path));
        }
        Ok(records)
    }
}

async fn write_snapshot_bytes(
    root: &Path,
    relative_path: &str,
    bytes: &[u8],
    files: &mut Vec<SnapshotFileEntry>,
) -> StorageResult<()> {
    let path = root.join(relative_path);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.with_path(parent)?;
    }
    tokio::fs::write(&path, bytes).await.with_path(&path)?;
    files.push(SnapshotFileEntry {
        path: relative_path.to_string(),
        byte_size: bytes.len() as u64,
        blake3: blake3::hash(bytes).to_hex().to_string(),
    });
    Ok(())
}

fn validate_snapshot_segment(value: &str) -> StorageResult<()> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
    {
        Err(StorageError::OutsideDatasetRoot(PathBuf::from(value)))
    } else {
        Ok(())
    }
}
