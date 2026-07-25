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

pub async fn read_current_json<T: labello_domain::VersionedArtifact>(
    path: &Path,
) -> StorageResult<T> {
    if !tokio::fs::try_exists(path).await.with_path(path)? {
        return Err(StorageError::NotFound(path.to_path_buf()));
    }
    let bytes = tokio::fs::read(path).await.with_path(path)?;
    Ok(labello_domain::deserialize_current_artifact(&bytes)?)
}

pub async fn read_schema_version(path: &Path) -> StorageResult<u32> {
    if !tokio::fs::try_exists(path).await.with_path(path)? {
        return Err(StorageError::NotFound(path.to_path_buf()));
    }
    let bytes = tokio::fs::read(path).await.with_path(path)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).with_json_path(path)?;
    value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
        .ok_or_else(|| {
            labello_domain::DomainError::InvalidSchemaArtifact(
                "schemaVersion is missing or invalid".to_string(),
            )
            .into()
        })
}

pub async fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> StorageResult<()> {
    let mut bytes = serde_json::to_vec_pretty(value).with_json_path(path)?;
    bytes.push(b'\n');
    write_bytes_atomic(path, &bytes).await
}

pub async fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> StorageResult<()> {
    if let Some(parent) = path.parent() {
        create_dir_all_synced(parent).await?;
    }
    let tmp_path = path.with_extension(format!("tmp-{}", uuid::Uuid::now_v7().simple()));
    let mut file = tokio::fs::File::create(&tmp_path)
        .await
        .with_path(&tmp_path)?;
    file.write_all(bytes).await.with_path(&tmp_path)?;
    file.sync_all().await.with_path(&tmp_path)?;
    drop(file);
    tokio::fs::rename(&tmp_path, path).await.with_path(path)?;
    sync_parent(path).await?;
    Ok(())
}

pub async fn create_dir_all_synced(path: &Path) -> StorageResult<()> {
    if tokio::fs::try_exists(path).await.with_path(path)? {
        return Ok(());
    }
    let mut missing = Vec::new();
    let mut cursor = path;
    while !tokio::fs::try_exists(cursor).await.with_path(cursor)? {
        missing.push(cursor.to_path_buf());
        cursor = cursor.parent().ok_or_else(|| StorageError::Io {
            path: path.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "directory has no existing parent",
            ),
        })?;
    }
    tokio::fs::create_dir_all(path).await.with_path(path)?;
    for directory in missing.iter().rev() {
        sync_parent(directory).await?;
    }
    Ok(())
}

pub async fn sync_parent(path: &Path) -> StorageResult<()> {
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
