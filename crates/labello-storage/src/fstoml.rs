use std::path::Path;

use serde::{Serialize, de::DeserializeOwned};
use tokio::io::AsyncWriteExt;

use crate::error::{PathIo, PathTomlDecode, PathTomlEncode, StorageError, StorageResult};

pub async fn read_toml<T: DeserializeOwned>(path: &Path) -> StorageResult<T> {
    if !tokio::fs::try_exists(path).await.with_path(path)? {
        return Err(StorageError::NotFound(path.to_path_buf()));
    }
    let text = tokio::fs::read_to_string(path).await.with_path(path)?;
    toml::from_str(&text).with_toml_decode_path(path)
}

pub async fn write_toml_atomic<T: Serialize>(path: &Path, value: &T) -> StorageResult<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.with_path(parent)?;
    }
    let tmp_path = path.with_extension(format!("tmp-{}", uuid::Uuid::now_v7().simple()));
    let text = toml::to_string_pretty(value).with_toml_encode_path(path)?;
    let mut file = tokio::fs::File::create(&tmp_path)
        .await
        .with_path(&tmp_path)?;
    file.write_all(text.as_bytes()).await.with_path(&tmp_path)?;
    if !text.ends_with('\n') {
        file.write_all(b"\n").await.with_path(&tmp_path)?;
    }
    file.sync_all().await.with_path(&tmp_path)?;
    drop(file);
    tokio::fs::rename(&tmp_path, path).await.with_path(path)?;
    Ok(())
}
