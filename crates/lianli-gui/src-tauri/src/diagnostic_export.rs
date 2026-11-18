use anyhow::{ensure, Context, Result};
use lianli_control::diagnostic_logs::LogSource;
use lianli_control::ui_text::keyed;
use lianli_shared::daemon::DaemonInfo;
use lianli_shared::installation::{InstallationContext, InstallationReport};
use lianli_shared::services::{ServiceProbe, UnitState};
use lianli_shared::ui_message::{UiError, UiMessage};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::Duration;
use tauri_plugin_dialog::DialogExt;

const MAX_BYTES: usize = 128 * 1024;
static RUNNING: AtomicBool = AtomicBool::new(false);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static PREVIEW: Mutex<Option<Preview>> = Mutex::new(None);

#[derive(Deserialize, Serialize)]
pub struct Input {
    report: InstallationReport,
    daemon: Option<DaemonInfo>,
    log_source: Option<LogSource>,
    #[serde(default)]
    media: std::collections::BTreeMap<usize, lianli_shared::ipc::MediaPreparationStatus>,
    #[serde(default)]
    desktop_streams: Vec<lianli_shared::ipc::DesktopStreamStatus>,
}

#[derive(Clone, Serialize)]
pub struct Preview {
    id: u64,
    text: String,
    can_save: bool,
}

struct Running;
impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::Release);
    }
}

fn redact(value: &str) -> String {
    let plain = lianli_control::diagnostic_logs::plain_text(value);
    let value = plain.as_str();
    let sensitive = [
        "token",
        "secret",
        "password",
        "authorization",
        "cookie",
        "credential",
    ];
    let file_extensions = [
        ".json", ".png", ".jpg", ".jpeg", ".gif", ".webp", ".mp4", ".mkv", ".webm", ".ttf", ".otf",
        ".conf", ".log",
    ];
    let mut words = Vec::new();
    for word in value.split_whitespace() {
        let lower = word.to_ascii_lowercase();
        if sensitive.iter().any(|sensitive| lower.contains(sensitive)) {
            words.push("[redacted]");
            break;
        }
        if word.contains(['/', '\\']) && !public_drm_node(word) && !public_slash_term(word) {
            words.push("[path redacted]");
            break;
        }
        if file_extensions
            .iter()
            .any(|extension| lower.contains(extension))
        {
            words.push("[file redacted]");
        } else if word
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|part| part.len() >= 24)
        {
            words.push("[identifier redacted]");
        } else {
            words.push(word);
        }
    }
    words
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .take(2048)
        .collect()
}

fn public_slash_term(value: &str) -> bool {
    let value = value.trim_matches(['\'', '"', '(', ')', '[', ']', ':', ',', '.']);
    matches!(
        value,
        "SL/AL"
            | "USB/HID"
            | "retry/retries"
            | "active/running"
            | "inactive/dead"
            | "failed/failed"
    ) || value.split_once('/').is_some_and(|(left, right)| {
        !left.is_empty()
            && !right.is_empty()
            && left.bytes().all(|byte| byte.is_ascii_digit())
            && right.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn public_drm_node(value: &str) -> bool {
    let value = value.trim_matches(['\'', '"', '(', ')', '[', ']', ':', ',']);
    value
        .strip_prefix("/dev/dri/")
        .and_then(|name| {
            name.strip_prefix("renderD")
                .or_else(|| name.strip_prefix("card"))
        })
        .is_some_and(|index| {
            !index.is_empty() && index.len() <= 5 && index.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn unit(probe: &ServiceProbe<UnitState>) -> Value {
    match probe {
        ServiceProbe::Known { value } => json!({
            "name": redact(&value.name), "load_state": redact(&value.load_state),
            "active_state": redact(&value.active_state), "sub_state": redact(&value.sub_state),
            "startup": redact(&value.unit_file_state), "pid": value.main_pid,
        }),
        ServiceProbe::Unavailable { reason, .. } => json!({"unavailable": redact(reason)}),
    }
}

fn summary(input: &Input, logs: Option<Result<Vec<String>>>) -> Result<String> {
    ensure!(
        input.report.findings.len() <= 64,
        "Too many findings to export"
    );
    ensure!(input.media.len() <= 64, "Too many media states to export");
    ensure!(
        input.desktop_streams.len() <= 64,
        "Too many desktop states to export"
    );
    let desktop_streams: Vec<_> = input.desktop_streams.iter().map(|status| json!({
        "bus": status.bus, "address": status.address, "product_id": status.product_id,
        "state": status.state, "backend": status.backend.as_deref().map(redact),
        "fallback_reason": status.fallback_reason.as_deref().map(redact),
        "error": status.error.as_deref().map(redact),
        "applied_generation": status.applied_generation, "applied_policy": status.applied_policy,
        "encoding": status.encoding,
    })).collect();
    let media: Vec<_> = input
        .media
        .iter()
        .map(|(index, status)| {
            json!({
                "lcd_index": index, "generation": status.generation, "state": status.state,
                "error": status.error.as_deref().map(redact),
                "last_playback_error": status.last_playback_error.as_deref().map(redact),
                "runtime": status.runtime.as_ref().map(|runtime| json!({
                    "stage": runtime.stage, "fps_limit": runtime.fps_limit,
                    "h264_transfer_started": runtime.h264_transfer_started,
                    "hardware_video_allowed": runtime.hardware_video_allowed,
                    "fallback_reason": runtime.fallback_reason.as_deref().map(redact),
                    "encoder": runtime.encoder.as_ref().map(|encoder| json!({"name": redact(&encoder.name), "software_fallback": encoder.software_fallback})),
                })),
            })
        })
        .collect();
    let findings: Vec<_> = input
        .report
        .findings
        .iter()
        .map(|finding| {
            json!({
                "code": redact(&finding.code), "state": finding.state, "severity": finding.severity,
                "feature": redact(&finding.feature), "title": redact(&finding.title),
                "evidence": redact(&finding.evidence),
            })
        })
        .collect();
    let daemon = input.daemon.as_ref().map(|info| {
        json!({
            "version": redact(&info.version), "protocol_version": info.protocol_version,
            "mode": info.mode, "pid": info.pid,
        })
    });
    let services = input.report.services.as_ref().map(|report| {
        json!({
            "user": unit(&report.user), "system": unit(&report.system),
            "global_user_startup": match &report.global_user {
                ServiceProbe::Known { value } => redact(value),
                ServiceProbe::Unavailable { reason, .. } => redact(reason),
            },
        })
    });
    let logs = logs.map(|result| match result {
        Ok(lines) => json!({"source": input.log_source, "messages": lines.iter().take(100).map(|line| redact(line)).collect::<Vec<_>>() }),
        Err(error) => json!({"source": input.log_source, "unavailable": redact(&format!("{error:#}"))}),
    });
    let text = serde_json::to_string_pretty(&json!({
        "format_version": 1, "gui_version": env!("CARGO_PKG_VERSION"),
        "checked_at_unix_ms": input.report.checked_at_unix_ms,
        "installation_context": match input.report.context { InstallationContext::Native => "native", InstallationContext::Distrobox { .. } => "distrobox", InstallationContext::UnsupportedContainer => "unsupported_container" },
        "daemon_context": input.report.daemon_context.as_ref().map(|context| match context { InstallationContext::Native => "native", InstallationContext::Distrobox { .. } => "distrobox", InstallationContext::UnsupportedContainer => "unsupported_container" }),
        "daemon": daemon, "services": services, "findings": findings, "media_preparation": media, "desktop_streams": desktop_streams, "logs": logs,
        "privacy": "Paths, container names, process environment, configuration contents and capture credentials are excluded. Free-text details containing paths, credential terms or long opaque identifiers are omitted. Review this preview before sharing.",
        "limitations": "Installation and telemetry are displayed snapshots. Saving requires daemon logs: up to 100 messages from the latest service invocation, or the last 100 nonempty lines of a selected log extract. Desktop status reflects the last successful USB frame delivery, not confirmed panel playback. Ordinary H.264 status identifies the transcode or live encoder, not delivery. Missing encoder status is unknown. Nothing is uploaded.",
    }))?;
    ensure!(text.len() <= MAX_BYTES, "Diagnostic export exceeds 128 KiB");
    Ok(text)
}

pub fn preview(input: Input) -> Result<Preview> {
    start(input, None)
}

pub fn preview_file(app: &tauri::AppHandle, input: Input, title: &str) -> Result<Option<Preview>> {
    let Some(selected) = app.dialog().file().set_title(title).blocking_pick_file() else {
        return Ok(None);
    };
    let path = selected
        .into_path()
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    start(input, Some(path)).map(Some)
}

fn saved_logs(path: &std::path::Path) -> Result<Vec<String>> {
    let metadata = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    ensure!(
        metadata.metadata()?.is_file(),
        "Choose a regular daemon log file"
    );
    let file = std::fs::File::open(format!("/proc/self/fd/{}", metadata.as_raw_fd()))?;
    let mut text = String::new();
    file.take(1024 * 1024 + 1).read_to_string(&mut text)?;
    ensure!(
        text.len() <= 1024 * 1024,
        "Choose a daemon log extract of at most 1 MiB"
    );
    let mut lines: Vec<_> = text
        .lines()
        .rev()
        .filter(|line| !line.trim().is_empty())
        .take(100)
        .map(str::to_owned)
        .collect();
    lines.reverse();
    Ok(lines)
}

fn start(input: Input, saved: Option<std::path::PathBuf>) -> Result<Preview> {
    ensure!(
        serde_json::to_vec(&input)?.len() <= 256 * 1024,
        "Diagnostic input exceeds 256 KiB"
    );
    ensure!(
        RUNNING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok(),
        keyed(
            "gui.diagnostics.previewRunning",
            "Another diagnostic preview is still running"
        )
    );
    let running = Running;
    let (sender, receiver) = mpsc::sync_channel(1);
    // Keep the slot until journal-child cleanup finishes even if filesystem I/O outlives the wait.
    std::thread::Builder::new()
        .name("diagnostic-export".into())
        .spawn(move || {
            let _running = running;
            let mut input = input;
            let source = match (saved.as_ref(), input.log_source) {
                (Some(_), _) => LogSource::SavedDaemonLog,
                (None, Some(LogSource::SessionWorker | LogSource::SavedDaemonLog)) => {
                    let _ = sender.send(Err(keyed(
                        "gui.diagnostics.selectDaemonLogs",
                        "Select daemon logs. Session-worker logs alone are insufficient",
                    )
                    .into()));
                    return;
                }
                (None, Some(source)) => source,
                (None, None) => {
                    if input.daemon.as_ref().is_some_and(|daemon| {
                        daemon.mode == lianli_shared::daemon::DaemonMode::System
                    }) {
                        LogSource::SystemDaemon
                    } else {
                        LogSource::UserDaemon
                    }
                }
            };
            input.log_source = Some(source);
            let logs = match saved {
                Some(path) => saved_logs(&path),
                None => {
                    lianli_control::diagnostic_logs::read(&InstallationContext::detect(), source)
                }
            };
            let can_save = usable_logs(&logs);
            let _ = sender.send(summary(&input, Some(logs)).map(|text| (text, can_save)));
        })?;
    let (text, can_save) = receiver
        .recv_timeout(Duration::from_secs(24))
        .map_err(|error| {
            UiError::new(
                UiMessage::new("gui.diagnostics.previewTimedOut").param("error", error),
                format!("Diagnostic preview timed out. Repair journal access and retry: {error}"),
            )
        })??;
    let preview = Preview {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        text,
        can_save,
    };
    *PREVIEW.lock().unwrap_or_else(|error| error.into_inner()) = Some(preview.clone());
    Ok(preview)
}

fn usable_logs(logs: &Result<Vec<String>>) -> bool {
    logs.as_ref().is_ok_and(|lines| {
        lines.iter().any(|line| {
            !line.trim().is_empty()
                && ![
                    "[redacted]",
                    "[path redacted]",
                    "[file redacted]",
                    "[identifier redacted]",
                ]
                .contains(&redact(line).as_str())
                && line != "[non-text journal message omitted]"
        })
    })
}

pub fn save(app: &tauri::AppHandle, id: u64) -> Result<bool> {
    let preview = PREVIEW
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
        .filter(|preview| preview.id == id)
        .cloned()
        .ok_or_else(|| {
            keyed(
                "gui.diagnostics.previewChanged",
                "The preview changed. Generate and review it again before saving",
            )
        })?;
    ensure!(
        preview.can_save,
        keyed("gui.diagnostics.logsRequired", "Daemon logs are required. Restore journal access and generate another preview before saving")
    );
    let Some(selected) = app
        .dialog()
        .file()
        .add_filter("JSON", &["json"])
        .set_file_name("lianli-diagnostics.json")
        .blocking_save_file()
    else {
        return Ok(false);
    };
    let path = selected
        .into_path()
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    save_to(&path, &preview.text)?;
    Ok(true)
}

fn save_to(path: &std::path::Path, text: &str) -> Result<()> {
    let parent = path.parent().context("No destination directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(text.as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .with_context(|| format!("Saving {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_logs_are_required_and_saved_logs_remain_bounded() {
        assert!(!usable_logs(&Ok(vec![])));
        assert!(!usable_logs(&Err(anyhow::anyhow!("unavailable"))));
        assert!(!usable_logs(&Ok(vec!["token=secret".into()])));
        assert!(usable_logs(&Ok(vec!["Daemon initialized".into()])));
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("daemon.log");
        std::fs::write(
            &path,
            (0..150)
                .map(|index| format!("message {index}\n"))
                .collect::<String>(),
        )
        .unwrap();
        let lines = saved_logs(&path).unwrap();
        assert_eq!(lines.len(), 100);
        assert_eq!(lines[0], "message 50");
        assert_eq!(lines[99], "message 149");
        assert!(saved_logs(root.path()).is_err());
        std::fs::File::create(&path)
            .unwrap()
            .set_len(1024 * 1024 + 1)
            .unwrap();
        assert!(saved_logs(&path).is_err());
    }

    #[test]
    fn redaction_preserves_controller_failures_and_retry_counts() {
        for line in [
            "Could not begin opening ENE 6K77 SL/AL Fan Controller (0cf2:a102): An earlier device open is still outstanding",
            "USB/HID node permissions",
            "Retrying 1 device(s) that failed to open (attempt 1/18)",
            "All wired devices opened successfully after 1 retry/retries",
            "lianli-daemon-system.service: load loaded, state active/running",
            "lianli-daemon.service: load loaded, state inactive/dead",
        ] {
            assert_eq!(redact(line), line);
        }
        assert_eq!(
            redact("SL/AL failed to read /home/Alice/private folder/config.json"),
            "SL/AL failed to read [path redacted]"
        );
        for path in [
            "SL/AL/private",
            "private/report",
            "1/18/private",
            "USB/HID/private",
        ] {
            assert_eq!(redact(path), "[path redacted]");
        }
    }

    #[test]
    fn redaction_removes_paths_credentials_and_capture_identifiers() {
        for (value, expected) in [
            (
                "Unable to read /home/Alice/private folder/image.png",
                "Unable to read [path redacted]",
            ),
            ("C:\\Users\\Alice\\file", "[path redacted]"),
            ("token=abc", "[redacted]"),
            ("SESSION_TOKEN=123", "[redacted]"),
            ("0123456789abcdef0123456789abcdef", "[identifier redacted]"),
            ("secret=hello", "[redacted]"),
            (
                "Could not open private-video.mp4",
                "Could not open [file redacted]",
            ),
            (
                "Capture rejected token=\"two word secret\"",
                "Capture rejected [redacted]",
            ),
            (
                "Device 0123456789abcdef0123456789abcdef disconnected",
                "Device [identifier redacted] disconnected",
            ),
            (
                "/dev/dri/renderD128: unsupported modifier",
                "/dev/dri/renderD128: unsupported modifier",
            ),
            ("/dev/dri/renderD128/private", "[path redacted]"),
        ] {
            assert_eq!(redact(value), expected);
        }
        assert_eq!(
            redact("Hermes-KMS unavailable. Trying EVDI"),
            "Hermes-KMS unavailable. Trying EVDI"
        );
        assert_eq!(
            redact("VAAPI initialization failed\n"),
            "VAAPI initialization failed"
        );
    }

    #[test]
    fn saving_uses_private_atomic_output_without_following_a_destination_symlink() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("original");
        let output = root.path().join("report.json");
        std::fs::write(&original, b"untouched").unwrap();
        symlink(&original, &output).unwrap();
        save_to(&output, "reviewed snapshot").unwrap();
        assert_eq!(std::fs::read_to_string(&original).unwrap(), "untouched");
        assert_eq!(
            std::fs::read_to_string(&output).unwrap(),
            "reviewed snapshot"
        );
        assert_eq!(
            std::fs::metadata(output).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn export_omits_private_context_and_reports_log_failure_without_losing_findings() {
        let input = Input {
            report: InstallationReport {
                daemon_context: Some(InstallationContext::Distrobox {
                    name: "private-daemon-box".into(),
                }),
                context: InstallationContext::Distrobox {
                    name: "private-box-name".into(),
                },
                checked_at_unix_ms: 1,
                findings: vec![],
                services: None,
            },
            daemon: None,
            log_source: Some(LogSource::SystemDaemon),
            media: Default::default(),
            desktop_streams: vec![lianli_shared::ipc::DesktopStreamStatus {
                bus: 1,
                address: 2,
                product_id: 0xad21,
                state: lianli_shared::ipc::DesktopStreamState::Failed,
                backend: Some("evdi".into()),
                fallback_reason: Some("Hermes unavailable".into()),
                error: Some("Unable to open /home/private/display".into()),
                error_ui: None,
                applied_generation: None,
                applied_policy: None,
                encoding: None,
                codecs: None,
            }],
        };
        let text = summary(&input, Some(Err(anyhow::anyhow!("Access denied")))).unwrap();
        assert!(!text.contains("private-box-name"));
        assert!(!text.contains("private-daemon-box"));
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["daemon_context"], "distrobox");
        assert_eq!(value["logs"]["unavailable"], "Access denied");
        assert!(value["findings"].is_array());
        assert!(value["daemon"].is_null());
        assert_eq!(value["desktop_streams"][0]["backend"], "evdi");
        assert_eq!(
            value["desktop_streams"][0]["fallback_reason"],
            "Hermes unavailable"
        );
        assert!(value["desktop_streams"][0]["applied_policy"].is_null());
        assert!(!text.contains("/home/private"));
    }
}
