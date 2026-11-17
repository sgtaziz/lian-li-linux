use super::SharedState;
use lianli_shared::installation::RuntimeInstallationReport;
use lianli_shared::installation::{
    CheckState, FindingSeverity, InstallationFinding, InstallationGuide,
};
use lianli_shared::ipc::IpcResponse;
use lianli_shared::ui_message::UiMessage;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

static RUNNING: AtomicBool = AtomicBool::new(false);
static CACHE: Mutex<Option<(Instant, RuntimeInstallationReport)>> = Mutex::new(None);

struct Running;
impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::Release);
    }
}

fn needs_media_tools(state: &super::DaemonState) -> bool {
    use lianli_shared::media::MediaType;
    use lianli_shared::template::{TemplateBackground, WidgetKind};
    state.config.as_ref().is_some_and(|config| {
        config.lcds.iter().any(|lcd| {
            matches!(lcd.media_type, MediaType::Video | MediaType::Gif)
                || (lcd.media_type == MediaType::Custom
                    && state.user_templates.iter().any(|template| {
                        Some(template.id.as_str()) == lcd.template_id.as_deref()
                            && (matches!(template.background, TemplateBackground::Video { .. })
                                || template
                                    .widgets
                                    .iter()
                                    .any(|widget| matches!(widget.kind, WidgetKind::Video { .. })))
                    }))
        })
    }) || state
        .telemetry
        .media_preparation
        .values()
        .any(|preparation| {
            preparation.runtime.as_ref().is_some_and(|runtime| {
                runtime.encoder.is_some() || runtime.fallback_reason.is_some()
            })
        })
}

fn openrgb_finding(
    status: &lianli_shared::ipc::OpenRgbServerStatus,
) -> Option<InstallationFinding> {
    if !status.enabled {
        return None;
    }
    let (state, severity, evidence) = if let Some(error) = &status.error {
        (
            CheckState::Failed,
            FindingSeverity::Warning,
            error.chars().take(2048).collect(),
        )
    } else if status.running {
        (
            CheckState::Passed,
            FindingSeverity::Info,
            status.port.map_or_else(
                || "OpenRGB is listening. Its port was not reported".into(),
                |port| format!("OpenRGB is listening on port {port}"),
            ),
        )
    } else {
        (
            CheckState::Unavailable,
            FindingSeverity::Info,
            "OpenRGB is waiting for its server or RGB controller to become ready".into(),
        )
    };
    Some(InstallationFinding {
        code:"integration.openrgb".into(), state, severity, feature:"OpenRGB SDK server".into(),
        context:"Selected daemon".into(), title:"OpenRGB server".into(), evidence,
        remediation:"Free the configured TCP port or fix access in the daemon's network namespace. Use Retry OpenRGB in Settings to retry saved settings, or edit the port and Save. Recheck only reads status.".into(),
        guide:InstallationGuide::Troubleshooting,
        message_key: Some("daemon.openrgbServer".into()), message_params: BTreeMap::new(),
    })
}

pub(super) fn check(state: &SharedState) -> IpcResponse {
    let (instance_id, backend, media_tools, config_path) = {
        let state = state.lock();
        let media_tools = needs_media_tools(&state);
        (
            state.info.instance_id.clone(),
            state.runtime_hid_backend,
            media_tools,
            state.config_path.clone(),
        )
    };
    let Some(backend) = backend else {
        return IpcResponse::error_ui(
            "Runtime setup is still in progress. Recheck shortly",
            UiMessage::new("daemon.installation.setupInProgress"),
        );
    };
    if let Some((time, report)) = &*CACHE.lock().unwrap_or_else(|error| error.into_inner()) {
        if report.instance_id == instance_id && time.elapsed() < Duration::from_secs(2) {
            return IpcResponse::ok(report);
        }
    }
    if RUNNING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return IpcResponse::error_ui(
            "A runtime installation check is already running. Recheck shortly",
            UiMessage::new("daemon.installation.checkRunning"),
        );
    }
    let _running = Running;
    let result = (|| -> anyhow::Result<_> {
        let executable = std::env::current_exe()?.with_file_name("lianli-control");
        Ok(lianli_control::runtime_health::collect_report_with_media(
            &executable,
            Some(backend),
            media_tools,
            Some(
                config_path
                    .parent()
                    .filter(|path| !path.as_os_str().is_empty())
                    .unwrap_or(std::path::Path::new(".")),
            ),
        )?
        .findings)
    })();
    let mut findings = result.unwrap_or_else(|error| vec![InstallationFinding {
        code: "runtime.unavailable".into(), state: CheckState::Unavailable, severity: FindingSeverity::Warning,
        feature: "Runtime installation checks".into(), context: "Selected daemon".into(), title: "Runtime helper unavailable".into(),
        evidence: format!("{error:#}"), remediation: "Install or rebuild the matching lianli-control beside the daemon. Repair unavailable storage or account services, then Recheck.".into(), guide: InstallationGuide::Troubleshooting,
        message_key: Some("daemon.runtimeHelperUnavailable".into()), message_params: BTreeMap::new(),
    }]);
    {
        let state = state.lock();
        findings.extend(state.state_health.findings());
        findings.extend(openrgb_finding(&state.telemetry.openrgb_status));
    }
    findings.push(crate::controllers::cooling::finding());
    let report = RuntimeInstallationReport {
        instance_id,
        uid: unsafe { libc::geteuid() },
        context: Some(lianli_shared::installation::InstallationContext::detect()),
        findings,
    };
    *CACHE.lock().unwrap_or_else(|error| error.into_inner()) =
        Some((Instant::now(), report.clone()));
    IpcResponse::ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_shared::config::AppConfig;
    use lianli_shared::ipc::{
        MediaPreparationState, MediaPreparationStatus, MediaRuntimeStage, MediaRuntimeStatus,
    };

    #[test]
    fn openrgb_findings_ignore_disabled_servers_and_clear_after_recovery() {
        let mut status = lianli_shared::ipc::OpenRgbServerStatus {
            error: Some("Port in use".repeat(500)),
            ..Default::default()
        };
        assert!(openrgb_finding(&status).is_none());
        status.enabled = true;
        let finding = openrgb_finding(&status).unwrap();
        assert_eq!(finding.state, CheckState::Failed);
        assert_eq!(finding.severity, FindingSeverity::Warning);
        assert!(finding.evidence.len() <= 2048);
        status.error = None;
        assert_eq!(
            openrgb_finding(&status).unwrap().severity,
            FindingSeverity::Info
        );
        status.running = true;
        status.port = Some(7777);
        let finding = openrgb_finding(&status).unwrap();
        assert_eq!(finding.state, CheckState::Passed);
        assert!(finding.evidence.contains("7777"));
    }

    #[test]
    fn media_checks_follow_selected_template_children_and_live_encoder_state() {
        let mut state = super::super::DaemonState::new("unused-config.json".into());
        state.config = Some(AppConfig::default());
        state.user_templates.push(
            serde_json::from_value(serde_json::json!({
                "id":"video-template", "name":"Video", "base_width":100, "base_height":100,
                "background":{"type":"color","rgb":[0,0,0,255]},
                "widgets":[{"id":"clip", "kind":{"type":"video","path":"clip.mp4"},
                    "x":50,"y":50,"width":100,"height":100}]
            }))
            .unwrap(),
        );
        assert!(!needs_media_tools(&state));
        state.config.as_mut().unwrap().lcds.push(
            serde_json::from_value(serde_json::json!({
                "index":0,"type":"custom","template_id":"different"
            }))
            .unwrap(),
        );
        assert!(!needs_media_tools(&state));
        state.config.as_mut().unwrap().lcds[0].template_id = Some("video-template".into());
        assert!(needs_media_tools(&state));
        state.user_templates[0].widgets.clear();
        assert!(!needs_media_tools(&state));
        state.telemetry.media_preparation.insert(
            0,
            MediaPreparationStatus {
                startup_recovery_required: false,
                generation: 1,
                device_id: "test".into(),
                state: MediaPreparationState::Ready,
                error: None,
                error_ui: None,
                last_playback_error: None,
                last_playback_error_ui: None,
                runtime: Some(MediaRuntimeStatus {
                    stage: MediaRuntimeStage::FrameSubmitted,
                    fps_limit: 20.0,
                    hardware_video_allowed: false,
                    fallback_reason: Some("H.264 encoder unavailable".into()),
                    fallback_reason_ui: None,
                    encoder: None,
                    h264_transfer_started: None,
                }),
            },
        );
        assert!(needs_media_tools(&state));
        state.telemetry.media_preparation.clear();
        assert!(!needs_media_tools(&state));
        state.config.as_mut().unwrap().lcds[0].media_type = lianli_shared::media::MediaType::Gif;
        assert!(needs_media_tools(&state));
    }
}
