//! O11 Vision-M 9.2" LCD kit driver.
//!
//! VID=0x1CBE, PID=0xA092 — 1920x462 LCD via WinUSB.
//!
//! Uses the generic WinUSB LCD protocol (DES-CBC encrypted headers),
//! identical to the Universal Screen 8.8" apart from panel geometry.

use crate::winusb_lcd::WinUsbLcdDevice;
use anyhow::Result;
use lianli_shared::screen::vision92_screen;
use rusb::{Device, GlobalContext};

pub const VID: u16 = 0x1CBE;
pub const PID: u16 = 0xA092;

/// Open an O11 Vision-M 9.2" LCD device.
pub fn open(device: Device<GlobalContext>) -> Result<WinUsbLcdDevice> {
    WinUsbLcdDevice::new(device, vision92_screen(), "O11 Vision-M 9.2\" LCD")
}
