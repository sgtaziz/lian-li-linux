//! ENE 6K77 wired fan controller driver (SL/AL series).
//!
//! VID=0x0CF2, PID=0xA100-0xA106
//!
//! Protocol uses HID Feature Reports with Report ID 0xE0.
//! Each controller has 4 fan groups with independent PWM duty control.
//! RPM is read via feature report 0x50 sub-command 0x00.

mod controller;
mod group_rgb;
mod model;

pub use controller::Ene6k77Controller;
pub use group_rgb::Ene6k77GroupDevice;
pub use model::{Ene6k77Firmware, Ene6k77Model};

use crate::registry::{DeviceDriver, OpenContext, OpenedDevice, SharedHid};
use anyhow::Result;
use lianli_shared::device_id::{DeviceFamily, TransportKind};
use std::sync::Arc;
use std::time::Duration;

const REPORT_ID: u8 = 0xE0;
const CMD_DELAY: Duration = Duration::from_millis(20);

/// Driver entry point for the ENE 6K77 fan hub family.
pub struct Ene6k77Driver;

impl DeviceDriver for Ene6k77Driver {
    fn family(&self) -> DeviceFamily {
        DeviceFamily::Ene6k77
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
        let ctrl = std::sync::Arc::new(Ene6k77Controller::new(backend.clone(), ctx.pid)?);
        let rgb = ctrl
            .group_devices()
            .into_iter()
            .map(|(group, dev)| {
                (
                    format!("group{group}"),
                    Arc::new(dev) as Arc<dyn crate::traits::RgbDevice>,
                )
            })
            .collect();
        let firmware = ctrl.firmware().map(|f| f.to_string());
        let model = ctrl.model().name().to_string();

        Ok(OpenedDevice {
            id: ctx.device_id(),
            family: DeviceFamily::Ene6k77,
            capabilities: DeviceFamily::Ene6k77.capabilities(),
            transport_kind: TransportKind::Hid,
            model_name: model,
            firmware,
            fan: Some(Box::new(ctrl)),
            lcd: None,
            rgb,
            aio: None,
            shared_hid: Some(backend),
        })
    }
}
