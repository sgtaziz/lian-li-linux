//! RGB controller: manages LED effects for all RGB-capable devices.
//!
//! Coordinates between native config effects and OpenRGB overrides.
//! Wired devices use the `RgbDevice` trait. Wireless devices stream
//! compressed per-LED frames via the `WirelessController`.

mod direct_color;
mod wireless;

pub use direct_color::{start_direct_color_writer, DirectColorBuffer};

use lianli_devices::traits::RgbDevice;
use lianli_devices::wireless::{WirelessController, WirelessFanType};
use lianli_shared::rgb::{
    RgbAppConfig, RgbDeviceCapabilities, RgbEffect, RgbMode, RgbPresetZone, RgbZoneInfo,
};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info, warn};
use wireless::WirelessRgbState;

pub struct RgbController {
    /// Wired RGB devices keyed by device_id.
    wired: HashMap<String, Box<dyn RgbDevice>>,
    /// Wireless controller for RF-based LED control.
    wireless: Option<Arc<WirelessController>>,
    /// Wireless device state keyed by device_id ("wireless:xx:xx:xx:xx:xx:xx").
    wireless_state: HashMap<String, WirelessRgbState>,
    /// Current RGB config (from AppConfig).
    config: Option<RgbAppConfig>,
    /// Cached presets for restoring active preset LED colors.
    presets: Vec<lianli_shared::rgb::RgbPreset>,
    /// When true, OpenRGB has active control — suppress native config application.
    openrgb_active: bool,
}

impl RgbController {
    pub fn new(
        wired: HashMap<String, Box<dyn RgbDevice>>,
        wireless: Option<Arc<WirelessController>>,
    ) -> Self {
        let mut wireless_state = HashMap::new();

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
            presets: Vec::new(),
            openrgb_active: false,
        }
    }

    /// Apply an RGB config. Called on config load/change.
    pub fn apply_config(
        &mut self,
        config: &RgbAppConfig,
        presets: &[lianli_shared::rgb::RgbPreset],
    ) {
        self.config = Some(config.clone());
        self.presets = presets.to_vec();

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

            if let Some(ref preset_name) = dev_cfg.active_preset {
                if let Some(preset) = presets
                    .iter()
                    .find(|p| &p.name == preset_name && p.device_id == dev_cfg.device_id)
                {
                    for zone_entry in &preset.zones {
                        if !zone_entry.colors.is_empty() {
                            if let Err(e) = self.set_direct_colors(
                                &dev_cfg.device_id,
                                zone_entry.zone,
                                &zone_entry.colors,
                            ) {
                                warn!(
                                    "Failed to restore preset '{}' zone {}: {e}",
                                    preset_name, zone_entry.zone
                                );
                            }
                        }
                    }
                    debug!(
                        "Restored active preset '{}' for {}",
                        preset_name, dev_cfg.device_id
                    );
                }
            }
        }
    }

    pub fn set_effect(
        &mut self,
        device_id: &str,
        zone: u8,
        effect: &RgbEffect,
    ) -> anyhow::Result<()> {
        if let Some(dev) = self.wired.get(device_id) {
            dev.set_zone_effect(zone, effect)?;
            debug!(
                "Set RGB effect on {device_id} zone {zone}: {:?}",
                effect.mode
            );
            return Ok(());
        }

        if let (Some(ref wireless), Some(state)) =
            (&self.wireless, self.wireless_state.get_mut(device_id))
        {
            let zone_idx = zone as usize;
            let total_zones = if state.fan_type.is_aio() {
                state.fan_count as usize + 1
            } else if state.fan_type.is_rgb_only() {
                1
            } else {
                state.fan_count as usize
            };

            if zone_idx >= total_zones {
                anyhow::bail!(
                    "Zone {zone} out of range (device has {total_zones} zones, fan_type={:?}, fan_count={})", state.fan_type, state.fan_count
                );
            }

            let leds_in_zone = if state.fan_type.is_rgb_only() {
                state.led_state.len()
            } else {
                state.leds_per_fan as usize
            };

            let zone_color = render_zone_color(effect, leds_in_zone);

            let start = zone_idx * leds_in_zone;
            let end = start + leds_in_zone;
            state.led_state[start..end].copy_from_slice(&zone_color);

            let idx = effect_index_from_state(&state.led_state);

            wireless.send_rgb_direct(&state.mac, &state.led_state, &idx, 4)?;
            debug!(
                "Set wireless RGB on {device_id} zone {zone}: {:?}, {} LEDs/zone",
                effect.mode, leds_in_zone
            );
            return Ok(());
        }

        anyhow::bail!("RGB device not found: {device_id}");
    }

    pub fn set_direct_colors(
        &mut self,
        device_id: &str,
        zone: u8,
        colors: &[[u8; 3]],
    ) -> anyhow::Result<()> {
        if let Some(dev) = self.wired.get(device_id) {
            dev.set_direct_colors(zone, colors)?;
            return Ok(());
        }

        if let (Some(ref wireless), Some(state)) =
            (&self.wireless, self.wireless_state.get_mut(device_id))
        {
            let zone_idx = zone as usize;
            let total_zones = if state.fan_type.is_aio() {
                state.fan_count as usize + 1
            } else if state.fan_type.is_rgb_only() {
                1
            } else {
                state.fan_count as usize
            };

            if zone_idx >= total_zones {
                anyhow::bail!(
                    "Zone {zone} out of range (device has {total_zones} zones, fan_type={:?}, fan_count={})", state.fan_type, state.fan_count
                );
            }

            let leds_in_zone = if state.fan_type.is_rgb_only() {
                state.led_state.len()
            } else {
                state.leds_per_fan as usize
            };

            let start = zone_idx * leds_in_zone;
            let copy_len = colors.len().min(leds_in_zone);
            state.led_state[start..start + copy_len].copy_from_slice(&colors[..copy_len]);

            let idx = effect_index_from_state(&state.led_state);
            wireless.send_rgb_direct(&state.mac, &state.led_state, &idx, 2)?;
            return Ok(());
        }

        anyhow::bail!("RGB device not found: {device_id}");
    }

    /// Upload a multi-frame animation to a wireless device via
    /// `send_rgb_frames`. Each frame must be a full-device LED buffer; the
    /// firmware then loops the animation onboard at `interval_ms` with no
    /// further host packets. The effect_index is hashed across all frames so
    /// the wireless drift check treats the running animation as the desired
    /// state; `led_state` is parked on the final frame for a consistent later
    /// single-frame re-send.
    ///
    /// Note: only the final frame is cached in `led_state`, so a later
    /// `refresh_wireless_devices()` reconstructs a static image, not the
    /// animation. The device also doesn't persist the effect (only bindings do),
    /// so callers are expected to re-`SetRgbFrames` on reconnect to keep an
    /// animation running.
    pub fn set_rgb_frames(
        &mut self,
        device_id: &str,
        frames: &[Vec<[u8; 3]>],
        interval_ms: u16,
    ) -> anyhow::Result<()> {
        if frames.is_empty() {
            anyhow::bail!("no frames supplied");
        }
        if let (Some(ref wireless), Some(state)) =
            (&self.wireless, self.wireless_state.get_mut(device_id))
        {
            let want = state.led_state.len();
            for (i, f) in frames.iter().enumerate() {
                if f.len() != want {
                    anyhow::bail!("frame {i} has {} LEDs, device has {want}", f.len());
                }
            }
            let idx = effect_index_from_frames(frames);
            wireless.send_rgb_frames(&state.mac, frames, interval_ms, &idx, 4)?;
            state.led_state.copy_from_slice(&frames[frames.len() - 1]);
            return Ok(());
        }
        anyhow::bail!("wireless RGB device not found: {device_id}");
    }

    pub fn capabilities(&self) -> Vec<RgbDeviceCapabilities> {
        let mut caps = Vec::new();

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

        for (device_id, state) in &self.wireless_state {
            let mut zones: Vec<RgbZoneInfo> = Vec::new();

            if let Some(total) = state.fan_type.total_led_count_override() {
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
                if state.fan_type.is_aio() {
                    zones.push(RgbZoneInfo {
                        name: "Pump Head".to_string(),
                        led_count: state.fan_type.pump_led_count() as u16,
                    });
                }
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

    pub fn set_mb_rgb_sync(&self, device_id: &str, enabled: bool) -> anyhow::Result<()> {
        if let Some(dev) = self.wired.get(device_id) {
            if !dev.supports_mb_rgb_sync() {
                anyhow::bail!("Device {device_id} does not support MB RGB sync");
            }
            dev.set_mb_rgb_sync(enabled)?;
            info!(
                "MB RGB sync {}: {device_id}",
                if enabled { "enabled" } else { "disabled" }
            );
            return Ok(());
        }
        anyhow::bail!("RGB device not found: {device_id}");
    }

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
            debug!(
                "Set fan direction on {device_id} zone {zone}: swap_lr={swap_lr} swap_tb={swap_tb}"
            );
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
                        let presets = self.presets.clone();
                        self.apply_config(&config, &presets);
                    }
                }
            }
        }
    }

    /// Compute zone count and LEDs-per-zone for a wireless device state.
    /// Override-based devices (V150, Strimer, LC217, Led88) are single-zone
    /// with all LEDs in one flat buffer.
    fn zone_layout(state: &WirelessRgbState) -> (usize, usize) {
        if state.fan_type.total_led_count_override().is_some() {
            return (1, state.led_state.len());
        }
        let total_zones = if state.fan_type.is_aio() {
            state.fan_count as usize + 1
        } else {
            state.fan_count as usize
        };
        (total_zones, state.leds_per_fan as usize)
    }

    pub fn get_zone_colors(&self, device_id: &str, zone: u8) -> Option<Vec<[u8; 3]>> {
        let state = self.wireless_state.get(device_id)?;
        let (_, leds_in_zone) = Self::zone_layout(state);
        let start = zone as usize * leds_in_zone;
        let end = (start + leds_in_zone).min(state.led_state.len());
        if start >= state.led_state.len() {
            return None;
        }
        Some(state.led_state[start..end].to_vec())
    }

    pub fn get_all_zone_colors(&self, device_id: &str) -> Option<Vec<RgbPresetZone>> {
        let state = self.wireless_state.get(device_id)?;
        let (total_zones, leds_in_zone) = Self::zone_layout(state);
        let mut zones = Vec::new();
        for z in 0..total_zones {
            let start = z * leds_in_zone;
            let end = (start + leds_in_zone).min(state.led_state.len());
            if start < state.led_state.len() {
                zones.push(RgbPresetZone {
                    zone: z as u8,
                    colors: state.led_state[start..end].to_vec(),
                    effect: None,
                });
            }
        }
        Some(zones)
    }

    pub fn is_wireless(&self, device_id: &str) -> bool {
        self.wireless_state.contains_key(device_id)
    }

    pub fn set_wireless(&mut self, wireless: Option<Arc<WirelessController>>) {
        self.wireless = wireless;
    }

    pub fn refresh_wireless_devices(&mut self) {
        if let Some(ref w) = self.wireless {
            let mut new_state = HashMap::new();
            for dev in w.devices() {
                let device_id = format!("wireless:{}", dev.mac_str());
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

fn effect_index_from_state(led_state: &[[u8; 3]]) -> [u8; 4] {
    let mut h: u32 = 0x811c_9dc5;
    for px in led_state {
        for &b in px {
            h ^= b as u32;
            h = h.wrapping_mul(0x0100_0193);
        }
    }
    if h == 0 {
        h = 1;
    }
    h.to_be_bytes()
}

/// Same FNV-1a as `effect_index_from_state`, but over a frame sequence without
/// flattening it into one buffer first (a multi-frame animation can be large).
/// The byte order is identical to hashing the concatenated frames.
fn effect_index_from_frames(frames: &[Vec<[u8; 3]>]) -> [u8; 4] {
    let mut h: u32 = 0x811c_9dc5;
    for frame in frames {
        for px in frame {
            for &b in px {
                h ^= b as u32;
                h = h.wrapping_mul(0x0100_0193);
            }
        }
    }
    if h == 0 {
        h = 1;
    }
    h.to_be_bytes()
}

/// Render a solid color array for a single zone from an RgbEffect.
fn render_zone_color(effect: &RgbEffect, led_count: usize) -> Vec<[u8; 3]> {
    let color = match effect.mode {
        RgbMode::Off => [0, 0, 0],
        _ => {
            let base = effect.colors.first().copied().unwrap_or([255, 255, 255]);
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
