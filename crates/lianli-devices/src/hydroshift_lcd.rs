//! HydroShift LCD / Galahad2 LCD / Vision AIO driver (pump + fan + LCD + temp).
//!
//! HydroShift LCD:   VID=0x0416, PID=0x7398/0x7399/0x739A
//! Galahad2 LCD:     VID=0x0416, PID=0x7391
//! Galahad2 Vision:  VID=0x0416, PID=0x7395
//!
//! All use an identical protocol with three HID report types:
//!   A-command (64B, Report ID 1): pump/fan PWM, handshake, firmware
//!   B-command (1024B out, Report ID 2): LCD control, JPEG frames
//!   C-command (512B, Report ID 3): LCD frames (firmware >= 1.2)
//!
//! LCD: 480x480 pixels, 24fps. Pump/fan PWM: 0-100%.
//! Coolant temperature sensor available.

use crate::traits::{AioDevice, FanDevice, LcdDevice, RgbDevice};
use anyhow::{bail, Context, Result};
use lianli_shared::rgb::{RgbEffect, RgbMode, RgbScope, RgbZoneInfo};
use lianli_shared::screen::ScreenInfo;
use lianli_transport::HidBackend;
use parking_lot::Mutex;
use std::sync::Arc;
use tracing::{debug, info, warn};

// Report IDs
const REPORT_ID_A: u8 = 0x01;
const REPORT_ID_B: u8 = 0x02;
const REPORT_ID_C: u8 = 0x03;

// Packet sizes
const A_PACKET_SIZE: usize = 64;
const A_HEADER_LEN: usize = 6;
const B_PACKET_SIZE: usize = 1024;
const B_HEADER_LEN: usize = 11;
const B_MAX_PAYLOAD: usize = B_PACKET_SIZE - B_HEADER_LEN; // 1013
const C_PACKET_SIZE: usize = 512;
const C_HEADER_LEN: usize = 11;
const C_MAX_PAYLOAD: usize = C_PACKET_SIZE - C_HEADER_LEN; // 501

const READ_TIMEOUT_MS: i32 = 1000;
const INIT_READ_TIMEOUT_MS: i32 = 3000;

// A-Commands
const CMD_HANDSHAKE: u8 = 0x81;
const CMD_GET_FIRMWARE: u8 = 0x86;
const CMD_SET_PUMP_PWM: u8 = 0x8A;
const CMD_SET_FAN_PWM: u8 = 0x8B;

// B-Commands
const CMD_LCD_CONTROL: u8 = 0x0C;
const CMD_SEND_JPEG: u8 = 0x0E;
const CMD_LCD_AVAILABLE: u8 = 0x17;

// A-Commands (device management)
const CMD_RESET_DEVICE: u8 = 0x8E;

/// AIO LCD device variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AioLcdVariant {
    /// HydroShift LCD (0x7398)
    HydroShiftLcd,
    /// HydroShift LCD RGB (0x7399)
    HydroShiftLcdRgb,
    /// HydroShift LCD TL (0x739A)
    HydroShiftLcdTl,
    /// Galahad2 LCD (0x7391)
    Galahad2Lcd,
    /// Galahad2 Vision (0x7395)
    Galahad2Vision,
}

impl AioLcdVariant {
    pub fn from_pid(pid: u16) -> Option<Self> {
        match pid {
            0x7398 => Some(Self::HydroShiftLcd),
            0x7399 => Some(Self::HydroShiftLcdRgb),
            0x739A => Some(Self::HydroShiftLcdTl),
            0x7391 => Some(Self::Galahad2Lcd),
            0x7395 => Some(Self::Galahad2Vision),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::HydroShiftLcd => "HydroShift LCD",
            Self::HydroShiftLcdRgb => "HydroShift LCD RGB",
            Self::HydroShiftLcdTl => "HydroShift LCD TL",
            Self::Galahad2Lcd => "Galahad II LCD",
            Self::Galahad2Vision => "Galahad II Vision",
        }
    }

    /// Whether this variant has pump head RGB (SetPumpLighting 0x83).
    /// Galahad2 LCD + Vision have pump RGB; HydroShift variants do not.
    pub fn has_pump_rgb(&self) -> bool {
        matches!(self, Self::Galahad2Lcd | Self::Galahad2Vision)
    }
}

/// LCD control mode.
#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum LcdControlMode {
    LocalUi = 0,
    Application = 1,
    LocalH264 = 2,
    LocalAvi = 3,
    LcdSetting = 4,
    LcdTest = 5,
}

/// Screen rotation.
#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum ScreenRotation {
    Rotate0 = 0,
    Rotate90 = 1,
    Rotate180 = 2,
    Rotate270 = 3,
}

impl ScreenRotation {
    pub fn from_degrees(degrees: u16) -> Self {
        match degrees {
            90 => Self::Rotate90,
            180 => Self::Rotate180,
            270 => Self::Rotate270,
            _ => Self::Rotate0,
        }
    }
}

/// Handshake response: RPM + temperature.
#[derive(Debug, Clone)]
pub struct AioHandshake {
    pub fan_rpm: u16,
    pub pump_rpm: u16,
    pub temp_valid: bool,
    pub coolant_temp: f32,
}

/// HydroShift LCD / Galahad2 LCD AIO controller.
///
/// Provides pump + fan speed control, coolant temperature reading, and LCD streaming.
pub struct HydroShiftLcdController {
    device: Arc<Mutex<HidBackend>>,
    variant: AioLcdVariant,
    last_handshake: Option<AioHandshake>,
    brightness: u8,
    rotation: ScreenRotation,
    initialized: bool,
    use_c_command: bool,
}

impl HydroShiftLcdController {
    pub fn new(device: Arc<Mutex<HidBackend>>, pid: u16) -> Result<Self> {
        let variant = AioLcdVariant::from_pid(pid)
            .ok_or_else(|| anyhow::anyhow!("Unknown AIO LCD PID: {pid:#06x}"))?;

        let mut ctrl = Self {
            device,
            variant,
            last_handshake: None,
            brightness: 50,
            rotation: ScreenRotation::Rotate0,
            initialized: false,
            use_c_command: false,
        };

        ctrl.init()?;
        Ok(ctrl)
    }

    fn init(&mut self) -> Result<()> {
        if self.initialized {
            return Ok(());
        }
        info!("Initializing {}", self.variant.name());

        match self.read_firmware_internal(INIT_READ_TIMEOUT_MS) {
            Ok(fw) => {
                if let Some(ver) = parse_firmware_version(&fw) {
                    if ver > (1, 2) {
                        info!("  Firmware: {fw} (using 512-byte frame mode)");
                        self.use_c_command = true;
                    } else {
                        info!("  Firmware: {fw}");
                    }
                } else {
                    info!("  Firmware: {fw}");
                }
            }
            Err(e) => warn!("  Failed to read firmware: {e:#}"),
        }

        match self.handshake() {
            Ok(hs) => {
                info!(
                    "  Fan RPM: {}, Pump RPM: {}, Temp: {:.1}°C (valid={})",
                    hs.fan_rpm, hs.pump_rpm, hs.coolant_temp, hs.temp_valid
                );
            }
            Err(e) => warn!("  Handshake failed: {e:#}"),
        }

        std::thread::sleep(std::time::Duration::from_secs(2));

        self.initialized = true;
        Ok(())
    }

    /// Perform handshake to read RPM and temperature.
    pub fn handshake(&mut self) -> Result<AioHandshake> {
        let timeout = if self.initialized {
            READ_TIMEOUT_MS
        } else {
            INIT_READ_TIMEOUT_MS
        };
        let resp = self.send_a_command(CMD_HANDSHAKE, &[], timeout)?;
        let data = &resp[A_HEADER_LEN..];
        let data_len = resp[5] as usize;

        if data_len < 4 {
            bail!("AIO LCD: handshake response too short ({data_len} bytes)");
        }

        let temp_valid = data_len >= 5 && data[4] != 0;
        let coolant_temp = if data_len >= 7 {
            let integer = data[5] as f32;
            let fraction = (data[6] % 10) as f32 / 10.0;
            integer + fraction
        } else {
            0.0
        };

        let hs = AioHandshake {
            fan_rpm: u16::from_be_bytes([data[0], data[1]]),
            pump_rpm: u16::from_be_bytes([data[2], data[3]]),
            temp_valid,
            coolant_temp,
        };

        debug!(
            "Handshake: fan={}rpm pump={}rpm temp={:.1}°C",
            hs.fan_rpm, hs.pump_rpm, hs.coolant_temp
        );
        self.last_handshake = Some(hs.clone());
        Ok(hs)
    }

    /// Set LCD to Application mode with current brightness/rotation.
    pub fn apply_lcd_settings(&self) -> Result<()> {
        let mut payload = [0u8; 8];
        payload[0] = LcdControlMode::Application as u8;
        payload[1] = self.brightness;
        payload[2] = self.rotation as u8;
        payload[7] = 24; // fps

        self.send_b_command(CMD_LCD_CONTROL, &payload)?;
        debug!(
            "LCD settings applied: brightness={}, rotation={:?}",
            self.brightness, self.rotation
        );
        Ok(())
    }

    /// Send a JPEG frame to the LCD.
    pub fn send_jpeg(&self, jpeg_data: &[u8]) -> Result<()> {
        self.send_chunked(CMD_SEND_JPEG, jpeg_data)
    }

    pub fn variant(&self) -> AioLcdVariant {
        self.variant
    }

    /// Check if the LCD is ready to receive frames (B-command 0x17).
    /// Returns true if available, false if not, or error on timeout.
    pub fn is_lcd_available(&self) -> Result<bool> {
        let dev = self.device.lock();

        let mut pkt = vec![0u8; B_PACKET_SIZE];
        pkt[0] = REPORT_ID_B;
        pkt[1] = CMD_LCD_AVAILABLE;
        dev.write(&pkt)
            .context("AIO LCD: write LCD available check")?;

        let mut buf = vec![0u8; B_PACKET_SIZE];
        let n = dev
            .read_timeout(&mut buf, READ_TIMEOUT_MS)
            .context("AIO LCD: read LCD available response")?;

        if n == 0 || buf[1] != CMD_LCD_AVAILABLE {
            return Ok(false);
        }

        let data_len = (buf[9] as usize) << 8 | buf[10] as usize;
        Ok(data_len == 1 && buf[B_HEADER_LEN] == 0)
    }

    /// Reset device (A-command 0x8E). Returns true on success.
    pub fn reset_device(&self) -> bool {
        match self.send_a_command(CMD_RESET_DEVICE, &[], READ_TIMEOUT_MS) {
            Ok(resp) if resp.len() > A_HEADER_LEN && resp[1] == CMD_RESET_DEVICE => {
                let data_len = resp[5] as usize;
                data_len == 1 && resp[A_HEADER_LEN] == 1
            }
            _ => false,
        }
    }

    /// Check LCD availability and attempt recovery if unavailable.
    /// Only relevant when using C-packets; B-packet mode doesn't need this.
    pub fn check_and_recover_lcd(&mut self) -> Result<()> {
        if !self.use_c_command {
            return Ok(());
        }
        match self.is_lcd_available() {
            Ok(true) => Ok(()),
            Ok(false) => {
                warn!("LCD not available, attempting reset");
                if self.reset_device() {
                    info!("Device reset successful, reinitializing LCD");
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    self.apply_lcd_settings()
                } else {
                    warn!("Device reset failed");
                    Ok(())
                }
            }
            Err(e) => {
                debug!("LCD availability check failed: {e:#}");
                Ok(())
            }
        }
    }

    fn read_firmware_internal(&self, timeout_ms: i32) -> Result<String> {
        let dev = self.device.lock();

        let mut pkt = [0u8; A_PACKET_SIZE];
        pkt[0] = REPORT_ID_A;
        pkt[1] = CMD_GET_FIRMWARE;
        let written = dev.write(&pkt).context("AIO LCD: write firmware request")?;
        debug!("firmware request: wrote {written} bytes");

        // Loop reading until we get a firmware response, discarding stale responses
        // from a previous session (e.g. 0x81 handshake, 0x8E reset) as they arrive.
        let mut buf = [0u8; A_PACKET_SIZE];
        let version_str = loop {
            let n = dev
                .read_timeout(&mut buf, timeout_ms)
                .context("AIO LCD: read firmware")?;

            if n == 0 {
                bail!("AIO LCD: no firmware response (timeout after {timeout_ms}ms)");
            }

            debug!("firmware read: {n} bytes, cmd={:#04x}", buf[1]);

            if buf[1] == CMD_GET_FIRMWARE {
                let data_len = buf[5] as usize;
                let data = &buf[A_HEADER_LEN..A_HEADER_LEN + data_len.min(58)];
                break String::from_utf8_lossy(data)
                    .trim_end_matches('\0')
                    .to_string();
            }

            debug!("firmware read: skipping stale response {:#04x}", buf[1]);
        };

        // Response 2: date/time string (must be consumed to keep buffer in sync)
        let n2 = dev.read_timeout(&mut buf, timeout_ms).unwrap_or(0);
        if n2 > 0 {
            let len2 = buf[5] as usize;
            let data2 = &buf[A_HEADER_LEN..A_HEADER_LEN + len2.min(58)];
            debug!(
                "firmware date: {}",
                String::from_utf8_lossy(data2).trim_end_matches('\0')
            );
        }

        Ok(version_str)
    }

    fn send_a_command(&self, cmd: u8, data: &[u8], timeout_ms: i32) -> Result<Vec<u8>> {
        let mut pkt = [0u8; A_PACKET_SIZE];
        pkt[0] = REPORT_ID_A;
        pkt[1] = cmd;
        pkt[5] = data.len() as u8;
        let copy_len = data.len().min(58);
        pkt[A_HEADER_LEN..A_HEADER_LEN + copy_len].copy_from_slice(&data[..copy_len]);

        let dev = self.device.lock();
        let written = dev.write(&pkt).context("AIO LCD: write A-command")?;
        debug!(
            "A-cmd {cmd:#04x}: wrote {written} bytes, payload={:02x?}",
            &data[..copy_len]
        );

        let mut buf = [0u8; A_PACKET_SIZE];
        let n = dev
            .read_timeout(&mut buf, timeout_ms)
            .context("AIO LCD: read A-response")?;

        debug!(
            "A-cmd {cmd:#04x}: response {n} bytes, raw={:02x?}",
            &buf[..n.min(20)]
        );

        if n == 0 {
            bail!("AIO LCD: no response to A-command {cmd:#04x} (timeout after {timeout_ms}ms)");
        }

        Ok(buf[..n].to_vec())
    }

    fn send_b_command(&self, cmd: u8, data: &[u8]) -> Result<()> {
        self.send_chunked(cmd, data)
    }

    fn send_chunked(&self, cmd: u8, data: &[u8]) -> Result<()> {
        let (report_id, pkt_size, max_payload) = if self.use_c_command {
            (REPORT_ID_C, C_PACKET_SIZE, C_MAX_PAYLOAD)
        } else {
            (REPORT_ID_B, B_PACKET_SIZE, B_MAX_PAYLOAD)
        };

        let total_size = data.len();
        let mut offset = 0;
        let mut packet_num: u32 = 0;
        let dev = self.device.lock();

        loop {
            let remaining = total_size.saturating_sub(offset);
            let chunk_len = remaining.min(max_payload);

            let pkt = build_lcd_packet(
                report_id,
                pkt_size,
                cmd,
                total_size as u32,
                packet_num,
                if chunk_len > 0 {
                    &data[offset..offset + chunk_len]
                } else {
                    &[]
                },
            );

            dev.write(&pkt).context("AIO LCD: write LCD command")?;

            offset += chunk_len;
            packet_num += 1;

            if offset >= total_size {
                break;
            }
        }

        let mut buf = [0u8; 512];
        let _ = dev.read_timeout(&mut buf, 20);

        Ok(())
    }
}

impl FanDevice for HydroShiftLcdController {
    fn set_fan_speed(&self, _slot: u8, duty: u8) -> Result<()> {
        let pwm = duty.min(100);
        self.send_a_command(CMD_SET_FAN_PWM, &[0x00, pwm], READ_TIMEOUT_MS)?;
        debug!("Set fan PWM to {pwm}%");
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

    fn has_pump_control(&self) -> bool {
        true
    }

    fn set_pump_speed(&self, duty: u8) -> Result<()> {
        let pwm = duty.min(100);
        self.send_a_command(CMD_SET_PUMP_PWM, &[0x00, pwm], READ_TIMEOUT_MS)?;
        debug!("Set pump PWM to {pwm}%");
        Ok(())
    }
}

impl FanDevice for Arc<HydroShiftLcdController> {
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
    fn has_pump_control(&self) -> bool {
        (**self).has_pump_control()
    }
    fn set_pump_speed(&self, duty: u8) -> Result<()> {
        (**self).set_pump_speed(duty)
    }
}

impl AioDevice for HydroShiftLcdController {
    fn read_pump_rpm(&self) -> Result<u16> {
        Ok(self
            .last_handshake
            .as_ref()
            .map(|hs| hs.pump_rpm)
            .unwrap_or(0))
    }

    fn read_coolant_temp(&self) -> Result<f32> {
        match &self.last_handshake {
            Some(hs) if hs.temp_valid => Ok(hs.coolant_temp),
            Some(_) => bail!("Coolant temperature sensor reports invalid"),
            None => bail!("No handshake data available"),
        }
    }
}

impl LcdDevice for HydroShiftLcdController {
    fn screen_info(&self) -> &ScreenInfo {
        &ScreenInfo::AIO_LCD_480
    }

    fn send_jpeg_frame(&mut self, jpeg_data: &[u8]) -> Result<()> {
        self.send_jpeg(jpeg_data)
    }

    fn set_brightness(&self, brightness: u8) -> Result<()> {
        let mut payload = [0u8; 8];
        payload[0] = LcdControlMode::LcdSetting as u8;
        payload[1] = brightness.min(100);
        payload[2] = self.rotation as u8;
        payload[7] = 24;
        self.send_b_command(CMD_LCD_CONTROL, &payload)?;
        Ok(())
    }

    fn set_rotation(&self, degrees: u16) -> Result<()> {
        let rotation = ScreenRotation::from_degrees(degrees);
        let mut payload = [0u8; 8];
        payload[0] = LcdControlMode::LcdSetting as u8;
        payload[1] = self.brightness;
        payload[2] = rotation as u8;
        payload[7] = 24;
        self.send_b_command(CMD_LCD_CONTROL, &payload)?;
        Ok(())
    }

    fn initialize(&mut self) -> Result<()> {
        if self.initialized {
            return Ok(());
        }
        self.init()?;
        Ok(())
    }

    fn check_and_recover_lcd(&mut self) -> Result<()> {
        HydroShiftLcdController::check_and_recover_lcd(self)
    }
}

/// Build a B-command (1024B) or C-command (512B) LCD packet.
/// Header layout is identical; only report ID, packet size, and max payload differ.
fn build_lcd_packet(
    report_id: u8,
    pkt_size: usize,
    cmd: u8,
    total_data_size: u32,
    packet_num: u32,
    payload: &[u8],
) -> Vec<u8> {
    let header_len = 11;
    let max_payload = pkt_size - header_len;
    let mut pkt = vec![0u8; pkt_size];

    pkt[0] = report_id;
    pkt[1] = cmd;

    // Total data size (4 bytes BE)
    pkt[2] = (total_data_size >> 24) as u8;
    pkt[3] = (total_data_size >> 16) as u8;
    pkt[4] = (total_data_size >> 8) as u8;
    pkt[5] = total_data_size as u8;

    // Packet number (3 bytes BE)
    pkt[6] = (packet_num >> 16) as u8;
    pkt[7] = (packet_num >> 8) as u8;
    pkt[8] = packet_num as u8;

    // Payload length (2 bytes BE)
    let len = payload.len().min(max_payload);
    pkt[9] = (len >> 8) as u8;
    pkt[10] = len as u8;

    // Payload
    if len > 0 {
        pkt[header_len..header_len + len].copy_from_slice(&payload[..len]);
    }

    pkt
}

/// Parse firmware version from strings like "1.2", "V1.3", or "N9,01,HS,SQ,HydroShift,V3.0C.013,1.3".
/// Scans for the last "major.minor" pattern where both are numeric.
fn parse_firmware_version(fw: &str) -> Option<(u32, u32)> {
    // Split by comma and check segments from the end for "X.Y" patterns
    for segment in fw.rsplit(',') {
        let s = segment
            .trim()
            .trim_start_matches(|c: char| !c.is_ascii_digit());
        let mut parts = s.split('.');
        if let (Some(maj_s), Some(min_s)) = (parts.next(), parts.next()) {
            if let (Ok(major), Ok(minor)) = (maj_s.parse::<u32>(), min_s.parse::<u32>()) {
                return Some((major, minor));
            }
        }
    }
    None
}

const CMD_SET_PUMP_LIGHT: u8 = 0x83;
const CMD_SET_FAN_LIGHT: u8 = 0x85;
const FAN_LED_COUNT: u16 = 24;

pub struct AioLcdRgbController {
    device: Arc<Mutex<HidBackend>>,
    variant: AioLcdVariant,
}

impl AioLcdRgbController {
    pub fn new(device: Arc<Mutex<HidBackend>>, pid: u16) -> Result<Self> {
        let variant = AioLcdVariant::from_pid(pid)
            .ok_or_else(|| anyhow::anyhow!("Unknown AIO LCD PID: {pid:#06x}"))?;
        info!("Opened {} RGB controller", variant.name());
        Ok(Self { device, variant })
    }

    fn set_pump_light(&self, effect: &RgbEffect, source_mcu: bool) -> Result<()> {
        let scope = match effect.scope {
            RgbScope::Inner => 0u8,
            RgbScope::Outer => 1,
            _ => 2,
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
        self.send_rgb_command(CMD_SET_PUMP_LIGHT, &payload)?;
        debug!("Set pump light: mode={mode_byte} scope={scope}");
        Ok(())
    }

    fn set_fan_light(
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
        self.send_rgb_command(CMD_SET_FAN_LIGHT, &payload)?;
        debug!("Set fan light: mode={mode_byte} sync_to_pump={sync_to_pump}");
        Ok(())
    }

    fn send_rgb_command(&self, cmd: u8, data: &[u8]) -> Result<Vec<u8>> {
        let mut pkt = [0u8; A_PACKET_SIZE];
        pkt[0] = REPORT_ID_A;
        pkt[1] = cmd;
        pkt[5] = data.len() as u8;
        let copy_len = data.len().min(58);
        pkt[A_HEADER_LEN..A_HEADER_LEN + copy_len].copy_from_slice(&data[..copy_len]);

        let dev = self.device.lock();
        dev.write(&pkt).context("AIO LCD RGB: write")?;

        let mut buf = [0u8; A_PACKET_SIZE];
        let n = dev
            .read_timeout(&mut buf, READ_TIMEOUT_MS)
            .context("AIO LCD RGB: read")?;
        if n == 0 {
            bail!("AIO LCD RGB: no response to {cmd:#04x}");
        }
        Ok(buf[..n].to_vec())
    }
}

impl RgbDevice for AioLcdRgbController {
    fn device_name(&self) -> String {
        format!("{} AIO", self.variant.name())
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
        if self.variant.has_pump_rgb() {
            vec![
                RgbZoneInfo {
                    name: "Pump Head".to_string(),
                    led_count: 24,
                },
                RgbZoneInfo {
                    name: "Fans".to_string(),
                    led_count: FAN_LED_COUNT,
                },
            ]
        } else {
            vec![RgbZoneInfo {
                name: "Fans".to_string(),
                led_count: FAN_LED_COUNT,
            }]
        }
    }

    fn set_zone_effect(&self, zone: u8, effect: &RgbEffect) -> Result<()> {
        if self.variant.has_pump_rgb() {
            match zone {
                0 => self.set_pump_light(effect, true),
                1 => self.set_fan_light(effect, true, false),
                _ => bail!("{}: zone {zone} out of range (0-1)", self.variant.name()),
            }
        } else {
            match zone {
                0 => self.set_fan_light(effect, true, false),
                _ => bail!("{}: zone {zone} out of range (0)", self.variant.name()),
            }
        }
    }

    fn supported_scopes(&self) -> Vec<Vec<RgbScope>> {
        if self.variant.has_pump_rgb() {
            vec![
                vec![RgbScope::All, RgbScope::Inner, RgbScope::Outer], // Pump Head
                vec![],                                                // Fans
            ]
        } else {
            vec![]
        }
    }

    fn supports_mb_rgb_sync(&self) -> bool {
        true
    }

    fn set_mb_rgb_sync(&self, enabled: bool) -> Result<()> {
        let source_mcu = !enabled;
        let dummy = RgbEffect::default();
        if self.variant.has_pump_rgb() {
            self.set_pump_light(&dummy, source_mcu)?;
        }
        self.set_fan_light(&dummy, source_mcu, false)?;
        debug!("Set MB RGB sync: enabled={enabled}");
        Ok(())
    }
}
