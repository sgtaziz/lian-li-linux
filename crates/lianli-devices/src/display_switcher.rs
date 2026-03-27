//! Display mode switcher (VID=0x1A86).
//!
//! Used to switch certain LCD devices between normal and display mode.
//! Communicates via HID 64-byte reports with a simple magic-byte handshake.
//!
//! Known switcher PIDs:
//!   0x7523 — Generic CH340 (listed in udev rules)
//!   0xACD1 — Lancool 207 Digital display mode
//!   0xACE1 — Universal Screen 8.8" display mode
//!   0xAD20 — HydroShift II LCD Circle display mode

use anyhow::{Context, Result};
use hidapi::{HidApi, HidDevice};
use tracing::{debug, info, warn};

pub const SWITCHER_VID: u16 = 0x1A86;

/// Known display mode switcher PIDs.
pub const SWITCHER_PIDS: &[(u16, &str)] = &[
    (0x7523, "Generic CH340"),
    (0xACD1, "Lancool 207 Display Mode (rev1)"),
    (0xAD11, "Lancool 207 Display Mode (rev2)"),
    (0xACE1, "Universal Screen 8.8\" Display Mode (rev1)"),
    (0xAD21, "Universal Screen 8.8\" Display Mode (rev2)"),
    (0xAD20, "HydroShift II LCD Circle Display Mode"),
];

/// Send the display mode activation command to a switcher device.
///
/// The handshake is a simple magic byte sequence: [0xAA, 0x55, 0x35].
pub fn activate_display_mode(api: &HidApi, pid: u16) -> Result<()> {
    let device = api
        .open(SWITCHER_VID, pid)
        .context("opening display mode switcher")?;

    send_magic(&device)?;
    info!("Display mode activated for switcher {SWITCHER_VID:#06x}:{pid:#06x}");
    Ok(())
}

/// Try to activate display mode on any connected switcher device.
pub fn activate_any(api: &HidApi) -> Result<bool> {
    for &(pid, name) in SWITCHER_PIDS {
        match api.open(SWITCHER_VID, pid) {
            Ok(device) => {
                if let Err(e) = send_magic(&device) {
                    warn!("Failed to activate {name}: {e}");
                    continue;
                }
                info!("Display mode activated via {name} ({SWITCHER_VID:#06x}:{pid:#06x})");
                return Ok(true);
            }
            Err(_) => continue,
        }
    }
    debug!("No display mode switcher found");
    Ok(false)
}

fn send_magic(device: &HidDevice) -> Result<()> {
    let mut report = [0u8; 64];
    report[0] = 0xAA;
    report[1] = 0x55;
    report[2] = 0x35;
    device.write(&report).context("writing magic bytes")?;

    let mut buf = [0u8; 64];
    let _ = device.read_timeout(&mut buf, 200);
    Ok(())
}
