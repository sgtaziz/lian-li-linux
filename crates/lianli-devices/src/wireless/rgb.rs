use super::controller::WirelessController;
use super::{WirelessFanType, RF_DATA_SIZE, RF_SELECT, RF_SET_RGB};
use anyhow::{ensure, Context, Result};
use lianli_shared::rgb::RgbPlaybackTiming;
use std::thread;
use std::time::{Duration, Instant};

// MasterDevice.LzoMaxRgbDataLen and lzo_rgb_rf_valid_len in the vendor RF uploader.
const MAX_COMPRESSED_BYTES: usize = 12_288;
const CHUNK_BYTES: usize = 220;
const MAX_FRAMES: usize = lianli_shared::rgb::MAX_RGB_ANIMATION_FRAMES;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WirelessRgbUpload {
    compressed: Vec<u8>,
    led_count: u8,
    frame_count: u16,
    interval_ticks: u16,
    interval_fraction: u8,
    effect_index: [u8; 4],
    timing: RgbPlaybackTiming,
    sync_layout: bool,
}

impl WirelessRgbUpload {
    pub fn frame_count(&self) -> u16 {
        self.frame_count
    }

    pub fn effect_index(&self) -> [u8; 4] {
        self.effect_index
    }
    pub fn new(
        frames: &[Vec<[u8; 3]>],
        interval_ms: u16,
        reverse_fan_leds: Option<usize>,
    ) -> Result<Self> {
        ensure!(
            (1..=40_959).contains(&interval_ms),
            "RGB interval must be 1..=40959 ms"
        );
        Self::with_tick_interval(frames, u32::from(interval_ms) * 160, reverse_fan_leds)
    }

    /// Interval in hundredths of a 0.625 ms RF tick, matching the vendor header.
    pub fn with_tick_interval(
        frames: &[Vec<[u8; 3]>],
        interval_hundredths: u32,
        reverse_fan_leds: Option<usize>,
    ) -> Result<Self> {
        Self::with_timing(
            frames,
            RgbPlaybackTiming {
                interval_hundredths,
                ..Default::default()
            },
            reverse_fan_leds,
        )
    }

    pub fn with_timing(
        frames: &[Vec<[u8; 3]>],
        timing: RgbPlaybackTiming,
        reverse_fan_leds: Option<usize>,
    ) -> Result<Self> {
        let interval_hundredths = timing.interval_hundredths;
        ensure!(
            !frames.is_empty() && frames.len() <= MAX_FRAMES,
            "RGB upload requires 1..={MAX_FRAMES} frames"
        );
        let led_count = frames[0].len();
        ensure!(
            (1..=255).contains(&led_count),
            "RGB upload requires 1..=255 LEDs per frame"
        );
        ensure!(
            frames.iter().all(|f| f.len() == led_count),
            "RGB frames must have equal LED counts"
        );
        ensure!(
            (100..=6_553_599).contains(&interval_hundredths),
            "RGB interval must be 1..=65535.99 RF ticks"
        );
        ensure!(
            if timing.secondary_frame_count == 0 {
                timing.secondary_interval_ticks == 0 && !timing.outer_longest
            } else {
                timing.secondary_interval_ticks > 0
                    && usize::from(timing.secondary_frame_count) <= frames.len()
            },
            "invalid secondary RGB region timing"
        );
        if let Some(count) = reverse_fan_leds {
            ensure!(
                count > 0 && led_count.is_multiple_of(count),
                "invalid RGB fan layout"
            );
        }
        let mut raw = Vec::with_capacity(frames.len() * led_count * 3);
        for frame in frames {
            if let Some(count) = reverse_fan_leds {
                for fan in frame.chunks_exact(count).rev() {
                    for color in fan {
                        raw.extend_from_slice(color);
                    }
                }
            } else {
                for color in frame {
                    raw.extend_from_slice(color);
                }
            }
        }
        let compressed = crate::tinyuz::compress(&raw).context("compressing RGB animation")?;
        ensure!(
            compressed.len() <= MAX_COMPRESSED_BYTES,
            "compressed RGB animation exceeds {MAX_COMPRESSED_BYTES} bytes"
        );
        let interval_ticks = (interval_hundredths / 100) as u16;
        let interval_fraction = (interval_hundredths % 100) as u8;
        let frame_count = frames.len() as u16;
        let mut hash = 0x811c_9dc5u32;
        for byte in compressed
            .iter()
            .copied()
            .chain(interval_ticks.to_be_bytes())
            .chain([interval_fraction])
            .chain(frame_count.to_be_bytes())
            .chain([led_count as u8])
            .chain(timing.secondary_interval_ticks.to_be_bytes())
            .chain(timing.secondary_frame_count.to_be_bytes())
            .chain([u8::from(timing.outer_longest)])
        {
            hash = (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193);
        }
        Ok(Self {
            compressed,
            led_count: led_count as u8,
            frame_count,
            interval_ticks,
            interval_fraction,
            effect_index: hash.max(1).to_be_bytes(),
            timing,
            sync_layout: false,
        })
    }

    fn packet(&self, mac: &[u8; 6], master: &[u8; 6], index: usize) -> [u8; RF_DATA_SIZE] {
        let mut packet = [0; RF_DATA_SIZE];
        packet[0] = RF_SELECT;
        packet[1] = RF_SET_RGB;
        packet[2..8].copy_from_slice(mac);
        packet[8..14].copy_from_slice(master);
        packet[14..18].copy_from_slice(&self.effect_index);
        packet[18] = index as u8;
        packet[19] = (self.compressed.len().div_ceil(CHUNK_BYTES) + 1) as u8;
        if index == 0 {
            packet[20..24].copy_from_slice(&(self.compressed.len() as u32).to_be_bytes());
            packet[25..27].copy_from_slice(&self.frame_count.to_be_bytes());
            packet[27] = self.led_count;
            packet[32..34].copy_from_slice(&self.interval_ticks.to_be_bytes());
            packet[34] = self.interval_fraction;
            packet[35..37].copy_from_slice(&self.timing.secondary_interval_ticks.to_be_bytes());
            packet[37] = u8::from(self.timing.outer_longest);
            packet[38..40].copy_from_slice(&self.timing.secondary_frame_count.to_be_bytes());
        } else {
            let offset = (index - 1) * CHUNK_BYTES;
            let data = &self.compressed[offset..self.compressed.len().min(offset + CHUNK_BYTES)];
            packet[20..20 + data.len()].copy_from_slice(data);
        }
        packet
    }
}

impl WirelessController {
    pub fn prepare_rgb_sync_animation(
        &self,
        mac: &[u8; 6],
        frames: &[Vec<[u8; 3]>],
        timing: RgbPlaybackTiming,
    ) -> Result<WirelessRgbUpload> {
        let device = self.device_by_mac_snapshot(mac)?;
        let profile = device
            .fan_type
            .rgb_render_profile(device.fan_count)
            .context("unsupported RGB sync layout")?;
        ensure!(
            frames
                .iter()
                .all(|frame| frame.len() == usize::from(profile.sync_frame_led_count())),
            "RGB sync frame does not match device layout"
        );
        ensure!(
            !matches!(device.fan_type, WirelessFanType::Led88),
            "screen RGB sync requires its USB lighting interface"
        );
        let mut upload = WirelessRgbUpload::with_timing(frames, timing, None)?;
        upload.sync_layout = true;
        Ok(upload)
    }

    pub fn clear_rgb_targets(&self) {
        self.desired_effects.lock().clear();
    }

    pub fn forget_rgb_target(&self, mac: &[u8; 6], effect_index: [u8; 4]) {
        let mut targets = self.desired_effects.lock();
        if targets.get(mac) == Some(&effect_index) {
            targets.remove(mac);
        }
    }

    pub fn prepare_rgb_upload(
        &self,
        mac: &[u8; 6],
        frames: &[Vec<[u8; 3]>],
        interval_ms: u16,
    ) -> Result<WirelessRgbUpload> {
        self.prepare_rgb_animation(mac, frames, RgbPlaybackTiming::from_millis(interval_ms))
    }

    pub fn prepare_rgb_animation(
        &self,
        mac: &[u8; 6],
        frames: &[Vec<[u8; 3]>],
        timing: RgbPlaybackTiming,
    ) -> Result<WirelessRgbUpload> {
        let device = self.device_by_mac_snapshot(mac)?;
        ensure!(
            device.fan_type != WirelessFanType::Unknown,
            "unknown wireless RGB layout"
        );
        let expected: usize = device
            .fan_type
            .rgb_zone_led_counts(device.fan_count)
            .iter()
            .sum();
        ensure!(
            frames.iter().all(|f| f.len() == expected),
            "RGB frame must contain {expected} LEDs for {}",
            device.fan_type.display_name()
        );
        let reverse = (device.is_inf_right_attach
            && matches!(
                device.fan_type,
                WirelessFanType::SlInf | WirelessFanType::SlInfV3 { .. }
            ))
        .then_some(device.fan_type.leds_per_fan() as usize);
        WirelessRgbUpload::with_timing(frames, timing, reverse)
    }

    pub fn send_rgb_upload(
        &self,
        mac: &[u8; 6],
        upload: &WirelessRgbUpload,
        header_repeats: u8,
    ) -> Result<()> {
        let device = self.device_by_mac_snapshot(mac)?;
        let master = *self.master_mac.lock();
        ensure!(
            device.bind_intent || device.master_mac == master,
            "RGB device is no longer bound to this controller"
        );
        ensure!(
            device.fan_type != WirelessFanType::Unknown,
            "unknown wireless RGB layout"
        );
        let expected: usize = device
            .fan_type
            .rgb_zone_led_counts(device.fan_count)
            .iter()
            .sum();
        let expected = if upload.sync_layout {
            usize::from(
                device
                    .fan_type
                    .rgb_render_profile(device.fan_count)
                    .context("RGB sync layout changed")?
                    .sync_frame_led_count(),
            )
        } else {
            expected
        };
        ensure!(
            expected == upload.led_count as usize,
            "RGB device layout changed before upload"
        );
        ensure!(
            self.mb_rgb_ready_for_upload(mac)?,
            "waiting for motherboard RGB sync to disable"
        );
        let started = Instant::now();
        tracing::trace!(
            mac = ?mac,
            frames = upload.frame_count,
            leds = upload.led_count,
            compressed_bytes = upload.compressed.len(),
            packets = upload.compressed.len().div_ceil(CHUNK_BYTES) + 1,
            interval_ticks = upload.interval_ticks,
            interval_fraction = upload.interval_fraction,
            effect_index = ?upload.effect_index,
            "Uploading wireless RGB loop"
        );
        self.tx_recover(|handle| {
            for index in 0..=upload.compressed.len().div_ceil(CHUNK_BYTES) {
                ensure!(
                    started.elapsed() < Duration::from_secs(3),
                    "wireless RGB upload timed out"
                );
                let packet = upload.packet(mac, &master, index);
                let repeats = if index == 0 {
                    header_repeats.clamp(1, 4)
                } else {
                    1
                };
                for repeat in 0..repeats {
                    self.send_rf_packet(handle, &device, &packet)
                        .with_context(|| {
                            format!("sending wireless RGB packet {index}, repeat {repeat}")
                        })?;
                    if repeat + 1 < repeats {
                        thread::sleep(Duration::from_millis(if repeats <= 2 { 2 } else { 20 }));
                    }
                }
            }
            Ok(())
        })?;
        self.desired_effects
            .lock()
            .insert(*mac, upload.effect_index);
        Ok(())
    }

    pub fn send_rgb_direct(
        &self,
        mac: &[u8; 6],
        colors: &[[u8; 3]],
        effect_index: &[u8; 4],
        header_repeats: u8,
    ) -> Result<()> {
        let mut upload = self.prepare_rgb_upload(mac, &[colors.to_vec()], 5000)?;
        upload.effect_index = *effect_index;
        self.send_rgb_upload(mac, &upload, header_repeats)
    }

    pub fn send_rgb_frames(
        &self,
        mac: &[u8; 6],
        frames: &[Vec<[u8; 3]>],
        interval_ms: u16,
        effect_index: &[u8; 4],
        header_repeats: u8,
    ) -> Result<()> {
        let mut upload = self.prepare_rgb_upload(mac, frames, interval_ms)?;
        upload.effect_index = *effect_index;
        self.send_rgb_upload(mac, &upload, header_repeats)
    }
}

impl WirelessFanType {
    pub fn rgb_zone_led_counts(self, fan_count: u8) -> Vec<usize> {
        if let Some(total) = self.total_led_count_override() {
            return vec![total as usize];
        }
        let mut zones = Vec::with_capacity(fan_count as usize + usize::from(self.is_aio()));
        if self.is_aio() {
            zones.push(self.pump_led_count() as usize);
        }
        zones.extend(std::iter::repeat_n(
            self.leds_per_fan() as usize,
            fan_count as usize,
        ));
        zones
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canceling_an_old_upload_does_not_forget_a_newer_target() {
        let controller = WirelessController::new();
        controller.desired_effects.lock().insert([1; 6], [2; 4]);
        controller.forget_rgb_target(&[1; 6], [1; 4]);
        assert_eq!(
            controller.desired_effects.lock().get(&[1; 6]),
            Some(&[2; 4])
        );
        controller.forget_rgb_target(&[1; 6], [2; 4]);
        assert!(controller.desired_effects.lock().is_empty());
        controller.desired_effects.lock().insert([3; 6], [4; 4]);
        controller.clear_rgb_targets();
        assert!(controller.desired_effects.lock().is_empty());
    }

    #[test]
    fn secondary_region_timing_is_transmitted_and_changes_identity() {
        let frames = vec![vec![[20, 30, 40]; 120]; 30];
        let timing = RgbPlaybackTiming {
            interval_hundredths: 5_775,
            secondary_interval_ticks: 63,
            secondary_frame_count: 28,
            outer_longest: true,
        };
        let upload = WirelessRgbUpload::with_timing(&frames, timing, None).unwrap();
        let header = upload.packet(&[1; 6], &[2; 6], 0);
        assert_eq!(&header[32..40], &[0, 57, 75, 0, 63, 1, 0, 28]);
        let changed = WirelessRgbUpload::with_timing(
            &frames,
            RgbPlaybackTiming {
                secondary_frame_count: 29,
                ..timing
            },
            None,
        )
        .unwrap();
        assert_ne!(upload.effect_index(), changed.effect_index());
    }

    #[test]
    fn accepts_long_native_loops_without_decimating_frames() {
        let frames = vec![vec![[5, 6, 7]; 174]; 4096];
        let upload = WirelessRgbUpload::new(&frames, 50, None).unwrap();
        assert_eq!(upload.frame_count(), 4096);
        let header = upload.packet(&[1; 6], &[2; 6], 0);
        assert_eq!(&header[25..27], &[16, 0]);
        assert!(WirelessRgbUpload::new(&vec![vec![[0; 3]; 1]; 4097], 50, None).is_err());
    }

    #[test]
    fn header_and_chunks_describe_exact_payload() {
        let frames = vec![vec![[17, 38, 59]; 174]; 120];
        let upload = WirelessRgbUpload::new(&frames, 50, None).unwrap();
        let header = upload.packet(&[1; 6], &[2; 6], 0);
        assert_eq!(
            &header[..14],
            &[0x12, 0x20, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2]
        );
        assert_eq!(&header[25..28], &[0, 120, 174]);
        assert_eq!(&header[32..34], &[0, 80]);
        assert_eq!(
            u32::from_be_bytes(header[20..24].try_into().unwrap()) as usize,
            upload.compressed.len()
        );
        let mut reconstructed = Vec::new();
        for index in 1..header[19] {
            let packet = upload.packet(&[1; 6], &[2; 6], index as usize);
            assert_eq!(packet[18], index);
            reconstructed.extend_from_slice(&packet[20..]);
        }
        reconstructed.truncate(upload.compressed.len());
        assert_eq!(reconstructed, upload.compressed);
    }

    #[test]
    fn reverses_fans_within_each_frame_without_reversing_time() {
        let frames = vec![
            vec![[1; 3], [2; 3], [3; 3], [4; 3]],
            vec![[5; 3], [6; 3], [7; 3], [8; 3]],
        ];
        let expected = vec![
            vec![[3; 3], [4; 3], [1; 3], [2; 3]],
            vec![[7; 3], [8; 3], [5; 3], [6; 3]],
        ];
        assert_eq!(
            WirelessRgbUpload::new(&frames, 50, Some(2)).unwrap(),
            WirelessRgbUpload::new(&expected, 50, None).unwrap()
        );
    }

    #[test]
    fn validates_counts_timing_and_device_layouts() {
        assert!(WirelessRgbUpload::new(&[], 50, None).is_err());
        assert!(WirelessRgbUpload::new(&[vec![]], 50, None).is_err());
        assert!(WirelessRgbUpload::new(&[vec![[0; 3]; 256]], 50, None).is_err());
        assert!(WirelessRgbUpload::new(&[vec![[0; 3]; 9]], 0, None).is_err());
        assert!(WirelessRgbUpload::new(&[vec![[0; 3]; 9], vec![[0; 3]; 10]], 50, None).is_err());
        assert_eq!(
            WirelessFanType::WaterBlock.rgb_zone_led_counts(3),
            [24, 24, 24, 24]
        );
        assert_eq!(WirelessFanType::WaterBlock2.rgb_zone_led_counts(0), [24]);
        assert_eq!(WirelessFanType::V150.rgb_zone_led_counts(4), [88]);
        assert_eq!(WirelessFanType::SlInf.rgb_zone_led_counts(3), [44, 44, 44]);
        assert_eq!(WirelessFanType::Strimer(3).rgb_zone_led_counts(0), [174]);
        assert_ne!(
            WirelessRgbUpload::new(&[vec![[0; 3]; 9]], 50, None)
                .unwrap()
                .effect_index,
            WirelessRgbUpload::new(&[vec![[0; 3]; 9]], 100, None)
                .unwrap()
                .effect_index
        );
    }

    #[test]
    fn preserves_vendor_fractional_rf_intervals() {
        let frames = vec![vec![[255, 0, 0]; 78], vec![[0, 255, 0]; 78]];
        for (ticks, bytes) in [(1100, [0, 11, 0]), (1430, [0, 14, 30]), (1650, [0, 16, 50])] {
            let upload = WirelessRgbUpload::with_tick_interval(&frames, ticks, None).unwrap();
            assert_eq!(&upload.packet(&[1; 6], &[2; 6], 0)[32..35], &bytes);
        }
        let upload = WirelessRgbUpload::new(&frames, 11, None).unwrap();
        assert_eq!(&upload.packet(&[1; 6], &[2; 6], 0)[32..35], &[0, 17, 60]);
        assert!(WirelessRgbUpload::with_tick_interval(&frames, 0, None).is_err());
        assert!(WirelessRgbUpload::with_tick_interval(&frames, 6_553_600, None).is_err());
    }

    #[test]
    fn rejects_payload_larger_than_firmware_memory() {
        let mut seed = 1u32;
        let frames: Vec<_> = (0..120)
            .map(|_| {
                (0..255)
                    .map(|_| {
                        std::array::from_fn(|_| {
                            seed ^= seed << 13;
                            seed ^= seed >> 17;
                            seed ^= seed << 5;
                            seed as u8
                        })
                    })
                    .collect()
            })
            .collect();
        assert!(WirelessRgbUpload::new(&frames, 50, None)
            .unwrap_err()
            .to_string()
            .contains("12288"));
    }
}
