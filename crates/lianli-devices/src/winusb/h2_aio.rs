//! HydroShift II AIO controller — pump + fan + RGB ring.
//!
//! Shares the LCD device's USB handle via `Arc<Mutex<RusbBulk>>`.

use crate::crypto::PacketBuilder;
use crate::traits::{AioDevice, FanDevice, RgbDevice};
use anyhow::{Context, Result};
use lianli_shared::fan::duty_to_percent;
use lianli_shared::rgb::{RgbEffect, RgbMode, RgbZoneInfo};
use lianli_transport::usb::{RusbBulk, LCD_READ_TIMEOUT, LCD_WRITE_TIMEOUT};
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Duration;
use tracing::debug;

const PUMP_MIN_RPM: u16 = 1600;
const PUMP_MAX_RPM_CIRCLE: u16 = 2500;
const PUMP_MAX_RPM_SQUARE: u16 = 3200;
const RING_LED_COUNT: usize = 24;

/// Telemetry parsed from GetH2Params response.
pub struct H2Params {
    pub cpu_temp: u8,
    pub cpu_load: u8,
    pub gpu_temp: u8,
    pub gpu_load: u8,
    pub pump_rpm: u16,
    pub fan_rpm: [u16; 3],
    pub coolant_temp: u8,
    pub mac: Option<[u8; 6]>,
}

/// After LCD play mode the device ignores control commands until this
/// StopPlay → StopClock → GetVer preamble re-arms the channel.
fn wake(transport: &Arc<Mutex<RusbBulk>>) {
    let mut builder = PacketBuilder::new();
    let cmds = [
        builder.stop_play_header_winusb(),
        builder.stop_clock_header_winusb(),
        builder.get_ver_header_winusb(),
    ];
    for cmd in &cmds {
        let t = transport.lock();
        let _ = t.write(cmd, LCD_WRITE_TIMEOUT);
        let mut buf = [0u8; 512];
        let _ = t.read(&mut buf, LCD_READ_TIMEOUT);
        drop(t);
        std::thread::sleep(Duration::from_millis(150));
    }
    debug!("H2 control channel: wake preamble sent");
}

/// HydroShift II AIO controller (pump + fan + RGB ring via shared handle).
pub struct H2AioController {
    transport: Arc<Mutex<RusbBulk>>,
    builder: Mutex<PacketBuilder>,
    last_fan_duties: Mutex<[u8; 3]>,
    last_pump_duty: Mutex<u8>,
    is_square: bool,
}

impl H2AioController {
    pub fn new(transport: Arc<Mutex<RusbBulk>>, pid: u16) -> Self {
        let ctrl = Self {
            transport: Arc::clone(&transport),
            builder: Mutex::new(PacketBuilder::new()),
            last_fan_duties: Mutex::new([50, 50, 50]),
            last_pump_duty: Mutex::new(128),
            is_square: pid == 0xA034,
        };
        wake(&transport);
        tracing::info!("HydroShift II control channel opened (shared transport)");
        ctrl
    }

    /// Read telemetry via GetH2Params (0xFA).
    pub fn get_h2_params(&self) -> Result<H2Params> {
        let header = self.builder.lock().get_h2_params_header_winusb();
        {
            let transport = self.transport.lock();
            transport
                .write(&header, LCD_WRITE_TIMEOUT)
                .context("H2: GetH2Params write")?;
        }

        let mut buf = [0u8; 512];
        let n = {
            let transport = self.transport.lock();
            transport
                .read(&mut buf, LCD_READ_TIMEOUT)
                .context("H2: GetH2Params read")?
        };

        if n < 32 {
            anyhow::bail!("H2: GetH2Params response too short ({n} bytes)");
        }

        Ok(H2Params {
            cpu_temp: 0,
            cpu_load: 0,
            gpu_temp: 0,
            gpu_load: 0,
            pump_rpm: u16::from_be_bytes([buf[20], buf[21]]),
            fan_rpm: [
                u16::from_be_bytes([buf[14], buf[15]]),
                u16::from_be_bytes([buf[16], buf[17]]),
                u16::from_be_bytes([buf[18], buf[19]]),
            ],
            coolant_temp: buf[13],
            mac: {
                let m = [buf[22], buf[23], buf[24], buf[25], buf[26], buf[27]];
                if m.iter().all(|&b| b == 0) {
                    None
                } else {
                    Some(m)
                }
            },
        })
    }

    /// Send pump + fan PWM via SyncPumpFan (0xFB).
    pub fn sync_pump_fan(&self, pump_pwm: u16, fan_duties: [u8; 3]) -> Result<()> {
        let header = self.builder.lock().sync_pump_fan_header_winusb(
            pump_pwm,
            fan_duties[0],
            fan_duties[1],
            fan_duties[2],
        );
        let transport = self.transport.lock();
        transport
            .write(&header, LCD_WRITE_TIMEOUT)
            .context("H2: SyncPumpFan write")?;
        let mut buf = [0u8; 512];
        let _ = transport.read(&mut buf, Duration::from_millis(50));
        debug!("H2: SyncPumpFan pump_pwm={pump_pwm} fans={:?}", fan_duties);
        Ok(())
    }

    /// Upload full-ring RGB frames via PushRgbData (0xFC); firmware loops
    /// them at `interval_ms`.
    pub fn send_rgb_frames(&self, frames: &[Vec<[u8; 3]>], interval_ms: u8) -> Result<()> {
        if frames.is_empty() {
            return Ok(());
        }
        let total_frames = frames.len();

        let mut raw = Vec::with_capacity(total_frames * RING_LED_COUNT * 3);
        for frame in frames {
            for led in 0..RING_LED_COUNT {
                let c = frame.get(led).copied().unwrap_or([0, 0, 0]);
                raw.extend_from_slice(&c);
            }
        }

        let compressed = crate::tinyuz::compress(&raw).context("compressing RGB data")?;

        let mut payload = compressed;
        payload.push((total_frames >> 8) as u8);
        payload.push((total_frames & 0xFF) as u8);
        payload.push(interval_ms);
        payload.push(RING_LED_COUNT as u8);

        let header = self
            .builder
            .lock()
            .push_rgb_data_header_winusb(payload.len());
        let mut packet = Vec::with_capacity(512 + payload.len());
        packet.extend_from_slice(&header);
        packet.extend_from_slice(&payload);

        let transport = self.transport.lock();
        transport
            .write(&packet, LCD_WRITE_TIMEOUT)
            .context("H2: PushRgbData write")?;
        let mut buf = [0u8; 512];
        let _ = transport.read(&mut buf, Duration::from_millis(100));
        debug!(
            "H2: PushRgbData {} frame(s), {} LEDs, {} bytes",
            total_frames,
            RING_LED_COUNT,
            payload.len()
        );
        Ok(())
    }

    fn pump_max_rpm(&self) -> u16 {
        if self.is_square {
            PUMP_MAX_RPM_SQUARE
        } else {
            PUMP_MAX_RPM_CIRCLE
        }
    }

    fn rpm_to_pwm(&self, rpm: u16) -> u16 {
        let rpm = rpm.clamp(PUMP_MIN_RPM, self.pump_max_rpm()) as f32;
        let pwm = if self.is_square {
            if rpm <= 1800.0 {
                1590.0 - (rpm - 1600.0) * 0.95
            } else if rpm <= 2000.0 {
                1400.0 - (rpm - 1800.0)
            } else if rpm <= 2200.0 {
                1200.0 - (rpm - 2000.0)
            } else if rpm <= 2400.0 {
                1000.0 - (rpm - 2200.0)
            } else if rpm <= 2600.0 {
                800.0 - (rpm - 2400.0)
            } else if rpm <= 2800.0 {
                580.0 - (rpm - 2600.0) * 1.11
            } else if rpm <= 3000.0 {
                330.0 - (rpm - 2800.0) * 1.2
            } else {
                90.0 - (rpm - 3000.0) * 0.45
            }
        } else {
            if rpm < 1720.0 {
                1500.0 - (rpm - 1600.0) * 1.625
            } else if rpm < 1870.0 {
                1300.0 - (rpm - 1720.0) * 2.0
            } else if rpm < 2000.0 {
                1000.0 - (rpm - 1870.0) * 1.23
            } else if rpm < 2300.0 {
                840.0 - (rpm - 2000.0) * 2.0
            } else if rpm < 2400.0 {
                240.0 - (rpm - 2300.0) * 1.8
            } else {
                60.0 - (rpm - 2400.0) * 0.5
            }
        };
        pwm.round() as u16
    }

    fn duty_to_pwm(&self, duty: u8) -> u16 {
        let pct = (duty as f32 / 255.0).clamp(0.0, 1.0);
        let rpm = PUMP_MIN_RPM as f32 + pct * (self.pump_max_rpm() - PUMP_MIN_RPM) as f32;
        self.rpm_to_pwm(rpm.round() as u16)
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

impl FanDevice for H2AioController {
    fn set_fan_speed(&self, slot: u8, duty: u8) -> Result<()> {
        let mut duties = *self.last_fan_duties.lock();
        duties[slot as usize % 3] = duty_to_percent(duty);
        *self.last_fan_duties.lock() = duties;
        let pump_pwm = self.duty_to_pwm(*self.last_pump_duty.lock());
        self.sync_pump_fan(pump_pwm, duties)
    }

    fn set_fan_speeds(&self, duties: &[u8]) -> Result<()> {
        let mut fan_duties = [0u8; 3];
        for (i, &d) in duties.iter().enumerate().take(3) {
            fan_duties[i] = duty_to_percent(d);
        }
        *self.last_fan_duties.lock() = fan_duties;
        let pump_pwm = self.duty_to_pwm(*self.last_pump_duty.lock());
        self.sync_pump_fan(pump_pwm, fan_duties)
    }

    fn read_fan_rpm(&self) -> Result<Vec<u16>> {
        let params = self.get_h2_params()?;
        Ok(params.fan_rpm.to_vec())
    }

    fn fan_slot_count(&self) -> u8 {
        3
    }

    fn has_pump_control(&self) -> bool {
        true
    }

    fn poll_coolant_temp(&self) -> Option<f32> {
        self.get_h2_params().ok().map(|p| p.coolant_temp as f32)
    }

    fn set_pump_speed(&self, duty: u8) -> Result<()> {
        *self.last_pump_duty.lock() = duty;
        let pump_pwm = self.duty_to_pwm(duty);
        let fans = *self.last_fan_duties.lock();
        self.sync_pump_fan(pump_pwm, fans)
    }
}

impl AioDevice for H2AioController {
    fn read_pump_rpm(&self) -> Result<u16> {
        let params = self.get_h2_params()?;
        Ok(params.pump_rpm)
    }

    fn read_coolant_temp(&self) -> Result<f32> {
        let params = self.get_h2_params()?;
        Ok(params.coolant_temp as f32)
    }
}

impl RgbDevice for H2AioController {
    fn device_name(&self) -> String {
        "HydroShift II LCD RGB Ring".to_string()
    }

    fn supported_modes(&self) -> Vec<RgbMode> {
        vec![RgbMode::Off, RgbMode::Static, RgbMode::Direct]
    }

    fn zone_info(&self) -> Vec<RgbZoneInfo> {
        vec![RgbZoneInfo {
            name: "Ring".to_string(),
            led_count: RING_LED_COUNT as u16,
        }]
    }

    fn supports_direct(&self) -> bool {
        true
    }

    fn set_zone_effect(&self, zone: u8, effect: &RgbEffect) -> Result<()> {
        if zone != 0 {
            anyhow::bail!("H2 RGB: zone {zone} out of range (only zone 0)");
        }
        let color = if effect.mode == RgbMode::Off || effect.disabled {
            [0, 0, 0]
        } else {
            let base = effect.colors.first().copied().unwrap_or([255, 255, 255]);
            scale_brightness(base, effect.brightness)
        };
        let frame = vec![color; RING_LED_COUNT];
        self.send_rgb_frames(&[frame], 100)
    }

    fn set_direct_colors(&self, zone: u8, colors: &[[u8; 3]]) -> Result<()> {
        if zone != 0 {
            anyhow::bail!("H2 RGB: zone {zone} out of range (only zone 0)");
        }
        self.send_rgb_frames(&[colors.to_vec()], 100)
    }
}
