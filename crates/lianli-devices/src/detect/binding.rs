use super::enumerate::enumerate_devices;
use lianli_shared::device_id::uses_hid;
use lianli_transport::RusbHid;
use rusb::{Device, GlobalContext};
use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;
use tracing::{info, warn};

/// Ensure all known HID devices have hidraw nodes by performing USB resets
/// on devices the kernel failed to bind. Some devices persist malformed HID
/// report descriptors across reboots; a USB port reset restores them.
pub fn ensure_hid_devices_bound() {
    let usb_hid_devices: Vec<(u16, u16, &str)> = match enumerate_devices() {
        Ok(devs) => devs
            .into_iter()
            .filter(|d| uses_hid(d.family))
            .map(|d| (d.vid, d.pid, d.name))
            .collect(),
        Err(_) => return,
    };

    if usb_hid_devices.is_empty() {
        return;
    }

    let bound = enumerate_bound_hid_vids_pids();

    let mut reset_count = 0u32;
    for (vid, pid, name) in &usb_hid_devices {
        if bound.contains(&(*vid, *pid)) {
            continue;
        }
        info!("No hidraw node for {name} ({vid:04x}:{pid:04x}), performing USB reset");
        let dev = rusb::devices().ok().and_then(|devs| {
            devs.iter().find(|d| {
                d.device_descriptor()
                    .map(|desc| desc.vendor_id() == *vid && desc.product_id() == *pid)
                    .unwrap_or(false)
            })
        });
        if let Some(usb_dev) = dev {
            if device_responds_to_descriptor(&usb_dev) {
                nudge_kernel_to_bind_hid(&usb_dev);
                if poll_for_hidraw(*vid, *pid, Duration::from_millis(2000)) {
                    info!("{name}: hidraw appeared after kernel bind, no reset needed");
                    continue;
                }
                info!("{name}: hidraw still missing after wait, falling through to reset");
            }
            match RusbHid::reset_usb_device(&usb_dev) {
                Ok(()) => {
                    info!("USB reset successful for {name}");
                    reset_count += 1;
                }
                Err(e) => {
                    let msg = format!("{e}");
                    if msg.contains("Entity not found") {
                        info!("USB reset successful for {name} (device re-enumerated)");
                        reset_count += 1;
                    } else {
                        warn!("USB reset failed for {name}: {e}");
                    }
                }
            }
        }
    }

    if reset_count > 0 {
        info!("Waiting 3s for {reset_count} device(s) to re-enumerate after USB reset");
        std::thread::sleep(std::time::Duration::from_secs(3));
    }
}

/// Walk `/sys/class/hwmon` and `/sys/class/hidraw` to build the set of
/// (VID, PID) pairs the kernel has already bound to a hidraw or hwmon node.
fn enumerate_bound_hid_vids_pids() -> HashSet<(u16, u16)> {
    let mut out = HashSet::new();
    walk_usb_sysclass(Path::new("/sys/class/hidraw"), &mut out);
    walk_usb_sysclass(Path::new("/sys/class/hwmon"), &mut out);
    out
}

/// Walk a `/sys/class/*` directory and collect VID/PID pairs from each entry's
/// `device/../uevent` or `device/idVendor/idProduct` files.
fn walk_usb_sysclass(root: &Path, out: &mut HashSet<(u16, u16)>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let dev = entry.path().join("device");
        let Ok(resolved) = std::fs::canonicalize(&dev) else {
            continue;
        };
        // Each device symlink target contains a `idVendor` and `idProduct`
        // file (USB devices) — read both and add to the set.
        let vid = read_hex_file(&resolved.join("idVendor"));
        let pid = read_hex_file(&resolved.join("idProduct"));
        if let (Some(v), Some(p)) = (vid, pid) {
            out.insert((v, p));
        }
    }
}

fn read_hex_file(path: &Path) -> Option<u16> {
    let text = std::fs::read_to_string(path).ok()?;
    u16::from_str_radix(text.trim(), 16).ok()
}

fn nudge_kernel_to_bind_hid(device: &Device<GlobalContext>) {
    let Ok(handle) = device.open() else {
        return;
    };
    let Ok(config) = device.active_config_descriptor() else {
        return;
    };
    for iface in config.interfaces() {
        for desc in iface.descriptors() {
            if desc.class_code() == 0x03 {
                let _ = handle.attach_kernel_driver(desc.interface_number());
            }
        }
    }
}

fn poll_for_hidraw(vid: u16, pid: u16, max_wait: Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < max_wait {
        if enumerate_bound_hid_vids_pids().contains(&(vid, pid)) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn device_responds_to_descriptor(device: &Device<GlobalContext>) -> bool {
    let Ok(desc) = device.device_descriptor() else {
        return false;
    };
    let Ok(handle) = device.open() else {
        return false;
    };
    let langs = match handle.read_languages(Duration::from_millis(250)) {
        Ok(l) if !l.is_empty() => l,
        _ => return desc.product_string_index().is_some(),
    };
    if let Some(_idx) = desc.product_string_index() {
        return handle
            .read_product_string(langs[0], &desc, Duration::from_millis(250))
            .is_ok();
    }
    true
}
