//! TL Fan controller driver.
//!
//! VID=0x0416, PID=0x7372
//!
//! Protocol uses HID Output Reports with Report ID 0x01.
//! 64-byte packets with a 6-byte header: [reportId, cmd, reserved, pktNumHi, pktNumLo, dataLen].
//! Each command expects a synchronous response (read after write).
//!
//! The controller supports 4 ports, each with multiple fans.
//! Fan speed is set per-fan via command 0xAA.
//! RPM values are only available from the handshake response (0xA1).

use crate::traits::{FanDevice, RgbDevice};
use anyhow::{bail, Context, Result};
use hidapi::HidDevice;
use lianli_shared::rgb::{RgbEffect, RgbMode, RgbZoneInfo};
use parking_lot::Mutex;
use tracing::{debug, info, warn};

const REPORT_ID: u8 = 0x01;
const PACKET_SIZE: usize = 64;
const HEADER_LEN: usize = 6;
const MAX_PAYLOAD: usize = PACKET_SIZE - HEADER_LEN;
const READ_TIMEOUT_MS: i32 = 100;

// Commands — Fan control
const CMD_HANDSHAKE: u8 = 0xA1;
const CMD_GET_PRODUCT_INFO: u8 = 0xA6;
const CMD_SET_FAN_SPEED: u8 = 0xAA;
const CMD_SET_MB_RPM_SYNC: u8 = 0xB1;

// Commands — LED control (from decompiled L-Connect 3 LEDCommands.cs)
const CMD_SET_FAN_LIGHT: u8 = 0xA3;
const CMD_SET_FAN_GROUP_LIGHT: u8 = 0xB0;
const CMD_SET_FAN_DIRECTION: u8 = 0xAE;
const CMD_SET_PORT_DIRECTION: u8 = 0xAF;

/// Number of LEDs per TL fan.
const LEDS_PER_FAN: u16 = 20;

/// Information about a single detected fan.
#[derive(Debug, Clone)]
pub struct TlFanInfo {
    pub port: u8,
    pub fan_index: u8,
    pub rpm: u16,
    pub is_detected: bool,
}

/// TL Fan handshake result containing discovered fans per port.
#[derive(Debug, Clone)]
pub struct TlFanHandshake {
    /// Fans detected on each port. Index = port number (0-3).
    pub port_fan_counts: [u8; 4],
    /// All detected fans with their RPM values.
    pub fans: Vec<TlFanInfo>,
}

/// TL Fan controller.
///
/// Wraps an opened HID device for a TL Fan controller (0x0416:0x7372).
/// Provides fan speed control, RPM reading, and RGB/LED effects.
pub struct TlFanController {
    device: Mutex<HidDevice>,
    /// Last handshake result (updated on each handshake call).
    /// Behind a Mutex for interior mutability — allows `read_fan_rpm(&self)`
    /// to refresh RPMs while the device is shared across threads.
    last_handshake: Mutex<Option<TlFanHandshake>>,
}

impl TlFanController {
    /// Open a TL Fan controller from an already-opened HID device.
    pub fn new(device: HidDevice) -> Result<Self> {
        let ctrl = Self {
            device: Mutex::new(device),
            last_handshake: Mutex::new(None),
        };

        ctrl.initialize()?;
        Ok(ctrl)
    }

    /// Initialize: perform handshake to discover fans.
    fn initialize(&self) -> Result<()> {
        info!("Initializing TL Fan Controller (0x0416:0x7372)");

        match self.read_product_info() {
            Ok(version) => info!("  Firmware: {version}"),
            Err(e) => warn!("  Failed to read firmware: {e}"),
        }

        match self.handshake() {
            Ok(hs) => {
                info!(
                    "  Detected fans: port0={}, port1={}, port2={}, port3={}",
                    hs.port_fan_counts[0],
                    hs.port_fan_counts[1],
                    hs.port_fan_counts[2],
                    hs.port_fan_counts[3]
                );
                for fan in &hs.fans {
                    debug!(
                        "    Port {} Fan {}: {} RPM",
                        fan.port, fan.fan_index, fan.rpm
                    );
                }
            }
            Err(e) => warn!("  Handshake failed: {e}"),
        }

        Ok(())
    }

    /// Perform a handshake to discover connected fans and read RPMs.
    pub fn handshake(&self) -> Result<TlFanHandshake> {
        let response = self.send_command(CMD_HANDSHAKE, &[])?;

        let mut port_fan_counts = [0u8; 4];
        let mut fans = Vec::new();

        // Response data: 3 bytes per fan entry
        // Byte 0: [7]IsDetected | [6]IsUpgrading | [5:4]Port | [3:0]FanIndex
        // Byte 1: RPM high byte
        // Byte 2: RPM low byte
        let data = &response[HEADER_LEN..];
        let data_len = response[5] as usize;
        let fan_count = data_len / 3;

        for i in 0..fan_count {
            let offset = i * 3;
            if offset + 2 >= data.len() {
                break;
            }

            let info_byte = data[offset];
            let is_detected = (info_byte & 0x80) != 0;
            let port = (info_byte >> 4) & 0x03;
            let fan_index = info_byte & 0x0F;
            let rpm = u16::from_be_bytes([data[offset + 1], data[offset + 2]]);

            if is_detected {
                port_fan_counts[port as usize] =
                    port_fan_counts[port as usize].max(fan_index + 1);
            }

            fans.push(TlFanInfo {
                port,
                fan_index,
                rpm,
                is_detected,
            });
        }

        let hs = TlFanHandshake {
            port_fan_counts,
            fans,
        };
        *self.last_handshake.lock() = Some(hs.clone());
        Ok(hs)
    }

    /// Read product/firmware info.
    fn read_product_info(&self) -> Result<String> {
        // Request controller firmware (not fan firmware)
        let response = self.send_command(CMD_GET_PRODUCT_INFO, &[0x00, 0x00])?;
        let data_len = response[5] as usize;
        let data = &response[HEADER_LEN..HEADER_LEN + data_len.min(MAX_PAYLOAD)];

        // Firmware version is ASCII text
        let version = String::from_utf8_lossy(data)
            .trim_end_matches('\0')
            .to_string();
        Ok(version)
    }

    /// Set fan speed (PWM duty) for a specific port and fan index.
    ///
    /// `port`: 0-3
    /// `fan_index`: 0-15 (typically 0-3)
    /// `duty`: 0-255 (0% to 100%)
    pub fn set_fan_speed_single(&self, port: u8, fan_index: u8, duty: u8) -> Result<()> {
        if port >= 4 {
            bail!("Port {port} out of range (0-3)");
        }

        let addr = (port << 4) | (fan_index & 0x0F);
        self.send_command_quiet(CMD_SET_FAN_SPEED, &[addr, duty])?;

        debug!(
            "Set port {} fan {} speed to duty={duty} ({:.0}%)",
            port,
            fan_index,
            duty as f32 / 2.55
        );
        Ok(())
    }

    /// Set the same speed for all fans on a port.
    pub fn set_port_speed(&self, port: u8, duty: u8) -> Result<()> {
        let fan_count = self
            .last_handshake
            .lock()
            .as_ref()
            .map(|hs| hs.port_fan_counts[port as usize])
            .unwrap_or(1);

        for idx in 0..fan_count {
            self.set_fan_speed_single(port, idx, duty)?;
        }
        Ok(())
    }

    /// Enable or disable motherboard RPM sync for a specific fan.
    ///
    /// When enabled, the TL Fan Controller firmware reads the motherboard's
    /// PWM signal directly from its 4-pin fan header, bypassing software control.
    ///
    /// Data byte: `(isSync << 7) | (port << 4) | fanIndex`
    pub fn set_mb_rpm_sync(&self, port: u8, fan_index: u8, sync: bool) -> Result<()> {
        if port >= 4 {
            bail!("Port {port} out of range (0-3)");
        }
        let data = ((sync as u8) << 7) | (port << 4) | (fan_index & 0x0F);
        self.send_command_quiet(CMD_SET_MB_RPM_SYNC, &[data])?;
        debug!(
            "Set MB RPM sync port={port} fan={fan_index} sync={sync}"
        );
        Ok(())
    }

    /// Enable or disable motherboard RPM sync for all fans on a port.
    pub fn set_port_mb_rpm_sync(&self, port: u8, sync: bool) -> Result<()> {
        let fan_count = self
            .last_handshake
            .lock()
            .as_ref()
            .map(|hs| hs.port_fan_counts[port as usize])
            .unwrap_or(1);

        for idx in 0..fan_count {
            self.set_mb_rpm_sync(port, idx, sync)?;
        }
        Ok(())
    }

    /// Total number of detected fans across all ports.
    pub fn total_fan_count(&self) -> u8 {
        self.last_handshake
            .lock()
            .as_ref()
            .map(|hs| hs.port_fan_counts.iter().sum())
            .unwrap_or(0)
    }

    // -- LED control methods --

    /// Set LED effect for a fan group on a port.
    ///
    /// Uses SetFanGroupLight (0xB0) command with 20-byte payload.
    /// From decompiled TLFanDevice.cs SetGroupLight().
    ///
    /// Payload layout:
    /// ```text
    /// [0]  = 0x00 (reserved)
    /// [1]  = group_num
    /// [2]  = mode % 1000 (effect mode byte)
    /// [3]  = brightness (0-4)
    /// [4]  = speed (0-4)
    /// [5-16] = R,G,B × 4 colors (12 bytes)
    /// [17] = direction (0-5)
    /// [18] = disable flag (0=enabled, 1=disabled)
    /// [19] = color count
    /// ```
    pub fn set_group_light(&self, group: u8, effect: &RgbEffect) -> Result<()> {
        let mode_byte = effect
            .mode
            .to_tl_mode_byte()
            .unwrap_or(3); // Default to Static(3)

        let mut payload = [0u8; 20];
        payload[0] = 0x00;
        payload[1] = group;
        payload[2] = mode_byte;
        payload[3] = effect.brightness.min(4);
        payload[4] = effect.speed.min(4);

        // Fill up to 4 RGB colors
        let color_count = effect.colors.len().min(4);
        for (i, color) in effect.colors.iter().take(4).enumerate() {
            let offset = 5 + i * 3;
            payload[offset] = color[0];     // R
            payload[offset + 1] = color[1]; // G
            payload[offset + 2] = color[2]; // B
        }

        payload[17] = effect.direction.to_tl_byte();
        payload[18] = if effect.mode == RgbMode::Off { 1 } else { 0 };
        payload[19] = color_count as u8;

        self.send_command_quiet(CMD_SET_FAN_GROUP_LIGHT, &payload)?;
        debug!(
            "Set group {group} light: mode={mode_byte} brightness={} speed={} colors={color_count}",
            effect.brightness, effect.speed
        );
        Ok(())
    }

    /// Set LED effect for a specific fan on a port.
    ///
    /// Uses SetFanLight (0xA3) command with 20-byte payload.
    /// From decompiled TLFanDevice.cs SetFanLight().
    ///
    /// Payload layout:
    /// ```text
    /// [0]  = (port << 4) | is_sync
    /// [1]  = (port << 4) | fan_index
    /// [2]  = mode % 1000
    /// [3]  = brightness (0-4)
    /// [4]  = speed (0-4)
    /// [5-16] = R,G,B × 4 colors
    /// [17] = direction (0-5)
    /// [18] = disable flag
    /// [19] = color count
    /// ```
    pub fn set_fan_light(&self, port: u8, fan_index: u8, effect: &RgbEffect, sync: bool) -> Result<()> {
        if port >= 4 {
            bail!("Port {port} out of range (0-3)");
        }

        let mode_byte = effect
            .mode
            .to_tl_mode_byte()
            .unwrap_or(3);

        let mut payload = [0u8; 20];
        payload[0] = (port << 4) | (sync as u8);
        payload[1] = (port << 4) | (fan_index & 0x0F);
        payload[2] = mode_byte;
        payload[3] = effect.brightness.min(4);
        payload[4] = effect.speed.min(4);

        let color_count = effect.colors.len().min(4);
        for (i, color) in effect.colors.iter().take(4).enumerate() {
            let offset = 5 + i * 3;
            payload[offset] = color[0];
            payload[offset + 1] = color[1];
            payload[offset + 2] = color[2];
        }

        payload[17] = effect.direction.to_tl_byte();
        payload[18] = if effect.mode == RgbMode::Off { 1 } else { 0 };
        payload[19] = color_count as u8;

        self.send_command_quiet(CMD_SET_FAN_LIGHT, &payload)?;
        debug!(
            "Set port {port} fan {fan_index} light: mode={mode_byte} sync={sync}"
        );
        Ok(())
    }

    /// Set fan direction flags for a specific fan.
    ///
    /// From decompiled TLFanDevice.cs SetFanDirection().
    /// Data: `[(port<<4)|fanIndex, (swapTopBot<<1)|swapLR]`
    pub fn set_fan_direction(&self, port: u8, fan_index: u8, swap_lr: bool, swap_tb: bool) -> Result<()> {
        let addr = (port << 4) | (fan_index & 0x0F);
        let flags = ((swap_tb as u8) << 1) | (swap_lr as u8);
        self.send_command_quiet(CMD_SET_FAN_DIRECTION, &[addr, flags])?;
        debug!("Set fan direction port={port} fan={fan_index} swap_lr={swap_lr} swap_tb={swap_tb}");
        Ok(())
    }

    /// Set port-level direction swap.
    ///
    /// From decompiled TLFanDevice.cs SetPortDirection().
    /// Data: `[(port<<4), isSwap]`
    pub fn set_port_direction(&self, port: u8, swap: bool) -> Result<()> {
        self.send_command_quiet(CMD_SET_PORT_DIRECTION, &[port << 4, swap as u8])?;
        debug!("Set port {port} direction swap={swap}");
        Ok(())
    }

    // -- Low-level packet helpers --

    /// Build a TL Fan packet.
    fn build_packet(cmd: u8, data: &[u8]) -> [u8; PACKET_SIZE] {
        let mut pkt = [0u8; PACKET_SIZE];
        pkt[0] = REPORT_ID;
        pkt[1] = cmd;
        pkt[2] = 0x00; // reserved
        pkt[3] = 0x00; // packet number high
        pkt[4] = 0x00; // packet number low
        pkt[5] = data.len() as u8;

        let copy_len = data.len().min(MAX_PAYLOAD);
        pkt[HEADER_LEN..HEADER_LEN + copy_len].copy_from_slice(&data[..copy_len]);
        pkt
    }

    /// Drain any stale data sitting in the read buffer.
    fn drain_read_buffer(dev: &HidDevice) {
        let mut buf = [0u8; PACKET_SIZE];
        while dev.read_timeout(&mut buf, 0).unwrap_or(0) > 0 {}
    }

    /// Send a command, try to read a response but ignore failure.
    /// Matches L-Connect 3 behavior where readPacket() result is discarded.
    fn send_command_quiet(&self, cmd: u8, data: &[u8]) -> Result<()> {
        let pkt = Self::build_packet(cmd, data);
        let dev = self.device.lock();
        Self::drain_read_buffer(&dev);
        dev.write(&pkt).context("TL Fan: write command")?;
        let mut buf = [0u8; PACKET_SIZE];
        let _ = dev.read_timeout(&mut buf, READ_TIMEOUT_MS);
        Ok(())
    }

    /// Send a command and read the synchronous response.
    fn send_command(&self, cmd: u8, data: &[u8]) -> Result<Vec<u8>> {
        let pkt = Self::build_packet(cmd, data);
        let dev = self.device.lock();

        Self::drain_read_buffer(&dev);

        dev.write(&pkt)
            .context("TL Fan: write command")?;

        let mut buf = [0u8; PACKET_SIZE];
        // Read up to a few times to skip stale responses
        for _ in 0..5 {
            let n = dev
                .read_timeout(&mut buf, READ_TIMEOUT_MS)
                .context("TL Fan: read response")?;

            if n == 0 {
                bail!("TL Fan: no response to command {cmd:#04x}");
            }

            if buf[1] == cmd {
                return Ok(buf[..n].to_vec());
            }

            debug!(
                "TL Fan: skipping stale response {:#04x} (waiting for {cmd:#04x})",
                buf[1]
            );
        }

        bail!("TL Fan: never received response for command {cmd:#04x}");
    }
}

impl FanDevice for TlFanController {
    fn set_fan_speed(&self, slot: u8, duty: u8) -> Result<()> {
        // Treat slot as port index, set all fans on that port
        self.set_port_speed(slot, duty)
    }

    fn set_fan_speeds(&self, duties: &[u8]) -> Result<()> {
        // Set speed per port
        for (port, &duty) in duties.iter().take(4).enumerate() {
            self.set_port_speed(port as u8, duty)?;
        }
        Ok(())
    }

    fn read_fan_rpm(&self) -> Result<Vec<u16>> {
        // Refresh handshake for live RPM data
        let _ = self.handshake();

        let guard = self.last_handshake.lock();
        match guard.as_ref() {
            Some(hs) => {
                // Return per-fan RPMs ordered by port then fan_index
                let mut rpms = Vec::new();
                for port in 0..4u8 {
                    for fan_idx in 0..hs.port_fan_counts[port as usize] {
                        let rpm = hs
                            .fans
                            .iter()
                            .find(|f| f.port == port && f.fan_index == fan_idx && f.is_detected)
                            .map(|f| f.rpm)
                            .unwrap_or(0);
                        rpms.push(rpm);
                    }
                }
                Ok(rpms)
            }
            None => Ok(vec![]),
        }
    }

    fn fan_slot_count(&self) -> u8 {
        4 // 4 ports
    }

    fn fan_port_info(&self) -> Vec<(u8, u8)> {
        let guard = self.last_handshake.lock();
        match guard.as_ref() {
            Some(hs) => hs
                .port_fan_counts
                .iter()
                .enumerate()
                .filter(|(_, &count)| count > 0)
                .map(|(port, &count)| (port as u8, count))
                .collect(),
            None => vec![(0, 4)],
        }
    }

    fn per_fan_control(&self) -> bool {
        false // all fans on a port share the same speed
    }

    fn supports_mb_sync(&self) -> bool {
        true
    }

    fn set_mb_rpm_sync(&self, port: u8, sync: bool) -> Result<()> {
        self.set_port_mb_rpm_sync(port, sync)
    }
}

/// TL Fan LED zones: one zone per active port (port with detected fans).
/// Each zone covers all fans on that port via SetFanGroupLight.
impl RgbDevice for TlFanController {
    fn device_name(&self) -> String {
        "UNI FAN TL Controller".to_string()
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
        let guard = self.last_handshake.lock();
        match guard.as_ref() {
            Some(hs) => hs
                .port_fan_counts
                .iter()
                .enumerate()
                .filter(|(_, &count)| count > 0)
                .map(|(port, &count)| RgbZoneInfo {
                    name: format!("Port {port}"),
                    led_count: count as u16 * LEDS_PER_FAN,
                })
                .collect(),
            None => vec![],
        }
    }

    fn set_zone_effect(&self, zone: u8, effect: &RgbEffect) -> Result<()> {
        // Zone index maps to port index (only counting active ports)
        let guard = self.last_handshake.lock();
        let port = match guard.as_ref() {
            Some(hs) => {
                let active_ports: Vec<u8> = hs
                    .port_fan_counts
                    .iter()
                    .enumerate()
                    .filter(|(_, &count)| count > 0)
                    .map(|(port, _)| port as u8)
                    .collect();
                active_ports
                    .get(zone as usize)
                    .copied()
                    .ok_or_else(|| anyhow::anyhow!("Zone {zone} out of range"))?
            }
            None => bail!("No handshake data available"),
        };
        drop(guard);

        // Use group 0 for the port (SetFanGroupLight targets all fans in the group)
        self.set_group_light(port, effect)
    }

    fn supports_mb_rgb_sync(&self) -> bool {
        true
    }

    fn set_mb_rgb_sync(&self, enabled: bool) -> Result<()> {
        // From decompiled TLFanDevice.cs: SetFanLight(port, fanIndex, config, isSync, disable)
        // is called with isSync=true for each fan.
        // The isSync flag is in data byte 0, bit 0: (port << 4) | isSync
        // Command: 0xA3 (SetFanLight)
        let port_fan_counts = {
            let guard = self.last_handshake.lock();
            guard
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("No handshake data"))?
                .port_fan_counts
        };

        let dummy_effect = RgbEffect::default();
        for (port, &fan_count) in port_fan_counts.iter().enumerate() {
            for fan in 0..fan_count {
                self.set_fan_light(port as u8, fan, &dummy_effect, enabled)?;
            }
        }

        debug!("Set MB RGB sync: enabled={enabled}");
        Ok(())
    }
}
