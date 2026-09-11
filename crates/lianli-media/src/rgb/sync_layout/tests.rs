use lianli_shared::rgb::{RgbRenderFamily, RgbRenderProfile};

use super::Layout;

fn profile(
    family: RgbRenderFamily,
    fan_count: u8,
    led_count: u16,
    right_attach: bool,
) -> RgbRenderProfile {
    RgbRenderProfile {
        family,
        fan_count,
        led_count,
        right_attach,
    }
}

fn indexed_frame(count: usize) -> Vec<[u8; 3]> {
    (0..count)
        .map(|index| [index as u8, index as u8, index as u8])
        .collect()
}

fn indices(frame: &[[u8; 3]]) -> Vec<u8> {
    frame.iter().map(|color| color[0]).collect()
}

fn projected_indices(layout: &Layout) -> Vec<u8> {
    indices(
        &layout
            .project_frame(
                &indexed_frame(layout.logical_led_count()),
                0..layout.logical_led_count(),
                false,
            )
            .unwrap(),
    )
}

#[test]
fn reports_vendor_logical_and_transport_counts() {
    let cases = [
        (RgbRenderFamily::Tl, 3, 78, 39, 78),
        (RgbRenderFamily::Sl, 2, 80, 24, 80),
        (RgbRenderFamily::SlInf, 4, 176, 32, 176),
        (RgbRenderFamily::SlInfV3, 3, 132, 24, 132),
        (RgbRenderFamily::SlV4, 1, 52, 13, 52),
        (RgbRenderFamily::Cl, 2, 48, 16, 48),
        (RgbRenderFamily::P28, 2, 18, 18, 18),
        (RgbRenderFamily::HydroShiftII, 3, 96, 48, 96),
        (RgbRenderFamily::HydroShiftIIOled, 0, 35, 14, 35),
        (RgbRenderFamily::HydroShiftIIOled, 0, 45, 14, 35),
        (RgbRenderFamily::UniversalScreen, 0, 60, 31, 60),
        (RgbRenderFamily::UniversalScreen, 0, 88, 31, 60),
        (RgbRenderFamily::Lancool217, 0, 96, 96, 96),
        (RgbRenderFamily::LancoolV150, 4, 88, 88, 88),
    ];

    for (family, fan_count, led_count, logical, physical) in cases {
        let layout = Layout::for_profile(profile(family, fan_count, led_count, false)).unwrap();
        assert_eq!(layout.logical_led_count(), logical, "{family:?}");
        assert_eq!(layout.physical_led_count(), physical, "{family:?}");
    }
}

#[test]
fn tl_duplicates_each_logical_fan_path_and_reverses_the_whole_span() {
    let layout = Layout::for_profile(profile(RgbRenderFamily::Tl, 2, 52, false)).unwrap();
    let source = indexed_frame(36);
    let forward = layout.project_frame(&source, 5..31, false).unwrap();
    let reverse = layout.project_frame(&source, 5..31, true).unwrap();

    assert_eq!(
        &indices(&forward)[0..26],
        &[
            5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
            16, 17
        ]
    );
    assert_eq!(
        &indices(&forward)[26..52],
        &[
            18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 18, 19, 20, 21, 22, 23, 24, 25, 26,
            27, 28, 29, 30
        ]
    );
    assert_eq!(
        &indices(&reverse)[0..13],
        &[30, 29, 28, 27, 26, 25, 24, 23, 22, 21, 20, 19, 18]
    );
}

#[test]
fn sl_uses_the_vendor_side_led_sampling_table() {
    let layout = Layout::for_profile(profile(RgbRenderFamily::Sl, 1, 40, false)).unwrap();
    assert_eq!(
        projected_indices(&layout),
        [
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 0, 1, 3, 5, 6, 8, 10, 11, 0, 1, 2, 3, 4, 5, 6, 7,
            8, 9, 10, 11, 0, 1, 3, 5, 6, 8, 10, 11,
        ]
    );
}

#[test]
fn sl_inf_attachment_changes_the_physical_path() {
    let left = Layout::for_profile(profile(RgbRenderFamily::SlInf, 1, 44, false)).unwrap();
    let right = Layout::for_profile(profile(RgbRenderFamily::SlInfV3, 1, 44, true)).unwrap();

    assert_eq!(
        projected_indices(&left),
        [
            3, 5, 7, 6, 4, 2, 0, 1, 0, 1, 2, 2, 3, 4, 5, 6, 6, 7, 0, 1, 2, 3, 4, 5, 6, 7, 0, 1, 2,
            2, 3, 4, 5, 6, 6, 7, 0, 1, 2, 3, 4, 5, 6, 7,
        ]
    );
    assert_eq!(
        projected_indices(&right),
        [
            4, 2, 0, 1, 3, 5, 7, 6, 7, 6, 5, 5, 4, 3, 2, 1, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5,
            5, 4, 3, 2, 1, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0,
        ]
    );
}

#[test]
fn cl_and_hydroshift_fans_share_the_three_path_projection() {
    let cl = Layout::for_profile(profile(RgbRenderFamily::Cl, 1, 24, false)).unwrap();
    assert_eq!(
        projected_indices(&cl),
        [0, 1, 2, 3, 4, 5, 6, 7, 7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0,]
    );

    let h2 = Layout::for_profile(profile(RgbRenderFamily::HydroShiftII, 1, 48, false)).unwrap();
    assert_eq!(&projected_indices(&h2)[0..24], &(0..24).collect::<Vec<_>>());
    assert_eq!(
        &projected_indices(&h2)[24..48],
        &[
            24, 25, 26, 27, 28, 29, 30, 31, 31, 30, 29, 28, 27, 26, 25, 24, 31, 30, 29, 28, 27, 26,
            25, 24
        ]
    );
}

#[test]
fn sl_v4_preserves_the_black_half_of_partially_populated_fan_groups() {
    let sl_v4 = Layout::for_profile(profile(RgbRenderFamily::SlV4, 1, 52, false)).unwrap();
    let sl_v4_frame = sl_v4
        .project_frame(&indexed_frame(13), 0..13, false)
        .unwrap();
    assert_eq!(
        &indices(&sl_v4_frame)[0..26],
        &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
    );
    assert!(sl_v4_frame[26..].iter().all(|color| *color == [0; 3]));

    let p28 = Layout::for_profile(profile(RgbRenderFamily::P28, 2, 18, false)).unwrap();
    let p28_frame = p28.project_frame(&indexed_frame(18), 0..18, false).unwrap();
    assert_eq!(indices(&p28_frame), (0..18).collect::<Vec<_>>());
}

#[test]
fn strimer_transmits_four_or_six_repeated_logical_lanes() {
    for (native_count, logical_count) in [(116, 29), (174, 29), (88, 22), (132, 22)] {
        let layout =
            Layout::for_profile(profile(RgbRenderFamily::Strimer, 0, native_count, false)).unwrap();
        let projected = projected_indices(&layout);
        assert_eq!(layout.logical_led_count(), logical_count);
        assert_eq!(layout.physical_led_count(), native_count as usize);
        let transmitted_lanes = native_count as usize / logical_count;
        for lane in 0..transmitted_lanes {
            assert_eq!(
                &projected[lane * logical_count..(lane + 1) * logical_count],
                &(0..logical_count as u8).collect::<Vec<_>>()
            );
        }
        assert_eq!(
            transmitted_lanes,
            if matches!(native_count, 116 | 88) {
                4
            } else {
                6
            }
        );
    }
}

#[test]
fn screen_ring_projections_match_the_transmitted_led_order() {
    let universal =
        Layout::for_profile(profile(RgbRenderFamily::UniversalScreen, 0, 88, false)).unwrap();
    assert_eq!(
        projected_indices(&universal),
        [
            13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 29, 28, 27, 26,
            25, 24, 23, 22, 21, 20, 19, 18, 17, 16, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2,
            1, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
        ]
    );

    let oled =
        Layout::for_profile(profile(RgbRenderFamily::HydroShiftIIOled, 0, 35, false)).unwrap();
    let projected = oled
        .project_frame(&indexed_frame(14), 0..14, false)
        .unwrap();
    assert_eq!(
        &indices(&projected)[0..14],
        &[13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0]
    );
    assert!(projected[14..21].iter().all(|color| *color == [0; 3]));
    assert_eq!(&indices(&projected)[21..35], &(0..14).collect::<Vec<_>>());
}

#[test]
fn rejects_invalid_profiles_and_source_spans() {
    assert!(Layout::for_profile(profile(RgbRenderFamily::Tl, 0, 0, false)).is_none());
    assert!(Layout::for_profile(profile(RgbRenderFamily::Tl, 2, 51, false)).is_none());
    assert!(Layout::for_profile(profile(RgbRenderFamily::Strimer, 0, 100, false)).is_none());

    let layout = Layout::for_profile(profile(RgbRenderFamily::Cl, 1, 24, false)).unwrap();
    let source = indexed_frame(20);
    assert!(layout.project_frame(&source, 0..7, false).is_err());
    assert!(layout.project_frame(&source, 13..21, false).is_err());
    let reversed_span = std::ops::Range { start: 8, end: 4 };
    assert!(layout.project_frame(&source, reversed_span, false).is_err());
}
