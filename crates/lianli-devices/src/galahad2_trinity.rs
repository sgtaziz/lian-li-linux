//! Galahad II Trinity AIO driver (pump + fan control, NO LCD).
//!
//! VID=0x0416, PID=0x7371 (Performance) / 0x7373 (Regular)
//!
//! Protocol uses 64-byte HID output reports (Report ID 0x01) with 6-byte header.
//! Pump and fan PWM use 0-100% scale (not 0-255).
//! RPM is read via the handshake command (0x81).
//! No coolant temperature sensor — CPU temp must come from the system.

use crate::traits::{AioDevice, FanDevice, RgbDevice};
use anyhow::{bail, Context, Result};
use lianli_shared::rgb::{RgbEffect, RgbMode, RgbScope, RgbZoneInfo};
use lianli_transport::HidBackend;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{debug, info, warn};

const REPORT_ID: u8 = 0x01;
const PACKET_SIZE: usize = 64;
const HEADER_LEN: usize = 6;
const READ_TIMEOUT_MS: i32 = 200;

// A-Commands — Control
const CMD_HANDSHAKE: u8 = 0x81;
const CMD_GET_FIRMWARE: u8 = 0x86;
const CMD_SET_PUMP_PWM: u8 = 0x8A;
const CMD_SET_FAN_PWM: u8 = 0x8B;

// A-Commands — LED
const CMD_SET_PUMP_LIGHT: u8 = 0x83;
const CMD_SET_FAN_LIGHT: u8 = 0x85;

/// Default number of LEDs on each radiator fan.
const FAN_LED_COUNT: u16 = 24;

/// Galahad2 Trinity model variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Galahad2TrinityModel {
    /// PID 0x7371 — Performance (pump RPM 2200-4200)
    Performance,
    /// PID 0x7373 — Regular (pump RPM 2200-3200)
    Regular,
}

impl Galahad2TrinityModel {
    pub fn from_pid(pid: u16) -> Option<Self> {
        match pid {
            0x7371 => Some(Self::Performance),
            0x7373 => Some(Self::Regular),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Performance => "Galahad II Trinity Performance",
            Self::Regular => "Galahad II Trinity",
        }
    }
}

/// Handshake response data.
#[derive(Debug, Clone)]
pub struct Galahad2Handshake {
    pub fan_rpm: u16,
    pub pump_rpm: u16,
}

/// Galahad II Trinity AIO controller.
///
/// Provides pump + fan speed control and RGB/LED effects.
/// Does NOT have LCD or coolant temp sensor.
pub struct Galahad2TrinityController {
    device: Arc<Mutex<HidBackend>>,
    model: Galahad2TrinityModel,
    last_handshake: Option<Galahad2Handshake>,
    mb_sync: AtomicBool,
}

impl Galahad2TrinityController {
    pub fn new(device: Arc<Mutex<HidBackend>>, pid: u16) -> Result<Self> {
        let model = Galahad2TrinityModel::from_pid(pid)
            .ok_or_else(|| anyhow::anyhow!("Unknown Galahad2 Trinity PID: {pid:#06x}"))?;

        let mut ctrl = Self {
            device,
            model,
            last_handshake: None,
            mb_sync: AtomicBool::new(false),
        };

        ctrl.initialize()?;
        Ok(ctrl)
    }

    fn initialize(&mut self) -> Result<()> {
        info!(
            "Initializing {} (PID={:#06x})",
            self.model.name(),
            match self.model {
                Galahad2TrinityModel::Performance => 0x7371u16,
                Galahad2TrinityModel::Regular => 0x7373u16,
            }
        );

        match self.read_firmware() {
            Ok(fw) => info!("  Firmware: {fw}"),
            Err(e) => warn!("  Failed to read firmware: {e}"),
        }

        match self.handshake() {
            Ok(hs) => {
                info!("  Fan RPM: {}, Pump RPM: {}", hs.fan_rpm, hs.pump_rpm);
            }
            Err(e) => warn!("  Handshake failed: {e}"),
        }

        Ok(())
    }

    /// Perform handshake to read fan and pump RPM.
    pub fn handshake(&mut self) -> Result<Galahad2Handshake> {
        self.device.lock().read_flush();
        let resp = self.send_a_command(CMD_HANDSHAKE, &[])?;
        let data = &resp[HEADER_LEN..];
        let data_len = resp[5] as usize;

        if data_len < 4 {
            bail!("Galahad2 Trinity: handshake response too short ({data_len} bytes)");
        }

        let hs = Galahad2Handshake {
            fan_rpm: u16::from_be_bytes([data[0], data[1]]),
            pump_rpm: u16::from_be_bytes([data[2], data[3]]),
        };

        debug!("Handshake: fan={}rpm pump={}rpm", hs.fan_rpm, hs.pump_rpm);
        self.last_handshake = Some(hs.clone());
        Ok(hs)
    }

    fn read_firmware(&self) -> Result<String> {
        let mut dev = self.device.lock();
        dev.read_flush();

        let mut pkt = [0u8; PACKET_SIZE];
        pkt[0] = REPORT_ID;
        pkt[1] = CMD_GET_FIRMWARE;
        dev.write(&pkt)
            .context("Galahad2 Trinity: write firmware request")?;

        // Response 1: version string
        let mut buf = [0u8; PACKET_SIZE];
        let n = dev
            .read_timeout(&mut buf, READ_TIMEOUT_MS)
            .context("Galahad2 Trinity: read firmware")?;
        if n == 0 {
            bail!("Galahad2 Trinity: no firmware response");
        }
        let data_len = buf[5] as usize;
        let data = &buf[HEADER_LEN..HEADER_LEN + data_len.min(58)];
        let version_str = String::from_utf8_lossy(data)
            .trim_end_matches('\0')
            .to_string();

        // Response 2: date/time string (must be consumed to keep buffer in sync)
        let n2 = dev.read_timeout(&mut buf, READ_TIMEOUT_MS).unwrap_or(0);
        if n2 > 0 {
            let len2 = buf[5] as usize;
            let data2 = &buf[HEADER_LEN..HEADER_LEN + len2.min(58)];
            let date_str = String::from_utf8_lossy(data2)
                .trim_end_matches('\0')
                .to_string();
            debug!("Firmware date: {date_str}");
        }

        Ok(version_str)
    }

    pub fn model(&self) -> Galahad2TrinityModel {
        self.model
    }

    /// Set pump LED effect.
    ///
    /// Uses CMD_SET_PUMP_LIGHT (0x83) with 19-byte payload:
    /// ```text
    /// [0]  = scope (0=Inner, 1=Outer, 2=All)
    /// [1]  = mode % 1000
    /// [2]  = brightness (0-4)
    /// [3]  = speed (0-4)
    /// [4-15] = R,G,B × 4 colors
    /// [16] = direction (0-5)
    /// [17] = disabled (0=enabled, 1=disabled)
    /// [18] = ARGB source (0=MCU, 1=Motherboard)
    /// ```
    pub fn set_pump_light(&self, effect: &RgbEffect, source_mcu: bool) -> Result<()> {
        let scope = match effect.scope {
            RgbScope::Inner => 0u8,
            RgbScope::Outer => 1,
            _ => 2, // All
        };

        let mode_byte = effect.mode.to_tl_mode_byte().unwrap_or(3);

        let mut payload = [0u8; 19];
        payload[0] = scope;
        payload[1] = mode_byte;
        payload[2] = effect.brightness.min(4);
        payload[3] = effect.speed.min(4);

        for (i, color) in effect.colors.iter().take(4).enumerate() {
            let offset = 4 + i * 3;
            payload[offset] = color[0];
            payload[offset + 1] = color[1];
            payload[offset + 2] = color[2];
        }

        payload[16] = effect.direction.to_tl_byte();
        payload[17] = if effect.mode == RgbMode::Off { 1 } else { 0 };
        payload[18] = if source_mcu { 0 } else { 1 };

        self.send_a_command(CMD_SET_PUMP_LIGHT, &payload)?;
        debug!("Set pump light: mode={mode_byte} scope={scope}");
        Ok(())
    }

    /// Set radiator fan LED effect.
    ///
    /// Uses CMD_SET_FAN_LIGHT (0x85) with 20-byte payload:
    /// ```text
    /// [0]  = mode % 1000
    /// [1]  = brightness (0-4)
    /// [2]  = speed (0-4)
    /// [3-14] = R,G,B × 4 colors
    /// [15] = direction (0-5)
    /// [16] = disabled (0=enabled, 1=disabled)
    /// [17] = ARGB source (0=MCU, 1=Motherboard)
    /// [18] = sync to pump (0=independent, 1=sync)
    /// [19] = number of LEDs (default 24)
    /// ```
    pub fn set_fan_light(
        &self,
        effect: &RgbEffect,
        source_mcu: bool,
        sync_to_pump: bool,
    ) -> Result<()> {
        let mode_byte = effect.mode.to_tl_mode_byte().unwrap_or(3);

        let mut payload = [0u8; 20];
        payload[0] = mode_byte;
        payload[1] = effect.brightness.min(4);
        payload[2] = effect.speed.min(4);

        for (i, color) in effect.colors.iter().take(4).enumerate() {
            let offset = 3 + i * 3;
            payload[offset] = color[0];
            payload[offset + 1] = color[1];
            payload[offset + 2] = color[2];
        }

        payload[15] = effect.direction.to_tl_byte();
        payload[16] = if effect.mode == RgbMode::Off { 1 } else { 0 };
        payload[17] = if source_mcu { 0 } else { 1 };
        payload[18] = sync_to_pump as u8;
        payload[19] = FAN_LED_COUNT as u8;

        self.send_a_command(CMD_SET_FAN_LIGHT, &payload)?;
        debug!("Set fan light: mode={mode_byte} sync_to_pump={sync_to_pump}");
        Ok(())
    }

    fn send_a_command(&self, cmd: u8, data: &[u8]) -> Result<Vec<u8>> {
        let mut pkt = [0u8; PACKET_SIZE];
        pkt[0] = REPORT_ID;
        pkt[1] = cmd;
        pkt[5] = data.len() as u8;
        let copy_len = data.len().min(58);
        pkt[HEADER_LEN..HEADER_LEN + copy_len].copy_from_slice(&data[..copy_len]);

        let mut dev = self.device.lock();
        dev.write(&pkt).context("Galahad2 Trinity: write")?;

        let mut buf = [0u8; PACKET_SIZE];
        let n = dev
            .read_timeout(&mut buf, READ_TIMEOUT_MS)
            .context("Galahad2 Trinity: read")?;

        if n == 0 {
            bail!("Galahad2 Trinity: no response to command {cmd:#04x}");
        }

        Ok(buf[..n].to_vec())
    }
}

fn duty_to_percent(duty: u8) -> u8 {
    ((duty as u32 * 100) / 255) as u8
}

impl FanDevice for Galahad2TrinityController {
    fn set_fan_speed(&self, _slot: u8, duty: u8) -> Result<()> {
        let pwm = duty_to_percent(duty);
        let mb = self.mb_sync.load(Ordering::Relaxed) as u8;
        self.send_a_command(CMD_SET_FAN_PWM, &[mb, pwm])?;
        debug!("Set fan PWM to {pwm}% (mb_sync={mb})");
        Ok(())
    }

    fn set_fan_speeds(&self, duties: &[u8]) -> Result<()> {
        if let Some(&duty) = duties.first() {
            self.set_fan_speed(0, duty)?;
        }
        Ok(())
    }

    fn read_fan_rpm(&self) -> Result<Vec<u16>> {
        Ok(vec![self
            .last_handshake
            .as_ref()
            .map(|hs| hs.fan_rpm)
            .unwrap_or(0)])
    }

    fn fan_slot_count(&self) -> u8 {
        1
    }

    fn supports_mb_sync(&self) -> bool {
        true
    }

    fn set_mb_rpm_sync(&self, _port: u8, sync: bool) -> Result<()> {
        self.mb_sync.store(sync, Ordering::Relaxed);
        Ok(())
    }

    fn has_pump_control(&self) -> bool {
        true
    }

    fn set_pump_speed(&self, duty: u8) -> Result<()> {
        let pwm = duty_to_percent(duty);
        let mb = self.mb_sync.load(Ordering::Relaxed) as u8;
        self.send_a_command(CMD_SET_PUMP_PWM, &[mb, pwm])?;
        debug!("Set pump PWM to {pwm}% (mb_sync={mb})");
        Ok(())
    }
}

impl AioDevice for Galahad2TrinityController {
    fn read_pump_rpm(&self) -> Result<u16> {
        Ok(self
            .last_handshake
            .as_ref()
            .map(|hs| hs.pump_rpm)
            .unwrap_or(0))
    }

    fn read_coolant_temp(&self) -> Result<f32> {
        bail!("Galahad2 Trinity does not have a coolant temperature sensor")
    }
}

/// Galahad2 Trinity LED zones:
///   Zone 0 = Pump head (inner + outer LEDs)
///   Zone 1 = Radiator fans
impl RgbDevice for Galahad2TrinityController {
    fn device_name(&self) -> String {
        "Galahad II Trinity AIO".to_string()
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
            RgbMode::Ripple,
            RgbMode::Reflect,
            RgbMode::TailChasing,
            RgbMode::Paint,
            RgbMode::PingPong,
            RgbMode::BigBang,
            RgbMode::Vortex,
            RgbMode::Pump,
            RgbMode::ColorsMorph,
        ]
    }

    fn zone_info(&self) -> Vec<RgbZoneInfo> {
        vec![
            RgbZoneInfo {
                name: "Pump Head".to_string(),
                led_count: 24, // Approximate pump LED count
            },
            RgbZoneInfo {
                name: "Fans".to_string(),
                led_count: FAN_LED_COUNT,
            },
        ]
    }

    fn set_zone_effect(&self, zone: u8, effect: &RgbEffect) -> Result<()> {
        match zone {
            0 => self.set_pump_light(effect, true),
            1 => self.set_fan_light(effect, true, false),
            _ => bail!("Galahad2 Trinity: zone {zone} out of range (0-1)"),
        }
    }

    fn supported_scopes(&self) -> Vec<Vec<RgbScope>> {
        vec![
            vec![RgbScope::All, RgbScope::Inner, RgbScope::Outer], // Pump Head
            vec![],                                                // Fans (All only)
        ]
    }

    fn supports_mb_rgb_sync(&self) -> bool {
        true
    }

    fn set_mb_rgb_sync(&self, enabled: bool) -> Result<()> {
        let source_mcu = !enabled;
        let dummy = RgbEffect::default();
        self.set_pump_light(&dummy, source_mcu)?;
        self.set_fan_light(&dummy, source_mcu, false)?;
        debug!("Set MB RGB sync: enabled={enabled}");
        Ok(())
    }
}
