use super::{family, parameters};
use lianli_shared::rgb::{
    RgbEffect, RgbMode, RgbRegionConfig, RgbRenderFamily, RgbRenderProfile, RgbScope,
};

#[test]
fn variable_screen_cycles_scale_with_the_selected_palette_length() {
    for (family, led_count) in [
        (RgbRenderFamily::UniversalScreen, 60),
        (RgbRenderFamily::UniversalScreen, 88),
        (RgbRenderFamily::HydroShiftIIOled, 45),
    ] {
        let profile = RgbRenderProfile {
            family,
            fan_count: 0,
            led_count,
            right_attach: false,
        };
        for controls in parameters::for_scope(profile, RgbScope::All) {
            if controls.min_colors == controls.max_colors {
                continue;
            }
            assert_eq!(controls.min_colors, 1);
            let mut region = RgbRegionConfig {
                effect: RgbEffect {
                    mode: controls.mode,
                    colors: vec![[255, 0, 0]],
                    brightness: 4,
                    ..Default::default()
                },
                flip: false,
            };
            let single = family::render(profile, &[region.clone()]).unwrap();
            for count in 2..=controls.max_colors {
                region.effect.colors = vec![[255, 0, 0]; usize::from(count)];
                let animation = family::render(profile, &[region.clone()]).unwrap();
                assert_eq!(
                    animation.frames.len(),
                    single.frames.len() * usize::from(count),
                    "{family:?}/{:?}/{count}",
                    controls.mode
                );
                assert_eq!(
                    animation.timing().interval_hundredths,
                    single.timing().interval_hundredths
                );
            }
            if matches!(controls.mode, RgbMode::Wave | RgbMode::BlowUp) {
                region.effect.colors = vec![[255, 0, 0], [0, 255, 0]];
                let animation = family::render(profile, &[region.clone()]).unwrap();
                for (phase, channel) in animation
                    .frames
                    .chunks_exact(single.frames.len())
                    .zip([0, 1])
                {
                    assert!(phase.iter().flatten().any(|color| color[channel] > 0));
                    assert!(phase
                        .iter()
                        .flatten()
                        .all(|color| color[1 - channel] == 0 && color[2] == 0));
                    let mut dark_run = 0;
                    for frame in phase {
                        dark_run = if frame.iter().all(|color| *color == [0; 3]) {
                            dark_run + 1
                        } else {
                            0
                        };
                        assert!(
                            dark_run <= 11,
                            "{family:?}/{:?}: unexpected blank color phase",
                            controls.mode
                        );
                    }
                }
            }
            region.effect.colors.clear();
            let empty = family::render(profile, &[region.clone()]).unwrap();
            assert!(!empty.frames.is_empty());
            region.effect.colors.push([0; 3]);
            let black = family::render(profile, &[region]).unwrap();
            assert_eq!(
                empty.frames, black.frames,
                "{family:?}/{:?}: empty palette",
                controls.mode
            );
        }
    }
}
