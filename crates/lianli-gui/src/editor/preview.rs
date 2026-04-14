use super::SharedEditor;
use crate::ipc_client;
use crate::{Shared, TemplateEditorWindow};
use lianli_shared::ipc::IpcRequest;
use slint::Image;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Spawns a preview render in the background. The version counter discards
/// late responses if the user edits faster than the daemon can render.
pub(super) fn request_preview(
    weak: &slint::Weak<TemplateEditorWindow>,
    state: &SharedEditor,
    version: &Arc<AtomicU64>,
    _shared: &Shared,
) {
    let tpl = {
        let st = state.lock();
        match &st.template {
            Some(t) => t.clone(),
            None => return,
        }
    };
    let my_version = version.fetch_add(1, Ordering::SeqCst) + 1;
    let version = version.clone();
    let weak = weak.clone();

    std::thread::spawn(move || {
        let req = IpcRequest::RenderTemplatePreview {
            template: tpl.clone(),
            width: tpl.base_width,
            height: tpl.base_height,
        };
        let resp = match ipc_client::send_request(&req) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("preview IPC failed: {e}");
                return;
            }
        };
        if version.load(Ordering::SeqCst) != my_version {
            return;
        }
        let bytes = match decode_preview(&resp) {
            Some(b) => b,
            None => return,
        };
        // Per-version filename so Slint doesn't serve a cached image.
        let tmp = std::env::temp_dir().join(format!(
            "lianli-preview-{}-{}.jpg",
            std::process::id(),
            my_version
        ));
        if let Err(e) = std::fs::write(&tmp, &bytes) {
            tracing::warn!("preview write failed: {e}");
            return;
        }
        let tmp_clone = tmp.clone();
        slint::invoke_from_event_loop(move || {
            if let Some(e) = weak.upgrade() {
                if let Ok(img) = Image::load_from_path(&tmp_clone) {
                    e.set_preview_image(img);
                }
            }
        })
        .ok();
    });
}

pub(super) fn decode_preview(resp: &lianli_shared::ipc::IpcResponse) -> Option<Vec<u8>> {
    use base64::Engine;
    match resp {
        lianli_shared::ipc::IpcResponse::Ok { data } => {
            let b64 = data.get("jpeg_base64")?.as_str()?;
            base64::engine::general_purpose::STANDARD.decode(b64).ok()
        }
        lianli_shared::ipc::IpcResponse::Error { message } => {
            tracing::warn!("preview error: {message}");
            None
        }
    }
}
