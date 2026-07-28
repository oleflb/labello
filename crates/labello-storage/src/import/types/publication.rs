#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportCommitResult {
    pub import_id: ImportId,
    pub dataset_id: DatasetId,
    pub dataset_path: PathBuf,
    pub recovered: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryReport {
    pub recovered_successes: usize,
    pub resumed_to_awaiting_decision: usize,
    pub failed_incomplete_commits: usize,
    pub released_reservations: usize,
    pub expired_abandoned_jobs: usize,
}
