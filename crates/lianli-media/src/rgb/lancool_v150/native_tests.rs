use super::{render, render_region};
use lianli_shared::rgb::{RgbDirection, RgbEffect, RgbMode, RgbRegionConfig, RgbScope};

const COLORS: [[u8; 3]; 4] = [[201, 33, 17], [5, 144, 220], [67, 199, 88], [155, 55, 200]];

fn effect(mode: RgbMode, scope: RgbScope, brightness: u8, direction: RgbDirection) -> RgbEffect {
    RgbEffect {
        mode,
        colors: COLORS.to_vec(),
        speed: 2,
        brightness,
        direction,
        scope,
        disabled: false,
    }
}

#[test]
fn all_scope_matches_independent_front_and_rear_regions() {
    let all = RgbRegionConfig {
        effect: effect(RgbMode::Runway, RgbScope::All, 4, RgbDirection::Clockwise),
        flip: false,
    };
    let front = RgbRegionConfig {
        effect: effect(RgbMode::Runway, RgbScope::Front, 4, RgbDirection::Clockwise),
        flip: false,
    };
    let rear = RgbRegionConfig {
        effect: effect(RgbMode::Runway, RgbScope::Rear, 4, RgbDirection::Clockwise),
        flip: false,
    };
    assert_eq!(
        render(&[all], 88).unwrap(),
        render(&[front, rear], 88).unwrap()
    );
}

#[test]
fn independent_static_regions_preserve_the_physical_split() {
    let mut front = effect(RgbMode::Static, RgbScope::Front, 4, RgbDirection::Clockwise);
    front.colors = vec![[255, 0, 0]];
    let mut rear = effect(RgbMode::Static, RgbScope::Rear, 4, RgbDirection::Clockwise);
    rear.colors = vec![[0, 0, 255]];
    let animation = render(
        &[
            RgbRegionConfig {
                effect: front,
                flip: false,
            },
            RgbRegionConfig {
                effect: rear,
                flip: false,
            },
        ],
        88,
    )
    .unwrap();
    assert_eq!(
        animation.frames,
        vec![[[254, 0, 0]; 72]
            .into_iter()
            .chain([[0, 0, 254]; 16])
            .collect::<Vec<_>>()]
    );
}

#[test]
fn paired_meteors_use_the_vendor_360_frame_cycle() {
    let front = RgbRegionConfig {
        effect: effect(RgbMode::Meteor, RgbScope::Front, 4, RgbDirection::Clockwise),
        flip: false,
    };
    let rear = RgbRegionConfig {
        effect: effect(
            RgbMode::Meteor,
            RgbScope::Rear,
            4,
            RgbDirection::CounterClockwise,
        ),
        flip: false,
    };
    assert_eq!(render(&[front, rear], 88).unwrap().frames.len(), 360);
}

fn add(hash: u64, byte: u8) -> u64 {
    (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
}

fn matrix_hash(mode: RgbMode) -> u64 {
    let mut hash = 0xcbf29ce484222325;
    for scope in [RgbScope::Front, RgbScope::Rear] {
        for brightness in [1, 4] {
            for direction in [RgbDirection::Clockwise, RgbDirection::CounterClockwise] {
                let animation =
                    render_region(&effect(mode, scope, brightness, direction), scope).unwrap();
                for byte in (animation.frames.len() as i32).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for byte in (animation.interval_hundredths as i32).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for frame in animation.frames {
                    for led in frame {
                        for channel in led {
                            hash = add(hash, channel);
                        }
                    }
                }
            }
        }
    }
    hash
}

#[test]
fn native_modes_match_recovered_source_for_both_regions() {
    for (mode, expected) in [
        (RgbMode::Rainbow, 0x484cb39f6766b1bd),
        (RgbMode::RainbowMorph, 0xeb7b6503542b2885),
        (RgbMode::Static, 0x6289968151f8f8d5),
        (RgbMode::Breathing, 0x80a02dcc3d2c2d65),
        (RgbMode::Runway, 0xe93849181dab97e5),
        (RgbMode::Meteor, 0x9e2044ec48ab71e7),
        (RgbMode::ColorCycle, 0xcf0ce4ce726cb1fd),
        (RgbMode::CoverCycle, 0x03b9f8543e431df5),
        (RgbMode::Wave, 0x2f786089a616d26d),
        (RgbMode::MeteorShower, 0x8473b665a17b30c9),
        (RgbMode::Twinkle, 0x72df01924f486f95),
        (RgbMode::TaiChi, 0xd5ca0c796e6631e5),
        (RgbMode::Warning, 0x5dc521e0419c8ca5),
        (RgbMode::Mixing, 0x9dea1c9b737b2755),
        (RgbMode::Tide, 0xe2f2e58cbf350165),
        (RgbMode::DoubleMeteor, 0x79ea7521982d5485),
        (RgbMode::MeteorContest, 0xbf26b5062de81b95),
        (RgbMode::ReturnArc, 0x861d005adad4cf85),
        (RgbMode::HeartBeat, 0x779a4d13b1ea3205),
        (RgbMode::HeartBeatRunway, 0xe72d10fa6c372945),
        (RgbMode::Disco, 0xfbbfce84449c9e45),
        (RgbMode::CandyBox, 0x77053cc4ad3e0a65),
    ] {
        assert_eq!(matrix_hash(mode), expected, "{mode:?}");
    }
}
