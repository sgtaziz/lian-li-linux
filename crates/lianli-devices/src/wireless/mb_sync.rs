use super::controller::WirelessController;
use super::{RF_CHUNKS, RF_CHUNK_SIZE, RF_DATA_SIZE, RF_MB_LIGHT_SYNC, RF_SELECT, USB_CMD_SEND_RF};
use anyhow::{Context, Result};
use lianli_transport::usb::USB_TIMEOUT;
use std::thread;
use std::time::Duration;
use tracing::debug;

impl WirelessController {
    /// Enable/disable motherboard ARGB sync for a wireless-bound device.
    /// When enabled, the device reads from its physical ARGB header instead
    /// of playing host-pushed effects. Sent as a burst of identical RF packets
    /// for reliability (one-shot, no continuous re-stream needed).
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

        let seq = {
            let s = device.cmd_seq.wrapping_add(1);
            if s == 0 { 1 } else { s }
        };

        let mut rf_data = vec![0u8; RF_DATA_SIZE];
        rf_data[0] = RF_SELECT;
        rf_data[1] = RF_MB_LIGHT_SYNC;
        rf_data[2..8].copy_from_slice(&device.mac);
        rf_data[8..14].copy_from_slice(&master_mac);
        rf_data[14] = device.rx_type;
        rf_data[15] = master_ch;
        rf_data[16] = slave_index;
        rf_data[17] = seq;
        rf_data[20] = if enabled { 1 } else { 0 };

        self.tx_recover(|handle| {
            for repeat in 0..10u8 {
                for chunk_idx in 0..RF_CHUNKS as u8 {
                    let mut packet = [0u8; 64];
                    packet[0] = USB_CMD_SEND_RF;
                    packet[1] = chunk_idx;
                    packet[2] = device.channel;
                    packet[3] = device.rx_type;
                    let start = chunk_idx as usize * RF_CHUNK_SIZE;
                    packet[4..].copy_from_slice(&rf_data[start..start + RF_CHUNK_SIZE]);
                    handle.write(&packet, USB_TIMEOUT)?;
                    thread::sleep(Duration::from_millis(1));
                }
                if repeat < 9 {
                    thread::sleep(Duration::from_millis(300));
                }
            }
            Ok(())
        })?;

        debug!(
            "MB RGB sync {}: {}",
            if enabled { "enabled" } else { "disabled" },
            device.mac_str(),
        );
        Ok(())
    }
}
