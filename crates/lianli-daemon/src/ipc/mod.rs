//! IPC layer: Unix domain socket server + per-concern request handlers.
//!
//! The server ([`server`]) accepts connections, deserializes [`IpcRequest`]s,
//! and dispatches to one of the handler submodules:
//!
//! - [`system`] — read-only queries: ping, sensor/device enumeration, telemetry.
//! - [`config`] — config-writing handlers (`SetConfig`, `SetLcdMedia`, etc.).
//! - [`fan`] — fan-specific handlers (`SetEne6k77FanQuantity`, fan direction).
//! - [`rgb`] — RGB effect / direct-color / zone queries.
//! - [`lcd`] — display-mode switching, template preview rendering.
//! - [`wireless`] — bind / unbind RF devices.
//! - [`templates`] — LCD template CRUD.
//! - [`presets`] — RGB preset save / load / delete / apply.

mod server;

pub mod config;
pub mod fan;
pub mod lcd;
pub mod presets;
pub mod profiles;
pub mod rgb;
pub mod system;
pub mod templates;
pub mod wireless;

pub use server::{start_ipc_server, DaemonState};

use std::sync::mpsc::Sender;

use lianli_shared::ipc::IpcResponse;
use parking_lot::Mutex;
use std::sync::Arc;

use crate::service::DaemonEvent;

pub(crate) use crate::persistence::{write_config, write_rgb_presets};

/// Type alias for the shared state reference handlers receive.
pub(crate) type SharedState = Arc<Mutex<DaemonState>>;

/// Persist `state.config` to disk and notify the daemon's event loop that
/// something changed. Used by every IPC handler that mutates config.
pub(crate) fn persist_and_notify(
    state: &mut DaemonState,
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
