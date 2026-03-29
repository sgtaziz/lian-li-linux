//! Display mode switcher for WinUSB LCD devices (VID=0x1A86).
//!
//! Certain Lian Li LCD devices can operate in two modes:
//! - **LCD mode** (VID=0x1CBE): Full LCD streaming, controlled by the daemon.
//! - **Desktop mode** (VID=0x1A86, CH340): Acts as a secondary display via the OS.
//!
//! The same physical device re-enumerates on USB with a different VID:PID when switched.
//!
//! Desktop-mode PIDs (CH340):
//!   0xAD20 — HydroShift II LCD Circle
//!   0xACD1 — Lancool 207 Digital (rev1)
//!   0xAD11 — Lancool 207 Digital (rev2)
//!   0xACE1 — Universal Screen 8.8" (rev1)
//!   0xAD21 — Universal Screen 8.8" (rev2)

use anyhow::{Context, Result};
use hidapi::HidApi;
use tracing::info;

pub const SWITCHER_VID: u16 = 0x1A86;

/// Magic bytes to switch FROM desktop mode (CH340) TO LCD mode.
/// Device reboots and re-enumerates as VID=0x1CBE.
const SWITCH_TO_LCD: &[u8] = &[0x35, 0x66, 0x33, 0x37, 0x35, 0x39, 0x64, 0x66];

/// Lancool 207 prepends 0x00 to the switch bytes.
const SWITCH_TO_LCD_LANCOOL: &[u8] = &[0x00, 0x35, 0x66, 0x33, 0x37, 0x35, 0x39, 0x64, 0x66];

fn is_lancool_pid(pid: u16) -> bool {
    matches!(pid, 0xACD1 | 0xAD11)
}

/// Switch a device from desktop mode (CH340) to LCD mode.
///
/// Opens the CH340 HID device and sends the mode-switch bytes.
/// The device will reboot and re-enumerate as VID=0x1CBE on USB.
pub fn switch_to_lcd_mode(api: &HidApi, pid: u16) -> Result<()> {
    let device = api
        .open(SWITCHER_VID, pid)
        .context("opening CH340 display-mode device")?;

    let payload = if is_lancool_pid(pid) {
        SWITCH_TO_LCD_LANCOOL
    } else {
        SWITCH_TO_LCD
    };

    device
        .write(payload)
        .context("sending LCD mode switch bytes")?;

    info!(
        "Sent LCD mode switch to {SWITCHER_VID:#06x}:{pid:#06x} — device will reboot"
    );
    Ok(())
}
