//! HydroShift II LCD Circle AIO driver.
//!
//! VID=0x1CBE, PID=0xA021 — 480x480 LCD via WinUSB.
//!
//! Uses the generic WinUSB LCD protocol (DES-CBC encrypted headers).
//! This device also has pump/fan control, but those commands go through
//! the same encrypted command protocol.

use crate::winusb_lcd::WinUsbLcdDevice;
use anyhow::Result;
use lianli_shared::screen::ScreenInfo;
use rusb::{Device, GlobalContext};

pub const VID: u16 = 0x1CBE;
pub const PID: u16 = 0xA021;

/// Open a HydroShift II LCD Circle device.
pub fn open(device: Device<GlobalContext>) -> Result<WinUsbLcdDevice> {
    WinUsbLcdDevice::new(device, ScreenInfo::HYDROSHIFT2, "HydroShift II LCD Circle")
}
