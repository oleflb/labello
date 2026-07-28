#[cfg(target_arch = "wasm32")]
fn browser_storage() -> Result<web_sys::Storage, String> {
    web_sys::window()
        .ok_or_else(|| "missing browser window".to_string())?
        .local_storage()
        .map_err(js_error)?
        .ok_or_else(|| "localStorage is unavailable in this browser context".to_string())
}

#[cfg(target_arch = "wasm32")]
fn local_set(key: &str, value: &str) -> Result<(), String> {
    browser_storage()?.set_item(key, value).map_err(|error| {
        format!(
            "could not save browser workspace preference: {}",
            js_error(error)
        )
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn local_set(_key: &str, _value: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn local_get(key: &str) -> Result<Option<String>, String> {
    browser_storage()?.get_item(key).map_err(|error| {
        format!(
            "could not load browser workspace preference: {}",
            js_error(error)
        )
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn local_get(_key: &str) -> Result<Option<String>, String> {
    Ok(None)
}
