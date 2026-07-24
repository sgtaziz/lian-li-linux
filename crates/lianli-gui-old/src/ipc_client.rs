//! IPC client: connects to the daemon's Unix socket.

use lianli_shared::ipc::{IpcRequest, IpcResponse};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::LazyLock;
use std::time::Duration;

pub static SOCKET_PATH: LazyLock<String> = LazyLock::new(|| {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    format!("{runtime_dir}/lianli-daemon.sock")
});
const TIMEOUT: Duration = Duration::from_secs(5);

/// Send a single IPC request and return the response.
pub fn send_request(request: &IpcRequest) -> Result<IpcResponse, String> {
    let stream = UnixStream::connect(SOCKET_PATH.as_str())
        .map_err(|e| format!("cannot connect to daemon at {}: {e}", *SOCKET_PATH))?;

    stream.set_read_timeout(Some(TIMEOUT)).ok();
    stream.set_write_timeout(Some(TIMEOUT)).ok();

    let mut writer = &stream;
    let json = serde_json::to_string(request).map_err(|e| format!("serialize error: {e}"))?;
    writer
        .write_all(json.as_bytes())
        .map_err(|e| format!("write error: {e}"))?;
    writer
        .write_all(b"\n")
        .map_err(|e| format!("write error: {e}"))?;
    writer.flush().map_err(|e| format!("flush error: {e}"))?;

    // Shut down write side so daemon sees EOF for reading
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

/// Check if the daemon is reachable.
pub fn is_daemon_running() -> bool {
    send_request(&IpcRequest::Ping).is_ok()
}

/// Extract the data field from an OK response, or return the error message.
pub fn unwrap_response<T: serde::de::DeserializeOwned>(response: IpcResponse) -> Result<T, String> {
    match response {
        IpcResponse::Ok { data } => {
            serde_json::from_value(data).map_err(|e| format!("response parse error: {e}"))
        }
        IpcResponse::Error { message } => Err(message),
    }
}
