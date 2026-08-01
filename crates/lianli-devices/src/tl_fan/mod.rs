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

mod controller;
mod port_rgb;

pub use controller::TlFanController;
pub use port_rgb::TlFanPortDevice;

use crate::registry::{DeviceDriver, OpenContext, OpenedDevice, SharedHid};
use anyhow::Result;
use lianli_shared::device_id::{DeviceFamily, TransportKind};
use std::sync::Arc;

/// Number of LEDs per TL fan.
const LEDS_PER_FAN: u16 = 20;

/// Driver entry point for the TL Fan controller.
pub struct TlFanDriver;

impl DeviceDriver for TlFanDriver {
    fn family(&self) -> DeviceFamily {
        DeviceFamily::TlFan
    }

    fn open(&self, ctx: &OpenContext) -> Result<OpenedDevice> {
        let backend: SharedHid = crate::detect::open_hid_with_reopener(
            ctx.device.clone(),
            ctx.hid_usage_page,
            ctx.vid,
            ctx.pid,
            ctx.bus,
            ctx.device.port_numbers().unwrap_or_default(),
        )?;
        let ctrl = std::sync::Arc::new(TlFanController::new(backend.clone())?);
        let rgb = ctrl
            .port_devices()
            .into_iter()
            .map(|(port, dev)| {
                (
                    format!("port{port}"),
                    Arc::new(dev) as Arc<dyn crate::traits::RgbDevice>,
                )
            })
            .collect();
        Ok(OpenedDevice {
            id: ctx.device_id(),
            family: DeviceFamily::TlFan,
            capabilities: DeviceFamily::TlFan.capabilities(),
            transport_kind: TransportKind::Hid,
            model_name: "UNI FAN TL Controller".to_string(),
            firmware: None,
            fan: Some(Box::new(ctrl)),
            lcd: None,
            rgb,
            aio: None,
            shared_hid: Some(backend),
        })
    }
}

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
