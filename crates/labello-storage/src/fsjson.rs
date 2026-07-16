use std::path::Path;

use serde::{Serialize, de::DeserializeOwned};
use tokio::io::AsyncWriteExt;

use crate::error::{PathIo, PathJson, StorageError, StorageResult};

pub async fn read_json<T: DeserializeOwned>(path: &Path) -> StorageResult<T> {
    if !tokio::fs::try_exists(path).await.with_path(path)? {
        return Err(StorageError::NotFound(path.to_path_buf()));
    }
    let bytes = tokio::fs::read(path).await.with_path(path)?;
    serde_json::from_slice(&bytes).with_json_path(path)
}

pub async fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> StorageResult<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.with_path(parent)?;
    }
    let tmp_path = path.with_extension(format!("tmp-{}", uuid::Uuid::now_v7().simple()));
    let bytes = serde_json::to_vec_pretty(value).with_json_path(path)?;
    let mut file = tokio::fs::File::create(&tmp_path)
        .await
        .with_path(&tmp_path)?;
    file.write_all(&bytes).await.with_path(&tmp_path)?;
    file.write_all(b"\n").await.with_path(&tmp_path)?;
    file.sync_all().await.with_path(&tmp_path)?;
    drop(file);
    tokio::fs::rename(&tmp_path, path).await.with_path(path)?;
    if let Some(parent) = path.parent() {
        tokio::fs::File::open(parent)
            .await
            .with_path(parent)?
            .sync_all()
            .await
            .with_path(parent)?;
    }
    Ok(())
}
