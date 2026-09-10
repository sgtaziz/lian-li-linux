mod classic;
mod composition;
mod engine;
#[cfg(test)]
mod native_tests;
mod paths;
mod twinkle;

use crate::rgb::Animation;
use anyhow::{bail, ensure, Result};
use lianli_shared::rgb::{is_brightness_off, RgbDirection, RgbEffect, RgbMode, RgbScope};

pub const MODES: &[RgbMode] = &[
    RgbMode::Off,
    RgbMode::Rainbow,
    RgbMode::RainbowMorph,
    RgbMode::Static,
    RgbMode::Breathing,
    RgbMode::Runway,
    RgbMode::Meteor,
    RgbMode::TaiChi,
    RgbMode::Twinkle,
    RgbMode::Voice,
    RgbMode::Pump,
    RgbMode::Bounce,
];

pub use composition::render_regions;

pub fn modes_for_scope(scope: RgbScope) -> &'static [RgbMode] {
    match scope {
        RgbScope::Pump => MODES,
        RgbScope::Center | RgbScope::Outer => crate::rgb::cl::MODES,
        _ => &[],
    }
}

pub fn render(effect: &RgbEffect, led_count: usize) -> Result<Animation> {
    ensure!(
        led_count == 24,
        "HydroShift II pump effects require exactly 24 LEDs"
    );
    ensure!(
        effect.scope == RgbScope::All,
        "HydroShift II pump effects support the whole ring only"
    );
    ensure!(
        effect.colors.len() <= 4,
        "HydroShift II pump palette supports at most four colors"
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
    let colors = engine::palette(effect);
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    let (frames, base_hundredths) = if effect.disabled || effect.mode == RgbMode::Off {
        (vec![engine::frame()], 2_000)
    } else {
        match effect.mode {
            RgbMode::Rainbow => (classic::rainbow(brightness, reverse), 2_000),
            RgbMode::RainbowMorph => (classic::morph(brightness), 1_100),
            RgbMode::Static => (classic::solid(colors[0], brightness), 2_000),
            RgbMode::Breathing => (classic::breathing(colors[0], brightness), 1_100),
            RgbMode::Runway => (paths::runway(&colors, brightness), 2_000),
            RgbMode::Meteor => (paths::meteor(&colors, brightness, reverse), 2_000),
            RgbMode::TaiChi => (paths::tai_chi(&colors, brightness, reverse), 2_000),
            RgbMode::Twinkle => (twinkle::render(brightness), 2_000),
            RgbMode::Voice => (paths::voice(colors[0], brightness, reverse), 2_000),
            RgbMode::Pump => (paths::pump(colors[0], brightness, reverse), 2_000),
            RgbMode::Bounce => (paths::bounce(&colors, brightness), 2_000),
            _ => bail!("unsupported HydroShift II pump RGB mode: {:?}", effect.mode),
        }
    };
    Ok(Animation {
        frames,
        interval_hundredths: base_hundredths * [7, 6, 5, 4, 3][effect.speed as usize],
        secondary: None,
    })
}
