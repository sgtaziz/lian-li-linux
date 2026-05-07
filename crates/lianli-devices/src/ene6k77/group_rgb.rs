use super::controller::Ene6k77Controller;
use super::{Ene6k77Model, CMD_DELAY, REPORT_ID};
use crate::traits::RgbDevice;
use anyhow::Result;
use lianli_shared::rgb::{RgbEffect, RgbMode, RgbScope, RgbZoneInfo};
use std::sync::Arc;
use std::thread;

/// Per-group RGB device wrapper — each physical group appears as a separate device.
pub struct Ene6k77GroupDevice {
    controller: Arc<Ene6k77Controller>,
    group: u8,
}

impl Ene6k77GroupDevice {
    pub fn new(controller: Arc<Ene6k77Controller>, group: u8) -> Self {
        Self { controller, group }
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
        vec![
            RgbMode::Off,
            RgbMode::Static,
            RgbMode::Breathing,
            RgbMode::ColorCycle,
            RgbMode::Rainbow,
            RgbMode::Runway,
            RgbMode::Meteor,
            RgbMode::Staggered,
            RgbMode::Tide,
            RgbMode::Mixing,
        ]
    }

    fn zone_info(&self) -> Vec<RgbZoneInfo> {
        let fans = self.controller.fan_quantity(self.group).max(1);
        let leds_per_fan = self.controller.leds_per_fan();
        (0..fans)
            .map(|fan| RgbZoneInfo {
                name: format!("Fan {}", fan + 1),
                led_count: leds_per_fan,
            })
            .collect()
    }

    fn supported_scopes(&self) -> Vec<Vec<RgbScope>> {
        let fans = self.controller.fan_quantity(self.group).max(1) as usize;
        if self.controller.model.uses_double_port() {
            vec![vec![RgbScope::All, RgbScope::Inner, RgbScope::Outer]; fans]
        } else {
            vec![vec![]; fans]
        }
    }

    fn set_zone_effect(&self, _zone: u8, effect: &RgbEffect) -> Result<()> {
        // ENE applies effects per-group (all fans same mode/speed/brightness).
        // Scope routes to inner/outer/both ports for dual-ring models.
        self.controller.set_group_effect(self.group, effect)
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
}
