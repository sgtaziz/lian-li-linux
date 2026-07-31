use super::controller::WirelessController;
use super::convergence::AckSignal;
use super::{
    RF_217_CLOSE_WIFI, RF_DATA_SIZE, RF_REBOOT_LCD, RF_SELECT, RF_SELECTED_GROUP, RF_SEND_PIC,
};
use anyhow::{bail, Context, Result};
use lianli_transport::usb::USB_TIMEOUT;
use std::time::Duration;
use tracing::debug;

const PIC_CHUNK_SIZE: usize = 220;
const PIC_TERMINATOR: u8 = 0xFF;

impl WirelessController {
    pub fn selected_group(&self, mac: &[u8; 6]) -> Result<()> {
        let device = self
            .device_by_mac(mac)
            .context("device not found for selected group")?;
        let master_mac = *self.master_mac.lock();
        let master_ch = *self.master_channel.lock();
        let target_cmd_seq = self.bump_target_cmd_seq(mac, device.cmd_seq);

        let mut rf_data = vec![0u8; RF_DATA_SIZE];
        rf_data[0] = RF_SELECT;
        rf_data[1] = RF_SELECTED_GROUP;
        rf_data[2..8].copy_from_slice(&device.mac);
        rf_data[8..14].copy_from_slice(&master_mac);
        rf_data[14] = device.rx_type;
        rf_data[15] = master_ch;
        rf_data[17] = target_cmd_seq;

        self.enqueue_rf_command(
            &device,
            rf_data,
            AckSignal::CmdSeq(target_cmd_seq),
            "selected group".to_string(),
        );

        debug!("Selected group: {}", device.mac_str());
        Ok(())
    }

    pub fn reboot_lcd_group(&self, mac: &[u8; 6]) -> Result<()> {
        let device = self
            .device_by_mac(mac)
            .context("device not found for LCD reboot")?;
        let master_mac = *self.master_mac.lock();
        let master_ch = *self.master_channel.lock();
        let target_cmd_seq = self.bump_target_cmd_seq(mac, device.cmd_seq);

        let mut rf_data = vec![0u8; RF_DATA_SIZE];
        rf_data[0] = RF_SELECT;
        rf_data[1] = RF_REBOOT_LCD;
        rf_data[2..8].copy_from_slice(&device.mac);
        rf_data[8..14].copy_from_slice(&master_mac);
        rf_data[14] = device.rx_type;
        rf_data[15] = master_ch;
        rf_data[17] = target_cmd_seq;

        self.enqueue_rf_command(
            &device,
            rf_data,
            AckSignal::CmdSeq(target_cmd_seq),
            "LCD reboot".to_string(),
        );

        debug!("LCD reboot: {}", device.mac_str());
        Ok(())
    }

    pub fn close_217_wifi(&self, mac: &[u8; 6], disable: bool) -> Result<()> {
        let device = self
            .device_by_mac(mac)
            .context("device not found for 217 wifi close")?;
        let master_mac = *self.master_mac.lock();
        let master_ch = *self.master_channel.lock();
        let target_cmd_seq = self.bump_target_cmd_seq(mac, device.cmd_seq);

        let mut rf_data = vec![0u8; RF_DATA_SIZE];
        rf_data[0] = RF_SELECT;
        rf_data[1] = RF_217_CLOSE_WIFI;
        rf_data[2..8].copy_from_slice(&device.mac);
        rf_data[8..14].copy_from_slice(&master_mac);
        rf_data[14] = device.rx_type;
        rf_data[15] = master_ch;
        rf_data[17] = target_cmd_seq;
        rf_data[20] = if disable { 1 } else { 0 };

        self.enqueue_rf_command(
            &device,
            rf_data,
            AckSignal::CmdSeq(target_cmd_seq),
            format!("217 wifi {}", if disable { "disable" } else { "enable" }),
        );

        debug!(
            "217 wifi {}: {}",
            if disable { "disabled" } else { "enabled" },
            device.mac_str()
        );
        Ok(())
    }

    pub fn send_aio_pic(&self, mac: &[u8; 6], image: &[u8]) -> Result<()> {
        let device = self
            .device_by_mac(mac)
            .context("device not found for SendPic")?;
        let master_mac = *self.master_mac.lock();
        let master_ch = *self.master_channel.lock();
        let tx = self
            .tx
            .as_ref()
            .context("TX device not available for SendPic")?;

        let total_chunks = image.len().div_ceil(PIC_CHUNK_SIZE);
        if total_chunks > 254 {
            bail!("image too large: {total_chunks} chunks (max 254)");
        }

        for chunk_idx in 0..total_chunks as u8 {
            let start = chunk_idx as usize * PIC_CHUNK_SIZE;
            let end = (start + PIC_CHUNK_SIZE).min(image.len());
            let next_seq = self.send_pic_chunk(
                tx,
                &device.mac,
                &master_mac,
                master_ch,
                device.rx_type,
                chunk_idx,
                &image[start..end],
            )?;
            self.wait_pic_ack(&device.mac, next_seq);
        }

        let len = image.len() as u16;
        let mut term_payload = [0u8; PIC_CHUNK_SIZE];
        term_payload[0] = (len >> 8) as u8;
        term_payload[1] = (len & 0xFF) as u8;
        let next_seq = self.send_pic_chunk(
            tx,
            &device.mac,
            &master_mac,
            master_ch,
            device.rx_type,
            PIC_TERMINATOR,
            &term_payload,
        )?;
        self.wait_pic_ack(&device.mac, next_seq);

        debug!(
            "SendPic: {} ({} chunks + terminator, {} bytes)",
            device.mac_str(),
            total_chunks,
            image.len(),
        );
        Ok(())
    }

    fn send_pic_chunk(
        &self,
        tx: &std::sync::Arc<parking_lot::Mutex<lianli_transport::usb::RusbBulk>>,
        mac: &[u8; 6],
        master_mac: &[u8; 6],
        channel: u8,
        rx_type: u8,
        chunk_idx: u8,
        data: &[u8],
    ) -> Result<u8> {
        let current_seq = self.device_by_mac(mac).map(|d| d.cmd_seq).unwrap_or(0);
        let next_seq = current_seq.wrapping_add(1).max(1);

        let mut rf_data = vec![0u8; RF_DATA_SIZE];
        rf_data[0] = RF_SELECT;
        rf_data[1] = RF_SEND_PIC;
        rf_data[2..8].copy_from_slice(mac);
        rf_data[8..14].copy_from_slice(master_mac);
        rf_data[14] = rx_type;
        rf_data[15] = channel;
        rf_data[17] = next_seq;
        rf_data[18] = chunk_idx;
        let copy_len = data.len().min(PIC_CHUNK_SIZE);
        rf_data[19..19 + copy_len].copy_from_slice(&data[..copy_len]);

        let chunks = rf_data.len() / 60;
        let handle = tx.lock();
        for i in 0..chunks {
            let mut packet = [0u8; 64];
            packet[0] = super::USB_CMD_SEND_RF;
            packet[1] = i as u8;
            packet[2] = channel;
            packet[3] = 0xFF;
            let start = i * 60;
            packet[4..64].copy_from_slice(&rf_data[start..start + 60]);
            handle.write(&packet, USB_TIMEOUT)?;
            std::thread::sleep(Duration::from_millis(2));
        }
        drop(handle);
        Ok(next_seq)
    }

    fn wait_pic_ack(&self, mac: &[u8; 6], expected_seq: u8) {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if let Some(d) = self.device_by_mac(mac) {
                if d.cmd_seq == expected_seq {
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
