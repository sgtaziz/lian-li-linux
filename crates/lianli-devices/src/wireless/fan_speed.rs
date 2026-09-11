use super::controller::WirelessController;
use super::convergence::AckSignal;
use super::discovery::DiscoveredDevice;
use super::fan_type::WirelessFanType;
use super::{RF_DATA_SIZE, RF_PWM_CMD, RF_SELECT};
use anyhow::{Context, Result};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Instant;
use tracing::debug;

// Wireless fans can revert to hardware-default speed if PWM traffic goes quiet.
// Keep sending steady-state targets periodically even when the reported PWM
// already matches the requested value.
const PWM_KEEPALIVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

fn pwm_last_sent() -> &'static Mutex<HashMap<[u8; 6], Instant>> {
    static LAST_SENT: OnceLock<Mutex<HashMap<[u8; 6], Instant>>> = OnceLock::new();
    LAST_SENT.get_or_init(|| Mutex::new(HashMap::new()))
}

impl WirelessController {
    pub fn set_fan_speeds_by_mac(&self, mac: &[u8; 6], fan_pwm: &[u8; 4]) -> Result<()> {
        self.send_fan_pwm(mac, Some(fan_pwm))
    }

    pub fn set_hardware_pwm_sync(&self, mac: &[u8; 6]) -> Result<()> {
        self.send_fan_pwm(mac, None)
    }

    fn send_fan_pwm(&self, mac: &[u8; 6], fan_pwm: Option<&[u8; 4]>) -> Result<()> {
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

        let slot_index = devices
            .iter()
            .filter(|d| (d.bind_intent || d.master_mac == master_mac) && d.device_type != 0xFF)
            .position(|d| d.mac == *mac)
            .map(|i| (i + 1) as u8)
            .unwrap_or(1);

        drop(devices);

        let pwm = prepare_pwm(fan_pwm, &device)?;

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
            .is_none_or(|t| now.duration_since(*t) >= PWM_KEEPALIVE_INTERVAL);
        if !needs_send && !force_keepalive {
            return Ok(());
        }

        let rf_data = build_pwm_packet(&device, &master_mac, master_ch, slot_index, pwm);

        self.enqueue_rf_command(&device, rf_data, AckSignal::Pwm(pwm), "fan PWM")?;

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

        if matches!(
            device.fan_type,
            WirelessFanType::Clv1 | WirelessFanType::ClV2 { .. }
        ) {
            match *val {
                153 | 154 => *val = 152,
                155 => *val = 156,
                _ => {}
            }
        }
    }
}

/// Reverse per-fan slot ordering for SL-INF right-attach daisy-chains.
/// Slot 0 (leftmost in user space) becomes the rightmost slot on the wire.
fn reverse_fan_order<T: Copy>(slots: &mut [T; 4], fan_count: usize) {
    let n = fan_count.min(4);
    if n > 1 {
        slots[..n].reverse();
    }
}

fn build_pwm_packet(
    device: &DiscoveredDevice,
    master_mac: &[u8; 6],
    channel: u8,
    slot: u8,
    pwm: [u8; 4],
) -> Vec<u8> {
    let mut data = vec![0; RF_DATA_SIZE];
    data[0] = RF_SELECT;
    data[1] = RF_PWM_CMD;
    data[2..8].copy_from_slice(&device.mac);
    data[8..14].copy_from_slice(master_mac);
    data[14] = device.rx_type;
    data[15] = channel;
    data[16] = slot;
    data[17..21].copy_from_slice(&pwm);
    data
}

fn prepare_pwm(fan_pwm: Option<&[u8; 4]>, device: &DiscoveredDevice) -> Result<[u8; 4]> {
    let Some(fan_pwm) = fan_pwm else {
        anyhow::ensure!(
            device.fan_type.supports_hw_mobo_sync(),
            "device does not support hardware PWM sync"
        );
        return Ok([6; 4]);
    };
    let mut pwm = *fan_pwm;
    apply_pwm_constraints(&mut pwm, device);
    if device.is_inf_right_attach {
        reverse_fan_order(&mut pwm, device.fan_count as usize);
    }
    Ok(pwm)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(fan_type: WirelessFanType) -> DiscoveredDevice {
        let mut record = [0; 42];
        record[..6].copy_from_slice(&[1, 2, 3, 4, 5, 6]);
        record[13] = 2;
        record[19] = 3;
        record[41] = 0x1c;
        let mut device = super::super::discovery::parse_device_record(&record, 0).unwrap();
        device.fan_type = fan_type;
        device
    }

    #[test]
    fn hardware_sync_packet_preserves_all_four_sentinels() {
        for family in [WirelessFanType::Slv3Led, WirelessFanType::Slv3Lcd] {
            let device = device(family);
            let pwm = prepare_pwm(None, &device).unwrap();
            let packet = build_pwm_packet(&device, &[9; 6], 8, 3, pwm);
            assert_eq!(
                &packet[..21],
                &[0x12, 0x10, 1, 2, 3, 4, 5, 6, 9, 9, 9, 9, 9, 9, 2, 8, 3, 6, 6, 6, 6]
            );
            assert_eq!(packet.len(), 240);
            assert!(packet[21..].iter().all(|&byte| byte == 0));
            assert_eq!(
                prepare_pwm(Some(&[6; 4]), &device).unwrap(),
                [35, 35, 35, 0]
            );
        }
        assert!(prepare_pwm(None, &device(WirelessFanType::SlInf)).is_err());
    }

    #[test]
    fn normal_pwm_retains_stop_floors_filters_and_slot_order() {
        assert_eq!(
            prepare_pwm(Some(&[0, 1, 255, 255]), &device(WirelessFanType::Slv3Led)).unwrap(),
            [0, 35, 255, 0]
        );
        assert_eq!(
            prepare_pwm(Some(&[153, 154, 155, 255]), &device(WirelessFanType::Clv1)).unwrap(),
            [152, 152, 156, 0]
        );
        let mut reversed = device(WirelessFanType::SlInf);
        reversed.is_inf_right_attach = true;
        assert_eq!(
            prepare_pwm(Some(&[100, 150, 200, 255]), &reversed).unwrap(),
            [200, 150, 100, 0]
        );
        assert_eq!(
            prepare_pwm(Some(&[255; 4]), &device(WirelessFanType::WaterBlock)).unwrap(),
            [255; 4]
        );
    }
}
