#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn start() {
    let mut logging = tracing_wasm::WASMLayerConfigBuilder::new();
    logging
        .set_report_logs_in_timings(false)
        .set_console_config(tracing_wasm::ConsoleConfig::ReportWithoutConsoleColor)
        .set_max_level(if cfg!(debug_assertions) {
            tracing::Level::DEBUG
        } else {
            tracing::Level::WARN
        });
    tracing_wasm::set_as_global_default_with_config(logging.build());
    console_error_panic_hook::set_once();
    tracing::info!(
        event = "wasm.started",
        version = env!("CARGO_PKG_VERSION"),
        "Labello WASM starting"
    );
    wasm_bindgen_futures::spawn_local(async {
        if let Err(error) = run().await {
            tracing::error!(
                event = "wasm.startup.failed",
                "Labello WASM failed to start"
            );
            web_sys::console::error_1(&error);
            show_startup_error(&error);
        }
    });
}

#[cfg(target_arch = "wasm32")]
async fn run() -> Result<(), JsValue> {
    let canvas = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("labello-canvas"))
        .ok_or_else(|| JsValue::from_str("missing #labello-canvas element"))?
        .dyn_into::<web_sys::HtmlCanvasElement>()?;
    let config = app_config_from_url()?;
    let options = eframe::WebOptions::default();
    eframe::WebRunner::new()
        .start(
            canvas,
            options,
            Box::new(move |_creation_context| {
                Ok(Box::new(labello_ui::LabelloApp::live_http(config.clone())))
            }),
        )
        .await?;
    remove_startup_status();
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn show_startup_error(error: &JsValue) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Some(status) = document.get_element_by_id("startup-status") else {
        return;
    };
    let message = error.as_string().unwrap_or_else(|| format!("{error:?}"));
    let _ = status.set_attribute("data-error", "true");
    status.set_text_content(Some(&format!(
        "Labello could not start. Reload after rebuilding the WASM application. Details: {message}"
    )));
}

#[cfg(target_arch = "wasm32")]
fn remove_startup_status() {
    if let Some(status) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("startup-status"))
    {
        status.remove();
    }
}

#[cfg(target_arch = "wasm32")]
fn app_config_from_url() -> Result<labello_ui::AppConfig, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("missing window"))?;
    let location = window.location();
    let search = location.search()?;
    let params = web_sys::UrlSearchParams::new_with_str(&search)?;
    let default_api_url = default_api_url(&location.protocol()?, &location.hostname()?);
    Ok(labello_ui::AppConfig {
        api_base_url: param(&params, "api", &default_api_url),
        application_url: Some(location.href()?),
        // Tokens must never be accepted through the URL, where browser history,
        // referrers, and copied links can expose them. Development users enter
        // the token in the masked connection field instead.
        dev_token: String::new(),
        user_id: labello_domain::UserId::from(param(&params, "user", "admin")),
        dataset_id: labello_domain::DatasetId::from(param(&params, "dataset", "demo")),
        queue_size: labello_ui::IMAGE_QUEUE_SIZE,
    })
}

#[cfg(any(target_arch = "wasm32", test))]
fn default_api_url(protocol: &str, hostname: &str) -> String {
    format!("{protocol}//{hostname}:8080")
}

#[cfg(target_arch = "wasm32")]
fn param(params: &web_sys::UrlSearchParams, name: &str, default: &str) -> String {
    params.get(name).unwrap_or_else(|| default.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn native_placeholder() {}

#[cfg(test)]
mod tests {
    use super::default_api_url;

    #[test]
    fn default_api_uses_the_application_origin_host() {
        assert_eq!(
            default_api_url("https:", "labello.example"),
            "https://labello.example:8080"
        );
    }
}
