use super::controller::WirelessController;
use super::{RF_DATA_SIZE, RF_SELECT, RF_SET_RGB};
use anyhow::{Context, Result};
use std::thread;
use std::time::Duration;
use tracing::debug;

impl WirelessController {
    /// Send a single frame of per-LED RGB colors to a wireless device.
    pub fn send_rgb_direct(
        &self,
        mac: &[u8; 6],
        colors: &[[u8; 3]],
        effect_index: &[u8; 4],
        header_repeats: u8,
    ) -> Result<()> {
        let led_num = colors.len() as u8;
        let mut raw_rgb = Vec::with_capacity(colors.len() * 3);
        for color in colors {
            raw_rgb.extend_from_slice(color);
        }
        self.send_rgb_payload(
            mac,
            &raw_rgb,
            led_num,
            1,
            5000,
            effect_index,
            header_repeats,
        )
    }

    /// Send a multi-frame animation. Firmware stores the compressed blob and
    /// loops at `interval_ms`.
    pub fn send_rgb_frames(
        &self,
        mac: &[u8; 6],
        frames: &[Vec<[u8; 3]>],
        interval_ms: u16,
        effect_index: &[u8; 4],
        header_repeats: u8,
    ) -> Result<()> {
        if frames.is_empty() {
            return Ok(());
        }
        let led_num = frames[0].len() as u8;
        let total_frames = frames.len() as u16;

        let mut raw_rgb = Vec::with_capacity(frames.len() * led_num as usize * 3);
        for frame in frames {
            for color in frame {
                raw_rgb.extend_from_slice(color);
            }
        }

        self.send_rgb_payload(
            mac,
            &raw_rgb,
            led_num,
            total_frames,
            interval_ms,
            effect_index,
            header_repeats,
        )
    }

    /// Compress raw RGB data, split into 220-byte chunks, send via RF.
    /// Header packet (index=0) carries metadata and is repeated for reliability.
    fn send_rgb_payload(
        &self,
        mac: &[u8; 6],
        raw_rgb: &[u8],
        led_num: u8,
        total_frames: u16,
        interval_ms: u16,
        effect_index: &[u8; 4],
        header_repeats: u8,
    ) -> Result<()> {
        let device = self
            .discovered_devices
            .lock()
            .iter()
            .find(|d| &d.mac == mac)
            .cloned()
            .context("device not found for RGB send")?;

        let master_mac = *self.master_mac.lock();

        let raw_payload = if device.is_inf_right_attach {
            reverse_per_fan_chunks(raw_rgb, led_num as usize, device.fan_count as usize)
        } else {
            raw_rgb.to_vec()
        };
        let raw_rgb = raw_payload.as_slice();

        let compressed = crate::tinyuz::compress(raw_rgb).context("failed to compress RGB data")?;

        const LZO_RF_VALID_LEN: usize = 220;
        let total_pk_num = (compressed.len() as f64 / LZO_RF_VALID_LEN as f64).ceil() as u8;

        let mut packets_sent: u8 = 0;
        self.tx_recover(|handle| {
            let mut offset: usize = 0;
            let mut index: u8 = 0;
            while offset < compressed.len() || index == 0 {
                let mut rf_data = vec![0u8; RF_DATA_SIZE];

                rf_data[0] = RF_SELECT;
                rf_data[1] = RF_SET_RGB;
                rf_data[2..8].copy_from_slice(&device.mac);
                rf_data[8..14].copy_from_slice(&master_mac);
                rf_data[14..18].copy_from_slice(effect_index);
                rf_data[18] = index;
                rf_data[19] = total_pk_num + 1;

                if index == 0 {
                    let data_len = compressed.len() as u32;
                    rf_data[20] = (data_len >> 24) as u8;
                    rf_data[21] = ((data_len >> 16) & 0xFF) as u8;
                    rf_data[22] = ((data_len >> 8) & 0xFF) as u8;
                    rf_data[23] = (data_len & 0xFF) as u8;
                    rf_data[24] = 0;
                    rf_data[25] = (total_frames >> 8) as u8;
                    rf_data[26] = (total_frames & 0xFF) as u8;
                    rf_data[27] = led_num;
                    rf_data[32] = (interval_ms >> 8) as u8;
                    rf_data[33] = (interval_ms & 0xFF) as u8;

                    let repeats = header_repeats.max(1);
                    let gap_ms = if repeats <= 2 { 2 } else { 20 };
                    for repeat in 0..repeats {
                        self.send_rf_packet(handle, &device, &rf_data)?;
                        if repeat < repeats - 1 {
                            thread::sleep(Duration::from_millis(gap_ms));
                        }
                    }
                } else {
                    let remaining = compressed.len() - offset;
                    let chunk_len = remaining.min(LZO_RF_VALID_LEN);
                    rf_data[20..20 + chunk_len]
                        .copy_from_slice(&compressed[offset..offset + chunk_len]);
                    offset += LZO_RF_VALID_LEN;

                    self.send_rf_packet(handle, &device, &rf_data)?;
                }

                index += 1;
            }
            packets_sent = index;
            Ok(())
        })?;

        self.desired_effects
            .lock()
            .insert(device.mac, *effect_index);

        debug!(
            "Sent RGB to {} ({} frame(s), {} LEDs, {} compressed, {} packets, {}ms interval)",
            device.mac_str(),
            total_frames,
            led_num,
            compressed.len(),
            packets_sent,
            interval_ms
        );
        Ok(())
    }
}

/// Reverse the per-fan chunk order in a flat RGB byte buffer. SL-INF
/// daisy-chains wire right-to-left, so fan 0 in user space must land on
/// the highest fan index on the wire.
fn reverse_per_fan_chunks(raw_rgb: &[u8], leds_per_fan: usize, fan_count: usize) -> Vec<u8> {
    if leds_per_fan == 0 || fan_count <= 1 {
        return raw_rgb.to_vec();
    }
    let bytes_per_fan = leds_per_fan * 3;
    let mut out = Vec::with_capacity(raw_rgb.len());
    let total_chunks = raw_rgb.len().div_ceil(bytes_per_fan);
    for fan_idx in (0..fan_count).rev() {
        let start = fan_idx * bytes_per_fan;
        let end = (start + bytes_per_fan).min(raw_rgb.len());
        if start < end {
            out.extend_from_slice(&raw_rgb[start..end]);
        }
    }
    // Append any trailing bytes beyond the last complete fan slot unchanged.
    let consumed = fan_count.min(total_chunks) * bytes_per_fan;
    if consumed < raw_rgb.len() {
        out.extend_from_slice(&raw_rgb[consumed..]);
    }
    out
}
