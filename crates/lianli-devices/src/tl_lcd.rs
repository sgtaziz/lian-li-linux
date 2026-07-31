//! TLLCD fan LCD driver.
//!
//! VID=0x04FC, PID=0x7393
//!
//! Protocol uses HID Output Reports (Report ID 0x02, 512 bytes).
//! 11-byte header: [reportId, cmd, dataSize(4 BE), packetNum(3 BE), payloadLen(2 BE)]
//! JPEG frames are chunked into 501-byte payloads per packet.
//! Display is 400x400 pixels, max ~30fps.

use crate::traits::LcdDevice;
use anyhow::{bail, Context, Result};
use lianli_shared::screen::ScreenInfo;
use lianli_transport::RusbHid;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::sync::Arc;
use tracing::{debug, info, warn};

static CHAIN_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

const REPORT_ID: u8 = 0x02;
const PACKET_SIZE: usize = 512;
const HEADER_LEN: usize = 11;
const MAX_PAYLOAD_PER_PACKET: usize = PACKET_SIZE - HEADER_LEN; // 501
const READ_TIMEOUT_MS: i32 = 200;
const INIT_READ_TIMEOUT_MS: i32 = 3000;

// Commands
const CMD_GET_HANDSHAKE: u8 = 60;
const CMD_GET_PRODUCT_INFO: u8 = 61;
const CMD_READ_SERIAL: u8 = 62;
const CMD_WRITE_SERIAL: u8 = 63;
const CMD_LCD_CONTROL: u8 = 64;
const CMD_WRITE_JPG: u8 = 0x41;
#[allow(dead_code)]
const CMD_WRITE_AVI: u8 = 0x45;
#[allow(dead_code)]
const CMD_WRITE_BOOT_AVI: u8 = 0x47;
#[allow(dead_code)]
const CMD_WRITE_BOOT_JPG: u8 = 0x48;
const CMD_WRITE_SYNC_JPG: u8 = 0x46;

/// LCD control mode.
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum LcdControlMode {
    ShowJpg = 1,
    ShowAvi = 3,
    ShowAppSync = 4,
    LcdSetting = 5,
    LcdTest = 6,
}

/// Screen rotation.
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
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

/// Handshake info from the device.
#[derive(Debug, Clone)]
pub struct TlLcdHandshake {
    pub mode: u8,
    pub frame_index: u16,
}

/// Device identity (port, index, serial).
#[derive(Debug, Clone)]
pub struct TlLcdIdentity {
    pub serial: String,
    pub port: u8,
    pub index: u8,
}

/// TLLCD fan LCD controller.
///
/// Wraps an opened HID device for a TLLCD fan (0x04FC:0x7393).
/// Provides LCD streaming via 512-byte HID output reports.
pub struct TlLcdDevice {
    device: Arc<Mutex<RusbHid>>,
    identity: Option<TlLcdIdentity>,
    brightness: u8,
    rotation: ScreenRotation,
    initialized: bool,
}

impl TlLcdDevice {
    /// Create a new TLLCD device from an opened HID device handle.
    pub fn new(device: Arc<Mutex<RusbHid>>) -> Self {
        Self {
            device,
            identity: None,
            brightness: 50,
            rotation: ScreenRotation::Rotate0,
            initialized: false,
        }
    }

    /// Read the device serial number, port, and index. If the device returns a
    /// firmware string (e.g. `TL_LCDV0.1`) rather than a persistent unique ID,
    /// generate a UUID-like serial, write it via CMD 63, and re-read.
    pub fn read_identity(&mut self) -> Result<TlLcdIdentity> {
        let mut ident = self.read_identity_raw()?;
        if !looks_like_unique_serial(&ident.serial) {
            let new_serial = generate_unique_serial();
            if let Err(e) = self.write_serial(&new_serial) {
                warn!("TLLCD: failed to persist unique serial: {e}");
            } else {
                match self.read_identity_raw() {
                    Ok(refreshed) => ident = refreshed,
                    Err(e) => warn!("TLLCD: failed to re-read serial after write: {e}"),
                }
            }
        }
        self.identity = Some(ident.clone());
        Ok(ident)
    }

    pub fn read_identity_raw(&self) -> Result<TlLcdIdentity> {
        let resp =
            self.send_command_with_response_timeout(CMD_READ_SERIAL, &[], INIT_READ_TIMEOUT_MS)?;
        let data = &resp[HEADER_LEN..];
        let serial_bytes = &data[..32.min(data.len())];
        let serial = String::from_utf8_lossy(serial_bytes)
            .trim_end_matches('\0')
            .to_string();
        let port = if data.len() > 32 { data[32] } else { 0 };
        let index = if data.len() > 33 { data[33] } else { 0 };
        Ok(TlLcdIdentity {
            serial,
            port,
            index,
        })
    }

    fn write_serial(&self, serial: &str) -> Result<()> {
        let mut payload = [0u8; 32];
        let bytes = serial.as_bytes();
        let n = bytes.len().min(32);
        payload[..n].copy_from_slice(&bytes[..n]);
        self.send_command_no_response(CMD_WRITE_SERIAL, &payload)?;
        Ok(())
    }

    /// Read handshake info (current mode and frame index).
    pub fn read_handshake(&self) -> Result<TlLcdHandshake> {
        let resp =
            self.send_command_with_response_timeout(CMD_GET_HANDSHAKE, &[], INIT_READ_TIMEOUT_MS)?;
        let data = &resp[HEADER_LEN..];

        Ok(TlLcdHandshake {
            mode: data.first().copied().unwrap_or(0),
            frame_index: if data.len() >= 3 {
                u16::from_be_bytes([data[1], data[2]])
            } else {
                0
            },
        })
    }

    /// Read firmware version string.
    pub fn read_firmware(&self) -> Result<String> {
        let _chain = CHAIN_LOCK.lock();
        let mut dev = self.device.lock();
        dev.read_flush();

        let pkt = build_packet(CMD_GET_PRODUCT_INFO, 0, 0, &[]);
        dev.write(&pkt).context("TLLCD: write firmware request")?;

        // Response 1: version string
        let mut buf = [0u8; 64];
        let n = dev
            .read_timeout(&mut buf, INIT_READ_TIMEOUT_MS)
            .context("TLLCD: read firmware")?;
        if n == 0 {
            bail!("TLLCD: no firmware response");
        }
        let data_len = payload_length(&buf);
        let data = &buf[HEADER_LEN..HEADER_LEN + data_len.min(MAX_PAYLOAD_PER_PACKET)];
        let version_str = String::from_utf8_lossy(data)
            .trim_end_matches('\0')
            .to_string();

        // Response 2: date/time string (must be consumed to keep buffer in sync)
        let n2 = dev
            .read_timeout(&mut buf, INIT_READ_TIMEOUT_MS)
            .unwrap_or(0);
        if n2 > 0 {
            let len2 = payload_length(&buf);
            let data2 = &buf[HEADER_LEN..HEADER_LEN + len2.min(MAX_PAYLOAD_PER_PACKET)];
            let date_str = String::from_utf8_lossy(data2)
                .trim_end_matches('\0')
                .to_string();
            debug!("Firmware date: {date_str}");
        }

        Ok(version_str)
    }

    /// Set LCD brightness and rotation via LCD Control command.
    pub fn apply_lcd_settings(&self) -> Result<()> {
        let mut payload = [0u8; 11];
        payload[0] = LcdControlMode::LcdSetting as u8;
        payload[4] = self.brightness;
        payload[5] = 30; // fps
        payload[6] = self.rotation as u8;

        self.send_command_with_response(CMD_LCD_CONTROL, &payload)?;
        debug!(
            "LCD settings applied: brightness={}, rotation={:?}",
            self.brightness, self.rotation
        );
        Ok(())
    }

    /// Send a JPEG frame for immediate display (with response, for single images).
    pub fn send_jpeg(&self, jpeg_data: &[u8]) -> Result<()> {
        self.send_chunked(CMD_WRITE_JPG, jpeg_data, true)
    }

    /// Send a JPEG frame for streaming (no response wait, for video/sensor).
    pub fn send_sync_jpeg(&self, jpeg_data: &[u8]) -> Result<()> {
        self.send_chunked(CMD_WRITE_SYNC_JPG, jpeg_data, false)
    }

    pub fn switch_to_show_jpg(&self) -> Result<()> {
        let mut payload = [0u8; 11];
        payload[0] = LcdControlMode::ShowJpg as u8;
        payload[4] = self.brightness;
        payload[5] = 30;
        payload[6] = self.rotation as u8;

        let _chain = CHAIN_LOCK.lock();
        let mut dev = self.device.lock();
        let pkt = build_packet(CMD_LCD_CONTROL, payload.len() as u32, 0, &payload);
        dev.write(&pkt).context("TLLCD: write LCDControl ShowJpg")?;
        let mut buf = [0u8; 64];
        let _ = dev.read_timeout(&mut buf, READ_TIMEOUT_MS);
        Ok(())
    }

    /// Identity (serial, port, index) if read.
    pub fn identity(&self) -> Option<&TlLcdIdentity> {
        self.identity.as_ref()
    }

    pub fn serial(&self) -> Option<&str> {
        self.identity.as_ref().map(|i| i.serial.as_str())
    }

    /// Send data in 501-byte chunks as multiple 512-byte HID packets.
    fn send_chunked(&self, cmd: u8, data: &[u8], read_response: bool) -> Result<()> {
        let total_size = data.len();
        let mut offset = 0;
        let mut packet_num: u32 = 0;
        let _chain = CHAIN_LOCK.lock();
        let mut dev = self.device.lock();

        let mut ack_buf = [0u8; 64];
        while offset < total_size {
            let remaining = total_size - offset;
            let chunk_len = remaining.min(MAX_PAYLOAD_PER_PACKET);

            let pkt = build_packet(
                cmd,
                total_size as u32,
                packet_num,
                &data[offset..offset + chunk_len],
            );
            dev.write(&pkt).context("TLLCD: write packet")?;
            if read_response {
                dev.read_timeout(&mut ack_buf, READ_TIMEOUT_MS)
                    .context("TLLCD: read packet ack")?;
                if ack_buf.len() > 1 && ack_buf[1] != cmd {
                    anyhow::bail!(
                        "TLLCD: ack mismatch (expected 0x{cmd:02x}, got 0x{:02x})",
                        ack_buf[1]
                    );
                }
            }

            offset += chunk_len;
            packet_num += 1;
        }

        if total_size == 0 {
            let pkt = build_packet(cmd, 0, 0, &[]);
            dev.write(&pkt).context("TLLCD: write empty packet")?;
            if read_response {
                dev.read_timeout(&mut ack_buf, READ_TIMEOUT_MS)
                    .context("TLLCD: read empty packet ack")?;
            }
        }

        Ok(())
    }

    /// Send a command with payload and don't wait for any response.
    fn send_command_no_response(&self, cmd: u8, payload: &[u8]) -> Result<()> {
        let _chain = CHAIN_LOCK.lock();
        let mut dev = self.device.lock();
        let pkt = build_packet(cmd, payload.len() as u32, 0, payload);
        dev.write(&pkt).context("TLLCD: write command (no resp)")?;
        Ok(())
    }

    /// Send a command with payload and read response.
    fn send_command_with_response(&self, cmd: u8, payload: &[u8]) -> Result<Vec<u8>> {
        self.send_command_with_response_timeout(cmd, payload, READ_TIMEOUT_MS)
    }

    fn send_command_with_response_timeout(
        &self,
        cmd: u8,
        payload: &[u8],
        timeout_ms: i32,
    ) -> Result<Vec<u8>> {
        let _chain = CHAIN_LOCK.lock();
        let mut dev = self.device.lock();
        let pkt = build_packet(cmd, payload.len() as u32, 0, payload);

        dev.write(&pkt).context("TLLCD: write command")?;

        let mut buf = [0u8; 64];
        let n = dev
            .read_timeout(&mut buf, timeout_ms)
            .context("TLLCD: read response")?;

        if n == 0 {
            bail!("TLLCD: no response to command {cmd}");
        }

        Ok(buf[..n].to_vec())
    }
}

impl LcdDevice for TlLcdDevice {
    fn screen_info(&self) -> &ScreenInfo {
        &ScreenInfo::TLLCD
    }

    fn send_jpeg_frame(&mut self, jpeg_data: &[u8]) -> Result<()> {
        self.send_sync_jpeg(jpeg_data)
    }

    fn send_static_frame(&mut self, jpeg_data: &[u8]) -> Result<()> {
        self.send_jpeg(jpeg_data)?;
        self.switch_to_show_jpg()?;
        Ok(())
    }

    fn set_brightness(&self, brightness: u8) -> Result<()> {
        let mut payload = [0u8; 11];
        payload[0] = LcdControlMode::LcdSetting as u8;
        payload[4] = brightness.min(100);
        payload[5] = 30;
        payload[6] = self.rotation as u8;
        self.send_command_with_response(CMD_LCD_CONTROL, &payload)?;
        Ok(())
    }

    fn set_rotation(&self, degrees: u16) -> Result<()> {
        let rotation = ScreenRotation::from_degrees(degrees);
        let mut payload = [0u8; 11];
        payload[0] = LcdControlMode::LcdSetting as u8;
        payload[4] = self.brightness;
        payload[5] = 30;
        payload[6] = rotation as u8;
        self.send_command_with_response(CMD_LCD_CONTROL, &payload)?;
        Ok(())
    }

    fn initialize(&mut self) -> Result<()> {
        if self.initialized {
            return Ok(());
        }

        info!("Initializing TLLCD (0x04FC:0x7393)");

        match self.read_identity() {
            Ok(ident) => {
                info!(
                    "  Serial: {}, Port: {}, Index: {}",
                    ident.serial, ident.port, ident.index
                );
            }
            Err(e) => warn!("  Failed to read identity: {e}"),
        }

        match self.read_handshake() {
            Ok(hs) => {
                debug!("  Mode: {}, Frame: {}", hs.mode, hs.frame_index);
            }
            Err(e) => warn!("  Failed to read handshake: {e}"),
        }

        match self.read_firmware() {
            Ok(fw) => info!("  Firmware: {fw}"),
            Err(e) => warn!("  Failed to read firmware: {e}"),
        }

        self.apply_lcd_settings()?;
        self.initialized = true;

        Ok(())
    }
}

/// Build a 512-byte TLLCD HID packet.
fn build_packet(
    cmd: u8,
    total_data_size: u32,
    packet_num: u32,
    payload: &[u8],
) -> [u8; PACKET_SIZE] {
    let mut pkt = [0u8; PACKET_SIZE];

    pkt[0] = REPORT_ID;
    pkt[1] = cmd;

    // Data size (4 bytes, big-endian)
    pkt[2] = (total_data_size >> 24) as u8;
    pkt[3] = (total_data_size >> 16) as u8;
    pkt[4] = (total_data_size >> 8) as u8;
    pkt[5] = total_data_size as u8;

    // Packet number (3 bytes, big-endian)
    pkt[6] = (packet_num >> 16) as u8;
    pkt[7] = (packet_num >> 8) as u8;
    pkt[8] = packet_num as u8;

    // Payload length (2 bytes, big-endian)
    let len = payload.len().min(MAX_PAYLOAD_PER_PACKET);
    pkt[9] = (len >> 8) as u8;
    pkt[10] = len as u8;

    // Payload
    if len > 0 {
        pkt[HEADER_LEN..HEADER_LEN + len].copy_from_slice(&payload[..len]);
    }

    pkt
}

/// Extract payload length from a response packet.
fn payload_length(pkt: &[u8]) -> usize {
    if pkt.len() >= HEADER_LEN {
        ((pkt[9] as usize) << 8) | (pkt[10] as usize)
    } else {
        0
    }
}

/// A serial we wrote ourselves looks like `lcd-<32 hex chars>`. Anything else
/// (firmware version strings like `TL_LCDV0.1`, empty, etc.) means the device
/// hasn't been claimed by us yet.
fn looks_like_unique_serial(s: &str) -> bool {
    let s = s.trim();
    s.len() == 36
        && s.as_bytes()[8] == b'-'
        && s.as_bytes()[13] == b'-'
        && s.as_bytes()[18] == b'-'
        && s.as_bytes()[23] == b'-'
}

fn generate_unique_serial() -> String {
    if let Ok(bytes) = std::fs::read("/proc/sys/kernel/random/uuid") {
        let s = String::from_utf8_lossy(&bytes).trim().to_string();
        if looks_like_unique_serial(&s) {
            return s;
        }
    }
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        nanos as u32,
        (nanos >> 16) as u16,
        (nanos >> 32) as u16,
        (nanos >> 48) as u16,
        nanos as u64 & 0xFFFFFFFFFFFF
    )
}

/// Driver entry point for the TL LCD fan.
pub struct TlLcdDriver;

impl crate::registry::DeviceDriver for TlLcdDriver {
    fn family(&self) -> lianli_shared::device_id::DeviceFamily {
        lianli_shared::device_id::DeviceFamily::TlLcd
    }

    fn open(
        &self,
        ctx: &crate::registry::OpenContext,
    ) -> anyhow::Result<crate::registry::OpenedDevice> {
        let backend: crate::registry::SharedHid = crate::detect::open_hid_with_reopener(
            ctx.device.clone(),
            ctx.hid_usage_page,
            ctx.vid,
            ctx.pid,
            ctx.bus,
            ctx.device.port_numbers().unwrap_or_default(),
        )?;
        let mut lcd = TlLcdDevice::new(backend);
        crate::traits::LcdDevice::initialize(&mut lcd)?;
        Ok(crate::registry::OpenedDevice {
            id: ctx.device_id(),
            family: lianli_shared::device_id::DeviceFamily::TlLcd,
            capabilities: lianli_shared::device_id::DeviceFamily::TlLcd.capabilities(),
            transport_kind: lianli_shared::device_id::TransportKind::Hid,
            model_name: "UNI FAN TL LCD".to_string(),
            firmware: None,
            fan: None,
            lcd: Some(Box::new(lcd)),
            rgb: Vec::new(),
            aio: None,
            shared_hid: None,
        })
    }
}
