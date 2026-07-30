use serde::Deserialize;

const MAX_BROWSER_CONFIG_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BrowserConfig {
    api_base_url: Option<String>,
}

impl BrowserConfig {
    fn parse(text: &str) -> Result<Self, String> {
        if text.len() > MAX_BROWSER_CONFIG_BYTES {
            return Err("browser runtime configuration exceeds 16 KiB".to_string());
        }
        let mut config: Self = serde_json::from_str(text)
            .map_err(|_| "browser runtime configuration is invalid".to_string())?;
        config.api_base_url = config
            .api_base_url
            .as_deref()
            .map(validate_api_base_url)
            .transpose()?;
        Ok(config)
    }

    pub(crate) fn api_base_url(&self) -> Option<&str> {
        self.api_base_url.as_deref()
    }
}

pub(crate) fn resolve_api_base_url(
    query_override: Option<&str>,
    runtime_config: &BrowserConfig,
    protocol: &str,
    hostname: &str,
) -> String {
    query_override
        .or_else(|| runtime_config.api_base_url())
        .map(str::to_string)
        .unwrap_or_else(|| default_api_url(protocol, hostname))
}

fn validate_api_base_url(value: &str) -> Result<String, String> {
    let authority = value
        .split_once("://")
        .map(|(_, authority)| authority)
        .unwrap_or_default();
    if authority.is_empty() || authority.starts_with('/') {
        return Err(
            "browser runtime apiBaseUrl must use http or https and include a host".to_string(),
        );
    }
    let url = url::Url::parse(value)
        .map_err(|_| "browser runtime apiBaseUrl must be an absolute URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(
            "browser runtime apiBaseUrl must use http or https and include a host".to_string(),
        );
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("browser runtime apiBaseUrl must not contain credentials".to_string());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("browser runtime apiBaseUrl must not contain a query or fragment".to_string());
    }
    if url.path() != "/" && !url.path().ends_with('/') {
        return Err("browser runtime apiBaseUrl path prefixes must end with a slash".to_string());
    }
    Ok(url.to_string())
}

fn default_api_url(protocol: &str, hostname: &str) -> String {
    format!("{protocol}//{hostname}:8080")
}

#[cfg(target_arch = "wasm32")]
pub(crate) async fn load() -> Result<BrowserConfig, wasm_bindgen::JsValue> {
    use wasm_bindgen::JsCast;

    let window = web_sys::window()
        .ok_or_else(|| wasm_bindgen::JsValue::from_str("missing browser window"))?;
    let document_url = window
        .location()
        .href()
        .map_err(|_| config_error("cannot resolve browser runtime configuration"))?;
    let config_url = web_sys::Url::new_with_base("./labello.client.json", &document_url)
        .map_err(|_| config_error("cannot resolve browser runtime configuration"))?
        .href();
    let init = web_sys::RequestInit::new();
    init.set_method("GET");
    init.set_cache(web_sys::RequestCache::NoStore);
    let request = web_sys::Request::new_with_str_and_init(&config_url, &init)
        .map_err(|_| config_error("cannot request browser runtime configuration"))?;
    request
        .headers()
        .set("accept", "application/json")
        .map_err(|_| config_error("cannot request browser runtime configuration"))?;
    let response = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|_| config_error("cannot load browser runtime configuration"))?
        .dyn_into::<web_sys::Response>()
        .map_err(|_| config_error("browser runtime configuration response is invalid"))?;
    if response.status() == 404 {
        return Ok(BrowserConfig::default());
    }
    if !response.ok() {
        return Err(config_error(&format!(
            "browser runtime configuration request failed with status {}",
            response.status()
        )));
    }
    let text = wasm_bindgen_futures::JsFuture::from(
        response
            .text()
            .map_err(|_| config_error("cannot read browser runtime configuration"))?,
    )
    .await
    .map_err(|_| config_error("cannot read browser runtime configuration"))?
    .as_string()
    .ok_or_else(|| config_error("browser runtime configuration response is not text"))?;
    BrowserConfig::parse(&text).map_err(|error| config_error(&error))
}

#[cfg(target_arch = "wasm32")]
fn config_error(message: &str) -> wasm_bindgen::JsValue {
    wasm_bindgen::JsValue::from_str(message)
}

#[cfg(test)]
mod tests {
    use super::{BrowserConfig, MAX_BROWSER_CONFIG_BYTES, resolve_api_base_url};

    #[test]
    fn empty_or_null_config_preserves_the_legacy_default() {
        for text in [r#"{}"#, r#"{"apiBaseUrl":null}"#] {
            let config = BrowserConfig::parse(text).unwrap();
            assert_eq!(config.api_base_url(), None);
            assert_eq!(
                resolve_api_base_url(None, &config, "https:", "labello.example"),
                "https://labello.example:8080"
            );
        }
    }

    #[test]
    fn tracked_example_contains_every_supported_field_with_its_default() {
        let text = include_str!("../labello.client.example.json");
        let value: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(value, serde_json::json!({ "apiBaseUrl": null }));
        assert_eq!(
            BrowserConfig::parse(text).unwrap(),
            BrowserConfig::default()
        );
    }

    #[test]
    fn runtime_config_accepts_http_https_and_trailing_slash_prefixes() {
        for (value, expected) in [
            ("http://127.0.0.1:8090", "http://127.0.0.1:8090/"),
            ("https://api.example.com", "https://api.example.com/"),
            (
                "https://example.com/labello-api/",
                "https://example.com/labello-api/",
            ),
        ] {
            let config = BrowserConfig::parse(&format!(r#"{{"apiBaseUrl":"{value}"}}"#)).unwrap();
            assert_eq!(config.api_base_url(), Some(expected));
        }
    }

    #[test]
    fn runtime_config_rejects_unsafe_or_ambiguous_api_urls() {
        for value in [
            "/api",
            "ftp://example.com/",
            "http:///api",
            "https://user@example.com/",
            "https://user:secret@example.com/",
            "https://example.com/?query=value",
            "https://example.com/#fragment",
            "https://example.com/api",
        ] {
            assert!(
                BrowserConfig::parse(&format!(r#"{{"apiBaseUrl":"{value}"}}"#)).is_err(),
                "{value} should be rejected"
            );
        }
    }

    #[test]
    fn runtime_config_rejects_unknown_fields_malformed_json_and_large_files() {
        assert!(BrowserConfig::parse(r#"{"apiUrl":"https://example.com"}"#).is_err());
        assert!(BrowserConfig::parse("{").is_err());
        assert!(BrowserConfig::parse(&" ".repeat(MAX_BROWSER_CONFIG_BYTES + 1)).is_err());
    }

    #[test]
    fn query_override_wins_over_runtime_config() {
        let config =
            BrowserConfig::parse(r#"{"apiBaseUrl":"https://configured.example"}"#).unwrap();
        assert_eq!(
            resolve_api_base_url(
                Some("https://override.example"),
                &config,
                "https:",
                "app.example"
            ),
            "https://override.example"
        );
        assert_eq!(
            resolve_api_base_url(None, &config, "https:", "app.example"),
            "https://configured.example/"
        );
    }
}
