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
