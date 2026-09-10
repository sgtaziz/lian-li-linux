mod classic;
mod geometry;
mod paths;
mod twinkle;

use crate::rgb::Animation;
use anyhow::{bail, ensure, Result};
use lianli_shared::rgb::{is_brightness_off, RgbDirection, RgbEffect, RgbMode, RgbScope};

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
        led_count == geometry::TOTAL_LEDS,
        "HydroShift II OLED effects require exactly 45 LEDs"
    );
    ensure!(
        effect.scope == RgbScope::All,
        "HydroShift II OLED effects support the whole ring only"
    );
    ensure!(
        effect.colors.len() <= 6,
        "HydroShift II OLED palette supports at most six colors"
    );
    ensure!(
        effect.brightness <= 4 || is_brightness_off(effect.brightness),
        "invalid RGB brightness"
    );
    ensure!(effect.speed <= 4, "RGB speed must be 0..=4");

    let brightness = if is_brightness_off(effect.brightness) {
        0
    } else {
        [0, 48, 96, 192, 255][effect.brightness as usize]
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
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    let frames = if effect.disabled || effect.mode == RgbMode::Off {
        vec![geometry::frame()]
    } else {
        match effect.mode {
            RgbMode::Rainbow => classic::rainbow(brightness, reverse),
            RgbMode::Wave => classic::wave(variable_colors, brightness, reverse),
            RgbMode::Static => classic::solid(colors[0], brightness),
            RgbMode::Breathing => classic::breathing(colors[0], brightness),
            RgbMode::RainbowMorph => classic::morph(brightness),
            RgbMode::Paint => paths::paint(variable_colors, brightness),
            RgbMode::Runway => paths::runway(&colors, brightness),
            RgbMode::Tide => paths::tide(variable_colors, brightness, reverse),
            RgbMode::BlowUp => paths::blow_up(variable_colors, brightness),
            RgbMode::Meteor => paths::meteor(variable_colors, brightness, reverse),
            RgbMode::Snooker => paths::snooker(variable_colors, brightness),
            RgbMode::Mixing => paths::mixing(&colors, brightness),
            RgbMode::PingPong => paths::ping_pong(variable_colors, brightness),
            RgbMode::BulletStack => paths::bullet_stack(brightness, reverse),
            RgbMode::Twinkle => twinkle::render(brightness),
            RgbMode::River => paths::river(&colors, brightness, reverse),
            RgbMode::Hourglass => paths::hourglass(variable_colors, brightness),
            RgbMode::ElectricCurrent => paths::electric_current(variable_colors, brightness),
            RgbMode::RainbowWave => classic::rainbow_wave(brightness, reverse),
            _ => bail!("unsupported HydroShift II OLED RGB mode: {:?}", effect.mode),
        }
    };
    Ok(Animation {
        frames,
        interval_hundredths: 1_100 * [7, 6, 5, 4, 3][effect.speed as usize],
        secondary: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
    const EXPECTED: [u64; 19] = [
        0x11baf2285616488f,
        0xcc1f5cb104861e25,
        0x8a6a07162e1bd9e7,
        0x01259cfb7804a9b5,
        0x03cd2f5c0d20b001,
        0x8d2d673cdb27fcc5,
        0x6941d1001e9cdca5,
        0x0ab3ffe1d5e8d765,
        0xae71a6c2864b32ad,
        0xc299ac505f6a7dce,
        0xc6feab8fa71f59ad,
        0x611e5b9fb97c05a9,
        0x464055695653c3e5,
        0x03d22ae3f104d921,
        0x40882807396d7205,
        0x28ee3cfec870e48d,
        0x23676e4861d743b9,
        0xdcafbf99ce3e2441,
        0x9ad46bc6fcd28365,
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

    fn hash_matrix(mode: RgbMode) -> u64 {
        let mut hash = 0xcbf29ce484222325;
        for brightness in 0..=4 {
            for direction in [RgbDirection::Clockwise, RgbDirection::CounterClockwise] {
                let animation = render(&effect(mode, brightness, direction), 45).unwrap();
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
    fn every_effect_matches_recovered_hs2_source() {
        for (index, mode) in EFFECTS.into_iter().enumerate() {
            assert_eq!(hash_matrix(mode), EXPECTED[index], "{mode:?}");
        }
    }

    #[test]
    fn preserves_the_source_black_tail_in_the_45_led_transport_frame() {
        for mode in EFFECTS {
            let animation = render(&effect(mode, 4, RgbDirection::Clockwise), 45).unwrap();
            assert!(animation
                .frames
                .iter()
                .all(|frame| frame[geometry::ACTIVE_LEDS..] == [[0; 3]; 10]));
        }
    }

    #[test]
    fn uses_hs2_brightness_adjustment_and_rf_speed_ticks() {
        let level_one = render(&effect(RgbMode::Static, 1, RgbDirection::Clockwise), 45).unwrap();
        assert_eq!(level_one.frames[0][0], [47, 0, 23]);
        for (speed, expected) in [7_700, 6_600, 5_500, 4_400, 3_300].into_iter().enumerate() {
            let mut effect = effect(RgbMode::Rainbow, 4, RgbDirection::Clockwise);
            effect.speed = speed as u8;
            assert_eq!(render(&effect, 45).unwrap().interval_hundredths, expected);
        }
    }

    #[test]
    fn rejects_non_oled_geometry() {
        let effect = effect(RgbMode::Static, 4, RgbDirection::Clockwise);
        assert!(render(&effect, 35).is_err());
        assert!(render(&effect, 60).is_err());
    }

    #[test]
    fn variable_modes_cycle_only_the_two_supplied_colors() {
        let cases = [
            (RgbMode::Wave, 96),
            (RgbMode::Paint, 36),
            (RgbMode::Tide, 36),
            (RgbMode::BlowUp, 86),
            (RgbMode::Meteor, 46),
            (RgbMode::Snooker, 64),
            (RgbMode::PingPong, 76),
            (RgbMode::Hourglass, 194),
            (RgbMode::ElectricCurrent, 80),
        ];
        for (mode, expected_frames) in cases {
            let mut effect = effect(mode, 4, RgbDirection::Clockwise);
            effect.colors.truncate(2);
            assert_eq!(render(&effect, 45).unwrap().frames.len(), expected_frames);
        }
    }

    #[test]
    fn variable_palettes_preserve_black_and_handle_an_empty_list() {
        let mut effect = effect(RgbMode::Wave, 4, RgbDirection::Clockwise);
        effect.colors = vec![[0; 3], [255, 0, 0]];
        let frames = render(&effect, 45).unwrap().frames;
        assert!(frames[..48].iter().flatten().all(|&color| color == [0; 3]));
        assert!(frames[48..].iter().flatten().any(|&color| color != [0; 3]));

        effect.colors.clear();
        let frames = render(&effect, 45).unwrap().frames;
        assert_eq!(frames.len(), 48);
        assert!(frames.iter().flatten().all(|&color| color == [0; 3]));
    }

    #[test]
    fn fixed_role_modes_keep_the_compatibility_palette() {
        for (mode, expected_frames) in [
            (RgbMode::Runway, 44),
            (RgbMode::Mixing, 36),
            (RgbMode::River, 6),
        ] {
            let mut effect = effect(mode, 4, RgbDirection::Clockwise);
            effect.colors.truncate(1);
            let frames = render(&effect, 45).unwrap().frames;
            assert_eq!(frames.len(), expected_frames);
            assert!(frames.iter().flatten().any(|&color| color != [0; 3]));
        }
    }
}
