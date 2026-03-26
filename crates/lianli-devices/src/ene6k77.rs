//! ENE 6K77 wired fan controller driver (SL/AL series).
//!
//! VID=0x0CF2, PID=0xA100-0xA106
//!
//! Protocol uses HID Feature Reports with Report ID 0xE0.
//! Each controller has 4 fan groups with independent PWM duty control.
//! RPM is read via feature report 0x50 sub-command 0x00.

use crate::traits::{FanDevice, RgbDevice};
use anyhow::{bail, Context, Result};
use lianli_shared::rgb::{RgbEffect, RgbMode, RgbScope, RgbZoneInfo};
use lianli_transport::HidBackend;
use parking_lot::Mutex;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tracing::{debug, info, warn};

const REPORT_ID: u8 = 0xE0;
const CMD_DELAY: Duration = Duration::from_millis(20);

/// ENE 6K77 model variant, determined by PID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ene6k77Model {
    /// 0xA100 — SL Fan (4 groups, 4 fans max each)
    SlFan,
    /// 0xA101 — AL Fan (4 groups, dual-ring LEDs)
    AlFan,
    /// 0xA102 — SL Infinity (4 groups)
    SlInfinity,
    /// 0xA103 — SL V2 Fan (4 groups, 6 fans max each)
    SlV2Fan,
    /// 0xA104 — AL V2 Fan (4 groups, 6 fans max each)
    AlV2Fan,
    /// 0xA105 — SL V2A Fan
    SlV2aFan,
    /// 0xA106 — SL Redragon
    SlRedragon,
}

impl Ene6k77Model {
    pub fn from_pid(pid: u16) -> Option<Self> {
        match pid {
            0xA100 => Some(Self::SlFan),
            0xA101 => Some(Self::AlFan),
            0xA102 => Some(Self::SlInfinity),
            0xA103 => Some(Self::SlV2Fan),
            0xA104 => Some(Self::AlV2Fan),
            0xA105 => Some(Self::SlV2aFan),
            0xA106 => Some(Self::SlRedragon),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::SlFan => "SL Fan",
            Self::AlFan => "AL Fan",
            Self::SlInfinity => "SL Infinity",
            Self::SlV2Fan => "SL V2 Fan",
            Self::AlV2Fan => "AL V2 Fan",
            Self::SlV2aFan => "SL V2A Fan",
            Self::SlRedragon => "SL Redragon",
        }
    }

    /// Whether this is a V2 model (supports 6 fans/group, 9-byte RPM response).
    pub fn is_v2(&self) -> bool {
        matches!(self, Self::SlV2Fan | Self::AlV2Fan | Self::SlV2aFan)
    }

    /// Whether this model uses doubled port encoding (0x10|(group*2) for effects).
    pub fn uses_double_port(&self) -> bool {
        matches!(self, Self::AlFan | Self::AlV2Fan | Self::SlInfinity)
    }

    /// Max fans per group.
    pub fn max_fans_per_group(&self) -> u8 {
        if self.is_v2() { 6 } else { 4 }
    }
}

/// Firmware version info read from the device.
#[derive(Debug, Clone)]
pub struct Ene6k77Firmware {
    pub customer_id: u8,
    pub project_id: u8,
    pub major_id: u8,
    pub minor_id: u8,
    pub fine_tune: u8,
}

impl std::fmt::Display for Ene6k77Firmware {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let version = if self.fine_tune < 8 {
            "1.0".to_string()
        } else {
            let v = ((self.fine_tune >> 4) * 10 + (self.fine_tune & 0x0F) + 2) as f32 / 10.0;
            format!("{v:.1}")
        };
        write!(
            f,
            "v{} (cust={:#04x} proj={:#04x} major={:#04x} minor={:#04x})",
            version, self.customer_id, self.project_id, self.major_id, self.minor_id
        )
    }
}

/// ENE 6K77 fan controller.
///
/// Wraps an opened HID device and provides fan speed control, RPM reading,
/// and RGB/LED effects.
pub struct Ene6k77Controller {
    device: Arc<Mutex<HidBackend>>,
    model: Ene6k77Model,
    pid: u16,
    firmware: Option<Ene6k77Firmware>,
    /// Number of fans configured per group [group0, group1, group2, group3].
    fan_quantities: [u8; 4],
}

impl Ene6k77Controller {
    /// Open an ENE 6K77 controller by HID device handle and PID.
    pub fn new(device: Arc<Mutex<HidBackend>>, pid: u16) -> Result<Self> {
        let model = Ene6k77Model::from_pid(pid)
            .ok_or_else(|| anyhow::anyhow!("Unknown ENE 6K77 PID: {pid:#06x}"))?;

        let mut ctrl = Self {
            device,
            model,
            pid,
            firmware: None,
            fan_quantities: [0; 4],
        };

        ctrl.initialize()?;
        Ok(ctrl)
    }

    /// Initialize the controller: read firmware version.
    fn initialize(&mut self) -> Result<()> {
        info!(
            "Initializing ENE 6K77 {} (PID={:#06x})",
            self.model.name(),
            self.pid
        );

        match self.read_firmware() {
            Ok(fw) => {
                info!("  Firmware: {fw}");
                self.firmware = Some(fw);
            }
            Err(e) => {
                warn!("  Failed to read firmware: {e}");
            }
        }

        let max_fans = self.model.max_fans_per_group();
        for group in 0..4u8 {
            if let Err(e) = self.set_fan_quantity(group, max_fans) {
                warn!("  Failed to set group {group} fan quantity: {e}");
            }
        }

        Ok(())
    }

    /// Read firmware version from the device.
    fn read_firmware(&self) -> Result<Ene6k77Firmware> {
        self.send_feature(&[REPORT_ID, 0x50, 0x01])?;
        thread::sleep(CMD_DELAY);
        let data = self.read_input(5)?;
        Ok(Ene6k77Firmware {
            customer_id: data[0],
            project_id: data[1],
            major_id: data[2],
            minor_id: data[3],
            fine_tune: data[4],
        })
    }

    /// Set fan quantity for a group.
    ///
    /// This tells the controller how many fans are connected to each group,
    /// which affects RPM reporting accuracy.
    pub fn set_fan_quantity(&mut self, group: u8, quantity: u8) -> Result<()> {
        if group >= 4 {
            bail!("Group index {group} out of range (0-3)");
        }
        let max = self.model.max_fans_per_group();
        let qty = quantity.min(max);

        let cmd = match self.model {
            Ene6k77Model::AlFan => {
                vec![REPORT_ID, 0x10, 0x40, group + 1, qty, 0x00]
            }
            Ene6k77Model::AlV2Fan | Ene6k77Model::SlInfinity => {
                vec![REPORT_ID, 0x10, 0x60, group + 1, qty, 0x00]
            }
            Ene6k77Model::SlV2Fan | Ene6k77Model::SlV2aFan => {
                vec![REPORT_ID, 0x10, 0x60, (group << 4) | (qty & 0x0F)]
            }
            _ => {
                vec![REPORT_ID, 0x10, 0x32, (group << 4) | (qty & 0x0F)]
            }
        };

        self.send_feature(&cmd)?;
        self.fan_quantities[group as usize] = qty;
        debug!(
            "Set group {group} fan quantity to {qty} (model={})",
            self.model.name()
        );
        thread::sleep(CMD_DELAY);
        Ok(())
    }

    /// Read RPM values for all 4 groups.
    ///
    /// Returns [group0_rpm, group1_rpm, group2_rpm, group3_rpm].
    pub fn read_rpms(&self) -> Result<[u16; 4]> {
        self.send_feature(&[REPORT_ID, 0x50, 0x00])?;
        thread::sleep(CMD_DELAY);

        let mut rpms = [0u16; 4];

        if self.model.is_v2() {
            // V2 models return 9 bytes (1 padding + 4x2 RPM)
            let data = self.read_input(9)?;
            for i in 0..4 {
                let offset = 1 + i * 2;
                rpms[i] = u16::from_be_bytes([data[offset], data[offset + 1]]);
            }
        } else {
            // Standard models return 8 bytes (4x2 RPM)
            let data = self.read_input(8)?;
            for i in 0..4 {
                let offset = i * 2;
                rpms[i] = u16::from_be_bytes([data[offset], data[offset + 1]]);
            }
        }

        Ok(rpms)
    }

    /// Set fan speed (PWM duty) for a single group.
    ///
    /// `group`: 0-3
    /// `duty`: 0-255 (0% to 100%)
    pub fn set_group_speed(&self, group: u8, duty: u8) -> Result<()> {
        if group >= 4 {
            bail!("Group index {group} out of range (0-3)");
        }

        // [0xE0, 0x2G, 0x00, DUTY] where G = group index
        self.send_feature(&[REPORT_ID, 0x20 | group, 0x00, duty])?;
        debug!("Set group {group} speed to duty={duty} ({:.0}%)", duty as f32 / 2.55);
        thread::sleep(CMD_DELAY);
        Ok(())
    }

    /// Set fan speeds for all 4 groups at once.
    pub fn set_all_speeds(&self, duties: &[u8; 4]) -> Result<()> {
        for (group, &duty) in duties.iter().enumerate() {
            self.set_group_speed(group as u8, duty)?;
        }
        Ok(())
    }

    pub fn model(&self) -> Ene6k77Model {
        self.model
    }

    pub fn pid(&self) -> u16 {
        self.pid
    }

    pub fn firmware(&self) -> Option<&Ene6k77Firmware> {
        self.firmware.as_ref()
    }

    /// Number of LEDs per fan for this model.
    pub fn leds_per_fan(&self) -> u16 {
        match self.model {
            Ene6k77Model::SlFan | Ene6k77Model::SlRedragon => 16,
            Ene6k77Model::SlV2Fan | Ene6k77Model::SlV2aFan => 16,
            Ene6k77Model::AlFan => 20,   // 8 inner + 12 outer
            Ene6k77Model::AlV2Fan => 20,  // 8 inner + 12 outer
            Ene6k77Model::SlInfinity => 20, // 8 inner + 12 outer
        }
    }

    /// Set LED effect for a group.
    ///
    /// **NOTE**: ENE uses R,B,G byte order (not R,G,B)!
    pub fn set_group_effect(&self, group: u8, effect: &RgbEffect) -> Result<()> {
        if group >= 4 {
            bail!("Group index {group} out of range (0-3)");
        }

        let mode_byte = self.map_mode_to_ene(effect.mode);
        let speed_byte = self.map_speed(effect.speed);
        let dir_byte = effect.direction.to_ene_byte();
        let brightness_byte = self.map_brightness(effect.brightness);

        if self.model.uses_double_port() {
            let inner_port = group * 2;
            let outer_port = group * 2 + 1;
            match effect.scope {
                RgbScope::Inner => {
                    self.send_ring_colors(inner_port, effect, 8)?;
                    self.send_effect(inner_port, mode_byte, speed_byte, dir_byte, brightness_byte)?;
                }
                RgbScope::Outer => {
                    self.send_ring_colors(outer_port, effect, 12)?;
                    self.send_effect(outer_port, mode_byte, speed_byte, dir_byte, brightness_byte)?;
                }
                _ => {
                    self.send_ring_colors(inner_port, effect, 8)?;
                    self.send_effect(inner_port, mode_byte, speed_byte, dir_byte, brightness_byte)?;
                    self.send_ring_colors(outer_port, effect, 12)?;
                    self.send_effect(outer_port, mode_byte, speed_byte, dir_byte, brightness_byte)?;
                }
            }
        } else {
            self.send_port_effect(group, effect, mode_byte, speed_byte, dir_byte, brightness_byte)?;
        }

        // Commit frame to display changes
        self.send_feature(&[REPORT_ID, 0x60, 0x00, 0x01])?;
        thread::sleep(CMD_DELAY);

        debug!(
            "Set group {group}: colors={:?} mode={mode_byte} speed={speed_byte} dir={dir_byte} brightness={brightness_byte} scope={:?}",
            &effect.colors, effect.scope
        );
        Ok(())
    }

    /// Send color + effect for SL (single-ring) models.
    fn send_port_effect(&self, port: u8, effect: &RgbEffect, mode: u8, speed: u8, dir: u8, brightness: u8) -> Result<()> {
        let mut color_cmd = vec![REPORT_ID, 0x30 | port];
        for color in effect.colors.iter().take(4) {
            color_cmd.push(color[0]); // R
            color_cmd.push(color[2]); // B
            color_cmd.push(color[1]); // G
        }
        while color_cmd.len() < 14 {
            color_cmd.push(0);
        }
        match self.send_output(&color_cmd) {
            Ok(()) => debug!("Port {port}: wrote {} color bytes", color_cmd.len()),
            Err(e) => warn!("Port {port}: color output report failed: {e}"),
        }
        thread::sleep(CMD_DELAY);
        self.send_effect(port, mode, speed, dir, brightness)
    }

    /// Send expanded color data for a dual-ring port (inner=8 LEDs/fan, outer=12 LEDs/fan).
    fn send_ring_colors(&self, port: u8, effect: &RgbEffect, leds_per_fan: usize) -> Result<()> {
        let mut color_cmd = vec![REPORT_ID, 0x30 | port];
        let last_color = effect.colors.last().copied().unwrap_or([0, 0, 0]);
        for i in 0..6usize {
            let color = effect.colors.get(i).copied().unwrap_or(last_color);
            for _ in 0..leds_per_fan {
                color_cmd.push(color[0]); // R
                color_cmd.push(color[2]); // B
                color_cmd.push(color[1]); // G
            }
        }
        match self.send_output(&color_cmd) {
            Ok(()) => debug!("Port {port}: wrote {} color bytes ({leds_per_fan} LEDs/fan)", color_cmd.len()),
            Err(e) => warn!("Port {port}: color output report failed: {e}"),
        }
        thread::sleep(CMD_DELAY);
        Ok(())
    }

    fn send_effect(&self, port: u8, mode: u8, speed: u8, dir: u8, brightness: u8) -> Result<()> {
        self.send_feature(&[REPORT_ID, 0x10 | port, mode, speed, dir, brightness])?;
        thread::sleep(CMD_DELAY);
        Ok(())
    }

    /// Map RgbMode to ENE mode byte.
    fn map_mode_to_ene(&self, mode: RgbMode) -> u8 {
        match mode {
            RgbMode::Off => 0,
            RgbMode::Static => 1,
            RgbMode::Breathing => 2,
            RgbMode::ColorCycle => 3,
            RgbMode::Rainbow => 4,
            RgbMode::Runway => 5,
            RgbMode::Meteor => 6,
            RgbMode::Staggered => 7,
            RgbMode::Tide => 8,
            RgbMode::Mixing => 9,
            _ => 1, // Default to Static for unsupported modes
        }
    }

    /// Map 0-4 speed scale to ENE speed byte.
    /// ENE: Lowest(2), Lower(1), Normal(0), Faster(255), Fastest(254)
    fn map_speed(&self, speed: u8) -> u8 {
        match speed {
            0 => 2,   // Lowest
            1 => 1,   // Lower
            2 => 0,   // Normal
            3 => 255, // Faster
            4 => 254, // Fastest
            _ => 0,
        }
    }

    /// Map 0-4 brightness scale to ENE brightness byte.
    /// ENE: Off(8), Lowest(4), Lower(3), Normal(2), Higher(1), Highest(0)
    fn map_brightness(&self, brightness: u8) -> u8 {
        match brightness {
            0 => 4, // Lowest
            1 => 3, // Lower
            2 => 2, // Normal
            3 => 1, // Higher
            4 => 0, // Highest
            _ => 2,
        }
    }

    fn send_feature(&self, data: &[u8]) -> Result<()> {
        let dev = self.device.lock();
        dev.send_feature_report(data)
            .context("ENE 6K77: send feature report")?;
        Ok(())
    }

    fn send_output(&self, data: &[u8]) -> Result<()> {
        let dev = self.device.lock();
        dev.write(data)
            .context("ENE 6K77: send output report")?;
        Ok(())
    }

    fn read_input(&self, expected_len: usize) -> Result<Vec<u8>> {
        let dev = self.device.lock();
        let mut buf = vec![0u8; 65];
        buf[0] = REPORT_ID;
        let n = dev
            .get_input_report(&mut buf)
            .context("ENE 6K77: get input report")?;
        if n < expected_len {
            bail!(
                "ENE 6K77: expected {expected_len} bytes, got {n}"
            );
        }
        Ok(buf[1..=expected_len].to_vec())
    }
}

impl FanDevice for Ene6k77Controller {
    fn set_fan_speed(&self, slot: u8, duty: u8) -> Result<()> {
        self.set_group_speed(slot, duty)
    }

    fn set_fan_speeds(&self, duties: &[u8]) -> Result<()> {
        for (i, &duty) in duties.iter().take(4).enumerate() {
            self.set_group_speed(i as u8, duty)?;
        }
        Ok(())
    }

    fn read_fan_rpm(&self) -> Result<Vec<u16>> {
        Ok(self.read_rpms()?.to_vec())
    }

    fn fan_slot_count(&self) -> u8 {
        4
    }

    fn fan_port_info(&self) -> Vec<(u8, u8)> {
        (0..4).map(|g| (g, self.fan_quantities[g as usize].max(1))).collect()
    }

    fn per_fan_control(&self) -> bool {
        false
    }

    fn supports_mb_sync(&self) -> bool {
        true
    }

    fn set_mb_rpm_sync(&self, group: u8, sync: bool) -> Result<()> {
        if group >= 4 {
            bail!("Group index {group} out of range (0-3)");
        }
        let sub_cmd = match self.model {
            Ene6k77Model::SlFan | Ene6k77Model::SlRedragon => 0x31,
            Ene6k77Model::AlFan => 0x42,
            Ene6k77Model::SlV2Fan | Ene6k77Model::SlV2aFan
            | Ene6k77Model::AlV2Fan | Ene6k77Model::SlInfinity => 0x62,
        };
        let data = (1u8 << (group + 4)) | ((sync as u8) << group);
        self.send_feature(&[REPORT_ID, 0x10, sub_cmd, data, 0x00, 0x00])?;
        debug!("Set group {group} MB RPM sync to {sync}");
        thread::sleep(CMD_DELAY);
        Ok(())
    }
}

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
        format!("UNI FAN {} Group {}", self.controller.model.name(), self.group)
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
        let fans = self.controller.model.max_fans_per_group();
        let leds_per_fan = self.controller.leds_per_fan();
        (0..fans)
            .map(|fan| RgbZoneInfo {
                name: format!("Fan {}", fan + 1),
                led_count: leds_per_fan,
            })
            .collect()
    }

    fn supported_scopes(&self) -> Vec<Vec<RgbScope>> {
        let fans = self.controller.model.max_fans_per_group() as usize;
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
            Ene6k77Model::SlV2Fan | Ene6k77Model::SlV2aFan
            | Ene6k77Model::AlV2Fan | Ene6k77Model::SlInfinity => 0x61,
        };
        self.controller.send_feature(&[REPORT_ID, 0x10, sub_cmd, enabled as u8, 0, 0])?;
        thread::sleep(CMD_DELAY);
        Ok(())
    }
}

/// `Arc<Ene6k77Controller>` can be used directly as a `FanDevice`.
/// This allows the same controller instance to serve both fan and RGB.
impl FanDevice for Arc<Ene6k77Controller> {
    fn set_fan_speed(&self, slot: u8, duty: u8) -> Result<()> {
        (**self).set_fan_speed(slot, duty)
    }
    fn set_fan_speeds(&self, duties: &[u8]) -> Result<()> {
        (**self).set_fan_speeds(duties)
    }
    fn read_fan_rpm(&self) -> Result<Vec<u16>> {
        (**self).read_fan_rpm()
    }
    fn fan_slot_count(&self) -> u8 {
        (**self).fan_slot_count()
    }
    fn fan_port_info(&self) -> Vec<(u8, u8)> {
        (**self).fan_port_info()
    }
    fn per_fan_control(&self) -> bool {
        (**self).per_fan_control()
    }
    fn supports_mb_sync(&self) -> bool {
        (**self).supports_mb_sync()
    }
    fn set_mb_rpm_sync(&self, port: u8, sync: bool) -> Result<()> {
        (**self).set_mb_rpm_sync(port, sync)
    }
}
