mod basic;
pub mod capabilities;
mod chase_patterns;
mod collisions;
mod engine;
mod fills;
mod flow_patterns;
mod movement;
mod patterns;
mod shuttle_masks;
#[cfg(test)]
mod source_tests;
mod twinkle;

use super::{Animation, SecondaryTiming};
use anyhow::{bail, ensure, Result};
use engine::{validate, Color, Side};
use lianli_shared::rgb::{is_brightness_off, RgbEffect, RgbMode, RgbRegionConfig, RgbScope};

pub const WHOLE_MODES: &[RgbMode] = &[
    RgbMode::Off,
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
    RgbMode::Endless,
    RgbMode::River,
    RgbMode::Duel,
    RgbMode::Pioneer,
    RgbMode::Hourglass,
    RgbMode::ShuttleRun,
    RgbMode::GradientRibbon,
    RgbMode::Twinkle,
];

pub const SIDE_MODES: &[RgbMode] = &[
    RgbMode::Off,
    RgbMode::Rainbow,
    RgbMode::RainbowMorph,
    RgbMode::Static,
    RgbMode::Breathing,
    RgbMode::Runway,
    RgbMode::Meteor,
    RgbMode::ColorCycle,
    RgbMode::Staggered,
    RgbMode::Tide,
    RgbMode::Mixing,
    RgbMode::Render,
    RgbMode::PingPong,
    RgbMode::Stack,
    RgbMode::Ripple,
    RgbMode::Collide,
    RgbMode::Reflect,
    RgbMode::ElectricCurrent,
];

struct RegionalAnimation {
    frames: Vec<Vec<Color>>,
    interval_base: u32,
}

pub fn render(regions: &[RgbRegionConfig], fans: usize) -> Result<Animation> {
    ensure!((1..=4).contains(&fans), "SL V4 fan count must be 1..=4");
    let mut inner = None;
    let mut outer = None;
    for region in regions {
        match region.effect.scope {
            RgbScope::All => {
                inner = Some(region);
                outer = Some(region);
            }
            RgbScope::Inner => inner = Some(region),
            RgbScope::Outer => outer = Some(region),
            _ => bail!("SL V4 effects support All, Inner and Outer regions"),
        }
    }
    let off = RgbRegionConfig {
        effect: RgbEffect {
            mode: RgbMode::Off,
            ..Default::default()
        },
        flip: false,
    };
    let fallback = inner.or(outer).unwrap_or(&off);
    let inner = inner.unwrap_or(fallback);
    let outer = outer.unwrap_or(fallback);
    ensure!(
        !inner.flip && !outer.flip,
        "SL V4 does not support flipping"
    );
    if std::ptr::eq(inner, outer)
        && inner.effect.scope == RgbScope::All
        && matches!(
            inner.effect.mode,
            RgbMode::Endless | RgbMode::Pioneer | RgbMode::GradientRibbon | RgbMode::Twinkle
        )
    {
        return render_whole(&inner.effect, fans);
    }
    let inner_animation = render_region(&inner.effect, fans, Side::Inner)?;
    let outer_animation = render_region(&outer.effect, fans, Side::Outer)?;
    let speed = regions
        .last()
        .map(|region| region.effect.speed)
        .unwrap_or(outer.effect.speed);
    combine(inner_animation, outer_animation, speed)
}

fn render_whole(effect: &RgbEffect, fans: usize) -> Result<Animation> {
    ensure!(effect.speed <= 4, "RGB speed must be 0..=4");
    let inner = render_region(effect, fans, Side::Inner)?;
    let outer = render_region(effect, fans, Side::Outer)?;
    ensure!(
        inner.frames.len() == outer.frames.len(),
        "whole SL V4 effect produced mismatched region frame counts"
    );
    let mut frames = outer.frames;
    for (target, source) in frames.iter_mut().zip(inner.frames) {
        copy_side(target, &source, Side::Inner);
    }
    Ok(Animation {
        frames,
        interval_hundredths: u32::from(7 - effect.speed) * outer.interval_base * 100,
        secondary: None,
    })
}

fn render_region(effect: &RgbEffect, fans: usize, side: Side) -> Result<RegionalAnimation> {
    if effect.disabled || effect.mode == RgbMode::Off {
        return Ok(RegionalAnimation {
            frames: vec![vec![[0; 3]; fans * 52]; 30],
            interval_base: 11,
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
    let supported = match effect.scope {
        RgbScope::All => WHOLE_MODES,
        RgbScope::Inner | RgbScope::Outer => SIDE_MODES,
        _ => bail!("SL V4 effects support All, Inner and Outer regions"),
    };
    ensure!(supported.contains(&effect.mode), "unsupported SL V4 mode");
    let frames = match effect.mode {
        RgbMode::Rainbow => basic::rainbow(effect, fans, side),
        RgbMode::Static => basic::static_color(effect, fans, side),
        RgbMode::RainbowMorph => basic::rainbow_morph(effect, fans, side),
        RgbMode::Breathing => basic::breathing(effect, fans, side),
        RgbMode::Runway => movement::runway(effect, fans, side),
        RgbMode::Meteor => movement::meteor(effect, fans, side),
        RgbMode::ColorCycle => movement::color_cycle(effect, fans, side),
        RgbMode::Staggered => patterns::staggered(effect, fans, side),
        RgbMode::Tide => patterns::tide(effect, fans, side),
        RgbMode::Mixing => fills::mixing(effect, fans, side),
        RgbMode::Render => fills::render_effect(effect, fans, side),
        RgbMode::PingPong => fills::ping_pong(effect, fans, side),
        RgbMode::Stack => fills::stack(effect, fans, side),
        RgbMode::Ripple => collisions::ripple(effect, fans, side),
        RgbMode::Collide => collisions::collide(effect, fans, side),
        RgbMode::Reflect => collisions::reflect(effect, fans, side),
        RgbMode::ElectricCurrent => collisions::electric_current(effect, fans, side),
        RgbMode::Endless => flow_patterns::endless(effect, fans, side),
        RgbMode::River => flow_patterns::river(effect, fans, side),
        RgbMode::Duel => flow_patterns::duel(effect, fans, side),
        RgbMode::Hourglass => flow_patterns::hourglass(effect, fans, side),
        RgbMode::Pioneer => chase_patterns::pioneer(effect, fans, side),
        RgbMode::ShuttleRun => chase_patterns::shuttle_run(effect, fans, side),
        RgbMode::GradientRibbon => chase_patterns::gradient_ribbon(effect, fans, side),
        RgbMode::Twinkle => twinkle::render(effect, fans, side),
        _ => unreachable!(),
    };
    Ok(RegionalAnimation {
        frames,
        interval_base: if effect.mode == RgbMode::Stack {
            20
        } else {
            11
        },
    })
}

fn combine(inner: RegionalAnimation, outer: RegionalAnimation, speed: u8) -> Result<Animation> {
    ensure!(speed <= 4, "RGB speed must be 0..=4");
    let primary_ticks = u32::from(7 - speed) * outer.interval_base;
    let outer_longest = outer.frames.len() >= inner.frames.len();
    let (mut longer, mut shorter) = if outer_longest {
        (outer.frames, inner.frames)
    } else {
        (inner.frames, outer.frames)
    };
    let maximum = longer.len();
    if maximum / 2 >= shorter.len() {
        let repeats = maximum / shorter.len();
        let source = shorter.clone();
        shorter.clear();
        for _ in 0..repeats {
            shorter.extend(source.iter().cloned());
        }
    }
    let secondary_ticks = primary_ticks as usize * maximum / shorter.len();
    for (frame, target) in longer.iter_mut().enumerate() {
        let source = frame * shorter.len() / maximum;
        copy_side(
            target,
            &shorter[source],
            if outer_longest {
                Side::Inner
            } else {
                Side::Outer
            },
        );
    }
    Ok(Animation {
        frames: longer,
        interval_hundredths: primary_ticks * 100,
        secondary: Some(SecondaryTiming {
            interval_ticks: secondary_ticks as u16,
            frame_count: shorter.len() as u16,
            outer_longest,
        }),
    })
}

fn copy_side(target: &mut [Color], source: &[Color], side: Side) {
    let target = target.as_chunks_mut::<52>().0;
    let source = source.as_chunks::<52>().0;
    for (target, source) in target.iter_mut().zip(source) {
        match side {
            Side::Inner => {
                target[..13].copy_from_slice(&source[..13]);
                target[26..39].copy_from_slice(&source[26..39]);
            }
            Side::Outer => {
                target[13..26].copy_from_slice(&source[13..26]);
                target[39..52].copy_from_slice(&source[39..52]);
            }
        }
    }
}
