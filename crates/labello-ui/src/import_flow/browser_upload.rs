pub(crate) fn import_key(action: &str, request_id: u64) -> String {
    format!("ui-{action}-{request_id}")
}

#[cfg(target_arch = "wasm32")]
pub(crate) mod browser {
    use super::*;

    use wasm_bindgen::{JsCast, closure::Closure};

    pub(crate) fn pick_import_folder(
        app: &LabelloApp,
        request: crate::app::ImportRequestIdentity,
        limits: labello_client::ImportLimits,
    ) -> Result<(), String> {
        let window = web_sys::window().ok_or_else(|| "missing browser window".to_string())?;
        let document = window
            .document()
            .ok_or_else(|| "missing browser document".to_string())?;
        let input = document
            .create_element("input")
            .map_err(js_error)?
            .dyn_into::<web_sys::HtmlInputElement>()
            .map_err(|_| "failed to create import folder input".to_string())?;
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
            .ok_or_else(|| "missing browser document body".to_string())?
            .append_child(&input)
            .map_err(js_error)?;
        let tx = app.runtime.tx.clone();
        let repaint = app
            .runtime
            .repaint_ctx
            .clone()
            .ok_or_else(|| "folder picker opened before egui was ready".to_string())?;
        let input_for_callback = input.clone();
        let finished = std::rc::Rc::new(std::cell::Cell::new(false));
        let finished_for_callback = finished.clone();
        let callback = Closure::<dyn FnMut(_)>::new(move |event: web_sys::Event| {
            if finished_for_callback.replace(true) {
                return;
            }
            let files = (event.type_() == "change")
                .then(|| input_for_callback.files())
                .flatten();
            input_for_callback.remove();
            let tx = tx.clone();
            let repaint = repaint.clone();
            let request = request.clone();
            let limits = limits.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let result = collect_files(files, &limits).await;
                let _ =
                    tx.send(crate::app::UiMessage::ImportBrowserFilesSelected { request, result });
                repaint.request_repaint();
            });
        });
        let function = callback.as_ref().unchecked_ref();
        input.set_onchange(Some(function));
        input
            .unchecked_ref::<web_sys::HtmlElement>()
            .set_oncancel(Some(function));
        callback.forget();
        input.click();
        Ok(())
    }

    async fn collect_files(
        files: Option<web_sys::FileList>,
        limits: &labello_client::ImportLimits,
    ) -> Result<Vec<BrowserImportFile>, String> {
        let files = files.ok_or_else(|| "folder selection cancelled".to_string())?;
        if files.length() == 0 {
            return Err("selected folder contains no files".to_string());
        }
        let mut pending = Vec::with_capacity(files.length() as usize);
        let mut total_bytes = 0_u64;
        for index in 0..files.length() {
            let file = files
                .item(index)
                .ok_or_else(|| "selected folder contains an unreadable file".to_string())?;
            let byte_size = file.unchecked_ref::<web_sys::Blob>().size() as u64;
            let relative_path = relative_file_path(&file);
            total_bytes = total_bytes.saturating_add(byte_size);
            pending.push((index, file, relative_path, byte_size));
        }
        validate_browser_selection_limits(
            pending.len(),
            total_bytes,
            pending.iter().map(|(_, _, _, size)| *size),
            limits,
        )?;
        let mut selected = Vec::with_capacity(pending.len());
        for (index, file, relative_path, byte_size) in pending {
            let blake3 = hash_file(&file, byte_size).await?;
            selected.push(BrowserImportFile {
                client_file_id: format!("browser-{index}"),
                relative_path,
                byte_size,
                blake3,
                file,
            });
        }
        Ok(selected)
    }

    async fn hash_file(file: &web_sys::File, size: u64) -> Result<String, String> {
        const HASH_CHUNK: u64 = 8 * 1024 * 1024;
        let mut hasher = blake3::Hasher::new();
        let mut offset = 0;
        while offset < size {
            let length = (size - offset).min(HASH_CHUNK);
            hasher.update(&read_file_range(file, offset, length).await?);
            offset += length;
        }
        Ok(hasher.finalize().to_hex().to_string())
    }

    pub(crate) async fn read_file_range(
        file: &web_sys::File,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, String> {
        let blob = file
            .unchecked_ref::<web_sys::Blob>()
            .slice_with_f64_and_f64(offset as f64, (offset + length) as f64)
            .map_err(js_error)?;
        let buffer = wasm_bindgen_futures::JsFuture::from(blob.array_buffer())
            .await
            .map_err(js_error)?;
        Ok(js_sys::Uint8Array::new(&buffer).to_vec())
    }

    fn relative_file_path(file: &web_sys::File) -> String {
        js_sys::Reflect::get(
            file.as_ref(),
            &wasm_bindgen::JsValue::from_str("webkitRelativePath"),
        )
        .ok()
        .and_then(|path| path.as_string())
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| file.name())
    }

    fn js_error(error: wasm_bindgen::JsValue) -> String {
        error
            .as_string()
            .unwrap_or_else(|| "browser import operation failed".to_string())
    }
}
