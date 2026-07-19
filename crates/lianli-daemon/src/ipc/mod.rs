//! Per-concern IPC request handlers, split out from `ipc_server::handle_request`.
//!
//! Each submodule owns the handlers for one area of functionality:
//!
//! - [`system`] — read-only queries: ping, sensor/device enumeration, telemetry.
//! - [`config`] — config-writing handlers (`SetConfig`, `SetLcdMedia`, etc.).
//! - [`fan`] — fan-specific handlers (`SetEne6k77FanQuantity`, fan direction).
//! - [`rgb`] — RGB effect / direct-color / zone queries.
//! - [`lcd`] — display-mode switching, template preview rendering.
//! - [`wireless`] — bind / unbind RF devices.
//! - [`templates`] — LCD template CRUD.
//! - [`presets`] — RGB preset save / load / delete / apply.
//!
//! Each handler takes the shared [`DaemonState`] (plus a [`Sender`] for daemon
//! events where mutation needs to trigger a refresh) and returns an
//! [`IpcResponse`]. The dispatcher in [`crate::ipc_server`] is a thin match
//! that delegates here.

use std::sync::mpsc::Sender;

use lianli_shared::ipc::IpcResponse;
use parking_lot::Mutex;
use std::sync::Arc;

use crate::service::DaemonEvent;

pub mod config;
pub mod fan;
pub mod lcd;
pub mod presets;
pub mod rgb;
pub mod system;
pub mod templates;
pub mod wireless;

pub(crate) use crate::persistence::{write_config, write_rgb_presets};

/// Type alias for the shared state reference handlers receive.
pub(crate) type SharedState = Arc<Mutex<crate::ipc_server::DaemonState>>;

/// Persist `state.config` to disk and notify the daemon's event loop that
/// something changed. Used by every IPC handler that mutates config.
pub(crate) fn persist_and_notify(
    state: &mut crate::ipc_server::DaemonState,
    tx: &Sender<DaemonEvent>,
    label: &str,
) -> IpcResponse {
    use tracing::info;
    let Some(config) = state.config.as_ref() else {
        return IpcResponse::error("no config loaded");
    };
    match write_config(&state.config_path, config) {
        Ok(()) => {
            let _ = tx.send(DaemonEvent::IpcUpdate);
            info!("{label}: config persisted, notified daemon");
            IpcResponse::ok(serde_json::json!(null))
        }
        Err(e) => IpcResponse::error(format!("failed to write config: {e}")),
    }
}

/// Suppress unused-import warning — `SharedState` is re-exported for handler
/// modules to use.
#[allow(dead_code)]
fn _imports_ok(_: SharedState) {}
