//! RGB effects, software rendering, and OpenRGB ownership.
mod capabilities;
mod configuration;
mod control;
mod direct_color;
mod playback;
mod regions;
mod render;
mod strimer_sync;
mod sync_clock;
mod sync_device;
mod sync_plan;
#[cfg(test)]
mod sync_tests;
mod synchronization;
mod upload;
mod wired;

pub use direct_color::{start_direct_color_writer, DirectColorBuffer};

use lianli_devices::traits::RgbDevice;
use lianli_devices::wireless::{WirelessController, WirelessFanType, WirelessRgbUpload};
use lianli_media::rgb::FRAME_INTERVAL_MS;
use lianli_shared::rgb::{
    RgbAppConfig, RgbDeviceCapabilities, RgbEffect, RgbMode, RgbPreset, RgbPresetZone, RgbZoneInfo,
};
use render::RenderState;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, warn};
use upload::{Command, UploadWorker};
use wired::WiredRenderer;

struct WirelessDevice {
    mac: [u8; 6],
    fan_count: u8,
    fan_type: WirelessFanType,
    right_attach: bool,
}

pub struct RgbController {
    wired: HashMap<String, Arc<dyn RgbDevice>>,
    wireless: Option<Arc<WirelessController>>,
    wireless_state: HashMap<String, WirelessDevice>,
    rendered: HashMap<String, RenderState>,
    applied: HashMap<String, RenderState>,
    uploads: HashMap<String, Arc<WirelessRgbUpload>>,
    upload_worker: UploadWorker,
    wired_renderer: WiredRenderer,
    sync_clock: sync_clock::SyncClockWorker,
    sync_signature: Option<String>,
    sync_active: std::collections::HashSet<String>,
    config: Option<RgbAppConfig>,
    presets: Vec<RgbPreset>,
    openrgb_active: bool,
    openrgb_server_enabled: bool,
    thermal_override: crate::thermal_alert::SharedThermalAlert,
    thermal_last_color: Option<[u8; 3]>,
    last_direct: HashMap<(String, u8), Vec<[u8; 3]>>,
    mb_sync_state: HashMap<String, bool>,
}

impl RgbController {
    pub fn new(
        wired: HashMap<String, Arc<dyn RgbDevice>>,
        wireless: Option<Arc<WirelessController>>,
    ) -> Self {
        let mut controller = Self {
            wired,
            wireless,
            wireless_state: HashMap::new(),
            rendered: HashMap::new(),
            applied: HashMap::new(),
            uploads: HashMap::new(),
            upload_worker: UploadWorker::new(),
            wired_renderer: WiredRenderer::new(),
            sync_clock: sync_clock::SyncClockWorker::new(),
            sync_signature: None,
            sync_active: Default::default(),
            config: None,
            presets: Vec::new(),
            openrgb_active: false,
            openrgb_server_enabled: false,
            thermal_override: crate::thermal_alert::new_shared(),
            thermal_last_color: None,
            last_direct: HashMap::new(),
            mb_sync_state: HashMap::new(),
        };
        controller.refresh_wireless_devices();
        controller
    }

    pub fn stop(&mut self) {
        self.sync_clock.stop();
        self.upload_worker.stop();
        self.wired_renderer.stop();
    }

    fn clear_pending(&mut self) {
        self.sync_clock.clear();
        self.sync_signature = None;
        self.sync_active.clear();
        self.upload_worker.clear();
        self.uploads.clear();
        if let Some(wireless) = &self.wireless {
            wireless.clear_rgb_targets();
        }
        self.wired_renderer.clear();
        self.applied.clear();
        self.thermal_last_color = None;
    }

    pub fn set_thermal_override(&mut self, state: crate::thermal_alert::SharedThermalAlert) {
        self.thermal_override = state;
    }

    pub fn thermal_override_active(&self) -> bool {
        self.thermal_override.lock().is_some()
    }

    pub fn check_thermal_override(&mut self) -> bool {
        if self.is_openrgb_controlled() {
            return false;
        }
        let color = *self.thermal_override.lock();
        if color != self.thermal_last_color {
            self.clear_pending();
            self.mb_sync_state.clear();
            if let Some(color) = color {
                let effect = RgbEffect {
                    colors: vec![color],
                    ..Default::default()
                };
                for cap in self.exposed_capabilities() {
                    if self.software_controlled(&cap.device_id) {
                        let mut state = RenderState::new(
                            cap.zones.iter().map(|z| z.led_count as usize).collect(),
                        );
                        for zone in 0..state.counts.len() {
                            if let Err(error) = state.set_effect(zone as u8, &effect) {
                                warn!("Thermal RGB effect failed for {}: {error}", cap.device_id);
                            }
                        }
                        if let Err(error) = self.submit_render(&cap.device_id, &mut state) {
                            warn!("Thermal RGB override failed for {}: {error}", cap.device_id);
                        }
                    } else if let Some(device) = self.wired.get(&cap.device_id) {
                        if let Err(error) = device.set_all_effects(&effect) {
                            warn!("Thermal RGB override failed for {}: {error}", cap.device_id);
                        }
                    }
                }
                // The temporary alert must not become the native restore state.
                self.applied.clear();
            } else if let Some(config) = self.config.clone() {
                self.thermal_last_color = None;
                self.apply_config(&config, &self.presets.clone());
            }
            self.thermal_last_color = color;
        }
        color.is_some()
    }

    pub fn software_controlled(&self, id: &str) -> bool {
        self.wired
            .get(id)
            .is_some_and(|d| d.software_frame_delivery().is_some() && !d.rf_owned())
            || self
                .wireless_state
                .get(id)
                .is_some_and(|d| d.fan_type.rgb_render_profile(d.fan_count).is_some())
    }

    fn render_state(&self, id: &str) -> anyhow::Result<RenderState> {
        let counts = if let Some(device) = self.wired.get(id) {
            device
                .zone_info()
                .iter()
                .map(|z| z.led_count as usize)
                .collect()
        } else if let Some(device) = self.wireless_state.get(id) {
            device.fan_type.rgb_zone_led_counts(device.fan_count)
        } else {
            anyhow::bail!("RGB device not found: {id}");
        };
        if let Some(state) = self.rendered.get(id).filter(|state| state.counts == counts) {
            return Ok(state.clone());
        }
        Ok(RenderState::new(counts))
    }

    fn render_profile(&self, id: &str) -> Option<lianli_shared::rgb::RgbRenderProfile> {
        self.wired
            .get(id)
            .and_then(|d| d.software_render_profile())
            .or_else(|| {
                self.wireless_state.get(id).and_then(|d| {
                    d.fan_type
                        .rgb_render_profile(d.fan_count)
                        .map(|mut profile| {
                            profile.right_attach = d.right_attach;
                            profile
                        })
                })
            })
    }

    fn regional_profile(&self, id: &str) -> Option<lianli_shared::rgb::RgbRenderProfile> {
        self.render_profile(id)
            .filter(|p| !lianli_media::rgb::family::modes(p.family).is_empty())
    }

    pub fn ping(&self, id: &str, zone: u8) -> anyhow::Result<()> {
        if let Some(device) = self.wired.get(id) {
            return device.ping(zone);
        }
        let matched: Vec<_> = self
            .wired
            .iter()
            .filter(|(key, _)| key.starts_with(id))
            .collect();
        if !matched.is_empty() {
            for (_, device) in matched {
                device.ping(zone)?;
            }
            return Ok(());
        }
        if let (Some(wireless), Some(device)) = (&self.wireless, self.wireless_state.get(id)) {
            return wireless.selected_group(&device.mac);
        }
        anyhow::bail!("RGB device not found: {id}")
    }

    pub fn clone_wired_device(&self, id: &str) -> Option<Arc<dyn RgbDevice>> {
        self.wired.get(id).cloned()
    }

    pub fn is_openrgb_controlled(&self) -> bool {
        self.openrgb_active || self.openrgb_server_enabled
    }

    pub fn set_openrgb_active(&mut self, active: bool) {
        if self.openrgb_active == active {
            return;
        }
        self.openrgb_active = active;
        self.clear_pending();
        if !active && !self.openrgb_server_enabled {
            if let Some(config) = self.config.clone() {
                self.apply_config(&config, &self.presets.clone());
            }
        }
    }

    pub fn get_zone_colors(&self, id: &str, zone: u8) -> Option<Vec<[u8; 3]>> {
        let state = self.render_state(id).ok()?;
        Some(state.colors[state.range(zone).ok()?].to_vec())
    }

    pub fn get_all_zone_colors(&self, id: &str) -> Option<Vec<RgbPresetZone>> {
        self.software_controlled(id)
            .then(|| self.render_state(id).ok().map(|s| s.preset_zones()))
            .flatten()
    }

    pub fn get_effect_regions(&self, id: &str) -> Option<Vec<lianli_shared::rgb::RgbRegionConfig>> {
        self.rendered
            .get(id)
            .and_then(|state| state.regions.clone())
    }

    pub fn set_wireless(&mut self, wireless: Option<Arc<WirelessController>>) {
        self.sync_clock.clear();
        self.sync_signature = None;
        self.upload_worker.clear();
        if let Some(previous) = &self.wireless {
            previous.clear_rgb_targets();
        }
        self.wireless = wireless;
    }

    pub fn drain_wired(&mut self) -> HashMap<String, Arc<dyn RgbDevice>> {
        self.clear_pending();
        std::mem::take(&mut self.wired)
    }

    pub fn replace_wired(&mut self, wired: HashMap<String, Arc<dyn RgbDevice>>) {
        self.sync_signature = None;
        self.wired = wired;
        self.rendered
            .retain(|id, _| self.wired.contains_key(id) || self.wireless_state.contains_key(id));
        self.last_direct.retain(|(id, _), _| {
            self.wired.contains_key(id) || self.wireless_state.contains_key(id)
        });
    }

    pub fn retain_wired(&mut self, present: &std::collections::HashSet<String>) {
        let previous: Vec<_> = self.wired.keys().cloned().collect();
        self.wired.retain(|id, _| {
            present.iter().any(|base| {
                id == base
                    || id
                        .strip_prefix(base)
                        .is_some_and(|suffix| suffix.starts_with(':'))
            })
        });
        for id in previous {
            if !self.wired.contains_key(&id) {
                self.sync_signature = None;
                self.sync_clock.clear();
                self.wired_renderer.remove(&id);
                self.applied.remove(&id);
                self.rendered.remove(&id);
                self.mb_sync_state.remove(&id);
                self.last_direct.retain(|(device, _), _| device != &id);
            }
        }
    }

    pub fn refresh_wireless_devices(&mut self) {
        self.sync_signature = None;
        self.thermal_last_color = None;
        let mut devices = HashMap::new();
        if let Some(wireless) = &self.wireless {
            for device in wireless.devices() {
                let id = format!("wireless:{}", device.mac_str());
                let counts = device.fan_type.rgb_zone_led_counts(device.fan_count);
                if self
                    .rendered
                    .get(&id)
                    .is_some_and(|state| state.counts != counts)
                {
                    self.rendered.remove(&id);
                }
                self.applied.remove(&id);
                self.uploads.remove(&id);
                self.mb_sync_state.remove(&id);
                devices.insert(
                    id,
                    WirelessDevice {
                        mac: device.mac,
                        fan_count: device.fan_count,
                        fan_type: device.fan_type,
                        right_attach: device.is_inf_right_attach,
                    },
                );
            }
        }
        self.wireless_state = devices;
        self.rendered
            .retain(|id, _| !id.starts_with("wireless:") || self.wireless_state.contains_key(id));
        self.applied
            .retain(|id, _| !id.starts_with("wireless:") || self.wireless_state.contains_key(id));
        self.uploads
            .retain(|id, _| self.wireless_state.contains_key(id));
        self.last_direct.retain(|(id, _), _| {
            self.wired.contains_key(id) || self.wireless_state.contains_key(id)
        });
    }

    pub fn wireless_topology_matches(
        &self,
        devices: &[lianli_devices::wireless::DiscoveredDevice],
    ) -> bool {
        devices.len() == self.wireless_state.len()
            && devices.iter().all(|device| {
                self.wireless_state
                    .get(&format!("wireless:{}", device.mac_str()))
                    .is_some_and(|old| {
                        old.fan_count == device.fan_count
                            && old.fan_type == device.fan_type
                            && old.right_attach == device.is_inf_right_attach
                    })
            })
    }

    pub fn invalidate_hardware_state(&mut self) {
        self.clear_pending();
        self.mb_sync_state.clear();
    }
}

#[cfg(test)]
mod tests;
