use anyhow::{Context, Result};
use lianli_media::MediaAsset;
use lianli_shared::ipc::{IpcRequest, IpcResponse};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct SavedTargetState {
    pub target_index: usize,
    pub device_identity: String,
    pub media_asset: Arc<MediaAsset>,
    pub custom_h264: bool,
    pub original_brightness: Option<u8>,
}

#[derive(Debug)]
pub struct PixelCleanSession {
    pub session_id: u64,
    pub duration_minutes: u16,
    pub original_targets: Vec<SavedTargetState>,
    pub clean_until: Instant,
}

fn send_ipc(socket_path: &PathBuf, request: &IpcRequest) -> Result<IpcResponse> {
    let mut stream = UnixStream::connect(socket_path).with_context(|| {
        format!(
            "Failed to connect to lianli-daemon socket at {}",
            socket_path.display()
        )
    })?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    let json = serde_json::to_string(request)?;
    stream.write_all(json.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.shutdown(std::net::Shutdown::Write)?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let resp: IpcResponse = serde_json::from_str(&line)?;
    Ok(resp)
}

struct SignalHandlers(Vec<signal_hook::SigId>);

impl SignalHandlers {
    fn register(cancelled: &Arc<AtomicBool>) -> Result<Self> {
        let mut handlers = Self(Vec::new());
        for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
            handlers.0.push(
                signal_hook::flag::register(signal, Arc::clone(cancelled))
                    .context("registering pixel cleaner cancellation handler")?,
            );
        }
        Ok(handlers)
    }
}

impl Drop for SignalHandlers {
    fn drop(&mut self) {
        for id in self.0.drain(..) {
            signal_hook::low_level::unregister(id);
        }
    }
}

fn response_data(response: IpcResponse) -> Result<serde_json::Value> {
    match response {
        IpcResponse::Ok { data } => Ok(data),
        IpcResponse::Error { message } => anyhow::bail!("{message}"),
    }
}

fn prepare_and_start(
    socket: &PathBuf,
    device_id: Option<String>,
    minutes: u16,
    cancelled: &AtomicBool,
) -> Result<Option<u64>> {
    anyhow::ensure!(minutes > 0, "Duration must be positive");
    if cancelled.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let data = response_data(send_ipc(
        socket,
        &IpcRequest::StartPixelClean {
            device_id: device_id.clone(),
            duration_minutes: minutes,
            preparation_id: None,
        },
    )?)?;
    let id = data
        .get("session_id")
        .and_then(serde_json::Value::as_u64)
        .context("Daemon response missing session_id")?;
    let result = (|| {
        if data.get("started").and_then(serde_json::Value::as_bool) == Some(true) {
            return Ok(Some(id));
        }
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            if cancelled.load(Ordering::Relaxed) {
                return Ok(None);
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "Pixel cleaner preparation timed out"
            );
            let status = response_data(send_ipc(
                socket,
                &IpcRequest::GetPixelCleanPreparation { session_id: id },
            )?)?;
            if let Some(error) = status.get("error").and_then(serde_json::Value::as_str) {
                anyhow::bail!("{error}");
            }
            if status.get("ready").and_then(serde_json::Value::as_bool) == Some(true) {
                if cancelled.load(Ordering::Relaxed) {
                    return Ok(None);
                }
                let started = response_data(send_ipc(
                    socket,
                    &IpcRequest::StartPixelClean {
                        device_id,
                        duration_minutes: minutes,
                        preparation_id: Some(id),
                    },
                )?)?;
                anyhow::ensure!(
                    started.get("started").and_then(serde_json::Value::as_bool) == Some(true),
                    "Daemon did not confirm cleaner activation"
                );
                return Ok(Some(id));
            }
            sleep_until_cancelled(cancelled, Duration::from_millis(500));
        }
    })();
    if !matches!(result, Ok(Some(_))) {
        match send_ipc(
            socket,
            &IpcRequest::StopPixelClean {
                device_id: None,
                session_id: Some(id),
            },
        ) {
            Ok(IpcResponse::Ok { .. }) => {}
            Ok(IpcResponse::Error { message }) => {
                eprintln!("Could not cancel preparation: {message}")
            }
            Err(error) => eprintln!("Could not cancel preparation: {error:#}"),
        }
    }
    result
}

pub fn run_clean_command(
    socket_path: PathBuf,
    device_id: Option<String>,
    minutes: u16,
) -> Result<()> {
    anyhow::ensure!(minutes > 0, "Duration must be positive");
    let minutes = minutes.min(lianli_shared::ipc::MAX_CLEAN_MINUTES);
    let cancelled = Arc::new(AtomicBool::new(false));
    let _handlers = SignalHandlers::register(&cancelled)?;
    println!("Preparing pixel conditioning ({minutes} minutes). Press Ctrl+C to cancel.");
    let Some(id) = prepare_and_start(&socket_path, device_id, minutes, &cancelled)? else {
        println!("Pixel cleaner preparation cancelled.");
        return Ok(());
    };
    println!("Pixel cleaner active at 75% brightness. Press Ctrl+C to stop.");
    let polling = (|| -> Result<()> {
        while !cancelled.load(Ordering::Relaxed) {
            let statuses =
                response_data(send_ipc(&socket_path, &IpcRequest::GetPixelCleanStatus)?)?;
            let remaining = statuses
                .as_object()
                .context("Invalid cleaner status response")?
                .values()
                .filter(|s| s.get("session_id").and_then(serde_json::Value::as_u64) == Some(id))
                .filter_map(|s| {
                    s.get("remaining_seconds")
                        .and_then(serde_json::Value::as_u64)
                })
                .max();
            let Some(seconds) = remaining else { break };
            print!(
                "\rTime remaining: {:02}:{:02} (Ctrl+C to stop)",
                seconds / 60,
                seconds % 60
            );
            std::io::stdout().flush()?;
            sleep_until_cancelled(&cancelled, Duration::from_secs(1));
        }
        Ok(())
    })();
    let result = send_ipc(
        &socket_path,
        &IpcRequest::StopPixelClean {
            device_id: None,
            session_id: Some(id),
        },
    )?;
    println!("\n{}", stop_message(result)?);
    polling
}

fn stop_message(response: IpcResponse) -> Result<&'static str> {
    let data = response_data(response)?;
    match data.get("stopped").and_then(serde_json::Value::as_bool) {
        Some(true) => Ok("Stopped the pixel cleaner and requested restoration of the previous display."),
        Some(false) => Ok("The session is no longer active; completion and restoration were not confirmed by this stop request."),
        None => anyhow::bail!("Daemon response missing stopped result"),
    }
}

fn sleep_until_cancelled(cancelled: &AtomicBool, duration: Duration) {
    let deadline = Instant::now() + duration;
    while !cancelled.load(Ordering::Relaxed) && Instant::now() < deadline {
        std::thread::sleep(
            Duration::from_millis(50).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn false_stop_never_reports_successful_completion() {
        let message = stop_message(IpcResponse::ok(serde_json::json!({"stopped": false}))).unwrap();
        assert!(message.contains("not confirmed"));
        assert!(stop_message(IpcResponse::ok(serde_json::json!({}))).is_err());
        assert!(stop_message(IpcResponse::error("failed")).is_err());
    }

    #[test]
    fn cancelled_start_does_not_connect_or_wait() {
        let cancel = AtomicBool::new(true);
        assert!(
            prepare_and_start(&"/nonexistent/socket".into(), None, 1, &cancel)
                .unwrap()
                .is_none()
        );
        sleep_until_cancelled(&cancel, Duration::from_secs(60));
    }
}
