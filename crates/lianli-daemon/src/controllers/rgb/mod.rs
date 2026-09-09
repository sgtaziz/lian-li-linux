//! RGB effects, software rendering, and OpenRGB ownership.
mod direct_color;
mod render;
mod upload;
mod wired;

pub use direct_color::{start_direct_color_writer, DirectColorBuffer};

use lianli_devices::traits::{RgbDevice, RgbFrameDelivery};
use lianli_devices::wireless::{WirelessController, WirelessFanType, WirelessRgbUpload};
use lianli_media::rgb::{FRAME_INTERVAL_MS, SOFTWARE_MODES};
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
        self.upload_worker.stop();
        self.wired_renderer.stop();
    }

    fn clear_pending(&mut self) {
        self.upload_worker.clear();
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
                        if let Err(error) = self.submit_render(&cap.device_id, &state) {
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

    pub fn validate_config(&self, config: &RgbAppConfig) -> anyhow::Result<()> {
        if !config.enabled || config.openrgb_server {
            return Ok(());
        }
        for device in &config.devices {
            if device.mb_rgb_sync || !self.software_controlled(&device.device_id) {
                continue;
            }
            let mut state = self.render_state(&device.device_id)?;
            for zone in &device.zones {
                state.set_effect(zone.zone_index, &zone.effect)?;
            }
            if self.wireless_state.contains_key(&device.device_id)
                || self.wired.get(&device.device_id).is_some_and(|d| {
                    d.software_frame_delivery() == Some(RgbFrameDelivery::LoopUpload)
                })
            {
                state.upload_frames()?;
            }
        }
        Ok(())
    }

    pub fn apply_config(&mut self, config: &RgbAppConfig, presets: &[RgbPreset]) {
        self.config = Some(config.clone());
        self.presets = presets.to_vec();
        self.openrgb_server_enabled = config.openrgb_server;
        if !config.enabled || self.is_openrgb_controlled() {
            self.clear_pending();
            return;
        }
        if self.thermal_override_active() {
            return;
        }

        let removed: Vec<_> = self
            .rendered
            .keys()
            .filter(|id| !config.devices.iter().any(|d| &d.device_id == *id))
            .cloned()
            .collect();
        if removed
            .iter()
            .any(|id| self.wireless_state.contains_key(id))
        {
            self.upload_worker.clear();
            self.applied
                .retain(|id, _| !self.wireless_state.contains_key(id));
        }
        for id in removed {
            self.wired_renderer.remove(&id);
            self.rendered.remove(&id);
            self.applied.remove(&id);
            self.uploads.remove(&id);
        }

        for device in &config.devices {
            let result = (|| -> anyhow::Result<()> {
                if device.mb_rgb_sync {
                    return self.set_mb_rgb_sync(&device.device_id, true);
                }
                if self.software_controlled(&device.device_id) {
                    let mut next = self.render_state(&device.device_id)?;
                    let old = next.clone();
                    next.effects.fill(None);
                    next.colors.fill([0; 3]);
                    for zone in &device.zones {
                        if zone.effect.mode == RgbMode::Direct {
                            let range = old.range(zone.zone_index)?;
                            next.set_direct(zone.zone_index, &old.colors[range])?;
                        } else {
                            next.set_effect(zone.zone_index, &zone.effect)?;
                        }
                    }
                    if let Some(preset) = device.active_preset.as_ref().and_then(|name| {
                        presets
                            .iter()
                            .find(|p| &p.name == name && p.device_id == device.device_id)
                    }) {
                        for zone in &preset.zones {
                            if !zone.colors.is_empty() {
                                next.set_direct(zone.zone, &zone.colors)?;
                            } else if let Some(effect) = &zone.effect {
                                next.set_effect(zone.zone, effect)?;
                            }
                        }
                    }
                    self.apply_render(&device.device_id, next)?;
                } else {
                    for zone in &device.zones {
                        self.set_effect(&device.device_id, zone.zone_index, &zone.effect)?;
                        if zone.swap_lr || zone.swap_tb {
                            self.set_fan_direction(
                                &device.device_id,
                                zone.zone_index,
                                zone.swap_lr,
                                zone.swap_tb,
                            )?;
                        }
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                warn!(
                    "Failed to apply RGB config for {}: {error}",
                    device.device_id
                );
            }
        }
    }

    pub fn software_controlled(&self, id: &str) -> bool {
        self.wired
            .get(id)
            .is_some_and(|d| d.software_frame_delivery().is_some() && !d.rf_owned())
            || self.wireless_state.get(id).is_some_and(|d| {
                let total: usize = d.fan_type.rgb_zone_led_counts(d.fan_count).iter().sum();
                d.fan_type != WirelessFanType::Unknown && (1..=255).contains(&total)
            })
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

    fn submit_render(&mut self, id: &str, state: &RenderState) -> anyhow::Result<()> {
        if let Some(device) = self.wired.get(id) {
            let (frames, interval) =
                if device.software_frame_delivery() == Some(RgbFrameDelivery::LoopUpload) {
                    state.upload_frames()?
                } else {
                    (state.frames(), FRAME_INTERVAL_MS)
                };
            self.wired_renderer
                .submit(id, device.clone(), frames, interval)?;
        } else if let (Some(wireless), Some(device)) = (&self.wireless, self.wireless_state.get(id))
        {
            let (frames, interval) = state.upload_frames()?;
            let upload = Arc::new(wireless.prepare_rgb_upload(&device.mac, &frames, interval)?);
            self.upload_worker.submit(
                wireless.clone(),
                device.mac,
                Command::Upload(upload.clone()),
            )?;
            self.uploads.insert(id.to_owned(), upload);
        } else {
            anyhow::bail!("RGB device not found: {id}");
        }
        self.mb_sync_state.insert(id.to_owned(), false);
        Ok(())
    }

    fn apply_render(&mut self, id: &str, state: RenderState) -> anyhow::Result<()> {
        if self.applied.get(id) != Some(&state) {
            self.submit_render(id, &state)?;
            self.applied.insert(id.to_owned(), state.clone());
        }
        self.rendered.insert(id.to_owned(), state);
        Ok(())
    }

    pub fn set_effect(&mut self, id: &str, zone: u8, effect: &RgbEffect) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.is_openrgb_controlled() || !self.thermal_override_active(),
            "thermal alert currently controls RGB"
        );
        if self.software_controlled(id) {
            let mut state = self.render_state(id)?;
            state.set_effect(zone, effect)?;
            self.apply_render(id, state)?;
            return Ok(());
        }
        if let Some(device) = self.wired.get(id) {
            anyhow::ensure!(
                device.supported_modes().contains(&effect.mode),
                "RGB effect {:?} is not supported by {id}",
                effect.mode
            );
            device.set_zone_effect(zone, effect)?;
            return Ok(());
        }
        anyhow::bail!("RGB device is unavailable or has an unsupported layout: {id}")
    }

    pub fn cache_direct_batch(&mut self, updates: &HashMap<String, HashMap<u8, Vec<[u8; 3]>>>) {
        for (id, zones) in updates {
            for (&zone, colors) in zones {
                self.last_direct.insert((id.clone(), zone), colors.clone());
            }
        }
    }

    pub fn set_direct_colors(
        &mut self,
        id: &str,
        zone: u8,
        colors: &[[u8; 3]],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.is_openrgb_controlled() || !self.thermal_override_active(),
            "thermal alert currently controls RGB"
        );
        if self.software_controlled(id) {
            let mut state = self.render_state(id)?;
            state.set_direct(zone, colors)?;
            self.apply_render(id, state)?;
        } else if let Some(device) = self.wired.get(id) {
            device.set_direct_colors(zone, colors)?;
        } else {
            anyhow::bail!("RGB device is unavailable or has an unsupported layout: {id}");
        }
        self.last_direct
            .insert((id.to_owned(), zone), colors.to_vec());
        Ok(())
    }

    pub fn apply_wireless_batch(
        &mut self,
        id: &str,
        zones: &HashMap<u8, Vec<[u8; 3]>>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.software_controlled(id),
            "device {id} does not support software RGB"
        );
        let mut state = self.render_state(id)?;
        for (&zone, colors) in zones {
            state.set_direct(zone, colors)?;
        }
        self.apply_render(id, state)?;
        for (&zone, colors) in zones {
            self.last_direct
                .insert((id.to_owned(), zone), colors.clone());
        }
        Ok(())
    }

    pub fn resync_wireless_direct_colors(&mut self) {
        self.resync_wireless_effects();
    }

    pub fn resync_wireless_effects(&mut self) {
        if self.config.as_ref().is_some_and(|config| !config.enabled) {
            return;
        }
        if let Some(wireless) = &self.wireless {
            for (id, upload) in &self.uploads {
                if self.mb_sync_state.get(id) == Some(&true) {
                    continue;
                }
                if let Some(device) = self.wireless_state.get(id) {
                    if let Err(error) = self.upload_worker.submit(
                        wireless.clone(),
                        device.mac,
                        Command::Upload(upload.clone()),
                    ) {
                        debug!("RGB resync failed for {id}: {error}");
                    }
                }
            }
        }
    }

    pub fn set_rgb_frames(
        &mut self,
        id: &str,
        frames: &[Vec<[u8; 3]>],
        interval_ms: u16,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.is_openrgb_controlled() || !self.thermal_override_active(),
            "thermal alert currently controls RGB"
        );
        anyhow::ensure!(
            self.software_controlled(id),
            "device {id} does not support software RGB"
        );
        let (Some(wireless), Some(device)) = (&self.wireless, self.wireless_state.get(id)) else {
            anyhow::bail!("wireless RGB device not found: {id}");
        };
        anyhow::ensure!(
            interval_ms >= FRAME_INTERVAL_MS,
            "RGB animation is limited to 20 frames per second"
        );
        let upload = Arc::new(wireless.prepare_rgb_upload(&device.mac, frames, interval_ms)?);
        self.upload_worker.submit(
            wireless.clone(),
            device.mac,
            Command::Upload(upload.clone()),
        )?;
        let mut state = self.render_state(id)?;
        state.colors.copy_from_slice(&frames[0]);
        state.effects.fill(None);
        self.applied.remove(id);
        self.rendered.insert(id.to_owned(), state);
        self.uploads.insert(id.to_owned(), upload);
        self.mb_sync_state.insert(id.to_owned(), false);
        Ok(())
    }

    pub fn capabilities(&self) -> Vec<RgbDeviceCapabilities> {
        let mut caps = Vec::new();
        for (id, device) in &self.wired {
            let software_modes = if self.software_controlled(id) {
                SOFTWARE_MODES.to_vec()
            } else {
                vec![]
            };
            let mut modes = device.supported_modes();
            for &mode in &software_modes {
                if !modes.contains(&mode) {
                    modes.push(mode);
                }
            }
            caps.push(RgbDeviceCapabilities {
                device_id: id.clone(),
                device_name: device.device_name(),
                supported_modes: modes,
                software_modes,
                zones: device.zone_info(),
                supports_direct: device.supports_direct(),
                supports_mb_rgb_sync: device.supports_mb_rgb_sync(),
                total_led_count: device.total_led_count(),
                supported_scopes: device.supported_scopes(),
                supports_direction: device.supports_direction(),
                supports_merge_lighting: device.supports_merge_lighting(),
                rf_owned: device.rf_owned(),
            });
        }
        for (id, device) in &self.wireless_state {
            if !self.software_controlled(id) {
                continue;
            }
            let zones: Vec<_> = device
                .fan_type
                .rgb_zone_led_counts(device.fan_count)
                .iter()
                .enumerate()
                .map(|(index, &count)| RgbZoneInfo {
                    name: if device.fan_type.total_led_count_override().is_some() {
                        match device.fan_type {
                            WirelessFanType::Lc217 => "Case Ring",
                            WirelessFanType::Led88 => "Screen Ring",
                            _ => "LED Strip",
                        }
                        .to_owned()
                    } else if device.fan_type.is_aio() && index == 0 {
                        "Pump Head".to_owned()
                    } else {
                        format!("Fan {}", index + usize::from(!device.fan_type.is_aio()))
                    },
                    led_count: count as u16,
                })
                .collect();
            let mut supported_modes = SOFTWARE_MODES.to_vec();
            supported_modes.push(RgbMode::Direct);
            caps.push(RgbDeviceCapabilities {
                device_id: id.clone(),
                device_name: device.fan_type.display_name().to_owned(),
                supported_modes,
                software_modes: SOFTWARE_MODES.to_vec(),
                total_led_count: zones.iter().map(|z| z.led_count).sum(),
                zones,
                supports_direct: true,
                supports_mb_rgb_sync: device.fan_type.supports_mb_rgb_sync(),
                supported_scopes: vec![],
                supports_direction: false,
                supports_merge_lighting: false,
                rf_owned: false,
            });
        }
        caps.sort_by(|a, b| a.device_id.cmp(&b.device_id));
        caps
    }

    pub fn exposed_capabilities(&self) -> Vec<RgbDeviceCapabilities> {
        self.capabilities()
            .into_iter()
            .filter(|c| !c.rf_owned)
            .collect()
    }

    pub fn set_mb_rgb_sync(&mut self, id: &str, enabled: bool) -> anyhow::Result<()> {
        anyhow::ensure!(
            !enabled || self.is_openrgb_controlled() || !self.thermal_override_active(),
            "thermal alert currently controls RGB"
        );
        if self.mb_sync_state.get(id) == Some(&enabled) {
            return Ok(());
        }
        if let Some(device) = self.wired.get(id) {
            anyhow::ensure!(
                device.supports_mb_rgb_sync(),
                "Device {id} does not support MB RGB sync"
            );
            self.wired_renderer.remove(id);
            device.set_mb_rgb_sync(enabled)?;
        } else if let (Some(wireless), Some(device)) = (&self.wireless, self.wireless_state.get(id))
        {
            anyhow::ensure!(
                device.fan_type.supports_mb_rgb_sync(),
                "Device {id} does not support MB RGB sync"
            );
            self.upload_worker.submit(
                wireless.clone(),
                device.mac,
                Command::MotherboardSync(enabled),
            )?;
        } else {
            anyhow::bail!("RGB device not found: {id}");
        }
        self.applied.remove(id);
        self.mb_sync_state.insert(id.to_owned(), enabled);
        Ok(())
    }

    pub fn set_fan_direction(
        &self,
        id: &str,
        zone: u8,
        swap_lr: bool,
        swap_tb: bool,
    ) -> anyhow::Result<()> {
        let device = self
            .wired
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("RGB device not found: {id}"))?;
        anyhow::ensure!(
            device.supports_direction(),
            "Device {id} does not support fan direction"
        );
        device.set_fan_direction(zone, swap_lr, swap_tb)
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

    pub fn set_wireless(&mut self, wireless: Option<Arc<WirelessController>>) {
        self.upload_worker.clear();
        self.wireless = wireless;
    }

    pub fn drain_wired(&mut self) -> HashMap<String, Arc<dyn RgbDevice>> {
        self.clear_pending();
        std::mem::take(&mut self.wired)
    }

    pub fn replace_wired(&mut self, wired: HashMap<String, Arc<dyn RgbDevice>>) {
        self.wired = wired;
        self.rendered
            .retain(|id, _| self.wired.contains_key(id) || self.wireless_state.contains_key(id));
        self.last_direct.retain(|(id, _), _| {
            self.wired.contains_key(id) || self.wireless_state.contains_key(id)
        });
    }

    pub fn retain_wired(&mut self, present: &std::collections::HashSet<String>) {
        self.wired.retain(|id, _| {
            present.iter().any(|base| {
                id == base
                    || id
                        .strip_prefix(base)
                        .is_some_and(|suffix| suffix.starts_with(':'))
            })
        });
    }

    pub fn refresh_wireless_devices(&mut self) {
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
                        old.fan_count == device.fan_count && old.fan_type == device.fan_type
                    })
            })
    }

    pub fn invalidate_hardware_state(&mut self) {
        self.clear_pending();
        self.mb_sync_state.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct MockRgb {
        rf: bool,
    }
    impl RgbDevice for MockRgb {
        fn device_name(&self) -> String {
            "mock".into()
        }
        fn supported_modes(&self) -> Vec<RgbMode> {
            vec![RgbMode::Static]
        }
        fn zone_info(&self) -> Vec<RgbZoneInfo> {
            vec![]
        }
        fn set_zone_effect(&self, _: u8, _: &RgbEffect) -> anyhow::Result<()> {
            Ok(())
        }
        fn rf_owned(&self) -> bool {
            self.rf
        }
    }

    #[test]
    fn exposed_capabilities_hide_rf_owned_and_do_not_infer_software_support() {
        let wired = HashMap::from([
            (
                "hid:kept".into(),
                Arc::new(MockRgb { rf: false }) as Arc<dyn RgbDevice>,
            ),
            (
                "hid:rf".into(),
                Arc::new(MockRgb { rf: true }) as Arc<dyn RgbDevice>,
            ),
        ]);
        let controller = RgbController::new(wired, None);
        let caps = controller.exposed_capabilities();
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].device_id, "hid:kept");
        assert!(caps[0].software_modes.is_empty());
        assert_eq!(caps[0].supported_modes, [RgbMode::Static]);
    }

    #[test]
    fn wireless_capabilities_follow_runtime_layout_and_reject_unknown_types() {
        let mut controller = RgbController::new(HashMap::new(), None);
        for (id, fan_type, fan_count) in [
            ("wireless:pump", WirelessFanType::WaterBlock2, 3),
            ("wireless:v150", WirelessFanType::V150, 4),
            ("wireless:unknown", WirelessFanType::Unknown, 1),
            ("wireless:oversize", WirelessFanType::SlV4, 6),
        ] {
            controller.wireless_state.insert(
                id.into(),
                WirelessDevice {
                    mac: [0; 6],
                    fan_type,
                    fan_count,
                },
            );
        }
        let caps = controller.capabilities();
        assert_eq!(caps.len(), 2);
        assert_eq!(
            caps[0]
                .zones
                .iter()
                .map(|z| z.led_count)
                .collect::<Vec<_>>(),
            [24, 24, 24, 24]
        );
        assert_eq!(caps[1].zones[0].led_count, 88);
        assert!(caps[0].supported_modes.contains(&RgbMode::Rainbow));
    }
}
