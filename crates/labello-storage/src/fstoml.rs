use std::path::Path;

use serde::{Serialize, de::DeserializeOwned};

use crate::{
    error::{PathIo, PathTomlDecode, PathTomlEncode, StorageError, StorageResult},
    fsjson::write_bytes_atomic,
};

pub async fn read_toml<T: DeserializeOwned>(path: &Path) -> StorageResult<T> {
    if !tokio::fs::try_exists(path).await.with_path(path)? {
        return Err(StorageError::NotFound(path.to_path_buf()));
    }
    let text = tokio::fs::read_to_string(path).await.with_path(path)?;
    toml::from_str(&text).with_toml_decode_path(path)
}

pub async fn read_current_toml<T>(path: &Path) -> StorageResult<T>
where
    T: DeserializeOwned + labello_domain::VersionedArtifact,
{
    let mut value: T = read_toml(path).await?;
    labello_domain::validate_supported_schema_version(value.schema_version())?;
    value.set_schema_version(labello_domain::SCHEMA_VERSION);
    value.finish_upcast();
    Ok(value)
}

pub async fn write_toml_atomic<T: Serialize>(path: &Path, value: &T) -> StorageResult<()> {
    let mut text = toml::to_string_pretty(value).with_toml_encode_path(path)?;
    if !text.ends_with('\n') {
        text.push('\n');
    }
    write_bytes_atomic(path, text.as_bytes()).await
}
