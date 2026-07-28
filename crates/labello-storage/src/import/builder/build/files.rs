fn canonical_images(ir: &ImportIr) -> StorageResult<BTreeMap<String, Vec<String>>> {
    let mut values: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for image in ir.images.values() {
        values
            .entry(image.blake3.clone())
            .or_default()
            .push(image.source_key.clone());
    }
    for keys in values.values_mut() {
        keys.sort();
    }
    Ok(values)
}

fn copy_verified(
    source: &Path,
    destination: &Path,
    expected_hash: &str,
    expected_size: u64,
) -> StorageResult<()> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).with_path(parent)?;
    }
    let mut input = std::fs::File::open(source).with_path(source)?;
    let mut output = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)
        .with_path(destination)?;
    let mut hasher = blake3::Hasher::new();
    let mut copied = 0_u64;
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let read = input.read(&mut buffer).with_path(source)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        output.write_all(&buffer[..read]).with_path(destination)?;
        copied += read as u64;
    }
    if copied != expected_size || hasher.finalize().to_hex().as_str() != expected_hash {
        return Err(import_error(
            "source_changed",
            "image bytes changed after source sealing",
        ));
    }
    output.sync_all().with_path(destination)
}

fn to_native_box(value: F64Box) -> StorageResult<BoundingBox> {
    let output = BoundingBox {
        x: value.x as f32,
        y: value.y as f32,
        width: value.width as f32,
        height: value.height as f32,
    };
    output.validate()?;
    Ok(output)
}
fn to_native_skeleton(values: &[IrKeypoint]) -> StorageResult<SkeletonGeometry> {
    let output = SkeletonGeometry {
        keypoints: values
            .iter()
            .map(|value| KeypointAnnotation {
                name: value.name.clone(),
                state: value.state.clone(),
                point: value.x.zip(value.y).map(|(x, y)| NormalizedPoint {
                    x: x as f32,
                    y: y as f32,
                }),
            })
            .collect(),
    };
    output.validate()?;
    Ok(output)
}
fn template_skeleton(bbox: F64Box, values: &[TemplateKeypoint]) -> StorageResult<SkeletonGeometry> {
    let mut points = Vec::with_capacity(values.len());
    for value in values {
        if !value.x.is_finite()
            || !value.y.is_finite()
            || !(0.0..=1.0).contains(&value.x)
            || !(0.0..=1.0).contains(&value.y)
            || value.name.is_empty()
        {
            return Err(import_error(
                "template_keypoint_invalid",
                "template keypoints require unique names and finite relative coordinates",
            ));
        }
        let point =
            (value.state != labello_domain::KeypointState::Absent).then_some(NormalizedPoint {
                x: (bbox.x + value.x * bbox.width) as f32,
                y: (bbox.y + value.y * bbox.height) as f32,
            });
        points.push(KeypointAnnotation {
            name: value.name.clone(),
            state: value.state.clone(),
            point,
        });
    }
    if points
        .iter()
        .map(|point| &point.name)
        .collect::<BTreeSet<_>>()
        .len()
        != points.len()
    {
        return Err(import_error(
            "template_keypoint_invalid",
            "template keypoint names must be unique",
        ));
    }
    let output = SkeletonGeometry { keypoints: points };
    output.validate()?;
    Ok(output)
}

fn deterministic_id(prefix: &str, import_id: &str, source_key: &str, target: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"labello:import-id:v1\0");
    for value in [import_id, source_key, target] {
        hasher.update(&(value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    format!("{prefix}_{}", hasher.finalize().to_hex())
}

fn write_source_objects(
    path: &Path,
    ir: &ImportIr,
    outputs: &BTreeMap<String, ImportedObjectMapping>,
) -> StorageResult<()> {
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .with_path(path)?;
    let mut objects = ir.objects.iter().collect::<Vec<_>>();
    objects.sort_by(|left, right| left.source_object_key.cmp(&right.source_object_key));
    for object in objects {
        let record = SourceObjectRecord {
            source_object_key: &object.source_object_key,
            source_namespace: &object.source_namespace,
            source_image_key: &object.source_image_key,
            source_category_key: &object.source_category_key,
            direct_bbox: object.direct_bbox,
            direct_skeleton: object.direct_skeleton.as_deref(),
            source_bbox: object.source_bbox.as_deref(),
            source_area: object.source_area,
            clipped: object.clipped,
            normalization: object.boundary_rounding_normalized.then_some(
                SourceObjectNormalization {
                    transform_id: "yolo_boundary_rounding_v1",
                    tolerance: super::formats::YOLO_BOUNDARY_ROUNDING_TOLERANCE,
                },
            ),
            output: outputs.get(&object.source_object_key).cloned().unwrap_or(
                ImportedObjectMapping {
                    source_object_key: object.source_object_key.clone(),
                    object_group_id: None,
                    annotation_ids: Vec::new(),
                },
            ),
        };
        let line = serde_json::to_vec(&record).map_err(|source| StorageError::Json {
            path: path.to_path_buf(),
            source,
        })?;
        file.write_all(&line).with_path(path)?;
        file.write_all(b"\n").with_path(path)?;
    }
    file.sync_all().with_path(path)
}

fn output_integrity(
    root: &Path,
    excluded_names: &[&str],
) -> StorageResult<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|error| import_error("output_walk_failed", error.to_string()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .expect("walk root")
            .to_string_lossy()
            .replace('\\', "/");
        if excluded_names.iter().any(|name| relative.ends_with(name)) {
            continue;
        }
        values.insert(relative, super::source::hash_file(entry.path())?);
    }
    Ok(values)
}

fn sync_tree(root: &Path) -> StorageResult<()> {
    let mut directories = Vec::<PathBuf>::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|error| import_error("output_sync_failed", error.to_string()))?;
        if entry.file_type().is_symlink() {
            return Err(import_error(
                "output_symlink",
                "generated output unexpectedly contains a symlink",
            ));
        }
        if entry.file_type().is_file() {
            std::fs::File::open(entry.path())
                .with_path(entry.path())?
                .sync_all()
                .with_path(entry.path())?;
        } else if entry.file_type().is_dir() {
            directories.push(entry.path().to_path_buf());
        }
    }
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for directory in directories {
        sync_directory(&directory)?;
    }
    Ok(())
}
