use super::*;

impl DatasetRepository {
    pub(crate) async fn ensure_artifact_migration(&self) -> StorageResult<()> {
        if self.migration_complete.load(Ordering::Acquire) {
            return Ok(());
        }
        let _guard = self.migration_lock.lock().await;
        if self.migration_complete.load(Ordering::Acquire) {
            return Ok(());
        }
        let journal_path = self.artifact_migration_journal_path();
        let mut journal = if tokio::fs::try_exists(&journal_path)
            .await
            .with_path(&journal_path)?
        {
            let journal: ArtifactMigrationJournal = read_json(&journal_path).await?;
            validate_artifact_migration_journal(&journal)?;
            if journal.phase == ArtifactMigrationPhase::Completed {
                self.migration_complete.store(true, Ordering::Release);
                return Ok(());
            }
            journal
        } else {
            if !tokio::fs::try_exists(self.dataset_path())
                .await
                .with_path(self.dataset_path())?
            {
                return Ok(());
            }
            let config: DatasetConfig = read_toml(&self.dataset_path()).await?;
            labello_domain::validate_supported_schema_version(config.schema_version)?;
            if config.schema_version == SCHEMA_VERSION && !self.has_legacy_artifacts().await? {
                self.migration_complete.store(true, Ordering::Release);
                return Ok(());
            }
            self.prepare_artifact_migration(config).await?
        };

        if journal.phase < ArtifactMigrationPhase::DatasetConfigPublished {
            self.publish_migration_kind(&journal, ArtifactMigrationKind::DatasetConfig)
                .await?;
            self.record_migration_phase(
                &mut journal,
                ArtifactMigrationPhase::DatasetConfigPublished,
            )
            .await?;
        }
        if journal.phase < ArtifactMigrationPhase::ImagesIndexPublished {
            self.publish_migration_kind(&journal, ArtifactMigrationKind::ImagesIndex)
                .await?;
            self.record_migration_phase(&mut journal, ArtifactMigrationPhase::ImagesIndexPublished)
                .await?;
        }
        if journal.phase < ArtifactMigrationPhase::SchemaPublished {
            self.publish_migration_kind(&journal, ArtifactMigrationKind::Schema)
                .await?;
            self.record_migration_phase(&mut journal, ArtifactMigrationPhase::SchemaPublished)
                .await?;
        }
        if journal.phase < ArtifactMigrationPhase::KeybindingsPublished {
            self.publish_migration_kind(&journal, ArtifactMigrationKind::Keybindings)
                .await?;
            self.record_migration_phase(&mut journal, ArtifactMigrationPhase::KeybindingsPublished)
                .await?;
        }
        if journal.phase < ArtifactMigrationPhase::StatesRebuilt {
            self.rebuild_migrated_state_caches(&journal).await?;
            self.record_migration_phase(&mut journal, ArtifactMigrationPhase::StatesRebuilt)
                .await?;
        }
        if journal.phase < ArtifactMigrationPhase::Completed {
            self.record_migration_phase(&mut journal, ArtifactMigrationPhase::Completed)
                .await?;
        }
        self.stats_cache.invalidate();
        self.assignment_availability_cache.invalidate();
        self.migration_complete.store(true, Ordering::Release);
        Ok(())
    }

    async fn prepare_artifact_migration(
        &self,
        mut config: DatasetConfig,
    ) -> StorageResult<ArtifactMigrationJournal> {
        labello_domain::validate_supported_schema_version(config.schema_version)?;
        let timestamp = now();
        config.schema_version = SCHEMA_VERSION;
        config.updated_at = timestamp;
        if !config.migration_history.iter().any(|record| {
            record.from_version == LEGACY_SCHEMA_VERSION && record.to_version == SCHEMA_VERSION
        }) {
            config.migration_history.push(MigrationRecord {
                from_version: LEGACY_SCHEMA_VERSION,
                to_version: SCHEMA_VERSION,
                name: "schema-v2-to-v3-artifacts".to_string(),
                applied_at: timestamp,
            });
        }

        let mut index = if tokio::fs::try_exists(self.images_index_path())
            .await
            .with_path(self.images_index_path())?
        {
            let mut index: ImagesIndex = read_json(&self.images_index_path()).await?;
            labello_domain::validate_supported_schema_version(index.schema_version)?;
            index.schema_version = SCHEMA_VERSION;
            index
        } else {
            ImagesIndex::default()
        };
        index.image_count = index.images_by_hash.len();

        let generation = self.next_artifact_migration_generation().await?;
        let generation_dir = self.artifact_migration_generation_dir(generation);
        create_dir_all_synced(&generation_dir).await?;
        let mut files = Vec::new();

        let staged_config = generation_dir.join(paths::DATASET_FILE);
        write_toml_atomic(&staged_config, &config).await?;
        push_migration_file(
            &mut files,
            ArtifactMigrationKind::DatasetConfig,
            paths::DATASET_FILE,
            &staged_config,
        )
        .await?;

        let staged_index = generation_dir.join(paths::IMAGES_INDEX_FILE);
        write_json_atomic(&staged_index, &index).await?;
        push_migration_file(
            &mut files,
            ArtifactMigrationKind::ImagesIndex,
            paths::IMAGES_INDEX_FILE,
            &staged_index,
        )
        .await?;

        let staged_schema = generation_dir.join(paths::SCHEMA_FILE);
        write_json_atomic(&staged_schema, &labello_schema_bundle()).await?;
        push_migration_file(
            &mut files,
            ArtifactMigrationKind::Schema,
            paths::SCHEMA_FILE,
            &staged_schema,
        )
        .await?;

        for (relative_path, keybindings) in self.legacy_keybindings().await? {
            let staged = generation_dir.join(&relative_path);
            write_toml_atomic(&staged, &keybindings).await?;
            push_migration_file(
                &mut files,
                ArtifactMigrationKind::Keybindings,
                &relative_path,
                &staged,
            )
            .await?;
        }
        files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

        let mut journal = ArtifactMigrationJournal {
            journal_version: ARTIFACT_MIGRATION_JOURNAL_VERSION,
            from_version: LEGACY_SCHEMA_VERSION,
            to_version: SCHEMA_VERSION,
            generation,
            phase: ArtifactMigrationPhase::GenerationPrepared,
            files,
            phase_history: Vec::new(),
            started_at: timestamp,
            updated_at: timestamp,
        };
        journal.record_phase(ArtifactMigrationPhase::GenerationPrepared, timestamp);
        write_json_atomic(&self.artifact_migration_journal_path(), &journal).await?;
        self.maybe_fail_artifact_migration(ArtifactMigrationPhase::GenerationPrepared)?;
        Ok(journal)
    }

    async fn legacy_keybindings(&self) -> StorageResult<Vec<(String, KeybindingSet)>> {
        let users = self.root.join(paths::USERS_DIR);
        if !tokio::fs::try_exists(&users).await.with_path(&users)? {
            return Ok(Vec::new());
        }
        let mut reader = tokio::fs::read_dir(&users).await.with_path(&users)?;
        let mut paths_to_read = Vec::new();
        while let Some(entry) = reader.next_entry().await.with_path(&users)? {
            if !entry.file_type().await.with_path(entry.path())?.is_dir() {
                continue;
            }
            let path = entry.path().join(paths::KEYBINDINGS_FILE);
            if tokio::fs::try_exists(&path).await.with_path(&path)? {
                paths_to_read.push(path);
            }
        }
        paths_to_read.sort();
        let mut keybindings = Vec::with_capacity(paths_to_read.len());
        for path in paths_to_read {
            let mut value: KeybindingSet = read_toml(&path).await?;
            labello_domain::validate_supported_schema_version(value.schema_version)?;
            let directory_name = path
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                .ok_or_else(|| StorageError::OutsideDatasetRoot(path.clone()))?;
            let directory_user_id = UserId::from(directory_name);
            directory_user_id
                .validate_path_segment()
                .map_err(|_| StorageError::OutsideDatasetRoot(path.clone()))?;
            if value.user_id != directory_user_id {
                return Err(StorageError::OutsideDatasetRoot(path));
            }
            value.schema_version = SCHEMA_VERSION;
            value.normalize();
            value.validate()?;
            let relative = path
                .strip_prefix(&*self.root)
                .map_err(|_| StorageError::OutsideDatasetRoot(path.clone()))?
                .to_str()
                .ok_or_else(|| StorageError::OutsideDatasetRoot(path.clone()))?
                .replace(std::path::MAIN_SEPARATOR, "/");
            keybindings.push((relative, value));
        }
        Ok(keybindings)
    }

    async fn has_legacy_artifacts(&self) -> StorageResult<bool> {
        if tokio::fs::try_exists(self.images_index_path())
            .await
            .with_path(self.images_index_path())?
        {
            let version = read_schema_version(&self.images_index_path()).await?;
            labello_domain::validate_supported_schema_version(version)?;
            if version == LEGACY_SCHEMA_VERSION {
                return Ok(true);
            }
        }
        if tokio::fs::try_exists(self.schema_path())
            .await
            .with_path(self.schema_path())?
        {
            let version = read_schema_version(&self.schema_path()).await?;
            labello_domain::validate_supported_schema_version(version)?;
            if version == LEGACY_SCHEMA_VERSION {
                return Ok(true);
            }
        }

        let users = self.root.join(paths::USERS_DIR);
        if !tokio::fs::try_exists(&users).await.with_path(&users)? {
            return Ok(false);
        }
        let mut reader = tokio::fs::read_dir(&users).await.with_path(&users)?;
        while let Some(entry) = reader.next_entry().await.with_path(&users)? {
            if !entry.file_type().await.with_path(entry.path())?.is_dir() {
                continue;
            }
            let keybindings = entry.path().join(paths::KEYBINDINGS_FILE);
            if tokio::fs::try_exists(&keybindings)
                .await
                .with_path(&keybindings)?
            {
                let value: KeybindingSet = read_toml(&keybindings).await?;
                labello_domain::validate_supported_schema_version(value.schema_version)?;
                if value.schema_version == LEGACY_SCHEMA_VERSION {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    async fn next_artifact_migration_generation(&self) -> StorageResult<u64> {
        let mut generation = now().timestamp_micros().unsigned_abs();
        while tokio::fs::try_exists(self.artifact_migration_generation_dir(generation))
            .await
            .with_path(self.artifact_migration_generation_dir(generation))?
        {
            generation += 1;
        }
        Ok(generation)
    }

    pub(super) fn artifact_migration_generation_dir(&self, generation: u64) -> PathBuf {
        self.root
            .join(paths::ARTIFACT_MIGRATION_DIR)
            .join(paths::ARTIFACT_MIGRATION_GENERATIONS_DIR)
            .join(format!("{generation:020}"))
    }

    async fn publish_migration_kind(
        &self,
        journal: &ArtifactMigrationJournal,
        kind: ArtifactMigrationKind,
    ) -> StorageResult<()> {
        let files = journal
            .files
            .iter()
            .filter(|file| file.kind == kind)
            .cloned()
            .collect::<Vec<_>>();
        for file in files {
            validate_migration_relative_path(&file.relative_path)?;
            let staged = self
                .artifact_migration_generation_dir(journal.generation)
                .join(&file.relative_path);
            let bytes = tokio::fs::read(&staged).await.with_path(&staged)?;
            if blake3::hash(&bytes).to_hex().as_str() != file.blake3 {
                return Err(StorageError::BackgroundTask(format!(
                    "artifact migration generation {} failed integrity verification",
                    journal.generation
                )));
            }
            write_bytes_atomic(&self.root.join(&file.relative_path), &bytes).await?;
        }
        Ok(())
    }

    async fn record_migration_phase(
        &self,
        journal: &mut ArtifactMigrationJournal,
        phase: ArtifactMigrationPhase,
    ) -> StorageResult<()> {
        journal.record_phase(phase, now());
        write_json_atomic(&self.artifact_migration_journal_path(), journal).await?;
        self.maybe_fail_artifact_migration(phase)
    }

    async fn rebuild_migrated_state_caches(
        &self,
        journal: &ArtifactMigrationJournal,
    ) -> StorageResult<()> {
        let index_file = journal
            .files
            .iter()
            .find(|file| file.kind == ArtifactMigrationKind::ImagesIndex)
            .ok_or_else(|| {
                StorageError::BackgroundTask(
                    "artifact migration generation has no image index".to_string(),
                )
            })?;
        let staged_index = self
            .artifact_migration_generation_dir(journal.generation)
            .join(&index_file.relative_path);
        let index: ImagesIndex = read_current_json(&staged_index).await?;
        let mut image_ids = index
            .images_by_hash
            .values()
            .map(|record| record.image_id.clone())
            .collect::<BTreeSet<_>>();

        let annotations = self.root.join(paths::ANNOTATIONS_DIR);
        if tokio::fs::try_exists(&annotations)
            .await
            .with_path(&annotations)?
        {
            let mut reader = tokio::fs::read_dir(&annotations)
                .await
                .with_path(&annotations)?;
            while let Some(entry) = reader.next_entry().await.with_path(&annotations)? {
                if entry.file_type().await.with_path(entry.path())?.is_dir()
                    && let Some(name) = entry.file_name().to_str()
                    && !name.starts_with('.')
                {
                    let image_id = ImageId::from(name);
                    image_id
                        .validate_path_segment()
                        .map_err(|_| StorageError::OutsideDatasetRoot(entry.path()))?;
                    image_ids.insert(image_id);
                }
            }
        }

        for image_id in image_ids {
            let events = self.load_events(&image_id).await?;
            let state = rebuild_state(image_id.clone(), &events)?;
            write_json_atomic(&self.state_path(&image_id), &state).await?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn fail_artifact_migration_after(&self, phase: ArtifactMigrationPhase) {
        *self.migration_failure.lock() = Some(phase);
    }

    #[cfg(test)]
    fn maybe_fail_artifact_migration(&self, phase: ArtifactMigrationPhase) -> StorageResult<()> {
        let mut failure = self.migration_failure.lock();
        if *failure == Some(phase) {
            *failure = None;
            Err(StorageError::BackgroundTask(format!(
                "injected artifact migration crash after {phase:?}"
            )))
        } else {
            Ok(())
        }
    }

    #[cfg(not(test))]
    fn maybe_fail_artifact_migration(&self, _phase: ArtifactMigrationPhase) -> StorageResult<()> {
        Ok(())
    }
}

async fn push_migration_file(
    files: &mut Vec<ArtifactMigrationFile>,
    kind: ArtifactMigrationKind,
    relative_path: &str,
    staged_path: &Path,
) -> StorageResult<()> {
    validate_migration_relative_path(relative_path)?;
    let bytes = tokio::fs::read(staged_path).await.with_path(staged_path)?;
    files.push(ArtifactMigrationFile {
        kind,
        relative_path: relative_path.to_string(),
        blake3: blake3::hash(&bytes).to_hex().to_string(),
    });
    Ok(())
}

fn validate_artifact_migration_journal(journal: &ArtifactMigrationJournal) -> StorageResult<()> {
    if journal.journal_version != ARTIFACT_MIGRATION_JOURNAL_VERSION
        || journal.from_version != LEGACY_SCHEMA_VERSION
        || journal.to_version != SCHEMA_VERSION
    {
        return Err(StorageError::BackgroundTask(
            "unsupported artifact migration journal".to_string(),
        ));
    }
    let expected_phases = [
        ArtifactMigrationPhase::GenerationPrepared,
        ArtifactMigrationPhase::DatasetConfigPublished,
        ArtifactMigrationPhase::ImagesIndexPublished,
        ArtifactMigrationPhase::SchemaPublished,
        ArtifactMigrationPhase::KeybindingsPublished,
        ArtifactMigrationPhase::StatesRebuilt,
        ArtifactMigrationPhase::Completed,
    ];
    let current_phase = expected_phases
        .iter()
        .position(|phase| *phase == journal.phase)
        .expect("all artifact migration phases are listed");
    if journal.generation == 0
        || journal
            .phase_history
            .iter()
            .map(|record| record.phase)
            .ne(expected_phases[..=current_phase].iter().copied())
    {
        return Err(StorageError::BackgroundTask(
            "invalid artifact migration phase history".to_string(),
        ));
    }

    let mut unique = BTreeSet::new();
    let mut core_counts = [0_u8; 3];
    for file in &journal.files {
        validate_migration_file(file)?;
        if !unique.insert(&file.relative_path) || blake3::Hash::from_hex(&file.blake3).is_err() {
            return Err(StorageError::BackgroundTask(
                "invalid artifact migration file manifest".to_string(),
            ));
        }
        match file.kind {
            ArtifactMigrationKind::DatasetConfig => core_counts[0] += 1,
            ArtifactMigrationKind::ImagesIndex => core_counts[1] += 1,
            ArtifactMigrationKind::Schema => core_counts[2] += 1,
            ArtifactMigrationKind::Keybindings => {}
        }
    }
    if core_counts != [1, 1, 1] {
        return Err(StorageError::BackgroundTask(
            "artifact migration journal is missing a core artifact".to_string(),
        ));
    }
    Ok(())
}

fn validate_migration_file(file: &ArtifactMigrationFile) -> StorageResult<()> {
    validate_migration_relative_path(&file.relative_path)?;
    let valid = match file.kind {
        ArtifactMigrationKind::DatasetConfig => file.relative_path == paths::DATASET_FILE,
        ArtifactMigrationKind::ImagesIndex => file.relative_path == paths::IMAGES_INDEX_FILE,
        ArtifactMigrationKind::Schema => file.relative_path == paths::SCHEMA_FILE,
        ArtifactMigrationKind::Keybindings => {
            let components = Path::new(&file.relative_path)
                .components()
                .collect::<Vec<_>>();
            components.len() == 3
                && components[0].as_os_str() == paths::USERS_DIR
                && components[2].as_os_str() == paths::KEYBINDINGS_FILE
                && components[1]
                    .as_os_str()
                    .to_str()
                    .is_some_and(|value| UserId::from(value).validate_path_segment().is_ok())
        }
    };
    if valid {
        Ok(())
    } else {
        Err(StorageError::OutsideDatasetRoot(PathBuf::from(
            &file.relative_path,
        )))
    }
}

fn validate_migration_relative_path(relative_path: &str) -> StorageResult<()> {
    let path = Path::new(relative_path);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        Err(StorageError::OutsideDatasetRoot(path.to_path_buf()))
    } else {
        Ok(())
    }
}
