//! HydroShift II AIO controller — pump + fan control via WinUSB bulk.
//!
//! Opens a separate USB handle to the same physical device as the LCD driver.
//! Both handles can coexist on Linux (interface claims are reference-counted).
//!
//! Ref: WinUsbH2.cs, H2Controller.cs

use crate::crypto::PacketBuilder;
use crate::traits::{AioDevice, FanDevice};
use anyhow::{Context, Result};
use lianli_shared::fan::duty_to_percent;
use lianli_transport::usb::{RusbBulk, LCD_READ_TIMEOUT, LCD_WRITE_TIMEOUT};
use parking_lot::Mutex;
use rusb::{Device, GlobalContext};
use std::time::Duration;
use tracing::{debug, info};

const PUMP_MIN_RPM: u16 = 1600;
const PUMP_MAX_RPM: u16 = 2500;

/// Telemetry parsed from GetH2Params response.
pub struct H2Params {
    pub cpu_temp: u8,
    pub cpu_load: u8,
    pub gpu_temp: u8,
    pub gpu_load: u8,
    pub pump_rpm: u16,
    pub fan_rpm: [u16; 3],
    pub coolant_temp: u8,
}

/// HydroShift II AIO controller (pump + fan via SyncPumpFan opcode).
pub struct H2AioController {
    transport: Mutex<RusbBulk>,
    builder: Mutex<PacketBuilder>,
    last_fan_duties: Mutex<[u8; 3]>,
    last_pump_duty: Mutex<u8>,
}

impl H2AioController {
    pub fn new(device: Device<GlobalContext>) -> Result<Self> {
        let mut transport =
            RusbBulk::open_device(device).context("opening HydroShift II AIO device")?;
        transport
            .detach_and_configure("HydroShift II AIO")
            .context("configuring HydroShift II AIO device")?;
        info!("HydroShift II AIO controller opened");
        Ok(Self {
            transport: Mutex::new(transport),
            builder: Mutex::new(PacketBuilder::new()),
            last_fan_duties: Mutex::new([50, 50, 50]),
            last_pump_duty: Mutex::new(128),
        })
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
            cpu_temp: buf[8],
            cpu_load: buf[9],
            gpu_temp: buf[10],
            gpu_load: buf[11],
            pump_rpm: u16::from_be_bytes([buf[14], buf[15]]),
            fan_rpm: [
                u16::from_be_bytes([buf[16], buf[17]]),
                u16::from_be_bytes([buf[18], buf[19]]),
                u16::from_be_bytes([buf[20], buf[21]]),
            ],
            coolant_temp: buf[24],
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

    /// Convert pump RPM to raw PWM via the C# piecewise-linear curve.
    fn rpm_to_pwm(rpm: u16) -> u16 {
        let rpm = rpm.clamp(PUMP_MIN_RPM, PUMP_MAX_RPM) as f32;
        let pwm = if rpm < 1720.0 {
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
        };
        pwm.round() as u16
    }

    /// Map duty (0-255) → RPM (1600-2500) → PWM via curve.
    fn duty_to_pwm(duty: u8) -> u16 {
        let pct = (duty as f32 / 255.0).clamp(0.0, 1.0);
        let rpm = PUMP_MIN_RPM as f32 + pct * (PUMP_MAX_RPM - PUMP_MIN_RPM) as f32;
        Self::rpm_to_pwm(rpm.round() as u16)
    }
}

impl FanDevice for H2AioController {
    fn set_fan_speed(&self, slot: u8, duty: u8) -> Result<()> {
        let mut duties = *self.last_fan_duties.lock();
        duties[slot as usize % 3] = duty_to_percent(duty);
        *self.last_fan_duties.lock() = duties;
        let pump_pwm = Self::duty_to_pwm(*self.last_pump_duty.lock());
        self.sync_pump_fan(pump_pwm, duties)
    }

    fn set_fan_speeds(&self, duties: &[u8]) -> Result<()> {
        let mut fan_duties = [0u8; 3];
        for (i, &d) in duties.iter().enumerate().take(3) {
            fan_duties[i] = duty_to_percent(d);
        }
        *self.last_fan_duties.lock() = fan_duties;
        let pump_pwm = Self::duty_to_pwm(*self.last_pump_duty.lock());
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

    fn set_pump_speed(&self, duty: u8) -> Result<()> {
        *self.last_pump_duty.lock() = duty;
        let pump_pwm = Self::duty_to_pwm(duty);
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
