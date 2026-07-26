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

pub(super) async fn register_browser_files(
    job_dir: &Path,
    index: &mut SourceIndex,
    registrations: Vec<BrowserFileRegistration>,
    limits: &ImportLimits,
) -> StorageResult<Vec<RegisteredFile>> {
    if index.sealed {
        return Err(import_error(
            "source_sealed",
            "sealed source cannot be changed",
        ));
    }
    if index.files.len().saturating_add(registrations.len()) > limits.browser_source_files {
        return Err(import_error(
            "source_file_limit",
            "browser source file count exceeds the configured limit",
        ));
    }
    let mut path_keys = index
        .files
        .values()
        .map(|file| collision_key(&file.relative_path))
        .collect::<BTreeSet<_>>();
    let mut total = index.files.values().map(|file| file.byte_size).sum::<u64>();
    let mut registered = Vec::with_capacity(registrations.len());
    for registration in registrations {
        validate_source_path(&registration.relative_path, limits)?;
        validate_digest(&registration.blake3)?;
        if registration.byte_size > limits.single_source_file_bytes {
            return Err(import_error(
                "source_file_too_large",
                "registered source file exceeds the configured limit",
            ));
        }
        total = total.checked_add(registration.byte_size).ok_or_else(|| {
            import_error(
                "source_byte_limit",
                "registered source byte count overflowed",
            )
        })?;
        if total > limits.browser_source_bytes
            || total > limits.total_source_bytes
            || total > limits.staged_bytes
        {
            return Err(import_error(
                "source_byte_limit",
                "browser source bytes exceed the configured limit",
            ));
        }
        if !path_keys.insert(collision_key(&registration.relative_path)) {
            return Err(import_error(
                "source_path_collision",
                "source paths collide after case and Unicode normalization",
            ));
        }
        let file = RegisteredFile {
            file_id: format!("file_{}", uuid::Uuid::new_v4().simple()),
            relative_path: registration.relative_path,
            byte_size: registration.byte_size,
            blake3: registration.blake3.to_ascii_lowercase(),
            accepted_bytes: 0,
            complete: false,
            accepted_chunks: BTreeMap::new(),
        };
        index.files.insert(file.file_id.clone(), file.clone());
        registered.push(file);
    }
    save_source_index(job_dir, index).await?;
    Ok(registered)
}

pub(super) async fn upload_chunk(
    job_dir: &Path,
    index: &mut SourceIndex,
    file_id: &str,
    offset: u64,
    bytes: &[u8],
    digest: &str,
    limits: &ImportLimits,
) -> StorageResult<RegisteredFile> {
    if index.sealed {
        return Err(import_error(
            "source_sealed",
            "sealed source cannot be changed",
        ));
    }
    if bytes.is_empty() || bytes.len() > limits.upload_chunk_bytes {
        return Err(import_error(
            "upload_chunk_limit",
            "upload chunk length is outside the configured bounds",
        ));
    }
    validate_digest(digest)?;
    let actual = blake3::hash(bytes).to_hex().to_string();
    if actual != digest.to_ascii_lowercase() {
        return Err(import_error(
            "upload_chunk_digest_mismatch",
            "upload chunk digest does not match its bytes",
        ));
    }
    let file = index.files.get_mut(file_id).ok_or_else(|| {
        import_error(
            "source_file_missing",
            "registered source file does not exist",
        )
    })?;
    let source_dir = job_dir.join(SOURCE_DIR);
    tokio::fs::create_dir_all(&source_dir)
        .await
        .with_path(&source_dir)?;
    let path = source_dir.join(file_id);
    recover_staged_file(&path, file.accepted_bytes)?;
    if let Some(accepted) = file.accepted_chunks.get(&offset) {
        if accepted.length != bytes.len() || accepted.blake3 != actual {
            return Err(import_error(
                "upload_chunk_retry_mismatch",
                "an accepted upload chunk retry differs from the original",
            ));
        }
        let mut existing = vec![0; bytes.len()];
        let mut handle = File::open(&path).with_path(&path)?;
        handle.seek(SeekFrom::Start(offset)).with_path(&path)?;
        handle.read_exact(&mut existing).with_path(&path)?;
        if blake3::hash(&existing) != blake3::hash(bytes) {
            return Err(import_error(
                "upload_chunk_staging_corrupt",
                "accepted staged bytes no longer match their digest",
            ));
        }
        return Ok(file.clone());
    }
    if offset != file.accepted_bytes {
        return Err(import_error(
            "upload_chunk_not_sequential",
            "upload chunks must begin at the next accepted offset",
        ));
    }
    let new_length = offset
        .checked_add(bytes.len() as u64)
        .ok_or_else(|| import_error("upload_chunk_limit", "upload offset overflowed"))?;
    if new_length > file.byte_size {
        return Err(import_error(
            "upload_chunk_limit",
            "upload chunk exceeds the registered file size",
        ));
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true);
    let mut handle = options.open(&path).with_path(&path)?;
    let current_length = handle.metadata().with_path(&path)?.len();
    if current_length != offset {
        return Err(import_error(
            "upload_chunk_staging_corrupt",
            "staged file length does not equal its accepted offset",
        ));
    }
    handle.seek(SeekFrom::Start(offset)).with_path(&path)?;
    handle.write_all(bytes).with_path(&path)?;
    handle.sync_data().with_path(&path)?;
    file.accepted_chunks.insert(
        offset,
        AcceptedChunk {
            length: bytes.len(),
            blake3: actual,
        },
    );
    file.accepted_bytes = new_length;
    if new_length == file.byte_size {
        let full_digest = hash_file(&path)?;
        if full_digest != file.blake3 {
            handle.set_len(offset).with_path(&path)?;
            handle.sync_data().with_path(&path)?;
            return Err(import_error(
                "source_file_digest_mismatch",
                "completed source file does not match its registered digest",
            ));
        }
        file.complete = true;
    }
    let result = file.clone();
    save_source_index(job_dir, index).await?;
    Ok(result)
}

fn recover_staged_file(path: &Path, durable_offset: u64) -> StorageResult<()> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    let file = options.open(path).with_path(path)?;
    let length = file.metadata().with_path(path)?.len();
    if length < durable_offset {
        return Err(import_error(
            "upload_chunk_staging_corrupt",
            "staged file is shorter than its durable accepted offset",
        ));
    }
    if length > durable_offset {
        file.set_len(durable_offset).with_path(path)?;
        file.sync_data().with_path(path)?;
    }
    Ok(())
}

pub(super) fn copy_server_directory(
    job_dir: &Path,
    root: &Path,
    root_handle: &File,
    selection: &ServerDirectorySelection,
    limits: &ImportLimits,
) -> StorageResult<SourceIndex> {
    validate_optional_directory(&selection.relative_directory, limits)?;
    let root_selection =
        selection.relative_directory.is_empty() || selection.relative_directory == ".";
    let selected_relative = if root_selection {
        Path::new("")
    } else {
        Path::new(&selection.relative_directory)
    };
    let selected_handle = secure_open_directory(root_handle, selected_relative)?;
    let selected = pinned_handle_path(&selected_handle);
    let mut candidates = Vec::new();
    for entry in WalkDir::new(&selected).follow_links(false) {
        let entry = entry.map_err(|error| {
            import_error(
                "server_source_walk",
                format!("server source traversal failed: {error}"),
            )
        })?;
        if entry.path() == selected {
            continue;
        }
        if entry.file_type().is_symlink() {
            return Err(import_error(
                "server_source_symlink",
                "server source contains a symbolic link",
            ));
        }
        if entry.file_type().is_file() {
            let below_selection = entry.path().strip_prefix(&selected).map_err(|_| {
                import_error(
                    "server_source_outside_root",
                    "server source escaped its root",
                )
            })?;
            candidates.push(if root_selection {
                below_selection.to_path_buf()
            } else {
                selected_relative.join(below_selection)
            });
        } else if !entry.file_type().is_dir() {
            return Err(import_error(
                "server_source_special_file",
                "server source contains a special file",
            ));
        }
    }
    candidates.sort();
    if candidates.len() > limits.server_source_files {
        return Err(import_error(
            "source_file_limit",
            "server source file count exceeds the configured limit",
        ));
    }

    let source_dir = job_dir.join(SOURCE_DIR);
    std::fs::create_dir_all(&source_dir).with_path(&source_dir)?;
    let mut files = BTreeMap::new();
    let mut path_keys = BTreeSet::new();
    let mut total = 0_u64;
    for (index, path) in candidates.into_iter().enumerate() {
        let relative = path_to_slash(&path)?;
        validate_source_path(&relative, limits)?;
        if !path_keys.insert(collision_key(&relative)) {
            return Err(import_error(
                "source_path_collision",
                "server source paths collide after case and Unicode normalization",
            ));
        }
        let mut source = secure_open_regular(root_handle, &path)?;
        let metadata = source.metadata().with_path(root.join(&path))?;
        reject_hardlink(&metadata)?;
        if metadata.len() > limits.single_source_file_bytes {
            return Err(import_error(
                "source_file_too_large",
                "server source file exceeds the configured limit",
            ));
        }
        total = total.checked_add(metadata.len()).ok_or_else(|| {
            import_error("source_byte_limit", "server source byte count overflowed")
        })?;
        if total > limits.total_source_bytes || total > limits.staged_bytes {
            return Err(import_error(
                "source_byte_limit",
                "server source bytes exceed the configured limit",
            ));
        }
        let file_id = format!("file_{index:08x}_{}", uuid::Uuid::new_v4().simple());
        let destination = source_dir.join(&file_id);
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .with_path(&destination)?;
        let mut hasher = Hasher::new();
        let mut buffer = [0_u8; 1024 * 1024];
        let mut copied = 0_u64;
        loop {
            let read = source.read(&mut buffer).with_path(root.join(&path))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            output.write_all(&buffer[..read]).with_path(&destination)?;
            copied += read as u64;
        }
        if copied != metadata.len() {
            return Err(import_error(
                "server_source_changed",
                "server source file changed while it was copied",
            ));
        }
        output.sync_all().with_path(&destination)?;
        let digest = hasher.finalize().to_hex().to_string();
        files.insert(
            file_id.clone(),
            RegisteredFile {
                file_id,
                relative_path: relative,
                byte_size: copied,
                blake3: digest,
                accepted_bytes: copied,
                complete: true,
                accepted_chunks: BTreeMap::new(),
            },
        );
    }
    sync_directory(&source_dir)?;
    Ok(SourceIndex {
        sealed: false,
        source_fingerprint: None,
        parser_version: None,
        files,
    })
}

pub(super) fn browse_server_directory(
    root_handle: &File,
    relative_directory: &str,
    offset: usize,
    limits: &ImportLimits,
) -> StorageResult<ImportBrowsePage> {
    validate_optional_directory(relative_directory, limits)?;
    let root_selection = relative_directory.is_empty() || relative_directory == ".";
    let selected_relative = if root_selection {
        Path::new("")
    } else {
        Path::new(relative_directory)
    };
    let selected_handle = secure_open_directory(root_handle, selected_relative)?;
    let selected = pinned_handle_path(&selected_handle);
    let directory = std::fs::read_dir(&selected).map_err(|_| {
        import_error(
            "server_source_browse_failed",
            "server source directory could not be listed",
        )
    })?;
    let mut entries = Vec::new();
    for (index, entry) in directory.enumerate() {
        if index >= BROWSE_SCAN_LIMIT {
            return Err(import_error(
                "server_source_browse_limit",
                "server source directory has too many immediate entries",
            ));
        }
        let entry = entry.map_err(|_| {
            import_error(
                "server_source_browse_failed",
                "server source directory could not be listed",
            )
        })?;
        let file_type = entry.file_type().map_err(|_| {
            import_error(
                "server_source_browse_failed",
                "server source entry could not be inspected",
            )
        })?;
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let path = if root_selection {
            PathBuf::from(&name)
        } else {
            selected_relative.join(&name)
        };
        let relative_path = path_to_slash(&path)?;
        validate_optional_directory(&relative_path, limits)?;
        entries.push(ImportBrowseEntry {
            name,
            relative_path,
            kind: ImportBrowseEntryKind::Directory,
            file_id: None,
        });
    }
    entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(paginate_browse_entries(
        if root_selection {
            String::new()
        } else {
            relative_directory.to_string()
        },
        entries,
        offset,
    ))
}

pub(super) fn browse_staged_source(
    index: &SourceIndex,
    profile: ImportProfile,
    relative_directory: &str,
    offset: usize,
    mode: ImportBrowseMode,
    limits: &ImportLimits,
) -> StorageResult<ImportBrowsePage> {
    validate_optional_directory(relative_directory, limits)?;
    let root_selection = relative_directory.is_empty() || relative_directory == ".";
    let prefix = if root_selection {
        String::new()
    } else {
        format!("{}/", relative_directory.trim_end_matches('/'))
    };
    let mut directories = BTreeSet::new();
    let mut files = Vec::new();
    for file in index.files.values().filter(|file| file.complete) {
        let matches = match mode {
            ImportBrowseMode::Descriptors => descriptor_matches(profile, &file.relative_path),
            ImportBrowseMode::Images => is_image_path(&file.relative_path),
        };
        if !matches {
            continue;
        }
        let Some(remainder) = file.relative_path.strip_prefix(&prefix) else {
            continue;
        };
        if let Some((directory, _)) = remainder.split_once('/') {
            let relative_path = if root_selection {
                directory.to_string()
            } else {
                format!("{}/{directory}", relative_directory.trim_end_matches('/'))
            };
            directories.insert((directory.to_string(), relative_path));
        } else if !remainder.is_empty() {
            files.push(ImportBrowseEntry {
                name: remainder.to_string(),
                relative_path: file.relative_path.clone(),
                kind: ImportBrowseEntryKind::File,
                file_id: Some(file.file_id.clone()),
            });
        }
    }
    let mut entries = directories
        .into_iter()
        .map(|(name, relative_path)| ImportBrowseEntry {
            name,
            relative_path,
            kind: ImportBrowseEntryKind::Directory,
            file_id: None,
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    entries.extend(files);
    Ok(paginate_browse_entries(
        if root_selection {
            String::new()
        } else {
            relative_directory.to_string()
        },
        entries,
        offset,
    ))
}

fn paginate_browse_entries(
    relative_path: String,
    entries: Vec<ImportBrowseEntry>,
    offset: usize,
) -> ImportBrowsePage {
    let end = offset.saturating_add(BROWSE_PAGE_SIZE).min(entries.len());
    let page = if offset < entries.len() {
        entries[offset..end].to_vec()
    } else {
        Vec::new()
    };
    ImportBrowsePage {
        relative_path,
        entries: page,
        next_offset: (end < entries.len()).then_some(end),
    }
}

fn descriptor_matches(profile: ImportProfile, path: &str) -> bool {
    let extension = source_extension(path);
    match profile {
        ImportProfile::UltralyticsYoloDetectV1 | ImportProfile::UltralyticsYoloPoseV1 => {
            matches!(extension.as_deref(), Some("yaml" | "yml"))
        }
        ImportProfile::CocoInstancesGtV1 | ImportProfile::CocoKeypointsGtV1 => {
            extension.as_deref() == Some("json")
        }
    }
}

fn is_image_path(path: &str) -> bool {
    matches!(
        source_extension(path).as_deref(),
        Some("jpg" | "jpeg" | "png" | "webp" | "bmp" | "gif")
    )
}

#[cfg(target_os = "linux")]
fn secure_open_directory(root: &File, relative: &Path) -> StorageResult<File> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    let relative = if relative.as_os_str().is_empty() {
        Path::new(".")
    } else {
        relative
    };
    let fd = openat2(
        root,
        relative,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY | OFlags::NOFOLLOW,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|_| {
        import_error(
            "server_source_directory_invalid",
            "selected server source must be a real directory below its configured root",
        )
    })?;
    Ok(File::from(fd))
}

#[cfg(not(target_os = "linux"))]
fn secure_open_directory(_root: &File, _relative: &Path) -> StorageResult<File> {
    Err(import_error(
        "server_source_unsupported",
        "secure server-directory traversal is unavailable on this platform",
    ))
}

#[cfg(target_os = "linux")]
fn pinned_handle_path(handle: &File) -> PathBuf {
    use std::os::fd::AsRawFd;
    PathBuf::from(format!("/proc/self/fd/{}", handle.as_raw_fd()))
}

#[cfg(not(target_os = "linux"))]
fn pinned_handle_path(_handle: &File) -> PathBuf {
    PathBuf::new()
}

pub(super) async fn seal_source(
    job_dir: &Path,
    index: &mut SourceIndex,
    profile_id: &str,
) -> StorageResult<String> {
    let previous_fingerprint = index.source_fingerprint.clone();
    let previous_parser_version = index
        .parser_version
        .as_deref()
        .unwrap_or("labello-storage-import-v1");
    if index.files.is_empty() || index.files.values().any(|file| !file.complete) {
        return Err(import_error(
            "source_incomplete",
            "every registered source file must be complete before sealing",
        ));
    }
    let mut ordered = index.files.values().collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    for file in &ordered {
        let path = job_dir.join(SOURCE_DIR).join(&file.file_id);
        if std::fs::metadata(&path).with_path(&path)?.len() != file.byte_size
            || hash_file(&path)? != file.blake3
        {
            return Err(import_error(
                "source_file_digest_mismatch",
                "staged source changed before sealing",
            ));
        }
    }
    if index.sealed {
        let expected_previous = source_fingerprint(&ordered, profile_id, previous_parser_version)?;
        if previous_fingerprint.as_deref() != Some(&expected_previous) {
            return Err(import_error(
                "source_changed",
                "sealed source fingerprint changed",
            ));
        }
    }
    let fingerprint =
        source_fingerprint(&ordered, profile_id, super::types::IMPORT_PARSER_VERSION)?;
    index.sealed = true;
    index.source_fingerprint = Some(fingerprint.clone());
    index.parser_version = Some(super::types::IMPORT_PARSER_VERSION.to_string());
    save_source_index(job_dir, index).await?;
    Ok(fingerprint)
}

pub(super) fn source_fingerprint(
    ordered: &[&RegisteredFile],
    profile_id: &str,
    parser_version: &str,
) -> StorageResult<String> {
    let mut hasher = Hasher::new();
    hasher.update(b"labello:import-source:v1\0");
    hash_string(&mut hasher, profile_id);
    hash_string(&mut hasher, parser_version);
    for file in ordered {
        hash_string(&mut hasher, &file.relative_path);
        hasher.update(&file.byte_size.to_be_bytes());
        let digest = blake3::Hash::from_hex(&file.blake3)
            .map_err(|_| import_error("digest_invalid", "stored source digest is invalid"))?;
        hasher.update(digest.as_bytes());
    }
    Ok(hasher.finalize().to_hex().to_string())
}

pub(super) async fn load_source_index(job_dir: &Path) -> StorageResult<SourceIndex> {
    let path = job_dir.join(SOURCE_INDEX_FILE);
    if !tokio::fs::try_exists(&path).await.with_path(&path)? {
        return Ok(SourceIndex::default());
    }
    crate::fsjson::read_json(&path).await
}

pub(super) async fn save_source_index(job_dir: &Path, index: &SourceIndex) -> StorageResult<()> {
    write_json_atomic(&job_dir.join(SOURCE_INDEX_FILE), index).await
}

pub(super) fn validate_source_path(path: &str, limits: &ImportLimits) -> StorageResult<()> {
    if path.is_empty()
        || path.len() > limits.source_path_bytes
        || path.contains('\\')
        || path.bytes().any(|byte| byte.is_ascii_control())
        || path.starts_with('/')
        || path.starts_with("//")
    {
        return Err(import_error(
            "source_path_invalid",
            "source path is not portable",
        ));
    }
    let components = path.split('/').collect::<Vec<_>>();
    if components.len() > limits.source_path_depth
        || components.iter().any(|component| {
            component.is_empty()
                || *component == "."
                || *component == ".."
                || component.len() > limits.source_component_bytes
                || component.ends_with(' ')
                || component.ends_with('.')
                || component.contains(':')
                || windows_reserved(component)
        })
        || has_windows_prefix(path)
    {
        return Err(import_error(
            "source_path_invalid",
            "source path is not portable",
        ));
    }
    Ok(())
}

fn validate_optional_directory(path: &str, limits: &ImportLimits) -> StorageResult<()> {
    if path.is_empty() || path == "." {
        Ok(())
    } else {
        validate_source_path(path.trim_end_matches('/'), limits)
    }
}

pub(super) fn join_source_path(
    base: &str,
    relative: &str,
    limits: &ImportLimits,
) -> StorageResult<String> {
    if relative.starts_with('/') || relative.contains("://") || has_windows_prefix(relative) {
        return Err(import_error(
            "source_path_invalid",
            "absolute paths and URLs are not supported",
        ));
    }
    let mut output = Vec::new();
    if !base.is_empty() {
        output.extend(base.split('/'));
    }
    for component in relative.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                return Err(import_error(
                    "source_path_outside_root",
                    "parent components are not supported in source paths",
                ));
            }
            value => output.push(value),
        }
    }
    let path = output.join("/");
    validate_source_path(&path, limits)?;
    Ok(path)
}

pub(super) fn parent_source_path(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

pub(super) fn source_extension(path: &str) -> Option<String> {
    path.rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
}

fn collision_key(path: &str) -> String {
    path.nfc().flat_map(char::to_lowercase).collect()
}

fn windows_reserved(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .to_ascii_lowercase();
    matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || stem
            .strip_prefix("com")
            .or_else(|| stem.strip_prefix("lpt"))
            .is_some_and(|number| {
                matches!(number, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
}

fn has_windows_prefix(path: &str) -> bool {
    path.as_bytes().get(1) == Some(&b':')
        && path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
}

fn validate_digest(value: &str) -> StorageResult<()> {
    if value.len() != 64 || blake3::Hash::from_hex(value).is_err() {
        Err(import_error(
            "digest_invalid",
            "digest must be a 64-character BLAKE3 hex value",
        ))
    } else {
        Ok(())
    }
}

pub(super) fn hash_file(path: &Path) -> StorageResult<String> {
    hash_file_inner(path, None)
}

pub(super) fn hash_file_cancellable(path: &Path, cancelled: &AtomicBool) -> StorageResult<String> {
    hash_file_inner(path, Some(cancelled))
}

fn hash_file_inner(path: &Path, cancelled: Option<&AtomicBool>) -> StorageResult<String> {
    let mut file = File::open(path).with_path(path)?;
    let mut hasher = Hasher::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        if cancelled.is_some_and(|cancelled| cancelled.load(Ordering::Relaxed)) {
            return Err(import_error(
                "parser_cancelled",
                "source hashing was cancelled",
            ));
        }
        let read = file.read(&mut buffer).with_path(path)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn hash_string(hasher: &mut Hasher, value: &str) {
    hasher.update(&(value.len() as u64).to_be_bytes());
    hasher.update(value.as_bytes());
}

fn path_to_slash(path: &Path) -> StorageResult<String> {
    let mut values = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => values.push(value.to_str().ok_or_else(|| {
                import_error("source_path_invalid", "source path is not valid UTF-8")
            })?),
            _ => {
                return Err(import_error(
                    "source_path_invalid",
                    "source path is not relative",
                ));
            }
        }
    }
    Ok(values.join("/"))
}

#[cfg(target_os = "linux")]
fn secure_open_regular(root: &File, relative: &Path) -> StorageResult<File> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    let fd = openat2(
        root,
        relative,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|source| StorageError::Io {
        path: relative.to_path_buf(),
        source: std::io::Error::from_raw_os_error(source.raw_os_error()),
    })?;
    let file = File::from(fd);
    if !file.metadata().with_path(relative)?.file_type().is_file() {
        return Err(import_error(
            "server_source_special_file",
            "server source entry is not a regular file",
        ));
    }
    Ok(file)
}

#[cfg(not(target_os = "linux"))]
fn secure_open_regular(_root: &File, _relative: &Path) -> StorageResult<File> {
    Err(import_error(
        "server_source_unsupported",
        "secure server-directory traversal is unavailable on this platform",
    ))
}

#[cfg(unix)]
fn reject_hardlink(metadata: &std::fs::Metadata) -> StorageResult<()> {
    use std::os::unix::fs::MetadataExt;
    if metadata.nlink() != 1 {
        Err(import_error(
            "server_source_hardlink",
            "server source regular files must have exactly one hard link",
        ))
    } else {
        Ok(())
    }
}

#[cfg(not(unix))]
fn reject_hardlink(_metadata: &std::fs::Metadata) -> StorageResult<()> {
    Err(import_error(
        "server_source_unsupported",
        "hard-link validation is unavailable on this platform",
    ))
}

pub(super) fn sync_directory(path: &Path) -> StorageResult<()> {
    File::open(path).with_path(path)?.sync_all().with_path(path)
}

pub(super) fn import_error(code: impl Into<String>, message: impl Into<String>) -> StorageError {
    StorageError::Import {
        code: code.into(),
        message: message.into(),
    }
}
