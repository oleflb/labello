#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StorageIdentity {
    pub server: String,
    pub user_id: UserId,
}

impl StorageIdentity {
    pub(crate) fn new(api_base_url: &str, user_id: UserId) -> Result<Self, String> {
        Ok(Self {
            server: normalize_server_identity(api_base_url)?,
            user_id,
        })
    }

    fn prefix(&self) -> String {
        format!(
            "{LOCAL_PREFIX}:{}:{}",
            key_segment(&self.server),
            key_segment(self.user_id.as_str())
        )
    }

    fn owns_key(&self, key: &str) -> bool {
        key.strip_prefix(&self.prefix())
            .is_some_and(|suffix| suffix.starts_with(':'))
    }
}

pub(crate) fn normalize_server_identity(value: &str) -> Result<String, String> {
    let mut url = url::Url::parse(value.trim())
        .map_err(|error| format!("API URL cannot identify browser storage: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("API URL must use http or https and include a host".to_string());
    }
    url.set_fragment(None);
    url.set_query(None);
    let path = url.path().trim_end_matches('/').to_string();
    url.set_path(if path.is_empty() { "/" } else { &path });
    let mut normalized = url.to_string();
    if normalized.ends_with('/') {
        normalized.pop();
    }
    Ok(normalized)
}
