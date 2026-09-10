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
    let expected = [
        (RgbMode::Rainbow, 0xfa672e711c03ce85),
        (RgbMode::RainbowMorph, 0x0ea4c75e4df97465),
        (RgbMode::Static, 0xb62eebed16797dd5),
        (RgbMode::Breathing, 0xfa61c07b6f7fb605),
        (RgbMode::Runway, 0x1038b57fb7dfa4c1),
        (RgbMode::Meteor, 0x98921921539c07d7),
        (RgbMode::ColorCycle, 0xd0e3a41d704905bd),
        (RgbMode::CoverCycle, 0xdbcffb8b5ffbd415),
        (RgbMode::Wave, 0x980a873728ec3a15),
        (RgbMode::MeteorShower, 0x5b5a1621085e8861),
        (RgbMode::Twinkle, 0xf03e31446259861d),
        (RgbMode::TaiChi, 0xa0af7fbba1893e05),
        (RgbMode::Warning, 0xd32ae8b6649be1a5),
        (RgbMode::Mixing, 0xe2cea126ea774ab5),
        (RgbMode::Tide, 0xeef8b5387d4ae2a5),
        (RgbMode::DoubleMeteor, 0x58743a3bfd16de85),
        (RgbMode::MeteorContest, 0x71c9013cdd9be505),
        (RgbMode::ReturnArc, 0x2342afdebf6b9c65),
        (RgbMode::HeartBeat, 0xcba20491a6daba05),
        (RgbMode::HeartBeatRunway, 0x97b72a878632cf05),
        (RgbMode::Disco, 0xe8847a094c817265),
        (RgbMode::CandyBox, 0x3f678779d12fdf65),
    ];
    for (mode, expected_hash) in expected {
        assert_eq!(matrix_hash(mode), expected_hash, "{mode:?}");
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
        render(&[all], 96).unwrap(),
        render(&[front, rear], 96).unwrap()
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
        96,
    )
    .unwrap();
    assert_eq!(
        animation.frames,
        vec![[[254, 0, 0]; 80]
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
    assert_eq!(render(&[front, rear], 96).unwrap().frames.len(), 360);
}
