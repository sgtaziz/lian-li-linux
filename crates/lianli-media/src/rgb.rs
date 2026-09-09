use anyhow::{ensure, Result};
use lianli_shared::rgb::{is_brightness_off, RgbDirection, RgbEffect, RgbMode, RgbScope};

pub const SOFTWARE_MODES: &[RgbMode] = &[
    RgbMode::Off,
    RgbMode::Static,
    RgbMode::Rainbow,
    RgbMode::RainbowMorph,
    RgbMode::Breathing,
    RgbMode::ColorCycle,
    RgbMode::Runway,
    RgbMode::Meteor,
    RgbMode::Wave,
    RgbMode::Tide,
    RgbMode::PingPong,
    RgbMode::Twinkle,
];

pub const LOOP_FRAMES: usize = 120;
pub const FRAME_INTERVAL_MS: u16 = 50;

pub fn validate_effect(effect: &RgbEffect) -> Result<()> {
    ensure!(
        SOFTWARE_MODES.contains(&effect.mode),
        "unsupported software RGB mode: {:?}",
        effect.mode
    );
    ensure!(
        effect.scope == RgbScope::All,
        "software RGB supports the whole zone only"
    );
    ensure!(
        effect.colors.len() <= 4,
        "RGB palette supports at most four colors"
    );
    Ok(())
}

pub fn is_animated(effect: &RgbEffect) -> bool {
    !effect.disabled
        && !is_brightness_off(effect.brightness)
        && !matches!(
            effect.mode,
            RgbMode::Off | RgbMode::Static | RgbMode::Direct
        )
}

/// `frame` is a sample in a six-second loop; integer speed multipliers keep
/// independently configured zones seamless in the same device animation.
pub fn render_zone(effect: &RgbEffect, frame: usize, leds: &mut [[u8; 3]]) {
    if effect.disabled || effect.mode == RgbMode::Off || is_brightness_off(effect.brightness) {
        leds.fill([0; 3]);
        return;
    }
    let cycles = [1, 2, 3, 4, 6][effect.speed.min(4) as usize];
    let phase = ((frame % LOOP_FRAMES) * cycles % LOOP_FRAMES) as f32 / LOOP_FRAMES as f32;
    let phase = match effect.direction {
        RgbDirection::CounterClockwise | RgbDirection::Down | RgbDirection::Gather => {
            (1.0 - phase) % 1.0
        }
        _ => phase,
    };
    let brightness = (effect.brightness.min(4) as f32 + 1.0) / 5.0;
    let count = leds.len().max(1) as f32;
    for (index, led) in leds.iter_mut().enumerate() {
        let mut position = index as f32 / count;
        if matches!(
            effect.direction,
            RgbDirection::Spread | RgbDirection::Gather
        ) {
            position = (position * 2.0 - 1.0).abs();
        }
        let travel = (position - phase).rem_euclid(1.0);
        let base = palette(effect, phase);
        let (color, intensity) = match effect.mode {
            RgbMode::Rainbow => (rainbow(travel), 1.0),
            RgbMode::RainbowMorph => (rainbow(phase), 1.0),
            RgbMode::ColorCycle => (base, 1.0),
            RgbMode::Breathing => {
                let p = phase * effect.colors.len().clamp(1, 4) as f32;
                (
                    palette_step(effect, p as usize),
                    1.0 - (p.fract() * 2.0 - 1.0).abs(),
                )
            }
            RgbMode::Runway => (base, if travel < 0.2 { 1.0 } else { 0.0 }),
            RgbMode::Meteor => (base, (1.0 - travel * 4.0).max(0.0).powi(2)),
            RgbMode::Wave => (palette(effect, travel), 1.0),
            RgbMode::Tide => (base, (1.0 - (position - phase).abs() * 3.0).max(0.0)),
            RgbMode::PingPong => (
                base,
                (1.0 - (position - (1.0 - (phase * 2.0 - 1.0).abs())).abs() * 6.0).max(0.0),
            ),
            RgbMode::Twinkle => {
                let offset = ((index * 37 + 11) % 101) as f32 / 101.0;
                let p = (phase + offset).fract();
                (
                    palette_step(effect, index),
                    (1.0 - (p * 2.0 - 1.0).abs()).powi(8),
                )
            }
            _ => (palette_step(effect, 0), 1.0),
        };
        *led = color.map(|c| (c as f32 * brightness * intensity).round() as u8);
    }
}

fn palette_step(effect: &RgbEffect, index: usize) -> [u8; 3] {
    if effect.colors.is_empty() {
        [255; 3]
    } else {
        effect.colors[index % effect.colors.len().min(4)]
    }
}

fn palette(effect: &RgbEffect, phase: f32) -> [u8; 3] {
    let p = phase * effect.colors.len().clamp(1, 4) as f32;
    blend(
        palette_step(effect, p as usize),
        palette_step(effect, p as usize + 1),
        p.fract(),
    )
}

fn blend(a: [u8; 3], b: [u8; 3], amount: f32) -> [u8; 3] {
    std::array::from_fn(|i| (a[i] as f32 * (1.0 - amount) + b[i] as f32 * amount).round() as u8)
}

fn rainbow(phase: f32) -> [u8; 3] {
    const COLORS: [[u8; 3]; 6] = [
        [255, 0, 0],
        [255, 255, 0],
        [0, 255, 0],
        [0, 255, 255],
        [0, 0, 255],
        [255, 0, 255],
    ];
    let p = phase.rem_euclid(1.0) * 6.0;
    blend(
        COLORS[p as usize % 6],
        COLORS[(p as usize + 1) % 6],
        p.fract(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brightness_and_off_have_distinct_outputs() {
        let mut effect = RgbEffect {
            colors: vec![[255, 100, 0]],
            brightness: 0,
            ..Default::default()
        };
        let mut leds = [[0; 3]; 9];
        render_zone(&effect, 0, &mut leds);
        assert_eq!(leds, [[51, 20, 0]; 9]);
        effect.brightness = 255;
        render_zone(&effect, 0, &mut leds);
        assert_eq!(leds, [[0; 3]; 9]);
        effect.brightness = 4;
        effect.disabled = true;
        render_zone(&effect, 0, &mut leds);
        assert_eq!(leds, [[0; 3]; 9]);
    }

    #[test]
    fn every_advertised_animation_changes_and_loops_at_every_speed() {
        for &mode in SOFTWARE_MODES
            .iter()
            .filter(|&&m| !matches!(m, RgbMode::Static | RgbMode::Off))
        {
            for speed in 0..=4 {
                let effect = RgbEffect {
                    mode,
                    speed,
                    colors: vec![[255, 0, 0], [0, 0, 255]],
                    ..Default::default()
                };
                let mut first = [[0; 3]; 44];
                render_zone(&effect, 0, &mut first);
                let mut current = first;
                assert!(
                    (1..LOOP_FRAMES).any(|frame| {
                        render_zone(&effect, frame, &mut current);
                        current != first
                    }),
                    "{mode:?} speed {speed}"
                );
                render_zone(&effect, LOOP_FRAMES, &mut current);
                assert_eq!(first, current);
            }
        }
    }

    #[test]
    fn direction_reverses_time_and_speed_advances_phase() {
        let mut effect = RgbEffect {
            mode: RgbMode::Meteor,
            speed: 0,
            ..Default::default()
        };
        let mut forward = [[0; 3]; 26];
        let mut reverse = forward;
        render_zone(&effect, 119, &mut forward);
        effect.direction = RgbDirection::CounterClockwise;
        render_zone(&effect, 1, &mut reverse);
        assert_eq!(forward, reverse);
        effect.direction = RgbDirection::Clockwise;
        render_zone(&effect, 6, &mut forward);
        effect.speed = 4;
        render_zone(&effect, 1, &mut reverse);
        assert_eq!(forward, reverse);
    }

    #[test]
    fn rejects_unsupported_effects_and_scopes() {
        assert!(validate_effect(&RgbEffect {
            mode: RgbMode::Voice,
            ..Default::default()
        })
        .is_err());
        assert!(validate_effect(&RgbEffect {
            scope: RgbScope::Inner,
            ..Default::default()
        })
        .is_err());
        render_zone(&RgbEffect::default(), usize::MAX, &mut []);
    }
}
