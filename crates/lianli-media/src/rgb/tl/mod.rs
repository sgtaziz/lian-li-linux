mod breathing;
mod chase;
mod composition;
mod cover_cycle;
mod cycle;
mod door;
mod engine;
mod intertwine;
mod lottery;
mod meteor;
mod mixing;
mod morph;
#[cfg(test)]
mod native_tests;
mod paint;
mod palette;
mod ping_pong;
mod racing;
mod rainbow;
mod reflect;
mod render;
mod ripple;
mod runway;
mod solid;
mod stack;
mod staggered;
mod tide;
mod twinkle;
mod voice;
mod wave;

use anyhow::{bail, ensure, Result};
use lianli_shared::rgb::{is_brightness_off, RgbEffect, RgbMode, RgbRegionConfig, RgbScope};

pub const MODES: &[RgbMode] = &[
    RgbMode::Off,
    RgbMode::Rainbow,
    RgbMode::RainbowMorph,
    RgbMode::Static,
    RgbMode::Breathing,
    RgbMode::Runway,
    RgbMode::Meteor,
    RgbMode::ColorCycle,
    RgbMode::Staggered,
    RgbMode::Tide,
    RgbMode::Mixing,
    RgbMode::Voice,
    RgbMode::Door,
    RgbMode::Render,
    RgbMode::Ripple,
    RgbMode::Reflect,
    RgbMode::TailChasing,
    RgbMode::Paint,
    RgbMode::PingPong,
    RgbMode::Stack,
    RgbMode::CoverCycle,
    RgbMode::Wave,
    RgbMode::Racing,
    RgbMode::Lottery,
    RgbMode::Intertwine,
    RgbMode::MeteorShower,
    RgbMode::Collide,
    RgbMode::ElectricCurrent,
    RgbMode::Kaleidoscope,
    RgbMode::Twinkle,
];

use super::Animation;

pub fn render(regions: &[RgbRegionConfig], fans: usize) -> Result<Animation> {
    ensure!((1..=4).contains(&fans), "TL fan count must be 1..=4");
    let mut top = None;
    let mut bottom = None;
    for region in regions {
        match region.effect.scope {
            RgbScope::All => {
                top = Some(region);
                bottom = Some(region);
            }
            RgbScope::Top => top = Some(region),
            RgbScope::Bottom => bottom = Some(region),
            _ => bail!("TL effects support All, Top and Bottom regions"),
        }
    }
    let off = RgbRegionConfig {
        effect: RgbEffect {
            mode: RgbMode::Off,
            ..Default::default()
        },
        flip: regions.last().is_some_and(|r| r.flip),
    };
    let top = top.unwrap_or(&off);
    let bottom = bottom.unwrap_or(&off);
    ensure!(
        top.flip == bottom.flip,
        "TL region orientation must agree across the group"
    );
    let speed = regions.last().map_or(2, |r| r.effect.speed);
    ensure!(speed <= 4, "RGB speed must be 0..=4");
    let interval = u32::from(7 - speed)
        * if top.effect.mode == RgbMode::Collide {
            16
        } else {
            11
        };
    let top_frames = render_region(top, fans, false)?;
    let bottom_frames = render_region(bottom, fans, true)?;
    Ok(Animation {
        frames: if top.flip {
            composition::combine(bottom_frames, top_frames, interval)?
        } else {
            composition::combine(top_frames, bottom_frames, interval)?
        },
        interval_hundredths: interval * 100,
        secondary: None,
    })
}

fn render_region(region: &RgbRegionConfig, fans: usize, bottom: bool) -> Result<Vec<Vec<[u8; 3]>>> {
    let effect = &region.effect;
    ensure!(
        effect.colors.len() <= 4,
        "TL palette supports at most four colors"
    );
    ensure!(
        effect.brightness <= 4 || is_brightness_off(effect.brightness),
        "invalid RGB brightness"
    );
    if effect.disabled || is_brightness_off(effect.brightness) || effect.mode == RgbMode::Off {
        return Ok(vec![vec![[0; 3]; fans * 26]; 30]);
    }
    let prepared = palette::prepare(effect, bottom);
    let effect = &prepared;
    let mut frames = match effect.mode {
        RgbMode::Static => solid::render(effect, fans, bottom),
        RgbMode::Rainbow => rainbow::render(effect, fans, bottom),
        RgbMode::RainbowMorph => morph::render(effect, fans, bottom),
        RgbMode::Breathing => breathing::render(effect, fans, bottom),
        RgbMode::Runway => runway::render(effect, fans, bottom),
        RgbMode::Meteor => meteor::render(effect, fans, bottom),
        RgbMode::ColorCycle => cycle::render(effect, fans, bottom),
        RgbMode::Wave => wave::render(effect, fans, bottom),
        RgbMode::Tide => tide::render(effect, fans, bottom),
        RgbMode::PingPong => ping_pong::render(effect, fans, bottom),
        RgbMode::Stack => stack::render(effect, fans, bottom),
        RgbMode::Twinkle => twinkle::render(effect, fans, bottom),
        RgbMode::Staggered => staggered::render(effect, fans, bottom),
        RgbMode::Mixing => mixing::render(effect, fans, bottom),
        RgbMode::Racing => racing::render(effect, fans, bottom),
        RgbMode::Lottery => lottery::render(effect, fans, bottom),
        RgbMode::Intertwine => intertwine::render(effect, fans, bottom),
        RgbMode::Voice => voice::render(effect, fans, bottom),
        RgbMode::Collide | RgbMode::ElectricCurrent => collision::render(effect, fans, bottom),
        RgbMode::Kaleidoscope => kaleidoscope::render(effect, fans, bottom),
        RgbMode::MeteorShower => meteor_shower::render(effect, fans, bottom),
        RgbMode::Door => door::render(effect, fans, bottom),
        RgbMode::Render => render::render(effect, fans, bottom),
        RgbMode::Ripple => ripple::render(effect, fans, bottom),
        RgbMode::Reflect => reflect::render(effect, fans, bottom),
        RgbMode::TailChasing => chase::render(effect, fans, bottom),
        RgbMode::Paint => paint::render(effect, fans, bottom),
        RgbMode::CoverCycle => cover_cycle::render(effect, fans, bottom),
        _ => bail!("TL effect {:?} has no verified renderer", effect.mode),
    }?;
    if region.flip {
        for frame in &mut frames {
            for fan in frame.chunks_exact_mut(26) {
                let (top, bottom) = fan.split_at_mut(13);
                top.swap_with_slice(bottom);
            }
        }
    }
    Ok(frames)
}

#[cfg(test)]
mod region_tests {
    use super::*;

    fn hash_byte(hash: u64, value: u8) -> u64 {
        (hash ^ u64::from(value)).wrapping_mul(1_099_511_628_211)
    }

    fn hash_i32(mut hash: u64, value: usize) -> u64 {
        for shift in (0..32).step_by(8) {
            hash = hash_byte(hash, (value >> shift) as u8);
        }
        hash
    }

    #[test]
    fn flipped_single_region_moves_to_the_other_physical_side() {
        let animation = render(
            &[RgbRegionConfig {
                effect: RgbEffect {
                    mode: RgbMode::Static,
                    scope: RgbScope::Top,
                    colors: vec![[255, 0, 0]],
                    ..Default::default()
                },
                flip: true,
            }],
            1,
        )
        .unwrap();
        assert_eq!(&animation.frames[0][..13], &[[0; 3]; 13]);
        assert_eq!(&animation.frames[0][13..], &[[254, 0, 0]; 13]);
    }

    #[test]
    fn deterministic_witmod_modes_match_the_extracted_csharp_matrix() {
        const CASES: [(RgbMode, u64); 17] = [
            (RgbMode::Staggered, 0x379e0b5aafd0d81d),
            (RgbMode::Tide, 0xc1c8811a9b243ed1),
            (RgbMode::Mixing, 0x7e72696ae2447e31),
            (RgbMode::Door, 0x9e9d30b288a04429),
            (RgbMode::Render, 0x3effa1d3ad36d495),
            (RgbMode::Ripple, 0x2003ea78672683ad),
            (RgbMode::Reflect, 0x554c4b668a27187b),
            (RgbMode::TailChasing, 0x298b07128d9471e8),
            (RgbMode::Paint, 0x65c96e828ba9c171),
            (RgbMode::PingPong, 0xdc7529c20dcfca39),
            (RgbMode::CoverCycle, 0xa55f1a1008a187c9),
            (RgbMode::Wave, 0x02ba5a0adc6b1a86),
            (RgbMode::Racing, 0x3764ecaa3f696655),
            (RgbMode::Lottery, 0x6f8c495207630190),
            (RgbMode::Intertwine, 0xed5abb6faf4a786d),
            (RgbMode::Collide, 0x68927cf133051f99),
            (RgbMode::ElectricCurrent, 0x523fc61e9c9acc7d),
        ];
        let colors = vec![[255, 1, 127], [5, 200, 17], [90, 33, 240], [18, 77, 150]];
        let mut mismatches = Vec::new();
        for (mode, expected) in CASES {
            let mut hash = 14_695_981_039_346_656_037;
            for fans in 1..=4 {
                for brightness in 1..=4 {
                    for direction in 0..=1 {
                        for source_scope in 0..=1 {
                            let region = RgbRegionConfig {
                                effect: RgbEffect {
                                    mode,
                                    colors: colors.clone(),
                                    brightness,
                                    direction: if direction == 0 {
                                        lianli_shared::rgb::RgbDirection::Clockwise
                                    } else {
                                        lianli_shared::rgb::RgbDirection::CounterClockwise
                                    },
                                    scope: if source_scope == 0 {
                                        RgbScope::Bottom
                                    } else {
                                        RgbScope::Top
                                    },
                                    ..Default::default()
                                },
                                flip: false,
                            };
                            let frames = render_region(&region, fans, source_scope == 0).unwrap();
                            hash = hash_i32(hash, fans);
                            hash = hash_i32(hash, brightness as usize);
                            hash = hash_i32(hash, direction);
                            hash = hash_i32(hash, source_scope);
                            hash = hash_i32(hash, frames.len());
                            for color in frames.iter().flatten() {
                                for &channel in color {
                                    hash = hash_byte(hash, channel);
                                }
                            }
                        }
                    }
                }
            }
            if hash != expected {
                mismatches.push((mode, hash, expected));
            }
        }
        assert!(mismatches.is_empty(), "{mismatches:#x?}");
    }
}
mod collision;
#[cfg(test)]
mod collision_tests;
mod kaleidoscope;
#[cfg(test)]
mod kaleidoscope_tests;
mod meteor_shower;
#[cfg(test)]
mod meteor_shower_tests;
pub mod parameters;
mod random;
