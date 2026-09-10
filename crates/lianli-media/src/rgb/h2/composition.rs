use super::{render, MODES};
use anyhow::{bail, ensure, Result};
use lianli_shared::rgb::{RgbEffect, RgbMode, RgbRegionConfig, RgbScope};

use crate::rgb::cl::{self, engine::Plane};
use crate::rgb::Animation;

struct RegionAnimation {
    frames: Vec<Vec<[u8; 3]>>,
    interval_ticks: u32,
    base_ticks: u32,
}

pub fn render_regions(regions: &[RgbRegionConfig], fans: usize) -> Result<Animation> {
    ensure!(fans <= 4, "HydroShift II fan count must be 0..=4");
    ensure!(
        !regions.is_empty(),
        "HydroShift II requires a lighting region"
    );

    let mut pump = None;
    let mut center = None;
    let mut outer = None;
    for region in regions {
        ensure!(
            !region.flip,
            "HydroShift II does not support region flipping"
        );
        let target = match region.effect.scope {
            RgbScope::Pump => &mut pump,
            RgbScope::Center if fans > 0 => &mut center,
            RgbScope::Outer if fans > 0 => &mut outer,
            RgbScope::All if fans == 0 => &mut pump,
            _ => bail!("HydroShift II effects support Pump, Center and Outer regions"),
        };
        ensure!(target.is_none(), "duplicate HydroShift II RGB region");
        *target = Some(&region.effect);
    }

    if fans == 0 {
        ensure!(
            center.is_none() && outer.is_none(),
            "pump-only device has no fan regions"
        );
        let effect = pump.expect("non-empty regions must contain the pump");
        return render_pump(effect);
    }

    let pump = match pump {
        Some(effect) => pump_animation(effect)?,
        None => empty(24),
    };
    let center = match center {
        Some(effect) => cl_animation(effect, fans, Plane::Center)?,
        None => empty(fans * 24),
    };
    let outer = match outer {
        Some(effect) => cl_animation(effect, fans, Plane::Outer)?,
        None => empty(fans * 24),
    };
    combine(pump, outer, center, fans)
}

fn render_pump(effect: &RgbEffect) -> Result<Animation> {
    ensure!(MODES.contains(&effect.mode), "unsupported pump RGB mode");
    let mut effect = effect.clone();
    effect.scope = RgbScope::All;
    render(&effect, 24)
}

fn pump_animation(effect: &RgbEffect) -> Result<RegionAnimation> {
    let animation = render_pump(effect)?;
    Ok(RegionAnimation {
        interval_ticks: animation.interval_hundredths / 100,
        base_ticks: pump_base(effect.mode),
        frames: animation.frames,
    })
}

fn cl_animation(effect: &RgbEffect, fans: usize, plane: Plane) -> Result<RegionAnimation> {
    ensure!(
        cl::MODES.contains(&effect.mode),
        "unsupported CL fan RGB mode"
    );
    let animation = cl::render_plane(effect, fans, plane)?;
    let speed_multiplier = u32::from(7 - effect.speed);
    Ok(RegionAnimation {
        base_ticks: animation.interval_ticks / speed_multiplier,
        interval_ticks: animation.interval_ticks,
        frames: animation.frames,
    })
}

fn pump_base(mode: RgbMode) -> u32 {
    if matches!(mode, RgbMode::RainbowMorph | RgbMode::Breathing) {
        11
    } else {
        20
    }
}

fn empty(width: usize) -> RegionAnimation {
    RegionAnimation {
        frames: vec![vec![[0; 3]; width]],
        interval_ticks: 0,
        base_ticks: 0,
    }
}

fn combine(
    mut pump: RegionAnimation,
    mut outer: RegionAnimation,
    mut center: RegionAnimation,
    fans: usize,
) -> Result<Animation> {
    let dominant =
        if pump.frames.len() >= outer.frames.len() && pump.frames.len() >= center.frames.len() {
            0
        } else if outer.frames.len() >= center.frames.len() {
            1
        } else {
            2
        };
    let maximum = [pump.frames.len(), outer.frames.len(), center.frames.len()][dominant];
    repeat_short_animation(&mut pump.frames, maximum);
    repeat_short_animation(&mut outer.frames, maximum);
    repeat_short_animation(&mut center.frames, maximum);

    let base_ticks = if center.base_ticks > 0 {
        center.base_ticks
    } else {
        outer.base_ticks.max(pump.base_ticks)
    };
    let speed_multiplier = outer
        .interval_ticks
        .checked_div(outer.base_ticks)
        .or_else(|| center.interval_ticks.checked_div(center.base_ticks))
        .or_else(|| pump.interval_ticks.checked_div(pump.base_ticks))
        .unwrap_or(0);
    let initial_interval = base_ticks * speed_multiplier;
    ensure!(
        initial_interval > 0,
        "HydroShift II regions have no playback timing"
    );

    let mut frames = Vec::with_capacity(maximum);
    for index in 0..maximum {
        let pump_index = region_index(index, maximum, &pump, dominant == 0, initial_interval);
        let outer_index = region_index(index, maximum, &outer, dominant == 1, initial_interval);
        let center_index = region_index(index, maximum, &center, dominant == 2, initial_interval);
        let mut output = Vec::with_capacity(24 + fans * 24);
        output.extend_from_slice(&pump.frames[pump_index]);
        for fan in 0..fans {
            let start = fan * 24;
            output.extend_from_slice(&center.frames[center_index][start..start + 8]);
            output.extend_from_slice(&outer.frames[outer_index][start + 8..start + 24]);
        }
        frames.push(output);
    }

    let interval_hundredths = if dominant != 2 && center.interval_ticks > 0 {
        center.interval_ticks * center.frames.len() as u32 * 100 / maximum as u32
    } else {
        initial_interval * 100
    };
    Ok(Animation {
        frames,
        interval_hundredths,
        secondary: None,
    })
}

fn repeat_short_animation(frames: &mut Vec<Vec<[u8; 3]>>, maximum: usize) {
    let ratio = maximum as f64 / frames.len() as f64;
    if maximum / 2 >= frames.len() || ratio.fract() >= 0.9 {
        let source = frames.clone();
        let repeats = if ratio.fract() >= 0.9 {
            ratio.ceil() as usize
        } else {
            maximum / source.len()
        };
        frames.clear();
        for _ in 0..repeats {
            frames.extend(source.iter().cloned());
        }
        frames.truncate(maximum);
    }
}

fn region_index(
    index: usize,
    maximum: usize,
    region: &RegionAnimation,
    dominant: bool,
    interval_ticks: u32,
) -> usize {
    if dominant || region.frames.len() == maximum {
        return index;
    }
    let numerator = interval_ticks as usize * maximum;
    let divisor = numerator / region.frames.len();
    (index * interval_ticks as usize / divisor).min(region.frames.len() - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_region(
        frame_count: usize,
        width: usize,
        interval_ticks: u32,
        base_ticks: u32,
        seed: u8,
    ) -> RegionAnimation {
        let frames = (0..frame_count)
            .map(|frame| {
                (0..width)
                    .map(|led| {
                        std::array::from_fn(|channel| {
                            seed.wrapping_add((frame * 17 + channel * 29 + led * 7) as u8)
                        })
                    })
                    .collect()
            })
            .collect();
        RegionAnimation {
            frames,
            interval_ticks,
            base_ticks,
        }
    }

    fn animation_hash(animation: &Animation) -> u64 {
        fn add(hash: u64, byte: u8) -> u64 {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        }

        let mut hash = 0xcbf29ce484222325;
        for byte in (animation.frames.len() as i32).to_le_bytes() {
            hash = add(hash, byte);
        }
        for byte in (animation.interval_hundredths as i32).to_le_bytes() {
            hash = add(hash, byte);
        }
        for frame in &animation.frames {
            for led in frame {
                for &channel in led {
                    hash = add(hash, channel);
                }
            }
        }
        hash
    }

    fn synthetic_composition_hash(pump: usize, outer: usize, center: usize) -> u64 {
        animation_hash(
            &combine(
                synthetic_region(pump, 24, 55, 11, 3),
                synthetic_region(outer, 48, 80, 20, 71),
                synthetic_region(center, 48, 100, 20, 149),
                2,
            )
            .unwrap(),
        )
    }

    fn region(scope: RgbScope, mode: RgbMode, color: [u8; 3]) -> RgbRegionConfig {
        RgbRegionConfig {
            effect: RgbEffect {
                mode,
                colors: vec![color],
                brightness: 4,
                speed: 2,
                scope,
                ..RgbEffect::default()
            },
            flip: false,
        }
    }

    #[test]
    fn pump_only_accepts_the_legacy_all_alias() {
        let pump = region(RgbScope::Pump, RgbMode::Rainbow, [0; 3]);
        let mut all = pump.clone();
        all.effect.scope = RgbScope::All;
        assert_eq!(
            render_regions(&[pump], 0).unwrap(),
            render_regions(&[all], 0).unwrap()
        );
    }

    #[test]
    fn combines_independent_pump_and_cl_planes_in_transport_order() {
        let animation = render_regions(
            &[
                region(RgbScope::Pump, RgbMode::Static, [255, 0, 0]),
                region(RgbScope::Center, RgbMode::Static, [0, 255, 0]),
                region(RgbScope::Outer, RgbMode::Static, [0, 0, 255]),
            ],
            2,
        )
        .unwrap();
        assert_eq!(animation.frames.len(), 30);
        for frame in animation.frames {
            assert_eq!(frame.len(), 72);
            assert_eq!(frame[..24], [[254, 0, 0]; 24]);
            for fan in 0..2 {
                let start = 24 + fan * 24;
                assert_eq!(frame[start..start + 8], [[0, 254, 0]; 8]);
                assert_eq!(frame[start + 8..start + 24], [[0, 0, 254]; 16]);
            }
        }
    }

    #[test]
    fn missing_regions_are_black_without_hiding_configured_regions() {
        let animation =
            render_regions(&[region(RgbScope::Outer, RgbMode::Static, [0, 0, 255])], 1).unwrap();
        assert!(animation
            .frames
            .iter()
            .all(|frame| frame[..32] == [[0; 3]; 32] && frame[32..] == [[0, 0, 254]; 16]));
    }

    #[test]
    fn enforces_region_specific_mode_lists() {
        assert!(render_regions(
            &[region(RgbScope::Pump, RgbMode::ColorCycle, [255, 0, 0])],
            1
        )
        .is_err());
        assert!(
            render_regions(&[region(RgbScope::Center, RgbMode::Pump, [255, 0, 0])], 1).is_err()
        );
        assert!(render_regions(&[region(RgbScope::All, RgbMode::Rainbow, [0; 3])], 1).is_err());
    }

    #[test]
    fn matches_source_composition_for_each_dominant_region() {
        assert_eq!(synthetic_composition_hash(127, 60, 80), 0x1ba8239ada736fa1);
        assert_eq!(synthetic_composition_hash(24, 144, 80), 0xf3473bc107631b3d);
        assert_eq!(synthetic_composition_hash(127, 60, 170), 0xd3f840b16d20074e);
    }
}
