use super::{render, render_region, render_whole, Side, SIDE_MODES};
use lianli_shared::rgb::{RgbDirection, RgbEffect, RgbMode, RgbRegionConfig, RgbScope};

fn effect(mode: RgbMode, scope: RgbScope, brightness: u8, direction: RgbDirection) -> RgbEffect {
    RgbEffect {
        mode,
        scope,
        brightness,
        direction,
        colors: vec![[211, 17, 67], [13, 199, 29], [31, 47, 173], [107, 83, 23]],
        ..Default::default()
    }
}

fn hash_animation(mut hash: u64, frames: &[Vec<[u8; 3]>]) -> u64 {
    for value in (frames.len() as u32).to_le_bytes() {
        hash = (hash ^ u64::from(value)).wrapping_mul(0x100000001b3);
    }
    for value in frames.iter().flatten().flatten() {
        hash = (hash ^ u64::from(*value)).wrapping_mul(0x100000001b3);
    }
    hash
}

#[test]
fn native_modes_match_the_extracted_csharp_matrix() {
    let cases: &[(RgbMode, u64)] = &[
        (RgbMode::Rainbow, 0x60c9e4821d854b95),
        (RgbMode::RainbowMorph, 0xd72bdce391514bbd),
        (RgbMode::Static, 0x84106faeabbeb07d),
        (RgbMode::Breathing, 0x4d48fe46f60851bd),
        (RgbMode::Runway, 0x5f9ba1121488baa5),
        (RgbMode::Meteor, 0x1db29d7580b156af),
        (RgbMode::ColorCycle, 0xb9d0c2ba077f59f9),
        (RgbMode::Staggered, 0x25bacea6f5939775),
        (RgbMode::Tide, 0x8d74d0ad6e4a9d89),
        (RgbMode::Mixing, 0x6bae83a19bb74c1d),
        (RgbMode::Render, 0xa3b54624bfd43da5),
        (RgbMode::PingPong, 0xdb3a0a5b13eaa2ed),
        (RgbMode::Stack, 0x9c61c4a385db54c5),
        (RgbMode::Ripple, 0x52c140ca22100de5),
        (RgbMode::Collide, 0x25a78632804dc955),
        (RgbMode::Reflect, 0x2b940a0a0ea3f5e5),
        (RgbMode::ElectricCurrent, 0x18c52727e53a6265),
        (RgbMode::Endless, 0x211b49d97165c9fd),
        (RgbMode::River, 0xa52f41726f38f325),
        (RgbMode::Duel, 0x1832d84577df6ec5),
        (RgbMode::Hourglass, 0x6203e57628718a7d),
        (RgbMode::Pioneer, 0x1f9d8a6dcdfa5ae5),
        (RgbMode::ShuttleRun, 0xf04f538c30e3f395),
        (RgbMode::GradientRibbon, 0x078c9e00d9998d25),
        (RgbMode::Twinkle, 0x8d8b07d3a6b9a035),
    ];
    for &(mode, expected) in cases {
        let mut hash = 0xcbf29ce484222325u64;
        for fans in 1..=4 {
            for brightness in 0..=4 {
                for direction in [RgbDirection::Clockwise, RgbDirection::CounterClockwise] {
                    for side in [Side::Outer, Side::Inner] {
                        let whole = matches!(
                            mode,
                            RgbMode::Endless
                                | RgbMode::Hourglass
                                | RgbMode::Pioneer
                                | RgbMode::GradientRibbon
                                | RgbMode::Twinkle
                        );
                        let scope = if whole || !SIDE_MODES.contains(&mode) {
                            RgbScope::All
                        } else if side == Side::Outer {
                            RgbScope::Outer
                        } else {
                            RgbScope::Inner
                        };
                        let effect = effect(mode, scope, brightness, direction);
                        let frames = if whole {
                            render_whole(&effect, fans).unwrap().frames
                        } else {
                            render_region(&effect, fans, side).unwrap().frames
                        };
                        for value in (frames.len() as u32).to_le_bytes() {
                            hash = (hash ^ u64::from(value)).wrapping_mul(0x100000001b3);
                        }
                        for value in frames.iter().flatten().flatten() {
                            hash = (hash ^ u64::from(*value)).wrapping_mul(0x100000001b3);
                        }
                    }
                }
            }
        }
        assert_eq!(hash, expected, "{mode:?}");
    }
}

#[test]
fn all_scope_composition_matches_the_extracted_csharp_matrix() {
    let modes = [
        RgbMode::Rainbow,
        RgbMode::RainbowMorph,
        RgbMode::Static,
        RgbMode::Breathing,
        RgbMode::Runway,
        RgbMode::Meteor,
        RgbMode::ColorCycle,
        RgbMode::Render,
        RgbMode::Stack,
        RgbMode::ElectricCurrent,
        RgbMode::River,
        RgbMode::Duel,
        RgbMode::Hourglass,
        RgbMode::ShuttleRun,
    ];
    let mut hash = 0xcbf29ce484222325u64;
    for mode in modes {
        for fans in 1..=4 {
            for brightness in 0..=4 {
                for direction in [RgbDirection::Clockwise, RgbDirection::CounterClockwise] {
                    let region = RgbRegionConfig {
                        effect: effect(mode, RgbScope::All, brightness, direction),
                        flip: false,
                    };
                    let animation = render(&[region], fans).unwrap();
                    hash = hash_animation(hash, &animation.frames);
                }
            }
        }
    }
    assert_eq!(hash, 0xed5734e04d97ce2b);
}

#[test]
fn special_all_dispatch_matches_the_extracted_csharp_matrix() {
    let mut hash = 0xcbf29ce484222325u64;
    for mode in [
        RgbMode::Endless,
        RgbMode::Pioneer,
        RgbMode::GradientRibbon,
        RgbMode::Twinkle,
    ] {
        for fans in 1..=4 {
            for brightness in 0..=4 {
                for direction in [RgbDirection::Clockwise, RgbDirection::CounterClockwise] {
                    let region = RgbRegionConfig {
                        effect: effect(mode, RgbScope::All, brightness, direction),
                        flip: false,
                    };
                    let animation = render(&[region], fans).unwrap();
                    hash = hash_animation(hash, &animation.frames);
                }
            }
        }
    }
    assert_eq!(hash, 0x3e9510cd06da8069);
}

#[test]
fn mixed_region_composition_matches_the_extracted_csharp_cases() {
    let cases = [
        (RgbMode::Stack, RgbMode::Runway, 3, 4, 2, 1, 0),
        (RgbMode::Static, RgbMode::Rainbow, 4, 1, 3, 0, 1),
        (RgbMode::PingPong, RgbMode::Mixing, 1, 3, 4, 1, 0),
        (RgbMode::ElectricCurrent, RgbMode::Ripple, 2, 2, 1, 0, 1),
    ];
    let mut hash = 0xcbf29ce484222325u64;
    for (
        case,
        (outer_mode, inner_mode, fans, outer_brightness, inner_brightness, outer_dir, inner_dir),
    ) in cases.into_iter().enumerate()
    {
        let direction = |value| {
            if value == 0 {
                RgbDirection::Clockwise
            } else {
                RgbDirection::CounterClockwise
            }
        };
        let inner = RgbRegionConfig {
            effect: effect(
                inner_mode,
                RgbScope::Inner,
                inner_brightness,
                direction(inner_dir),
            ),
            flip: false,
        };
        let outer_speed = case as u8;
        let outer = RgbRegionConfig {
            effect: RgbEffect {
                speed: outer_speed,
                ..effect(
                    outer_mode,
                    RgbScope::Outer,
                    outer_brightness,
                    direction(outer_dir),
                )
            },
            flip: false,
        };
        let animation = render(&[inner, outer], fans).unwrap();
        let interval_base = if outer_mode == RgbMode::Stack { 20 } else { 11 };
        assert_eq!(
            animation.interval_hundredths,
            u32::from(7 - outer_speed) * interval_base * 100
        );
        hash = hash_animation(hash, &animation.frames);
        let secondary = animation.secondary.unwrap();
        for value in u32::from(secondary.frame_count).to_le_bytes() {
            hash = (hash ^ u64::from(value)).wrapping_mul(0x100000001b3);
        }
        hash = (hash ^ u64::from(secondary.outer_longest)).wrapping_mul(0x100000001b3);
    }
    assert_eq!(hash, 0xe6333e7c3bd88821);
}
