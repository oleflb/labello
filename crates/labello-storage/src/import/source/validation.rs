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
