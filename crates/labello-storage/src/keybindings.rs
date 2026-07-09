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
            read_toml(&path).await
        } else {
            Ok(KeybindingSet::defaults_for(user_id.clone()))
        }
    }

    pub async fn save_keybindings(&self, keybindings: &KeybindingSet) -> StorageResult<()> {
        keybindings.validate_conflicts()?;
        write_toml_atomic(&self.keybindings_path(&keybindings.user_id), keybindings).await
    }

    fn keybindings_path(&self, user_id: &UserId) -> std::path::PathBuf {
        self.root()
            .join(paths::USERS_DIR)
            .join(user_id.as_str())
            .join(paths::KEYBINDINGS_FILE)
    }
}
