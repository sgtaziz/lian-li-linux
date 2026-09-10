use super::{family, parameters};
use lianli_shared::rgb::{RgbEffect, RgbRegionConfig, RgbRenderFamily, RgbRenderProfile};

#[test]
fn every_screen_palette_effect_preserves_short_configuration_playback() {
    use lianli_shared::rgb::RgbScope;
    for (family, led_count) in [
        (RgbRenderFamily::UniversalScreen, 60),
        (RgbRenderFamily::UniversalScreen, 88),
        (RgbRenderFamily::HydroShiftIIOled, 45),
    ] {
        let profile = RgbRenderProfile {
            family,
            led_count,
            fan_count: 0,
            right_attach: false,
        };
        for controls in parameters::for_scope(profile, RgbScope::All) {
            assert_eq!(controls.min_colors, controls.max_colors);
            for count in 1..controls.min_colors {
                let mut region = RgbRegionConfig {
                    effect: RgbEffect {
                        mode: controls.mode,
                        colors: vec![[23, 170, 91]; usize::from(count)],
                        brightness: 4,
                        ..Default::default()
                    },
                    flip: false,
                };
                let short = family::render(profile, &[region.clone()]).unwrap();
                region
                    .effect
                    .colors
                    .resize(usize::from(controls.min_colors), [23, 170, 91]);
                let full = family::render(profile, &[region]).unwrap();
                assert_eq!(
                    short.frames, full.frames,
                    "{family:?}/{:?}, {count} colors",
                    controls.mode
                );
                assert_eq!(short.timing(), full.timing());
            }
        }
    }
}

#[test]
fn screen_wave_extends_legacy_palettes_without_inventing_black_slots() {
    use lianli_shared::rgb::{RgbMode, RgbScope};
    for (family, led_count) in [
        (RgbRenderFamily::UniversalScreen, 60),
        (RgbRenderFamily::UniversalScreen, 88),
        (RgbRenderFamily::HydroShiftIIOled, 45),
    ] {
        let profile = RgbRenderProfile {
            family,
            led_count,
            fan_count: 0,
            right_attach: false,
        };
        let controls = parameters::for_scope(profile, RgbScope::All)
            .into_iter()
            .find(|controls| controls.mode == RgbMode::Wave)
            .unwrap();
        assert_eq!((controls.min_colors, controls.max_colors), (6, 6));
        let mut region = RgbRegionConfig {
            effect: RgbEffect {
                mode: RgbMode::Wave,
                colors: vec![[255, 0, 0]],
                brightness: 4,
                ..Default::default()
            },
            flip: false,
        };
        let legacy = family::render(profile, &[region.clone()]).unwrap();
        region.effect.colors = vec![[255, 0, 0]; 6];
        let full = family::render(profile, &[region.clone()]).unwrap();
        assert_eq!(legacy.frames, full.frames);
        assert!(legacy
            .frames
            .iter()
            .all(|frame| frame.iter().any(|color| *color != [0; 3])));
        region.effect.colors[5] = [0; 3];
        let explicit_black = family::render(profile, &[region]).unwrap();
        assert!(explicit_black
            .frames
            .iter()
            .any(|frame| frame.iter().all(|color| *color == [0; 3])));
    }
}

#[test]
fn advertised_region_controls_produce_valid_native_layouts() {
    use RgbRenderFamily::*;
    for (family, fan_count, led_count) in [
        (Tl, 3, 78),
        (Sl, 3, 120),
        (SlV4, 3, 156),
        (SlInf, 3, 132),
        (SlInfV3, 3, 132),
        (Cl, 3, 72),
        (P28, 3, 27),
        (HydroShiftII, 3, 96),
        (UniversalScreen, 0, 60),
        (HydroShiftIIOled, 0, 45),
        (Lancool217, 0, 96),
        (LancoolV150, 0, 88),
        (Strimer, 0, 88),
        (Strimer, 0, 116),
        (Strimer, 0, 132),
        (Strimer, 0, 174),
    ] {
        let profile = RgbRenderProfile {
            family,
            fan_count,
            led_count,
            right_attach: false,
        };
        for region in parameters::for_regions(profile) {
            for controls in region.effects {
                let effect = RgbEffect {
                    mode: controls.mode,
                    scope: region.scope,
                    colors: vec![[255, 0, 0]; controls.min_colors as usize],
                    brightness: 4,
                    speed: 2,
                    ..Default::default()
                };
                let animation = family::render(
                    profile,
                    &[RgbRegionConfig {
                        effect: effect.clone(),
                        flip: false,
                    }],
                )
                .unwrap_or_else(|error| {
                    panic!(
                        "{family:?}/{:?}/{:?}: {error:#}",
                        region.scope, controls.mode
                    )
                });
                assert!(!animation.frames.is_empty());
                assert!(animation
                    .frames
                    .iter()
                    .all(|frame| frame.len() == led_count as usize));
                assert!(animation.interval_hundredths > 0);
                if region.scope == lianli_shared::rgb::RgbScope::All {
                    let mut off = effect;
                    off.brightness = 255;
                    let animation = family::render(
                        profile,
                        &[RgbRegionConfig {
                            effect: off,
                            flip: false,
                        }],
                    )
                    .unwrap();
                    assert!(
                        animation
                            .frames
                            .iter()
                            .flatten()
                            .all(|color| *color == [0; 3]),
                        "{family:?}/{:?}: brightness Off must extinguish the whole device",
                        controls.mode
                    );
                }
            }
        }
    }
}

#[test]
fn unsupported_sl_scope_combinations_are_rejected_before_upload() {
    use lianli_shared::rgb::{RgbMode, RgbScope};
    let profile = RgbRenderProfile {
        family: RgbRenderFamily::Sl,
        fan_count: 3,
        led_count: 120,
        right_attach: false,
    };
    for (mode, scope) in [
        (RgbMode::Mixing, RgbScope::All),
        (RgbMode::GradientRibbon, RgbScope::Inner),
    ] {
        assert!(family::render(
            profile,
            &[RgbRegionConfig {
                effect: RgbEffect {
                    mode,
                    scope,
                    ..Default::default()
                },
                flip: false,
            }]
        )
        .is_err());
    }
}

#[test]
fn device_engines_reject_generic_directions_without_a_native_mapping() {
    use lianli_shared::rgb::{RgbDirection, RgbMode};
    assert!(family::render(
        RgbRenderProfile {
            family: RgbRenderFamily::Tl,
            fan_count: 3,
            led_count: 78,
            right_attach: false,
        },
        &[RgbRegionConfig {
            effect: RgbEffect {
                mode: RgbMode::Rainbow,
                direction: RgbDirection::Spread,
                ..Default::default()
            },
            flip: false
        }]
    )
    .is_err());
}
