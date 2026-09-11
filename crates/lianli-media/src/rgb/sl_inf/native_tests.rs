use super::{render_region, InfVariant};
use crate::rgb::sl_inf::engine::Plane;
use lianli_shared::rgb::{RgbDirection, RgbEffect, RgbMode, RgbRegionConfig, RgbScope};

const CASES: &[(RgbMode, u64)] = &[
    (RgbMode::Rainbow, 0x2df8_9927_5e3d_7bf5),
    (RgbMode::RainbowMorph, 0x7de7_6a83_c0d7_af05),
    (RgbMode::Static, 0x5cc2_afb5_d1eb_3725),
    (RgbMode::Breathing, 0x0a47_2c38_5aef_5505),
    (RgbMode::Runway, 0x1d73_bea5_50c5_e465),
    (RgbMode::Meteor, 0x4299_7980_01ec_91a5),
    (RgbMode::Twinkle, 0xc667_1ce7_5c55_2215),
    (RgbMode::TaiChi, 0x2960_8dad_95d6_6625),
    (RgbMode::ColorCycle, 0xe972_6c97_c707_6825),
    (RgbMode::MopUp, 0xa9e3_99b9_ab5b_f465),
    (RgbMode::MeteorRainbow, 0x9c07_e39a_9e7c_d6ed),
    (RgbMode::ColorfulMeteor, 0xe901_1946_ef76_4cd5),
    (RgbMode::Lottery, 0xaf44_6859_b993_2465),
    (RgbMode::Warning, 0xd115_0859_e25c_9325),
    (RgbMode::Voice, 0x4bf3_ac97_5d35_58e5),
    (RgbMode::Mixing, 0x946a_dcec_0632_ca15),
    (RgbMode::Tide, 0xe0a3_ff97_834f_5565),
    (RgbMode::Scan, 0xb383_c7fa_1b90_e025),
    (RgbMode::DoubleMeteor, 0x5b33_dd26_614a_a805),
    (RgbMode::MeteorContest, 0xd9ab_72e7_7328_8d65),
    (RgbMode::MeteorMix, 0xd16a_5f76_14dc_f715),
    (RgbMode::ReturnArc, 0x4645_bc1e_be78_6725),
    (RgbMode::DoubleArc, 0xef2f_3ac6_8765_fba5),
    (RgbMode::Door, 0x2d11_ec1b_3592_1a55),
    (RgbMode::HeartBeat, 0x7b46_bb3c_0f29_03e5),
    (RgbMode::HeartBeatRunway, 0xec82_8d2a_7ce9_ea75),
    (RgbMode::Disco, 0x0eea_75a5_d1ef_9885),
    (RgbMode::ElectricCurrent, 0xe813_bcce_71fa_488d),
    (RgbMode::Reflect, 0x9e46_37ce_be20_fce5),
    (RgbMode::GradientRibbon, 0x85d4_a325_49fb_24a5),
    (RgbMode::Wing, 0xdd31_add2_2aa9_24c5),
    (RgbMode::Drumming, 0x36d3_580e_e3fd_df25),
    (RgbMode::Boomerang, 0xbfd1_bd65_20f7_7c75),
    (RgbMode::CandyBox, 0x7cfd_b84e_8b41_dda5),
];

fn effect(mode: RgbMode, brightness: u8, direction: RgbDirection) -> RgbEffect {
    RgbEffect {
        mode,
        colors: vec![[211, 17, 67], [13, 199, 29], [31, 47, 173], [107, 83, 23]],
        brightness,
        direction,
        ..Default::default()
    }
}

#[test]
fn native_modes_match_extracted_csharp_matrices() {
    for &(mode, expected) in CASES {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for fans in 1..=4 {
            for plane in [Plane::Outer, Plane::Inner, Plane::Center] {
                for brightness in 0..=4 {
                    for direction in [RgbDirection::Clockwise, RgbDirection::CounterClockwise] {
                        for pn in 0..=1 {
                            let animation = render_region(
                                &effect(mode, brightness, direction),
                                fans,
                                plane,
                                pn,
                            )
                            .unwrap();
                            for channel in animation.frames.iter().flatten().flatten() {
                                hash ^= u64::from(*channel);
                                hash = hash.wrapping_mul(0x100_0000_01b3);
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(hash, expected, "{mode:?}");
    }
}

#[test]
fn attachment_variant_maps_the_native_pn_without_reversing_fan_chunks() {
    let region = RgbRegionConfig {
        effect: RgbEffect {
            mode: RgbMode::TaiChi,
            colors: vec![[255, 0, 0], [0, 255, 0]],
            brightness: 4,
            scope: RgbScope::All,
            ..Default::default()
        },
        flip: false,
    };
    let original_right =
        super::render(std::slice::from_ref(&region), 2, true, InfVariant::Original).unwrap();
    let v3_left = super::render(std::slice::from_ref(&region), 2, false, InfVariant::V3).unwrap();
    assert_eq!(original_right.frames, v3_left.frames);
    assert_eq!(&original_right.frames[0][18..22], &[[0, 254, 0]; 4]);
    assert_eq!(&original_right.frames[0][62..66], &[[0, 254, 0]; 4]);

    let v3_right = super::render(std::slice::from_ref(&region), 2, true, InfVariant::V3).unwrap();
    assert_eq!(&v3_right.frames[0][18..22], &[[254, 0, 0]; 4]);
    assert_ne!(original_right.frames, v3_right.frames);
}

#[test]
fn combined_timeline_uses_the_inner_regions_speed() {
    let region = |scope, mode, speed| RgbRegionConfig {
        effect: RgbEffect {
            mode,
            speed,
            brightness: 4,
            scope,
            ..Default::default()
        },
        flip: false,
    };
    let mut regions = vec![
        region(RgbScope::Inner, RgbMode::Static, 0),
        region(RgbScope::Outer, RgbMode::Static, 2),
        region(RgbScope::Center, RgbMode::RainbowMorph, 4),
    ];
    let slow_inner = super::render(&regions, 1, false, InfVariant::Original).unwrap();
    assert_eq!(slow_inner.interval_hundredths, 7_700);

    regions[2].effect.speed = 0;
    let changed_center = super::render(&regions, 1, false, InfVariant::Original).unwrap();
    assert_eq!(changed_center.interval_hundredths, 7_700);

    regions[0].effect.speed = 4;
    let fast_inner = super::render(&regions, 1, false, InfVariant::Original).unwrap();
    assert_eq!(fast_inner.interval_hundredths, 3_300);
}

#[test]
fn native_frame_counts_follow_each_selected_track_width() {
    let cases = [
        (RgbMode::Rainbow, [8, 10, 8]),
        (RgbMode::RainbowMorph, [127, 127, 127]),
        (RgbMode::Static, [30, 30, 30]),
        (RgbMode::Breathing, [170, 170, 170]),
        (RgbMode::Runway, [18, 22, 18]),
        (RgbMode::Meteor, [36, 44, 36]),
        (RgbMode::Twinkle, [200, 200, 200]),
        (RgbMode::TaiChi, [8, 10, 8]),
        (RgbMode::ColorCycle, [32, 40, 32]),
        (RgbMode::MopUp, [40, 48, 40]),
        (RgbMode::MeteorRainbow, [9, 11, 9]),
        (RgbMode::ColorfulMeteor, [9, 11, 9]),
        (RgbMode::Lottery, [10, 12, 8]),
        (RgbMode::Warning, [64, 64, 64]),
        (RgbMode::Voice, [60, 76, 60]),
        (RgbMode::Mixing, [11, 12, 11]),
        (RgbMode::Tide, [20, 24, 20]),
        (RgbMode::Scan, [20, 24, 20]),
        (RgbMode::DoubleMeteor, [16, 20, 16]),
        (RgbMode::MeteorContest, [8, 10, 8]),
        (RgbMode::MeteorMix, [8, 10, 8]),
        (RgbMode::ReturnArc, [64, 80, 64]),
        (RgbMode::DoubleArc, [64, 80, 64]),
        (RgbMode::Door, [32, 32, 32]),
        (RgbMode::HeartBeat, [42, 42, 42]),
        (RgbMode::HeartBeatRunway, [168, 168, 168]),
        (RgbMode::Disco, [8, 10, 8]),
        (RgbMode::ElectricCurrent, [68, 72, 68]),
        (RgbMode::Reflect, [32, 36, 32]),
        (RgbMode::GradientRibbon, [48, 48, 48]),
        (RgbMode::Wing, [10, 10, 10]),
        (RgbMode::Drumming, [108, 108, 108]),
        (RgbMode::Boomerang, [18, 18, 18]),
        (RgbMode::CandyBox, [170, 170, 170]),
    ];
    for (mode, expected) in cases {
        for (index, plane) in [Plane::Outer, Plane::Inner, Plane::Center]
            .into_iter()
            .enumerate()
        {
            let animation =
                render_region(&effect(mode, 4, RgbDirection::Clockwise), 1, plane, 1).unwrap();
            assert_eq!(
                animation.frames.len(),
                expected[index],
                "{mode:?} {plane:?}"
            );
        }
    }
}
