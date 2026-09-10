mod classic;
mod geometry;
mod paths;
mod tables;
mod twinkle;

use crate::rgb::Animation;
use anyhow::{bail, ensure, Result};
use lianli_shared::rgb::{is_brightness_off, RgbEffect, RgbMode, RgbScope};

pub const MODES: &[RgbMode] = &[
    RgbMode::Off,
    RgbMode::Rainbow,
    RgbMode::Wave,
    RgbMode::Static,
    RgbMode::Breathing,
    RgbMode::RainbowMorph,
    RgbMode::Paint,
    RgbMode::Runway,
    RgbMode::Tide,
    RgbMode::BlowUp,
    RgbMode::Meteor,
    RgbMode::Snooker,
    RgbMode::Mixing,
    RgbMode::PingPong,
    RgbMode::BulletStack,
    RgbMode::Twinkle,
    RgbMode::River,
    RgbMode::Hourglass,
    RgbMode::ElectricCurrent,
    RgbMode::RainbowWave,
];

pub fn render(effect: &RgbEffect, led_count: usize) -> Result<Animation> {
    ensure!(
        (60..=u8::MAX as usize).contains(&led_count),
        "Universal Screen effects require 60..=255 LEDs"
    );
    ensure!(
        effect.scope == RgbScope::All,
        "Universal Screen effects support the whole ring only"
    );
    ensure!(
        effect.colors.len() <= 6,
        "Universal Screen palette supports at most six colors"
    );
    ensure!(
        effect.brightness <= 4 || is_brightness_off(effect.brightness),
        "invalid RGB brightness"
    );
    ensure!(effect.speed <= 4, "RGB speed must be 0..=4");

    let brightness = if is_brightness_off(effect.brightness) {
        0
    } else {
        [0, 64, 128, 192, 255][effect.brightness as usize]
    };
    let colors = geometry::palette(effect);
    let variable_colors = geometry::variable_palette(
        effect,
        if matches!(effect.mode, RgbMode::Hourglass | RgbMode::ElectricCurrent) {
            4
        } else {
            6
        },
    );
    let reverse = matches!(
        effect.direction,
        lianli_shared::rgb::RgbDirection::CounterClockwise
    );
    let (frames, base_hundredths) = if effect.disabled || effect.mode == RgbMode::Off {
        (vec![vec![[0; 3]; led_count]], 1_100)
    } else {
        match effect.mode {
            RgbMode::Rainbow => (classic::rainbow(led_count, brightness, reverse), 1_650),
            RgbMode::Wave => (
                classic::wave(led_count, variable_colors, brightness, reverse),
                1_430,
            ),
            RgbMode::Static => (classic::solid(led_count, colors[0], brightness), 1_100),
            RgbMode::Breathing => (classic::breathing(led_count, colors[0], brightness), 1_100),
            RgbMode::RainbowMorph => (classic::morph(led_count, brightness), 1_100),
            RgbMode::Paint => (paths::paint(led_count, variable_colors, brightness), 1_100),
            RgbMode::Runway => (paths::runway(led_count, &colors, brightness), 1_100),
            RgbMode::Tide => (paths::tide(led_count, variable_colors, brightness), 1_100),
            RgbMode::BlowUp => (
                paths::blow_up(led_count, variable_colors, brightness),
                1_100,
            ),
            RgbMode::Meteor => (
                paths::meteor(led_count, variable_colors, brightness, reverse),
                1_100,
            ),
            RgbMode::Snooker => (
                paths::snooker(led_count, variable_colors, brightness),
                1_100,
            ),
            RgbMode::Mixing => (paths::mixing(led_count, &colors, brightness), 1_100),
            RgbMode::PingPong => (
                paths::ping_pong(led_count, variable_colors, brightness),
                1_100,
            ),
            RgbMode::BulletStack => (paths::bullet_stack(led_count, brightness, reverse), 1_100),
            RgbMode::Twinkle => (twinkle::render(led_count, brightness), 1_100),
            RgbMode::River => (paths::river(led_count, &colors, brightness, reverse), 1_430),
            RgbMode::Hourglass => (
                tables::hourglass(led_count, variable_colors, brightness),
                1_100,
            ),
            RgbMode::ElectricCurrent => (
                tables::electric_current(led_count, variable_colors, brightness),
                1_100,
            ),
            RgbMode::RainbowWave => (paths::rainbow_wave(led_count, brightness, reverse), 1_650),
            _ => bail!("unsupported Universal Screen RGB mode: {:?}", effect.mode),
        }
    };
    let speed_multiplier = [7, 6, 5, 4, 3][effect.speed as usize];
    Ok(Animation {
        frames,
        interval_hundredths: base_hundredths * speed_multiplier,
        secondary: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_shared::rgb::RgbDirection;

    const EFFECTS: [RgbMode; 19] = [
        RgbMode::Rainbow,
        RgbMode::Wave,
        RgbMode::Static,
        RgbMode::Breathing,
        RgbMode::RainbowMorph,
        RgbMode::Paint,
        RgbMode::Runway,
        RgbMode::Tide,
        RgbMode::BlowUp,
        RgbMode::Meteor,
        RgbMode::Snooker,
        RgbMode::Mixing,
        RgbMode::PingPong,
        RgbMode::BulletStack,
        RgbMode::Twinkle,
        RgbMode::River,
        RgbMode::Hourglass,
        RgbMode::ElectricCurrent,
        RgbMode::RainbowWave,
    ];
    const EXPECTED_60: [u64; 19] = [
        0xc9076775c7833bc5,
        0x78857da1d2d75b35,
        0x4447b92054dc6d05,
        0xe34e4439fb417e65,
        0xce5cdf3a32edd2ad,
        0xd53f7e7f394e36bd,
        0xc579e111c5cc21a5,
        0x70aaadf96af06055,
        0x1287915932718aa9,
        0x838de01d63efa915,
        0xc872667339822e05,
        0xa5526faf6c4be215,
        0x116a700f823bea6d,
        0xa3b5f9b33c1341d9,
        0x54ae5769d308f49d,
        0x14730a3619dc4f55,
        0x9d74599ad9dd3c6d,
        0xa030d399a09414d5,
        0x220cd6ee6b300d87,
    ];
    const EXPECTED_88: [u64; 19] = [
        0x278e510c561f0a05,
        0xd685e4f035bc6df5,
        0xaa9098b098e87395,
        0xe4156dd33062f1a5,
        0x3188c993d5009a7d,
        0x5c40473ab591ca3d,
        0xcbd1a48a45a0e3a5,
        0x0501c8c7c600d455,
        0x2ac9166d6f1dde09,
        0x64117fd98d4e06b5,
        0xdfa4703d57f9d745,
        0x6636b1207eac2155,
        0xe190f4f668a046ed,
        0x1bd6b649fda600b9,
        0x518299556801191d,
        0xcaa3ed9ef381dd95,
        0x4ba1f7830e0eb0ed,
        0xfb00b79725035e95,
        0x4d03aae82131e807,
    ];

    fn effect(mode: RgbMode, brightness: u8, direction: RgbDirection) -> RgbEffect {
        RgbEffect {
            mode,
            colors: vec![
                [255, 1, 127],
                [5, 200, 17],
                [90, 33, 240],
                [18, 77, 150],
                [44, 222, 111],
                [199, 88, 3],
            ],
            brightness,
            direction,
            ..RgbEffect::default()
        }
    }

    fn hash_byte(hash: u64, byte: u8) -> u64 {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    }

    fn hash_matrix(mode: RgbMode, led_count: usize) -> u64 {
        let mut hash = 0xcbf29ce484222325;
        for brightness in 0..=4 {
            for direction in [RgbDirection::Clockwise, RgbDirection::CounterClockwise] {
                let animation = render(&effect(mode, brightness, direction), led_count).unwrap();
                for byte in (animation.frames.len() as i32).to_le_bytes() {
                    hash = hash_byte(hash, byte);
                }
                for channel in animation.frames.iter().flatten().flatten() {
                    hash = hash_byte(hash, *channel);
                }
            }
        }
        hash
    }

    #[test]
    fn every_effect_matches_recovered_source_at_60_and_88_leds() {
        for (index, mode) in EFFECTS.into_iter().enumerate() {
            assert_eq!(hash_matrix(mode, 60), EXPECTED_60[index], "60 LED {mode:?}");
            assert_eq!(hash_matrix(mode, 88), EXPECTED_88[index], "88 LED {mode:?}");
        }
    }

    #[test]
    fn intervals_use_recovered_rf_tick_multipliers() {
        let mut effect = effect(RgbMode::Rainbow, 4, RgbDirection::Clockwise);
        for (speed, expected) in [11_550, 9_900, 8_250, 6_600, 4_950].into_iter().enumerate() {
            effect.speed = speed as u8;
            assert_eq!(render(&effect, 60).unwrap().interval_hundredths, expected);
        }
    }

    #[test]
    fn rejects_the_unsupported_45_led_geometry() {
        let effect = effect(RgbMode::Static, 4, RgbDirection::Clockwise);
        assert!(render(&effect, 45).is_err());
    }

    #[test]
    fn variable_modes_cycle_only_the_two_supplied_colors() {
        let cases = [
            (RgbMode::Wave, 96),
            (RgbMode::Paint, 62),
            (RgbMode::Tide, 32),
            (RgbMode::BlowUp, 102),
            (RgbMode::Meteor, 60),
            (RgbMode::Snooker, 56),
            (RgbMode::PingPong, 116),
            (RgbMode::Hourglass, 140),
            (RgbMode::ElectricCurrent, 90),
        ];
        for (mode, expected_frames) in cases {
            let mut effect = effect(mode, 4, RgbDirection::Clockwise);
            effect.colors.truncate(2);
            assert_eq!(render(&effect, 60).unwrap().frames.len(), expected_frames);
        }
    }

    #[test]
    fn variable_palettes_preserve_black_and_handle_an_empty_list() {
        let mut effect = effect(RgbMode::Wave, 4, RgbDirection::Clockwise);
        effect.colors = vec![[0; 3], [255, 0, 0]];
        let frames = render(&effect, 60).unwrap().frames;
        assert!(frames[..48].iter().flatten().all(|&color| color == [0; 3]));
        assert!(frames[48..].iter().flatten().any(|&color| color != [0; 3]));

        effect.colors.clear();
        let frames = render(&effect, 60).unwrap().frames;
        assert_eq!(frames.len(), 48);
        assert!(frames.iter().flatten().all(|&color| color == [0; 3]));
    }

    #[test]
    fn fixed_role_modes_keep_the_compatibility_palette() {
        for (mode, expected_frames) in [
            (RgbMode::Runway, 70),
            (RgbMode::Mixing, 32),
            (RgbMode::River, 6),
        ] {
            let mut effect = effect(mode, 4, RgbDirection::Clockwise);
            effect.colors.truncate(1);
            let frames = render(&effect, 60).unwrap().frames;
            assert_eq!(frames.len(), expected_frames);
            assert!(frames.iter().flatten().any(|&color| color != [0; 3]));
        }
    }
}
