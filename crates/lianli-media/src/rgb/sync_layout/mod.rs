use std::ops::Range;

use anyhow::{ensure, Result};
use lianli_shared::rgb::{RgbRenderFamily, RgbRenderProfile};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    logical_led_count: usize,
    physical_to_logical: Vec<Option<usize>>,
}

impl Layout {
    pub fn for_profile(profile: RgbRenderProfile) -> Option<Self> {
        let map = match profile.family {
            RgbRenderFamily::Tl => per_fan(profile, 13, 26, tl_fan_map)?,
            RgbRenderFamily::Sl => per_fan(profile, 12, 40, sl_fan_map)?,
            RgbRenderFamily::SlInf | RgbRenderFamily::SlInfV3 => {
                let fan_map = sl_inf_fan_map(profile.right_attach);
                per_fan(profile, 8, 44, || fan_map.clone())?
            }
            RgbRenderFamily::SlV4 => {
                validate_per_fan_profile(profile, 52)?;
                let populated = repeated_fan_map(profile.fan_count, sl_v4_fan_map);
                with_black_tail(populated, usize::from(profile.led_count))?
            }
            RgbRenderFamily::Cl => per_fan(profile, 8, 24, cl_fan_map)?,
            RgbRenderFamily::P28 => {
                validate_per_fan_profile(profile, 9)?;
                identity(usize::from(profile.led_count))
            }
            RgbRenderFamily::Strimer => strimer_map(profile)?,
            RgbRenderFamily::HydroShiftII => hydroshift_ii_map(profile)?,
            RgbRenderFamily::HydroShiftIIOled => (profile.fan_count == 0
                && matches!(profile.led_count, 35 | 45))
            .then(hs2_oled_map)?,
            RgbRenderFamily::UniversalScreen => (profile.fan_count == 0
                && matches!(profile.led_count, 60 | 88))
            .then(universal_screen_map)?,
            RgbRenderFamily::Lancool217 => fixed_identity(profile, 96).then(|| identity(96))?,
            RgbRenderFamily::LancoolV150 => {
                (profile.fan_count <= 4 && profile.led_count == 88).then(|| identity(88))?
            }
        };

        let logical_led_count = map.iter().flatten().copied().max()? + 1;
        Some(Self {
            logical_led_count,
            physical_to_logical: map,
        })
    }

    pub fn logical_led_count(&self) -> usize {
        self.logical_led_count
    }

    pub fn physical_led_count(&self) -> usize {
        self.physical_to_logical.len()
    }

    pub fn project_frame(
        &self,
        source: &[[u8; 3]],
        span: Range<usize>,
        reverse: bool,
    ) -> Result<Vec<[u8; 3]>> {
        ensure!(
            span.start <= span.end && span.end <= source.len(),
            "RGB sync span {span:?} is outside a {}-LED source frame",
            source.len()
        );
        ensure!(
            span.len() == self.logical_led_count,
            "RGB sync layout requires {} logical LEDs, but span {span:?} contains {}",
            self.logical_led_count,
            span.len()
        );

        Ok(self
            .physical_to_logical
            .iter()
            .map(|logical| {
                logical.map_or([0; 3], |logical| {
                    let offset = if reverse {
                        self.logical_led_count - logical - 1
                    } else {
                        logical
                    };
                    source[span.start + offset]
                })
            })
            .collect())
    }
}

fn validate_per_fan_profile(profile: RgbRenderProfile, physical_per_fan: u16) -> Option<()> {
    ((1..=4).contains(&profile.fan_count)
        && profile.led_count == u16::from(profile.fan_count) * physical_per_fan)
        .then_some(())
}

fn per_fan<F>(
    profile: RgbRenderProfile,
    logical_per_fan: usize,
    physical_per_fan: u16,
    fan_map: F,
) -> Option<Vec<Option<usize>>>
where
    F: Fn() -> Vec<Option<usize>>,
{
    validate_per_fan_profile(profile, physical_per_fan)?;
    let map = repeated_fan_map(profile.fan_count, fan_map);
    (map.iter().flatten().copied().max()? + 1 == usize::from(profile.fan_count) * logical_per_fan)
        .then_some(map)
}

fn repeated_fan_map<F>(fan_count: u8, fan_map: F) -> Vec<Option<usize>>
where
    F: Fn() -> Vec<Option<usize>>,
{
    let one_fan = fan_map();
    let logical_per_fan = one_fan.iter().flatten().copied().max().unwrap_or(0) + 1;
    (0..usize::from(fan_count))
        .flat_map(|fan| {
            one_fan
                .iter()
                .map(move |logical| logical.map(|logical| fan * logical_per_fan + logical))
        })
        .collect()
}

fn identity(count: usize) -> Vec<Option<usize>> {
    (0..count).map(Some).collect()
}

fn with_black_tail(
    mut populated: Vec<Option<usize>>,
    physical_count: usize,
) -> Option<Vec<Option<usize>>> {
    (populated.len() <= physical_count).then(|| {
        populated.resize(physical_count, None);
        populated
    })
}

fn fixed_identity(profile: RgbRenderProfile, led_count: u16) -> bool {
    profile.fan_count == 0 && profile.led_count == led_count
}

fn tl_fan_map() -> Vec<Option<usize>> {
    (0..13).chain(0..13).map(Some).collect()
}

fn sl_fan_map() -> Vec<Option<usize>> {
    const SIDE: [usize; 8] = [0, 1, 3, 5, 6, 8, 10, 11];
    (0..12)
        .chain(SIDE)
        .chain(0..12)
        .chain(SIDE)
        .map(Some)
        .collect()
}

fn sl_v4_fan_map() -> Vec<Option<usize>> {
    (0..13).chain(0..13).map(Some).collect()
}

fn cl_fan_map() -> Vec<Option<usize>> {
    (0..8)
        .chain((0..8).rev())
        .chain((0..8).rev())
        .map(Some)
        .collect()
}

fn sl_inf_fan_map(right_attach: bool) -> Vec<Option<usize>> {
    const LEFT_INNER: [usize; 8] = [3, 5, 7, 6, 4, 2, 0, 1];
    const RIGHT_INNER: [usize; 8] = [4, 2, 0, 1, 3, 5, 7, 6];
    const LEFT_LONG: [usize; 10] = [0, 1, 2, 2, 3, 4, 5, 6, 6, 7];
    const RIGHT_LONG: [usize; 10] = [7, 6, 5, 5, 4, 3, 2, 1, 1, 0];
    const LEFT_SHORT: [usize; 8] = [0, 1, 2, 3, 4, 5, 6, 7];
    const RIGHT_SHORT: [usize; 8] = [7, 6, 5, 4, 3, 2, 1, 0];

    let (inner, long, short) = if right_attach {
        (&RIGHT_INNER, &RIGHT_LONG, &RIGHT_SHORT)
    } else {
        (&LEFT_INNER, &LEFT_LONG, &LEFT_SHORT)
    };
    inner
        .iter()
        .chain(long)
        .chain(short)
        .chain(long)
        .chain(short)
        .copied()
        .map(Some)
        .collect()
}

fn strimer_map(profile: RgbRenderProfile) -> Option<Vec<Option<usize>>> {
    if profile.fan_count != 0 {
        return None;
    }
    let logical_count = match profile.led_count {
        116 | 174 => 29,
        88 | 132 => 22,
        _ => return None,
    };
    let mut map: Vec<_> = (0..6).flat_map(|_| (0..logical_count).map(Some)).collect();
    map.truncate(usize::from(profile.led_count));
    Some(map)
}

fn hydroshift_ii_map(profile: RgbRenderProfile) -> Option<Vec<Option<usize>>> {
    (profile.fan_count <= 4 && profile.led_count == u16::from(profile.fan_count + 1) * 24).then(
        || {
            identity(24)
                .into_iter()
                .chain(
                    repeated_fan_map(profile.fan_count, cl_fan_map)
                        .into_iter()
                        .map(|logical| logical.map(|logical| logical + 24)),
                )
                .collect()
        },
    )
}

fn hs2_oled_map() -> Vec<Option<usize>> {
    (0..14)
        .rev()
        .map(Some)
        .chain(std::iter::repeat_n(None, 7))
        .chain((0..14).map(Some))
        .collect()
}

fn universal_screen_map() -> Vec<Option<usize>> {
    (13..31)
        .chain((0..30).rev())
        .chain(1..13)
        .map(Some)
        .collect()
}

#[cfg(test)]
mod tests;
