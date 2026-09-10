use anyhow::{ensure, Context, Result};
use lianli_shared::rgb::{RgbEffect, RgbMode, RgbPresetZone, RgbRegionConfig};
use std::ops::Range;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct RenderState {
    pub counts: Vec<usize>,
    pub colors: Vec<[u8; 3]>,
    pub effects: Vec<Option<RgbEffect>>,
    pub regions: Option<Vec<RgbRegionConfig>>,
}

impl RenderState {
    pub fn same_render(&self, other: &Self) -> bool {
        if self.regions.is_some() && other.regions.is_some() {
            self.counts == other.counts && self.regions == other.regions
        } else {
            self == other
        }
    }

    pub fn new(counts: Vec<usize>) -> Self {
        Self {
            colors: vec![[0; 3]; counts.iter().sum()],
            effects: vec![None; counts.len()],
            regions: None,
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
            self.regions = None;
        } else {
            ensure!(
                matches!(effect.mode, RgbMode::Static | RgbMode::Off),
                "animated RGB requires a supported device effect engine"
            );
            ensure!(
                effect.scope == lianli_shared::rgb::RgbScope::All,
                "direct RGB supports only the whole zone"
            );
            let brightness = if effect.disabled
                || effect.mode == RgbMode::Off
                || lianli_shared::rgb::is_brightness_off(effect.brightness)
            {
                0
            } else {
                *[0u16, 64, 128, 192, 255]
                    .get(effect.brightness as usize)
                    .context("RGB brightness must be 0..=4")?
            };
            let color = effect
                .colors
                .first()
                .copied()
                .unwrap_or([255; 3])
                .map(|channel| (u16::from(channel) * brightness / 255) as u8);
            self.colors[range].fill(color);
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
        self.regions = None;
        Ok(())
    }

    pub fn frames(&self) -> Vec<Vec<[u8; 3]>> {
        vec![self.colors.clone()]
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
    fn regional_preview_does_not_trigger_another_upload() {
        let mut desired = RenderState::new(vec![26; 3]);
        desired.regions = Some(vec![RgbRegionConfig {
            effect: RgbEffect::default(),
            flip: false,
        }]);
        let mut applied = desired.clone();
        applied.colors.fill([254, 0, 0]);
        assert!(desired.same_render(&applied));
        desired.regions.as_mut().unwrap()[0].effect.speed = 4;
        assert!(!desired.same_render(&applied));
        desired.regions = None;
        applied.regions = None;
        assert!(!desired.same_render(&applied));
    }

    #[test]
    fn three_fan_tl_rainbow_upload_retains_the_native_loop() {
        use lianli_shared::rgb::{RgbRegionConfig, RgbRenderFamily, RgbRenderProfile};
        let animation = lianli_media::rgb::family::render(
            RgbRenderProfile {
                family: RgbRenderFamily::Tl,
                fan_count: 3,
                led_count: 78,
                right_attach: false,
            },
            &[RgbRegionConfig {
                effect: RgbEffect {
                    mode: RgbMode::Rainbow,
                    ..Default::default()
                },
                flip: false,
            }],
        )
        .unwrap();
        assert_eq!(animation.frames.len(), 39);
        assert!(animation.frames.iter().all(|frame| frame.len() == 78));
        let upload = lianli_devices::wireless::WirelessRgbUpload::with_timing(
            &animation.frames,
            animation.timing(),
            None,
        )
        .unwrap();
        assert_eq!(upload.frame_count(), 39);
    }

    #[test]
    fn direct_zones_preserve_colors_and_reject_generic_animations() {
        let mut state = RenderState::new(vec![24, 26, 44]);
        state.set_direct(0, &[[7, 8, 9]; 24]).unwrap();
        assert!(state
            .set_effect(
                1,
                &RgbEffect {
                    mode: RgbMode::Rainbow,
                    ..Default::default()
                }
            )
            .is_err());
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
        assert_eq!(frames.len(), 1);
        assert_eq!(&frames[0][..24], &[[7, 8, 9]; 24]);
        assert_eq!(&frames[0][24..50], &[[0; 3]; 26]);
        assert_eq!(&frames[0][50..], &[[10, 20, 30]; 44]);
        let zones = state.preset_zones();
        assert_eq!(zones[0].colors, [[7, 8, 9]; 24]);
        assert_eq!(zones[2].effect.as_ref().unwrap().mode, RgbMode::Static);
        assert!(zones[2].colors.is_empty());
        assert!(state.set_direct(3, &[]).is_err());
        assert!(state.set_direct(0, &[[0; 3]; 25]).is_err());
    }
}
