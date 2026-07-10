use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

use labello_domain::{ImageId, ImageRecord};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::{
    DatasetRepository,
    error::{PathIo, StorageError, StorageResult},
    paths,
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestReport {
    pub discovered_files: usize,
    pub new_images: usize,
    pub duplicate_files: Vec<DuplicateImage>,
    pub changed_paths: Vec<ChangedPath>,
    pub unreadable_files: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateImage {
    pub image_id: ImageId,
    pub canonical_path: String,
    pub duplicate_path: String,
    pub blake3: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedPath {
    pub relative_path: String,
    pub previous_blake3: String,
    pub current_blake3: String,
}

impl DatasetRepository {
    pub async fn ingest_images(&self) -> StorageResult<IngestReport> {
        self.ensure_layout().await?;
        let mut metadata = self.load_dataset().await?;
        let mut index = self.load_images_index().await?;
        let mut report = IngestReport::default();
        let mut path_to_hash = path_to_hash(&metadata.images);
        let mut seen_by_hash: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut reconciled_roots = Vec::new();
        let mut preserve_paths = BTreeSet::new();

        for scan_root in self.scan_roots(&metadata).await? {
            if !tokio::fs::try_exists(&scan_root)
                .await
                .with_path(&scan_root)?
            {
                report
                    .unreadable_files
                    .push(self.relative_path_lossy(&scan_root));
                continue;
            }
            reconciled_roots.push(self.relative_path(&scan_root)?);

            for entry in WalkDir::new(&scan_root)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|e| e.file_type().is_file())
            {
                let path = entry.path().to_path_buf();
                let Ok(relative_path) = self.relative_path(&path) else {
                    report.unreadable_files.push(path.display().to_string());
                    continue;
                };
                if !looks_like_image(&path) {
                    continue;
                }
                report.discovered_files += 1;
                match read_image_record(&path, &relative_path).await {
                    Ok((hash, partial)) => {
                        if let Some(previous_hash) = path_to_hash.get(&relative_path)
                            && previous_hash != &hash
                        {
                            report.changed_paths.push(ChangedPath {
                                relative_path: relative_path.clone(),
                                previous_blake3: previous_hash.clone(),
                                current_blake3: hash.clone(),
                            });
                        }
                        seen_by_hash
                            .entry(hash.clone())
                            .or_default()
                            .insert(relative_path.clone());
                        if let Some(existing) = index.images_by_hash.get_mut(&hash) {
                            let newly_discovered = !existing.known_paths.contains(&relative_path);
                            if newly_discovered {
                                existing.known_paths.push(relative_path.clone());
                            }
                            if relative_path != existing.canonical_path
                                && !existing.duplicate_paths.contains(&relative_path)
                            {
                                existing.duplicate_paths.push(relative_path.clone());
                            }
                            if newly_discovered && relative_path != existing.canonical_path {
                                report.duplicate_files.push(DuplicateImage {
                                    image_id: existing.image_id.clone(),
                                    canonical_path: existing.canonical_path.clone(),
                                    duplicate_path: relative_path,
                                    blake3: hash,
                                });
                            }
                        } else {
                            index.images_by_hash.insert(hash.clone(), partial);
                            report.new_images += 1;
                        }
                    }
                    Err(_) => {
                        preserve_paths.insert(relative_path.clone());
                        report.unreadable_files.push(relative_path);
                    }
                }
            }
        }

        for (hash, record) in &mut index.images_by_hash {
            let paths_seen = seen_by_hash.get(hash);
            record.known_paths.retain(|path| {
                !path_in_roots(path, &reconciled_roots)
                    || paths_seen.is_some_and(|paths| paths.contains(path))
                    || preserve_paths.contains(path)
            });
            record
                .duplicate_paths
                .retain(|path| record.known_paths.contains(path) && path != &record.canonical_path);
            if !record.known_paths.contains(&record.canonical_path)
                && let Some(new_canonical) = record.known_paths.first()
            {
                record.canonical_path = new_canonical.clone();
                record.file_name = Path::new(new_canonical)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(new_canonical)
                    .to_string();
            }
        }
        index
            .images_by_hash
            .retain(|_, record| !record.known_paths.is_empty());

        metadata.images = index
            .images_by_hash
            .values()
            .map(|record| (record.image_id.clone(), record.clone()))
            .collect();
        metadata.updated_at = labello_domain::now();
        self.save_images_index(&index).await?;
        self.save_dataset(&metadata).await?;
        path_to_hash.clear();
        Ok(report)
    }

    async fn scan_roots(
        &self,
        metadata: &labello_domain::DatasetMetadata,
    ) -> StorageResult<Vec<PathBuf>> {
        let roots = if metadata.image_roots.is_empty() {
            vec![paths::IMAGES_DIR.to_string()]
        } else {
            metadata.image_roots.clone()
        };
        roots
            .into_iter()
            .map(|root| self.safe_relative_root(&root))
            .collect()
    }

    pub fn safe_relative_root(&self, relative_root: &str) -> StorageResult<PathBuf> {
        let path = Path::new(relative_root);
        if relative_root.trim().is_empty()
            || path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(StorageError::OutsideDatasetRoot(path.to_path_buf()));
        }
        Ok(self.root().join(path))
    }

    fn relative_path(&self, path: &Path) -> StorageResult<String> {
        let relative = path
            .strip_prefix(self.root())
            .map_err(|_| StorageError::OutsideDatasetRoot(path.to_path_buf()))?;
        Ok(relative
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"))
    }

    fn relative_path_lossy(&self, path: &Path) -> String {
        self.relative_path(path)
            .unwrap_or_else(|_| path.display().to_string())
    }
}

async fn read_image_record(
    path: &Path,
    relative_path: &str,
) -> StorageResult<(String, ImageRecord)> {
    let path = path.to_path_buf();
    let relative_path = relative_path.to_string();
    tokio::task::spawn_blocking(move || read_image_record_blocking(path, relative_path))
        .await
        .map_err(|error| StorageError::Io {
            path: PathBuf::from("ingest-image-worker"),
            source: std::io::Error::other(error.to_string()),
        })?
}

fn read_image_record_blocking(
    path: PathBuf,
    relative_path: String,
) -> StorageResult<(String, ImageRecord)> {
    let bytes = std::fs::read(&path).with_path(&path)?;
    let hash = blake3::hash(&bytes).to_hex().to_string();
    let metadata = std::fs::metadata(&path).with_path(&path)?;
    let (width, height) = image::image_dimensions(&path).map_err(|source| StorageError::Image {
        path: path.clone(),
        source,
    })?;
    let media_type = infer::get(&bytes)
        .map(|kind| kind.mime_type().to_string())
        .unwrap_or_else(|| {
            mime_guess::from_path(&path)
                .first_or_octet_stream()
                .essence_str()
                .to_string()
        });
    let image_id = ImageId::from_blake3_hex(&hash);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&relative_path)
        .to_string();
    let record = ImageRecord {
        image_id,
        blake3: hash.clone(),
        canonical_path: relative_path.clone(),
        known_paths: vec![relative_path],
        duplicate_paths: Vec::new(),
        file_name,
        byte_size: metadata.len(),
        width,
        height,
        media_type,
    };
    Ok((hash, record))
}

fn looks_like_image(path: &Path) -> bool {
    mime_guess::from_path(path)
        .first()
        .is_some_and(|mime| mime.type_() == "image")
}

fn path_to_hash(images: &BTreeMap<ImageId, ImageRecord>) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for record in images.values() {
        for path in &record.known_paths {
            map.insert(path.clone(), record.blake3.clone());
        }
    }
    map
}

fn path_in_roots(path: &str, roots: &[String]) -> bool {
    roots
        .iter()
        .any(|root| path == root || path.starts_with(&format!("{root}/")))
}

#[cfg(test)]
mod tests {
    use image::{ImageBuffer, Rgba};
    use labello_domain::{DatasetId, DatasetMetadata, now};

    use super::*;

    #[tokio::test]
    async fn deduplicates_images_by_blake3() {
        let temp = tempfile::tempdir().unwrap();
        let repo = DatasetRepository::new(temp.path());
        repo.initialize(DatasetMetadata::new(
            DatasetId::from("ds"),
            "Dataset",
            now(),
        ))
        .await
        .unwrap();
        let images = temp.path().join(paths::IMAGES_DIR);
        tokio::fs::create_dir_all(images.join("dupes"))
            .await
            .unwrap();
        write_png(&images.join("a.png"));
        std::fs::copy(images.join("a.png"), images.join("dupes/a-copy.png")).unwrap();

        let report = repo.ingest_images().await.unwrap();
        let metadata = repo.load_dataset().await.unwrap();
        assert_eq!(report.new_images, 1);
        assert_eq!(report.duplicate_files.len(), 1);
        assert_eq!(metadata.images.len(), 1);
        let image = metadata.images.values().next().unwrap();
        assert_eq!(image.width, 2);
        assert_eq!(image.height, 2);
        assert_eq!(image.duplicate_paths.len(), 1);

        let repeated = repo.ingest_images().await.unwrap();
        assert!(repeated.duplicate_files.is_empty());
    }

    #[tokio::test]
    async fn scans_configured_image_roots() {
        let temp = tempfile::tempdir().unwrap();
        let repo = DatasetRepository::new(temp.path());
        let mut metadata = DatasetMetadata::new(DatasetId::from("ds"), "Dataset", now());
        metadata.image_roots = vec!["images".to_string(), "imports/batch-1".to_string()];
        repo.initialize(metadata).await.unwrap();
        tokio::fs::create_dir_all(temp.path().join("imports/batch-1"))
            .await
            .unwrap();
        write_png(&temp.path().join("imports/batch-1/a.png"));

        let report = repo.ingest_images().await.unwrap();
        let metadata = repo.load_dataset().await.unwrap();
        assert_eq!(report.new_images, 1);
        let image = metadata.images.values().next().unwrap();
        assert_eq!(image.canonical_path, "imports/batch-1/a.png");
    }

    #[tokio::test]
    async fn rejects_unsafe_image_roots() {
        let temp = tempfile::tempdir().unwrap();
        let repo = DatasetRepository::new(temp.path());
        assert!(repo.safe_relative_root("../outside").is_err());
        assert!(repo.safe_relative_root("/tmp/images").is_err());
    }

    #[tokio::test]
    async fn reconciles_changed_and_removed_paths_without_stale_images() {
        let temp = tempfile::tempdir().unwrap();
        let repo = DatasetRepository::new(temp.path());
        repo.initialize(DatasetMetadata::new(
            DatasetId::from("ds"),
            "Dataset",
            now(),
        ))
        .await
        .unwrap();
        let image_path = temp.path().join(paths::IMAGES_DIR).join("a.png");
        write_png_color(&image_path, [1, 2, 3, 255]);
        repo.ingest_images().await.unwrap();
        let original_id = repo
            .load_dataset()
            .await
            .unwrap()
            .images
            .into_keys()
            .next()
            .unwrap();

        write_png_color(&image_path, [9, 8, 7, 255]);
        let changed = repo.ingest_images().await.unwrap();
        let metadata = repo.load_dataset().await.unwrap();
        assert_eq!(changed.changed_paths.len(), 1);
        assert_eq!(metadata.images.len(), 1);
        assert!(!metadata.images.contains_key(&original_id));

        std::fs::remove_file(image_path).unwrap();
        repo.ingest_images().await.unwrap();
        assert!(repo.load_dataset().await.unwrap().images.is_empty());
    }

    fn write_png(path: &Path) {
        write_png_color(path, [1, 2, 3, 255]);
    }

    fn write_png_color(path: &Path, color: [u8; 4]) {
        let image: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_pixel(2, 2, Rgba(color));
        image.save(path).unwrap();
    }
}
