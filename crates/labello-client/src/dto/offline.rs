#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfflineBundleRequest {
    #[serde(default = "default_offline_limit")]
    pub limit: usize,
    #[serde(default)]
    pub include_image_bytes: bool,
}

impl Default for OfflineBundleRequest {
    fn default() -> Self {
        Self {
            limit: 25,
            include_image_bytes: false,
        }
    }
}

fn default_offline_limit() -> usize {
    25
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfflineSyncEnvelope {
    pub request: OfflineSyncRequest,
}
