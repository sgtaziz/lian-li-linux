//! Newline-delimited JSON over the selected daemon's Unix socket.

use crate::command_error::CommandError;
use lianli_shared::daemon::DaemonInfo;
use lianli_shared::ipc::{IpcRequest, IpcResponse, TelemetrySnapshot};
use lianli_shared::ui_message::{UiError, UiMessage};
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, BufReader, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tracing::debug;

const TIMEOUT: Duration = Duration::from_secs(5);
const CONNECT_RETRY: Duration = Duration::from_millis(50);

// A blocking connect waits forever while a stalled daemon leaves its accept backlog full.
fn connect(path: &str) -> io::Result<UnixStream> {
    let bytes = std::path::Path::new(path).as_os_str().as_bytes();
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.len() >= addr.sun_path.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "socket path is too long",
        ));
    }
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (slot, byte) in addr.sun_path.iter_mut().zip(bytes) {
        *slot = *byte as libc::c_char;
    }
    let raw = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            0,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let result = unsafe {
            libc::connect(
                fd.as_raw_fd(),
                std::ptr::addr_of!(addr).cast::<libc::sockaddr>(),
                std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
            )
        };
        if result == 0 {
            break;
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::EINTR) => {}
            Some(libc::EAGAIN) if Instant::now() < deadline => std::thread::sleep(CONNECT_RETRY),
            Some(libc::EAGAIN) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the daemon did not accept the connection in time",
                ))
            }
            _ => return Err(error),
        }
    }
    let stream = UnixStream::from(fd);
    stream.set_nonblocking(false)?;
    Ok(stream)
}

fn system_socket() -> &'static str {
    static PATH: OnceLock<String> = OnceLock::new();
    PATH.get_or_init(|| {
        lianli_shared::installation::InstallationContext::detect()
            .system_socket_path()
            .to_string_lossy()
            .into_owned()
    })
}

/// Last socket that accepted a connection
static ACTIVE_SOCKET: OnceLock<Mutex<Option<String>>> = OnceLock::new();
fn active_lock() -> &'static Mutex<Option<String>> {
    ACTIVE_SOCKET.get_or_init(|| Mutex::new(None))
}

fn user_socket() -> String {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    format!("{runtime_dir}/lianli-daemon.sock")
}

/// Candidate daemon socket paths
fn candidate_paths() -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut v: Vec<String> = Vec::new();
    for p in active_lock()
        .lock()
        .unwrap()
        .clone()
        .into_iter()
        .chain(std::iter::once(user_socket()))
        .chain(std::iter::once(system_socket().to_string()))
    {
        if seen.insert(p.clone()) {
            v.push(p);
        }
    }
    v
}

/// Socket path to surface to the UI (the active one, else the per-user default).
pub fn socket_path() -> String {
    active_lock()
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(user_socket)
}

/// Combined result of a single poll cycle, returned to the frontend store.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PollResult {
    pub connected: bool,
    pub socket_path: String,
    #[serde(default)]
    pub daemon_info: Option<DaemonInfo>,
    pub write_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_error_ui: Option<UiMessage>,
    pub devices: Vec<lianli_shared::ipc::DeviceInfo>,
    pub telemetry: TelemetrySnapshot,
}

fn send_raw(request: IpcRequest, expected: Option<&str>) -> Result<IpcResponse, CommandError> {
    let mut last_err: Option<CommandError> = None;

    for path in candidate_paths() {
        let stream = match connect(&path) {
            Ok(s) => s,
            Err(e) => {
                last_err = Some(CommandError::ui(
                    format!("cannot connect to daemon at {path}: {e}"),
                    UiMessage::new("gui.ipc.connectFailed")
                        .param("path", &path)
                        .param("error", e),
                ));
                continue;
            }
        };
        let request = prepare_request_for(request, expected, || {
            exchange(&stream, r#"{"method":"GetDaemonInfo"}"#, false)
        })?;
        let json = serde_json::to_string(&request).map_err(|error| error.to_string())?;
        match ipc_round_trip(stream, &json) {
            Ok(resp) => {
                *active_lock().lock().unwrap() = Some(path);
                return Ok(resp);
            }
            Err(e) => {
                *active_lock().lock().unwrap() = None;
                return Err(CommandError::ui(
                    format!("Daemon response failed. The request was not retried because it may have been applied: {e}"),
                    UiMessage::new("gui.ipc.responseFailed").param("error", e),
                ));
            }
        }
    }

    *active_lock().lock().unwrap() = None;
    Err(last_err.unwrap_or_else(|| {
        CommandError::ui(
            "no daemon socket candidates",
            UiMessage::new("gui.ipc.noCandidates"),
        )
    }))
}

#[cfg(test)]
fn prepare_request(
    request: IpcRequest,
    read_info: impl FnOnce() -> Result<IpcResponse, String>,
) -> Result<IpcRequest, CommandError> {
    prepare_request_for(request, None, read_info)
}

fn prepare_request_for(
    request: IpcRequest,
    expected: Option<&str>,
    read_info: impl FnOnce() -> Result<IpcResponse, String>,
) -> Result<IpcRequest, CommandError> {
    if matches!(request, IpcRequest::Guarded { .. }) {
        return Err(CommandError::ui(
            "The GUI backend supplies compatibility checks",
            UiMessage::new("gui.ipc.guardSupplied"),
        ));
    }
    if request.is_read_only() && expected.is_none() {
        return Ok(request);
    }
    let info: DaemonInfo =
        serde_json::from_value(response_data(read_info()?)?).map_err(|error| {
            CommandError::ui(
                format!("Cannot verify daemon compatibility: {error}"),
                UiMessage::new("gui.ipc.compatibilityUnverified").param("error", error),
            )
        })?;
    if expected.is_some_and(|expected| expected != info.instance_id) {
        return Err(CommandError::ui(
            "The daemon changed. Review copied settings before saving",
            UiMessage::new("gui.ipc.daemonChanged"),
        ));
    }
    if request.is_read_only() {
        return Ok(request);
    }
    Ok(IpcRequest::Guarded {
        guard: info
            .checked_write_guard(env!("CARGO_PKG_VERSION"))
            .map_err(|error| CommandError::ui(error.to_string(), error.message))?,
        request: Box::new(request),
    })
}

/// Write one JSON request on a connected stream and read one JSON response.
fn ipc_round_trip(stream: UnixStream, json: &str) -> Result<IpcResponse, String> {
    exchange(&stream, json, true)
}

fn exchange(stream: &UnixStream, json: &str, finish: bool) -> Result<IpcResponse, String> {
    stream
        .set_read_timeout(Some(TIMEOUT))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(TIMEOUT))
        .map_err(|error| error.to_string())?;

    {
        let mut writer = stream;
        writer
            .write_all(json.as_bytes())
            .map_err(|e| format!("write error: {e}"))?;
        writer
            .write_all(b"\n")
            .map_err(|e| format!("write error: {e}"))?;
        writer.flush().map_err(|e| format!("flush error: {e}"))?;
    }

    if finish {
        stream
            .shutdown(std::net::Shutdown::Write)
            .map_err(|e| format!("shutdown error: {e}"))?;
    }

    let reader = BufReader::new(stream);
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

pub fn request(method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    request_expected(method, params, None).map_err(|error| error.message)
}

pub fn request_expected(
    method: &str,
    params: serde_json::Value,
    expected: Option<&str>,
) -> Result<serde_json::Value, crate::command_error::CommandError> {
    let req: IpcRequest = serde_json::from_value(
        serde_json::json!({ "method": method, "params": params }),
    )
    .map_err(|error| {
        CommandError::ui(
            format!("Unsupported request: {error}"),
            UiMessage::new("gui.ipc.unsupportedRequest").param("error", error),
        )
    })?;
    if !req.is_read_only() && crate::service_operations::active() {
        return Err(CommandError::ui(
            "Wait for the service operation to finish before changing daemon settings.",
            UiMessage::new("gui.ipc.serviceBusy"),
        ));
    }
    debug!("ipc -> {method}");
    match send_raw(req, expected)? {
        IpcResponse::Ok { data } => Ok(data),
        IpcResponse::Error { message, ui } => {
            Err(crate::command_error::CommandError { message, ui })
        }
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

pub fn poll() -> PollResult {
    for path in candidate_paths() {
        let result = poll_at(&path, |method| {
            let stream = connect(&path).map_err(|e| e.to_string())?;
            let json = serde_json::json!({ "method": method, "params": null }).to_string();
            ipc_round_trip(stream, &json)
        });
        match result {
            Ok(result) => {
                *active_lock().lock().unwrap() = Some(path);
                return result;
            }
            Err(error) => debug!("poll at {path} failed: {error}"),
        }
    }
    *active_lock().lock().unwrap() = None;
    PollResult {
        socket_path: user_socket(),
        ..Default::default()
    }
}

fn poll_at(
    path: &str,
    mut send: impl FnMut(&str) -> Result<IpcResponse, String>,
) -> Result<PollResult, String> {
    let daemon_info: Option<DaemonInfo> = match send("GetDaemonInfo")? {
        IpcResponse::Ok { data } => Some(
            serde_json::from_value(data).map_err(|e| format!("invalid daemon identity: {e}"))?,
        ),
        IpcResponse::Error { message, .. }
            if message.contains("unknown variant `GetDaemonInfo`") =>
        {
            response_data(send("Ping")?)?;
            None
        }
        IpcResponse::Error { message, .. } => return Err(message),
    };
    let devices = serde_json::from_value(response_data(send("ListDevices")?)?)
        .map_err(|e| format!("invalid device list: {e}"))?;
    let telemetry = serde_json::from_value(response_data(send("GetTelemetry")?)?)
        .map_err(|e| format!("invalid telemetry: {e}"))?;
    let write_error = match &daemon_info {
        Some(info) => info.checked_write_guard(env!("CARGO_PKG_VERSION")).err(),
        None => Some(UiError::new(
            UiMessage::new("gui.compatibility.unreported"),
            "Changes are disabled: this daemon cannot report compatibility. Update both applications and restart the selected daemon cleanly.",
        )),
    };

    Ok(PollResult {
        connected: true,
        socket_path: path.into(),
        daemon_info,
        write_error_ui: write_error.as_ref().map(|error| error.message.clone()),
        write_error: write_error.map(|error| error.to_string()),
        devices,
        telemetry,
    })
}

fn response_data(response: IpcResponse) -> Result<serde_json::Value, String> {
    match response {
        IpcResponse::Ok { data } => Ok(data),
        IpcResponse::Error { message, .. } => Err(message),
    }
}

pub fn connection_info() -> (bool, String) {
    (ping(), socket_path().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_shared::daemon::{DaemonMode, IPC_PROTOCOL_VERSION};

    #[test]
    fn identity_and_write_share_one_connection_without_closing_it_early() {
        let (client, server) = UnixStream::pair().unwrap();
        server.set_read_timeout(Some(TIMEOUT)).unwrap();
        let mut daemon = info("same-peer");
        daemon.version = env!("CARGO_PKG_VERSION").into();
        daemon
            .capabilities
            .push(lianli_shared::daemon::GUARDED_WRITES.into());
        let peer = std::thread::spawn(move || {
            let mut reader = BufReader::new(server);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(matches!(
                serde_json::from_str::<IpcRequest>(&line).unwrap(),
                IpcRequest::GetDaemonInfo
            ));
            writeln!(
                reader.get_mut(),
                "{}",
                serde_json::to_string(&IpcResponse::ok(&daemon)).unwrap()
            )
            .unwrap();
            line.clear();
            assert!(reader.read_line(&mut line).unwrap() > 0);
            let request = serde_json::from_str::<IpcRequest>(&line).unwrap();
            assert!(matches!(
                request.authorize(&daemon).unwrap(),
                IpcRequest::SetLcdTemplates { .. }
            ));
            writeln!(
                reader.get_mut(),
                "{}",
                serde_json::to_string(&IpcResponse::ok("applied")).unwrap()
            )
            .unwrap();
            line.clear();
            assert_eq!(reader.read_line(&mut line).unwrap(), 0);
        });
        let request = prepare_request(IpcRequest::SetLcdTemplates { templates: vec![] }, || {
            exchange(&client, r#"{"method":"GetDaemonInfo"}"#, false)
        })
        .unwrap();
        let response = ipc_round_trip(client, &serde_json::to_string(&request).unwrap()).unwrap();
        assert_eq!(response_data(response).unwrap(), "applied");
        peer.join().unwrap();
    }

    fn info(instance: &str) -> DaemonInfo {
        DaemonInfo {
            version: "1.0.0".into(),
            protocol_version: IPC_PROTOCOL_VERSION,
            instance_id: instance.into(),
            pid: 123,
            ownership_lock: None,
            service_invocation: None,
            service_operation_lock: None,
            mode: DaemonMode::System,
            config_path: "/var/lib/lianli/config.json".into(),
            capabilities: vec!["daemon_info".into()],
        }
    }

    #[test]
    fn reads_do_not_negotiate_but_writes_require_current_identity() {
        assert!(prepare_request(IpcRequest::GetConfig, || panic!("read negotiated")).is_ok());
        let write = || IpcRequest::SetLcdTemplates { templates: vec![] };
        assert!(prepare_request(write(), || Ok(IpcResponse::error("unknown method"))).is_err());
        assert!(prepare_request(write(), || Ok(IpcResponse::ok(info("old")))).is_err());
        let mut current = info("current");
        current.version = env!("CARGO_PKG_VERSION").into();
        current
            .capabilities
            .push(lianli_shared::daemon::GUARDED_WRITES.into());
        let request = prepare_request(write(), || Ok(IpcResponse::ok(&current))).unwrap();
        assert!(matches!(
            request.authorize(&current).unwrap(),
            IpcRequest::SetLcdTemplates { .. }
        ));
    }

    #[test]
    fn copied_selection_requests_reject_a_different_connected_instance() {
        let mut current = info("current");
        current.version = env!("CARGO_PKG_VERSION").into();
        current
            .capabilities
            .push(lianli_shared::daemon::GUARDED_WRITES.into());
        for request in [
            IpcRequest::GetLcdTemplates,
            IpcRequest::SetLcdTemplates { templates: vec![] },
        ] {
            assert!(
                prepare_request_for(request.clone(), Some("old"), || Ok(IpcResponse::ok(
                    &current
                )))
                .is_err()
            );
            assert!(
                prepare_request_for(request, Some("current"), || Ok(IpcResponse::ok(&current)))
                    .is_ok()
            );
        }
    }

    #[test]
    fn poll_reports_the_current_instance_without_an_extra_ping() {
        for instance in ["first", "restarted"] {
            let result = poll_at("/test.sock", |method| {
                Ok(match method {
                    "GetDaemonInfo" => IpcResponse::ok(info(instance)),
                    "ListDevices" => IpcResponse::ok(serde_json::json!([])),
                    "GetTelemetry" => IpcResponse::ok(TelemetrySnapshot::default()),
                    other => panic!("unexpected {other}"),
                })
            })
            .unwrap();
            assert!(result.connected);
            assert_eq!(result.daemon_info.unwrap().instance_id, instance);
        }
    }

    #[test]
    fn legacy_daemon_still_connects_without_fabricated_identity() {
        let result = poll_at("/legacy.sock", |method| {
            Ok(match method {
                "GetDaemonInfo" => {
                    IpcResponse::error("invalid request: unknown variant `GetDaemonInfo`")
                }
                "Ping" => IpcResponse::ok("pong"),
                "ListDevices" => IpcResponse::ok(serde_json::json!([])),
                "GetTelemetry" => IpcResponse::ok(TelemetrySnapshot::default()),
                other => panic!("unexpected {other}"),
            })
        })
        .unwrap();
        assert!(result.connected);
        assert!(result.daemon_info.is_none());
        assert!(result.write_error.is_some());
    }

    #[test]
    fn failed_telemetry_is_not_reported_as_a_healthy_empty_snapshot() {
        let error = poll_at("/test.sock", |method| {
            Ok(match method {
                "GetDaemonInfo" => IpcResponse::ok(info("first")),
                "ListDevices" => IpcResponse::ok(serde_json::json!([])),
                "GetTelemetry" => IpcResponse::error("service unavailable"),
                other => panic!("unexpected {other}"),
            })
        })
        .unwrap_err();
        assert_eq!(error, "service unavailable");
    }

    #[test]
    fn connect_gives_up_when_the_daemon_stops_accepting() {
        let path =
            std::env::temp_dir().join(format!("lianli-gui-backlog-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert_eq!(unsafe { libc::listen(listener.as_raw_fd(), 0) }, 0);
        let path_text = path.to_str().unwrap();
        let mut pending = Vec::new();
        let error = loop {
            match connect(path_text) {
                Ok(stream) => pending.push(stream),
                Err(error) => break error,
            }
            assert!(pending.len() < 8, "backlog never filled");
        };
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        let _ = std::fs::remove_file(&path);
    }
}
