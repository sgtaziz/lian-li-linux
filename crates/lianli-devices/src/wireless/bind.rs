use super::controller::WirelessController;
use super::discovery::poll_and_discover;
use super::{
    WirelessFanType, RF_CHUNKS, RF_CHUNK_SIZE, RF_DATA_SIZE, RF_PWM_CMD, RF_SELECT, USB_CMD_SEND_RF,
};
use anyhow::{bail, Context, Result};
use lianli_transport::usb::USB_TIMEOUT;
use std::thread;
use std::time::{Duration, Instant};
use tracing::info;

impl WirelessController {
    pub fn bind_device(&self, mac: &[u8; 6]) -> Result<()> {
        self.check_bind_allowed(mac)?;
        let master_mac = *self.master_mac.lock();
        let new_rx = self.get_rx_unused();
        self.set_bind_intent(mac, true);
        self.converge_bind_state(mac, &master_mac, new_rx)?;
        self.save_rf_config()
    }

    pub fn unbind_device(&self, mac: &[u8; 6]) -> Result<()> {
        self.check_unbind_allowed(mac)?;
        self.forget_mb_rgb_target(mac);
        self.set_bind_intent(mac, false);
        self.converge_bind_state(mac, &[0u8; 6], 0)?;
        self.save_rf_config()
    }

    fn check_bind_allowed(&self, mac: &[u8; 6]) -> Result<()> {
        let (raw_master, dead, fan_type) = {
            let health = self.device_health.lock();
            let Some(h) = health.get(mac) else {
                return Ok(());
            };
            (h.raw_master, h.dead, h.published.fan_type)
        };

        if dead {
            bail!("device is offline");
        }

        let local = *self.master_mac.lock();
        if raw_master != [0u8; 6] && raw_master != local && self.foreign_master_online(&raw_master)
        {
            bail!(
                "device {:02x?} is bound to another controller that is currently online",
                mac
            );
        }

        let bound: Vec<WirelessFanType> = {
            let health = self.device_health.lock();
            health
                .iter()
                .filter(|(m, h)| **m != *mac && h.bind_intent && !h.dead)
                .map(|(_, h)| h.published.fan_type)
                .collect()
        };

        if bound.len() >= 10 {
            bail!("at most 10 wireless devices can be bound");
        }
        match fan_type {
            WirelessFanType::Strimer(_)
                if bound
                    .iter()
                    .filter(|t| matches!(t, WirelessFanType::Strimer(_)))
                    .count()
                    >= 3 =>
            {
                bail!("at most 3 strimer devices can be bound");
            }
            WirelessFanType::WaterBlock
                if bound
                    .iter()
                    .any(|t| matches!(t, WirelessFanType::WaterBlock)) =>
            {
                bail!("only one HydroShift II LCD-C can be bound");
            }
            WirelessFanType::WaterBlock2
                if bound
                    .iter()
                    .any(|t| matches!(t, WirelessFanType::WaterBlock2)) =>
            {
                bail!("only one HydroShift II LCD-S can be bound");
            }
            _ => {}
        }

        Ok(())
    }

    fn check_unbind_allowed(&self, mac: &[u8; 6]) -> Result<()> {
        let Some(raw_master) = self.observed_master_of(mac) else {
            return Ok(());
        };
        let local = *self.master_mac.lock();
        if raw_master != [0u8; 6] && raw_master != local && self.foreign_master_online(&raw_master)
        {
            bail!(
                "device {:02x?} belongs to another controller that is currently online",
                mac
            );
        }
        Ok(())
    }

    pub(super) fn converge_bind_state(
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
                &self.device_health,
                &self.master_entries,
                &self.mobo_pwm,
                &self.fg_sync,
                &self.master_mac,
            );

            let observed = self
                .device_health
                .lock()
                .get(mac)
                .map(|h| (h.raw_master, h.raw_rx, h.raw_channel));

            let master_ch = *self.master_channel.lock();
            let converged = match observed {
                Some((m, r, ch)) => {
                    &m == target_master_mac && r == target_rx && (target_rx == 0 || ch == master_ch)
                }
                None => target_master_mac == &[0u8; 6],
            };
            if converged {
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!(
                    "bind convergence for {:02x?} timed out after {attempts} attempt(s); observed={:?} ch={}",
                    mac,
                    observed.map(|o| (o.0, o.1)),
                    observed.map(|o| o.2).unwrap_or(0)
                );
            }
        }
    }

    pub(super) fn send_bind_packet(
        &self,
        mac: &[u8; 6],
        target_master_mac: &[u8; 6],
        target_rx: u8,
    ) -> Result<()> {
        let device = self
            .discovered_devices
            .lock()
            .iter()
            .find(|d| d.mac == *mac)
            .cloned()
            .context("device not found in discovery")?;

        let master_ch = *self.master_channel.lock();
        let slot = if target_rx == 0 {
            0
        } else {
            self.next_slot_index(&device)
        };

        let mut rf_data = vec![0u8; RF_DATA_SIZE];
        rf_data[0] = RF_SELECT;
        rf_data[1] = RF_PWM_CMD;
        rf_data[2..8].copy_from_slice(&device.mac);
        rf_data[8..14].copy_from_slice(target_master_mac);
        rf_data[14] = target_rx;
        rf_data[15] = master_ch;
        rf_data[16] = slot;
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
            "{} sent to {} ({}) rx={} ch={} slot={}",
            verb,
            device.mac_str(),
            device.fan_type.display_name(),
            target_rx,
            master_ch,
            slot,
        );
        Ok(())
    }

    /// Find an unused RX endpoint (1-14) for a new device binding.
    fn get_rx_unused(&self) -> u8 {
        let health = self.device_health.lock();
        for rx in 1..14u8 {
            let in_use = health
                .values()
                .any(|h| h.bind_intent && !h.dead && h.raw_rx == rx);
            if !in_use {
                return rx;
            }
        }
        1
    }

    pub(super) fn save_rf_config(&self) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::super::discovery::{DeviceHealth, MasterEntry};
    use super::*;

    fn controller_with(local: [u8; 6], foreign_online: bool) -> WirelessController {
        let c = WirelessController::new();
        *c.master_mac.lock() = local;
        if foreign_online {
            let mut masters = c.master_entries.lock();
            masters.insert(
                [7u8; 6],
                MasterEntry {
                    channel: 8,
                    last_seen: Instant::now(),
                },
            );
        }
        c
    }

    fn seed_device(c: &WirelessController, mac: &[u8; 6], master: [u8; 6], intent: bool) {
        let rec = super::super::discovery::DiscoveredDevice {
            mac: *mac,
            master_mac: master,
            channel: 8,
            rx_type: 1,
            device_type: 0,
            fan_count: 3,
            is_inf_right_attach: false,
            fan_types: [0; 4],
            fan_rpms: [0; 4],
            current_pwm: [0; 4],
            cmd_seq: 0,
            fan_type: WirelessFanType::Slv3Led,
            list_index: 0,
            coolant_temp_c: None,
            effect_index: [0; 4],
            is_sync_mb_light: false,
            is_pwm_line_on: false,
            bind_intent: false,
        };
        let mut health = c.device_health.lock();
        let mut h = DeviceHealth::new(rec);
        h.raw_master = master;
        h.bind_intent = intent;
        health.insert(*mac, h);
    }

    #[test]
    fn bind_refused_while_foreign_master_online() {
        let c = controller_with([9u8; 6], true);
        seed_device(&c, &[1, 2, 3, 4, 5, 6], [7u8; 6], false);
        assert!(c.check_bind_allowed(&[1, 2, 3, 4, 5, 6]).is_err());
    }

    #[test]
    fn bind_allowed_when_foreign_master_offline() {
        let c = controller_with([9u8; 6], false);
        seed_device(&c, &[1, 2, 3, 4, 5, 6], [7u8; 6], false);
        assert!(c.check_bind_allowed(&[1, 2, 3, 4, 5, 6]).is_ok());
    }

    #[test]
    fn bind_allowed_for_own_and_masterless_devices() {
        let c = controller_with([9u8; 6], true);
        seed_device(&c, &[1, 2, 3, 4, 5, 6], [9u8; 6], false);
        seed_device(&c, &[2, 2, 3, 4, 5, 6], [0u8; 6], false);
        assert!(c.check_bind_allowed(&[1, 2, 3, 4, 5, 6]).is_ok());
        assert!(c.check_bind_allowed(&[2, 2, 3, 4, 5, 6]).is_ok());
    }

    #[test]
    fn bind_refused_for_dead_device() {
        let c = controller_with([9u8; 6], false);
        seed_device(&c, &[1, 2, 3, 4, 5, 6], [0u8; 6], false);
        c.device_health
            .lock()
            .get_mut(&[1, 2, 3, 4, 5, 6])
            .unwrap()
            .dead = true;
        assert!(c.check_bind_allowed(&[1, 2, 3, 4, 5, 6]).is_err());
    }

    #[test]
    fn bind_caps_enforced() {
        let c = controller_with([9u8; 6], false);
        for i in 0..10u8 {
            seed_device(&c, &[i, 2, 3, 4, 5, 6], [0u8; 6], true);
        }
        seed_device(&c, &[50, 2, 3, 4, 5, 6], [0u8; 6], false);
        assert!(c.check_bind_allowed(&[50, 2, 3, 4, 5, 6]).is_err());

        let c = controller_with([9u8; 6], false);
        for i in 0..3u8 {
            seed_device(&c, &[i, 2, 3, 4, 5, 6], [0u8; 6], true);
            c.device_health
                .lock()
                .get_mut(&[i, 2, 3, 4, 5, 6])
                .unwrap()
                .published
                .fan_type = WirelessFanType::Strimer(1);
        }
        seed_device(&c, &[50, 2, 3, 4, 5, 6], [0u8; 6], false);
        c.device_health
            .lock()
            .get_mut(&[50, 2, 3, 4, 5, 6])
            .unwrap()
            .published
            .fan_type = WirelessFanType::Strimer(1);
        assert!(c.check_bind_allowed(&[50, 2, 3, 4, 5, 6]).is_err());
    }

    #[test]
    fn unbind_refused_while_foreign_master_online() {
        let c = controller_with([9u8; 6], true);
        seed_device(&c, &[1, 2, 3, 4, 5, 6], [7u8; 6], false);
        assert!(c.check_unbind_allowed(&[1, 2, 3, 4, 5, 6]).is_err());
    }

    #[test]
    fn unbind_allowed_for_own_devices() {
        let c = controller_with([9u8; 6], true);
        seed_device(&c, &[1, 2, 3, 4, 5, 6], [9u8; 6], true);
        assert!(c.check_unbind_allowed(&[1, 2, 3, 4, 5, 6]).is_ok());
    }
}
