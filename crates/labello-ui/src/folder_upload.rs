use crate::app::LabelloApp;
#[cfg(target_arch = "wasm32")]
use crate::app::{FolderUploadProgress, RequestIdentity, UiMessage};

#[cfg(target_arch = "wasm32")]
const MAX_FILES_PER_BATCH: u32 = 24;
#[cfg(target_arch = "wasm32")]
const MAX_BATCH_BYTES: f64 = 32.0 * 1024.0 * 1024.0;

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Default)]
struct OneShotCompletion {
    completed: bool,
}

#[cfg(any(target_arch = "wasm32", test))]
impl OneShotCompletion {
    fn begin(&mut self) -> bool {
        if self.completed {
            false
        } else {
            self.completed = true;
            true
        }
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn notify_and_wake<T>(message: T, send: impl FnOnce(T), wake: impl FnOnce()) {
    send(message);
    wake();
}

impl LabelloApp {
    pub(crate) fn request_folder_upload(&mut self) {
        if self.admin_mutation_blocked() {
            return;
        }
        self.loading.uploading = true;
        self.loading.upload_progress = None;
        self.admin_tools.upload_error = None;
        let root = upload_root();
        let operation_id = self.next_operation();
        let request = self.operation_identity(operation_id, self.config.dataset_id.clone());
        match open_folder_picker(self, root.clone(), request) {
            Ok(()) => {}
            Err(error) => {
                self.loading.uploading = false;
                self.loading.upload_progress = None;
                self.admin_tools.upload_error = Some(error.clone());
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
fn open_folder_picker(
    _app: &LabelloApp,
    _root: String,
    _request: crate::app::RequestIdentity,
) -> Result<(), String> {
    Err("folder picker upload is available in the browser build".to_string())
}

#[cfg(target_arch = "wasm32")]
fn open_folder_picker(
    app: &LabelloApp,
    root: String,
    request: RequestIdentity,
) -> Result<(), String> {
    use wasm_bindgen::{JsCast, closure::Closure};

    let window = web_sys::window().ok_or_else(|| "missing browser window".to_string())?;
    let document = window
        .document()
        .ok_or_else(|| "missing document".to_string())?;
    let repaint = app
        .runtime
        .repaint_ctx
        .clone()
        .ok_or_else(|| "folder picker opened before egui was ready".to_string())?;
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
        root,
        request,
        csrf_token: app
            .runtime
            .api
            .as_ref()
            .and_then(|api| api.csrf_token())
            .ok_or_else(|| "folder upload requires an authenticated session".to_string())?,
    };
    type PickerCallback = Closure<dyn FnMut(web_sys::Event)>;
    let callback_holder = std::rc::Rc::new(std::cell::RefCell::new(None::<PickerCallback>));
    let completion = std::rc::Rc::new(std::cell::RefCell::new(OneShotCompletion::default()));
    let input_for_callback = input.clone();
    let holder_for_callback = callback_holder.clone();
    let completion_for_callback = completion.clone();
    let window_for_callback = window.clone();
    let callback = Closure::<dyn FnMut(_)>::wrap(Box::new(move |event: web_sys::Event| {
        if !completion_for_callback.borrow_mut().begin() {
            return;
        }
        let files = (event.type_() == "change")
            .then(|| input_for_callback.files())
            .flatten();
        input_for_callback.set_onchange(None);
        input_for_callback
            .unchecked_ref::<web_sys::HtmlElement>()
            .set_oncancel(None);
        input_for_callback.remove();

        let tx = tx.clone();
        let repaint = repaint.clone();
        let request = config.request.clone();
        if event.type_() == "change" {
            let config = config.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let result = upload_files(files, config, tx.clone(), repaint.clone()).await;
                send_ui_message(
                    &tx,
                    &repaint,
                    UiMessage::FolderUploadFinished { request, result },
                );
            });
        } else {
            send_ui_message(
                &tx,
                &repaint,
                UiMessage::FolderUploadFinished {
                    request,
                    result: Err("folder selection cancelled".to_string()),
                },
            );
        }

        let holder = holder_for_callback.clone();
        let drop_callback = Closure::once_into_js(move || {
            holder.borrow_mut().take();
        });
        window_for_callback.queue_microtask(drop_callback.unchecked_ref());
    }));
    *callback_holder.borrow_mut() = Some(callback);
    let callback = callback_holder.borrow();
    let callback = callback.as_ref().expect("folder picker callback installed");
    let function = callback.as_ref().unchecked_ref();
    input.set_onchange(Some(function));
    input
        .unchecked_ref::<web_sys::HtmlElement>()
        .set_oncancel(Some(function));
    input.click();
    Ok(())
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone)]
struct UploadConfig {
    api_base_url: String,
    dataset_id: String,
    root: String,
    request: RequestIdentity,
    csrf_token: String,
}

#[cfg(target_arch = "wasm32")]
async fn upload_files(
    files: Option<web_sys::FileList>,
    config: UploadConfig,
    tx: std::sync::mpsc::Sender<UiMessage>,
    repaint: egui::Context,
) -> Result<String, String> {
    use wasm_bindgen::JsCast;

    let files = files.ok_or_else(|| "no folder selected".to_string())?;
    let total_files = files.length();
    if total_files == 0 {
        return Err("selected folder contains no files".to_string());
    }

    send_progress(
        &tx,
        &repaint,
        &config.request,
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
            &repaint,
            &config.request,
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
            &repaint,
            &config.request,
            total_files,
            uploaded_files,
            current_batch,
            format!("Uploaded batch {current_batch}"),
        );
        yield_to_browser().await?;
    }

    send_progress(
        &tx,
        &repaint,
        &config.request,
        total_files,
        uploaded_files,
        current_batch,
        "Running ingest".to_string(),
    );
    yield_to_browser().await?;
    run_ingest_job(
        &config,
        &tx,
        &repaint,
        total_files,
        uploaded_files,
        current_batch,
    )
    .await?;

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
    let url = upload_batch_url(&config.api_base_url, &config.dataset_id, &config.root);
    let init = request_init("POST");
    init.set_body(form);
    fetch(&url, &init, Some(&config.csrf_token))
        .await
        .map(|_| ())
}

#[cfg(target_arch = "wasm32")]
async fn start_ingest_job(config: &UploadConfig) -> Result<String, String> {
    let url = format!(
        "{}/datasets/{}/ingest-jobs",
        config.api_base_url.trim_end_matches('/'),
        encode_component(&config.dataset_id),
    );
    let init = request_init("POST");
    let response = fetch(&url, &init, Some(&config.csrf_token)).await?;
    let value = wasm_bindgen_futures::JsFuture::from(response.json().map_err(js_error)?)
        .await
        .map_err(js_error)?;
    js_string_property(&value, "jobId")
        .ok_or_else(|| "ingest job response missing jobId".to_string())
}

#[cfg(target_arch = "wasm32")]
async fn run_ingest_job(
    config: &UploadConfig,
    tx: &std::sync::mpsc::Sender<UiMessage>,
    repaint: &egui::Context,
    total_files: u32,
    uploaded_files: u32,
    current_batch: u32,
) -> Result<(), String> {
    let job_id = start_ingest_job(config).await?;
    loop {
        browser_sleep(500).await?;
        send_progress(
            tx,
            repaint,
            &config.request,
            total_files,
            uploaded_files,
            current_batch,
            "Running ingest".to_string(),
        );
        match poll_ingest_job(config, &job_id).await? {
            IngestPoll::Running => {}
            IngestPoll::Completed => return Ok(()),
            IngestPoll::Failed(error) => return Err(error),
        }
    }
}

#[cfg(target_arch = "wasm32")]
enum IngestPoll {
    Running,
    Completed,
    Failed(String),
}

#[cfg(target_arch = "wasm32")]
async fn poll_ingest_job(config: &UploadConfig, job_id: &str) -> Result<IngestPoll, String> {
    let url = format!(
        "{}/datasets/{}/ingest-jobs/{}",
        config.api_base_url.trim_end_matches('/'),
        encode_component(&config.dataset_id),
        encode_component(job_id),
    );
    let init = request_init("GET");
    let response = fetch(&url, &init, None).await?;
    let value = wasm_bindgen_futures::JsFuture::from(response.json().map_err(js_error)?)
        .await
        .map_err(js_error)?;
    match js_string_property(&value, "status").as_deref() {
        Some("completed") => Ok(IngestPoll::Completed),
        Some("failed") => Ok(IngestPoll::Failed(
            js_string_property(&value, "error").unwrap_or_else(|| "ingest failed".to_string()),
        )),
        _ => Ok(IngestPoll::Running),
    }
}

#[cfg(target_arch = "wasm32")]
fn request_init(method: &str) -> web_sys::RequestInit {
    let init = web_sys::RequestInit::new();
    init.set_method(method);
    init
}

#[cfg(target_arch = "wasm32")]
async fn fetch(
    url: &str,
    init: &web_sys::RequestInit,
    csrf_token: Option<&str>,
) -> Result<web_sys::Response, String> {
    use wasm_bindgen::JsCast;

    init.set_credentials(web_sys::RequestCredentials::Include);
    let request = web_sys::Request::new_with_str_and_init(url, init).map_err(js_error)?;
    if let Some(token) = csrf_token {
        request
            .headers()
            .set("x-csrf-token", token)
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
    repaint: &egui::Context,
    request: &RequestIdentity,
    total_files: u32,
    uploaded_files: u32,
    current_batch: u32,
    message: String,
) {
    send_ui_message(
        tx,
        repaint,
        UiMessage::FolderUploadProgress {
            request: request.clone(),
            progress: FolderUploadProgress {
                uploaded_files,
                total_files,
                current_batch,
                message,
            },
        },
    );
}

#[cfg(target_arch = "wasm32")]
fn send_ui_message(
    tx: &std::sync::mpsc::Sender<UiMessage>,
    repaint: &egui::Context,
    message: UiMessage,
) {
    notify_and_wake(
        message,
        |message| {
            let _ = tx.send(message);
        },
        || repaint.request_repaint(),
    );
}

#[cfg(target_arch = "wasm32")]
async fn yield_to_browser() -> Result<(), String> {
    browser_animation_frame().await
}

#[cfg(target_arch = "wasm32")]
async fn browser_animation_frame() -> Result<(), String> {
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
async fn browser_sleep(milliseconds: i32) -> Result<(), String> {
    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        let Some(window) = web_sys::window() else {
            let _ = reject.call1(
                &wasm_bindgen::JsValue::NULL,
                &wasm_bindgen::JsValue::from_str("missing browser window"),
            );
            return;
        };
        if let Err(error) =
            window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, milliseconds)
        {
            let _ = reject.call1(&wasm_bindgen::JsValue::NULL, &error);
        }
    });
    wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map(|_| ())
        .map_err(js_error)
}

#[cfg(target_arch = "wasm32")]
fn js_string_property(value: &wasm_bindgen::JsValue, name: &str) -> Option<String> {
    js_sys::Reflect::get(value, &wasm_bindgen::JsValue::from_str(name))
        .ok()
        .and_then(|value| value.as_string())
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

#[cfg(any(target_arch = "wasm32", test))]
fn upload_batch_url(api_base_url: &str, dataset_id: &str, root: &str) -> String {
    format!(
        "{}/datasets/{}/uploads?root={}&ingest=false",
        api_base_url.trim_end_matches('/'),
        encode_component(dataset_id),
        encode_component(root),
    )
}

#[cfg(any(target_arch = "wasm32", test))]
fn encode_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
            )
        {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(target_arch = "wasm32")]
fn js_error(error: wasm_bindgen::JsValue) -> String {
    error
        .as_string()
        .unwrap_or_else(|| "browser API error".to_string())
}

#[cfg(test)]
mod tests {
    use super::{OneShotCompletion, notify_and_wake, upload_batch_url};

    #[test]
    fn upload_request_encodes_dataset_and_root_without_changing_the_api_base() {
        assert_eq!(
            upload_batch_url(
                "https://example.test/api/",
                "data/set",
                "uploads/My folder+#1"
            ),
            "https://example.test/api/datasets/data%2Fset/uploads?root=uploads%2FMy%20folder%2B%231&ingest=false"
        );
    }

    #[test]
    fn picker_cleanup_is_one_shot() {
        let mut completion = OneShotCompletion::default();
        assert!(completion.begin());
        assert!(!completion.begin());
    }

    #[test]
    fn message_delivery_wakes_the_ui_even_when_the_receiver_is_gone() {
        let mut woke = false;
        notify_and_wake((), |_| {}, || woke = true);
        assert!(woke);
    }
}
