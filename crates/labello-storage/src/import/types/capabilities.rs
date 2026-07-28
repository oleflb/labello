pub const PROFILE_YOLO_DETECT: &str = "ultralytics_yolo_detect_v1";
pub const PROFILE_YOLO_POSE: &str = "ultralytics_yolo_pose_v1";
pub const PROFILE_COCO_INSTANCES: &str = "coco_instances_gt_v1";
pub const PROFILE_COCO_KEYPOINTS: &str = "coco_keypoints_gt_v1";
pub const IMPORT_PARSER_VERSION: &str = "labello-storage-import-v2";
pub const MAX_IMAGE_VALIDATION_WORKERS: usize = 32;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportConfig {
    pub enabled: bool,
    pub import_roots: Vec<ImportRoot>,
    pub allowed_profiles: Vec<ImportProfile>,
    pub limits: ImportLimits,
    pub retain_raw_source: bool,
    pub failed_retention: Duration,
    pub successful_metadata_retention: Duration,
}

impl Default for ImportConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            import_roots: Vec::new(),
            allowed_profiles: ImportProfile::ALL.to_vec(),
            limits: ImportLimits::default(),
            retain_raw_source: false,
            failed_retention: Duration::from_secs(24 * 60 * 60),
            successful_metadata_retention: Duration::from_secs(30 * 24 * 60 * 60),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRoot {
    pub root_id: String,
    pub path: PathBuf,
    /// Empty means access is controlled entirely by the caller's bootstrap-admin check.
    pub allowed_owners: Vec<UserId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportLimits {
    pub concurrent_build_jobs: usize,
    pub image_validation_workers: usize,
    pub decoded_image_memory_bytes: u64,
    pub concurrent_browser_upload_jobs: usize,
    pub active_reservations_per_owner: usize,
    pub browser_source_files: usize,
    pub browser_source_bytes: u64,
    pub server_source_files: usize,
    pub total_source_bytes: u64,
    pub selected_images: usize,
    pub single_source_file_bytes: u64,
    pub descriptor_bytes: u64,
    pub upload_chunk_bytes: usize,
    pub source_path_bytes: usize,
    pub source_path_depth: usize,
    pub source_component_bytes: usize,
    pub selected_categories: usize,
    pub selected_tasks: usize,
    pub coverage_entries: usize,
    pub annotations_total: usize,
    pub annotations_per_image: usize,
    pub generated_file_bytes_per_image: u64,
    pub keypoints_per_skeleton: usize,
    pub yolo_line_bytes: usize,
    pub yolo_columns: usize,
    pub structured_data_nesting: usize,
    pub decoded_image_pixels: u64,
    pub decoded_image_bytes: u64,
    pub staged_bytes: u64,
    pub diagnostic_examples_per_code: usize,
}

impl Default for ImportLimits {
    fn default() -> Self {
        Self {
            concurrent_build_jobs: 1,
            image_validation_workers: 8,
            decoded_image_memory_bytes: 5 * 1024 * 1024 * 1024,
            concurrent_browser_upload_jobs: 2,
            active_reservations_per_owner: 2,
            browser_source_files: 25_000,
            browser_source_bytes: 20 * 1024 * 1024 * 1024,
            server_source_files: 50_000,
            total_source_bytes: 100 * 1024 * 1024 * 1024,
            selected_images: 10_000,
            single_source_file_bytes: 4 * 1024 * 1024 * 1024,
            descriptor_bytes: 16 * 1024 * 1024,
            upload_chunk_bytes: 8 * 1024 * 1024,
            source_path_bytes: 1024,
            source_path_depth: 32,
            source_component_bytes: 255,
            selected_categories: 100,
            selected_tasks: 200,
            coverage_entries: 2_000_000,
            annotations_total: 1_000_000,
            annotations_per_image: 10_000,
            generated_file_bytes_per_image: 64 * 1024 * 1024,
            keypoints_per_skeleton: 512,
            yolo_line_bytes: 1024 * 1024,
            yolo_columns: 4096,
            structured_data_nesting: 64,
            decoded_image_pixels: 50_000_000,
            decoded_image_bytes: 512 * 1024 * 1024,
            staged_bytes: 250 * 1024 * 1024 * 1024,
            diagnostic_examples_per_code: 100,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportProfile {
    UltralyticsYoloDetectV1,
    UltralyticsYoloPoseV1,
    CocoInstancesGtV1,
    CocoKeypointsGtV1,
}

impl ImportProfile {
    pub const ALL: [Self; 4] = [
        Self::UltralyticsYoloDetectV1,
        Self::UltralyticsYoloPoseV1,
        Self::CocoInstancesGtV1,
        Self::CocoKeypointsGtV1,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::UltralyticsYoloDetectV1 => PROFILE_YOLO_DETECT,
            Self::UltralyticsYoloPoseV1 => PROFILE_YOLO_POSE,
            Self::CocoInstancesGtV1 => PROFILE_COCO_INSTANCES,
            Self::CocoKeypointsGtV1 => PROFILE_COCO_KEYPOINTS,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportCapabilities {
    pub available: bool,
    pub unavailable_reason: Option<String>,
    pub profiles: Vec<ImportProfile>,
    pub browser_upload: bool,
    pub server_directory_roots: Vec<String>,
    pub limits: ImportLimits,
    pub schema_version: u32,
    pub parser_version: String,
    pub atomic_publication: bool,
    pub secure_server_open: bool,
}
