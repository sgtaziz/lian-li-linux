//! RGB controller: manages LED effects for all RGB-capable devices.
//!
//! Coordinates between native config effects and OpenRGB overrides.
//! Wired devices use the `RgbDevice` trait. Wireless devices stream
//! compressed per-LED frames via the `WirelessController`.

use lianli_devices::traits::RgbDevice;
use lianli_devices::wireless::{WirelessController, WirelessFanType};
use lianli_shared::rgb::{RgbAppConfig, RgbDeviceCapabilities, RgbEffect, RgbMode, RgbZoneInfo};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tracing::{debug, info, warn};

/// Tracks a wireless device's RGB state for `send_rgb_direct`.
struct WirelessRgbState {
    mac: [u8; 6],
    fan_count: u8,
    leds_per_fan: u8,
    fan_type: WirelessFanType,
    /// Per-LED color buffer — the full device LED state.
    /// Updated per-zone, then the whole buffer is sent via RF.
    led_state: Vec<[u8; 3]>,
    /// Monotonically increasing effect index (4 bytes, sent in RF header).
    effect_counter: u32,
}

impl WirelessRgbState {
    fn new(mac: [u8; 6], fan_count: u8, fan_type: WirelessFanType) -> Self {
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

pub struct RgbController {
    /// Wired RGB devices keyed by device_id.
    wired: HashMap<String, Box<dyn RgbDevice>>,
    /// Wireless controller for RF-based LED control.
    wireless: Option<Arc<WirelessController>>,
    /// Wireless device state keyed by device_id ("wireless:xx:xx:xx:xx:xx:xx").
    wireless_state: HashMap<String, WirelessRgbState>,
    /// Current RGB config (from AppConfig).
    config: Option<RgbAppConfig>,
    /// When true, OpenRGB has active control — suppress native config application.
    openrgb_active: bool,
}

impl RgbController {
    pub fn new(
        wired: HashMap<String, Box<dyn RgbDevice>>,
        wireless: Option<Arc<WirelessController>>,
    ) -> Self {
        let mut wireless_state = HashMap::new();

        // Build wireless state from discovered devices
        if let Some(ref w) = wireless {
            for dev in w.devices() {
                let device_id = format!("wireless:{}", dev.mac_str());
                wireless_state.insert(
                    device_id,
                    WirelessRgbState::new(dev.mac, dev.fan_count, dev.fan_type),
                );
            }
        }

        info!(
            "RGB controller: {} wired device(s), {} wireless device(s)",
            wired.len(),
            wireless_state.len()
        );

        Self {
            wired,
            wireless,
            wireless_state,
            config: None,
            openrgb_active: false,
        }
    }

    /// Apply an RGB config. Called on config load/change.
    pub fn apply_config(&mut self, config: &RgbAppConfig) {
        self.config = Some(config.clone());

        if !config.enabled {
            info!("RGB control disabled in config");
            return;
        }

        if config.openrgb_server {
            debug!("Skipping native RGB config — OpenRGB server is enabled");
            return;
        }

        if self.openrgb_active {
            debug!("Skipping native RGB config — OpenRGB has active control");
            return;
        }

        for dev_cfg in &config.devices {
            for zone_cfg in &dev_cfg.zones {
                if let Err(e) =
                    self.set_effect(&dev_cfg.device_id, zone_cfg.zone_index, &zone_cfg.effect)
                {
                    warn!(
                        "Failed to apply RGB effect to {} zone {}: {e}",
                        dev_cfg.device_id, zone_cfg.zone_index
                    );
                }
                // Apply fan direction if the device supports it
                if zone_cfg.swap_lr || zone_cfg.swap_tb {
                    if let Err(e) = self.set_fan_direction(
                        &dev_cfg.device_id,
                        zone_cfg.zone_index,
                        zone_cfg.swap_lr,
                        zone_cfg.swap_tb,
                    ) {
                        warn!(
                            "Failed to apply fan direction to {} zone {}: {e}",
                            dev_cfg.device_id, zone_cfg.zone_index
                        );
                    }
                }
            }
        }
    }

    /// Set an effect on a specific device zone.
    pub fn set_effect(
        &mut self,
        device_id: &str,
        zone: u8,
        effect: &RgbEffect,
    ) -> anyhow::Result<()> {
        // Try wired first
        if let Some(dev) = self.wired.get(device_id) {
            dev.set_zone_effect(zone, effect)?;
            debug!("Set RGB effect on {device_id} zone {zone}: {:?}", effect.mode);
            return Ok(());
        }

        // Try wireless — update only the target zone's LEDs, then send the full buffer
        if let (Some(ref wireless), Some(state)) =
            (&self.wireless, self.wireless_state.get_mut(device_id))
        {
            let lpf = state.leds_per_fan as usize;
            let zone_idx = zone as usize;

            if zone_idx >= state.fan_count as usize {
                anyhow::bail!(
                    "Zone {zone} out of range (device has {} fans)",
                    state.fan_count
                );
            }

            // Render this zone's LED colors
            let zone_color = render_zone_color(effect, lpf);

            // Update only this zone's slice in the full LED state buffer
            let start = zone_idx * lpf;
            let end = start + lpf;
            state.led_state[start..end].copy_from_slice(&zone_color);

            // Send the full device LED state
            state.effect_counter = state.effect_counter.wrapping_add(1);
            let idx = state.effect_counter.to_be_bytes();

            wireless.send_rgb_direct(&state.mac, &state.led_state, &idx, 4)?;
            debug!(
                "Set wireless RGB on {device_id} zone {zone}: {:?}, {} LEDs/fan",
                effect.mode, lpf
            );
            return Ok(());
        }

        anyhow::bail!("RGB device not found: {device_id}");
    }

    /// Set per-LED colors directly (used by OpenRGB `UpdateLEDs`).
    pub fn set_direct_colors(
        &mut self,
        device_id: &str,
        zone: u8,
        colors: &[[u8; 3]],
    ) -> anyhow::Result<()> {
        // Try wired
        if let Some(dev) = self.wired.get(device_id) {
            dev.set_direct_colors(zone, colors)?;
            return Ok(());
        }

        // Try wireless — update zone's LEDs and send full buffer
        if let (Some(ref wireless), Some(state)) =
            (&self.wireless, self.wireless_state.get_mut(device_id))
        {
            let lpf = state.leds_per_fan as usize;
            let zone_idx = zone as usize;

            if zone_idx >= state.fan_count as usize {
                anyhow::bail!(
                    "Zone {zone} out of range (device has {} fans)",
                    state.fan_count
                );
            }

            let start = zone_idx * lpf;
            let copy_len = colors.len().min(lpf);
            state.led_state[start..start + copy_len].copy_from_slice(&colors[..copy_len]);

            state.effect_counter = state.effect_counter.wrapping_add(1);
            let idx = state.effect_counter.to_be_bytes();
            wireless.send_rgb_direct(&state.mac, &state.led_state, &idx, 2)?;
            return Ok(());
        }

        anyhow::bail!("RGB device not found: {device_id}");
    }

    /// Return RGB capabilities for all connected devices.
    pub fn capabilities(&self) -> Vec<RgbDeviceCapabilities> {
        let mut caps = Vec::new();

        // Wired devices
        for (device_id, dev) in &self.wired {
            caps.push(RgbDeviceCapabilities {
                device_id: device_id.clone(),
                device_name: dev.device_name(),
                supported_modes: dev.supported_modes(),
                zones: dev.zone_info(),
                supports_direct: dev.supports_direct(),
                supports_mb_rgb_sync: dev.supports_mb_rgb_sync(),
                total_led_count: dev.total_led_count(),
                supported_scopes: dev.supported_scopes(),
                supports_direction: dev.supports_direction(),
            });
        }

        // Wireless devices
        for (device_id, state) in &self.wireless_state {
            let mut zones: Vec<RgbZoneInfo> = Vec::new();

            if let Some(total) = state.fan_type.total_led_count_override() {
                // RGB-only devices: single zone
                let zone_name = match state.fan_type {
                    WirelessFanType::Lc217 => "Case Ring",
                    WirelessFanType::Led88 => "Screen Ring",
                    _ => "LED Strip",
                };
                zones.push(RgbZoneInfo {
                    name: zone_name.to_string(),
                    led_count: total,
                });
            } else {
                // AIO: pump head zone first
                if state.fan_type.is_aio() {
                    zones.push(RgbZoneInfo {
                        name: "Pump Head".to_string(),
                        led_count: state.fan_type.pump_led_count() as u16,
                    });
                }
                // Fan zones
                zones.extend((0..state.fan_count).map(|i| RgbZoneInfo {
                    name: format!("Fan {}", i + 1),
                    led_count: state.leds_per_fan as u16,
                }));
            }

            let total_leds: u16 = zones.iter().map(|z| z.led_count).sum();

            caps.push(RgbDeviceCapabilities {
                device_id: device_id.clone(),
                device_name: state.fan_type.display_name().to_string(),
                supported_modes: vec![RgbMode::Static, RgbMode::Direct],
                zones,
                supports_direct: true,
                supports_mb_rgb_sync: false,
                total_led_count: total_leds,
                supported_scopes: vec![],
                supports_direction: false,
            });
        }

        caps
    }

    /// Enable or disable motherboard ARGB sync for a device.
    pub fn set_mb_rgb_sync(
        &self,
        device_id: &str,
        enabled: bool,
    ) -> anyhow::Result<()> {
        if let Some(dev) = self.wired.get(device_id) {
            if !dev.supports_mb_rgb_sync() {
                anyhow::bail!("Device {device_id} does not support MB RGB sync");
            }
            dev.set_mb_rgb_sync(enabled)?;
            info!("MB RGB sync {}: {device_id}", if enabled { "enabled" } else { "disabled" });
            return Ok(());
        }
        anyhow::bail!("RGB device not found: {device_id}");
    }

    /// Set fan direction (swap LR/TB) for a specific device zone.
    pub fn set_fan_direction(
        &self,
        device_id: &str,
        zone: u8,
        swap_lr: bool,
        swap_tb: bool,
    ) -> anyhow::Result<()> {
        if let Some(dev) = self.wired.get(device_id) {
            if !dev.supports_direction() {
                anyhow::bail!("Device {device_id} does not support fan direction");
            }
            dev.set_fan_direction(zone, swap_lr, swap_tb)?;
            debug!("Set fan direction on {device_id} zone {zone}: swap_lr={swap_lr} swap_tb={swap_tb}");
            return Ok(());
        }
        anyhow::bail!("RGB device not found: {device_id}");
    }

    /// Called when OpenRGB connects — suppress native config.
    pub fn set_openrgb_active(&mut self, active: bool) {
        if self.openrgb_active != active {
            self.openrgb_active = active;
            if active {
                info!("OpenRGB took control — suppressing native RGB config");
            } else {
                info!("OpenRGB released control");
                // Only restore native config if the OpenRGB server is disabled;
                // when the server is enabled, leave LEDs as-is so OpenRGB state persists.
                let server_enabled = self
                    .config
                    .as_ref()
                    .map(|c| c.openrgb_server)
                    .unwrap_or(false);
                if !server_enabled {
                    info!("Restoring native RGB config");
                    if let Some(config) = self.config.clone() {
                        self.apply_config(&config);
                    }
                }
            }
        }
    }

    /// Check if a device_id refers to a wireless device.
    pub fn is_wireless(&self, device_id: &str) -> bool {
        self.wireless_state.contains_key(device_id)
    }

    /// Refresh wireless device list (call after rediscovery / hot-plug).
    #[allow(dead_code)]
    pub fn refresh_wireless_devices(&mut self) {
        if let Some(ref w) = self.wireless {
            let mut new_state = HashMap::new();
            for dev in w.devices() {
                let device_id = format!("wireless:{}", dev.mac_str());
                // Preserve existing LED state and effect counter if device was already known
                let (counter, led_state) = self
                    .wireless_state
                    .get(&device_id)
                    .map(|s| (s.effect_counter, Some(s.led_state.clone())))
                    .unwrap_or((0, None));

                let mut state = WirelessRgbState::new(dev.mac, dev.fan_count, dev.fan_type);
                state.effect_counter = counter;
                if let Some(leds) = led_state {
                    if leds.len() == state.led_state.len() {
                        state.led_state = leds;
                    }
                }

                new_state.insert(device_id, state);
            }
            self.wireless_state = new_state;
        }
    }
}

/// Render a solid color array for a single zone from an RgbEffect.
fn render_zone_color(effect: &RgbEffect, led_count: usize) -> Vec<[u8; 3]> {
    let color = match effect.mode {
        RgbMode::Off => [0, 0, 0],
        _ => {
            let base = effect.colors.first().copied().unwrap_or([255, 255, 255]);
            // Apply brightness scaling (0-4 scale, 4 = full brightness)
            let scale = (effect.brightness as f32 / 4.0).clamp(0.0, 1.0);
            [
                (base[0] as f32 * scale) as u8,
                (base[1] as f32 * scale) as u8,
                (base[2] as f32 * scale) as u8,
            ]
        }
    };
    vec![color; led_count]
}

/// Buffers per-device, per-zone direct color updates for async flushing.
///
/// The OpenRGB TCP handler writes latest colors here (fast, no device I/O).
/// A writer thread flushes dirty devices at ~30fps, dropping intermediate frames.
pub struct DirectColorBuffer {
    pending: HashMap<String, HashMap<u8, Vec<[u8; 3]>>>,
}

impl DirectColorBuffer {
    pub fn new() -> Self {
        Self {
            pending: HashMap::new(),
        }
    }

    /// Store colors for a device zone (overwrites any previous pending value).
    pub fn set(&mut self, device_id: String, zone: u8, colors: Vec<[u8; 3]>) {
        self.pending.entry(device_id).or_default().insert(zone, colors);
    }

    /// Take all pending updates, clearing the buffer.
    pub fn take_all(&mut self) -> HashMap<String, HashMap<u8, Vec<[u8; 3]>>> {
        std::mem::take(&mut self.pending)
    }
}

/// Spawns a background thread that flushes buffered direct colors.
///
/// Wired devices are processed first for lowest latency.
/// Wireless devices use single-frame direct sends.
pub fn start_direct_color_writer(
    rgb: Arc<Mutex<RgbController>>,
    buffer: Arc<Mutex<DirectColorBuffer>>,
    stop_flag: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        debug!("Direct color writer started");

        loop {
            if stop_flag.load(Ordering::Relaxed) {
                break;
            }

            let updates = buffer.lock().take_all();

            if !updates.is_empty() {
                // Split into wired and wireless so wired always goes first
                let mut wired = Vec::new();
                let mut wireless = Vec::new();
                {
                    let rgb = rgb.lock();
                    for (device_id, zones) in updates {
                        if rgb.is_wireless(&device_id) {
                            wireless.push((device_id, zones));
                        } else {
                            wired.push((device_id, zones));
                        }
                    }
                }

                // Wired: flush immediately with minimal lock time
                if !wired.is_empty() {
                    let mut rgb = rgb.lock();
                    for (device_id, zones) in wired {
                        for (zone, colors) in zones {
                            if let Err(e) = rgb.set_direct_colors(&device_id, zone, &colors) {
                                debug!("Wired flush error for {device_id} zone {zone}: {e}");
                            }
                        }
                    }
                }

                // Wireless: send latest color state per device
                if !wireless.is_empty() {
                    let mut rgb = rgb.lock();
                    for (device_id, zones) in wireless {
                        for (zone, colors) in zones {
                            if let Err(e) = rgb.set_direct_colors(&device_id, zone, &colors) {
                                debug!("Wireless flush error for {device_id} zone {zone}: {e}");
                            }
                        }
                    }
                }
            } else {
                thread::sleep(Duration::from_millis(5));
            }
        }

        debug!("Direct color writer stopped");
    })
}
