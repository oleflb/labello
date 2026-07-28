impl LabelloApp {
    pub(crate) fn spawn_message<F>(&self, _request: RequestIdentity, future: F)
    where
        F: Future<Output = UiMessage> + 'static,
    {
        let tx = self.runtime.tx.clone();
        let repaint_ctx = self.runtime.repaint_ctx.clone();
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(async move {
            let _ = tx.send(future.await);
            if let Some(ctx) = repaint_ctx {
                ctx.request_repaint();
            }
        });
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(spawn) = &self.runtime.native_task_spawner {
                spawn(Box::pin(async move {
                    let _ = tx.send(future.await);
                    if let Some(ctx) = repaint_ctx {
                        ctx.request_repaint();
                    }
                }));
                return;
            }
        }
        #[cfg(all(not(target_arch = "wasm32"), test))]
        {
            let _ = tx.send(poll_ready(future));
            if let Some(ctx) = repaint_ctx {
                ctx.request_repaint();
            }
        }
        #[cfg(all(not(target_arch = "wasm32"), not(test)))]
        {
            drop(future);
            let _ = tx.send(UiMessage::RequestFailed {
                request: _request,
                error: "live HTTP UI is available in the WASM build".to_string(),
            });
            if let Some(ctx) = repaint_ctx {
                ctx.request_repaint();
            }
        }
    }

    pub(crate) fn spawn_import_message<F>(&self, future: F)
    where
        F: Future<Output = UiMessage> + 'static,
    {
        let tx = self.runtime.tx.clone();
        let repaint_ctx = self.runtime.repaint_ctx.clone();
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(async move {
            let _ = tx.send(future.await);
            if let Some(ctx) = repaint_ctx {
                ctx.request_repaint();
            }
        });
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(spawn) = &self.runtime.native_task_spawner {
                spawn(Box::pin(async move {
                    let _ = tx.send(future.await);
                    if let Some(ctx) = repaint_ctx {
                        ctx.request_repaint();
                    }
                }));
                return;
            }
        }
        #[cfg(all(not(target_arch = "wasm32"), test))]
        {
            let _ = tx.send(poll_ready(future));
            if let Some(ctx) = repaint_ctx {
                ctx.request_repaint();
            }
        }
        #[cfg(all(not(target_arch = "wasm32"), not(test)))]
        drop(future);
    }
}
