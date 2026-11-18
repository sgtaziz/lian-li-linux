use super::DeviceKey;
use lianli_shared::display::DisplayVideoPolicy;
use lianli_shared::ipc::{DesktopStreamState, DesktopStreamStatus};
use lianli_shared::ui_message::UiText;
use parking_lot::Mutex;

pub(super) struct StreamStatus(Mutex<DesktopStreamStatus>);

impl StreamStatus {
    pub fn new((bus, address): DeviceKey, product_id: u16) -> Self {
        Self(Mutex::new(DesktopStreamStatus {
            bus,
            address,
            product_id,
            state: DesktopStreamState::WaitingForSession,
            backend: None,
            fallback_reason: None,
            error: None,
            error_ui: None,
            applied_generation: None,
            applied_policy: None,
            encoding: None,
            codecs: None,
        }))
    }

    pub fn codecs(&self, support: lianli_shared::display::DesktopCodecSupport) {
        self.0.lock().codecs = Some(support);
    }

    pub fn snapshot(&self) -> DesktopStreamStatus {
        self.0.lock().clone()
    }

    pub fn codec_support(&self) -> Option<lianli_shared::display::DesktopCodecSupport> {
        self.0.lock().codecs
    }

    pub fn waiting(&self) {
        self.0.lock().state = DesktopStreamState::WaitingForSession;
    }

    pub fn starting(&self) {
        let mut state = self.0.lock();
        state.state = DesktopStreamState::Starting;
        state.backend = None;
        state.fallback_reason = None;
        state.error = None;
        state.error_ui = None;
        state.applied_generation = None;
        state.applied_policy = None;
        state.encoding = None;
    }

    pub fn backend(&self, backend: &str, fallback: Option<&str>) {
        let mut state = self.0.lock();
        state.backend = Some(backend.chars().take(80).collect());
        state.fallback_reason = fallback.map(|reason| reason.chars().take(2048).collect());
    }

    pub fn applied(
        &self,
        generation: u64,
        policy: DisplayVideoPolicy,
        encoding: Option<lianli_shared::display::DesktopEncodingStatus>,
    ) {
        let mut state = self.0.lock();
        state.state = DesktopStreamState::Streaming;
        state.applied_generation = Some(generation);
        state.applied_policy = Some(policy);
        state.encoding = encoding;
    }

    pub fn paused(&self) {
        self.0.lock().state = DesktopStreamState::Paused;
    }

    pub fn failed(&self, reason: UiText) {
        let reason = reason.truncated(2048);
        let mut state = self.0.lock();
        state.state = DesktopStreamState::Failed;
        state.error = Some(reason.english);
        state.error_ui = reason.ui;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_telemetry_does_not_invent_a_desktop_status() {
        let frame: lianli_shared::display::CaptureReply =
            serde_json::from_value(serde_json::json!({
                "type": "frame", "sequence": 1, "generation": 1,
                "mode": {"width": 16, "height": 16, "refresh_hz": 30}, "bytes": 1
            }))
            .unwrap();
        assert!(matches!(
            frame,
            lianli_shared::display::CaptureReply::Frame { encoding: None, .. }
        ));
        let telemetry: lianli_shared::ipc::TelemetrySnapshot =
            serde_json::from_value(serde_json::json!({
                "fan_rpms": {}, "coolant_temps": {}, "streaming_active": false
            }))
            .unwrap();
        assert!(telemetry.desktop_streams.is_empty());
    }

    #[test]
    fn startup_does_not_claim_delivery_and_retries_clear_previous_application() {
        let status = StreamStatus::new((1, 2), 0xad21);
        status.starting();
        status.backend("evdi", Some("Hermes unavailable"));
        assert!(status.snapshot().applied_generation.is_none());
        let policy = DisplayVideoPolicy {
            hardware_video: true,
            fps_limit: 30,
        };
        status.applied(2, policy, None);
        status.paused();
        assert_eq!(status.snapshot().state, DesktopStreamState::Paused);
        assert_eq!(status.snapshot().applied_generation, Some(2));
        status.failed("界".repeat(3000).into());
        let failed = status.snapshot();
        assert_eq!(failed.state, DesktopStreamState::Failed);
        assert_eq!(failed.error.unwrap().chars().count(), 2048);
        assert_eq!(
            failed.fallback_reason.as_deref(),
            Some("Hermes unavailable")
        );
        status.starting();
        let restarted = status.snapshot();
        assert!(restarted.error.is_none());
        assert!(restarted.backend.is_none());
        assert!(restarted.applied_policy.is_none());
    }
}
