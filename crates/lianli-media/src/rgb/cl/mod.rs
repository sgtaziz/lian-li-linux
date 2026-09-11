mod arcs;
mod basic;
mod color_cycles;
mod color_sweeps;
mod colored_meteors;
pub(crate) mod engine;
mod lottery;
mod meteor_trails;
mod movement;
pub(crate) mod parameters;
mod pulses;
mod reflections;
mod twinkle;
mod wing;

use super::Animation;
use anyhow::{bail, ensure, Result};
use engine::{Color, Plane, PlaneAnimation};
use lianli_shared::rgb::{is_brightness_off, RgbEffect, RgbMode, RgbRegionConfig, RgbScope};

pub const MODES: &[RgbMode] = &[
    RgbMode::Off,
    RgbMode::Rainbow,
    RgbMode::RainbowMorph,
    RgbMode::Static,
    RgbMode::Breathing,
    RgbMode::Runway,
    RgbMode::Meteor,
    RgbMode::Twinkle,
    RgbMode::TaiChi,
    RgbMode::ColorCycle,
    RgbMode::MopUp,
    RgbMode::MeteorRainbow,
    RgbMode::ColorfulMeteor,
    RgbMode::Lottery,
    RgbMode::Warning,
    RgbMode::Voice,
    RgbMode::Mixing,
    RgbMode::Tide,
    RgbMode::Scan,
    RgbMode::DoubleMeteor,
    RgbMode::MeteorContest,
    RgbMode::MeteorMix,
    RgbMode::ReturnArc,
    RgbMode::DoubleArc,
    RgbMode::Door,
    RgbMode::HeartBeat,
    RgbMode::HeartBeatRunway,
    RgbMode::Disco,
    RgbMode::ElectricCurrent,
    RgbMode::Reflect,
    RgbMode::GradientRibbon,
    RgbMode::Wing,
    RgbMode::Drumming,
    RgbMode::Boomerang,
    RgbMode::CandyBox,
];

pub fn render(regions: &[RgbRegionConfig], fans: usize) -> Result<Animation> {
    ensure!((1..=4).contains(&fans), "CL fan count must be 1..=4");
    ensure!(
        !regions.is_empty(),
        "CL requires at least one lighting region"
    );
    let mut center = None;
    let mut outer = None;
    for region in regions {
        ensure!(!region.flip, "CL does not support region flipping");
        match region.effect.scope {
            RgbScope::All => {
                center = Some(&region.effect);
                outer = Some(&region.effect);
            }
            RgbScope::Center => center = Some(&region.effect),
            RgbScope::Outer => outer = Some(&region.effect),
            _ => bail!("CL effects support All, Center and Outer regions"),
        }
    }
    let outer = match outer {
        Some(effect) => render_plane(effect, fans, Plane::Outer)?,
        None => empty(fans),
    };
    let center = match center {
        Some(effect) => render_plane(effect, fans, Plane::Center)?,
        None => empty(fans),
    };
    combine(outer, center)
}

pub(crate) fn render_plane(
    effect: &RgbEffect,
    fans: usize,
    plane: Plane,
) -> Result<PlaneAnimation> {
    engine::validate(effect, fans)?;
    if effect.disabled || effect.mode == RgbMode::Off || is_brightness_off(effect.brightness) {
        return Ok(PlaneAnimation {
            frames: vec![vec![[0; 3]; fans * 24]],
            interval_ticks: 11 * u32::from(7 - effect.speed),
            interval_base_ticks: 11,
        });
    }
    let (frames, interval_base) = match effect.mode {
        RgbMode::Rainbow => (basic::rainbow(effect, fans, plane), 35),
        RgbMode::RainbowMorph => (basic::rainbow_morph(effect, fans, plane), 11),
        RgbMode::Static => (basic::static_color(effect, fans, plane), 20),
        RgbMode::Breathing => (basic::breathing(effect, fans, plane), 11),
        RgbMode::Runway => (movement::runway(effect, fans, plane), 20),
        RgbMode::Meteor => (movement::meteor(effect, fans, plane), 20),
        RgbMode::Twinkle => (twinkle::render(effect, fans), 20),
        RgbMode::TaiChi => (movement::tai_chi(effect, fans, plane), 20),
        RgbMode::ColorCycle => (movement::color_cycle(effect, fans, plane), 20),
        RgbMode::MopUp => (movement::mop_up(effect, fans, plane), 20),
        RgbMode::MeteorRainbow => (colored_meteors::meteor_rainbow(effect, fans, plane), 20),
        RgbMode::ColorfulMeteor => (colored_meteors::colorful_meteor(effect, fans, plane), 20),
        RgbMode::Lottery => (lottery::lottery(effect, fans, plane), 20),
        RgbMode::Warning => (pulses::warning(effect, fans, plane), 20),
        RgbMode::Voice => (pulses::voice(effect, fans, plane), 20),
        RgbMode::Mixing => (color_sweeps::mixing(effect, fans, plane), 20),
        RgbMode::Tide => (color_sweeps::tide(effect, fans, plane), 20),
        RgbMode::Scan => (color_sweeps::scan(effect, fans, plane), 20),
        RgbMode::DoubleMeteor => (meteor_trails::double_meteor(effect, fans, plane), 20),
        RgbMode::MeteorContest => (meteor_trails::meteor_contest(effect, fans, plane), 25),
        RgbMode::MeteorMix => (meteor_trails::meteor_mix(effect, fans, plane), 25),
        RgbMode::ReturnArc => (arcs::return_arc(effect, fans, plane), 20),
        RgbMode::DoubleArc => (arcs::double_arc(effect, fans, plane), 35),
        RgbMode::Door => (color_sweeps::door(effect, fans, plane), 25),
        RgbMode::HeartBeat => (pulses::heart_beat(effect, fans, plane), 20),
        RgbMode::HeartBeatRunway => (pulses::heart_beat_runway(effect, fans, plane), 10),
        RgbMode::Disco => (pulses::disco(effect, fans, plane), 20),
        RgbMode::ElectricCurrent => (reflections::electric_current(effect, fans, plane), 20),
        RgbMode::Reflect => (reflections::reflect(effect, fans, plane), 20),
        RgbMode::GradientRibbon => (color_cycles::gradient_ribbon(effect, fans, plane), 20),
        RgbMode::Wing => (wing::wing(effect, fans, plane), 20),
        RgbMode::Drumming => (pulses::drumming(effect, fans, plane), 20),
        RgbMode::Boomerang => (arcs::boomerang(effect, fans, plane), 20),
        RgbMode::CandyBox => (color_cycles::candy_box(effect, fans, plane), 20),
        _ => bail!("unsupported CL RGB mode: {:?}", effect.mode),
    };
    Ok(PlaneAnimation {
        frames,
        interval_ticks: interval_base * u32::from(7 - effect.speed),
        interval_base_ticks: interval_base,
    })
}

fn empty(fans: usize) -> PlaneAnimation {
    PlaneAnimation {
        frames: vec![vec![[0; 3]; fans * 24]],
        interval_ticks: 0,
        interval_base_ticks: 0,
    }
}

fn combine(mut outer: PlaneAnimation, mut center: PlaneAnimation) -> Result<Animation> {
    let initial_interval = if center.interval_base_ticks > 0 {
        let speed_multiplier = if outer.interval_base_ticks > 0 {
            outer.interval_ticks / outer.interval_base_ticks
        } else {
            center.interval_ticks / center.interval_base_ticks
        };
        speed_multiplier * center.interval_base_ticks
    } else {
        outer.interval_ticks
    };
    if outer.frames.len() >= center.frames.len() {
        repeat_short_animation(&mut center.frames, outer.frames.len());
        let center_len = center.frames.len();
        let mut frames = outer.frames;
        resample_plane(&mut frames, &center.frames, initial_interval, Plane::Center);
        let interval_hundredths = if center.interval_ticks > 0 {
            center.interval_ticks * center_len as u32 * 100 / frames.len() as u32
        } else {
            initial_interval * 100
        };
        Ok(Animation {
            frames,
            interval_hundredths,
            secondary: None,
        })
    } else {
        repeat_short_animation(&mut outer.frames, center.frames.len());
        let mut frames = center.frames;
        resample_plane(&mut frames, &outer.frames, initial_interval, Plane::Outer);
        Ok(Animation {
            frames,
            interval_hundredths: initial_interval * 100,
            secondary: None,
        })
    }
}

fn repeat_short_animation(frames: &mut Vec<Vec<Color>>, maximum: usize) {
    if maximum / 2 >= frames.len() {
        let source = frames.clone();
        let repeats = maximum / source.len();
        frames.clear();
        for _ in 0..repeats {
            frames.extend(source.iter().cloned());
        }
    }
}

fn resample_plane(
    target: &mut [Vec<Color>],
    source: &[Vec<Color>],
    interval_ticks: u32,
    plane: Plane,
) {
    let divisor = interval_ticks as usize * target.len() / source.len();
    for (index, frame) in target.iter_mut().enumerate() {
        let source_index = (index * interval_ticks as usize / divisor).min(source.len() - 1);
        for (target_fan, source_fan) in frame
            .as_chunks_mut::<24>()
            .0
            .iter_mut()
            .zip(source[source_index].as_chunks::<24>().0)
        {
            match plane {
                Plane::Center => target_fan[..8].copy_from_slice(&source_fan[..8]),
                Plane::Outer => target_fan[8..24].copy_from_slice(&source_fan[8..24]),
            }
        }
    }
}

#[cfg(test)]
mod tests;
