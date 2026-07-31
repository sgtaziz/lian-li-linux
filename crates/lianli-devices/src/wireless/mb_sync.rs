use super::controller::WirelessController;
use super::convergence::AckSignal;
use super::{RF_DATA_SIZE, RF_MB_LIGHT_SYNC, RF_PWM_CMD, RF_SELECT};
use anyhow::{Context, Result};
use tracing::debug;

impl WirelessController {
    pub fn set_mb_rgb_sync(&self, mac: &[u8; 6], enabled: bool) -> Result<()> {
        let devices = self.discovered_devices.lock();
        let master_mac = *self.master_mac.lock();
        let master_ch = *self.master_channel.lock();

        let device = devices
            .iter()
            .find(|d| &d.mac == mac)
            .cloned()
            .context("device not found for MB RGB sync")?;

        let slave_index = devices
            .iter()
            .filter(|d| d.master_mac == master_mac && d.device_type != 0xFF)
            .position(|d| d.mac == *mac)
            .map(|i| i as u8)
            .unwrap_or(0);
        drop(devices);

        let target_cmd_seq = self.bump_target_cmd_seq(mac, device.cmd_seq);

        let mut rf_data = vec![0u8; RF_DATA_SIZE];
        rf_data[0] = RF_SELECT;
        rf_data[1] = RF_MB_LIGHT_SYNC;
        rf_data[2..8].copy_from_slice(&device.mac);
        rf_data[8..14].copy_from_slice(&master_mac);
        rf_data[14] = device.rx_type;
        rf_data[15] = master_ch;
        rf_data[16] = slave_index;
        rf_data[17] = target_cmd_seq;
        rf_data[20] = if enabled { 1 } else { 0 };

        self.enqueue_rf_command(
            &device,
            rf_data,
            AckSignal::CmdSeq(target_cmd_seq),
            format!("MB RGB sync {}", if enabled { "on" } else { "off" }),
        );

        debug!(
            "MB RGB sync {}: {} (target_cmd_seq={})",
            if enabled { "enabled" } else { "disabled" },
            device.mac_str(),
            target_cmd_seq,
        );
        Ok(())
    }

    pub fn set_mb_pwm_sync(&self, enabled: bool) -> Result<()> {
        let devices = self.discovered_devices.lock();
        let master_mac = *self.master_mac.lock();
        let master_ch = *self.master_channel.lock();

        let e1_devices: Vec<_> = devices
            .iter()
            .filter(|d| d.master_mac == master_mac && d.mac[5] == 0xE1)
            .cloned()
            .collect();
        drop(devices);

        if e1_devices.is_empty() {
            return Ok(());
        }

        for device in &e1_devices {
            let target_cmd_seq = self.bump_target_cmd_seq(&device.mac, device.cmd_seq);

            let mut rf_data = vec![0u8; RF_DATA_SIZE];
            rf_data[0] = RF_SELECT;
            rf_data[1] = RF_PWM_CMD;
            rf_data[2..8].copy_from_slice(&device.mac);
            rf_data[8..14].copy_from_slice(&master_mac);
            rf_data[14] = device.rx_type;
            rf_data[15] = master_ch;
            rf_data[17] = target_cmd_seq;

            if enabled {
                rf_data[20..24].copy_from_slice(&[6, 6, 6, 6]);
            } else {
                rf_data[20..24].copy_from_slice(&device.current_pwm);
            }

            self.enqueue_rf_command(
                device,
                rf_data,
                AckSignal::Pwm(if enabled { [6; 4] } else { device.current_pwm }),
                format!("MB PWM sync {}", if enabled { "on" } else { "off" }),
            );
        }

        debug!(
            "MB PWM sync {}: {} companion device(s)",
            if enabled { "enabled" } else { "disabled" },
            e1_devices.len(),
        );
        Ok(())
    }

    pub fn has_mb_pwm_companion(&self) -> bool {
        let master_mac = *self.master_mac.lock();
        self.discovered_devices
            .lock()
            .iter()
            .any(|d| d.master_mac == master_mac && d.mac[5] == 0xE1)
    }
}
