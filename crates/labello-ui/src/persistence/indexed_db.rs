#[cfg(target_arch = "wasm32")]
struct IndexedDbDraftStore;

#[cfg(target_arch = "wasm32")]
impl DraftStore for IndexedDbDraftStore {
    fn get<'a>(&'a self, key: &'a str) -> StoreFuture<'a, Option<DraftRecord>> {
        Box::pin(async move {
            let database = open_database().await?;
            let transaction = database
                .transaction_with_str_and_mode(DRAFT_STORE, web_sys::IdbTransactionMode::Readonly)
                .map_err(js_error)?;
            let transaction_done = watch_transaction(&transaction);
            let request = transaction
                .object_store(DRAFT_STORE)
                .map_err(js_error)?
                .get(&wasm_bindgen::JsValue::from_str(key))
                .map_err(js_error)?;
            let value = await_request(request).await?;
            transaction_done.await.map_err(js_error)?;
            if value.is_undefined() {
                return Ok(None);
            }
            let encoded = value
                .as_string()
                .ok_or_else(|| "IndexedDB draft is not encoded text".to_string())?;
            serde_json::from_str(&encoded)
                .map(Some)
                .map_err(|error| format!("IndexedDB draft is corrupt: {error}"))
        })
    }

    fn put<'a>(&'a self, record: DraftRecord) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let encoded = record.validate_size()?;
            let key = record.key().to_string();
            let database = open_database().await?;
            let transaction = database
                .transaction_with_str_and_mode(DRAFT_STORE, web_sys::IdbTransactionMode::Readwrite)
                .map_err(js_error)?;
            let transaction_done = watch_transaction(&transaction);
            let request = transaction
                .object_store(DRAFT_STORE)
                .map_err(js_error)?
                .put_with_key(
                    &wasm_bindgen::JsValue::from_str(&encoded),
                    &wasm_bindgen::JsValue::from_str(&key),
                )
                .map_err(js_error)?;
            await_request(request).await?;
            transaction_done.await.map(|_| ()).map_err(js_error)
        })
    }

    fn delete<'a>(&'a self, key: &'a str) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let database = open_database().await?;
            let transaction = database
                .transaction_with_str_and_mode(DRAFT_STORE, web_sys::IdbTransactionMode::Readwrite)
                .map_err(js_error)?;
            let transaction_done = watch_transaction(&transaction);
            let request = transaction
                .object_store(DRAFT_STORE)
                .map_err(js_error)?
                .delete(&wasm_bindgen::JsValue::from_str(key))
                .map_err(js_error)?;
            await_request(request).await?;
            transaction_done.await.map(|_| ()).map_err(js_error)
        })
    }

    fn garbage_collect<'a>(&'a self, now: Timestamp) -> StoreFuture<'a, usize> {
        Box::pin(async move {
            let database = open_database().await?;
            let read = database
                .transaction_with_str_and_mode(DRAFT_STORE, web_sys::IdbTransactionMode::Readonly)
                .map_err(js_error)?;
            let read_done = watch_transaction(&read);
            let values = await_request(
                read.object_store(DRAFT_STORE)
                    .map_err(js_error)?
                    .get_all()
                    .map_err(js_error)?,
            )
            .await?;
            read_done.await.map_err(js_error)?;
            let cutoff = now - chrono::Duration::seconds(DRAFT_TTL_SECONDS);
            let values = js_sys::Array::from(&values);
            let mut keys = Vec::new();
            for value in values.iter() {
                let Some(encoded) = value.as_string() else {
                    continue;
                };
                if let Ok(record) = serde_json::from_str::<DraftRecord>(&encoded)
                    && record.updated_at() < cutoff
                {
                    keys.push(record.key().to_string());
                }
            }
            for key in &keys {
                let transaction = database
                    .transaction_with_str_and_mode(
                        DRAFT_STORE,
                        web_sys::IdbTransactionMode::Readwrite,
                    )
                    .map_err(js_error)?;
                let transaction_done = watch_transaction(&transaction);
                await_request(
                    transaction
                        .object_store(DRAFT_STORE)
                        .map_err(js_error)?
                        .delete(&wasm_bindgen::JsValue::from_str(key))
                        .map_err(js_error)?,
                )
                .await?;
                transaction_done.await.map_err(js_error)?;
            }
            Ok(keys.len())
        })
    }
}

#[cfg(target_arch = "wasm32")]
async fn open_database() -> Result<web_sys::IdbDatabase, String> {
    use wasm_bindgen::JsCast;

    let factory = web_sys::window()
        .ok_or_else(|| "missing browser window".to_string())?
        .indexed_db()
        .map_err(js_error)?
        .ok_or_else(|| "IndexedDB is unavailable in this browser context".to_string())?;
    let request = factory.open_with_u32(DATABASE_NAME, 1).map_err(js_error)?;
    let upgrade_request = request.clone();
    let upgrade =
        wasm_bindgen::closure::Closure::<dyn FnMut(_)>::new(move |_event: web_sys::Event| {
            if let Ok(database) = upgrade_request.result()
                && let Ok(database) = database.dyn_into::<web_sys::IdbDatabase>()
                && !database.object_store_names().contains(DRAFT_STORE)
            {
                let _ = database.create_object_store(DRAFT_STORE);
            }
        });
    request.set_onupgradeneeded(Some(upgrade.as_ref().unchecked_ref()));
    let value = await_request(request.clone().unchecked_into()).await;
    request.set_onupgradeneeded(None);
    drop(upgrade);
    value?
        .dyn_into::<web_sys::IdbDatabase>()
        .map_err(|_| "IndexedDB open returned an invalid database".to_string())
}

#[cfg(target_arch = "wasm32")]
async fn await_request(request: web_sys::IdbRequest) -> Result<wasm_bindgen::JsValue, String> {
    use wasm_bindgen::JsCast;

    let watched = request.clone();
    let promise = js_sys::Promise::new(&mut move |resolve, reject| {
        let request = watched.clone();
        let resolve = resolve.clone();
        let reject = reject.clone();
        let callback =
            wasm_bindgen::closure::Closure::once_into_js(move |_event: web_sys::Event| {
                request.set_onsuccess(None);
                request.set_onerror(None);
                match request.error() {
                    Ok(Some(error)) => {
                        let _ = reject.call1(
                            &wasm_bindgen::JsValue::UNDEFINED,
                            &wasm_bindgen::JsValue::from_str(&error.message()),
                        );
                    }
                    _ => match request.result() {
                        Ok(value) => {
                            let _ = resolve.call1(&wasm_bindgen::JsValue::UNDEFINED, &value);
                        }
                        Err(error) => {
                            let _ = reject.call1(&wasm_bindgen::JsValue::UNDEFINED, &error);
                        }
                    },
                }
            });
        let function = callback.unchecked_ref::<js_sys::Function>();
        watched.set_onsuccess(Some(function));
        watched.set_onerror(Some(function));
    });
    wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(js_error)
}

#[cfg(target_arch = "wasm32")]
fn watch_transaction(transaction: &web_sys::IdbTransaction) -> wasm_bindgen_futures::JsFuture {
    use wasm_bindgen::JsCast;

    let watched = transaction.clone();
    let promise = js_sys::Promise::new(&mut move |resolve, reject| {
        let transaction = watched.clone();
        let resolve = resolve.clone();
        let reject = reject.clone();
        let callback =
            wasm_bindgen::closure::Closure::once_into_js(move |event: web_sys::Event| {
                transaction.set_oncomplete(None);
                transaction.set_onabort(None);
                transaction.set_onerror(None);
                if event.type_() == "complete" {
                    let _ = resolve.call0(&wasm_bindgen::JsValue::UNDEFINED);
                    return;
                }
                let error = transaction
                    .error()
                    .map(|error| error.message())
                    .unwrap_or_else(|| format!("IndexedDB transaction {}", event.type_()));
                let _ = reject.call1(
                    &wasm_bindgen::JsValue::UNDEFINED,
                    &wasm_bindgen::JsValue::from_str(&error),
                );
            });
        let function = callback.unchecked_ref::<js_sys::Function>();
        watched.set_oncomplete(Some(function));
        watched.set_onabort(Some(function));
        watched.set_onerror(Some(function));
    });
    wasm_bindgen_futures::JsFuture::from(promise)
}

#[cfg(target_arch = "wasm32")]
fn js_error(error: wasm_bindgen::JsValue) -> String {
    error
        .as_string()
        .unwrap_or_else(|| format!("browser storage error: {error:?}"))
}
