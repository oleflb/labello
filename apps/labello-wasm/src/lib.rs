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
    let options = eframe::WebOptions::default();
    eframe::WebRunner::new()
        .start(
            canvas,
            options,
            Box::new(|_creation_context| Ok(Box::new(labello_ui::LabelloApp::default()))),
        )
        .await
}

#[cfg(not(target_arch = "wasm32"))]
pub fn native_placeholder() {}
