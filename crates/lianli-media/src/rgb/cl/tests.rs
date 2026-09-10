use super::*;

fn region(scope: RgbScope, mode: RgbMode) -> RgbRegionConfig {
    RgbRegionConfig {
        effect: RgbEffect {
            mode,
            colors: vec![[255, 0, 0]],
            brightness: 4,
            speed: 2,
            scope,
            ..Default::default()
        },
        flip: false,
    }
}

#[test]
fn caller_scopes_select_the_recovered_cl_planes() {
    let center = render(&[region(RgbScope::Center, RgbMode::Static)], 1).unwrap();
    assert!(center.frames[0][..8]
        .iter()
        .all(|&color| color == [254, 0, 0]));
    assert!(center.frames[0][8..].iter().all(|&color| color == [0; 3]));

    let outer = render(&[region(RgbScope::Outer, RgbMode::Static)], 1).unwrap();
    assert!(outer.frames[0][..8].iter().all(|&color| color == [0; 3]));
    assert!(outer.frames[0][8..]
        .iter()
        .all(|&color| color == [254, 0, 0]));

    let all = render(&[region(RgbScope::All, RgbMode::Static)], 1).unwrap();
    assert!(all.frames[0].iter().all(|&color| color == [254, 0, 0]));
}

#[test]
fn outer_longer_composition_preserves_the_vendor_interval_rescale() {
    let animation = render(
        &[
            region(RgbScope::Outer, RgbMode::Static),
            region(RgbScope::Center, RgbMode::Rainbow),
        ],
        1,
    )
    .unwrap();
    assert_eq!(animation.frames.len(), 30);
    assert_eq!(animation.interval_hundredths, 14_000);
    assert!(animation.frames[0][8..]
        .iter()
        .all(|&color| color == [254, 0, 0]));
    assert!(animation.frames[0][..8]
        .iter()
        .any(|&color| color != [0; 3]));
}

#[test]
fn outer_plane_keeps_mode_specific_mixing_and_door_phases() {
    let mix = render(&[region(RgbScope::Outer, RgbMode::MeteorMix)], 1).unwrap();
    assert_eq!(mix.frames.len(), 8);
    assert_eq!(mix.frames[4][11], [254, 0, 254]);
    assert_eq!(mix.frames[4][12], [254, 0, 254]);
    assert!(mix.frames[4][..8].iter().all(|&color| color == [0; 3]));

    let door = render(&[region(RgbScope::Outer, RgbMode::Door)], 1).unwrap();
    assert_eq!(door.frames.len(), 32);
    assert!(door.frames[0].iter().all(|&color| color == [0; 3]));
    assert!(door.frames[4][8..]
        .iter()
        .all(|&color| color == [254, 0, 0]));
}

#[test]
fn outer_ribbon_and_drumming_keep_independent_native_banks() {
    let ribbon = render(&[region(RgbScope::Outer, RgbMode::GradientRibbon)], 1).unwrap();
    assert_eq!(ribbon.frames.len(), 48);
    assert_eq!(ribbon.frames[0][8], [159, 95, 0]);
    assert_eq!(ribbon.frames[0][16], [0, 159, 95]);
    assert_ne!(ribbon.frames[0][8], ribbon.frames[0][16]);

    let drumming = render(&[region(RgbScope::Outer, RgbMode::Drumming)], 1).unwrap();
    assert_eq!(drumming.frames.len(), 108);
    assert!(drumming.frames[0][..8].iter().all(|&color| color == [0; 3]));
    assert!(drumming.frames[0][8..]
        .iter()
        .all(|&color| color == [49, 49, 0]));
}

#[test]
fn center_longer_composition_uses_outer_speed_and_center_interval_base() {
    let mut outer = region(RgbScope::Outer, RgbMode::Rainbow);
    outer.effect.speed = 0;
    let mut center = region(RgbScope::Center, RgbMode::Breathing);
    center.effect.speed = 4;
    let animation = render(&[outer, center], 1).unwrap();
    assert_eq!(animation.frames.len(), 170);
    assert_eq!(animation.interval_hundredths, 7_700);
}

fn recovered_frame_count(mode: RgbMode, fans: usize) -> usize {
    match mode {
        RgbMode::Off => 1,
        RgbMode::Rainbow => 8 * fans,
        RgbMode::RainbowMorph => 127,
        RgbMode::Static => 30,
        RgbMode::Breathing => 170,
        RgbMode::Runway => 20 * fans - 2,
        RgbMode::Meteor => 40 * fans - 4,
        RgbMode::Twinkle => 200,
        RgbMode::TaiChi => 8,
        RgbMode::ColorCycle => 32 * fans,
        RgbMode::MopUp => 40 * fans,
        RgbMode::MeteorRainbow | RgbMode::ColorfulMeteor => 10 * fans - 1,
        RgbMode::Lottery => 12 * fans - 2,
        RgbMode::Warning => 64,
        RgbMode::Voice => 8 * (8 * fans + 16 * fans / 3 + 8 * fans / 3),
        RgbMode::Mixing => 12 * fans - 1,
        RgbMode::Tide => 24 * fans - 4,
        RgbMode::Scan => 20 * fans,
        RgbMode::DoubleMeteor => 16,
        RgbMode::MeteorContest | RgbMode::MeteorMix => 8,
        RgbMode::ReturnArc => 64,
        RgbMode::DoubleArc => 32,
        RgbMode::Door => 32 * fans,
        RgbMode::HeartBeat => 128,
        RgbMode::HeartBeatRunway => [168, 180, 184, 186][fans - 1],
        RgbMode::Disco => 8,
        RgbMode::ElectricCurrent => 36 * fans + 32,
        RgbMode::Reflect => 48 * fans - 8,
        RgbMode::GradientRibbon => 48,
        RgbMode::Wing => 12 * fans - 2,
        RgbMode::Drumming => 108,
        RgbMode::Boomerang => 20 * fans - 2,
        RgbMode::CandyBox => 340,
        _ => unreachable!(),
    }
}

#[test]
fn all_native_modes_match_recovered_frame_matrix_and_region_composition() {
    for &mode in MODES {
        for fans in 1..=4 {
            let all = render(&[region(RgbScope::All, mode)], fans).unwrap();
            assert_eq!(
                all.frames.len(),
                recovered_frame_count(mode, fans),
                "{mode:?} with {fans} fans"
            );
            assert!(all.frames.iter().all(|frame| frame.len() == fans * 24));

            let split = render(
                &[
                    region(RgbScope::Outer, mode),
                    region(RgbScope::Center, mode),
                ],
                fans,
            )
            .unwrap();
            assert_eq!(split.frames, all.frames);
            assert_eq!(split.interval_hundredths, all.interval_hundredths);
        }
    }
}

fn hash_byte(hash: u64, value: u8) -> u64 {
    (hash ^ u64::from(value)).wrapping_mul(1_099_511_628_211)
}

fn hash_i32(mut hash: u64, value: usize) -> u64 {
    for shift in (0..32).step_by(8) {
        hash = hash_byte(hash, (value >> shift) as u8);
    }
    hash
}

#[test]
fn all_native_mode_bytes_match_the_extracted_csharp_matrix() {
    const EXPECTED: [u64; 34] = [
        0xf4f1de37ac865a8d,
        0x66e1d977e2d955c5,
        0x82278b67549a8fe5,
        0x62e956801ad48f95,
        0x7b046fe516267e25,
        0x3b90ce180c2b85e3,
        0x17c85e0b3879767d,
        0x8f9c8a4d95d60da5,
        0xa9344e700437a615,
        0x307203c6f33b700d,
        0x10e9e0a017888f29,
        0x835f6ab83a48b317,
        0x61a8560a9713b585,
        0xd0727e45bc0171e5,
        0x9cced1997c89b8d1,
        0xda9fc309e2cab61d,
        0xc8a42b4e49a6e5e5,
        0x1707ab5585d35135,
        0xfba4f34c138ff7e5,
        0x214f0cddc5b101f5,
        0xbdbc3ce80c8d09a5,
        0x306bbe682f6c5c85,
        0x21fd23d5502681e5,
        0x9fc2d5c0517975f5,
        0xc967f6f57fd6f8a5,
        0x1c9d68d702f6ffe5,
        0xd08520f528016bf5,
        0x12b77e7401300a35,
        0x4fcd0dcba4e29edd,
        0xca19c24c1b4466e5,
        0x292278b57a7f2b05,
        0xf9a3adb27e2e3825,
        0xb86d655f2bbbd4d5,
        0x0935e308fa631025,
    ];
    let mut mismatches = Vec::new();
    for (mode_index, &mode) in MODES[1..].iter().enumerate() {
        let mut hash = 14_695_981_039_346_656_037;
        for fans in 1..=4 {
            for (plane_index, plane) in [Plane::Outer, Plane::Center].into_iter().enumerate() {
                for direction in 0..=1 {
                    let mut effect = region(RgbScope::All, mode).effect;
                    effect.colors = vec![[255, 128, 1], [1, 200, 99]];
                    effect.direction = if direction == 0 {
                        lianli_shared::rgb::RgbDirection::Clockwise
                    } else {
                        lianli_shared::rgb::RgbDirection::CounterClockwise
                    };
                    let animation = render_plane(&effect, fans, plane).unwrap();
                    hash = hash_i32(hash, fans);
                    hash = hash_i32(hash, plane_index);
                    hash = hash_i32(hash, direction);
                    hash = hash_i32(hash, animation.frames.len());
                    for color in animation.frames.iter().flatten() {
                        for &channel in color {
                            hash = hash_byte(hash, channel);
                        }
                    }
                }
            }
        }
        if hash != EXPECTED[mode_index] {
            mismatches.push((mode_index + 1, hash, EXPECTED[mode_index]));
        }
    }
    assert!(mismatches.is_empty(), "{mismatches:#x?}");
}
