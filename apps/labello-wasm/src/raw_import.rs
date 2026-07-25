use labello_ui::{RawImportChunkRequest, RawImportChunkResponse};
use wasm_bindgen::JsCast;

pub(crate) async fn upload_chunk(
    upload: RawImportChunkRequest,
) -> Result<RawImportChunkResponse, String> {
    if upload.length != upload.bytes.len() as u64 {
        return Err("import chunk length does not match its body".to_string());
    }
    let expected_end = upload
        .offset
        .checked_add(upload.length)
        .ok_or_else(|| "import chunk range overflowed".to_string())?;
    let url = format!(
        "{}/imports/{}/files/{}/chunks",
        upload.api_base_url.trim_end_matches('/'),
        encode_component(&upload.import_id),
        encode_component(&upload.file_id),
    );
    let init = web_sys::RequestInit::new();
    init.set_method("POST");
    init.set_credentials(web_sys::RequestCredentials::Include);
    let body = js_sys::Uint8Array::from(upload.bytes.as_slice());
    init.set_body(&body);
    let request = web_sys::Request::new_with_str_and_init(&url, &init).map_err(js_error)?;
    for (name, value) in [
        ("content-type", "application/octet-stream".to_string()),
        ("x-csrf-token", upload.csrf_token),
        ("idempotency-key", upload.idempotency_key),
        ("upload-offset", upload.offset.to_string()),
        ("upload-length", upload.length.to_string()),
        ("digest", upload.digest),
    ] {
        request.headers().set(name, &value).map_err(js_error)?;
    }
    let window = web_sys::window().ok_or_else(|| "missing browser window".to_string())?;
    let response = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(js_error)?
        .dyn_into::<web_sys::Response>()
        .map_err(|_| "import upload response was not HTTP".to_string())?;
    if !response.ok() {
        let text = wasm_bindgen_futures::JsFuture::from(response.text().map_err(js_error)?)
            .await
            .map_err(js_error)?;
        return Err(text
            .as_string()
            .unwrap_or_else(|| "import chunk upload failed".to_string()));
    }
    let value = wasm_bindgen_futures::JsFuture::from(response.json().map_err(js_error)?)
        .await
        .map_err(js_error)?;
    let accepted_offset =
        js_sys::Reflect::get(&value, &wasm_bindgen::JsValue::from_str("acceptedOffset"))
            .map_err(js_error)?
            .as_f64()
            .ok_or_else(|| "import chunk response omitted acceptedOffset".to_string())?
            as u64;
    let complete = js_sys::Reflect::get(&value, &wasm_bindgen::JsValue::from_str("complete"))
        .map_err(js_error)?
        .as_bool()
        .unwrap_or(false);
    validate_chunk_response(upload.offset, expected_end, accepted_offset, complete)?;
    Ok(RawImportChunkResponse {
        accepted_offset,
        complete,
    })
}

fn validate_chunk_response(
    offset: u64,
    expected_end: u64,
    accepted_offset: u64,
    complete: bool,
) -> Result<(), String> {
    if accepted_offset < offset || accepted_offset > expected_end {
        return Err("import chunk response returned an invalid accepted offset".to_string());
    }
    if complete && accepted_offset != expected_end {
        return Err("completed import chunk response did not accept the full body".to_string());
    }
    Ok(())
}

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

fn js_error(error: wasm_bindgen::JsValue) -> String {
    error
        .as_string()
        .unwrap_or_else(|| "browser import transport failed".to_string())
}

#[cfg(test)]
mod tests {
    use super::{encode_component, validate_chunk_response};

    #[test]
    fn raw_import_path_segments_are_encoded() {
        assert_eq!(encode_component("file/id +#"), "file%2Fid%20%2B%23");
    }

    #[test]
    fn raw_import_response_cannot_advance_outside_the_uploaded_range() {
        assert!(validate_chunk_response(10, 20, 20, true).is_ok());
        assert!(validate_chunk_response(10, 20, 9, false).is_err());
        assert!(validate_chunk_response(10, 20, 21, false).is_err());
        assert!(validate_chunk_response(10, 20, 19, true).is_err());
    }
}
