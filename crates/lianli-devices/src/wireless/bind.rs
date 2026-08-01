use super::controller::WirelessController;
use super::discovery::poll_and_discover;
use super::{RF_CHUNKS, RF_CHUNK_SIZE, RF_DATA_SIZE, RF_PWM_CMD, RF_SELECT, USB_CMD_SEND_RF};
use anyhow::{bail, Context, Result};
use lianli_transport::usb::USB_TIMEOUT;
use std::thread;
use std::time::{Duration, Instant};
use tracing::info;

impl WirelessController {
    pub fn bind_device(&self, mac: &[u8; 6]) -> Result<()> {
        let master_mac = *self.master_mac.lock();
        let new_rx = self.get_rx_unused();
        self.converge_bind_state(mac, &master_mac, new_rx)?;
        self.save_rf_config()
    }

    pub fn unbind_device(&self, mac: &[u8; 6]) -> Result<()> {
        self.converge_bind_state(mac, &[0u8; 6], 0)?;
        self.save_rf_config()
    }

    fn converge_bind_state(
        &self,
        mac: &[u8; 6],
        target_master_mac: &[u8; 6],
        target_rx: u8,
    ) -> Result<()> {
        const CONVERGE_TIMEOUT: Duration = Duration::from_secs(5);
        const POLL_GAP: Duration = Duration::from_millis(150);

        let rx = self.rx.as_ref().context("RX not connected")?;
        let deadline = Instant::now() + CONVERGE_TIMEOUT;
        let mut attempts = 0u32;
        loop {
            self.send_bind_packet(mac, target_master_mac, target_rx)?;
            attempts += 1;
            thread::sleep(POLL_GAP);

            let _ = poll_and_discover(
                rx,
                &self.discovered_devices,
                &self.mobo_pwm,
                &self.fg_sync,
                &self.master_mac,
            );

            let observed = self
                .discovered_devices
                .lock()
                .iter()
                .find(|d| &d.mac == mac)
                .map(|d| (d.master_mac, d.rx_type));

            let converged = match observed {
                Some((m, r)) => &m == target_master_mac && r == target_rx,
                None => target_master_mac == &[0u8; 6],
            };
            if converged {
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!(
                    "bind convergence for {:02x?} timed out after {attempts} attempt(s); observed={:?}",
                    mac,
                    observed
                );
            }
        }
    }

    fn send_bind_packet(
        &self,
        mac: &[u8; 6],
        target_master_mac: &[u8; 6],
        target_rx: u8,
    ) -> Result<()> {
        let device = self
            .discovered_devices
            .lock()
            .iter()
            .find(|d| &d.mac == mac)
            .cloned()
            .context("device not found in discovery")?;

        let master_ch = *self.master_channel.lock();

        let mut rf_data = vec![0u8; RF_DATA_SIZE];
        rf_data[0] = RF_SELECT;
        rf_data[1] = RF_PWM_CMD;
        rf_data[2..8].copy_from_slice(&device.mac);
        rf_data[8..14].copy_from_slice(target_master_mac);
        rf_data[14] = target_rx;
        rf_data[15] = master_ch;
        rf_data[16] = target_rx;
        rf_data[17..21].copy_from_slice(&device.current_pwm);

        self.tx_recover(|handle| {
            for _ in 0..6 {
                self.send_rf_packet(handle, &device, &rf_data)?;
                thread::sleep(Duration::from_millis(30));
            }
            Ok(())
        })?;

        let verb = if target_rx == 0 { "Unbind" } else { "Bind" };
        info!(
            "{} sent to {} ({}) rx={} ch={}",
            verb,
            device.mac_str(),
            device.fan_type.display_name(),
            target_rx,
            master_ch,
        );
        Ok(())
    }

    /// Find an unused RX endpoint (1-14) for a new device binding.
    fn get_rx_unused(&self) -> u8 {
        let devices = self.discovered_devices.lock();
        let local_mac = *self.master_mac.lock();
        for rx in 1..14u8 {
            let in_use = devices
                .iter()
                .any(|d| d.master_mac == local_mac && d.rx_type == rx);
            if !in_use {
                return rx;
            }
        }
        1
    }

    /// Broadcast SaveConfig to persist device bindings to flash.
    fn save_rf_config(&self) -> Result<()> {
        let master_mac = *self.master_mac.lock();
        let master_ch = *self.master_channel.lock();

        let mut rf_data = vec![0u8; RF_DATA_SIZE];
        rf_data[0] = RF_SELECT;
        rf_data[1] = 0x15; // SaveConfig
        rf_data[2..8].copy_from_slice(&[0xFF; 6]);
        rf_data[8..14].copy_from_slice(&master_mac);
        rf_data[14] = 0xFF;

        self.tx_recover(|handle| {
            for _ in 0..3 {
                for chunk_idx in 0..RF_CHUNKS as u8 {
                    let mut packet = vec![0u8; 64];
                    packet[0] = USB_CMD_SEND_RF;
                    packet[1] = chunk_idx;
                    packet[2] = master_ch;
                    packet[3] = 0xFF;
                    let start = chunk_idx as usize * RF_CHUNK_SIZE;
                    packet[4..64].copy_from_slice(&rf_data[start..start + RF_CHUNK_SIZE]);
                    handle
                        .write(&packet, USB_TIMEOUT)
                        .context("sending SaveConfig")?;
                    thread::sleep(Duration::from_millis(1));
                }
                thread::sleep(Duration::from_millis(200));
            }
            Ok(())
        })
    }
}
