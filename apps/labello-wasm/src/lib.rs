#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub async fn start() -> Result<(), JsValue> {
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
        .await
}

#[cfg(target_arch = "wasm32")]
fn app_config_from_url() -> Result<labello_ui::AppConfig, JsValue> {
    let search = web_sys::window()
        .ok_or_else(|| JsValue::from_str("missing window"))?
        .location()
        .search()?;
    let params = web_sys::UrlSearchParams::new_with_str(&search)?;
    Ok(labello_ui::AppConfig {
        api_base_url: param(&params, "api", "http://127.0.0.1:8080"),
        // Tokens must never be accepted through the URL, where browser history,
        // referrers, and copied links can expose them. Development users enter
        // the token in the masked connection field instead.
        dev_token: String::new(),
        user_id: labello_domain::UserId::from(param(&params, "user", "admin")),
        dataset_id: labello_domain::DatasetId::from(param(&params, "dataset", "demo")),
        queue_size: labello_ui::IMAGE_QUEUE_SIZE,
    })
}

#[cfg(target_arch = "wasm32")]
fn param(params: &web_sys::UrlSearchParams, name: &str, default: &str) -> String {
    params.get(name).unwrap_or_else(|| default.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn native_placeholder() {}
