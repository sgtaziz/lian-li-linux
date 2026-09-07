//! Pixel cleaner module for exercising LCD panels to relieve image retention.

use anyhow::{Context, Result};
use lianli_media::MediaAsset;
use lianli_shared::ipc::{IpcRequest, IpcResponse};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::warn;

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
    pub original_targets: Vec<SavedTargetState>,
    pub clean_until: Instant,
}

/// Locate or extract the pixel cleaner video asset.
pub fn pixel_cleaner_asset_path() -> PathBuf {
    if let Ok(path) = std::env::var("LIANLI_PIXEL_CLEANER_PATH") {
        let p = PathBuf::from(path);
        if p.exists() {
            return p;
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        let user_asset = PathBuf::from(home).join(".config/lianli/pixel_cleaner.mp4");
        if user_asset.exists() {
            return user_asset;
        }
    }

    for loc in [
        "/usr/share/lianli/media/pixel_cleaner.mp4",
        "/usr/local/share/lianli/media/pixel_cleaner.mp4",
        "assets/media/pixel_cleaner.mp4",
    ] {
        let p = PathBuf::from(loc);
        if p.exists() {
            return p;
        }
    }

    // Embedded fallback ensures binary functions stand-alone without asset files installed.
    // Prefer user-private runtime directory over shared /tmp, and create atomically with 0o600.
    let target_dir = std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .or_else(|_| {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join(".config/lianli"))
        })
        .unwrap_or_else(|_| std::env::temp_dir());
    let temp_path = target_dir.join("lianli_pixel_cleaner.mp4");
    if !temp_path.exists() {
        const EMBEDDED_CLEANER: &[u8] =
            include_bytes!("../../../assets/media/pixel_cleaner.mp4");
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temp_path)
            {
                Ok(mut f) => {
                    if let Err(e) = f.write_all(EMBEDDED_CLEANER) {
                        warn!("Failed to write embedded pixel cleaner asset: {e}");
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => {
                    warn!("Failed to extract embedded pixel cleaner asset: {e}");
                }
            }
        }
        #[cfg(not(unix))]
        {
            if let Err(e) = std::fs::write(&temp_path, EMBEDDED_CLEANER) {
                warn!("Failed to extract embedded pixel cleaner asset: {e}");
            }
        }
    }
    temp_path
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

/// Run pixel cleaner via CLI client against the active daemon.
pub fn run_clean_command(
    socket_path: PathBuf,
    device_id: Option<String>,
    minutes: u32,
) -> Result<()> {
    println!(
        "Connecting to lianli-daemon to start pixel conditioning (duration: {minutes}m)..."
    );

    let start_req = IpcRequest::StartPixelClean {
        device_id: device_id.clone(),
        duration_minutes: minutes,
    };

    let session_id = match send_ipc(&socket_path, &start_req)? {
        IpcResponse::Ok { data } => {
            let sid = data.get("session_id").and_then(|v| v.as_u64());
            println!(
                "[+] Pixel cleaner active at 75% brightness. Asset: pixel_cleaner.mp4\n\
                 [+] Running for {minutes} minutes. Press Ctrl+C at any time to cancel and restore previous display."
            );
            sid
        }
        IpcResponse::Error { message } => {
            anyhow::bail!("Daemon rejected StartPixelClean: {message}");
        }
    };

    let running = Arc::new(AtomicBool::new(true));
    let r = Arc::clone(&running);
    let _ = signal_hook::flag::register(signal_hook::consts::SIGINT, r.clone());
    let _ = signal_hook::flag::register(signal_hook::consts::SIGTERM, r);

    let total_secs = minutes as u64 * 60;
    let start_time = Instant::now();

    while running.load(Ordering::Relaxed) && start_time.elapsed().as_secs() < total_secs {
        let remaining = total_secs.saturating_sub(start_time.elapsed().as_secs());
        let mins = remaining / 60;
        let secs = remaining % 60;
        print!("\r[*] Time remaining: {:02}:{:02} (Ctrl+C to stop)", mins, secs);
        let _ = std::io::stdout().flush();
        thread_sleep_interruptible(&running, Duration::from_secs(1));
    }

    println!("\n[*] Restoring original LCD configuration...");
    let stop_req = IpcRequest::StopPixelClean {
        device_id,
        session_id,
    };
    match send_ipc(&socket_path, &stop_req)? {
        IpcResponse::Ok { .. } => {
            println!("[+] Restored previous LCD media and brightness successfully.");
        }
        IpcResponse::Error { message } => {
            eprintln!("[-] Warning: Failed to restore previous config: {message}");
        }
    }

    Ok(())
}

fn thread_sleep_interruptible(running: &AtomicBool, dur: Duration) {
    let step = Duration::from_millis(100);
    let mut elapsed = Duration::ZERO;
    while running.load(Ordering::Relaxed) && elapsed < dur {
        std::thread::sleep(step);
        elapsed += step;
    }
}
