use crate::app::LabelloApp;
#[cfg(target_arch = "wasm32")]
use crate::app::{FolderUploadProgress, UiMessage};

#[cfg(target_arch = "wasm32")]
const MAX_FILES_PER_BATCH: u32 = 24;
#[cfg(target_arch = "wasm32")]
const MAX_BATCH_BYTES: f64 = 32.0 * 1024.0 * 1024.0;

impl LabelloApp {
    pub(crate) fn request_folder_upload(&mut self) {
        if self.loading.uploading {
            return;
        }
        let root = upload_root();
        match open_folder_picker(self, root.clone()) {
            Ok(()) => {}
            Err(error) => {
                self.loading.uploading = false;
                self.loading.upload_progress = None;
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
            let result = upload_files(files, config, tx.clone()).await;
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
    tx: std::sync::mpsc::Sender<UiMessage>,
) -> Result<String, String> {
    use wasm_bindgen::JsCast;

    let files = files.ok_or_else(|| "no folder selected".to_string())?;
    let total_files = files.length();
    if total_files == 0 {
        return Err("selected folder contains no files".to_string());
    }

    send_progress(
        &tx,
        total_files,
        0,
        0,
        "Preparing folder upload".to_string(),
    );
    yield_to_browser().await?;

    let mut next_index = 0;
    let mut uploaded_files = 0;
    let mut current_batch = 0;
    while next_index < total_files {
        current_batch += 1;
        let mut batch = UploadBatch::new()?;
        while next_index < total_files && batch.file_count < MAX_FILES_PER_BATCH {
            let Some(file) = files.item(next_index) else {
                next_index += 1;
                continue;
            };
            let size = file.unchecked_ref::<web_sys::Blob>().size();
            if batch.file_count > 0 && batch.byte_size + size > MAX_BATCH_BYTES {
                break;
            }
            batch.append(file)?;
            next_index += 1;
        }
        if batch.file_count == 0 {
            return Err("selected folder contained no readable files".to_string());
        }

        send_progress(
            &tx,
            total_files,
            uploaded_files,
            current_batch,
            format!("Uploading batch {current_batch}"),
        );
        yield_to_browser().await?;

        upload_batch(&config, &batch.form).await?;
        uploaded_files += batch.file_count;
        send_progress(
            &tx,
            total_files,
            uploaded_files,
            current_batch,
            format!("Uploaded batch {current_batch}"),
        );
        yield_to_browser().await?;
    }

    send_progress(
        &tx,
        total_files,
        uploaded_files,
        current_batch,
        "Running ingest".to_string(),
    );
    yield_to_browser().await?;
    run_ingest(&config).await?;

    Ok(format!(
        "Uploaded {total_files} files into {} and completed ingest",
        config.root
    ))
}

#[cfg(target_arch = "wasm32")]
struct UploadBatch {
    form: web_sys::FormData,
    file_count: u32,
    byte_size: f64,
}

#[cfg(target_arch = "wasm32")]
impl UploadBatch {
    fn new() -> Result<Self, String> {
        Ok(Self {
            form: web_sys::FormData::new().map_err(js_error)?,
            file_count: 0,
            byte_size: 0.0,
        })
    }

    fn append(&mut self, file: web_sys::File) -> Result<(), String> {
        use wasm_bindgen::JsCast;

        let path = relative_file_path(&file);
        let blob = file.unchecked_ref::<web_sys::Blob>();
        self.byte_size += blob.size();
        self.form
            .append_with_blob_and_filename("files", blob, &path)
            .map_err(js_error)?;
        self.file_count += 1;
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
async fn upload_batch(config: &UploadConfig, form: &web_sys::FormData) -> Result<(), String> {
    let url = format!(
        "{}/datasets/{}/uploads?root={}&ingest=false",
        config.api_base_url.trim_end_matches('/'),
        encode_component(&config.dataset_id),
        encode_component(&config.root),
    );
    let init = request_init("POST");
    init.set_body(form);
    fetch(config, &url, &init).await.map(|_| ())
}

#[cfg(target_arch = "wasm32")]
async fn run_ingest(config: &UploadConfig) -> Result<(), String> {
    let url = format!(
        "{}/datasets/{}/ingest",
        config.api_base_url.trim_end_matches('/'),
        encode_component(&config.dataset_id),
    );
    let init = request_init("POST");
    fetch(config, &url, &init).await.map(|_| ())
}

#[cfg(target_arch = "wasm32")]
fn request_init(method: &str) -> web_sys::RequestInit {
    let init = web_sys::RequestInit::new();
    init.set_method(method);
    init
}

#[cfg(target_arch = "wasm32")]
async fn fetch(
    config: &UploadConfig,
    url: &str,
    init: &web_sys::RequestInit,
) -> Result<web_sys::Response, String> {
    use wasm_bindgen::JsCast;

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
        Ok(response)
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
fn send_progress(
    tx: &std::sync::mpsc::Sender<UiMessage>,
    total_files: u32,
    uploaded_files: u32,
    current_batch: u32,
    message: String,
) {
    let _ = tx.send(UiMessage::FolderUploadProgress(FolderUploadProgress {
        uploaded_files,
        total_files,
        current_batch,
        message,
    }));
}

#[cfg(target_arch = "wasm32")]
async fn yield_to_browser() -> Result<(), String> {
    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        let Some(window) = web_sys::window() else {
            let _ = reject.call1(
                &wasm_bindgen::JsValue::NULL,
                &wasm_bindgen::JsValue::from_str("missing browser window"),
            );
            return;
        };
        if let Err(error) = window.request_animation_frame(&resolve) {
            let _ = reject.call1(&wasm_bindgen::JsValue::NULL, &error);
        }
    });
    wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map(|_| ())
        .map_err(js_error)
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
