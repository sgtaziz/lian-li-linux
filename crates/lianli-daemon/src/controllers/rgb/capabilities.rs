use super::*;
use lianli_shared::ui_message::UiMessage;

impl RgbController {
    pub fn capabilities(&self) -> Vec<RgbDeviceCapabilities> {
        let mut caps = Vec::new();
        for (id, device) in &self.wired {
            let profile = self.render_profile(id);
            let hardware_regions = device.hardware_regions();
            let software_modes = if self.software_controlled(id) {
                self.regional_profile(id)
                    .map_or_else(Vec::new, lianli_media::rgb::parameters::modes)
            } else {
                vec![]
            };
            let mut modes = device.supported_modes();
            for &mode in &software_modes {
                if !modes.contains(&mode) {
                    modes.push(mode);
                }
            }
            let deferred = device.deferred_reason();
            caps.push(RgbDeviceCapabilities {
                fan_led_count_control: device.fan_led_count_control(),
                hardware_group_effects: !hardware_regions.is_empty(),
                group_effect_modes: device.group_effect_modes(),
                zone_effect_modes: device.zone_effect_modes(),
                device_id: id.clone(),
                sync_led_count: self
                    .sync_layout(id)
                    .map(|layout| layout.logical_led_count() as u16),
                sync_effect_parameters: self.sync_parameters(id),
                device_name: device.device_name(),
                supported_modes: modes,
                software_modes,
                effect_regions: if hardware_regions.is_empty() {
                    self.regional_profile(id)
                        .map_or_else(Vec::new, lianli_media::rgb::parameters::scopes)
                } else {
                    hardware_regions.iter().map(|region| region.scope).collect()
                },
                render_profile: profile,
                region_parameters: if hardware_regions.is_empty() {
                    profile.map_or_else(Vec::new, lianli_media::rgb::parameters::for_regions)
                } else {
                    hardware_regions
                },
                effect_parameters: profile
                    .map_or_else(Vec::new, lianli_media::rgb::parameters::for_profile),
                zones: device.zone_info(),
                supports_direct: device.supports_direct(),
                supports_mb_rgb_sync: device.supports_mb_rgb_sync(),
                total_led_count: device.total_led_count(),
                supported_scopes: device.supported_scopes(),
                supports_direction: device.supports_direction(),
                supports_merge_lighting: !device.merged_effect_modes().is_empty(),
                merged_effect_modes: device.merged_effect_modes(),
                merged_side_effect_modes: device.merged_side_effect_modes(),
                supports_port_reverse: device.supports_port_reverse(),
                rf_owned: device.rf_owned(),
                deferred_reason: deferred.as_ref().map(ToString::to_string),
                deferred_reason_ui: deferred.map(|reason| reason.message),
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
                .map(|(index, &count)| {
                    let (name, name_ui) = if device.fan_type.total_led_count_override().is_some() {
                        let (name, key) = match device.fan_type {
                            WirelessFanType::Lc217 => ("Case Ring", "devices.zone.caseRing"),
                            WirelessFanType::Led88 => ("Screen Ring", "devices.zone.screenRing"),
                            _ => ("LED Strip", "devices.zone.ledStrip"),
                        };
                        (name.to_owned(), UiMessage::new(key))
                    } else if device.fan_type.is_aio() && index == 0 {
                        (
                            "Pump Head".to_owned(),
                            UiMessage::new("devices.zone.pumpHead"),
                        )
                    } else {
                        let number = index + usize::from(!device.fan_type.is_aio());
                        (
                            format!("Fan {number}"),
                            UiMessage::new("devices.zone.fan").param("number", number),
                        )
                    };
                    RgbZoneInfo {
                        name,
                        name_ui: Some(name_ui),
                        led_count: count as u16,
                    }
                })
                .collect();
            let software_modes = self
                .regional_profile(id)
                .map_or_else(Vec::new, lianli_media::rgb::parameters::modes);
            let mut supported_modes = software_modes.clone();
            supported_modes.push(RgbMode::Direct);
            caps.push(RgbDeviceCapabilities {
                fan_led_count_control: None,
                hardware_group_effects: false,
                group_effect_modes: Vec::new(),
                zone_effect_modes: Vec::new(),
                device_id: id.clone(),
                sync_led_count: self
                    .sync_layout(id)
                    .map(|layout| layout.logical_led_count() as u16),
                sync_effect_parameters: self.sync_parameters(id),
                device_name: device.fan_type.display_name().to_owned(),
                supported_modes,
                software_modes,
                render_profile: self.render_profile(id),
                region_parameters: self
                    .render_profile(id)
                    .map_or_else(Vec::new, lianli_media::rgb::parameters::for_regions),
                effect_parameters: self
                    .render_profile(id)
                    .map_or_else(Vec::new, lianli_media::rgb::parameters::for_profile),
                effect_regions: self
                    .regional_profile(id)
                    .map_or_else(Vec::new, lianli_media::rgb::parameters::scopes),
                total_led_count: zones.iter().map(|z| z.led_count).sum(),
                zones,
                supports_direct: true,
                supports_mb_rgb_sync: device.fan_type.supports_mb_rgb_sync(),
                supported_scopes: vec![],
                supports_direction: false,
                supports_merge_lighting: false,
                merged_effect_modes: Vec::new(),
                merged_side_effect_modes: Vec::new(),
                supports_port_reverse: false,
                rf_owned: false,
                deferred_reason: None,
                deferred_reason_ui: None,
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

    pub(super) fn sync_layout(&self, id: &str) -> Option<lianli_media::rgb::sync_layout::Layout> {
        if self
            .wireless_state
            .get(id)
            .is_some_and(|d| d.fan_type == WirelessFanType::Led88)
        {
            return None;
        }
        self.software_controlled(id)
            .then(|| self.render_profile(id))
            .flatten()
            .and_then(lianli_media::rgb::sync_layout::Layout::for_profile)
    }

    fn sync_parameters(&self, id: &str) -> Vec<lianli_shared::rgb::RgbEffectParameters> {
        if self.sync_layout(id).is_none() {
            return Vec::new();
        }
        lianli_media::rgb::sync_effects::parameters()
            .into_iter()
            .filter(|p| {
                !matches!(p.mode, RgbMode::RainbowMorph | RgbMode::Twinkle)
                    || self.regional_profile(id).is_some_and(|profile| {
                        lianli_media::rgb::parameters::modes(profile).contains(&p.mode)
                    })
            })
            .collect()
    }
}
