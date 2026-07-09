use crate::app::LabelloApp;
#[cfg(target_arch = "wasm32")]
use crate::app::UiMessage;

impl LabelloApp {
    pub(crate) fn request_folder_upload(&mut self) {
        if self.loading.uploading {
            return;
        }
        self.loading.uploading = true;
        let root = upload_root();
        match open_folder_picker(self, root.clone()) {
            Ok(()) => {}
            Err(error) => {
                self.loading.uploading = false;
                self.runtime.error = Some(error);
            }
        }
    }
}

fn upload_root() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        format!("uploads/batch-{}", js_sys::Date::now() as u64)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        "uploads/batch".to_string()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn open_folder_picker(_app: &LabelloApp, _root: String) -> Result<(), String> {
    Err("folder picker upload is available in the browser build".to_string())
}

#[cfg(target_arch = "wasm32")]
fn open_folder_picker(app: &LabelloApp, root: String) -> Result<(), String> {
    use wasm_bindgen::{JsCast, closure::Closure};

    let window = web_sys::window().ok_or_else(|| "missing browser window".to_string())?;
    let document = window
        .document()
        .ok_or_else(|| "missing document".to_string())?;
    let input = document
        .create_element("input")
        .map_err(js_error)?
        .dyn_into::<web_sys::HtmlInputElement>()
        .map_err(|_| "failed to create file input".to_string())?;
    input.set_type("file");
    input.set_multiple(true);
    input
        .set_attribute("webkitdirectory", "")
        .map_err(js_error)?;
    input.set_attribute("directory", "").map_err(js_error)?;
    input
        .set_attribute("style", "display:none")
        .map_err(js_error)?;
    document
        .body()
        .ok_or_else(|| "missing document body".to_string())?
        .append_child(&input)
        .map_err(js_error)?;

    let tx = app.runtime.tx.clone();
    let config = UploadConfig {
        api_base_url: app.config.api_base_url.clone(),
        dataset_id: app.config.dataset_id.to_string(),
        user_id: app.config.user_id.to_string(),
        role: app.config.role.to_string(),
        dev_token: app.config.dev_token.clone(),
        root,
    };
    let input_for_callback = input.clone();
    let callback = Closure::<dyn FnMut(_)>::wrap(Box::new(move |_event: web_sys::Event| {
        let files = input_for_callback.files();
        input_for_callback.remove();
        let tx = tx.clone();
        let config = config.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let result = upload_files(files, config).await;
            let _ = tx.send(UiMessage::FolderUploadFinished(result));
        });
    }));
    input.set_onchange(Some(callback.as_ref().unchecked_ref()));
    input.click();
    callback.forget();
    Ok(())
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone)]
struct UploadConfig {
    api_base_url: String,
    dataset_id: String,
    user_id: String,
    role: String,
    dev_token: String,
    root: String,
}

#[cfg(target_arch = "wasm32")]
async fn upload_files(
    files: Option<web_sys::FileList>,
    config: UploadConfig,
) -> Result<String, String> {
    use wasm_bindgen::JsCast;

    let files = files.ok_or_else(|| "no folder selected".to_string())?;
    if files.length() == 0 {
        return Err("selected folder contains no files".to_string());
    }
    let form = web_sys::FormData::new().map_err(js_error)?;
    for index in 0..files.length() {
        if index > 0 && index % 100 == 0 {
            yield_to_browser().await;
        }
        let Some(file) = files.item(index) else {
            continue;
        };
        let path = relative_file_path(&file);
        let blob = file.unchecked_ref::<web_sys::Blob>();
        form.append_with_blob_and_filename("files", blob, &path)
            .map_err(js_error)?;
    }
    let url = format!(
        "{}/datasets/{}/uploads?root={}&ingest=true",
        config.api_base_url.trim_end_matches('/'),
        encode_component(&config.dataset_id),
        encode_component(&config.root),
    );
    let init = web_sys::RequestInit::new();
    init.set_method("POST");
    init.set_body(&form);
    let request = web_sys::Request::new_with_str_and_init(&url, &init).map_err(js_error)?;
    request
        .headers()
        .set("x-user-id", &config.user_id)
        .map_err(js_error)?;
    request
        .headers()
        .set("x-user-role", &config.role)
        .map_err(js_error)?;
    if !config.dev_token.is_empty() {
        request
            .headers()
            .set("x-dev-token", &config.dev_token)
            .map_err(js_error)?;
    }
    let window = web_sys::window().ok_or_else(|| "missing browser window".to_string())?;
    let response = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(js_error)?
        .dyn_into::<web_sys::Response>()
        .map_err(|_| "upload response was not an HTTP response".to_string())?;
    if response.ok() {
        Ok(format!(
            "Uploaded {} files into {} and started ingest",
            files.length(),
            config.root
        ))
    } else {
        let text = wasm_bindgen_futures::JsFuture::from(response.text().map_err(js_error)?)
            .await
            .map_err(js_error)?;
        Err(text
            .as_string()
            .unwrap_or_else(|| "folder upload failed".to_string()))
    }
}

#[cfg(target_arch = "wasm32")]
async fn yield_to_browser() {
    let _ = wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(
        &wasm_bindgen::JsValue::NULL,
    ))
    .await;
}

#[cfg(target_arch = "wasm32")]
fn relative_file_path(file: &web_sys::File) -> String {
    let path = js_sys::Reflect::get(
        file.as_ref(),
        &wasm_bindgen::JsValue::from_str("webkitRelativePath"),
    )
    .ok()
    .and_then(|value| value.as_string())
    .filter(|value| !value.is_empty());
    path.unwrap_or_else(|| file.name())
}

#[cfg(target_arch = "wasm32")]
fn encode_component(value: &str) -> String {
    js_sys::encode_uri_component(value)
        .as_string()
        .unwrap_or_else(|| value.to_string())
}

#[cfg(target_arch = "wasm32")]
fn js_error(error: wasm_bindgen::JsValue) -> String {
    error
        .as_string()
        .unwrap_or_else(|| "browser API error".to_string())
}
