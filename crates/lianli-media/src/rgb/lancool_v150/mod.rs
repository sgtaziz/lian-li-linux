mod classic;
mod composition;
mod engine;
#[cfg(test)]
mod native_tests;
pub(crate) mod parameters;
mod paths;
mod paths_13_16;
mod paths_17_22;
mod paths_7_12;
mod twinkle;

use crate::rgb::Animation;
use anyhow::{bail, ensure, Result};
use composition::RegionAnimation;
use lianli_shared::rgb::{
    is_brightness_off, RgbDirection, RgbEffect, RgbMode, RgbRegionConfig, RgbScope,
};

pub const MODES: &[RgbMode] = &[
    RgbMode::Off,
    RgbMode::Rainbow,
    RgbMode::RainbowMorph,
    RgbMode::Static,
    RgbMode::Breathing,
    RgbMode::Runway,
    RgbMode::Meteor,
    RgbMode::ColorCycle,
    RgbMode::CoverCycle,
    RgbMode::Wave,
    RgbMode::MeteorShower,
    RgbMode::Twinkle,
    RgbMode::TaiChi,
    RgbMode::Warning,
    RgbMode::Mixing,
    RgbMode::Tide,
    RgbMode::DoubleMeteor,
    RgbMode::MeteorContest,
    RgbMode::ReturnArc,
    RgbMode::HeartBeat,
    RgbMode::HeartBeatRunway,
    RgbMode::Disco,
    RgbMode::CandyBox,
];
pub const SCOPES: &[RgbScope] = &[RgbScope::All, RgbScope::Front, RgbScope::Rear];

pub fn render(regions: &[RgbRegionConfig], led_count: usize) -> Result<Animation> {
    ensure!(
        led_count == 88,
        "LANCOOL V150 effects require exactly 88 LEDs"
    );
    ensure!(
        !regions.is_empty(),
        "LANCOOL V150 requires a lighting region"
    );
    let mut front = None;
    let mut rear = None;
    for region in regions {
        ensure!(
            !region.flip,
            "LANCOOL V150 does not support region flipping"
        );
        match region.effect.scope {
            RgbScope::All => {
                ensure!(
                    regions.len() == 1,
                    "All cannot be combined with side regions"
                );
                front = Some(render_region(&region.effect, RgbScope::Front)?);
                rear = Some(render_region(&region.effect, RgbScope::Rear)?);
            }
            RgbScope::Front => {
                ensure!(front.is_none(), "duplicate LANCOOL V150 front region");
                front = Some(render_region(&region.effect, RgbScope::Front)?);
            }
            RgbScope::Rear => {
                ensure!(rear.is_none(), "duplicate LANCOOL V150 rear region");
                rear = Some(render_region(&region.effect, RgbScope::Rear)?);
            }
            _ => bail!("LANCOOL V150 effects support All, Front and Rear regions"),
        }
    }
    match (front, rear) {
        (Some(front), Some(rear)) => Ok(composition::combine(front, rear)),
        (Some(region), None) | (None, Some(region)) => Ok(Animation {
            frames: region.frames,
            interval_hundredths: region.interval_hundredths,
            secondary: None,
        }),
        (None, None) => unreachable!("non-empty validated regions"),
    }
}

fn render_region(effect: &RgbEffect, scope: RgbScope) -> Result<RegionAnimation> {
    ensure!(
        MODES.contains(&effect.mode),
        "unsupported LANCOOL V150 RGB mode"
    );
    ensure!(
        effect.colors.len() <= 4,
        "LANCOOL V150 palettes support at most four colors"
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
    let palettes = engine::palettes(effect);
    let colors = palettes[0];
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    let (frames, base_ticks) = if effect.disabled || effect.mode == RgbMode::Off {
        (vec![engine::frame()], 11)
    } else {
        match effect.mode {
            RgbMode::Rainbow => (classic::rainbow(scope, brightness, reverse), 11),
            RgbMode::RainbowMorph => (classic::morph(scope, brightness), 11),
            RgbMode::Static => (classic::solid(scope, colors[0], brightness), 11),
            RgbMode::Breathing => (classic::breathing(scope, &colors, brightness), 11),
            RgbMode::Runway => (paths::runway(scope, &colors, brightness), 9),
            RgbMode::Meteor => (paths::meteor(scope, &colors, brightness, reverse), 9),
            RgbMode::ColorCycle => (
                paths_7_12::color_cycle(scope, &colors, brightness, reverse),
                15,
            ),
            RgbMode::CoverCycle => (
                paths_7_12::cover_cycle(scope, &colors, brightness, reverse),
                11,
            ),
            RgbMode::Wave => (paths_7_12::wave(scope, colors[0], brightness, reverse), 16),
            RgbMode::MeteorShower => (
                paths_7_12::meteor_shower(scope, &colors, brightness, reverse),
                11,
            ),
            RgbMode::Twinkle => (twinkle::render(&colors, brightness), 11),
            RgbMode::TaiChi => (
                paths_7_12::tai_chi(scope, &palettes, brightness, reverse),
                11,
            ),
            RgbMode::Warning => (paths_13_16::warning(&palettes, brightness), 16),
            RgbMode::Mixing => (paths_13_16::mixing(scope, &palettes, brightness), 14),
            RgbMode::Tide => (paths_13_16::tide(scope, &palettes, brightness), 14),
            RgbMode::DoubleMeteor => (paths_13_16::double_meteor(scope, &palettes, brightness), 14),
            RgbMode::MeteorContest => (
                paths_17_22::meteor_contest(scope, &palettes, brightness, reverse),
                11,
            ),
            RgbMode::ReturnArc => (
                paths_17_22::return_arc(scope, &palettes, brightness, reverse),
                11,
            ),
            RgbMode::HeartBeat => (paths_17_22::heartbeat(scope, &palettes, brightness), 11),
            RgbMode::HeartBeatRunway => (paths_17_22::heartbeat_runway(&palettes, brightness), 11),
            RgbMode::Disco => (
                paths_17_22::disco(scope, &palettes, brightness, reverse),
                14,
            ),
            RgbMode::CandyBox => (paths_17_22::candy_box(brightness), 11),
            _ => bail!("unsupported LANCOOL V150 RGB mode: {:?}", effect.mode),
        }
    };
    Ok(RegionAnimation {
        frames,
        interval_hundredths: base_ticks * [7, 6, 5, 4, 3][effect.speed as usize] * 100,
        mode: effect.mode,
    })
}
