mod basic;
pub mod capabilities;
mod chase_patterns;
mod collisions;
mod engine;
mod fills;
mod flow_patterns;
mod integration;
mod movement;
#[cfg(test)]
mod native_all_tests;
#[cfg(test)]
mod native_tests;
mod patterns;
mod twinkle;

use super::{Animation, SecondaryTiming};
use anyhow::{bail, ensure, Result};
use engine::{validate, Color, Side};
use lianli_shared::rgb::{is_brightness_off, RgbEffect, RgbMode, RgbRegionConfig, RgbScope};

pub const MODES: &[RgbMode] = &[
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
    RgbMode::Endless,
    RgbMode::River,
    RgbMode::Duel,
    RgbMode::Hourglass,
    RgbMode::Pioneer,
    RgbMode::ShuttleRun,
    RgbMode::GradientRibbon,
    RgbMode::Twinkle,
];

struct RegionalAnimation {
    frames: Vec<Vec<Color>>,
    interval_base: u32,
}

pub fn render(regions: &[RgbRegionConfig], fans: usize) -> Result<Animation> {
    ensure!(
        (1..=4).contains(&fans),
        "SL Wireless fan count must be 1..=4"
    );
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
            _ => bail!("SL Wireless effects support All, Inner and Outer regions"),
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
        "SL Wireless does not support region flipping"
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
    ensure_eq_frame_counts(&inner.frames, &outer.frames)?;
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

fn ensure_eq_frame_counts(inner: &[Vec<Color>], outer: &[Vec<Color>]) -> Result<()> {
    ensure!(
        inner.len() == outer.len(),
        "whole SL Wireless effect produced mismatched region frame counts"
    );
    Ok(())
}

fn render_region(effect: &RgbEffect, fans: usize, side: Side) -> Result<RegionalAnimation> {
    if effect.disabled || effect.mode == RgbMode::Off {
        return Ok(RegionalAnimation {
            frames: vec![vec![[0; 3]; fans * 40]; 30],
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
    ensure!(
        MODES.contains(&effect.mode),
        "unsupported SL Wireless mode: {:?}",
        effect.mode
    );
    let integration = effect.scope == RgbScope::All;
    let frames = match (effect.mode, integration) {
        (RgbMode::Rainbow, true) => integration::rainbow(effect, fans, side),
        (RgbMode::Runway, true) => integration::runway(effect, fans, side),
        (RgbMode::Meteor, true) => integration::meteor(effect, fans, side),
        (RgbMode::ColorCycle, true) => integration::color_cycle(effect, fans, side),
        (RgbMode::Render, true) => integration::render_effect(effect, fans, side),
        (RgbMode::Rainbow, _) => basic::rainbow(effect, fans, side),
        (RgbMode::Static, _) => basic::static_color(effect, fans, side),
        (RgbMode::RainbowMorph, _) => basic::rainbow_morph(effect, fans, side),
        (RgbMode::Breathing, _) => basic::breathing(effect, fans, side),
        (RgbMode::Runway, _) => movement::runway(effect, fans, side),
        (RgbMode::Meteor, _) => movement::meteor(effect, fans, side),
        (RgbMode::ColorCycle, _) => movement::color_cycle(effect, fans, side),
        (RgbMode::Staggered, _) => patterns::staggered(effect, fans, side),
        (RgbMode::Tide, _) => patterns::tide(effect, fans, side),
        (RgbMode::Mixing, _) => fills::mixing(effect, fans, side),
        (RgbMode::Render, _) => fills::render_effect(effect, fans, side),
        (RgbMode::PingPong, _) => fills::ping_pong(effect, fans, side),
        (RgbMode::Stack, _) => fills::stack(effect, fans, side),
        (RgbMode::Ripple, _) => collisions::ripple(effect, fans, side),
        (RgbMode::Collide, _) => collisions::collide(effect, fans, side),
        (RgbMode::Reflect, _) => collisions::reflect(effect, fans, side),
        (RgbMode::ElectricCurrent, _) => collisions::electric_current(effect, fans, side),
        (RgbMode::Endless, _) => flow_patterns::endless(effect, fans, side),
        (RgbMode::River, _) => flow_patterns::river(effect, fans, side),
        (RgbMode::Duel, _) => flow_patterns::duel(effect, fans, side),
        (RgbMode::Hourglass, _) => flow_patterns::hourglass(effect, fans, side),
        (RgbMode::Pioneer, _) => chase_patterns::pioneer(effect, fans, side),
        (RgbMode::ShuttleRun, _) => chase_patterns::shuttle_run(effect, fans, side),
        (RgbMode::GradientRibbon, _) => chase_patterns::gradient_ribbon(effect, fans, side),
        (RgbMode::Twinkle, _) => twinkle::render(effect, fans, side),
        _ => unreachable!(),
    };
    Ok(RegionalAnimation {
        frames,
        interval_base: match effect.mode {
            RgbMode::Stack => 15,
            RgbMode::River => 30,
            RgbMode::Pioneer | RgbMode::GradientRibbon => 20,
            _ => 11,
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
        let source = (frame * primary_ticks as usize / secondary_ticks).min(shorter.len() - 1);
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
    let target = target.as_chunks_mut::<40>().0;
    let source = source.as_chunks::<40>().0;
    for (target, source) in target.iter_mut().zip(source) {
        match side {
            Side::Inner => {
                target[..12].copy_from_slice(&source[..12]);
                target[20..32].copy_from_slice(&source[20..32]);
            }
            Side::Outer => {
                target[12..20].copy_from_slice(&source[12..20]);
                target[32..40].copy_from_slice(&source[32..40]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composition_reports_native_secondary_timing() {
        let inner = RegionalAnimation {
            frames: vec![vec![[1; 3]; 40]; 30],
            interval_base: 11,
        };
        let outer = RegionalAnimation {
            frames: vec![vec![[2; 3]; 40]; 127],
            interval_base: 11,
        };
        let animation = combine(inner, outer, 2).unwrap();
        assert_eq!(animation.frames.len(), 127);
        assert_eq!(animation.interval_hundredths, 5500);
        assert_eq!(
            animation.secondary,
            Some(SecondaryTiming {
                interval_ticks: 58,
                frame_count: 120,
                outer_longest: true
            })
        );
        assert_eq!(animation.frames[0][0], [1; 3]);
        assert_eq!(animation.frames[0][12], [2; 3]);
    }

    #[test]
    fn composition_uses_the_last_edited_region_speed() {
        let regions = [
            RgbRegionConfig {
                effect: RgbEffect {
                    mode: RgbMode::Static,
                    scope: RgbScope::Outer,
                    speed: 0,
                    brightness: 4,
                    ..Default::default()
                },
                flip: false,
            },
            RgbRegionConfig {
                effect: RgbEffect {
                    mode: RgbMode::Static,
                    scope: RgbScope::Inner,
                    speed: 4,
                    brightness: 4,
                    ..Default::default()
                },
                flip: false,
            },
        ];
        let animation = render(&regions, 1).unwrap();
        assert_eq!(animation.interval_hundredths, 3_300);
    }

    #[test]
    fn a_single_region_initializes_the_other_region_from_the_same_effect() {
        let animation = render(
            &[RgbRegionConfig {
                effect: RgbEffect {
                    mode: RgbMode::Static,
                    scope: RgbScope::Outer,
                    colors: vec![[255, 0, 0]],
                    brightness: 4,
                    ..Default::default()
                },
                flip: false,
            }],
            1,
        )
        .unwrap();
        assert_eq!(animation.frames[0][0], [254, 0, 0]);
        assert_eq!(animation.frames[0][12], [254, 0, 0]);
    }

    #[test]
    fn brightness_off_preserves_the_native_timeline_as_black_frames() {
        let animation = render(
            &[RgbRegionConfig {
                effect: RgbEffect {
                    mode: RgbMode::GradientRibbon,
                    scope: RgbScope::All,
                    brightness: 255,
                    ..Default::default()
                },
                flip: false,
            }],
            1,
        )
        .unwrap();
        assert_eq!(animation.frames.len(), 96);
        assert!(animation
            .frames
            .iter()
            .flatten()
            .all(|color| *color == [0; 3]));
    }

    #[test]
    fn whole_effects_keep_the_native_single_timeline_metadata() {
        let animation = render(
            &[RgbRegionConfig {
                effect: RgbEffect {
                    mode: RgbMode::Pioneer,
                    scope: RgbScope::All,
                    brightness: 4,
                    speed: 2,
                    ..Default::default()
                },
                flip: false,
            }],
            1,
        )
        .unwrap();
        assert_eq!(animation.frames.len(), 12);
        assert_eq!(animation.interval_hundredths, 10_000);
        assert_eq!(animation.secondary, None);
    }
}
