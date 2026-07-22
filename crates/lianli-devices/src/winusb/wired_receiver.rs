//! Wired receiver driver for P28 V2 (0x43A8:0x0105) and TL Flex (0x43A8:0x0101).
//!
//! Both share the same WinUsbLed base protocol: 64-byte unencrypted packets,
//! TX+RX pattern. Key per-PID differences:
//!   - LED count: P28 = 9/fan, TL Flex = 26/fan
//!   - PWM floor: P28 = 8% (zero→1), TL Flex = 11% (zero→5)
//!   - RGB transport: P28 streams via 0x11, TL Flex flash-saves via 0x18+0x19
//!

use crate::traits::{FanDevice, RgbDevice};
use anyhow::{Context, Result};
use lianli_shared::rgb::{RgbEffect, RgbMode, RgbScope, RgbZoneInfo};
use lianli_transport::usb::{RusbBulk, LCD_READ_TIMEOUT, LCD_WRITE_TIMEOUT};
use parking_lot::Mutex;
use rusb::{Device, GlobalContext};
use tracing::{debug, info, warn};

const PACKET_SIZE: usize = 64;

// LEDCmdType opcodes
const CMD_GET_VER: u8 = 0x10;
const CMD_GET_INFO: u8 = 0x12;
const CMD_SET_FANS_PWM: u8 = 0x13;
const CMD_PING: u8 = 0x16;

/// Per-PID parameters.
#[derive(Debug, Clone, Copy)]
pub struct ReceiverParams {
    pub leds_per_fan: u16,
    pub pwm_floor: u8,
    pub pwm_zero: u8,
    pub name: &'static str,
}

impl ReceiverParams {
    fn from_pid(pid: u16) -> Option<Self> {
        match pid {
            0x0105 => Some(Self {
                leds_per_fan: 9,
                pwm_floor: 8,
                pwm_zero: 1,
                name: "P28 V2 Controller",
            }),
            0x0101 => Some(Self {
                leds_per_fan: 26,
                pwm_floor: 11,
                pwm_zero: 5,
                name: "TL Flex Controller",
            }),
            _ => None,
        }
    }
}

/// Status response from GetInfo (0x12).
pub struct ReceiverStatus {
    pub fan_count: u8,
    pub fan_rpm: [u16; 4],
    pub fan_pwm: [u8; 4],
    pub firmware: Option<String>,
}

pub struct WiredReceiverController {
    transport: Mutex<RusbBulk>,
    params: ReceiverParams,
    fan_count: Mutex<u8>,
    #[allow(dead_code)]
    firmware: Mutex<Option<String>>,
}

impl WiredReceiverController {
    pub fn new(device: Device<GlobalContext>, pid: u16) -> Result<Self> {
        let params = ReceiverParams::from_pid(pid)
            .ok_or_else(|| anyhow::anyhow!("unknown wired receiver PID {pid:#06x}"))?;

        let mut transport =
            RusbBulk::open_device(device).context("opening wired receiver device")?;
        transport
            .detach_and_configure(params.name)
            .context("configuring wired receiver device")?;

        info!("{} opened", params.name);

        let ctrl = Self {
            transport: Mutex::new(transport),
            params,
            fan_count: Mutex::new(4),
            firmware: Mutex::new(None),
        };

        // Read initial status
        if let Ok(status) = ctrl.get_info() {
            *ctrl.fan_count.lock() = status.fan_count.max(1).min(4);
            info!("{}: {} fans detected", params.name, status.fan_count);
        }

        Ok(ctrl)
    }

    pub fn params(&self) -> ReceiverParams {
        self.params
    }

    /// Send a command and read the response (TX+RX pattern).
    fn send_and_read(&self, tx: &[u8; PACKET_SIZE]) -> Result<[u8; PACKET_SIZE]> {
        let transport = self.transport.lock();
        transport
            .write(tx, LCD_WRITE_TIMEOUT)
            .context("wired receiver write")?;
        let mut rx = [0u8; PACKET_SIZE];
        transport
            .read(&mut rx, LCD_READ_TIMEOUT)
            .context("wired receiver read")?;
        Ok(rx)
    }

    /// GetInfo (0x12) — status read with fan count, RPM, firmware.
    pub fn get_info(&self) -> Result<ReceiverStatus> {
        let tx = [0u8; PACKET_SIZE];
        let mut tx = tx;
        tx[0] = CMD_GET_INFO;
        let rx = self.send_and_read(&tx)?;

        let fan_count = rx[20].min(4);
        let mut fan_rpm = [0u16; 4];
        for i in 0..4 {
            let off = 29 + i * 2;
            fan_rpm[i] = u16::from_be_bytes([rx[off] & 0x0F, rx[off + 1]]);
        }
        let fan_pwm = [rx[37], rx[38], rx[39], rx[40]];

        Ok(ReceiverStatus {
            fan_count,
            fan_rpm,
            fan_pwm,
            firmware: None,
        })
    }

    /// GetLedVer (0x10) — firmware version string.
    pub fn read_firmware(&self) -> Result<String> {
        let tx = [0u8; PACKET_SIZE];
        let mut tx = tx;
        tx[0] = CMD_GET_VER;
        let rx = self.send_and_read(&tx)?;
        let fw_bytes = &rx[3..19];
        let end = fw_bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(fw_bytes.len());
        let fw = String::from_utf8_lossy(&fw_bytes[..end]).trim().to_string();
        Ok(fw)
    }

    /// SetFansPWM (0x13) with per-device PWM floor.
    pub fn set_fans_pwm(&self, duties: [u8; 4]) -> Result<()> {
        let mut tx = [0u8; PACKET_SIZE];
        tx[0] = CMD_SET_FANS_PWM;
        for i in 0..4 {
            let p = duties[i] as u16;
            // Per-device floor: P28 max(p,8)/zero→1; TL Flex max(p,11)/zero→5
            let mapped = if p == 0 {
                self.params.pwm_zero
            } else {
                ((p.max(self.params.pwm_floor as u16) as f64 / 100.0) * 255.0).round() as u8
            };
            // 0x06 → 0x00 rewrite (external-sync sentinel)
            tx[1 + i] = if mapped == 0x06 { 0 } else { mapped };
        }
        let rx = self.send_and_read(&tx)?;
        if rx[0] != CMD_SET_FANS_PWM || rx[1] != 0 {
            warn!("SetFansPWM unexpected response: [{}, {}]", rx[0], rx[1]);
        }
        debug!("{}: SetFansPWM {:?}", self.params.name, &tx[1..5]);
        Ok(())
    }

    /// Ping (0x16) — keepalive.
    pub fn ping(&self) -> Result<()> {
        let mut tx = [0u8; PACKET_SIZE];
        tx[0] = CMD_PING;
        self.send_and_read(&tx)?;
        Ok(())
    }
}

impl FanDevice for WiredReceiverController {
    fn set_fan_speed(&self, slot: u8, duty: u8) -> Result<()> {
        let mut duties = [0u8; 4];
        duties[slot as usize % 4] = duty;
        self.set_fans_pwm(duties)
    }

    fn set_fan_speeds(&self, duties: &[u8]) -> Result<()> {
        let mut pwm = [0u8; 4];
        for (i, &d) in duties.iter().enumerate().take(4) {
            pwm[i] = lianli_shared::fan::duty_to_percent(d);
        }
        self.set_fans_pwm(pwm)
    }

    fn read_fan_rpm(&self) -> Result<Vec<u16>> {
        let status = self.get_info()?;
        Ok(status.fan_rpm.to_vec())
    }

    fn fan_slot_count(&self) -> u8 {
        *self.fan_count.lock()
    }

    fn stop_pwm(&self) -> u8 {
        // zero duty maps to pwm_zero sentinel, not actual 0
        self.params.pwm_zero
    }
}

impl RgbDevice for WiredReceiverController {
    fn device_name(&self) -> String {
        self.params.name.to_string()
    }

    fn supported_modes(&self) -> Vec<RgbMode> {
        // Lighting effects are host-rendered and streamed/flash-saved.
        // Direct mode lets the daemon push per-LED frames.
        vec![RgbMode::Off, RgbMode::Static, RgbMode::Direct]
    }

    fn zone_info(&self) -> Vec<RgbZoneInfo> {
        let count = *self.fan_count.lock() as u16;
        (0..count)
            .map(|fan| RgbZoneInfo {
                name: format!("Fan {}", fan + 1),
                led_count: self.params.leds_per_fan,
            })
            .collect()
    }

    fn supported_scopes(&self) -> Vec<Vec<RgbScope>> {
        let count = *self.fan_count.lock() as usize;
        vec![vec![]; count]
    }

    fn set_zone_effect(&self, _zone: u8, effect: &RgbEffect) -> Result<()> {
        // For now, only Off and Static are supported at the driver level.
        // Full effect streaming (0x11 for P28, 0x18+0x19 for TL Flex) requires
        // the daemon-side effect renderer + tinyuz compression.
        match effect.mode {
            RgbMode::Off => {
                let duties = [0u8; 4];
                self.set_fans_pwm(duties)?;
            }
            _ => {
                debug!(
                    "{}: RGB mode {:?} not yet supported at driver level (use Direct)",
                    self.params.name, effect.mode
                );
            }
        }
        Ok(())
    }
}

pub struct WiredReceiverDriver;

impl crate::registry::DeviceDriver for WiredReceiverDriver {
    fn family(&self) -> lianli_shared::device_id::DeviceFamily {
        lianli_shared::device_id::DeviceFamily::WiredReceiver
    }

    fn open(
        &self,
        ctx: &crate::registry::OpenContext,
    ) -> anyhow::Result<crate::registry::OpenedDevice> {
        let ctrl = std::sync::Arc::new(WiredReceiverController::new(
            rusb::Device::clone(&ctx.device),
            ctx.pid,
        )?);
        let name = ctrl.params().name.to_string();
        let firmware = ctrl.read_firmware().ok();

        Ok(crate::registry::OpenedDevice {
            id: ctx.device_id(),
            family: lianli_shared::device_id::DeviceFamily::WiredReceiver,
            capabilities: lianli_shared::device_id::DeviceFamily::WiredReceiver.capabilities(),
            transport_kind: lianli_shared::device_id::TransportKind::UsbBulk,
            model_name: name,
            firmware,
            fan: Some(Box::new(std::sync::Arc::clone(&ctrl))),
            lcd: None,
            rgb: vec![(
                String::new(),
                Box::new(ctrl) as Box<dyn crate::traits::RgbDevice>,
            )],
            aio: None,
            shared_hid: None,
        })
    }
}
