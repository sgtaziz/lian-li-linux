//! Unix-socket bridge to the lianli-daemon.
//!
//! Mirrors the protocol used by the Slint GUI's `ipc_client.rs`: newline-
//! delimited JSON over `$XDG_RUNTIME_DIR/lianli-daemon.sock`. Each request
//! opens a fresh connection, writes one JSON line, shuts down the write half,
//! and reads exactly one response line.

use lianli_shared::ipc::{IpcResponse, TelemetrySnapshot};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::OnceLock;
use std::time::Duration;
use tracing::debug;

const TIMEOUT: Duration = Duration::from_secs(5);

/// Resolved daemon socket path (`$XDG_RUNTIME_DIR/lianli-daemon.sock`).
pub fn socket_path() -> &'static str {
    static PATH: OnceLock<String> = OnceLock::new();
    PATH.get_or_init(|| {
        let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
        format!("{runtime_dir}/lianli-daemon.sock")
    })
}

/// Combined result of a single poll cycle, returned to the frontend store.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PollResult {
    pub connected: bool,
    pub socket_path: String,
    pub devices: Vec<lianli_shared::ipc::DeviceInfo>,
    pub telemetry: TelemetrySnapshot,
}

/// Send a raw JSON request object to the daemon and return the parsed response.
fn send_raw(request: &serde_json::Value) -> Result<IpcResponse, String> {
    let path = socket_path();
    let stream = UnixStream::connect(path)
        .map_err(|e| format!("cannot connect to daemon at {path}: {e}"))?;

    stream.set_read_timeout(Some(TIMEOUT)).ok();
    stream.set_write_timeout(Some(TIMEOUT)).ok();

    let json = serde_json::to_string(request).map_err(|e| format!("serialize error: {e}"))?;

    {
        let mut writer = &stream;
        writer
            .write_all(json.as_bytes())
            .map_err(|e| format!("write error: {e}"))?;
        writer
            .write_all(b"\n")
            .map_err(|e| format!("write error: {e}"))?;
        writer.flush().map_err(|e| format!("flush error: {e}"))?;
    }

    // Shut down the write side so the daemon sees EOF while reading.
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|e| format!("shutdown error: {e}"))?;

    let reader = BufReader::new(&stream);
    for line in reader.lines() {
        let line = line.map_err(|e| format!("read error: {e}"))?;
        if line.trim().is_empty() {
            continue;
        }
        let response: IpcResponse =
            serde_json::from_str(&line).map_err(|e| format!("parse error: {e}"))?;
        return Ok(response);
    }

    Err("no response from daemon".to_string())
}

/// Issue any IPC method by name, forwarding arbitrary params.
///
/// The request is serialized as `{"method": <method>, "params": <params>}`,
/// matching the daemon's `#[serde(tag = "method", content = "params")]` wire
/// format. On an `Ok` response the inner `data` value is returned; on `Error`
/// the message is propagated as `Err`.
pub fn request(method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    let req = serde_json::json!({ "method": method, "params": params });
    debug!("ipc -> {method}");
    match send_raw(&req)? {
        IpcResponse::Ok { data } => Ok(data),
        IpcResponse::Error { message } => Err(message),
    }
}

/// Quick liveness check — a single `Ping`.
pub fn ping() -> bool {
    match request("Ping", serde_json::Value::Null) {
        Ok(_) => true,
        Err(e) => {
            debug!("ping failed: {e}");
            false
        }
    }
}

/// Issue a Ping + ListDevices + GetTelemetry in sequence and bundle the result.
pub fn poll() -> PollResult {
    let connected = ping();
    let path = socket_path().to_string();
    if !connected {
        return PollResult {
            connected: false,
            socket_path: path,
            ..Default::default()
        };
    }

    let devices: Vec<lianli_shared::ipc::DeviceInfo> =
        serde_json::from_value(request("ListDevices", serde_json::Value::Null).unwrap_or_default())
            .unwrap_or_default();
    let telemetry: TelemetrySnapshot = serde_json::from_value(
        request("GetTelemetry", serde_json::Value::Null).unwrap_or_default(),
    )
    .unwrap_or_default();

    PollResult {
        connected: true,
        socket_path: path,
        devices,
        telemetry,
    }
}

/// Fetch the daemon's reported version string (best-effort, parsed from a
/// `Ping`-style probe). The daemon does not currently expose a dedicated
/// version IPC, so we surface the socket path and connection state instead.
pub fn connection_info() -> (bool, String) {
    (ping(), socket_path().to_string())
}
