use super::controller::TlFanController;
use super::LEDS_PER_FAN;
use crate::traits::RgbDevice;
use anyhow::{bail, Result};
use lianli_shared::rgb::{RgbEffect, RgbMode, RgbScope, RgbZoneInfo};
use std::sync::Arc;
use tracing::debug;

/// Per-port RGB device for the TL Fan controller.
///
/// Each port with detected fans becomes a separate `RgbDevice`.
/// Zones within the device = individual fans on that port.
/// Animated effects use SetFanGroupLight (0xB0) for synced animation across the port.
/// Static/Direct/Off use per-fan SetFanLight (0xA3) for individual color control.
pub struct TlFanPortDevice {
    controller: Arc<TlFanController>,
    port: u8,
    fan_count: u8,
}

impl TlFanPortDevice {
    pub fn new(controller: Arc<TlFanController>, port: u8, fan_count: u8) -> Self {
        Self {
            controller,
            port,
            fan_count,
        }
    }
}

impl TlFanController {
    /// Create per-port RGB devices from a shared controller reference.
    /// Each active port becomes a separate `RgbDevice`.
    pub fn port_devices(self: &Arc<Self>) -> Vec<(u8, TlFanPortDevice)> {
        let port_fan_counts = self
            .last_handshake
            .lock()
            .as_ref()
            .map(|hs| hs.port_fan_counts)
            .unwrap_or([0; 4]);

        port_fan_counts
            .iter()
            .enumerate()
            .filter(|(_, &count)| count > 0)
            .map(|(port, &count)| {
                (
                    port as u8,
                    TlFanPortDevice::new(Arc::clone(self), port as u8, count),
                )
            })
            .collect()
    }
}

impl RgbDevice for TlFanPortDevice {
    fn device_name(&self) -> String {
        format!("UNI FAN TL Port {}", self.port)
    }

    fn supported_modes(&self) -> Vec<RgbMode> {
        vec![
            RgbMode::Off,
            RgbMode::Static,
            RgbMode::Rainbow,
            RgbMode::RainbowMorph,
            RgbMode::Breathing,
            RgbMode::Runway,
            RgbMode::Meteor,
            RgbMode::ColorCycle,
            RgbMode::Staggered,
            RgbMode::Tide,
            RgbMode::Mixing,
            RgbMode::Voice,
            RgbMode::Door,
            RgbMode::Render,
            RgbMode::Ripple,
            RgbMode::Reflect,
            RgbMode::TailChasing,
            RgbMode::Paint,
            RgbMode::PingPong,
            RgbMode::Stack,
            RgbMode::CoverCycle,
            RgbMode::Wave,
            RgbMode::Racing,
            RgbMode::Lottery,
            RgbMode::Intertwine,
            RgbMode::MeteorShower,
            RgbMode::Collide,
            RgbMode::ElectricCurrent,
            RgbMode::Kaleidoscope,
        ]
    }

    fn zone_info(&self) -> Vec<RgbZoneInfo> {
        (0..self.fan_count)
            .map(|fan| RgbZoneInfo {
                name: format!("Fan {}", fan + 1),
                led_count: LEDS_PER_FAN,
            })
            .collect()
    }

    fn set_zone_effect(&self, zone: u8, effect: &RgbEffect) -> Result<()> {
        if zone >= self.fan_count {
            bail!(
                "Zone {zone} out of range (port {} has {} fans)",
                self.port,
                self.fan_count
            );
        }

        let base_group = (self.port as u16 * 4 * 2) as u8;
        let scoped = !matches!(effect.scope, RgbScope::All);

        // Per-fan light (0xA3) has no side bits; scoped modes use group light (0xB0).
        if !scoped
            && matches!(
                effect.mode,
                RgbMode::Static | RgbMode::Direct | RgbMode::Off
            )
        {
            return self
                .controller
                .set_fan_light(self.port, zone, effect, false);
        }

        match effect.scope {
            RgbScope::Bottom => self.controller.set_group_light(base_group + 1, effect),
            RgbScope::Top => self.controller.set_group_light(base_group, effect),
            _ => {
                self.controller.set_group_light(base_group, effect)?;
                self.controller.set_group_light(base_group + 1, effect)
            }
        }
    }

    fn supported_scopes(&self) -> Vec<Vec<RgbScope>> {
        vec![vec![RgbScope::All, RgbScope::Top, RgbScope::Bottom]; self.fan_count as usize]
    }

    fn supports_direction(&self) -> bool {
        true
    }

    fn set_fan_direction(&self, zone: u8, swap_lr: bool, swap_tb: bool) -> Result<()> {
        if zone >= self.fan_count {
            bail!(
                "Zone {zone} out of range (port {} has {} fans)",
                self.port,
                self.fan_count
            );
        }
        self.controller
            .set_fan_direction(self.port, zone, swap_lr, swap_tb)
    }

    fn supports_mb_rgb_sync(&self) -> bool {
        true
    }

    fn set_mb_rgb_sync(&self, enabled: bool) -> Result<()> {
        // MB sync is controller-wide — firmware applies it globally regardless of port.
        let port_fan_counts = self
            .controller
            .last_handshake
            .lock()
            .as_ref()
            .map(|hs| hs.port_fan_counts)
            .unwrap_or([0; 4]);

        // Zero-filled effect data is sent when toggling MB sync — the firmware
        // ignores effect fields in sync mode. Match that to avoid stale data.
        let zero_effect = RgbEffect {
            mode: RgbMode::Static,
            colors: vec![],
            speed: 0,
            brightness: 0,
            direction: Default::default(),
            scope: Default::default(),
            disabled: false,
        };
        for (port, &fan_count) in port_fan_counts.iter().enumerate() {
            for fan in 0..fan_count {
                self.controller
                    .set_fan_light(port as u8, fan, &zero_effect, enabled)?;
            }
        }
        debug!("Set MB RGB sync (all ports): enabled={enabled}");
        Ok(())
    }

    fn ping(&self, _zone: u8) -> Result<()> {
        self.controller.test_port_light(self.port)
    }

    fn set_direct_colors(&self, zone: u8, colors: &[[u8; 3]]) -> Result<()> {
        if zone >= self.fan_count {
            bail!("Zone {zone} out of range");
        }
        let color = colors.first().copied().unwrap_or([255, 255, 255]);
        let effect = RgbEffect {
            mode: RgbMode::Static,
            colors: vec![color],
            brightness: 4,
            speed: 0,
            ..RgbEffect::default()
        };
        self.controller
            .set_fan_light_noread(self.port, zone, &effect, false)
    }
}
