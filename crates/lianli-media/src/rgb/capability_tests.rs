use super::{family, parameters};
use lianli_shared::rgb::{RgbEffect, RgbRegionConfig, RgbRenderFamily, RgbRenderProfile};

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
