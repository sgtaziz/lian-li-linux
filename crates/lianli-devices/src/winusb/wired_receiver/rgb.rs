use super::*;

impl WiredReceiverController {
    /// Send a full RGB frame (all fans' LEDs) using the per-PID transport.
    pub fn send_rgb_frame(&self, colors: &[[u8; 3]]) -> Result<()> {
        self.send_rgb_frame_ext(colors, 0, 1, 0, 20, false)
    }

    /// Send a full RGB frame with explicit flash-save parameters (TL Flex).
    pub fn send_rgb_frame_ext(
        &self,
        colors: &[[u8; 3]],
        effect_index: u32,
        total_frame: u16,
        total_sub_frame: u16,
        interval_ms: u16,
        is_outer_match_max: bool,
    ) -> Result<()> {
        let timing = RgbPlaybackTiming {
            interval_hundredths: u32::from(interval_ms) * 160,
            secondary_interval_ticks: 0,
            secondary_frame_count: total_sub_frame,
            outer_longest: is_outer_match_max,
        };
        if self.is_wireless.load(Ordering::Relaxed) {
            return Ok(());
        }
        let raw = self.ordered_rgb_bytes(colors);
        let led_total = raw.len() / 3;
        if self.params.compresses_rgb {
            let effect_index = if effect_index == 0 {
                self.effect_nonce.fetch_add(1, Ordering::Relaxed).max(1)
            } else {
                effect_index
            };
            self.send_rgb_flash_save(&raw, led_total, effect_index, total_frame, timing)?;
        } else {
            self.send_rgb_stream(&raw)?;
        }
        Ok(())
    }

    fn ordered_rgb_bytes(&self, colors: &[[u8; 3]]) -> Vec<u8> {
        let right_attach = *self.is_inf_right_attach.lock();
        let ordered = if right_attach {
            reverse_per_fan_chunks(colors, self.params.leds_per_fan as usize)
        } else {
            colors.to_vec()
        };
        let mut raw = Vec::with_capacity(ordered.len() * 3);
        for c in &ordered {
            raw.extend_from_slice(c);
        }
        raw
    }

    fn send_rgb_stream_frame(&self, colors: &[[u8; 3]]) -> Result<()> {
        self.send_rgb_stream(&self.ordered_rgb_bytes(colors))
    }

    fn send_rgb_frames_loop(
        &self,
        frames: &[Vec<[u8; 3]>],
        timing: RgbPlaybackTiming,
    ) -> Result<()> {
        let raw = self.ordered_rgb_frames(frames);
        self.send_rgb_flash_save(
            &raw,
            frames[0].len(),
            self.effect_nonce.fetch_add(1, Ordering::Relaxed).max(1),
            checked_frame_count(frames.len())?,
            timing,
        )
    }

    fn ordered_rgb_frames(&self, frames: &[Vec<[u8; 3]>]) -> Vec<u8> {
        let right_attach = *self.is_inf_right_attach.lock();
        let leds_per_fan = self.params.leds_per_fan as usize;
        let mut raw = Vec::with_capacity(frames.len() * frames[0].len() * 3);
        for frame in frames {
            if right_attach {
                for fan in frame.chunks_exact(leds_per_fan).rev() {
                    raw.extend(fan.iter().flatten());
                }
            } else {
                raw.extend(frame.iter().flatten());
            }
        }
        raw
    }

    /// P28 V2 / CL V2: stream raw RGB via 0x11 in 20-LED chunks.
    /// Ack is read every 14th frame-terminating chunk to avoid USB contention.
    fn send_rgb_stream(&self, raw: &[u8]) -> Result<()> {
        let total_leds = raw.len() / 3;
        let transport = self.transport.lock();
        let mut ack_counter = self.stream_ack_counter.lock();
        let mut offset = 0usize;
        while offset < total_leds {
            let count = (total_leds - offset).min(RGB_LED_CHUNK);
            let is_last = offset + count >= total_leds;

            let mut pkt = [0u8; PACKET_SIZE];
            pkt[0] = CMD_STREAM_RGB;
            pkt[1] = offset as u8;
            pkt[2] = count as u8;
            pkt[3] = if is_last { 2 } else { 0 };

            let boff = offset * 3;
            let blen = count * 3;
            pkt[4..4 + blen].copy_from_slice(&raw[boff..boff + blen]);

            transport.write(&pkt, LCD_WRITE_TIMEOUT)?;

            if is_last {
                *ack_counter += 1;
                if *ack_counter >= RGB_ACK_INTERVAL {
                    drop(ack_counter);
                    let mut rx = [0u8; PACKET_SIZE];
                    let _ = transport.read(&mut rx, LCD_READ_TIMEOUT);
                    ack_counter = self.stream_ack_counter.lock();
                    *ack_counter = 0;
                }
            }
            offset += count;
        }
        debug!("{}: streamed {total_leds} LEDs via 0x11", self.params.name);
        Ok(())
    }

    /// TL Flex: tinyuz-compress, upload via 0x18, commit via 0x19.
    fn send_rgb_flash_save(
        &self,
        raw: &[u8],
        led_total: usize,
        effect_index: u32,
        total_frame: u16,
        timing: RgbPlaybackTiming,
    ) -> Result<()> {
        anyhow::ensure!(
            led_total <= u8::MAX as usize,
            "{}: too many LEDs",
            self.params.name
        );
        let compressed = crate::tinyuz::compress(raw).context("compressing RGB data")?;
        anyhow::ensure!(
            compressed.len() <= 12_288,
            "{}: compressed RGB upload exceeds 12288 bytes",
            self.params.name
        );
        anyhow::ensure!(
            compressed.len().div_ceil(60) <= u8::MAX as usize,
            "{}: RGB upload has too many packets",
            self.params.name
        );

        let hdr = rgb_flash_header(
            compressed.len(),
            led_total,
            effect_index,
            total_frame,
            timing,
        )?;

        let transport = self.transport.lock();
        let deadline = Instant::now() + RGB_PACKAGE_DEADLINE;
        transport.write(&hdr, rgb_timeout(deadline, LCD_WRITE_TIMEOUT)?)?;
        read_rgb_ack(&transport, deadline, CMD_SEND_LIGHT_PACKAGE)?;

        let mut offset = 0usize;
        let mut idx = 1u8;
        while offset < compressed.len() {
            let mut pkt = [0u8; PACKET_SIZE];
            pkt[0] = CMD_SEND_LIGHT_PACKAGE;
            pkt[1] = idx;
            let chunk = (compressed.len() - offset).min(60);
            pkt[4..4 + chunk].copy_from_slice(&compressed[offset..offset + chunk]);
            transport.write(&pkt, rgb_timeout(deadline, LCD_WRITE_TIMEOUT)?)?;
            read_rgb_ack(&transport, deadline, CMD_SEND_LIGHT_PACKAGE)?;
            offset += 60;
            idx += 1;
        }

        let mut apply = [0u8; PACKET_SIZE];
        apply[0] = CMD_APPLY_LIGHTING;
        transport.write(&apply, rgb_timeout(deadline, LCD_WRITE_TIMEOUT)?)?;
        read_rgb_ack(&transport, deadline, CMD_APPLY_LIGHTING)?;

        debug!(
            "{}: flash-saved {led_total} LEDs ({} compressed bytes, idx={:#010x}, frames={total_frame}, interval={}.{:02} ticks) via 0x18+0x19",
            self.params.name,
            compressed.len(),
            effect_index,
            timing.interval_hundredths / 100,
            timing.interval_hundredths % 100,
        );
        Ok(())
    }
}

impl RgbDevice for WiredReceiverController {
    fn device_name(&self) -> String {
        self.params.name.to_string()
    }

    fn supported_modes(&self) -> Vec<RgbMode> {
        vec![RgbMode::Off, RgbMode::Static, RgbMode::Direct]
    }

    fn zone_info(&self) -> Vec<RgbZoneInfo> {
        let count = *self.fan_count.lock() as u16;
        (0..count)
            .map(|fan| RgbZoneInfo {
                name: format!("Fan {}", fan + 1),
                led_count: self.params.leds_per_fan,
            })
            .collect()
    }

    fn supports_direct(&self) -> bool {
        true
    }

    fn software_render_profile(&self) -> Option<RgbRenderProfile> {
        self.params
            .render_profile(self.render_family, *self.fan_count.lock())
            .map(|mut profile| {
                profile.right_attach = *self.is_inf_right_attach.lock();
                profile
            })
    }

    fn rf_owned(&self) -> bool {
        self.is_wireless.load(Ordering::Relaxed)
    }

    fn software_frame_delivery(&self) -> Option<crate::traits::RgbFrameDelivery> {
        if self.params.compresses_rgb {
            Some(crate::traits::RgbFrameDelivery::LoopUpload)
        } else {
            Some(crate::traits::RgbFrameDelivery::Streaming)
        }
    }

    fn set_software_animation(
        &self,
        frames: &[Vec<[u8; 3]>],
        timing: RgbPlaybackTiming,
    ) -> Result<()> {
        if self.rf_owned() {
            return Ok(());
        }
        let frame_count = checked_frame_count(frames.len())?;
        let fan_count = *self.fan_count.lock() as usize;
        let expected_leds = fan_count * self.params.leds_per_fan as usize;
        anyhow::ensure!(
            frames.iter().all(|frame| frame.len() == expected_leds),
            "{}: RGB frames must contain {expected_leds} LEDs",
            self.params.name
        );

        match self.software_frame_delivery() {
            Some(crate::traits::RgbFrameDelivery::Streaming) => {
                anyhow::ensure!(
                    frame_count == 1,
                    "{}: streaming accepts one frame",
                    self.params.name
                );
                anyhow::ensure!(
                    timing.secondary_interval_ticks == 0
                        && timing.secondary_frame_count == 0
                        && !timing.outer_longest,
                    "{}: streaming does not accept secondary timing",
                    self.params.name
                );
                anyhow::ensure!(
                    (100..=6_553_599).contains(&timing.interval_hundredths),
                    "{}: RGB interval out of range",
                    self.params.name
                );
                self.send_rgb_stream_frame(&frames[0])
            }
            Some(crate::traits::RgbFrameDelivery::LoopUpload) => {
                self.send_rgb_frames_loop(frames, timing)
            }
            None => anyhow::bail!("{}: software RGB is not supported", self.params.name),
        }
    }

    fn set_software_frames(&self, frames: &[Vec<[u8; 3]>], interval_ms: u16) -> Result<()> {
        self.set_software_animation(frames, RgbPlaybackTiming::from_millis(interval_ms))
    }

    fn validate_software_animation(
        &self,
        frames: &[Vec<[u8; 3]>],
        timing: RgbPlaybackTiming,
    ) -> Result<()> {
        let count = checked_frame_count(frames.len())?;
        let leds = self.total_leds();
        anyhow::ensure!(
            leds > 0 && frames.iter().all(|frame| frame.len() == leds),
            "RGB frame does not match receiver layout"
        );
        if self.params.compresses_rgb {
            let compressed = crate::tinyuz::compress(&self.ordered_rgb_frames(frames))?;
            anyhow::ensure!(
                compressed.len() <= 12_288,
                "compressed RGB animation exceeds receiver memory"
            );
            rgb_flash_header(compressed.len(), leds, 0, count, timing)?;
        } else {
            anyhow::ensure!(
                (100..=6_553_599).contains(&timing.interval_hundredths),
                "invalid RGB playback interval"
            );
            anyhow::ensure!(
                timing.secondary_interval_ticks == 0
                    && timing.secondary_frame_count == 0
                    && !timing.outer_longest,
                "streaming RGB does not support secondary timing"
            );
        }
        Ok(())
    }

    fn supported_scopes(&self) -> Vec<Vec<RgbScope>> {
        vec![vec![RgbScope::All]; *self.fan_count.lock() as usize]
    }

    fn set_zone_effect(&self, zone: u8, effect: &RgbEffect) -> Result<()> {
        let color = if effect.mode == RgbMode::Off || effect.disabled {
            [0, 0, 0]
        } else {
            let base = effect.colors.first().copied().unwrap_or([255, 255, 255]);
            scale_brightness(base, effect.brightness)
        };

        let leds_per_fan = self.params.leds_per_fan as usize;
        let mut buf = self.led_buffer.lock();
        let start = (zone as usize).min(buf.len() / leds_per_fan.max(1)) * leds_per_fan;
        let end = (start + leds_per_fan).min(buf.len());
        for led in &mut buf[start..end] {
            *led = color;
        }
        let frame = buf.clone();
        drop(buf);
        self.send_rgb_frame(&frame)
    }

    fn set_direct_colors(&self, zone: u8, colors: &[[u8; 3]]) -> Result<()> {
        let leds_per_fan = self.params.leds_per_fan as usize;
        let mut buf = self.led_buffer.lock();
        let start = (zone as usize).min(buf.len() / leds_per_fan.max(1)) * leds_per_fan;
        for (i, &c) in colors.iter().enumerate().take(leds_per_fan) {
            if start + i < buf.len() {
                buf[start + i] = c;
            }
        }
        let frame = buf.clone();
        drop(buf);
        self.send_rgb_frame(&frame)
    }

    fn ping(&self, _zone: u8) -> Result<()> {
        self.selected_group()
    }
}

fn scale_brightness([r, g, b]: [u8; 3], brightness: u8) -> [u8; 3] {
    let scale = (lianli_shared::rgb::brightness_scale(brightness) as f32) / 4.0;
    [
        (r as f32 * scale).round() as u8,
        (g as f32 * scale).round() as u8,
        (b as f32 * scale).round() as u8,
    ]
}

/// Reverse the per-fan chunk order in a flat LED color buffer for SL-INF
/// right-attach daisy-chains (chain wires right-to-left).
fn reverse_per_fan_chunks(colors: &[[u8; 3]], leds_per_fan: usize) -> Vec<[u8; 3]> {
    if leds_per_fan == 0 {
        return colors.to_vec();
    }
    let total = colors.len();
    let fan_count = total.div_ceil(leds_per_fan);
    if fan_count <= 1 {
        return colors.to_vec();
    }
    let mut out = Vec::with_capacity(total);
    for fan_idx in (0..fan_count).rev() {
        let start = fan_idx * leds_per_fan;
        let end = (start + leds_per_fan).min(total);
        if start < end {
            out.extend_from_slice(&colors[start..end]);
        }
    }
    out
}
