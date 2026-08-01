//! HydroShift II OLED Curve LED MCU driver (0x0416:0x8051).
//!
//! Separate microcontroller from the LCD MCU — handles pump PWM, motor,
//! RGB, and telemetry via 8-byte unencrypted bulk packets.
//!
use crate::traits::{AioDevice, FanDevice, RgbDevice};
use anyhow::{Context, Result};
use lianli_shared::rgb::{RgbEffect, RgbMode, RgbZoneInfo};
use lianli_transport::usb::{RusbBulk, LCD_READ_TIMEOUT, LCD_WRITE_TIMEOUT};
use parking_lot::Mutex;
use rusb::{Device, GlobalContext};
use std::sync::Arc;
use tracing::{debug, info};

const PACKET_SIZE: usize = 8;

// Opcodes
const CMD_GET_VER: u8 = 0x10;
const CMD_SET_MOTOR: u8 = 0x50;
const CMD_STOP_MOTOR: u8 = 0x51;
const CMD_GET_TEMP: u8 = 0x60;
const CMD_SET_PUMP: u8 = 0x61;
const CMD_GET_PUMP: u8 = 0x62;
const CMD_SET_MB_SYNC: u8 = 0x64;

// RGB ring push (opcode 0x11). 45-LED ring sent as 3 chunks of 15 LEDs each
// in 64-byte packets: chunk offsets 0/15/30 LEDs.
const CMD_PUSH_RGB: u8 = 0x11;
const RGB_LED_COUNT: usize = 45;
const RGB_PACKET_SIZE: usize = 64;

/// 22-segment pump RPM→PWM table (monotonically decreasing).
const PUMP_DATA_POINTS: &[(u16, u16)] = &[
    (1577, 250),
    (1608, 300),
    (1663, 400),
    (1725, 500),
    (1783, 600),
    (1838, 700),
    (1895, 800),
    (1950, 900),
    (2013, 1000),
    (2073, 1100),
    (2130, 1200),
    (2188, 1300),
    (2248, 1400),
    (2305, 1500),
    (2365, 1600),
    (2420, 1700),
    (2477, 1800),
    (2537, 1900),
    (2592, 2000),
    (2651, 2100),
    (2703, 2200),
    (2735, 2300),
];

/// Linear-interpolate the pump output value from user RPM.
fn rpm_to_output(rpm: u16) -> u16 {
    if rpm <= PUMP_DATA_POINTS[0].0 {
        return PUMP_DATA_POINTS[0].1;
    }
    if rpm >= PUMP_DATA_POINTS.last().unwrap().0 {
        return PUMP_DATA_POINTS.last().unwrap().1;
    }
    for window in PUMP_DATA_POINTS.windows(2) {
        let (r0, o0) = (window[0].0 as f32, window[0].1 as f32);
        let (r1, o1) = (window[1].0 as f32, window[1].1 as f32);
        if (rpm as f32) <= r1 {
            let t = ((rpm as f32) - r0) / (r1 - r0);
            return (o0 + t * (o1 - o0)).round() as u16;
        }
    }
    PUMP_DATA_POINTS.last().unwrap().1
}

pub struct Hs2OledLedController {
    transport: Mutex<RusbBulk>,
    firmware: Mutex<Option<String>>,
}

impl Hs2OledLedController {
    pub fn new(device: Device<GlobalContext>) -> Result<Self> {
        let mut transport =
            RusbBulk::open_device(device).context("opening HS2 OLED Curve LED MCU")?;
        transport
            .detach_and_configure("HS2 OLED Curve LED")
            .context("configuring HS2 OLED Curve LED MCU")?;
        info!("HS2 OLED Curve LED MCU opened");
        Ok(Self {
            transport: Mutex::new(transport),
            firmware: Mutex::new(None),
        })
    }

    fn send_and_read(&self, tx: &[u8; PACKET_SIZE]) -> Result<[u8; PACKET_SIZE]> {
        let transport = self.transport.lock();
        transport.write(tx, LCD_WRITE_TIMEOUT)?;
        let mut rx = [0u8; PACKET_SIZE];
        match transport.read(&mut rx, LCD_READ_TIMEOUT) {
            Ok(_) => Ok(rx),
            Err(_) => Ok(rx), // best-effort read
        }
    }

    /// GetLedVer (0x10) — firmware version.
    pub fn read_firmware(&self) -> Result<String> {
        let rx = self.send_and_read(&[CMD_GET_VER, 0, 0, 0, 0, 0, 0, 0])?;
        let fw = if rx[1] < 5 {
            format!("{}.{}", rx[1], rx[2])
        } else {
            // UTF-8 string starting at rx[2]
            String::from_utf8_lossy(&rx[2..])
                .trim_end_matches('\0')
                .to_string()
        };
        *self.firmware.lock() = Some(fw.clone());
        Ok(fw)
    }

    /// GetTemperlate (0x60) — returns (coolant_temp, mb_sync_flag).
    pub fn read_temps(&self) -> Result<(u8, u8)> {
        let rx = self.send_and_read(&[CMD_GET_TEMP, 0, 0, 0, 0, 0, 0, 0])?;
        Ok((rx[1], rx[2]))
    }

    /// SetPumpSpeed (0x61) — raw output value (from RPM→output curve).
    pub fn set_pump_output(&self, output_value: u16) -> Result<()> {
        let tx = [
            CMD_SET_PUMP,
            (output_value >> 8) as u8,
            (output_value & 0xFF) as u8,
            0,
            0,
            0,
            0,
            0,
        ];
        self.send_and_read(&tx)?;
        debug!("HS2 OLED: SetPumpSpeed output={output_value}");
        Ok(())
    }

    /// GetPumpSpeed (0x62) — read pump RPM with calibration offset.
    pub fn read_pump_rpm_raw(&self) -> Result<u16> {
        let rx = self.send_and_read(&[CMD_GET_PUMP, 0, 0, 0, 0, 0, 0, 0])?;
        let raw = u16::from_be_bytes([rx[1], rx[2]]);
        // Calibration: -40 if ≤1800, -50 if ≤2500, -60 otherwise
        let calibrated = if raw <= 1800 {
            raw.saturating_sub(40)
        } else if raw <= 2500 {
            raw.saturating_sub(50)
        } else {
            raw.saturating_sub(60)
        };
        Ok(calibrated)
    }

    /// SetMBSync (0x64) — inverted: 0=sync on, 1=sync off.
    pub fn set_mb_sync(&self, enabled: bool) -> Result<()> {
        let tx = [
            CMD_SET_MB_SYNC,
            if enabled { 0 } else { 1 },
            0,
            0,
            0,
            0,
            0,
            0,
        ];
        self.send_and_read(&tx)?;
        Ok(())
    }

    /// SetMotor (0x50) — motorized angle/Y-position.
    pub fn set_motor(&self, motor_index: u8, dir: u8, step: u32, speed: u8) -> Result<()> {
        let tx = [
            CMD_SET_MOTOR,
            motor_index,
            dir,
            (step >> 24) as u8,
            (step >> 16) as u8,
            (step >> 8) as u8,
            step as u8,
            speed,
        ];
        self.send_and_read(&tx)?;
        debug!("HS2 OLED: SetMotor idx={motor_index} dir={dir} step={step} speed={speed}");
        Ok(())
    }

    /// StopMotor (0x51).
    pub fn stop_motor(&self, motor_index: u8) -> Result<()> {
        let tx = [CMD_STOP_MOTOR, motor_index, 0, 0, 0, 0, 0, 0];
        self.send_and_read(&tx)?;
        Ok(())
    }

    /// Push one RGB frame to the 45-LED ring via opcode 0x11, sent as 3
    /// equal chunks of 15 LEDs each (offsets 0, 15, 30).
    pub fn send_rgb_frame(&self, colors: &[[u8; 3]]) -> Result<()> {
        let mut frame = [[0u8; 3]; RGB_LED_COUNT];
        for (dst, src) in frame.iter_mut().zip(colors.iter().take(RGB_LED_COUNT)) {
            *dst = *src;
        }

        let transport = self.transport.lock();
        let per_chunk = (RGB_LED_COUNT + 2) / 3;
        for chunk in 0..3usize {
            let start = chunk * per_chunk;
            let this_count = (RGB_LED_COUNT - start).min(per_chunk);
            let mut packet = [0u8; RGB_PACKET_SIZE];
            packet[0] = CMD_PUSH_RGB;
            packet[1] = start as u8;
            for led in 0..this_count {
                let led_idx = start + led;
                let off = 4 + led * 3;
                packet[off] = frame[led_idx][0];
                packet[off + 1] = frame[led_idx][1];
                packet[off + 2] = frame[led_idx][2];
            }
            transport
                .write(&packet, LCD_WRITE_TIMEOUT)
                .with_context(|| format!("HS2 OLED LED: write RGB chunk {chunk}"))?;
            let mut rx = [0u8; PACKET_SIZE];
            let _ = transport.read(&mut rx, LCD_READ_TIMEOUT);
        }
        Ok(())
    }
}

fn scale_brightness([r, g, b]: [u8; 3], brightness: u8) -> [u8; 3] {
    let scale = (lianli_shared::rgb::brightness_scale(brightness) as f32) / 4.0;
    [
        (r as f32 * scale).round() as u8,
        (g as f32 * scale).round() as u8,
        (b as f32 * scale).round() as u8,
    ]
}

impl FanDevice for Hs2OledLedController {
    fn set_fan_speed(&self, _slot: u8, _duty: u8) -> Result<()> {
        // No direct fan channels on the LED MCU; fans are motherboard-PWM
        // synced. This impl exists only to expose pump control via
        // `set_pump_speed` below.
        Ok(())
    }

    fn set_fan_speeds(&self, _duties: &[u8]) -> Result<()> {
        Ok(())
    }

    fn read_fan_rpm(&self) -> Result<Vec<u16>> {
        Ok(vec![])
    }

    fn fan_slot_count(&self) -> u8 {
        0
    }

    fn has_pump_control(&self) -> bool {
        true
    }

    fn set_pump_speed(&self, duty: u8) -> Result<()> {
        let pct = lianli_shared::fan::duty_to_percent(duty) as f32 / 100.0;
        let rpm = (1600.0 + pct * (2735.0 - 1600.0)).round() as u16;
        let output = rpm_to_output(rpm);
        self.set_pump_output(output)
    }
}

impl AioDevice for Hs2OledLedController {
    fn read_pump_rpm(&self) -> Result<u16> {
        self.read_pump_rpm_raw()
    }

    fn read_coolant_temp(&self) -> Result<f32> {
        let (t1, _t2) = self.read_temps()?;
        Ok(t1 as f32)
    }
}

impl RgbDevice for Hs2OledLedController {
    fn device_name(&self) -> String {
        "HydroShift II OLED Curve LED".to_string()
    }

    fn supported_modes(&self) -> Vec<RgbMode> {
        vec![RgbMode::Off, RgbMode::Static, RgbMode::Direct]
    }

    fn zone_info(&self) -> Vec<RgbZoneInfo> {
        vec![RgbZoneInfo {
            name: "Ring".to_string(),
            led_count: RGB_LED_COUNT as u16,
        }]
    }

    fn supports_direct(&self) -> bool {
        true
    }

    fn set_zone_effect(&self, zone: u8, effect: &RgbEffect) -> Result<()> {
        if zone != 0 {
            anyhow::bail!("HS2 OLED LED: zone {zone} out of range (only zone 0)");
        }
        let color = if effect.mode == RgbMode::Off || effect.disabled {
            [0, 0, 0]
        } else {
            let base = effect.colors.first().copied().unwrap_or([255, 255, 255]);
            scale_brightness(base, effect.brightness)
        };
        let frame = vec![color; RGB_LED_COUNT];
        self.send_rgb_frame(&frame)
    }

    fn set_direct_colors(&self, zone: u8, colors: &[[u8; 3]]) -> Result<()> {
        if zone != 0 {
            anyhow::bail!("HS2 OLED LED: zone {zone} out of range (only zone 0)");
        }
        self.send_rgb_frame(colors)
    }
}

pub struct Hs2OledLedDriver;

impl crate::registry::DeviceDriver for Hs2OledLedDriver {
    fn family(&self) -> lianli_shared::device_id::DeviceFamily {
        lianli_shared::device_id::DeviceFamily::HydroShift2OledCurveLed
    }

    fn open(
        &self,
        ctx: &crate::registry::OpenContext,
    ) -> anyhow::Result<crate::registry::OpenedDevice> {
        let ctrl =
            std::sync::Arc::new(Hs2OledLedController::new(rusb::Device::clone(&ctx.device))?);
        let firmware = ctrl.read_firmware().ok();
        Ok(crate::registry::OpenedDevice {
            id: ctx.device_id(),
            family: lianli_shared::device_id::DeviceFamily::HydroShift2OledCurveLed,
            capabilities: lianli_shared::device_id::DeviceFamily::HydroShift2OledCurveLed
                .capabilities(),
            transport_kind: lianli_shared::device_id::TransportKind::UsbBulk,
            model_name: "HydroShift II OLED Curve LED".to_string(),
            firmware,
            fan: Some(Box::new(std::sync::Arc::clone(&ctrl))),
            lcd: None,
            rgb: vec![(
                String::new(),
                std::sync::Arc::clone(&ctrl) as Arc<dyn crate::traits::RgbDevice>,
            )],
            aio: Some(Box::new(ctrl)),
            shared_hid: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::rpm_to_output;

    #[test]
    fn rpm_table_clamps_low() {
        assert_eq!(rpm_to_output(1000), 250);
        assert_eq!(rpm_to_output(1577), 250);
    }

    #[test]
    fn rpm_table_clamps_high() {
        assert_eq!(rpm_to_output(3000), 2300);
        assert_eq!(rpm_to_output(2735), 2300);
    }

    #[test]
    fn rpm_table_interpolates() {
        // Between 1577→250 and 1608→300: midpoint 1592.5 → ~275
        let v = rpm_to_output(1592);
        assert!(v >= 270 && v <= 280, "got {v}");
    }
}
