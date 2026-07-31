use super::transport::with_transport_recovery;
use super::{WirelessFanType, RX_IDS, USB_CMD_SEND_RF};
use anyhow::{bail, Context, Result};
use lianli_transport::usb::{RusbBulk, USB_TIMEOUT};
use parking_lot::Mutex;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info};

/// A wireless device discovered via the RX GetDev command.
/// Parsed from the 42-byte device record in the response.
#[derive(Debug, Clone)]
pub struct DiscoveredDevice {
    pub mac: [u8; 6],
    pub master_mac: [u8; 6],
    pub channel: u8,
    pub rx_type: u8,
    pub device_type: u8,
    pub fan_count: u8,
    /// True when the bind group chains right-to-left (SL-INF daisy-chain
    /// with `fan_num >= 10` reported). Per-fan PWM/RGB ordering must be
    /// reversed before sending.
    pub is_inf_right_attach: bool,
    pub fan_types: [u8; 4],
    pub fan_rpms: [u16; 4],
    pub current_pwm: [u8; 4],
    pub cmd_seq: u8,
    pub fan_type: WirelessFanType,
    pub list_index: u8,
    /// Coolant temperature in °C (WaterBlock/WaterBlock2 only, from byte 27)
    pub coolant_temp_c: Option<u8>,
    /// Effect index the device firmware is currently running. Drifts to
    /// device-default if the firmware resets idle; compare against the desired
    /// effect_index to detect that and re-send the RGB packet.
    pub effect_index: [u8; 4],
    /// Status flags decoded from `fans_speed[0]` bitfield.
    pub is_sync_mb_light: bool,
    pub is_pwm_line_on: bool,
}

impl DiscoveredDevice {
    pub fn mac_str(&self) -> String {
        format!(
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            self.mac[0], self.mac[1], self.mac[2], self.mac[3], self.mac[4], self.mac[5],
        )
    }

    pub fn is_aio(&self) -> bool {
        self.fan_type.is_aio()
    }

    pub fn pump_rpm(&self) -> Option<u16> {
        if self.is_aio() {
            Some(self.fan_rpms[3])
        } else {
            None
        }
    }
}

impl fmt::Display for DiscoveredDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mac = self.mac_str();
        if self.fan_type.is_aio() {
            let temp_str = self
                .coolant_temp_c
                .map(|t| format!(", coolant={t}°C"))
                .unwrap_or_default();
            write!(
                f,
                "{} ({:?}, {} fans, pump={}rpm{temp_str}, ch={}, rx={})",
                mac, self.fan_type, self.fan_count, self.fan_rpms[3], self.channel, self.rx_type,
            )
        } else {
            write!(
                f,
                "{} ({:?}, {} fans, ch={}, rx={})",
                mac, self.fan_type, self.fan_count, self.channel, self.rx_type,
            )
        }
    }
}

/// Parse a 42-byte device record from GetDev response.
///
/// Record layout:
/// ```text
/// [0-5]   Device MAC (6 bytes)
/// [6-11]  Master MAC (6 bytes)
/// [12]    RF Channel
/// [13]    RX Type (radio endpoint)
/// [14-17] System time (ms * 0.625)
/// [18]    Device type (0=fan, 65=LC217, 255=master)
/// [19]    Fan count
/// [20-23] Effect index (4 bytes)
/// [24-26] Fan type bytes (3 bytes, per-slot)
/// [27]    Coolant temperature °C (WaterBlock/WaterBlock2 only)
/// [28-35] Fan speeds (4x u16 big-endian RPM)
/// [36-39] Current PWM (4 bytes)
/// [40]    Command sequence number
/// [41]    Validation marker (must be 0x1C = 28)
/// ```
pub(super) fn parse_device_record(data: &[u8], list_index: u8) -> Option<DiscoveredDevice> {
    if data.len() < 42 {
        return None;
    }

    if data[41] != 0x1C {
        debug!(
            "  Device record {list_index}: invalid marker 0x{:02x} (expected 0x1C)",
            data[41]
        );
        return None;
    }

    let device_type = data[18];

    if device_type == 0xFF {
        debug!("  Device record {list_index}: skipping master device");
        return None;
    }

    let mut mac = [0u8; 6];
    mac.copy_from_slice(&data[0..6]);

    let mut master_mac = [0u8; 6];
    master_mac.copy_from_slice(&data[6..12]);

    let channel = data[12];
    let rx_type = data[13];
    // fan_num >= 10 flags SL-INF right-attach (chains right-to-left).
    let raw_fan_count = data[19];
    let (fan_count, is_inf_right_attach) = if raw_fan_count >= 10 {
        (raw_fan_count.saturating_sub(10).min(4), true)
    } else {
        (raw_fan_count.min(4), false)
    };

    let mut fan_types = [0u8; 4];
    fan_types.copy_from_slice(&data[24..28]);

    let status_byte = data[28];
    let is_sync_mb_light = (status_byte & 0x40) != 0;
    let is_pwm_line_on = (status_byte & 0x20) != 0;

    let fan_rpms = [
        u16::from_be_bytes([data[28] & 0x0F, data[29]]),
        u16::from_be_bytes([data[30] & 0x0F, data[31]]),
        u16::from_be_bytes([data[32] & 0x0F, data[33]]),
        u16::from_be_bytes([data[34] & 0x0F, data[35]]),
    ];

    let mut current_pwm = [0u8; 4];
    current_pwm.copy_from_slice(&data[36..40]);

    let cmd_seq = data[40];

    let fan_type = match device_type {
        10 => WirelessFanType::WaterBlock,
        11 => WirelessFanType::WaterBlock2,
        1..=9 => WirelessFanType::Strimer(device_type),
        65 => WirelessFanType::Lc217,
        66 => WirelessFanType::V150,
        88 => WirelessFanType::Led88,
        _ => fan_types
            .iter()
            .find(|&&b| b != 0)
            .map(|&b| WirelessFanType::from_fan_type_byte(b))
            .unwrap_or(WirelessFanType::Unknown),
    };

    let coolant_temp_c = if fan_type.is_aio() && data[27] > 0 {
        Some(data[27])
    } else {
        None
    };

    let mut effect_index = [0u8; 4];
    effect_index.copy_from_slice(&data[20..24]);

    Some(DiscoveredDevice {
        mac,
        master_mac,
        channel,
        rx_type,
        device_type,
        fan_count,
        is_inf_right_attach,
        fan_types,
        fan_rpms,
        current_pwm,
        cmd_seq,
        fan_type,
        list_index,
        coolant_temp_c,
        effect_index,
        is_sync_mb_light,
        is_pwm_line_on,
    })
}

/// Polls the RX device for the current device list.
///
/// Sends GetDev command (0x10, page=1) and parses the response into
/// full 42-byte device records.
pub(super) fn poll_and_discover(
    rx: &Arc<Mutex<RusbBulk>>,
    discovered_devices: &Arc<Mutex<Vec<DiscoveredDevice>>>,
    mobo_pwm: &Arc<AtomicU16>,
    fg_sync: &Arc<AtomicBool>,
    master_mac: &Arc<Mutex<[u8; 6]>>,
) -> Result<()> {
    let mut cmd = vec![0u8; 64];
    cmd[0] = USB_CMD_SEND_RF;
    cmd[1] = 0x01;

    if fg_sync.load(Ordering::Relaxed) {
        let rpm = discovered_devices
            .lock()
            .iter()
            .flat_map(|d| d.fan_rpms.iter())
            .copied()
            .find(|&r| r > 0)
            .unwrap_or(0);
        cmd[2] = (rpm >> 8) as u8;
        cmd[3] = (rpm & 0xFF) as u8;
    }

    with_transport_recovery(rx, &RX_IDS, "RX", |handle| {
        handle.read_flush();
        handle
            .write(&cmd, USB_TIMEOUT)
            .context("sending GetDev command")?;
        Ok(())
    })?;
    let handle = rx.lock();

    let mut response = [0u8; 512];
    match handle.read(&mut response, Duration::from_millis(200)) {
        Ok(len) if len >= 4 => {
            if response[0] != USB_CMD_SEND_RF {
                info!(
                    "GetDev: unexpected response 0x{:02x}, will retry",
                    response[0]
                );
                bail!("GetDev: unexpected response 0x{:02x}", response[0]);
            }

            let device_count = response[1] as usize;

            // Mobo PWM extraction. High bit of byte[2] = unavailable flag.
            // When clear: off_time = byte[2] & 0x7F, on_time = byte[3]
            //   pwm = 255 * on_time / (on_time + off_time)
            let indicator = response[2];
            if indicator >> 7 == 1 {
                mobo_pwm.store(0xFFFF, Ordering::Relaxed);
            } else {
                let off_time = (indicator & 0x7F) as u16;
                let on_time = response[3] as u16;
                let denominator = off_time + on_time;
                if denominator > 0 {
                    let pwm = (255u16 * on_time / denominator).min(255);
                    mobo_pwm.store(pwm, Ordering::Relaxed);
                } else {
                    mobo_pwm.store(0xFFFF, Ordering::Relaxed);
                }
            }

            debug!("GetDev: {device_count} device(s) reported");

            if device_count == 0 || device_count > 12 {
                return Ok(());
            }

            let mut found = Vec::new();
            let mut offset = 4;

            for idx in 0..device_count {
                if offset + 42 > len {
                    debug!("GetDev: response truncated at device {idx}");
                    break;
                }

                if let Some(device) = parse_device_record(&response[offset..offset + 42], idx as u8)
                {
                    debug!(
                        "  [{}] {} type=0x{:02x} fans={} RPM=[{},{},{},{}] PWM=[{},{},{},{}]",
                        idx,
                        device,
                        device.device_type,
                        device.fan_count,
                        device.fan_rpms[0],
                        device.fan_rpms[1],
                        device.fan_rpms[2],
                        device.fan_rpms[3],
                        device.current_pwm[0],
                        device.current_pwm[1],
                        device.current_pwm[2],
                        device.current_pwm[3],
                    );
                    found.push(device);
                }

                offset += 42;
            }

            let mut devices = discovered_devices.lock();
            if !found.is_empty() {
                let old_count = devices.len();
                *devices = found;
                if old_count != devices.len() {
                    let local_mac = *master_mac.lock();
                    let bound = devices.iter().filter(|d| d.master_mac == local_mac).count();
                    let unbound = devices.len() - bound;
                    info!(
                        "Discovered {} wireless device(s) ({bound} bound, {unbound} unbound)",
                        devices.len()
                    );
                    for d in devices.iter().filter(|d| d.master_mac != local_mac) {
                        info!(
                            "  {} ({}) not bound to this dongle",
                            d.mac_str(),
                            d.fan_type.display_name()
                        );
                    }
                }
            }
        }
        Ok(_) => {}
        Err(lianli_transport::TransportError::Usb(rusb::Error::Timeout)) => {}
        Err(err) => {
            debug!("GetDev error: {err}");
        }
    }

    Ok(())
}
