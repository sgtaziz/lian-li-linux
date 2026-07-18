use super::protocol::{
    build_lcd_packet, duty_to_percent, parse_firmware_version, ACK_TIMEOUT_MS, A_HEADER_LEN,
    A_PACKET_SIZE, B_HEADER_LEN, B_MAX_PAYLOAD, B_PACKET_SIZE, CMD_GET_FIRMWARE, CMD_HANDSHAKE,
    CMD_LCD_AVAILABLE, CMD_LCD_CONTROL, CMD_RESET_DEVICE, CMD_SEND_H264, CMD_SEND_JPEG,
    CMD_SET_FAN_PWM, CMD_SET_PUMP_PWM, C_MAX_PAYLOAD, C_PACKET_SIZE, INIT_READ_TIMEOUT_MS,
    READ_TIMEOUT_MS, REPORT_ID_A, REPORT_ID_B, REPORT_ID_C,
};
use super::{AioHandshake, AioLcdVariant, LcdControlMode, ScreenRotation};
use crate::traits::{AioDevice, FanDevice, LcdDevice};
use anyhow::{bail, Context, Result};
use lianli_shared::screen::ScreenInfo;
use lianli_transport::HidBackend;
use parking_lot::Mutex;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};

/// Used by `stream_h264_reader` to split the pipe byte stream into complete frames.
fn find_au_split(data: &[u8]) -> Option<usize> {
    let mut found_first = false;
    let mut i = 0;
    while i + 4 < data.len() {
        if data[i..i + 4] == [0, 0, 0, 1] {
            let nal_type = data[i + 4] & 0x1F;
            if matches!(nal_type, 1 | 5 | 9) {
                if found_first {
                    return Some(i);
                }
                found_first = true;
            }
        }
        i += 1;
    }
    None
}

fn parse_handshake_response(resp: &[u8]) -> Result<AioHandshake> {
    if resp.len() < A_HEADER_LEN {
        bail!(
            "AIO LCD: handshake response too short ({} < {A_HEADER_LEN} bytes)",
            resp.len()
        );
    }
    if resp[0] != REPORT_ID_A || resp[1] != CMD_HANDSHAKE {
        bail!(
            "AIO LCD: unexpected handshake response (report={:#04x}, command={:#04x})",
            resp[0],
            resp[1]
        );
    }

    let data_len = resp[5] as usize;
    let available = resp.len() - A_HEADER_LEN;
    if data_len > available {
        bail!("AIO LCD: truncated handshake payload ({data_len} declared, {available} available)");
    }
    if data_len < 4 {
        bail!("AIO LCD: handshake payload too short ({data_len} bytes)");
    }

    let data = &resp[A_HEADER_LEN..A_HEADER_LEN + data_len];
    let temp_valid = data_len >= 5 && data[4] != 0;
    let coolant_temp = if data_len >= 7 {
        let integer = data[5] as f32;
        let fraction = (data[6] % 10) as f32 / 10.0;
        integer + fraction
    } else {
        0.0
    };

    Ok(AioHandshake {
        fan_rpm: u16::from_be_bytes([data[0], data[1]]),
        pump_rpm: u16::from_be_bytes([data[2], data[3]]),
        temp_valid,
        coolant_temp,
    })
}

fn validate_coolant_sample(hs: &AioHandshake) -> Result<f32> {
    if !hs.temp_valid || !hs.coolant_temp.is_finite() || !(0.0..=100.0).contains(&hs.coolant_temp) {
        bail!(
            "Coolant telemetry invalid (temp={:.1}°C, pump={}rpm, valid={})",
            hs.coolant_temp,
            hs.pump_rpm,
            hs.temp_valid
        );
    }

    // Some controllers briefly return this placeholder while the LCD
    // interface is starting. Reject the complete signature, but do not reject
    // a real hot sample or a zero-RPM pump: both must reach the fail-safe path.
    if hs.fan_rpm == 0 && hs.pump_rpm == 0 && (hs.coolant_temp - 1.0).abs() < f32::EPSILON {
        bail!("Coolant telemetry returned the startup placeholder");
    }

    Ok(hs.coolant_temp)
}

#[cfg(test)]
mod telemetry_tests {
    use super::*;

    fn response(fan_rpm: u16, pump_rpm: u16, valid: bool, temp: (u8, u8)) -> Vec<u8> {
        let mut packet = vec![0; A_HEADER_LEN + 7];
        packet[0] = REPORT_ID_A;
        packet[1] = CMD_HANDSHAKE;
        packet[5] = 7;
        packet[6..8].copy_from_slice(&fan_rpm.to_be_bytes());
        packet[8..10].copy_from_slice(&pump_rpm.to_be_bytes());
        packet[10] = u8::from(valid);
        packet[11] = temp.0;
        packet[12] = temp.1;
        packet
    }

    #[test]
    fn parses_complete_handshake() {
        let hs = parse_handshake_response(&response(1234, 2876, true, (35, 7))).unwrap();
        assert_eq!(hs.fan_rpm, 1234);
        assert_eq!(hs.pump_rpm, 2876);
        assert!(hs.temp_valid);
        assert!((hs.coolant_temp - 35.7).abs() < 0.01);
    }

    #[test]
    fn rejects_every_truncated_handshake_without_panicking() {
        let complete = response(1234, 2876, true, (35, 7));
        for len in 0..complete.len() {
            assert!(
                parse_handshake_response(&complete[..len]).is_err(),
                "length {len} unexpectedly parsed"
            );
        }
    }

    #[test]
    fn rejects_mismatched_report_and_command() {
        let mut packet = response(1234, 2876, true, (35, 7));
        packet[0] = REPORT_ID_B;
        assert!(parse_handshake_response(&packet).is_err());

        packet[0] = REPORT_ID_A;
        packet[1] = CMD_GET_FIRMWARE;
        assert!(parse_handshake_response(&packet).is_err());
    }

    #[test]
    fn rejects_declared_payload_larger_than_response() {
        let mut packet = response(1234, 2876, true, (35, 7));
        packet[5] = 8;
        assert!(parse_handshake_response(&packet).is_err());
    }

    #[test]
    fn validates_hot_and_zero_pump_samples() {
        let hot = parse_handshake_response(&response(900, 0, true, (99, 9))).unwrap();
        assert!((validate_coolant_sample(&hot).unwrap() - 99.9).abs() < 0.01);
    }

    #[test]
    fn rejects_invalid_and_placeholder_samples() {
        let invalid = parse_handshake_response(&response(900, 2800, false, (35, 0))).unwrap();
        assert!(validate_coolant_sample(&invalid).is_err());

        let placeholder = parse_handshake_response(&response(0, 0, true, (1, 0))).unwrap();
        assert!(validate_coolant_sample(&placeholder).is_err());
    }
}

/// HydroShift LCD / Galahad2 LCD AIO controller.
///
/// Provides pump + fan speed control, coolant temperature reading, and LCD streaming.
pub struct HydroShiftLcdController {
    device: Arc<Mutex<HidBackend>>,
    variant: AioLcdVariant,
    last_handshake: Mutex<Option<AioHandshake>>,
    brightness: u8,
    rotation: ScreenRotation,
    initialized: bool,
    use_c_command: bool,
    firmware_string: Option<String>,
    firmware_version: Option<(u32, u32)>,
    last_recovery_attempt: Option<Instant>,
}

impl HydroShiftLcdController {
    pub fn new(device: Arc<Mutex<HidBackend>>, pid: u16) -> Result<Self> {
        let variant = AioLcdVariant::from_pid(pid)
            .ok_or_else(|| anyhow::anyhow!("Unknown AIO LCD PID: {pid:#06x}"))?;

        Ok(Self {
            device,
            variant,
            last_handshake: Mutex::new(None),
            brightness: 50,
            rotation: ScreenRotation::Rotate0,
            initialized: false,
            use_c_command: false,
            firmware_string: None,
            firmware_version: None,
            last_recovery_attempt: None,
        })
    }

    fn init(&mut self) -> Result<()> {
        if self.initialized {
            return Ok(());
        }
        info!("Initializing {}", self.variant.name());

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

        if let Err(e) = self.apply_lcd_settings() {
            warn!("  apply_lcd_settings failed: {e:#}");
        }

        self.initialized = true;
        self.last_recovery_attempt = Some(Instant::now());
        Ok(())
    }

    pub fn supports_c_command(&self) -> bool {
        self.firmware_version
            .map(|v| v >= self.variant.c_command_min_firmware())
            .unwrap_or(false)
    }

    pub fn set_use_c_command(&mut self, enable: bool) {
        self.use_c_command = enable && self.supports_c_command();
        debug!(
            "AIO LCD: use_c_command set to {} (request={enable}, supported={})",
            self.use_c_command,
            self.supports_c_command()
        );
    }

    pub fn firmware_version_str(&self) -> Option<&str> {
        self.firmware_string.as_deref()
    }

    pub fn try_read_firmware(&mut self) -> Result<()> {
        if self.firmware_version.is_some() {
            return Ok(());
        }
        let fw = self.read_firmware_internal(INIT_READ_TIMEOUT_MS)?;
        self.firmware_version = parse_firmware_version(&fw);
        self.firmware_string = Some(fw.clone());
        info!("AIO LCD firmware for {}: {fw}", self.variant.name());
        Ok(())
    }

    pub fn handshake(&self) -> Result<AioHandshake> {
        let timeout = if self.initialized {
            READ_TIMEOUT_MS
        } else {
            INIT_READ_TIMEOUT_MS
        };
        let resp = self.send_a_command(CMD_HANDSHAKE, &[], timeout)?;
        let hs = parse_handshake_response(&resp)?;

        debug!(
            "Handshake: fan={}rpm pump={}rpm temp_valid={} temp={:.1}°C",
            hs.fan_rpm, hs.pump_rpm, hs.temp_valid, hs.coolant_temp
        );
        *self.last_handshake.lock() = Some(hs.clone());
        Ok(hs)
    }

    pub fn apply_lcd_settings(&self) -> Result<()> {
        let mut payload = [0u8; 8];
        payload[0] = LcdControlMode::Application as u8;
        payload[1] = self.brightness;
        payload[2] = self.rotation as u8;
        payload[7] = 24;

        self.send_b_command(CMD_LCD_CONTROL, &payload)?;
        debug!(
            "LCD settings applied: brightness={}, rotation={:?}",
            self.brightness, self.rotation
        );
        Ok(())
    }

    pub fn send_jpeg(&self, jpeg_data: &[u8]) -> Result<()> {
        self.send_chunked(CMD_SEND_JPEG, jpeg_data)
    }

    pub fn send_h264_frame(&self, frame: &[u8]) -> Result<()> {
        self.send_chunked(CMD_SEND_H264, frame)
    }

    pub fn stream_h264_reader(
        &self,
        reader: &mut dyn Read,
        stop: &AtomicBool,
        fps: f32,
    ) -> Result<()> {
        let frame_interval = std::time::Duration::from_secs_f32(1.0 / fps.max(1.0));
        let mut read_buf = vec![0u8; 64 * 1024];
        let mut accum: Vec<u8> = Vec::with_capacity(256 * 1024);
        let mut next_deadline = std::time::Instant::now() + frame_interval;
        loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let n = reader
                .read(&mut read_buf)
                .context("AIO LCD: read h264 stream")?;
            if n == 0 {
                break;
            }
            accum.extend_from_slice(&read_buf[..n]);
            while let Some(split) = find_au_split(&accum) {
                let au: Vec<u8> = accum.drain(..split).collect();
                if !au.is_empty() {
                    self.send_h264_frame(&au)?;
                    let now = std::time::Instant::now();
                    if now < next_deadline {
                        std::thread::sleep(next_deadline - now);
                    }
                    next_deadline += frame_interval;
                    if next_deadline < std::time::Instant::now() {
                        next_deadline = std::time::Instant::now() + frame_interval;
                    }
                }
            }
        }
        if !accum.is_empty() {
            self.send_h264_frame(&accum)?;
        }
        Ok(())
    }

    pub fn variant(&self) -> AioLcdVariant {
        self.variant
    }

    pub fn is_lcd_available(&self) -> Result<bool> {
        let mut dev = self.device.lock();

        let mut pkt = vec![0u8; B_PACKET_SIZE];
        pkt[0] = REPORT_ID_B;
        pkt[1] = CMD_LCD_AVAILABLE;
        dev.write(&pkt)
            .context("AIO LCD: write LCD available check")?;

        let mut buf = vec![0u8; B_PACKET_SIZE];
        loop {
            let n = dev
                .read_timeout(&mut buf, READ_TIMEOUT_MS)
                .context("AIO LCD: read LCD available response")?;

            if n == 0 {
                return Ok(false);
            }
            if buf[1] == CMD_LCD_AVAILABLE {
                let data_len = (buf[9] as usize) << 8 | buf[10] as usize;
                return Ok(data_len == 1 && buf[B_HEADER_LEN] == 0);
            }
            debug!(
                "AIO LCD: is_lcd_available: skipping stale response cmd={:#04x}",
                buf[1]
            );
        }
    }

    pub fn reset_device(&self) -> bool {
        const MAX_ATTEMPTS: u32 = 20;

        let mut dev = self.device.lock();
        if let Err(e) = self.write_a_command_internal(&mut *dev, CMD_RESET_DEVICE, &[]) {
            warn!("AIO LCD: reset device failed: {e}");
            return false;
        }

        let mut buf = [0u8; A_PACKET_SIZE];
        for _ in 0..MAX_ATTEMPTS {
            let n = match dev.read_timeout(&mut buf, 1000) {
                Ok(n) => n,
                Err(e) => {
                    warn!("AIO LCD: reset device read failed: {e}");
                    return false;
                }
            };
            if n == 0 {
                warn!("AIO LCD: reset device timed out");
                return false;
            }
            if n > A_HEADER_LEN && buf[1] == CMD_RESET_DEVICE {
                match buf[A_HEADER_LEN] {
                    1 => return true,
                    2 => continue,
                    _ => {}
                }
            }
        }
        warn!("AIO LCD: reset device did not converge after {MAX_ATTEMPTS} reads");
        false
    }

    pub fn check_and_recover_lcd(&mut self) -> Result<crate::traits::RecoveryAction> {
        use crate::traits::RecoveryAction;
        if !self.use_c_command {
            return Ok(RecoveryAction::NoChange);
        }
        const RECOVERY_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(15);
        if self
            .last_recovery_attempt
            .map(|t| t.elapsed() < RECOVERY_COOLDOWN)
            .unwrap_or(false)
        {
            return Ok(RecoveryAction::NoChange);
        }
        match self.is_lcd_available() {
            Ok(true) => Ok(RecoveryAction::NoChange),
            Ok(false) => {
                warn!("LCD not available, attempting reset");
                self.last_recovery_attempt = Some(Instant::now());
                if self.reset_device() {
                    info!("Device reset successful, reinitializing LCD");
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    self.apply_lcd_settings()?;
                    Ok(RecoveryAction::Recovered)
                } else {
                    warn!("Device reset failed");
                    Ok(RecoveryAction::NoChange)
                }
            }
            Err(e) => {
                debug!("LCD availability check failed: {e:#}");
                Ok(RecoveryAction::NoChange)
            }
        }
    }

    fn read_firmware_internal(&self, timeout_ms: i32) -> Result<String> {
        let mut dev = self.device.lock();

        self.write_a_command_internal(&mut *dev, CMD_GET_FIRMWARE, &[])?;

        // Loop reading until we see a firmware response, discarding stale
        // responses from a previous session (e.g. handshake/reset).
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

        // Response 2 (date/time string) must be consumed to keep buffer in sync.
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
        let mut dev = self.device.lock();
        self.write_a_command_internal(&mut *dev, cmd, data)?;

        let mut buf = [0u8; A_PACKET_SIZE];
        let timeout = std::time::Duration::from_millis(timeout_ms.max(0) as u64);
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                bail!(
                    "AIO LCD: no matching response to A-command {cmd:#04x} (timeout after {timeout_ms}ms)"
                );
            }
            let remaining_ms = remaining.as_millis().min(i32::MAX as u128).max(1) as i32;
            let n = dev
                .read_timeout(&mut buf, remaining_ms)
                .context("AIO LCD: read A-response")?;

            debug!(
                "A-cmd {cmd:#04x}: response {n} bytes, raw={:02x?}",
                &buf[..n.min(20)]
            );

            if n == 0 {
                bail!(
                    "AIO LCD: no response to A-command {cmd:#04x} (timeout after {timeout_ms}ms)"
                );
            }
            if n >= 2 && buf[0] == REPORT_ID_A && buf[1] == cmd {
                return Ok(buf[..n].to_vec());
            }
            debug!("A-cmd {cmd:#04x}: skipping stale or unrelated response");
        }
    }

    /// Public write_a_command. Locks `HidBackend` for the duration of the call —
    /// do NOT call when the device is already locked.
    pub fn write_a_command(&self, cmd: u8, data: &[u8]) -> Result<()> {
        let mut dev = self.device.lock();
        self.write_a_command_internal(&mut *dev, cmd, data)
    }

    fn write_a_command_internal(&self, dev: &mut HidBackend, cmd: u8, data: &[u8]) -> Result<()> {
        let max_payload = A_PACKET_SIZE - A_HEADER_LEN;
        if data.len() > max_payload {
            bail!(
                "AIO LCD: A-command {cmd:#04x} payload too large ({} > {max_payload})",
                data.len()
            );
        }
        let mut pkt = [0u8; A_PACKET_SIZE];
        pkt[0] = REPORT_ID_A;
        pkt[1] = cmd;
        pkt[5] = data.len() as u8;
        pkt[A_HEADER_LEN..A_HEADER_LEN + data.len()].copy_from_slice(data);

        let written = dev.write(&pkt).context("AIO LCD: write A-command")?;
        debug!(
            "A-cmd {cmd:#04x}: wrote {written} bytes, payload={:02x?}",
            data
        );
        Ok(())
    }

    fn send_b_command(&self, cmd: u8, data: &[u8]) -> Result<()> {
        let total_size = data.len();
        let mut offset = 0;
        let mut packet_num: u32 = 0;
        let mut dev = self.device.lock();

        loop {
            let remaining = total_size.saturating_sub(offset);
            let chunk_len = remaining.min(B_MAX_PAYLOAD);

            let pkt = build_lcd_packet(
                REPORT_ID_B,
                B_PACKET_SIZE,
                cmd,
                total_size as u32,
                packet_num,
                if chunk_len > 0 {
                    &data[offset..offset + chunk_len]
                } else {
                    &[]
                },
            );

            dev.write(&pkt).context("AIO LCD: write B command")?;

            offset += chunk_len;
            packet_num += 1;

            if offset >= total_size {
                break;
            }
        }

        self.read_ack(&mut dev, "send_b_command", READ_TIMEOUT_MS);
        Ok(())
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
        let mut dev = self.device.lock();

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

        self.read_ack(&mut dev, "send_chunked", ACK_TIMEOUT_MS);
        Ok(())
    }

    fn read_ack(&self, dev: &mut HidBackend, label: &str, timeout_ms: i32) {
        let mut buf = [0u8; 512];
        if let Err(e) = dev.read_timeout(&mut buf, timeout_ms) {
            debug!("AIO LCD: {label} ack: {e:#}");
        }
    }
}

impl FanDevice for HydroShiftLcdController {
    fn set_fan_speed(&self, _slot: u8, duty: u8) -> Result<()> {
        let pwm = duty_to_percent(duty);
        self.write_a_command(CMD_SET_FAN_PWM, &[0x00, pwm])?;
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
            .lock()
            .as_ref()
            .map(|hs| hs.fan_rpm)
            .unwrap_or(0)])
    }

    fn fan_slot_count(&self) -> u8 {
        1
    }

    fn poll_coolant_temp(&self) -> Result<Option<f32>> {
        self.handshake()
            .and_then(|hs| validate_coolant_sample(&hs))
            .map(Some)
    }

    fn has_pump_control(&self) -> bool {
        true
    }

    fn set_pump_speed(&self, duty: u8) -> Result<()> {
        let pwm = duty_to_percent(duty);
        self.write_a_command(CMD_SET_PUMP_PWM, &[0x00, pwm])?;
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
    fn poll_coolant_temp(&self) -> Result<Option<f32>> {
        <HydroShiftLcdController as FanDevice>::poll_coolant_temp(self)
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
            .lock()
            .as_ref()
            .map(|hs| hs.pump_rpm)
            .unwrap_or(0))
    }

    fn read_coolant_temp(&self) -> Result<f32> {
        match &*self.last_handshake.lock() {
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

    fn check_and_recover_lcd(&mut self) -> Result<crate::traits::RecoveryAction> {
        HydroShiftLcdController::check_and_recover_lcd(self)
    }

    fn supports_c_command(&self) -> bool {
        HydroShiftLcdController::supports_c_command(self)
    }

    fn firmware_version_str(&self) -> Option<&str> {
        HydroShiftLcdController::firmware_version_str(self)
    }

    fn set_use_c_command(&mut self, enable: bool) {
        HydroShiftLcdController::set_use_c_command(self, enable);
    }

    fn try_read_firmware(&mut self) -> Result<()> {
        HydroShiftLcdController::try_read_firmware(self)
    }

    fn stream_h264_reader(
        &mut self,
        reader: &mut dyn Read,
        stop: &AtomicBool,
        fps: f32,
    ) -> Result<()> {
        HydroShiftLcdController::stream_h264_reader(self, reader, stop, fps)
    }
}
