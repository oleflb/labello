use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use labello_domain::{ImportId, UserId};
use serde::{Serialize, de::DeserializeOwned};
use tokio::io::AsyncWriteExt;

use crate::{
    error::{PathIo, PathJson, StorageError, StorageResult},
    fsjson::read_json,
};

const API_STATE_DIR: &str = "api";
const API_JOBS_DIR: &str = "jobs";
const API_REQUESTS_DIR: &str = "requests";

/// Owns the durable filesystem mechanics for API-side import control records.
///
/// The API retains the record schemas and idempotency semantics. This store
/// only maps validated identities to private files and performs durable JSON
/// reads, listings, and atomic writes.
#[derive(Clone, Debug)]
pub struct ImportControlStore {
    imports_root: Arc<PathBuf>,
}

impl ImportControlStore {
    pub(super) fn new(imports_root: impl Into<PathBuf>) -> Self {
        Self {
            imports_root: Arc::new(imports_root.into()),
        }
    }

    pub async fn save_job<T: Serialize>(
        &self,
        import_id: &ImportId,
        value: &T,
    ) -> StorageResult<()> {
        validate_import_id(import_id)?;
        self.write_private_json(&self.job_path(import_id), value)
            .await
    }

    pub async fn load_job<T: DeserializeOwned>(&self, import_id: &ImportId) -> StorageResult<T> {
        validate_import_id(import_id)?;
        read_json(&self.job_path(import_id)).await
    }

    pub async fn list_jobs<T: DeserializeOwned>(&self) -> StorageResult<Vec<T>> {
        let directory = self.api_root().join(API_JOBS_DIR);
        if !tokio::fs::try_exists(&directory)
            .await
            .with_path(&directory)?
        {
            return Ok(Vec::new());
        }
        let mut entries = tokio::fs::read_dir(&directory)
            .await
            .with_path(&directory)?;
        let mut values = Vec::new();
        while let Some(entry) = entries.next_entry().await.with_path(&directory)? {
            if entry.file_type().await.with_path(entry.path())?.is_file() {
                values.push(read_json(&entry.path()).await?);
            }
        }
        Ok(values)
    }

    pub async fn load_request<T: DeserializeOwned>(
        &self,
        owner: &UserId,
        key: &str,
    ) -> StorageResult<Option<T>> {
        let path = self.request_path(owner, key);
        if !tokio::fs::try_exists(&path).await.with_path(&path)? {
            return Ok(None);
        }
        read_json(&path).await.map(Some)
    }

    pub async fn save_request<T: Serialize>(
        &self,
        owner: &UserId,
        key: &str,
        value: &T,
    ) -> StorageResult<()> {
        self.write_private_json(&self.request_path(owner, key), value)
            .await
    }

    fn api_root(&self) -> PathBuf {
        self.imports_root.join(API_STATE_DIR)
    }

    fn job_path(&self, import_id: &ImportId) -> PathBuf {
        self.api_root()
            .join(API_JOBS_DIR)
            .join(format!("{}.json", import_id.as_str()))
    }

    fn request_path(&self, owner: &UserId, key: &str) -> PathBuf {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"labello:import-api-idempotency:v1\0");
        hasher.update(owner.as_str().as_bytes());
        hasher.update(b"\0");
        hasher.update(key.as_bytes());
        self.api_root()
            .join(API_REQUESTS_DIR)
            .join(format!("{}.json", hasher.finalize().to_hex()))
    }

    async fn write_private_json<T: Serialize>(&self, path: &Path, value: &T) -> StorageResult<()> {
        let parent = path.parent().ok_or_else(|| StorageError::Io {
            path: path.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "import control path has no parent",
            ),
        })?;
        let api_directory = parent.parent().unwrap_or(parent);
        if let Some(imports_directory) = api_directory.parent() {
            tokio::fs::create_dir_all(imports_directory)
                .await
                .with_path(imports_directory)?;
        }
        create_private_directory(api_directory).await?;
        create_private_directory(parent).await?;

        let temporary = path.with_extension(format!("tmp-{}", uuid::Uuid::now_v7().simple()));
        let mut bytes = serde_json::to_vec_pretty(value).with_json_path(path)?;
        bytes.push(b'\n');
        let mut options = tokio::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).await.with_path(&temporary)?;
        file.write_all(&bytes).await.with_path(&temporary)?;
        file.sync_all().await.with_path(&temporary)?;
        drop(file);
        tokio::fs::rename(&temporary, path).await.with_path(path)?;
        tokio::fs::File::open(parent)
            .await
            .with_path(parent)?
            .sync_all()
            .await
            .with_path(parent)?;
        Ok(())
    }
}

async fn create_private_directory(path: &Path) -> StorageResult<()> {
    let mut builder = tokio::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        builder.mode(0o700);
    }
    match builder.create(path).await {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
                    .await
                    .with_path(path)?;
            }
            Ok(())
        }
        Err(source) => Err(StorageError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn validate_import_id(import_id: &ImportId) -> StorageResult<()> {
    import_id
        .validate_path_segment()
        .map_err(|_| StorageError::Import {
            code: "import_id_invalid".to_string(),
            message: "import ID is invalid".to_string(),
        })
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn control_records_and_directories_are_private() {
        let temp = tempfile::tempdir().unwrap();
        let imports_root = temp.path().join(".labello-server/imports");
        let store = ImportControlStore::new(&imports_root);
        let import_id = ImportId::from("imp_test");

        store
            .save_job(&import_id, &json!({"private": true}))
            .await
            .unwrap();

        let path = imports_root.join("api/jobs/imp_test.json");
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(path.parent().unwrap().parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
