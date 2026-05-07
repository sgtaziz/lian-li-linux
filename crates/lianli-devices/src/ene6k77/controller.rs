use super::{Ene6k77Firmware, Ene6k77Model, CMD_DELAY, REPORT_ID};
use crate::traits::FanDevice;
use anyhow::{bail, Context, Result};
use lianli_shared::rgb::{RgbEffect, RgbMode, RgbScope};
use lianli_transport::HidBackend;
use parking_lot::Mutex;
use std::sync::Arc;
use std::thread;
use tracing::{debug, info, warn};

/// ENE 6K77 fan controller.
///
/// Wraps an opened HID device and provides fan speed control, RPM reading,
/// and RGB/LED effects.
pub struct Ene6k77Controller {
    pub(super) device: Arc<Mutex<HidBackend>>,
    pub(super) model: Ene6k77Model,
    pid: u16,
    firmware: Option<Ene6k77Firmware>,
    /// Number of fans configured per group [group0, group1, group2, group3].
    fan_quantities: Mutex<[u8; 4]>,
}

impl Ene6k77Controller {
    pub fn new(device: Arc<Mutex<HidBackend>>, pid: u16) -> Result<Self> {
        let model = Ene6k77Model::from_pid(pid)
            .ok_or_else(|| anyhow::anyhow!("Unknown ENE 6K77 PID: {pid:#06x}"))?;

        let mut ctrl = Self {
            device,
            model,
            pid,
            firmware: None,
            fan_quantities: Mutex::new([0; 4]),
        };

        ctrl.initialize()?;
        Ok(ctrl)
    }

    fn initialize(&mut self) -> Result<()> {
        info!(
            "Initializing ENE 6K77 {} (PID={:#06x})",
            self.model.name(),
            self.pid
        );

        for attempt in 1..=3 {
            match self.read_firmware() {
                Ok(fw) => {
                    info!("  Firmware: {fw}");
                    self.firmware = Some(fw);
                    break;
                }
                Err(e) => {
                    warn!("  Firmware read attempt {attempt}/3 failed: {e}");
                    if attempt < 3 {
                        thread::sleep(std::time::Duration::from_secs(2));
                    }
                }
            }
        }

        let max = self.model.max_fans_per_group();
        let default_qty = 3u8.min(max);
        for group in 0..4u8 {
            if let Err(e) = self.set_fan_quantity(group, default_qty) {
                warn!("  Failed to set group {group} fan quantity: {e}");
            }
        }

        Ok(())
    }

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

    /// Set fan quantity for a group. Tells the controller how many fans are
    /// connected, which affects RPM reporting accuracy.
    pub fn set_fan_quantity(&self, group: u8, quantity: u8) -> Result<()> {
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
                vec![
                    REPORT_ID,
                    0x10,
                    0x60,
                    (group << 4) | (qty & 0x0F),
                    0x00,
                    0x00,
                ]
            }
            _ => {
                vec![
                    REPORT_ID,
                    0x10,
                    0x32,
                    (group << 4) | (qty & 0x0F),
                    0x00,
                    0x00,
                ]
            }
        };

        self.send_feature(&cmd)?;
        self.fan_quantities.lock()[group as usize] = qty;
        debug!(
            "Set group {group} fan quantity to {qty} (model={})",
            self.model.name()
        );
        thread::sleep(CMD_DELAY);
        Ok(())
    }

    /// Read RPM values for all 4 groups.
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

    pub fn set_group_speed(&self, group: u8, duty: u8) -> Result<()> {
        if group >= 4 {
            bail!("Group index {group} out of range (0-3)");
        }
        self.send_feature(&[REPORT_ID, 0x20 | group, 0x00, duty])?;
        debug!(
            "Set group {group} speed to duty={duty} ({:.0}%)",
            duty as f32 / 2.55
        );
        thread::sleep(CMD_DELAY);
        Ok(())
    }

    /// Set fan speeds for all 4 groups atomically (single lock hold so RGB
    /// writes from another thread can't interleave between groups).
    pub fn set_all_speeds(&self, duties: &[u8; 4]) -> Result<()> {
        let mut dev = self.device.lock();
        for (group, &duty) in duties.iter().enumerate() {
            let data = [REPORT_ID, 0x20 | (group as u8), 0x00, duty];
            dev.send_feature_report(&data)
                .with_context(|| format!("ENE set group {group} speed"))?;
            debug!(
                "Set group {group} speed to duty={duty} ({:.0}%)",
                duty as f32 / 2.55
            );
            thread::sleep(CMD_DELAY);
        }
        Ok(())
    }

    pub fn fan_quantity(&self, group: u8) -> u8 {
        self.fan_quantities.lock()[group as usize]
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
            Ene6k77Model::AlFan => 20,
            Ene6k77Model::AlV2Fan => 20,
            Ene6k77Model::SlInfinity => 20,
        }
    }

    /// Set LED effect for a group.
    ///
    /// **NOTE**: ENE uses R,B,G byte order (not R,G,B).
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
            self.send_port_effect(
                group,
                effect,
                mode_byte,
                speed_byte,
                dir_byte,
                brightness_byte,
            )?;
        }

        let qty = self.fan_quantity(group);
        if let Err(e) = self.set_fan_quantity(group, qty) {
            debug!("re-affirm fan quantity for group {group}: {e:#}");
        }

        let frame = self.model.frame_commit_value();
        self.send_feature(&[REPORT_ID, 0x60, (frame >> 8) as u8, frame as u8])?;
        thread::sleep(CMD_DELAY);

        debug!(
            "Set group {group}: colors={:?} mode={mode_byte} speed={speed_byte} dir={dir_byte} brightness={brightness_byte} scope={:?}",
            &effect.colors, effect.scope
        );
        Ok(())
    }

    fn send_port_effect(
        &self,
        port: u8,
        effect: &RgbEffect,
        mode: u8,
        speed: u8,
        dir: u8,
        brightness: u8,
    ) -> Result<()> {
        let max_fans = self.model.max_fans_per_group() as usize;
        let leds_per_fan = self.model.single_ring_leds_per_fan();
        let palette = self.model.palette_size();

        let colors = if matches!(effect.mode, RgbMode::Static | RgbMode::Breathing) {
            expand_per_led(&effect.colors, max_fans, leds_per_fan)
        } else {
            expand_palette(&effect.colors, max_fans, palette)
        };

        self.send_color_setting(port, &colors)?;
        thread::sleep(CMD_DELAY);
        self.send_effect(port, mode, speed, dir, brightness)
    }

    fn send_ring_colors(&self, port: u8, effect: &RgbEffect, leds_per_fan: usize) -> Result<()> {
        let max_fans = self.model.max_fans_per_group() as usize;
        let palette = self.model.palette_size();

        let colors = if matches!(effect.mode, RgbMode::Static | RgbMode::Breathing) {
            expand_per_led(&effect.colors, max_fans, leds_per_fan)
        } else {
            expand_palette(&effect.colors, max_fans, palette)
        };

        self.send_color_setting(port, &colors)?;
        thread::sleep(CMD_DELAY);
        Ok(())
    }

    fn send_color_setting(&self, port: u8, colors: &[[u8; 3]]) -> Result<()> {
        let mut buf = Vec::with_capacity(2 + colors.len() * 3);
        buf.push(REPORT_ID);
        buf.push(0x30 | port);
        for c in colors {
            buf.push(c[0]); // R
            buf.push(c[2]); // B
            buf.push(c[1]); // G
        }
        match self.send_output(&buf) {
            Ok(()) => debug!("Port {port}: wrote {} color bytes", buf.len()),
            Err(e) => warn!("Port {port}: color output report failed: {e}"),
        }
        Ok(())
    }

    fn send_effect(&self, port: u8, mode: u8, speed: u8, dir: u8, brightness: u8) -> Result<()> {
        self.send_feature(&[REPORT_ID, 0x10 | port, mode, speed, dir, brightness])?;
        thread::sleep(CMD_DELAY);
        Ok(())
    }

    fn map_mode_to_ene(&self, mode: RgbMode) -> u8 {
        if self.model.uses_double_port() {
            // Inner/outer ring models (SL Infinity, AL Fan, AL V2).
            // Inner and outer ports share the same mode byte values.
            match mode {
                RgbMode::Off => 0,
                RgbMode::Static => 1,
                RgbMode::Breathing => 2,
                RgbMode::ColorCycle => 24,
                RgbMode::Rainbow => 5,
                RgbMode::Runway => 26,
                RgbMode::Meteor => 25,
                RgbMode::Tide => 45,
                RgbMode::Mixing => 43,
                _ => 1,
            }
        } else {
            // Single-ring models (SL Fan, SL V2, SL V2a, SL Redragon).
            match mode {
                RgbMode::Off => 0,
                RgbMode::Static => 1,
                RgbMode::Breathing => 2,
                RgbMode::ColorCycle => 35,
                RgbMode::Rainbow => 5,
                RgbMode::Runway => 28,
                RgbMode::Meteor => 36,
                RgbMode::Staggered => 24,
                RgbMode::Tide => 26,
                RgbMode::Mixing => 30,
                _ => 1,
            }
        }
    }

    /// Map 0-4 speed scale to ENE byte. ENE: Lowest(2), Lower(1), Normal(0),
    /// Faster(255), Fastest(254).
    fn map_speed(&self, speed: u8) -> u8 {
        match speed {
            0 => 2,
            1 => 1,
            2 => 0,
            3 => 255,
            4 => 254,
            _ => 0,
        }
    }

    /// Map 0-4 brightness scale to ENE byte. ENE: Off(8), Lowest(4), Lower(3),
    /// Normal(2), Higher(1), Highest(0).
    fn map_brightness(&self, brightness: u8) -> u8 {
        match brightness {
            0 => 4,
            1 => 3,
            2 => 2,
            3 => 1,
            4 => 0,
            _ => 2,
        }
    }

    pub(super) fn send_feature(&self, data: &[u8]) -> Result<()> {
        let mut dev = self.device.lock();
        dev.send_feature_report(data)
            .context("ENE 6K77: send feature report")?;
        Ok(())
    }

    fn send_output(&self, data: &[u8]) -> Result<()> {
        let mut dev = self.device.lock();
        dev.write(data).context("ENE 6K77: send output report")?;
        Ok(())
    }

    fn read_input(&self, expected_len: usize) -> Result<Vec<u8>> {
        let mut dev = self.device.lock();
        let mut buf = vec![0u8; 65];
        buf[0] = REPORT_ID;
        let n = dev
            .get_input_report(&mut buf)
            .context("ENE 6K77: get input report")?;
        if n < expected_len {
            bail!("ENE 6K77: expected {expected_len} bytes, got {n}");
        }
        Ok(buf[1..=expected_len].to_vec())
    }
}

impl FanDevice for Ene6k77Controller {
    fn set_fan_speed(&self, slot: u8, duty: u8) -> Result<()> {
        self.set_group_speed(slot, duty)
    }

    fn set_fan_speeds(&self, duties: &[u8]) -> Result<()> {
        let mut arr = [0u8; 4];
        for (i, &d) in duties.iter().take(4).enumerate() {
            arr[i] = d;
        }
        self.set_all_speeds(&arr)
    }

    fn read_fan_rpm(&self) -> Result<Vec<u16>> {
        Ok(self.read_rpms()?.to_vec())
    }

    fn fan_slot_count(&self) -> u8 {
        4
    }

    fn fan_port_info(&self) -> Vec<(u8, u8)> {
        let qtys = *self.fan_quantities.lock();
        (0..4).map(|g| (g, qtys[g as usize])).collect()
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
            Ene6k77Model::SlV2Fan
            | Ene6k77Model::SlV2aFan
            | Ene6k77Model::AlV2Fan
            | Ene6k77Model::SlInfinity => 0x62,
        };
        let data = (1u8 << (group + 4)) | ((sync as u8) << group);
        self.send_feature(&[REPORT_ID, 0x10, sub_cmd, data, 0x00, 0x00])?;
        debug!("Set group {group} MB RPM sync to {sync}");
        thread::sleep(CMD_DELAY);
        Ok(())
    }

    fn supports_fan_quantity(&self) -> bool {
        true
    }

    fn max_fan_quantity_per_port(&self) -> u8 {
        self.model.max_fans_per_group()
    }

    fn set_port_fan_quantity(&self, port: u8, quantity: u8) -> Result<()> {
        self.set_fan_quantity(port, quantity)
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
    fn supports_fan_quantity(&self) -> bool {
        (**self).supports_fan_quantity()
    }
    fn max_fan_quantity_per_port(&self) -> u8 {
        (**self).max_fan_quantity_per_port()
    }
    fn set_port_fan_quantity(&self, port: u8, quantity: u8) -> Result<()> {
        (**self).set_port_fan_quantity(port, quantity)
    }
}

fn expand_per_led(ui: &[[u8; 3]], num_fans: usize, leds_per_fan: usize) -> Vec<[u8; 3]> {
    let fallback = ui.last().copied().unwrap_or([0, 0, 0]);
    let mut out = vec![[0u8; 3]; num_fans * leds_per_fan];
    for fan in 0..num_fans {
        let c = ui.get(fan).copied().unwrap_or(fallback);
        for led in 0..leds_per_fan {
            out[fan * leds_per_fan + led] = c;
        }
    }
    out
}

fn expand_palette(ui: &[[u8; 3]], num_fans: usize, palette: usize) -> Vec<[u8; 3]> {
    let fallback = ui.last().copied().unwrap_or([0, 0, 0]);
    let mut out = vec![[0u8; 3]; num_fans * palette];
    for fan in 0..num_fans {
        for slot in 0..palette {
            out[fan * palette + slot] = ui.get(slot).copied().unwrap_or(fallback);
        }
    }
    out
}
