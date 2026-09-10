use super::*;
use lianli_shared::rgb::{RgbRenderFamily as Family, RgbRenderProfile};

#[test]
fn source_capacity_stack_survives_projection_and_upload_preparation() {
    let effect = RgbEffect {
        mode: RgbMode::Stack,
        colors: vec![[120, 30, 50]],
        ..Default::default()
    };
    let animation = sync_effects::render(&effect, 307).unwrap();
    assert_eq!(animation.frames.len(), 4082);
    let layout = Layout::for_profile(RgbRenderProfile {
        family: Family::Tl,
        fan_count: 1,
        led_count: 26,
        right_attach: false,
    })
    .unwrap();
    let frames = animation
        .frames
        .iter()
        .map(|frame| layout.project_frame(frame, 0..13, false).unwrap())
        .collect::<Vec<_>>();
    let upload = WirelessRgbUpload::with_timing(&frames, animation.timing(), None).unwrap();
    assert_eq!(upload.frame_count(), 4082);
    assert!(sync_effects::render(&effect, 308).is_err());
}

#[test]
fn native_sync_modes_cover_every_software_family() {
    for (family, fan_count, led_count) in [
        (Family::Tl, 3, 78),
        (Family::Sl, 3, 120),
        (Family::SlInf, 3, 132),
        (Family::SlInfV3, 3, 132),
        (Family::SlV4, 3, 156),
        (Family::Cl, 3, 72),
        (Family::P28, 3, 27),
        (Family::Strimer, 0, 116),
        (Family::HydroShiftII, 3, 96),
        (Family::HydroShiftII, 0, 24),
        (Family::HydroShiftIIOled, 0, 45),
        (Family::UniversalScreen, 0, 60),
        (Family::Lancool217, 0, 96),
        (Family::LancoolV150, 4, 88),
    ] {
        let profile = RgbRenderProfile {
            family,
            fan_count,
            led_count,
            right_attach: false,
        };
        for mode in MATCHED_MODES.iter().copied().chain([RgbMode::Twinkle]) {
            let effect = RgbEffect {
                mode,
                colors: vec![[255, 0, 0], [0, 255, 0]],
                ..Default::default()
            };
            let animation = native_animation(profile, &effect)
                .unwrap_or_else(|e| panic!("{family:?} {mode:?}: {e:#}"));
            assert!(!animation.frames.is_empty());
            assert!(
                animation
                    .frames
                    .iter()
                    .all(|frame| frame.len() == usize::from(led_count)),
                "{family:?} {mode:?}"
            );
        }
    }
}

#[test]
fn sync_configuration_rejects_duplicate_devices_and_invalid_controls() {
    let mut sync = MergeLightingConfig {
        enabled: true,
        device_order: vec!["a".into(), "a".into()],
        ..Default::default()
    };
    assert!(validate_settings(&sync).is_err());
    sync.device_order.pop();
    assert!(validate_settings(&sync).is_ok());
    sync.effect.brightness = 5;
    assert!(validate_settings(&sync).is_err());
    sync.effect.brightness = 255;
    assert!(validate_settings(&sync).is_ok());
    sync.effect.mode = RgbMode::Voice;
    assert!(validate_settings(&sync).is_err());
}
