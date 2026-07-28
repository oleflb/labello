use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

use blake3::Hasher;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;
use walkdir::WalkDir;

use super::types::{
    AcceptedChunk, BrowserFileRegistration, ImportBrowseEntry, ImportBrowseEntryKind,
    ImportBrowseMode, ImportBrowsePage, ImportLimits, ImportProfile, RegisteredFile,
    ServerDirectorySelection,
};
use crate::{
    error::{PathIo, StorageError, StorageResult},
    fsjson::write_json_atomic,
};

pub(super) const SOURCE_INDEX_FILE: &str = "source-index.json";
pub(super) const SOURCE_DIR: &str = "source";
const BROWSE_PAGE_SIZE: usize = 200;
const BROWSE_SCAN_LIMIT: usize = 10_000;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SourceIndex {
    pub sealed: bool,
    pub source_fingerprint: Option<String>,
    #[serde(default)]
    pub parser_version: Option<String>,
    pub files: BTreeMap<String, RegisteredFile>,
}

impl SourceIndex {
    pub fn by_path(&self) -> BTreeMap<&str, &RegisteredFile> {
        self.files
            .values()
            .map(|file| (file.relative_path.as_str(), file))
            .collect()
    }
}

pub(super) struct SourceAccess<'a> {
    job_dir: &'a Path,
    index: &'a SourceIndex,
    by_path: BTreeMap<&'a str, &'a RegisteredFile>,
}

impl<'a> SourceAccess<'a> {
    pub fn new(job_dir: &'a Path, index: &'a SourceIndex) -> Self {
        Self {
            job_dir,
            index,
            by_path: index.by_path(),
        }
    }

    pub fn file(&self, relative_path: &str) -> StorageResult<&RegisteredFile> {
        self.by_path.get(relative_path).copied().ok_or_else(|| {
            import_error(
                "source_file_missing",
                "selected source file is not registered",
            )
        })
    }

    pub fn physical_path(&self, file: &RegisteredFile) -> PathBuf {
        self.job_dir.join(SOURCE_DIR).join(&file.file_id)
    }

    pub fn read_limited(&self, relative_path: &str, max_bytes: u64) -> StorageResult<Vec<u8>> {
        let file = self.file(relative_path)?;
        if !file.complete || file.accepted_bytes != file.byte_size {
            return Err(import_error(
                "source_file_incomplete",
                "selected source file is not completely staged",
            ));
        }
        if file.byte_size > max_bytes {
            return Err(import_error(
                "source_file_too_large",
                "source file exceeds the parser byte limit",
            ));
        }
        let path = self.physical_path(file);
        let mut bytes = Vec::new();
        File::open(&path)
            .with_path(&path)?
            .take(max_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .with_path(&path)?;
        if bytes.len() as u64 > max_bytes {
            return Err(import_error(
                "source_file_too_large",
                "source file exceeds the parser byte limit",
            ));
        }
        if bytes.len() as u64 != file.byte_size {
            return Err(import_error(
                "source_file_size_mismatch",
                "staged source file size does not match its registration",
            ));
        }
        Ok(bytes)
    }

    pub fn files_below<'b>(&'b self, prefix: &str) -> impl Iterator<Item = &'b RegisteredFile> {
        let prefix = prefix.trim_end_matches('/');
        self.index.files.values().filter(move |file| {
            file.relative_path == prefix
                || file
                    .relative_path
                    .strip_prefix(prefix)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
    }

    pub fn all_files(&self) -> impl Iterator<Item = &RegisteredFile> {
        self.index.files.values()
    }
}

include!("source/browser.rs");
include!("source/server.rs");
include!("source/sealing.rs");
include!("source/validation.rs");
