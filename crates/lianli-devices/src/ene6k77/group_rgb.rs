use super::controller::Ene6k77Controller;
use super::{Ene6k77Model, CMD_DELAY, REPORT_ID};
use crate::traits::RgbDevice;
use anyhow::Result;
use lianli_shared::rgb::{
    RgbDeviceConfig, RgbDirection, RgbEffect, RgbEffectParameters, RgbMode, RgbRegionParameters,
    RgbScope, RgbZoneInfo,
};
use lianli_shared::ui_message::UiMessage;
use parking_lot::Mutex;
use std::sync::Arc;
use std::thread;

fn fixed_colors(model: Ene6k77Model, mode: RgbMode) -> bool {
    mode == RgbMode::Off
        || (model == Ene6k77Model::AlV2Fan
            && matches!(
                mode,
                RgbMode::Rainbow | RgbMode::RainbowMorph | RgbMode::MeteorRainbow
            ))
}

fn resolve_effects(config: &RgbDeviceConfig, model: Ene6k77Model, fan_count: u8) -> Vec<RgbEffect> {
    let mut config = config.clone();
    config.expand_legacy_group_zone(model.max_fans_per_group());
    let scopes = if model.uses_double_port() {
        vec![RgbScope::Inner, RgbScope::Outer]
    } else {
        vec![RgbScope::All]
    };
    scopes
        .into_iter()
        .filter_map(|scope| {
            let mut effect = if let Some(regions) = &config.regions {
                regions
                    .iter()
                    .rev()
                    .find(|region| {
                        region.effect.scope == scope || region.effect.scope == RgbScope::All
                    })
                    .map(|region| region.effect.clone())
                    .unwrap_or(RgbEffect {
                        mode: RgbMode::Off,
                        ..Default::default()
                    })
            } else {
                let zones: Vec<_> = config
                    .zones
                    .iter()
                    .filter(|zone| {
                        zone.zone_index < fan_count
                            && (zone.effect.scope == scope || zone.effect.scope == RgbScope::All)
                    })
                    .collect();
                let mut effect = zones.last()?.effect.clone();
                if matches!(effect.mode, RgbMode::Static | RgbMode::Breathing) {
                    effect.colors = vec![[0; 3]; model.max_fans_per_group() as usize];
                    for zone in zones {
                        if zone.effect.mode == effect.mode && !zone.effect.disabled {
                            effect.colors[zone.zone_index as usize] =
                                zone.effect.colors.first().copied().unwrap_or([0; 3]);
                        }
                    }
                }
                effect
            };
            effect.scope = scope;
            if effect.disabled {
                effect.mode = RgbMode::Off;
            }
            if config.regions.is_none()
                && scope == RgbScope::Outer
                && super::controller::map_mode_outer_for(model, effect.mode).is_none()
            {
                return None;
            }
            Some(effect)
        })
        .collect()
}

fn separate_rings(model: Ene6k77Model, effects: &[RgbEffect]) -> Vec<RgbEffect> {
    let mut result = Vec::new();
    for effect in effects {
        if model.uses_double_port() && effect.scope == RgbScope::All {
            let mut inner = effect.clone();
            inner.scope = RgbScope::Inner;
            result.push(inner);
            if super::controller::map_mode_outer_for(model, effect.mode).is_some() {
                let mut outer = effect.clone();
                outer.scope = RgbScope::Outer;
                result.push(outer);
            }
        } else {
            result.push(effect.clone());
        }
    }
    result
}

/// Per-group RGB device wrapper — each physical group appears as a separate device.
pub struct Ene6k77GroupDevice {
    controller: Arc<Ene6k77Controller>,
    group: u8,
    effects: Mutex<Vec<RgbEffect>>,
}

impl Ene6k77GroupDevice {
    pub fn new(controller: Arc<Ene6k77Controller>, group: u8) -> Self {
        Self {
            controller,
            group,
            effects: Mutex::new(Vec::new()),
        }
    }
}

impl Ene6k77Controller {
    /// Create per-group RGB devices (similar to TL fan port_devices).
    pub fn group_devices(self: &Arc<Self>) -> Vec<(u8, Ene6k77GroupDevice)> {
        (0..4)
            .map(|g| (g, Ene6k77GroupDevice::new(Arc::clone(self), g)))
            .collect()
    }
}

impl RgbDevice for Ene6k77GroupDevice {
    fn device_name(&self) -> String {
        format!(
            "UNI FAN {} Group {}",
            self.controller.model.name(),
            self.group
        )
    }

    fn supported_modes(&self) -> Vec<RgbMode> {
        use super::Ene6k77Model;
        let m = self.controller.model;
        if m.uses_double_port() {
            // Shared base for all dual-ring models
            let mut modes = vec![
                RgbMode::Off,
                RgbMode::Static,
                RgbMode::Breathing,
                RgbMode::Rainbow,
                RgbMode::MeteorRainbow,
                RgbMode::ColorCycle,
                RgbMode::Meteor,
                RgbMode::Runway,
                RgbMode::MopUp,
                RgbMode::Lottery,
                RgbMode::Wave,
                RgbMode::Spring,
                RgbMode::TailChasing,
                RgbMode::Warning,
                RgbMode::Voice,
                RgbMode::Mixing,
                RgbMode::Stack,
                RgbMode::Tide,
                RgbMode::Scan,
                RgbMode::PacMan,
                RgbMode::StaticColorful,
                RgbMode::BreathingColorful,
            ];
            match m {
                Ene6k77Model::AlV2Fan => {
                    modes.extend([
                        RgbMode::RainbowMorph,
                        RgbMode::ColorfulCity,
                        RgbMode::Render,
                        RgbMode::Twinkle,
                    ]);
                }
                Ene6k77Model::SlInfinity => {
                    // SL Infinity has a completely different set
                    return vec![
                        RgbMode::Off,
                        RgbMode::Static,
                        RgbMode::Breathing,
                        RgbMode::Rainbow,
                        RgbMode::RainbowMorph,
                        RgbMode::BreathingRainbow,
                        RgbMode::MeteorRainbow,
                        RgbMode::ColorCycle,
                        RgbMode::Meteor,
                        RgbMode::Runway,
                        RgbMode::MopUp,
                        RgbMode::DoubleMeteor,
                        RgbMode::MeteorContest,
                        RgbMode::MeteorMix,
                        RgbMode::ReturnArc,
                        RgbMode::DoubleArc,
                        RgbMode::Door,
                        RgbMode::Disco,
                        RgbMode::HeartBeat,
                        RgbMode::Lottery,
                        RgbMode::Warning,
                        RgbMode::Voice,
                        RgbMode::Mixing,
                        RgbMode::Stack,
                        RgbMode::Tide,
                        RgbMode::Scan,
                        RgbMode::HeartBeatRunway,
                    ];
                }
                _ => {}
            }
            modes
        } else {
            // Single-ring models
            let mut modes = vec![
                RgbMode::Off,
                RgbMode::Static,
                RgbMode::Breathing,
                RgbMode::ColorCycle,
                RgbMode::Rainbow,
                RgbMode::RainbowMorph,
                RgbMode::Runway,
                RgbMode::Meteor,
                RgbMode::Staggered,
                RgbMode::Tide,
                RgbMode::Mixing,
                RgbMode::Stack,
                RgbMode::StackMulti,
                RgbMode::Neon,
            ];
            if m.is_v2() {
                modes.extend([
                    RgbMode::Voice,
                    RgbMode::Groove,
                    RgbMode::Render,
                    RgbMode::Tunnel,
                ]);
            }
            modes
        }
    }

    fn zone_info(&self) -> Vec<RgbZoneInfo> {
        let fans = self.controller.fan_quantity(self.group);
        let leds_per_fan = self.controller.leds_per_fan();
        (0..fans)
            .map(|fan| RgbZoneInfo {
                name: format!("Fan {}", fan + 1),
                name_ui: Some(UiMessage::new("devices.zone.fan").param("number", fan + 1)),
                led_count: leds_per_fan,
            })
            .collect()
    }

    fn supported_scopes(&self) -> Vec<Vec<RgbScope>> {
        let fans = self.controller.fan_quantity(self.group) as usize;
        if self.controller.model.uses_double_port() {
            vec![vec![RgbScope::All, RgbScope::Inner, RgbScope::Outer]; fans]
        } else {
            vec![vec![]; fans]
        }
    }

    fn hardware_regions(&self) -> Vec<RgbRegionParameters> {
        let scopes = if self.controller.model.uses_double_port() {
            vec![RgbScope::All, RgbScope::Inner, RgbScope::Outer]
        } else {
            vec![RgbScope::All]
        };
        scopes
            .into_iter()
            .map(|scope| RgbRegionParameters {
                scope,
                effects: self
                    .supported_modes()
                    .into_iter()
                    .filter(|&mode| {
                        if matches!(mode, RgbMode::StaticColorful | RgbMode::BreathingColorful) {
                            return scope == RgbScope::Outer;
                        }
                        !self.controller.model.uses_double_port()
                            || scope == RgbScope::Inner
                            || super::controller::map_mode_outer_for(self.controller.model, mode)
                                .is_some()
                    })
                    .map(|mode| {
                        let per_fan_colors = matches!(mode, RgbMode::Static | RgbMode::Breathing);
                        RgbEffectParameters {
                            mode,
                            min_colors: 0,
                            max_colors: if fixed_colors(self.controller.model, mode) {
                                0
                            } else if per_fan_colors {
                                self.controller.fan_quantity(self.group)
                            } else if matches!(
                                mode,
                                RgbMode::StaticColorful | RgbMode::BreathingColorful
                            ) {
                                4
                            } else {
                                self.controller.model.palette_size() as u8
                            },
                            per_fan_colors,
                            directions: vec![
                                RgbDirection::Clockwise,
                                RgbDirection::CounterClockwise,
                            ],
                            supports_speed: !matches!(
                                mode,
                                RgbMode::Off | RgbMode::Static | RgbMode::StaticColorful
                            ),
                        }
                    })
                    .collect(),
            })
            .collect()
    }

    fn resolve_group_config(&self, config: &RgbDeviceConfig) -> Result<Option<Vec<RgbEffect>>> {
        if let Some(regions) = &config.regions {
            anyhow::ensure!(regions.len() <= 3, "Too many ENE group regions");
            let available = self.hardware_regions();
            for region in regions {
                anyhow::ensure!(
                    !region.flip
                        && available.iter().any(|parameters| parameters.scope
                            == region.effect.scope
                            && parameters
                                .effects
                                .iter()
                                .any(|parameter| parameter.mode == region.effect.mode)),
                    "Unsupported ENE group mode, ring or orientation"
                );
            }
        }
        let mut effects = resolve_effects(
            config,
            self.controller.model,
            self.controller.fan_quantity(self.group),
        );
        let regions = self.hardware_regions();
        for effect in &mut effects {
            anyhow::ensure!(
                regions.iter().any(|region| region.scope == effect.scope
                    && region
                        .effects
                        .iter()
                        .any(|parameter| parameter.mode == effect.mode)),
                "Unsupported ENE group mode or ring: {:?}/{:?}",
                effect.mode,
                effect.scope
            );
            let limit = match effect.mode {
                _ if fixed_colors(self.controller.model, effect.mode) => 0,
                RgbMode::Static | RgbMode::Breathing => {
                    self.controller.model.max_fans_per_group() as usize
                }
                RgbMode::StaticColorful | RgbMode::BreathingColorful => 4,
                _ => self.controller.model.palette_size(),
            };
            if config.regions.is_none() || limit == 0 {
                effect.colors.truncate(limit);
            } else {
                anyhow::ensure!(effect.colors.len() <= limit, "Too many ENE group colors");
            }
        }
        Ok(Some(effects))
    }

    fn set_group_effects(&self, effects: &[RgbEffect]) -> Result<()> {
        self.controller.set_group_effects(self.group, effects)?;
        *self.effects.lock() = separate_rings(self.controller.model, effects);
        Ok(())
    }

    fn set_zone_effect(&self, zone: u8, effect: &RgbEffect) -> Result<()> {
        anyhow::ensure!(
            zone < self.controller.fan_quantity(self.group),
            "Fan index is outside this group"
        );
        anyhow::ensure!(
            self.hardware_regions()
                .iter()
                .any(|region| region.scope == effect.scope
                    && region
                        .effects
                        .iter()
                        .any(|parameter| parameter.mode == effect.mode)),
            "Unsupported ENE group mode or ring"
        );
        let mut effects = self.effects.lock().clone();
        let scopes = if self.controller.model.uses_double_port() && effect.scope == RgbScope::All {
            vec![RgbScope::Inner, RgbScope::Outer]
        } else {
            vec![effect.scope]
        };
        for scope in scopes {
            let mut next = effect.clone();
            next.scope = scope;
            if matches!(effect.mode, RgbMode::Static | RgbMode::Breathing) {
                next.colors = effects
                    .iter()
                    .find(|old| old.scope == scope || old.scope == RgbScope::All)
                    .filter(|old| old.mode == effect.mode)
                    .map(|old| old.colors.clone())
                    .unwrap_or_default();
                next.colors
                    .resize(self.controller.model.max_fans_per_group() as usize, [0; 3]);
                next.colors[zone as usize] = effect.colors.first().copied().unwrap_or([0; 3]);
            }
            effects.retain(|old| old.scope != scope && old.scope != RgbScope::All);
            effects.push(next);
        }
        self.set_group_effects(&effects)
    }

    fn set_all_effects(&self, effect: &RgbEffect) -> Result<()> {
        let mut effect = effect.clone();
        if matches!(effect.mode, RgbMode::Static | RgbMode::Breathing) && effect.colors.len() == 1 {
            effect.colors.resize(
                self.controller.model.max_fans_per_group() as usize,
                effect.colors[0],
            );
        }
        self.set_group_effects(&[effect])
    }

    fn supports_mb_rgb_sync(&self) -> bool {
        true
    }

    fn set_mb_rgb_sync(&self, enabled: bool) -> Result<()> {
        let sub_cmd = match self.controller.model {
            Ene6k77Model::SlFan | Ene6k77Model::SlRedragon => 0x30,
            Ene6k77Model::AlFan => 0x41,
            Ene6k77Model::SlV2Fan
            | Ene6k77Model::SlV2aFan
            | Ene6k77Model::AlV2Fan
            | Ene6k77Model::SlInfinity => 0x61,
        };
        self.controller
            .send_feature(&[REPORT_ID, 0x10, sub_cmd, enabled as u8, 0, 0])?;
        thread::sleep(CMD_DELAY);
        Ok(())
    }

    fn supports_merge_lighting(&self) -> bool {
        true
    }

    fn start_merge_lighting(&self) -> Result<()> {
        match self.controller.model {
            Ene6k77Model::SlFan | Ene6k77Model::SlRedragon => self.controller.start_merge(),
            Ene6k77Model::AlFan => self.controller.send_merge_command(true),
            Ene6k77Model::SlV2Fan
            | Ene6k77Model::SlV2aFan
            | Ene6k77Model::AlV2Fan
            | Ene6k77Model::SlInfinity => self.controller.set_merge_order([0, 1, 2, 3]),
        }
    }

    fn stop_merge_lighting(&self) -> Result<()> {
        match self.controller.model {
            Ene6k77Model::SlFan | Ene6k77Model::SlRedragon => self.controller.stop_merge(),
            Ene6k77Model::AlFan => self.controller.send_merge_command(false),
            // V2/SLInfinity variants: set_merge_order with identity exits merge mode
            _ => self.controller.set_merge_order([0, 1, 2, 3]),
        }
    }

    fn ping(&self, _zone: u8) -> Result<()> {
        let g = self.group & 0x0F;
        match self.controller.model {
            Ene6k77Model::SlFan
            | Ene6k77Model::SlRedragon
            | Ene6k77Model::SlV2Fan
            | Ene6k77Model::SlV2aFan => {
                self.controller
                    .send_feature(&[REPORT_ID, 0x10 | g, 0x11, 0xFF, 0x00, 0x02])?;
            }
            Ene6k77Model::AlFan => {
                self.controller.send_feature(&[
                    REPORT_ID,
                    0x10 | ((g * 2) & 0xF),
                    0x34,
                    0xFF,
                    0x00,
                    0x02,
                ])?;
            }
            Ene6k77Model::SlInfinity => {
                self.controller.send_feature(&[
                    REPORT_ID,
                    0x10 | ((g * 2) & 0xF),
                    0x3E,
                    0x00,
                    0x00,
                    0x02,
                ])?;
            }
            Ene6k77Model::AlV2Fan => {
                self.controller.send_feature(&[
                    REPORT_ID,
                    0x10 | ((g * 2) & 0xF),
                    0x36,
                    0x00,
                    0x00,
                    0x02,
                ])?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_shared::rgb::{RgbRegionConfig, RgbZoneConfig};

    fn config() -> RgbDeviceConfig {
        RgbDeviceConfig {
            device_id: "controller:group0".into(),
            fan_led_count: None,
            mb_rgb_sync: false,
            active_preset: None,
            zones: (0..4)
                .map(|zone_index| RgbZoneConfig {
                    zone_index,
                    effect: RgbEffect {
                        colors: vec![[zone_index + 1, 20, 30]],
                        ..Default::default()
                    },
                    swap_lr: false,
                    swap_tb: false,
                })
                .collect(),
            regions: None,
            effect_memory: Vec::new(),
            port_merge: None,
            reverse_port: false,
        }
    }

    #[test]
    fn legacy_single_zone_lights_every_connected_fan() {
        for model in [Ene6k77Model::SlInfinity, Ene6k77Model::SlV2Fan] {
            for mode in [RgbMode::Static, RgbMode::Breathing] {
                let mut config = config();
                config.zones.truncate(1);
                config.zones[0].effect.mode = mode;
                for count in [1, 3, model.max_fans_per_group()] {
                    let effects = resolve_effects(&config, model, count);
                    assert!(!effects.is_empty());
                    for effect in effects {
                        assert_eq!(effect.mode, mode);
                        assert_eq!(
                            &effect.colors[..usize::from(count)],
                            vec![[1, 20, 30]; usize::from(count)]
                        );
                        assert!(effect.colors[usize::from(count)..]
                            .iter()
                            .all(|color| *color == [0; 3]));
                    }
                }
                assert!(resolve_effects(&config, model, 0).is_empty());
                assert_eq!(config.zones.len(), 1);
            }
        }
    }

    #[test]
    fn legacy_fan_colors_are_combined_and_dormant_fans_are_retained() {
        let config = config();
        for model in [
            Ene6k77Model::SlFan,
            Ene6k77Model::SlV2Fan,
            Ene6k77Model::SlV2aFan,
            Ene6k77Model::SlRedragon,
            Ene6k77Model::AlFan,
            Ene6k77Model::AlV2Fan,
            Ene6k77Model::SlInfinity,
        ] {
            let reduced = resolve_effects(&config, model, 2);
            for effect in reduced {
                assert_eq!(&effect.colors[..2], &[[1, 20, 30], [2, 20, 30]]);
                assert!(effect.colors[2..].iter().all(|color| *color == [0; 3]));
            }
            let restored = resolve_effects(&config, model, 4);
            assert_eq!(restored[0].colors[3], [4, 20, 30]);
            assert!(resolve_effects(&config, model, 0).is_empty());
        }
        assert_eq!(config.zones.len(), 4);
    }

    #[test]
    fn legacy_conflicts_use_last_shared_parameters_without_losing_matching_colors() {
        let mut config = config();
        config.zones[0].effect.mode = RgbMode::Rainbow;
        config.zones[3].effect.brightness = 1;
        let effects = resolve_effects(&config, Ene6k77Model::SlInfinity, 4);
        assert_eq!(effects.len(), 2);
        assert_eq!(effects[0].colors[0], [0; 3]);
        assert_eq!(effects[0].colors[1], [2, 20, 30]);
        assert_eq!(effects[0].brightness, 1);
        assert_eq!(config.zones[0].effect.mode, RgbMode::Rainbow);
    }

    #[test]
    fn explicit_rings_override_legacy_zones_and_keep_disconnected_colors() {
        let mut config = config();
        config.regions = Some(vec![
            RgbRegionConfig {
                effect: RgbEffect {
                    scope: RgbScope::All,
                    colors: vec![[9, 8, 7]; 4],
                    ..Default::default()
                },
                flip: false,
            },
            RgbRegionConfig {
                effect: RgbEffect {
                    scope: RgbScope::Outer,
                    mode: RgbMode::Breathing,
                    colors: vec![[6, 5, 4]; 4],
                    ..Default::default()
                },
                flip: false,
            },
        ]);
        let effects = resolve_effects(&config, Ene6k77Model::SlInfinity, 1);
        assert_eq!(effects[0].scope, RgbScope::Inner);
        assert_eq!(effects[0].colors, [[9, 8, 7]; 4]);
        assert_eq!(effects[1].mode, RgbMode::Breathing);
        assert_eq!(effects[1].colors, [[6, 5, 4]; 4]);
        config.regions.as_mut().unwrap().remove(0);
        assert_eq!(
            resolve_effects(&config, Ene6k77Model::SlInfinity, 1)[0].mode,
            RgbMode::Off
        );
    }
}
