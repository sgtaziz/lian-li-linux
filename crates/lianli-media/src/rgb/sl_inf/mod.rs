mod arcs;
mod basic;
mod color_cycles;
mod color_sweeps;
mod colored_meteors;
mod effect_patterns;
mod electric_patterns;
mod engine;
mod lottery;
mod meteor_trails;
mod movement;
#[cfg(test)]
mod native_tests;
pub mod parameters;
mod pulses;
mod reflections;
mod twinkle;
mod wing;

use super::Animation;
use anyhow::{bail, ensure, Result};
use engine::{copy_plane, validate, Color, Plane};
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

pub const SCOPES: &[RgbScope] = &[
    RgbScope::All,
    RgbScope::Inner,
    RgbScope::Outer,
    RgbScope::Center,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InfVariant {
    Original,
    V3,
}

struct RegionalAnimation {
    frames: Vec<Vec<Color>>,
    interval_base: u32,
    speed_multiplier: u32,
}

pub fn render(
    regions: &[RgbRegionConfig],
    fans: usize,
    right_attach: bool,
    variant: InfVariant,
) -> Result<Animation> {
    ensure!((1..=4).contains(&fans), "SL-INF fan count must be 1..=4");
    let mut selected = [None, None, None];
    for region in regions {
        ensure!(!region.flip, "SL-INF does not support region flipping");
        match region.effect.scope {
            RgbScope::All => selected.fill(Some(region)),
            RgbScope::Outer => selected[0] = Some(region),
            RgbScope::Inner => selected[1] = Some(region),
            RgbScope::Center => selected[2] = Some(region),
            _ => bail!("SL-INF effects support All, Inner, Outer and Center regions"),
        }
    }
    let off = RgbRegionConfig {
        effect: RgbEffect {
            mode: RgbMode::Off,
            ..Default::default()
        },
        flip: false,
    };
    let fallback = selected.iter().flatten().next().copied().unwrap_or(&off);
    let outer = selected[0].unwrap_or(fallback);
    let inner = selected[1].unwrap_or(fallback);
    let center = selected[2].unwrap_or(fallback);
    let pn = match variant {
        InfVariant::Original => u8::from(!right_attach),
        InfVariant::V3 => u8::from(right_attach),
    };
    let outer = render_region(&outer.effect, fans, Plane::Outer, pn)?;
    let inner = render_region(&inner.effect, fans, Plane::Inner, pn)?;
    let center = render_region(&center.effect, fans, Plane::Center, pn)?;
    combine(inner, outer, center)
}

fn render_region(
    effect: &RgbEffect,
    fans: usize,
    plane: Plane,
    pn: u8,
) -> Result<RegionalAnimation> {
    if effect.disabled || effect.mode == RgbMode::Off {
        return Ok(RegionalAnimation {
            frames: vec![vec![[0; 3]; fans * 44]; 30],
            interval_base: 11,
            speed_multiplier: u32::from(7 - effect.speed.min(4)),
        });
    }
    let normalized;
    let effect = if is_brightness_off(effect.brightness) {
        normalized = RgbEffect {
            brightness: 0,
            ..effect.clone()
        };
        &normalized
    } else {
        effect
    };
    validate(effect, fans)?;
    ensure!(
        MODES.contains(&effect.mode),
        "unsupported SL-INF mode: {:?}",
        effect.mode
    );
    let frames = match effect.mode {
        RgbMode::Rainbow => basic::rainbow(effect, fans, plane, pn),
        RgbMode::RainbowMorph => basic::rainbow_morph(effect, fans, plane),
        RgbMode::Static => basic::static_color(effect, fans, plane),
        RgbMode::Breathing => basic::breathing(effect, fans, plane),
        RgbMode::Runway => movement::runway(effect, fans, plane, pn),
        RgbMode::Meteor => movement::meteor(effect, fans, plane, pn),
        RgbMode::Twinkle => twinkle::twinkle(effect, fans),
        RgbMode::TaiChi => movement::tai_chi(effect, fans, plane, pn),
        RgbMode::ColorCycle => color_cycles::color_cycle(effect, fans, plane, pn),
        RgbMode::MopUp => color_cycles::mop_up(effect, fans, plane, pn),
        RgbMode::MeteorRainbow => colored_meteors::meteor_rainbow(effect, fans, plane, pn),
        RgbMode::ColorfulMeteor => colored_meteors::colorful_meteor(effect, fans, plane, pn),
        RgbMode::Lottery => lottery::lottery(effect, fans, plane, pn),
        RgbMode::Warning => pulses::warning(effect, fans, plane, pn),
        RgbMode::Voice => pulses::voice(effect, fans, plane, pn),
        RgbMode::Mixing => color_sweeps::mixing(effect, fans, plane, pn),
        RgbMode::Tide => color_sweeps::tide(effect, fans, plane, pn),
        RgbMode::Scan => color_sweeps::scan(effect, fans, plane, pn),
        RgbMode::DoubleMeteor => meteor_trails::double_meteor(effect, fans, plane, pn),
        RgbMode::MeteorContest => meteor_trails::meteor_contest(effect, fans, plane, pn),
        RgbMode::MeteorMix => meteor_trails::meteor_mix(effect, fans, plane, pn),
        RgbMode::ReturnArc => arcs::return_arc(effect, fans, plane, pn),
        RgbMode::DoubleArc => arcs::double_arc(effect, fans, plane, pn),
        RgbMode::Door => color_sweeps::door(effect, fans, plane, pn),
        RgbMode::HeartBeat => pulses::heart_beat(effect, fans, plane, pn),
        RgbMode::HeartBeatRunway => pulses::heart_beat_runway(effect, fans, plane, pn),
        RgbMode::Disco => pulses::disco(effect, fans, plane, pn),
        RgbMode::ElectricCurrent => reflections::electric_current(effect, fans, plane, pn),
        RgbMode::Reflect => reflections::reflect(effect, fans, plane, pn),
        RgbMode::GradientRibbon => color_cycles::gradient_ribbon(effect, fans, plane, pn),
        RgbMode::Wing => wing::wing(effect, fans, plane, pn),
        RgbMode::Drumming => pulses::drumming(effect, fans, plane, pn),
        RgbMode::Boomerang => arcs::boomerang(effect, fans, plane, pn),
        RgbMode::CandyBox => color_cycles::candy_box(effect, fans, plane, pn),
        _ => unreachable!(),
    };
    let interval_base = match effect.mode {
        RgbMode::Rainbow
        | RgbMode::Runway
        | RgbMode::Meteor
        | RgbMode::Twinkle
        | RgbMode::TaiChi
        | RgbMode::ColorCycle
        | RgbMode::MopUp
        | RgbMode::MeteorRainbow
        | RgbMode::ColorfulMeteor
        | RgbMode::Warning
        | RgbMode::Voice
        | RgbMode::Mixing
        | RgbMode::Tide
        | RgbMode::Scan => 20,
        RgbMode::DoubleMeteor | RgbMode::ReturnArc => 20,
        RgbMode::MeteorContest | RgbMode::MeteorMix | RgbMode::DoubleArc => 30,
        RgbMode::Door => 30,
        RgbMode::HeartBeat | RgbMode::Disco | RgbMode::Reflect | RgbMode::GradientRibbon => 20,
        RgbMode::HeartBeatRunway => 10,
        RgbMode::ElectricCurrent => [40, 20, 15, 10][fans - 1],
        RgbMode::Wing => [50, 30, 20, 18][fans - 1],
        RgbMode::Drumming | RgbMode::CandyBox => 20,
        RgbMode::Boomerang => [50, 35, 20, 20][fans - 1],
        _ => 11,
    };
    Ok(RegionalAnimation {
        frames,
        interval_base,
        speed_multiplier: u32::from(7 - effect.speed),
    })
}

fn repeat_short(mut frames: Vec<Vec<Color>>, maximum: usize) -> Vec<Vec<Color>> {
    if maximum / 2 >= frames.len() {
        let repeats = maximum / frames.len();
        let source = frames;
        frames = Vec::with_capacity(source.len() * repeats);
        for _ in 0..repeats {
            frames.extend(source.iter().cloned());
        }
    }
    frames
}

fn sample(frame: usize, interval: usize, maximum: usize, length: usize) -> usize {
    let divisor = interval * maximum / length;
    (frame * interval / divisor).min(length - 1)
}

fn combine(
    inner: RegionalAnimation,
    outer: RegionalAnimation,
    center: RegionalAnimation,
) -> Result<Animation> {
    let interval = inner.speed_multiplier as usize * center.interval_base as usize;
    let center_interval = center.speed_multiplier * center.interval_base;
    let inner_len = inner.frames.len();
    let outer_len = outer.frames.len();
    let center_len = center.frames.len();
    let primary = if inner_len >= outer_len && inner_len >= center_len {
        Plane::Inner
    } else if outer_len >= inner_len && outer_len >= center_len {
        Plane::Outer
    } else {
        Plane::Center
    };
    let maximum = match primary {
        Plane::Inner => inner_len,
        Plane::Outer => outer_len,
        Plane::Center => center_len,
    };
    let inner = repeat_short(inner.frames, maximum);
    let outer = repeat_short(outer.frames, maximum);
    let center = repeat_short(center.frames, maximum);
    let mut frames = match primary {
        Plane::Inner => inner.clone(),
        Plane::Outer => outer.clone(),
        Plane::Center => center.clone(),
    };
    for (frame, target) in frames.iter_mut().enumerate() {
        if primary != Plane::Inner {
            copy_plane(
                target,
                &inner[sample(frame, interval, maximum, inner.len())],
                Plane::Inner,
            );
        }
        if primary != Plane::Outer {
            copy_plane(
                target,
                &outer[sample(frame, interval, maximum, outer.len())],
                Plane::Outer,
            );
        }
        if primary != Plane::Center {
            copy_plane(
                target,
                &center[sample(frame, interval, maximum, center.len())],
                Plane::Center,
            );
        }
    }
    let interval_hundredths = match primary {
        Plane::Inner => center_interval * center.len() as u32 * 100 / maximum as u32,
        Plane::Outer => (center_interval * center.len() as u32).div_ceil(maximum as u32) * 100,
        Plane::Center => interval as u32 * 100,
    };
    Ok(Animation {
        frames,
        interval_hundredths,
        secondary: None,
    })
}
