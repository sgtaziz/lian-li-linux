use super::{TlFanHandshake, TlFanInfo};
use crate::traits::FanDevice;
use anyhow::{bail, Context, Result};
use lianli_shared::rgb::{RgbEffect, RgbMode};
use lianli_transport::RusbHid;
use parking_lot::Mutex;
use std::sync::Arc;
use tracing::{debug, info, warn};

pub(super) const REPORT_ID: u8 = 0x01;
pub(super) const PACKET_SIZE: usize = 64;
pub(super) const HEADER_LEN: usize = 6;
pub(super) const MAX_PAYLOAD: usize = PACKET_SIZE - HEADER_LEN;
pub(super) const READ_TIMEOUT_MS: i32 = 100;
pub(super) const INIT_READ_TIMEOUT_MS: i32 = 3000;

// Commands — Fan control
pub(super) const CMD_HANDSHAKE: u8 = 0xA1;
pub(super) const CMD_GET_PRODUCT_INFO: u8 = 0xA6;
pub(super) const CMD_SET_FAN_SPEED: u8 = 0xAA;
pub(super) const CMD_SET_MB_RPM_SYNC: u8 = 0xB1;

// Commands — LED control
pub(super) const CMD_SET_FAN_LIGHT: u8 = 0xA3;
pub(super) const CMD_SET_FAN_GROUP: u8 = 0xAD;
pub(super) const CMD_SET_FAN_GROUP_LIGHT: u8 = 0xB0;
pub(super) const CMD_SET_FAN_DIRECTION: u8 = 0xAE;
pub(super) const CMD_SET_PORT_DIRECTION: u8 = 0xAF;
pub(super) const CMD_TEST_LIGHT: u8 = 0xB3;
pub(super) const CMD_BLINK_PORT: u8 = 0xB4;

/// TL Fan controller.
///
/// Wraps an opened HID device for a TL Fan controller (0x0416:0x7372).
/// Provides fan speed control, RPM reading, and RGB/LED effects.
pub struct TlFanController {
    pub(super) device: Arc<Mutex<RusbHid>>,
    /// Last handshake result. Behind a Mutex for interior mutability — allows
    /// `read_fan_rpm(&self)` to refresh RPMs while the device is shared across threads.
    pub(super) last_handshake: Mutex<Option<TlFanHandshake>>,
}

impl TlFanController {
    /// Open a TL Fan controller from an already-opened HID device.
    pub fn new(device: Arc<Mutex<RusbHid>>) -> Result<Self> {
        let ctrl = Self {
            device,
            last_handshake: Mutex::new(None),
        };

        ctrl.initialize()?;
        Ok(ctrl)
    }

    fn initialize(&self) -> Result<()> {
        info!("Initializing TL Fan Controller (0x0416:0x7372)");

        match self.read_product_info() {
            Ok(version) => info!("  Firmware: {version}"),
            Err(e) => warn!("  Failed to read firmware: {e}"),
        }

        match self.handshake_with_timeout(INIT_READ_TIMEOUT_MS) {
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

                if let Err(e) = self.setup_fan_groups(&hs.port_fan_counts) {
                    warn!("  Failed to set up fan groups: {e}");
                }
            }
            Err(e) => warn!("  Handshake failed: {e}"),
        }

        Ok(())
    }

    pub fn handshake(&self) -> Result<TlFanHandshake> {
        self.handshake_with_timeout(READ_TIMEOUT_MS)
    }

    fn handshake_with_timeout(&self, timeout_ms: i32) -> Result<TlFanHandshake> {
        let response = self.send_command_timeout(CMD_HANDSHAKE, &[], timeout_ms)?;

        let mut port_fan_counts = [0u8; 4];
        let mut fans = Vec::new();

        // Response data: 3 bytes per fan entry
        // Byte 0: [7]IsDetected | [6]IsUpgrading | [5:4]Port | [3:0]FanIndex
        // Byte 1-2: RPM big-endian
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
                port_fan_counts[port as usize] = port_fan_counts[port as usize].max(fan_index + 1);
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

    /// Set up fan groups via SetFanGroup (0xAD).
    ///
    /// Two groups per port for side mode support:
    ///   - base (top LEDs): `(port * 4) * 2`
    ///   - base+1 (bottom LEDs): `(port * 4) * 2 + 1`
    fn setup_fan_groups(&self, port_fan_counts: &[u8; 4]) -> Result<()> {
        for (port, &fan_count) in port_fan_counts.iter().enumerate() {
            if fan_count == 0 {
                continue;
            }

            let base_group = (port * 4 * 2) as u8;

            let mut top = vec![base_group, fan_count];
            for fan in 0..fan_count {
                top.push((1u8 << 7) | ((port as u8) << 4) | fan);
            }
            self.send_command_quiet(CMD_SET_FAN_GROUP, &top)?;

            let mut bot = vec![base_group + 1, fan_count];
            for fan in 0..fan_count {
                bot.push((1u8 << 6) | ((port as u8) << 4) | fan);
            }
            self.send_command_quiet(CMD_SET_FAN_GROUP, &bot)?;

            debug!(
                "Set fan groups {base_group}/{} for port {port}: {fan_count} fans",
                base_group + 1
            );
        }
        Ok(())
    }

    fn read_product_info(&self) -> Result<String> {
        let response =
            self.send_command_timeout(CMD_GET_PRODUCT_INFO, &[0x00, 0x00], INIT_READ_TIMEOUT_MS)?;
        let data_len = response[5] as usize;
        let data = &response[HEADER_LEN..HEADER_LEN + data_len.min(MAX_PAYLOAD)];

        let version = String::from_utf8_lossy(data)
            .trim_end_matches('\0')
            .to_string();

        // Consume second response (date/time) to keep buffer in sync.
        let mut dev = self.device.lock();
        let mut buf = [0u8; PACKET_SIZE];
        let n2 = dev
            .read_timeout(&mut buf, INIT_READ_TIMEOUT_MS)
            .unwrap_or(0);
        if n2 > 0 {
            let len2 = buf[5] as usize;
            let data2 = &buf[HEADER_LEN..HEADER_LEN + len2.min(MAX_PAYLOAD)];
            let date_str = String::from_utf8_lossy(data2)
                .trim_end_matches('\0')
                .to_string();
            debug!("Firmware date: {date_str}");
        }

        Ok(version)
    }

    /// Set fan speed (PWM duty) for a specific port and fan index.
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

    fn send_speed_locked(dev: &mut RusbHid, port: u8, fan_index: u8, duty: u8) -> Result<()> {
        let addr = (port << 4) | (fan_index & 0x0F);
        let pkt = Self::build_packet(CMD_SET_FAN_SPEED, &[addr, duty]);
        dev.read_flush();
        dev.write(&pkt).context("TL Fan: write fan speed")?;
        let mut buf = [0u8; PACKET_SIZE];
        let _ = dev.read_timeout(&mut buf, READ_TIMEOUT_MS);
        Ok(())
    }

    pub fn set_port_speed(&self, port: u8, duty: u8) -> Result<()> {
        let mut fan_count = self
            .last_handshake
            .lock()
            .as_ref()
            .map(|hs| hs.port_fan_counts[port as usize])
            .unwrap_or(1);

        if fan_count == 0 {
            let _ = self.handshake();
            fan_count = self
                .last_handshake
                .lock()
                .as_ref()
                .map(|hs| hs.port_fan_counts[port as usize])
                .unwrap_or(1);
        }

        for idx in 0..fan_count {
            self.set_fan_speed_single(port, idx, duty)?;
        }
        Ok(())
    }

    /// Set all port speeds atomically under one device lock.
    pub fn set_all_port_speeds(&self, duties: &[u8]) -> Result<()> {
        let mut fan_counts: [u8; 4] = self
            .last_handshake
            .lock()
            .as_ref()
            .map(|hs| hs.port_fan_counts)
            .unwrap_or([1, 1, 1, 1]);
        if fan_counts.iter().all(|&c| c == 0) {
            let _ = self.handshake();
            fan_counts = self
                .last_handshake
                .lock()
                .as_ref()
                .map(|hs| hs.port_fan_counts)
                .unwrap_or([1, 1, 1, 1]);
        }
        let mut dev = self.device.lock();
        for (port, &duty) in duties.iter().take(4).enumerate() {
            for idx in 0..fan_counts[port] {
                Self::send_speed_locked(&mut *dev, port as u8, idx, duty)?;
            }
        }
        Ok(())
    }

    /// Enable or disable motherboard RPM sync for a specific fan.
    pub fn set_mb_rpm_sync(&self, port: u8, fan_index: u8, sync: bool) -> Result<()> {
        if port >= 4 {
            bail!("Port {port} out of range (0-3)");
        }
        let data = ((sync as u8) << 7) | (port << 4) | (fan_index & 0x0F);
        self.send_command_quiet(CMD_SET_MB_RPM_SYNC, &[data])?;
        debug!("Set MB RPM sync port={port} fan={fan_index} sync={sync}");
        Ok(())
    }

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

    pub fn total_fan_count(&self) -> u8 {
        self.last_handshake
            .lock()
            .as_ref()
            .map(|hs| hs.port_fan_counts.iter().sum())
            .unwrap_or(0)
    }

    /// Set LED effect for a fan group on a port (SetFanGroupLight 0xB0).
    ///
    /// Payload layout:
    /// ```text
    /// [0]    = 0x00 (reserved)
    /// [1]    = group_num
    /// [2]    = mode % 1000 (effect mode byte)
    /// [3]    = brightness (0-4)
    /// [4]    = speed (0-4)
    /// [5-16] = R,G,B × 4 colors (12 bytes)
    /// [17]   = direction (0-5)
    /// [18]   = disable flag (0=enabled, 1=disabled)
    /// [19]   = color count
    /// ```
    pub fn set_group_light(&self, group: u8, effect: &RgbEffect) -> Result<()> {
        let mode_byte = effect.mode.to_tl_mode_byte().unwrap_or(3);

        let mut payload = [0u8; 20];
        payload[0] = 0x00;
        payload[1] = group;
        payload[2] = mode_byte;
        payload[3] = lianli_shared::rgb::brightness_scale(effect.brightness);
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

        self.send_command_quiet(CMD_SET_FAN_GROUP_LIGHT, &payload)?;
        debug!(
            "Set group {group} light: mode={mode_byte} brightness={} speed={} colors={color_count}",
            effect.brightness, effect.speed
        );
        Ok(())
    }

    pub fn set_group_light_noread(&self, group: u8, effect: &RgbEffect) -> Result<()> {
        let mode_byte = effect.mode.to_tl_mode_byte().unwrap_or(3);
        let mut payload = [0u8; 20];
        payload[0] = group;
        payload[1] = mode_byte;
        payload[2] = lianli_shared::rgb::brightness_scale(effect.brightness);
        payload[3] = effect.speed.min(4);
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
        self.send_command_noread(CMD_SET_FAN_GROUP_LIGHT, &payload)
    }

    /// Set LED effect for a specific fan (SetFanLight 0xA3). Per-fan light has
    /// no side bits — only usable with scope=All.
    ///
    /// Payload layout:
    /// ```text
    /// [0]    = (port << 4) | is_sync
    /// [1]    = (port << 4) | fan_index
    /// [2]    = mode % 1000
    /// [3]    = brightness (0-4)
    /// [4]    = speed (0-4)
    /// [5-16] = R,G,B × 4 colors
    /// [17]   = direction (0-5)
    /// [18]   = disable flag
    /// [19]   = color count
    /// ```
    pub fn set_fan_light(
        &self,
        port: u8,
        fan_index: u8,
        effect: &RgbEffect,
        sync: bool,
    ) -> Result<()> {
        if port >= 4 {
            bail!("Port {port} out of range (0-3)");
        }

        let mode_byte = effect.mode.to_tl_mode_byte().unwrap_or(3);

        let mut payload = [0u8; 20];
        payload[0] = (port << 4) | (sync as u8);
        payload[1] = (port << 4) | (fan_index & 0x0F);
        payload[2] = mode_byte;
        payload[3] = lianli_shared::rgb::brightness_scale(effect.brightness);
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
        debug!("Set port {port} fan {fan_index} light: mode={mode_byte} sync={sync}");
        Ok(())
    }

    pub fn set_fan_light_noread(
        &self,
        port: u8,
        fan_index: u8,
        effect: &RgbEffect,
        sync: bool,
    ) -> Result<()> {
        if port >= 4 {
            bail!("Port {port} out of range (0-3)");
        }
        let mode_byte = effect.mode.to_tl_mode_byte().unwrap_or(3);
        let mut payload = [0u8; 20];
        payload[0] = (port << 4) | (sync as u8);
        payload[1] = (port << 4) | (fan_index & 0x0F);
        payload[2] = mode_byte;
        payload[3] = lianli_shared::rgb::brightness_scale(effect.brightness);
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
        self.send_command_noread(CMD_SET_FAN_LIGHT, &payload)
    }

    /// Set fan direction flags for a specific fan.
    pub fn set_fan_direction(
        &self,
        port: u8,
        fan_index: u8,
        swap_lr: bool,
        swap_tb: bool,
    ) -> Result<()> {
        let addr = (port << 4) | (fan_index & 0x0F);
        let flags = ((swap_tb as u8) << 1) | (swap_lr as u8);
        self.send_command_quiet(CMD_SET_FAN_DIRECTION, &[addr, flags])?;
        debug!("Set fan direction port={port} fan={fan_index} swap_lr={swap_lr} swap_tb={swap_tb}");
        Ok(())
    }

    pub fn set_port_direction(&self, port: u8, swap: bool) -> Result<()> {
        self.send_command_quiet(CMD_SET_PORT_DIRECTION, &[port << 4, swap as u8])?;
        debug!("Set port {port} direction swap={swap}");
        Ok(())
    }

    pub fn test_port_light(&self, port: u8) -> Result<()> {
        self.send_command_quiet(CMD_TEST_LIGHT, &[0x03, port & 3, 0])?;
        Ok(())
    }

    pub fn blink_port(&self, port: u8) -> Result<()> {
        self.send_command_quiet(CMD_BLINK_PORT, &[port & 3])?;
        Ok(())
    }

    fn build_packet(cmd: u8, data: &[u8]) -> [u8; PACKET_SIZE] {
        let mut pkt = [0u8; PACKET_SIZE];
        pkt[0] = REPORT_ID;
        pkt[1] = cmd;
        pkt[2] = 0x00;
        pkt[3] = 0x00;
        pkt[4] = 0x00;
        pkt[5] = data.len() as u8;

        let copy_len = data.len().min(MAX_PAYLOAD);
        pkt[HEADER_LEN..HEADER_LEN + copy_len].copy_from_slice(&data[..copy_len]);
        pkt
    }

    fn send_command_quiet(&self, cmd: u8, data: &[u8]) -> Result<()> {
        let pkt = Self::build_packet(cmd, data);
        let mut dev = self.device.lock();
        dev.read_flush();
        dev.write(&pkt).context("TL Fan: write command")?;
        let mut buf = [0u8; PACKET_SIZE];
        let _ = dev.read_timeout(&mut buf, READ_TIMEOUT_MS);
        Ok(())
    }

    pub fn send_command_noread(&self, cmd: u8, data: &[u8]) -> Result<()> {
        let pkt = Self::build_packet(cmd, data);
        let mut dev = self.device.lock();
        dev.read_flush();
        dev.write(&pkt).context("TL Fan: write command")?;
        Ok(())
    }

    fn send_command_timeout(&self, cmd: u8, data: &[u8], timeout_ms: i32) -> Result<Vec<u8>> {
        let pkt = Self::build_packet(cmd, data);
        let mut dev = self.device.lock();

        dev.read_flush();

        dev.write(&pkt).context("TL Fan: write command")?;

        let mut buf = [0u8; PACKET_SIZE];
        for _ in 0..5 {
            let n = dev
                .read_timeout(&mut buf, timeout_ms)
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
        self.set_port_speed(slot, duty)
    }

    fn set_fan_speeds(&self, duties: &[u8]) -> Result<()> {
        self.set_all_port_speeds(duties)
    }

    fn read_fan_rpm(&self) -> Result<Vec<u16>> {
        let _ = self.handshake();

        let guard = self.last_handshake.lock();
        match guard.as_ref() {
            Some(hs) => {
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
        4
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
        true
    }

    fn supports_mb_sync(&self) -> bool {
        true
    }

    fn set_mb_rpm_sync(&self, port: u8, sync: bool) -> Result<()> {
        self.set_port_mb_rpm_sync(port, sync)
    }

    fn stop_pwm(&self) -> u8 {
        1
    }
}
