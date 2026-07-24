use labello_domain::{KeybindingSet, UserId};

use crate::{
    DatasetRepository, StorageResult,
    fstoml::{read_toml, write_toml_atomic},
    paths,
};

impl DatasetRepository {
    pub async fn load_keybindings(&self, user_id: &UserId) -> StorageResult<KeybindingSet> {
        let path = self.keybindings_path(user_id);
        if tokio::fs::try_exists(&path)
            .await
            .map_err(|source| crate::StorageError::Io {
                path: path.clone(),
                source,
            })?
        {
            let mut keybindings: KeybindingSet = read_toml(&path).await?;
            labello_domain::validate_schema_version(keybindings.schema_version)?;
            keybindings.normalize();
            keybindings.validate()?;
            Ok(keybindings)
        } else {
            Ok(KeybindingSet::defaults_for(user_id.clone()))
        }
    }

    pub async fn save_keybindings(&self, keybindings: &KeybindingSet) -> StorageResult<()> {
        let mut keybindings = keybindings.clone();
        labello_domain::validate_schema_version(keybindings.schema_version)?;
        keybindings.validate()?;
        keybindings.normalize();
        keybindings.validate()?;
        write_toml_atomic(&self.keybindings_path(&keybindings.user_id), &keybindings).await
    }

    fn keybindings_path(&self, user_id: &UserId) -> std::path::PathBuf {
        self.root()
            .join(paths::USERS_DIR)
            .join(user_id.as_str())
            .join(paths::KEYBINDINGS_FILE)
    }
}

#[cfg(test)]
mod tests {
    use labello_domain::{KeyChord, UserAction};

    use super::*;

    #[tokio::test]
    async fn load_normalizes_legacy_actions_and_save_round_trips() {
        let temp = tempfile::tempdir().unwrap();
        let repo = DatasetRepository::new(temp.path());
        let user_id = UserId::from("user_1");
        tokio::fs::create_dir_all(temp.path().join(paths::USERS_DIR).join(user_id.as_str()))
            .await
            .unwrap();
        let mut legacy = KeybindingSet::defaults_for(user_id.clone());
        legacy.bindings.clear();
        legacy
            .bindings
            .insert(UserAction::NextImage, KeyChord::new("X"));
        legacy
            .bindings
            .insert(UserAction::PreviousImage, KeyChord::new("ArrowLeft"));
        write_toml_atomic(&repo.keybindings_path(&user_id), &legacy)
            .await
            .unwrap();

        let loaded = repo.load_keybindings(&user_id).await.unwrap();
        assert_eq!(loaded.bindings[&UserAction::NextImage].key, "X");
        assert_ne!(
            loaded.bindings[&UserAction::NextImage],
            loaded.bindings[&UserAction::SkipAssignment]
        );
        assert_eq!(loaded.bindings[&UserAction::PreviousImage].key, "ArrowLeft");
        assert_eq!(loaded.bindings.len(), UserAction::ACTIVE.len());

        repo.save_keybindings(&loaded).await.unwrap();
        assert_eq!(repo.load_keybindings(&user_id).await.unwrap(), loaded);
    }
}
