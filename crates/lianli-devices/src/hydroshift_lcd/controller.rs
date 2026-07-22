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
use lianli_transport::RusbHid;
use parking_lot::Mutex;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};

/// Used by `stream_h264_reader` to split the pipe byte stream into complete
/// access units (AUs).
///
/// C#'s `WMSQLibAV` (`WMSQLibAV.cs:230`) uses `avcodec_receive_packet()` which
/// returns one complete AU per call — no re-splitting needed. Since Rust reads
/// from a continuous pipe, we must detect AU boundaries ourselves.
///
/// Boundary policy (H.264 §7.4.1.2.3): if AUD NALs (type 9) are present, they
/// alone delimit AUs. Otherwise, primary coded picture NALs (types 1 and 5)
/// are the boundary markers. This is determined lazily from the first
/// boundary-eligible NAL encountered.
fn find_au_split(data: &[u8]) -> Option<usize> {
    let mut split_on_aud: Option<bool> = None;
    let mut found_first = false;
    let mut i = 0;
    while i + 3 < data.len() {
        let (sc_len, nal_type) = if data[i..].starts_with(&[0, 0, 0, 1]) {
            (4, data[i + 4] & 0x1F)
        } else if data[i..].starts_with(&[0, 0, 1]) {
            (3, data[i + 3] & 0x1F)
        } else {
            i += 1;
            continue;
        };

        if split_on_aud.is_none() && matches!(nal_type, 1 | 5 | 9) {
            split_on_aud = Some(nal_type == 9);
        }

        let is_boundary = match split_on_aud {
            Some(true) => nal_type == 9,
            Some(false) => matches!(nal_type, 1 | 5),
            None => false,
        };

        if is_boundary {
            if found_first {
                return Some(i);
            }
            found_first = true;
        }

        i += sc_len + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::find_au_split;

    const SC4: &[u8] = &[0, 0, 0, 1];

    fn nal(typ: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(SC4);
        v.push(typ);
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn no_aud_splits_at_second_slice() {
        // [SPS][PPS][IDR] [P] [P]
        let mut buf = Vec::new();
        buf.extend(nal(7, &[1, 2])); // SPS
        buf.extend(nal(8, &[3, 4])); // PPS
        buf.extend(nal(5, &[5, 6])); // IDR (boundary #1)
        let p1_off = buf.len();
        buf.extend(nal(1, &[7, 8])); // P-slice (boundary #2)
        buf.extend(nal(1, &[9, 10])); // P-slice

        let split = find_au_split(&buf).expect("should find a split");
        assert_eq!(split, p1_off);
        // Drained AU = [SPS PPS IDR] — the complete first AU
    }

    #[test]
    fn aud_delimited_splits_at_next_aud() {
        // [AUD][SPS][PPS][IDR] [AUD][P]
        let mut buf = Vec::new();
        buf.extend(nal(9, &[0x10])); // AUD (boundary #1)
        buf.extend(nal(7, &[1, 2])); // SPS — NOT a boundary
        buf.extend(nal(8, &[3, 4])); // PPS — NOT a boundary
        buf.extend(nal(5, &[5, 6])); // IDR — NOT a boundary (AUD policy active)
        let aud2_off = buf.len();
        buf.extend(nal(9, &[0x10])); // AUD (boundary #2)
        buf.extend(nal(1, &[7, 8])); // P-slice

        let split = find_au_split(&buf).expect("should find a split");
        assert_eq!(split, aud2_off);
        // Drained AU = [AUD SPS PPS IDR] — complete, includes the IDR
    }

    #[test]
    fn three_byte_start_code() {
        let mut buf = Vec::new();
        buf.extend(&[0, 0, 0, 1, 5, 1]); // 4-byte SC + IDR
        let split_off = buf.len();
        buf.extend(&[0, 0, 1, 1, 2]); // 3-byte SC + P-slice
        let split = find_au_split(&buf).expect("should find a split");
        assert_eq!(split, split_off);
    }

    #[test]
    fn no_split_in_partial_buffer() {
        // Only one slice NAL — not enough for a split
        let buf = nal(5, &[1, 2, 3]);
        assert!(find_au_split(&buf).is_none());
    }
}

/// HydroShift LCD / Galahad2 LCD AIO controller.
///
/// Provides pump + fan speed control, coolant temperature reading, and LCD streaming.
pub struct HydroShiftLcdController {
    device: Arc<Mutex<RusbHid>>,
    variant: AioLcdVariant,
    last_handshake: Option<AioHandshake>,
    brightness: u8,
    rotation: ScreenRotation,
    initialized: bool,
    use_c_command: bool,
    firmware_string: Option<String>,
    firmware_version: Option<(u32, u32)>,
    last_recovery_attempt: Option<Instant>,
}

impl HydroShiftLcdController {
    pub fn new(device: Arc<Mutex<RusbHid>>, pid: u16) -> Result<Self> {
        let variant = AioLcdVariant::from_pid(pid)
            .ok_or_else(|| anyhow::anyhow!("Unknown AIO LCD PID: {pid:#06x}"))?;

        Ok(Self {
            device,
            variant,
            last_handshake: None,
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
            "Handshake: fan={}rpm pump={}rpm temp_valid={} temp={:.1}°C",
            hs.fan_rpm, hs.pump_rpm, hs.temp_valid, hs.coolant_temp
        );
        self.last_handshake = Some(hs.clone());
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

    /// Public write_a_command. Locks `RusbHid` for the duration of the call —
    /// do NOT call when the device is already locked.
    pub fn write_a_command(&self, cmd: u8, data: &[u8]) -> Result<()> {
        let mut dev = self.device.lock();
        self.write_a_command_internal(&mut *dev, cmd, data)
    }

    fn write_a_command_internal(&self, dev: &mut RusbHid, cmd: u8, data: &[u8]) -> Result<()> {
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

    fn read_ack(&self, dev: &mut RusbHid, label: &str, timeout_ms: i32) {
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
        let pwm = duty_to_percent(duty);
        self.write_a_command(CMD_SET_PUMP_PWM, &[0x00, pwm])?;
        debug!("Set pump PWM to {pwm}%");
        Ok(())
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
