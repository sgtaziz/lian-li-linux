use super::controller::WirelessController;
use super::discovery::DiscoveredDevice;
use super::fan_type::WirelessFanType;
use super::{RF_CHUNKS, RF_CHUNK_SIZE, RF_DATA_SIZE, RF_PWM_CMD, RF_SELECT, USB_CMD_SEND_RF};
use anyhow::{Context, Result};
use lianli_transport::usb::USB_TIMEOUT;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};
use tracing::debug;

// Wireless fans can revert to hardware-default speed if PWM traffic goes quiet.
// Keep sending steady-state targets periodically even when the reported PWM
// already matches the requested value.
const PWM_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(1);

fn pwm_last_sent() -> &'static Mutex<HashMap<[u8; 6], Instant>> {
    static LAST_SENT: OnceLock<Mutex<HashMap<[u8; 6], Instant>>> = OnceLock::new();
    LAST_SENT.get_or_init(|| Mutex::new(HashMap::new()))
}

impl WirelessController {
    /// Set fan PWM values for a specific device identified by MAC address.
    /// Uses the device's own rx_type and channel from discovery.
    ///
    /// RF PWM packet layout (240 bytes):
    /// ```text
    /// [0]     = 0x12 (RF_Select — envelope command)
    /// [1]     = 0x10 (RF_Bind — PWM sub-command)
    /// [2-7]   = Device (slave) MAC address
    /// [8-13]  = Master MAC address
    /// [14]    = Target RX type (from device discovery)
    /// [15]    = Target channel (master channel)
    /// [16]    = Sequence index (1 for one-shot commands)
    /// [17-20] = Fan PWM values (4 bytes, one per fan slot)
    /// [21-239]= Reserved
    /// ```
    pub fn set_fan_speeds_by_mac(&self, mac: &[u8; 6], fan_pwm: &[u8; 4]) -> Result<()> {
        let devices = self.discovered_devices.lock();
        let master_mac = *self.master_mac.lock();
        let master_ch = *self.master_channel.lock();

        let device = devices
            .iter()
            .find(|d| &d.mac == mac)
            .cloned()
            .context(format!(
                "Device MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} not found in discovery",
                mac[0], mac[1], mac[2], mac[3], mac[4], mac[5],
            ))?;

        let seq_index = devices
            .iter()
            .filter(|d| d.master_mac == master_mac && d.device_type != 0xFF)
            .position(|d| d.mac == *mac)
            .map(|i| (i + 1) as u8)
            .unwrap_or(1);

        drop(devices);

        let mut pwm = *fan_pwm;
        apply_pwm_constraints(&mut pwm, &device);

        let needs_send = pwm
            .iter()
            .zip(device.current_pwm.iter())
            .any(|(target, reported)| {
                target.abs_diff(*reported) > 5 || (*target <= 10 && *reported != *target)
            });
        let now = Instant::now();
        let force_keepalive = pwm_last_sent()
            .lock()
            .get(mac)
            .map_or(true, |t| now.duration_since(*t) >= PWM_KEEPALIVE_INTERVAL);
        if !needs_send && !force_keepalive {
            return Ok(());
        }

        let mut rf_data = vec![0u8; RF_DATA_SIZE];
        rf_data[0] = RF_SELECT;
        rf_data[1] = RF_PWM_CMD;
        rf_data[2..8].copy_from_slice(&device.mac);
        rf_data[8..14].copy_from_slice(&master_mac);
        rf_data[14] = device.rx_type;
        rf_data[15] = master_ch;
        rf_data[16] = seq_index;
        rf_data[17..21].copy_from_slice(&pwm);

        self.tx_recover(|handle| {
            for chunk_idx in 0..RF_CHUNKS as u8 {
                let mut packet = vec![0u8; 64];
                packet[0] = USB_CMD_SEND_RF;
                packet[1] = chunk_idx;
                packet[2] = device.channel;
                packet[3] = device.rx_type;

                let start = chunk_idx as usize * RF_CHUNK_SIZE;
                let end = start + RF_CHUNK_SIZE;
                packet[4..64].copy_from_slice(&rf_data[start..end]);

                handle
                    .write(&packet, USB_TIMEOUT)
                    .context("sending fan speed RF packet")?;
                thread::sleep(Duration::from_millis(1));
            }
            Ok(())
        })?;

        pwm_last_sent().lock().insert(*mac, Instant::now());

        debug!(
            "Set fan PWM for {} (rx={}, ch={}): {:?}",
            device.mac_str(),
            device.rx_type,
            device.channel,
            pwm
        );
        Ok(())
    }

    /// Set fan PWM values by device list index (backward compat with old API).
    pub fn set_fan_speeds(&self, device_index: u8, fan_pwm: &[u8; 4]) -> Result<()> {
        let mac = {
            let devices = self.discovered_devices.lock();
            devices
                .iter()
                .find(|d| d.list_index == device_index)
                .map(|d| d.mac)
                .context(format!(
                    "No device at index {device_index} (discovered {} device(s))",
                    devices.len()
                ))?
        };

        self.set_fan_speeds_by_mac(&mac, fan_pwm)
    }
}

/// Apply minimum duty enforcement and CLV1 PWM filter (values 153-155 → 152/156).
fn apply_pwm_constraints(pwm: &mut [u8; 4], device: &DiscoveredDevice) {
    let min_pwm = ((device.fan_type.min_duty_percent() as f32 / 100.0) * 255.0) as u8;

    for (i, val) in pwm.iter_mut().enumerate() {
        let is_pump_slot = i == 3 && device.fan_type.is_aio();
        if i as u8 >= device.fan_count && !is_pump_slot {
            *val = 0;
            continue;
        }

        if *val > 0 && *val < min_pwm {
            *val = min_pwm;
        }

        if device.fan_type == WirelessFanType::Clv1 {
            match *val {
                153 | 154 => *val = 152,
                155 => *val = 156,
                _ => {}
            }
        }
    }
}
