use anyhow::{ensure, Context, Result};
use lianli_media::rgb::{is_animated, render_zone, validate_effect, LOOP_FRAMES};
use lianli_shared::rgb::{RgbEffect, RgbMode, RgbPresetZone};
use std::ops::Range;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct RenderState {
    pub counts: Vec<usize>,
    pub colors: Vec<[u8; 3]>,
    pub effects: Vec<Option<RgbEffect>>,
}

impl RenderState {
    pub fn new(counts: Vec<usize>) -> Self {
        Self {
            colors: vec![[0; 3]; counts.iter().sum()],
            effects: vec![None; counts.len()],
            counts,
        }
    }

    pub fn range(&self, zone: u8) -> Result<Range<usize>> {
        let count = *self
            .counts
            .get(zone as usize)
            .context("RGB zone out of range")?;
        let start: usize = self.counts[..zone as usize].iter().sum();
        Ok(start..start + count)
    }

    pub fn set_effect(&mut self, zone: u8, effect: &RgbEffect) -> Result<()> {
        let range = self.range(zone)?;
        if effect.mode == RgbMode::Direct {
            self.effects[zone as usize] = None;
        } else {
            validate_effect(effect)?;
            render_zone(effect, 0, &mut self.colors[range]);
            self.effects[zone as usize] = Some(effect.clone());
        }
        Ok(())
    }

    pub fn set_direct(&mut self, zone: u8, colors: &[[u8; 3]]) -> Result<()> {
        let range = self.range(zone)?;
        ensure!(
            colors.len() <= range.len(),
            "too many colors for RGB zone {zone}"
        );
        self.colors[range.start..range.start + colors.len()].copy_from_slice(colors);
        self.effects[zone as usize] = None;
        Ok(())
    }

    pub fn frames(&self) -> Vec<Vec<[u8; 3]>> {
        let animated = self.effects.iter().flatten().any(is_animated);
        (0..if animated { LOOP_FRAMES } else { 1 })
            .map(|frame| {
                let mut colors = self.colors.clone();
                let mut offset = 0;
                for (count, effect) in self.counts.iter().zip(&self.effects) {
                    if let Some(effect) = effect {
                        render_zone(effect, frame, &mut colors[offset..offset + count]);
                    }
                    offset += count;
                }
                colors
            })
            .collect()
    }

    pub fn upload_frames(&self) -> Result<(Vec<Vec<[u8; 3]>>, u16)> {
        let frames = self.frames();
        for stride in [1, 2, 4] {
            let sampled: Vec<_> = frames.iter().step_by(stride).cloned().collect();
            let raw: Vec<_> = sampled.iter().flatten().flatten().copied().collect();
            // Leave room for compression differences after device-specific ordering.
            if lianli_devices::tinyuz::compress(&raw)?.len() <= 10_240 {
                return Ok((
                    sampled,
                    lianli_media::rgb::FRAME_INTERVAL_MS * stride as u16,
                ));
            }
        }
        anyhow::bail!("RGB animation exceeds receiver memory; reduce palette or effect complexity")
    }

    pub fn preset_zones(&self) -> Vec<RgbPresetZone> {
        self.effects
            .iter()
            .enumerate()
            .map(|(zone, effect)| RgbPresetZone {
                zone: zone as u8,
                colors: if effect.is_none() {
                    self.colors[self.range(zone as u8).unwrap()].to_vec()
                } else {
                    vec![]
                },
                effect: Some(effect.clone().unwrap_or_else(|| RgbEffect {
                    mode: RgbMode::Direct,
                    ..Default::default()
                })),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertised_effects_fit_supported_upload_layouts() {
        use lianli_devices::wireless::{WirelessFanType, WirelessRgbUpload};
        use lianli_media::rgb::{FRAME_INTERVAL_MS, SOFTWARE_MODES};
        for family in [
            WirelessFanType::SlV4,
            WirelessFanType::SlInf,
            WirelessFanType::Tlv2Led,
            WirelessFanType::Slv3Led,
            WirelessFanType::WaterBlock,
            WirelessFanType::P28V2,
            WirelessFanType::Strimer(3),
            WirelessFanType::Led88,
            WirelessFanType::Lc217,
            WirelessFanType::V150,
        ] {
            for &mode in SOFTWARE_MODES {
                let mut state = RenderState::new(family.rgb_zone_led_counts(4));
                for zone in 0..state.counts.len() {
                    state
                        .set_effect(
                            zone as u8,
                            &RgbEffect {
                                mode,
                                speed: zone as u8,
                                colors: vec![[255, 0, 0], [0, 255, 0], [0, 0, 255], [127, 63, 199]],
                                ..Default::default()
                            },
                        )
                        .unwrap();
                }
                let (frames, interval) = state
                    .upload_frames()
                    .unwrap_or_else(|error| panic!("{family:?} {mode:?}: {error}"));
                assert!(interval >= FRAME_INTERVAL_MS);
                WirelessRgbUpload::new(&frames, interval, None)
                    .unwrap_or_else(|error| panic!("{family:?} {mode:?}: {error}"));
            }
        }
    }

    #[test]
    fn independent_zones_keep_direct_colors_and_animation() {
        let mut state = RenderState::new(vec![24, 26, 44]);
        state.set_direct(0, &[[7, 8, 9]; 24]).unwrap();
        state
            .set_effect(
                1,
                &RgbEffect {
                    mode: RgbMode::Rainbow,
                    ..Default::default()
                },
            )
            .unwrap();
        state
            .set_effect(
                2,
                &RgbEffect {
                    colors: vec![[10, 20, 30]],
                    ..Default::default()
                },
            )
            .unwrap();
        let frames = state.frames();
        assert_eq!(frames.len(), LOOP_FRAMES);
        assert!(frames
            .iter()
            .all(|f| f.len() == 94 && f[..24] == [[7, 8, 9]; 24] && f[50..] == [[10, 20, 30]; 44]));
        assert_ne!(frames[0][24..50], frames[1][24..50]);
        let zones = state.preset_zones();
        assert_eq!(zones[1].effect.as_ref().unwrap().mode, RgbMode::Rainbow);
        assert!(zones[1].colors.is_empty());
        state.set_direct(1, &[[1; 3]; 26]).unwrap();
        assert_eq!(state.frames().len(), 1);
        assert!(state.set_direct(3, &[]).is_err());
        assert!(state.set_direct(0, &[[0; 3]; 25]).is_err());
    }
}
