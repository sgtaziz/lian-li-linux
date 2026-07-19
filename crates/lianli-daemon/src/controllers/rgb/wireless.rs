use lianli_devices::wireless::WirelessFanType;

/// Tracks a wireless device's RGB state for `send_rgb_direct`.
pub(super) struct WirelessRgbState {
    pub(super) mac: [u8; 6],
    pub(super) fan_count: u8,
    pub(super) leds_per_fan: u8,
    pub(super) fan_type: WirelessFanType,
    /// Per-LED color buffer — the full device LED state.
    /// Updated per-zone, then the whole buffer is sent via RF.
    pub(super) led_state: Vec<[u8; 3]>,
    /// Monotonically increasing effect index (4 bytes, sent in RF header).
    pub(super) effect_counter: u32,
}

impl WirelessRgbState {
    pub(super) fn new(mac: [u8; 6], fan_count: u8, fan_type: WirelessFanType) -> Self {
        let leds_per_fan = fan_type.leds_per_fan();
        let total_leds = if let Some(override_count) = fan_type.total_led_count_override() {
            override_count as usize
        } else {
            let pump_leds = fan_type.pump_led_count() as usize;
            pump_leds + fan_count as usize * leds_per_fan as usize
        };
        Self {
            mac,
            fan_count,
            leds_per_fan,
            fan_type,
            led_state: vec![[0, 0, 0]; total_leds],
            effect_counter: 0,
        }
    }
}
