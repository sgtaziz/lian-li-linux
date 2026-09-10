mod engine;
mod modes_16_22;
mod modes_1_6;
mod modes_7_12;
#[cfg(test)]
mod reference_tests;

use super::Animation;
use anyhow::{bail, ensure, Result};
use lianli_shared::rgb::{RgbDirection, RgbEffect, RgbEffectParameters, RgbMode};

pub const MODES: &[RgbMode] = &[
    RgbMode::Rainbow,
    RgbMode::RainbowMorph,
    RgbMode::Static,
    RgbMode::Breathing,
    RgbMode::Runway,
    RgbMode::Meteor,
    RgbMode::Stack,
    RgbMode::Twinkle,
    RgbMode::ColorCycle,
    RgbMode::CoverCycle,
    RgbMode::Wave,
    RgbMode::MeteorShower,
    RgbMode::Disco,
    RgbMode::BlowUp,
    RgbMode::HeartBeat,
    RgbMode::Warning,
    RgbMode::SeaFlow,
    RgbMode::Ripple,
    RgbMode::Echo,
];

pub fn parameters() -> Vec<RgbEffectParameters> {
    MODES
        .iter()
        .map(|&mode| {
            let colors = match mode {
                RgbMode::Rainbow | RgbMode::RainbowMorph | RgbMode::Twinkle => 0,
                RgbMode::Static
                | RgbMode::Breathing
                | RgbMode::Meteor
                | RgbMode::Stack
                | RgbMode::Wave
                | RgbMode::SeaFlow => 1,
                RgbMode::Runway
                | RgbMode::CoverCycle
                | RgbMode::BlowUp
                | RgbMode::Warning
                | RgbMode::Ripple
                | RgbMode::Echo => 2,
                RgbMode::ColorCycle | RgbMode::HeartBeat => 3,
                RgbMode::MeteorShower | RgbMode::Disco => 4,
                _ => unreachable!(),
            };
            let supports_direction = matches!(
                mode,
                RgbMode::Rainbow
                    | RgbMode::Meteor
                    | RgbMode::Stack
                    | RgbMode::ColorCycle
                    | RgbMode::CoverCycle
                    | RgbMode::Wave
                    | RgbMode::MeteorShower
                    | RgbMode::Disco
                    | RgbMode::SeaFlow
            );
            RgbEffectParameters {
                mode,
                min_colors: colors,
                max_colors: colors,
                per_fan_colors: false,
                directions: if supports_direction {
                    vec![RgbDirection::Clockwise, RgbDirection::CounterClockwise]
                } else {
                    vec![]
                },
                supports_speed: mode != RgbMode::Static,
            }
        })
        .collect()
}

pub fn render(effect: &RgbEffect, logical_leds: usize) -> Result<Animation> {
    engine::validate(effect, logical_leds)?;
    ensure!(
        MODES.contains(&effect.mode),
        "unsupported sync lighting effect"
    );
    if matches!(effect.mode, RgbMode::RainbowMorph | RgbMode::Twinkle) {
        bail!(
            "sync lighting {:?} uses the device-native special path",
            effect.mode
        );
    }
    if effect.mode == RgbMode::Stack {
        ensure!(
            modes_7_12::stack_frame_count(logical_leds)
                <= lianli_shared::rgb::MAX_RGB_ANIMATION_FRAMES,
            "sync lighting Stack exceeds the source 4096-frame buffer"
        );
    }

    let base_interval = if matches!(
        effect.mode,
        RgbMode::HeartBeat | RgbMode::Warning | RgbMode::SeaFlow | RgbMode::Ripple
    ) {
        18
    } else {
        12
    };
    let interval_hundredths = base_interval * [7, 6, 5, 4, 3][effect.speed as usize] * 100;
    if effect.disabled {
        return Ok(Animation {
            frames: vec![vec![[0; 3]; logical_leds]],
            interval_hundredths,
            secondary: None,
        });
    }

    let colors = engine::palette(effect);
    let bright = engine::brightness(effect);
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    let frames = match effect.mode {
        RgbMode::Rainbow => modes_1_6::rainbow(logical_leds, bright, reverse),
        RgbMode::Static => modes_1_6::static_color(logical_leds, colors[0], bright),
        RgbMode::Breathing => modes_1_6::breathing(logical_leds, colors[0], bright),
        RgbMode::Runway => modes_1_6::runway(logical_leds, &colors, bright),
        RgbMode::Meteor => modes_1_6::meteor(logical_leds, colors[0], bright, reverse),
        RgbMode::Stack => modes_7_12::stack(logical_leds, colors[0], bright, reverse),
        RgbMode::ColorCycle => modes_7_12::color_cycle(logical_leds, &colors, bright, reverse),
        RgbMode::CoverCycle => modes_7_12::cover_cycle(logical_leds, &colors, bright, reverse),
        RgbMode::Wave => modes_7_12::wave(logical_leds, colors[0], bright, reverse),
        RgbMode::MeteorShower => modes_7_12::meteor_shower(logical_leds, &colors, bright, reverse),
        RgbMode::Disco => modes_16_22::disco(logical_leds, &colors, bright, reverse),
        RgbMode::BlowUp => modes_16_22::blow_up(logical_leds, &colors, bright),
        RgbMode::HeartBeat => modes_16_22::heart_beat(logical_leds, &colors, bright),
        RgbMode::Warning => modes_16_22::warning(logical_leds, &colors, bright),
        RgbMode::SeaFlow => modes_16_22::sea_flow(logical_leds, colors[0], bright, reverse),
        RgbMode::Ripple => modes_16_22::ripple(logical_leds, &colors, bright),
        RgbMode::Echo => modes_16_22::echo(logical_leds, &colors, bright),
        _ => unreachable!(),
    };
    Ok(Animation {
        frames,
        interval_hundredths,
        secondary: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn effect(mode: RgbMode) -> RgbEffect {
        RgbEffect {
            mode,
            colors: vec![[255, 1, 127], [5, 200, 17], [90, 33, 240], [18, 77, 150]],
            brightness: 4,
            speed: 2,
            direction: RgbDirection::Clockwise,
            ..RgbEffect::default()
        }
    }

    #[test]
    fn catalog_matches_the_merge_lighting_profile() {
        let parameters = parameters();
        assert_eq!(parameters.len(), 19);
        assert_eq!(
            parameters
                .iter()
                .find(|p| p.mode == RgbMode::Static)
                .unwrap()
                .max_colors,
            1
        );
        assert_eq!(
            parameters
                .iter()
                .find(|p| p.mode == RgbMode::Disco)
                .unwrap()
                .max_colors,
            4
        );
        assert!(!parameters
            .iter()
            .find(|p| p.mode == RgbMode::Wave)
            .unwrap()
            .directions
            .is_empty());
        assert_eq!(
            parameters
                .iter()
                .find(|p| p.mode == RgbMode::HeartBeat)
                .unwrap()
                .max_colors,
            3
        );
        assert_eq!(
            parameters
                .iter()
                .find(|p| p.mode == RgbMode::SeaFlow)
                .unwrap()
                .max_colors,
            1
        );
        for mode in [
            RgbMode::BlowUp,
            RgbMode::HeartBeat,
            RgbMode::Warning,
            RgbMode::Ripple,
            RgbMode::Echo,
        ] {
            assert!(parameters
                .iter()
                .find(|p| p.mode == mode)
                .unwrap()
                .directions
                .is_empty());
        }
        assert!(
            !parameters
                .iter()
                .find(|p| p.mode == RgbMode::Static)
                .unwrap()
                .supports_speed
        );
    }

    #[test]
    fn native_special_modes_are_explicitly_deferred() {
        for mode in [RgbMode::RainbowMorph, RgbMode::Twinkle] {
            let error = render(&effect(mode), 24).unwrap_err().to_string();
            assert!(error.contains("device-native special path"));
        }
    }

    #[test]
    fn rejects_out_of_range_geometry_and_palette() {
        assert!(render(&effect(RgbMode::Rainbow), 23).is_err());
        assert!(render(&effect(RgbMode::Rainbow), 721).is_err());
        let mut effect = effect(RgbMode::Static);
        effect.colors = vec![[1; 3]; 7];
        assert!(render(&effect, 24).is_err());
    }

    #[test]
    fn stack_rejects_the_first_geometry_beyond_the_source_frame_buffer() {
        assert_eq!(
            render(&effect(RgbMode::Stack), 307).unwrap().frames.len(),
            4_082
        );
        assert!(render(&effect(RgbMode::Stack), 308)
            .unwrap_err()
            .to_string()
            .contains("4096-frame buffer"));
    }

    #[test]
    fn frame_counts_match_the_extracted_ui_generators() {
        let cases = [
            (RgbMode::Rainbow, 24, 24),
            (RgbMode::Static, 1, 1),
            (RgbMode::Breathing, 170, 170),
            (RgbMode::Runway, 70, 72),
            (RgbMode::Meteor, 36, 37),
            (RgbMode::Stack, 36, 39),
            (RgbMode::ColorCycle, 48, 48),
            (RgbMode::CoverCycle, 48, 50),
            (RgbMode::Wave, 24, 24),
            (RgbMode::MeteorShower, 48, 48),
            (RgbMode::Disco, 12, 12),
            (RgbMode::BlowUp, 47, 48),
            (RgbMode::HeartBeat, 48, 50),
            (RgbMode::Warning, 24, 28),
            (RgbMode::SeaFlow, 96, 98),
            (RgbMode::Ripple, 82, 84),
            (RgbMode::Echo, 148, 148),
        ];
        for (mode, at_24, at_25) in cases {
            assert_eq!(
                render(&effect(mode), 24).unwrap().frames.len(),
                at_24,
                "{mode:?} at 24"
            );
            assert_eq!(
                render(&effect(mode), 25).unwrap().frames.len(),
                at_25,
                "{mode:?} at 25"
            );
        }
    }

    #[test]
    fn slow_effects_use_the_recovered_eighteen_tick_base() {
        for mode in [
            RgbMode::HeartBeat,
            RgbMode::Warning,
            RgbMode::SeaFlow,
            RgbMode::Ripple,
        ] {
            assert_eq!(
                render(&effect(mode), 24).unwrap().interval_hundredths,
                9_000
            );
        }
        assert_eq!(
            render(&effect(RgbMode::Echo), 24)
                .unwrap()
                .interval_hundredths,
            6_000
        );
    }

    #[test]
    fn recovered_source_boundaries_keep_direction_and_phase_order() {
        let clockwise = render(&effect(RgbMode::Rainbow), 24).unwrap();
        assert_eq!(clockwise.frames[0][0], [223, 0, 31]);
        let mut counter = effect(RgbMode::Rainbow);
        counter.direction = RgbDirection::CounterClockwise;
        assert_eq!(render(&counter, 24).unwrap().frames[0][0], [254, 0, 0]);

        let runway = render(&effect(RgbMode::Runway), 24).unwrap();
        assert_eq!(runway.frames[0][0], [254, 0, 126]);
        assert_eq!(runway.frames[0][1], [4, 199, 16]);
        assert!(render(&effect(RgbMode::Meteor), 24).unwrap().frames[0]
            .iter()
            .all(|&color| color == [0; 3]));

        let heart = render(&effect(RgbMode::HeartBeat), 24).unwrap();
        assert_eq!(heart.frames[0][0], [4, 199, 16]);
        assert_eq!(heart.frames[0][1], [254, 0, 126]);
        let echo = render(&effect(RgbMode::Echo), 24).unwrap();
        assert!(echo.frames[..8]
            .iter()
            .flatten()
            .all(|&color| color == [0; 3]));
        assert_eq!(echo.frames[8][0], [254, 0, 126]);
    }

    #[test]
    fn palette_current_limit_uses_repeated_source_rounding() {
        let mut effect = effect(RgbMode::Static);
        effect.colors = vec![[255; 3]];
        assert_eq!(render(&effect, 24).unwrap().frames[0][0], [194; 3]);
    }
}
