#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportTransport {
    Browser,
    ServerDirectory,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportJobPhase {
    Registering,
    Uploading,
    Sealed,
    Preflighting,
    AwaitingDecision,
    Building,
    Verifying,
    Committing,
    Succeeded,
    Failed,
    Cancelled,
    Expired,
}

impl ImportJobPhase {
    pub fn terminal(&self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Expired
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportJob {
    pub schema_version: u32,
    pub import_id: ImportId,
    pub owner_user_id: UserId,
    pub destination_dataset_id: DatasetId,
    pub destination_name: String,
    pub profile: ImportProfile,
    pub transport: ImportTransport,
    pub phase: ImportJobPhase,
    pub source_fingerprint: Option<String>,
    pub plan_hash: Option<String>,
    #[serde(default)]
    pub preflight_generation: Option<String>,
    pub accepted_files: usize,
    pub accepted_bytes: u64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub failure_code: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateImportRequest {
    pub destination_dataset_id: DatasetId,
    pub destination_name: String,
    pub profile: ImportProfile,
    pub transport: ImportTransport,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserFileRegistration {
    pub relative_path: String,
    pub byte_size: u64,
    pub blake3: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredFile {
    pub file_id: String,
    pub relative_path: String,
    pub byte_size: u64,
    pub blake3: String,
    pub accepted_bytes: u64,
    pub complete: bool,
    #[serde(default)]
    pub accepted_chunks: BTreeMap<u64, AcceptedChunk>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptedChunk {
    pub length: usize,
    pub blake3: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerDirectorySelection {
    pub root_id: String,
    pub relative_directory: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportBrowseMode {
    Descriptors,
    Images,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportBrowseEntryKind {
    Directory,
    File,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportBrowseEntry {
    pub name: String,
    pub relative_path: String,
    pub kind: ImportBrowseEntryKind,
    pub file_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportBrowsePage {
    pub relative_path: String,
    pub entries: Vec<ImportBrowseEntry>,
    pub next_offset: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct YoloDescriptorInspection {
    pub splits: Vec<YoloSplitInspection>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct YoloSplitInspection {
    pub name: String,
    pub usable: bool,
    pub issue: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CocoDescriptorSelection {
    pub kind: ImportDescriptorKind,
    pub descriptor_path: String,
    pub image_root: String,
    pub split: String,
    pub source_namespace: String,
    pub release: String,
    pub pairing_group: Option<String>,
}
