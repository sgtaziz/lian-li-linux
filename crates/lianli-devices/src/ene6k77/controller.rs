use super::{Ene6k77Firmware, Ene6k77Model, CMD_DELAY, REPORT_ID};
use crate::traits::FanDevice;
use anyhow::{bail, Context, Result};
use lianli_shared::rgb::{RgbEffect, RgbMode, RgbScope};
use lianli_transport::RusbHid;
use parking_lot::Mutex;
use std::sync::Arc;
use std::thread;
use tracing::{debug, info, warn};

/// ENE 6K77 fan controller.
///
/// Wraps an opened HID device and provides fan speed control, RPM reading,
/// and RGB/LED effects.
pub struct Ene6k77Controller {
    pub(super) device: Arc<Mutex<RusbHid>>,
    pub(super) model: Ene6k77Model,
    pid: u16,
    firmware: Option<Ene6k77Firmware>,
    /// Number of fans configured per group [group0, group1, group2, group3].
    fan_quantities: Mutex<[u8; 4]>,
}

impl Ene6k77Controller {
    pub fn new(device: Arc<Mutex<RusbHid>>, pid: u16) -> Result<Self> {
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
        let fw = Ene6k77Firmware {
            model: self.model,
            customer_id: data[0],
            project_id: data[1],
            major_id: data[2],
            minor_id: data[3],
            fine_tune: data[4],
        };
        if !fw.is_valid() {
            warn!(
                "Firmware ID mismatch for {}: got cust={:#04x} proj={:#04x} major={:#04x} minor={:#04x}, expected {:?}",
                self.model.name(),
                fw.customer_id,
                fw.project_id,
                fw.major_id,
                fw.minor_id,
                self.model.expected_firmware_ids(),
            );
        }
        Ok(fw)
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

        let speed_byte = self.map_speed(effect.speed);
        let dir_byte = effect.direction.to_ene_byte();
        let brightness_byte = self.map_brightness(effect.brightness);

        if self.model.uses_double_port() {
            let inner_port = group * 2;
            let outer_port = group * 2 + 1;
            let inner_mode = self.map_mode_inner(effect.mode);
            match effect.scope {
                RgbScope::Inner => {
                    self.send_ring_colors(inner_port, effect, 8)?;
                    self.send_effect(
                        inner_port,
                        inner_mode,
                        speed_byte,
                        dir_byte,
                        brightness_byte,
                    )?;
                }
                RgbScope::Outer => {
                    if let Some(outer_mode) = self.map_mode_outer(effect.mode) {
                        self.send_ring_colors(outer_port, effect, 12)?;
                        self.send_effect(
                            outer_port,
                            outer_mode,
                            speed_byte,
                            dir_byte,
                            brightness_byte,
                        )?;
                    }
                }
                _ => {
                    self.send_ring_colors(inner_port, effect, 8)?;
                    self.send_effect(
                        inner_port,
                        inner_mode,
                        speed_byte,
                        dir_byte,
                        brightness_byte,
                    )?;
                    if let Some(outer_mode) = self.map_mode_outer(effect.mode) {
                        self.send_ring_colors(outer_port, effect, 12)?;
                        self.send_effect(
                            outer_port,
                            outer_mode,
                            speed_byte,
                            dir_byte,
                            brightness_byte,
                        )?;
                    }
                }
            }
        } else {
            let mode_byte = self.map_mode_to_ene(effect.mode);
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
            "Set group {group}: colors={:?} speed={speed_byte} dir={dir_byte} brightness={brightness_byte} scope={:?}",
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
            if leds_per_fan == 12 && effect.colors.len() >= 2 {
                // Outer-ring "colorful" expansion: 4 corners × 3 LEDs each
                expand_outer_corner(&effect.colors, max_fans)
            } else {
                expand_per_led(&effect.colors, max_fans, leds_per_fan)
            }
        } else if matches!(effect.mode, RgbMode::Meteor)
            && matches!(self.model, Ene6k77Model::AlV2Fan)
        {
            // ALV2Fan Meteor cycleFill: wrap palette modulo instead of black padding
            expand_palette_cycle(&effect.colors, max_fans, palette)
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
        match self.model {
            Ene6k77Model::SlInfinity => map_mode_sl_inf(mode),
            Ene6k77Model::AlFan | Ene6k77Model::AlV2Fan => map_mode_al_inner(mode),
            // Single-ring models (SL Fan, SL V2, SL V2a, SL Redragon).
            _ => match mode {
                RgbMode::Off => 0,
                RgbMode::Static => 1,
                RgbMode::Breathing => 2,
                RgbMode::RainbowMorph => 4,
                RgbMode::Rainbow => 5,
                RgbMode::Runway => 28,
                RgbMode::Meteor => 36,
                RgbMode::ColorCycle => 35,
                RgbMode::Staggered => 24,
                RgbMode::Tide => 26,
                RgbMode::Mixing => 30,
                RgbMode::Stack => 32,
                RgbMode::StackMulti => 33,
                RgbMode::Neon => 34,
                RgbMode::Voice => 38,
                RgbMode::Groove => 39,
                RgbMode::Render => 40,
                RgbMode::Tunnel => 41,
                _ => 1,
            },
        }
    }

    /// Resolve the inner-ring mode byte for any double-port model.
    fn map_mode_inner(&self, mode: RgbMode) -> u8 {
        map_mode_inner_for(self.model, mode)
    }

    /// Resolve the outer-ring mode byte for any double-port model. Returns
    /// `None` when the mode has no outer-ring variant.
    fn map_mode_outer(&self, mode: RgbMode) -> Option<u8> {
        map_mode_outer_for(self.model, mode)
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

    /// Map brightness scale to ENE byte. ENE: Off(8), Lowest(4), Lower(3),
    /// Normal(2), Higher(1), Highest(0). `BRIGHTNESS_OFF` sentinel → 8.
    fn map_brightness(&self, brightness: u8) -> u8 {
        if lianli_shared::rgb::is_brightness_off(brightness) {
            return 8;
        }
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

    /// Enter merge-lighting mode (SLFan/SLRedragon only).
    pub fn start_merge(&self) -> Result<()> {
        self.send_feature(&[REPORT_ID, 0x10, 0x33, 0x00, 0x01, 0x02, 0x03, 0x08])?;
        thread::sleep(CMD_DELAY);
        Ok(())
    }

    /// Exit merge-lighting mode (SLFan/SLRedragon only).
    pub fn stop_merge(&self) -> Result<()> {
        self.send_feature(&[REPORT_ID, 0x10, 0x34, 0x00, 0x00, 0x00])?;
        thread::sleep(CMD_DELAY);
        Ok(())
    }

    /// Set merge group order (SLV2/SLV2A/ALV2/SLInfinity).
    pub fn set_merge_order(&self, order: [u8; 4]) -> Result<()> {
        self.send_feature(&[
            REPORT_ID, 0x10, 0x63, order[0], order[1], order[2], order[3], 0x08,
        ])?;
        thread::sleep(CMD_DELAY);
        Ok(())
    }

    /// Send merge command (ALFan only — distinct from StartMerge/StopMerge).
    pub fn send_merge_command(&self, enable: bool) -> Result<()> {
        self.send_feature(&[REPORT_ID, 0x10, 0x43, enable as u8, 0x00, 0x00])?;
        thread::sleep(CMD_DELAY);
        Ok(())
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

    fn stop_pwm(&self) -> u8 {
        1
    }
}

fn expand_per_led(ui: &[[u8; 3]], num_fans: usize, leds_per_fan: usize) -> Vec<[u8; 3]> {
    let mut out = vec![[0u8; 3]; num_fans * leds_per_fan];
    for fan in 0..num_fans {
        let c = ui
            .get(fan)
            .copied()
            .or_else(|| ui.first().copied())
            .unwrap_or([0, 0, 0]);
        for led in 0..leds_per_fan {
            out[fan * leds_per_fan + led] = c;
        }
    }
    out
}

fn expand_palette(ui: &[[u8; 3]], num_fans: usize, palette: usize) -> Vec<[u8; 3]> {
    let mut out = vec![[0u8; 3]; num_fans * palette];
    for fan in 0..num_fans {
        for slot in 0..palette {
            out[fan * palette + slot] = ui.get(slot).copied().unwrap_or([0, 0, 0]);
        }
    }
    out
}

/// Outer-corner expansion: 12 outer LEDs per fan split into 4 corners × 3 LEDs,
/// each corner gets a different user color.
fn expand_outer_corner(ui: &[[u8; 3]], num_fans: usize) -> Vec<[u8; 3]> {
    let mut out = vec![[0u8; 3]; num_fans * 12];
    for fan in 0..num_fans {
        for corner in 0..4 {
            let c = ui.get(corner).copied().unwrap_or([0, 0, 0]);
            for led in 0..3 {
                out[fan * 12 + corner * 3 + led] = c;
            }
        }
    }
    out
}

/// Cycle-fill palette expansion: wraps the user palette modulo slot index
/// instead of padding with black. Used exclusively for Meteor mode on ALV2Fan.
fn expand_palette_cycle(ui: &[[u8; 3]], num_fans: usize, palette: usize) -> Vec<[u8; 3]> {
    if ui.is_empty() {
        return vec![[0, 0, 0]; num_fans * palette];
    }
    let mut out = vec![[0u8; 3]; num_fans * palette];
    for fan in 0..num_fans {
        for slot in 0..palette {
            out[fan * palette + slot] = ui[slot % ui.len()];
        }
    }
    out
}

fn map_mode_inner_for(model: Ene6k77Model, mode: RgbMode) -> u8 {
    match model {
        Ene6k77Model::SlInfinity => map_mode_sl_inf(mode),
        _ => map_mode_al_inner(mode),
    }
}

fn map_mode_outer_for(model: Ene6k77Model, mode: RgbMode) -> Option<u8> {
    match model {
        Ene6k77Model::SlInfinity => Some(map_mode_sl_inf(mode)),
        Ene6k77Model::AlFan | Ene6k77Model::AlV2Fan => map_mode_al_outer(model, mode),
        _ => None,
    }
}

fn map_mode_al_inner(mode: RgbMode) -> u8 {
    match mode {
        RgbMode::Off => 0,
        RgbMode::Static => 1,
        RgbMode::Breathing => 2,
        RgbMode::RainbowMorph => 4,
        RgbMode::Rainbow => 5,
        RgbMode::BreathingRainbow => 6,
        RgbMode::MeteorRainbow => 8,
        RgbMode::ColorCycle => 24,
        RgbMode::Meteor => 25,
        RgbMode::Runway => 26,
        RgbMode::MopUp => 27,
        RgbMode::Lottery => 29,
        RgbMode::Wave => 30,
        RgbMode::Spring => 31,
        RgbMode::TailChasing => 32,
        RgbMode::Warning => 33,
        RgbMode::Voice => 34,
        RgbMode::Mixing => 35,
        RgbMode::Stack => 36,
        RgbMode::Tide => 37,
        RgbMode::Scan => 38,
        RgbMode::PacMan => 39,
        RgbMode::ColorfulCity => 40,
        RgbMode::Render => 41,
        RgbMode::Twinkle => 42,
        _ => 1,
    }
}

fn map_mode_al_outer(model: Ene6k77Model, mode: RgbMode) -> Option<u8> {
    let byte = match model {
        Ene6k77Model::AlFan => match mode {
            RgbMode::Off => 0,
            RgbMode::Static => 1,
            RgbMode::Breathing => 2,
            RgbMode::BreathingRainbow => 6,
            RgbMode::Rainbow => 40,
            RgbMode::RainbowMorph => 53,
            RgbMode::ColorCycle => 43,
            RgbMode::TaiChi => 44,
            RgbMode::Meteor => 25,
            RgbMode::Runway => 26,
            RgbMode::Warning => 45,
            RgbMode::Voice => 46,
            RgbMode::SpanningTeacups => 56,
            RgbMode::Tornado => 54,
            RgbMode::Mixing => 47,
            RgbMode::Stack => 48,
            RgbMode::Staggered => 55,
            RgbMode::Tide => 49,
            RgbMode::Scan => 50,
            RgbMode::Contest => 51,
            _ => return None,
        },
        Ene6k77Model::AlV2Fan => match mode {
            RgbMode::Off => 0,
            RgbMode::Static => 1,
            RgbMode::Breathing => 2,
            RgbMode::RainbowMorph => 4,
            RgbMode::BreathingRainbow => 6,
            RgbMode::Meteor => 25,
            RgbMode::Runway => 26,
            RgbMode::MopUp => 62,
            RgbMode::Rainbow => 43,
            RgbMode::ColorCycle => 46,
            RgbMode::TaiChi => 47,
            RgbMode::Warning => 48,
            RgbMode::Voice => 49,
            RgbMode::Mixing => 50,
            RgbMode::Tide => 51,
            RgbMode::Scan => 52,
            RgbMode::Contest => 53,
            RgbMode::ColorfulCity => 56,
            RgbMode::Render => 57,
            RgbMode::Twinkle => 58,
            RgbMode::Wave => 59,
            RgbMode::Spring => 60,
            RgbMode::TailChasing => 61,
            RgbMode::Tornado => 63,
            RgbMode::Staggered => 64,
            RgbMode::SpanningTeacups => 65,
            RgbMode::ElectricCurrent => 66,
            RgbMode::Stack => 67,
            _ => return None,
        },
        _ => return None,
    };
    Some(byte)
}

fn map_mode_sl_inf(mode: RgbMode) -> u8 {
    match mode {
        RgbMode::Off => 0,
        RgbMode::Static => 1,
        RgbMode::Breathing => 2,
        RgbMode::RainbowMorph => 4,
        RgbMode::Rainbow => 5,
        RgbMode::BreathingRainbow => 6,
        RgbMode::MeteorRainbow => 8,
        RgbMode::ColorCycle => 24,
        RgbMode::Meteor => 25,
        RgbMode::Runway => 26,
        RgbMode::MopUp => 68,
        RgbMode::DoubleMeteor => 29,
        RgbMode::MeteorContest => 30,
        RgbMode::MeteorMix => 31,
        RgbMode::ReturnArc => 32,
        RgbMode::DoubleArc => 33,
        RgbMode::Door => 34,
        RgbMode::Disco => 35,
        RgbMode::HeartBeat => 36,
        RgbMode::Lottery => 38,
        RgbMode::Warning => 41,
        RgbMode::Voice => 42,
        RgbMode::Mixing => 43,
        RgbMode::Stack => 44,
        RgbMode::Tide => 45,
        RgbMode::Scan => 46,
        RgbMode::HeartBeatRunway => 69,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slv2_palette_expansion_matches_vendor_byte_count() {
        // 6 fans × 4 colors = 24 bytes regardless of user palette length.
        let ui = vec![[10, 20, 30], [40, 50, 60], [70, 80, 90], [100, 110, 120]];
        let out = expand_palette(&ui, 6, 4);
        assert_eq!(out.len(), 24);
        for fan in 0..6 {
            for slot in 0..4 {
                assert_eq!(out[fan * 4 + slot], ui[slot]);
            }
        }
    }

    #[test]
    fn slv2_led_expansion_fans_five_and_six_receive_color() {
        // Fans beyond the user color list receive the first color, not black.
        let ui = vec![[1, 2, 3]];
        let out = expand_per_led(&ui, 6, 16);
        assert_eq!(out.len(), 96);
        for fan in 0..6 {
            for led in 0..16 {
                assert_eq!(out[fan * 16 + led], [1, 2, 3]);
            }
        }
    }

    #[test]
    fn slv2_palette_pads_missing_slots_with_black() {
        // Missing palette slots are padded with black per fan.
        let ui = vec![[10, 20, 30]];
        let out = expand_palette(&ui, 6, 4);
        assert_eq!(out.len(), 24);
        for fan in 0..6 {
            assert_eq!(out[fan * 4 + 0], [10, 20, 30]);
            for slot in 1..4 {
                assert_eq!(out[fan * 4 + slot], [0, 0, 0]);
            }
        }
    }

    #[test]
    fn alfan_outer_rainbow_uses_outer_byte() {
        assert_eq!(
            map_mode_outer_for(Ene6k77Model::AlFan, RgbMode::Rainbow),
            Some(40)
        );
        assert_eq!(
            map_mode_outer_for(Ene6k77Model::AlFan, RgbMode::RainbowMorph),
            Some(53)
        );
        assert_eq!(
            map_mode_outer_for(Ene6k77Model::AlFan, RgbMode::TaiChi),
            Some(44)
        );
        assert_eq!(
            map_mode_outer_for(Ene6k77Model::AlFan, RgbMode::SpanningTeacups),
            Some(56)
        );
        assert_eq!(
            map_mode_outer_for(Ene6k77Model::AlFan, RgbMode::Tornado),
            Some(54)
        );
        assert_eq!(
            map_mode_outer_for(Ene6k77Model::AlFan, RgbMode::Staggered),
            Some(55)
        );
        assert_eq!(
            map_mode_outer_for(Ene6k77Model::AlFan, RgbMode::Contest),
            Some(51)
        );
        assert_eq!(
            map_mode_outer_for(Ene6k77Model::AlFan, RgbMode::BreathingRainbow),
            Some(6)
        );
    }

    #[test]
    fn alfan_outer_returns_none_for_modes_without_outer_variant() {
        assert_eq!(
            map_mode_outer_for(Ene6k77Model::AlFan, RgbMode::Lottery),
            None
        );
        assert_eq!(
            map_mode_outer_for(Ene6k77Model::AlFan, RgbMode::PacMan),
            None
        );
    }

    #[test]
    fn alv2_outer_mode_bytes_match_expected() {
        let m = Ene6k77Model::AlV2Fan;
        assert_eq!(map_mode_outer_for(m, RgbMode::Rainbow), Some(43));
        assert_eq!(map_mode_outer_for(m, RgbMode::MopUp), Some(62));
        assert_eq!(map_mode_outer_for(m, RgbMode::Wave), Some(59));
        assert_eq!(map_mode_outer_for(m, RgbMode::Spring), Some(60));
        assert_eq!(map_mode_outer_for(m, RgbMode::TailChasing), Some(61));
        assert_eq!(map_mode_outer_for(m, RgbMode::Warning), Some(48));
        assert_eq!(map_mode_outer_for(m, RgbMode::Voice), Some(49));
        assert_eq!(map_mode_outer_for(m, RgbMode::Mixing), Some(50));
        assert_eq!(map_mode_outer_for(m, RgbMode::Stack), Some(67));
        assert_eq!(map_mode_outer_for(m, RgbMode::Tide), Some(51));
        assert_eq!(map_mode_outer_for(m, RgbMode::Scan), Some(52));
        assert_eq!(map_mode_outer_for(m, RgbMode::ColorfulCity), Some(56));
        assert_eq!(map_mode_outer_for(m, RgbMode::Render), Some(57));
        assert_eq!(map_mode_outer_for(m, RgbMode::Twinkle), Some(58));
        assert_eq!(map_mode_outer_for(m, RgbMode::TaiChi), Some(47));
        assert_eq!(map_mode_outer_for(m, RgbMode::SpanningTeacups), Some(65));
        assert_eq!(map_mode_outer_for(m, RgbMode::Tornado), Some(63));
        assert_eq!(map_mode_outer_for(m, RgbMode::Staggered), Some(64));
        assert_eq!(map_mode_outer_for(m, RgbMode::ElectricCurrent), Some(66));
        assert_eq!(map_mode_outer_for(m, RgbMode::Contest), Some(53));
    }

    #[test]
    fn sl_infinity_mopup_uses_top_level_byte() {
        assert_eq!(
            map_mode_inner_for(Ene6k77Model::SlInfinity, RgbMode::MopUp),
            68
        );
        assert_eq!(
            map_mode_outer_for(Ene6k77Model::SlInfinity, RgbMode::MopUp),
            Some(68)
        );
    }
}
