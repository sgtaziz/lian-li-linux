use anyhow::{bail, Context, Result};
use lianli_transport::RusbHid;
use rusb::GlobalContext;
use tracing::{debug, info};

/// V2 dongle HID companion interface (CH340 VID `0x1A86`, PID `0x2107`).
pub const V2_HID_VID: u16 = 0x1A86;
pub const V2_HID_PID: u16 = 0x2107;

/// HID command that returns the wireless group MAC paired with this dongle.
const CMD_GET_HID_NUM: u8 = 0x1C;

/// A MAC address read from a V2 dongle's HID interface, together with the
/// USB topology of the interface for correlation with wired LCD receivers.
#[derive(Debug, Clone)]
pub struct V2HidEntry {
    pub bus: u8,
    pub port_numbers: Vec<u8>,
    pub mac: [u8; 6],
}

/// Enumerate every V2 dongle HID interface (`0x1A86:0x2107`) on the bus,
/// send cmd `0x1C`, and return the MAC + topology of each.
pub fn query_v2_hid_macs() -> Vec<V2HidEntry> {
    let mut results = Vec::new();

    let Ok(devices) = rusb::devices() else {
        return results;
    };

    for device in devices.iter() {
        let Ok(desc) = device.device_descriptor() else {
            continue;
        };
        if desc.vendor_id() != V2_HID_VID || desc.product_id() != V2_HID_PID {
            continue;
        }

        let bus = device.bus_number();
        let port_numbers = device.port_numbers().unwrap_or_default();

        match query_single_mac(device.clone()) {
            Ok(mac) => {
                info!(
                    "V2 HID {}-{}: MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                    bus,
                    format_ports(&port_numbers),
                    mac[0],
                    mac[1],
                    mac[2],
                    mac[3],
                    mac[4],
                    mac[5],
                );
                results.push(V2HidEntry {
                    bus,
                    port_numbers,
                    mac,
                });
            }
            Err(e) => {
                debug!(
                    "V2 HID {}-{} query failed: {e:#}",
                    bus,
                    format_ports(&port_numbers)
                );
            }
        }
    }

    results
}

/// Send cmd `0x1C` and read back the 6-byte MAC.
///
/// Vendor (HidLibrary): `ReportId = 0`, `Data = [0x1C, 0, ...]`. HidLibrary
/// builds the wire buffer as `[0x00, 0x1C, 0, ...]` (Report ID prepended).
/// On read, HidLibrary strips byte 0, then copies `Data[1..7]` as the MAC.
///
/// On Linux there's no HID-class driver stripping — `write_interrupt` /
/// `read_interrupt` exchange raw bytes. We prepend `0x00` to match the
/// vendor's wire format, and auto-detect the MAC offset on read (the
/// response may or may not include the Report ID byte depending on the
/// device's HID descriptor).
fn query_single_mac(device: rusb::Device<GlobalContext>) -> Result<[u8; 6]> {
    let mut hid = RusbHid::open_by_usage(device, None).context("opening V2 HID interface")?;

    let mut cmd = [0u8; 64];
    cmd[0] = 0x00;
    cmd[1] = CMD_GET_HID_NUM;
    hid.write(&cmd).context("sending GetHidNum (0x1C)")?;

    let mut resp = [0u8; 64];
    let n = hid
        .read_timeout(&mut resp, 2000)
        .context("reading V2 HID response")?;

    if n < 7 {
        bail!("V2 HID response too short: {n} bytes");
    }

    // If byte 0 is 0x00 (Report ID echo), the MAC is shifted by one.
    let offset = if resp[0] == 0x00 && n >= 8 { 2 } else { 1 };
    let mut mac = [0u8; 6];
    mac.copy_from_slice(&resp[offset..offset + 6]);

    if mac.iter().all(|&b| b == 0) {
        bail!("V2 HID returned all-zero MAC");
    }

    Ok(mac)
}

fn format_ports(ports: &[u8]) -> String {
    ports
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(".")
}

/// Returns `true` if two USB devices share the same parent hub (i.e. their
/// `port_numbers` match on all but the last element and they're on the same bus).
///
/// This mirrors the vendor's hub-ID matching: the V2 dongle's HID interface
/// and a wired LCD receiver connected to the same dongle hub will share a
/// common parent topology.
pub fn share_parent(a_bus: u8, a_ports: &[u8], b_bus: u8, b_ports: &[u8]) -> bool {
    if a_bus != b_bus || a_ports.is_empty() || b_ports.is_empty() {
        return false;
    }
    if a_ports.last() == Some(&0) || b_ports.last() == Some(&0) {
        return false;
    }
    let a_parent = &a_ports[..a_ports.len() - 1];
    let b_parent = &b_ports[..b_ports.len() - 1];
    a_parent == b_parent
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_parent_same_hub() {
        assert!(share_parent(1, &[2, 3], 1, &[2, 4]));
    }

    #[test]
    fn share_parent_different_hub() {
        assert!(!share_parent(1, &[2, 3], 1, &[5, 4]));
    }

    #[test]
    fn share_parent_different_bus() {
        assert!(!share_parent(1, &[2, 3], 2, &[2, 3]));
    }

    #[test]
    fn share_parent_root_devices() {
        // Two root-level devices on the same bus share the root hub.
        assert!(share_parent(1, &[2], 1, &[3]));
    }

    #[test]
    fn share_parent_empty() {
        assert!(!share_parent(1, &[], 1, &[2]));
    }

    #[test]
    fn share_parent_rejects_port_zero() {
        assert!(!share_parent(1, &[0], 1, &[2]));
        assert!(!share_parent(1, &[2, 0], 1, &[2, 3]));
    }
}
