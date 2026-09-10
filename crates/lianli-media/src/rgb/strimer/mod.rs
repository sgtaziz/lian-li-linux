mod classic;
mod engine;
mod mode_13;
mod mode_14;
mod mode_3;
mod mode_34;
mod mode_6;
mod mode_7;
mod mode_8;
mod modes_11_12;
mod modes_15_17;
mod modes_16_21;
mod modes_18_19;
mod modes_1_2;
mod modes_20_22;
mod modes_23_27;
mod modes_4_5;
mod modes_9_10;
#[cfg(test)]
mod native_tests;
pub(crate) mod parameters;

use anyhow::{bail, ensure, Result};
use engine::{frame, geometry, lane, Frame, Geometry};
use lianli_shared::rgb::{RgbDirection, RgbEffect, RgbMode, RgbRegionConfig, RgbScope};

use crate::rgb::Animation;

pub const MODES: &[RgbMode] = &[
    RgbMode::Off,
    RgbMode::ColorTransfer,
    RgbMode::FadeOut,
    RgbMode::Contest,
    RgbMode::CrossOver,
    RgbMode::BulletStack,
    RgbMode::Twinkle,
    RgbMode::Parallel,
    RgbMode::ShockWave,
    RgbMode::Ripple,
    RgbMode::Voice,
    RgbMode::Drizzling,
    RgbMode::Endless,
    RgbMode::ShuttleRun,
    RgbMode::River,
    RgbMode::Hourglass,
    RgbMode::Pioneer,
    RgbMode::ElectricCurrent,
    RgbMode::Transformation,
    RgbMode::GradientRibbon,
    RgbMode::RainbowWave,
    RgbMode::Snooker,
    RgbMode::Mixing,
    RgbMode::PingPong,
    RgbMode::Runway,
    RgbMode::Tide,
    RgbMode::BlowUp,
    RgbMode::Meteor,
    RgbMode::Stack,
    RgbMode::Rainbow,
    RgbMode::Wave,
    RgbMode::Static,
    RgbMode::Breathing,
    RgbMode::RainbowMorph,
    RgbMode::Paint,
];
pub const WHOLE_MODES: &[RgbMode] = &[
    RgbMode::Off,
    RgbMode::ColorTransfer,
    RgbMode::FadeOut,
    RgbMode::Contest,
    RgbMode::CrossOver,
    RgbMode::BulletStack,
    RgbMode::Twinkle,
    RgbMode::Parallel,
    RgbMode::ShockWave,
    RgbMode::Ripple,
    RgbMode::Voice,
    RgbMode::Drizzling,
    RgbMode::Endless,
    RgbMode::ShuttleRun,
    RgbMode::River,
    RgbMode::Hourglass,
    RgbMode::Pioneer,
    RgbMode::ElectricCurrent,
    RgbMode::Transformation,
    RgbMode::GradientRibbon,
    RgbMode::RainbowWave,
    RgbMode::Snooker,
    RgbMode::Mixing,
    RgbMode::PingPong,
    RgbMode::Runway,
    RgbMode::Tide,
    RgbMode::BlowUp,
    RgbMode::Meteor,
    RgbMode::Stack,
];
pub const SEGMENT_MODES: &[RgbMode] = &[
    RgbMode::Off,
    RgbMode::Rainbow,
    RgbMode::Wave,
    RgbMode::Static,
    RgbMode::Breathing,
    RgbMode::RainbowMorph,
    RgbMode::Paint,
];
pub const SCOPES: &[RgbScope] = &[
    RgbScope::All,
    RgbScope::Segment1,
    RgbScope::Segment2,
    RgbScope::Segment3,
    RgbScope::Segment4,
    RgbScope::Segment5,
    RgbScope::Segment6,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncKey {
    pub modes: Vec<RgbMode>,
    pub compared_speed: u8,
}

pub fn sync_key(regions: &[RgbRegionConfig], led_count: usize) -> Result<SyncKey> {
    let geometry = geometry(led_count)?;
    ensure!(!regions.is_empty(), "Strimer requires a lighting region");
    if regions.len() == 1 && regions[0].effect.scope == RgbScope::All {
        validate(&regions[0].effect)?;
        return Ok(SyncKey {
            modes: vec![regions[0].effect.mode],
            compared_speed: regions[0].effect.speed,
        });
    }
    let effects = normalize_segments(regions, geometry)?;
    Ok(SyncKey {
        modes: effects.iter().map(|effect| effect.mode).collect(),
        compared_speed: 2,
    })
}

pub fn render(regions: &[RgbRegionConfig], led_count: usize) -> Result<Animation> {
    let geometry = geometry(led_count)?;
    ensure!(!regions.is_empty(), "Strimer requires a lighting region");
    if regions.len() == 1 && regions[0].effect.scope == RgbScope::All {
        ensure!(!regions[0].flip, "Strimer does not support region flipping");
        return render_all(&regions[0].effect, geometry);
    }
    ensure!(
        regions
            .iter()
            .all(|region| region.effect.scope != RgbScope::All),
        "All cannot be combined with Strimer segments"
    );
    let effects = normalize_segments(regions, geometry)?;
    let target_frames = geometry.lane_length * 12;
    let mut output = vec![frame(geometry); target_frames];
    for (lane, effect) in effects.iter().take(geometry.lanes).enumerate() {
        let frames = render_segment(effect, geometry)?;
        let start = lane * geometry.lane_length;
        let end = start + geometry.lane_length;
        for index in 0..target_frames {
            output[index][start..end].copy_from_slice(&frames[index % frames.len()][start..end]);
        }
    }
    Ok(Animation {
        frames: output,
        interval_hundredths: 20 * speed(effects[0].speed)? * 100,
        secondary: None,
    })
}

fn normalize_segments(regions: &[RgbRegionConfig], geometry: Geometry) -> Result<[RgbEffect; 6]> {
    let mut effects = std::array::from_fn(default_segment);
    let mut used = [false; 6];
    for region in regions {
        ensure!(!region.flip, "Strimer does not support region flipping");
        let lane = lane(region.effect.scope).ok_or_else(|| {
            anyhow::anyhow!("Strimer effects support All and Segment1 through Segment6")
        })?;
        ensure!(
            lane < geometry.lanes,
            "Strimer segment is not present on this model"
        );
        ensure!(!used[lane], "duplicate Strimer segment");
        ensure!(
            SEGMENT_MODES.contains(&region.effect.mode),
            "unsupported Strimer segment RGB mode"
        );
        validate(&region.effect)?;
        used[lane] = true;
        effects[lane] = region.effect.clone();
    }
    Ok(effects)
}

fn default_segment(index: usize) -> RgbEffect {
    RgbEffect {
        mode: RgbMode::Rainbow,
        colors: vec![],
        speed: 3,
        brightness: 4,
        direction: RgbDirection::Clockwise,
        scope: SCOPES[index + 1],
        disabled: false,
    }
}

fn render_all(effect: &RgbEffect, geometry: Geometry) -> Result<Animation> {
    ensure!(
        MODES.contains(&effect.mode),
        "unsupported whole-Strimer RGB mode"
    );
    validate(effect)?;
    let brightness = engine::brightness(effect)?;
    let colors = engine::colors(effect);
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    let (frames, base_hundredths) = if effect.disabled || effect.mode == RgbMode::Off {
        (vec![frame(geometry)], 2_000)
    } else {
        match effect.mode {
            RgbMode::Rainbow => (classic::rainbow(geometry, brightness, reverse), 2_200),
            RgbMode::Wave => (classic::wave(geometry, &colors, brightness, reverse), 2_200),
            RgbMode::Static => (classic::solid(geometry, colors[0], brightness, 30), 2_000),
            RgbMode::Breathing => (
                classic::breathing(geometry, colors[0], brightness, false),
                1_100,
            ),
            RgbMode::RainbowMorph => (classic::morph(geometry, brightness, false), 1_100),
            RgbMode::Paint => (classic::paint(geometry, &colors, brightness), 1_100),
            RgbMode::ColorTransfer => (
                modes_1_2::color_transfer(geometry, &colors, brightness, reverse),
                2_400,
            ),
            RgbMode::FadeOut => (modes_1_2::fade_out(geometry, &colors, brightness), 1_100),
            RgbMode::Contest => (
                mode_3::contest(geometry, &colors, brightness, reverse),
                1_100,
            ),
            RgbMode::CrossOver => (
                modes_4_5::cross_over(geometry, &colors, brightness, reverse),
                1_100,
            ),
            RgbMode::BulletStack => (
                modes_4_5::bullet_stack(geometry, &colors, brightness, reverse),
                1_130,
            ),
            RgbMode::Twinkle => (mode_6::twinkle(geometry, &colors, brightness), 1_100),
            RgbMode::Parallel => (
                mode_7::parallel(geometry, &colors, brightness, reverse),
                1_100,
            ),
            RgbMode::ShockWave => (
                mode_8::shock_wave(geometry, &colors, brightness, reverse),
                1_100,
            ),
            RgbMode::Ripple => (modes_9_10::ripple(geometry, &colors, brightness), 1_100),
            RgbMode::Voice => (modes_9_10::voice(geometry, &colors, brightness), 1_100),
            RgbMode::Transformation => (
                modes_18_19::transformation(geometry, &colors, brightness, reverse),
                1_100,
            ),
            RgbMode::GradientRibbon => (
                modes_18_19::gradient_ribbon(geometry, brightness, reverse),
                1_100,
            ),
            RgbMode::Pioneer => (modes_16_21::pioneer(geometry, colors[0], brightness), 1_100),
            RgbMode::Snooker => (modes_16_21::snooker(geometry, &colors, brightness), 1_100),
            RgbMode::RainbowWave => (
                modes_20_22::rainbow_wave(geometry, brightness, reverse),
                1_100,
            ),
            RgbMode::Mixing => (modes_20_22::mixing(geometry, &colors, brightness), 850),
            RgbMode::PingPong => (modes_23_27::ping_pong(geometry, &colors, brightness), 1_200),
            RgbMode::Runway => (modes_23_27::runway(geometry, &colors, brightness), 1_200),
            RgbMode::Tide => (modes_23_27::tide(geometry, &colors, brightness), 1_100),
            RgbMode::BlowUp => (modes_23_27::blow_up(geometry, &colors, brightness), 1_550),
            RgbMode::Meteor => (
                modes_23_27::meteor(geometry, &colors, brightness, reverse),
                1_500,
            ),
            RgbMode::Stack => (
                mode_34::stack(geometry, &colors, brightness, reverse),
                1_100,
            ),
            RgbMode::Drizzling => (
                modes_11_12::drizzling(geometry, &colors, brightness, reverse),
                1_100,
            ),
            RgbMode::Endless => (modes_11_12::endless(geometry, &colors, brightness), 1_100),
            RgbMode::River => (
                mode_14::river(geometry, &colors, brightness, reverse),
                1_100,
            ),
            RgbMode::ShuttleRun => (mode_13::shuttle_run(geometry, &colors, brightness), 1_100),
            RgbMode::Hourglass => (modes_15_17::hourglass(geometry, &colors, brightness), 1_100),
            RgbMode::ElectricCurrent => (
                modes_15_17::electric_current(geometry, &colors, brightness),
                1_100,
            ),
            _ => bail!("unsupported whole-Strimer RGB mode: {:?}", effect.mode),
        }
    };
    Ok(Animation {
        frames,
        interval_hundredths: base_hundredths * speed(effect.speed)?,
        secondary: None,
    })
}

fn render_segment(effect: &RgbEffect, geometry: Geometry) -> Result<Vec<Frame>> {
    ensure!(
        SEGMENT_MODES.contains(&effect.mode),
        "unsupported Strimer segment RGB mode"
    );
    validate(effect)?;
    if effect.disabled || effect.mode == RgbMode::Off {
        return Ok(vec![frame(geometry)]);
    }
    let brightness = engine::brightness(effect)?;
    let colors = engine::colors(effect);
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    Ok(match effect.mode {
        RgbMode::Rainbow => classic::rainbow(geometry, brightness, reverse),
        RgbMode::Wave => classic::wave(geometry, &colors, brightness, reverse),
        RgbMode::Static => classic::solid(geometry, colors[0], brightness, 1),
        RgbMode::Breathing => classic::breathing(geometry, colors[0], brightness, true),
        RgbMode::RainbowMorph => classic::morph(geometry, brightness, true),
        RgbMode::Paint => classic::paint(geometry, &colors, brightness),
        _ => unreachable!("validated segment mode"),
    })
}

fn validate(effect: &RgbEffect) -> Result<()> {
    ensure!(
        effect.colors.len() <= 6,
        "Strimer palettes support at most six colors"
    );
    speed(effect.speed)?;
    engine::brightness(effect)?;
    Ok(())
}

fn speed(speed: u8) -> Result<u32> {
    [7, 6, 5, 4, 3]
        .get(speed as usize)
        .copied()
        .ok_or_else(|| anyhow::anyhow!("RGB speed must be 0..=4"))
}
