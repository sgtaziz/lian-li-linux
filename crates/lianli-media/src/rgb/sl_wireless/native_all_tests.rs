use super::{render, SecondaryTiming};
use lianli_shared::rgb::{RgbDirection, RgbEffect, RgbMode, RgbRegionConfig, RgbScope};

const MODES: [RgbMode; 25] = [
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

const EXPECTED: [[u64; 4]; 25] = [
    [
        0xd9a2be6d19bf14e5,
        0x39ea6ad10eb450b5,
        0x6dd9912af5b5ffe5,
        0x68c6c89512768525,
    ],
    [
        0x9978609373db94c5,
        0x85ed8688efa5f015,
        0xf6e2e45f646a8445,
        0xe8bb437d7e96f355,
    ],
    [
        0xe9abeb76214c76c5,
        0x54693bc349d23485,
        0x8d78b8c8b199d285,
        0xc1341836241410c5,
    ],
    [
        0x39274d46d590cf45,
        0xfa594c2282d4e1e5,
        0xc3746847c0d79e25,
        0xa511a496484b89c5,
    ],
    [
        0x6fb7b5aed4e40069,
        0x5acb14fd35815cc5,
        0x2fee47418f3bb349,
        0x6fe06ef8a131c495,
    ],
    [
        0x39ae87c65bac337d,
        0x8cb42d581db846b5,
        0x37684027d8a56d19,
        0x213a2fc08119b2e5,
    ],
    [
        0x481b17366b772cfd,
        0x3807fc1ab6f43a1d,
        0xeb20ba410a6884ad,
        0x99faf04f3041f635,
    ],
    [
        0x8aacdf0433fdb025,
        0x161fe037464efae5,
        0xc991c5475713db85,
        0x9a48214081c28685,
    ],
    [
        0xfc753a257086c3e5,
        0x62950844100b9fc5,
        0x01ae43112786f475,
        0x6ed20d84d3f1b695,
    ],
    [
        0x0e85d366749d4d2d,
        0xf6cfd7f57794376d,
        0xf388976b8094c83d,
        0x983fe0ce345867a5,
    ],
    [
        0x7dfe56178f2df035,
        0xc21362d5a52813a5,
        0x06fcba6a15703035,
        0x18298ec77ebb1585,
    ],
    [
        0x1f4f040be6923e45,
        0x4f7b013896a15725,
        0x5904c843d2db7365,
        0x078405388d75451d,
    ],
    [
        0x9826b7cb238c7785,
        0xad6bcd9c38f49735,
        0xb914f8e780c525e5,
        0x3d66eb4f38eaf885,
    ],
    [
        0x1a0777060ad0bad5,
        0x1554c786f4e44815,
        0xe7ce19704fdef4e5,
        0xc43407b3fddb5995,
    ],
    [
        0x6c6df5dc1cc74d65,
        0x043fdc52180430e5,
        0x387c0c39f4987885,
        0x1f7d3ae9e7657f65,
    ],
    [
        0xf4a3cd58b0664f9d,
        0x9eff3296329fdfe5,
        0xa6f76d9e2e3062c5,
        0xe8da2259419d3485,
    ],
    [
        0xefc4ace2d911f745,
        0xf11ed59cec744405,
        0xdf356c9ee93598fd,
        0xd40f0172dc4ee8a5,
    ],
    [
        0xefe7931635047fd5,
        0x1360c707a680b265,
        0x759777a69d114995,
        0x125f88e86273a695,
    ],
    [
        0x8b2ef5456845cf85,
        0xd4135cfeaa61ec25,
        0x8ac6b90b1e3c1035,
        0x57193a3c7e3a75a5,
    ],
    [
        0x82e99887c44fbf75,
        0xbe97239701b7c7b5,
        0x185f662bc2a68d91,
        0xe4fed7a664369535,
    ],
    [
        0x7602da35d8200b25,
        0x57a5c03ea98c1af5,
        0x5354df0ad884f345,
        0x8f51439f534bb815,
    ],
    [
        0x8b59d5925d3e1a05,
        0xb207570eb5155505,
        0x419d7d636900c8c5,
        0xc4fb9460003de145,
    ],
    [
        0x35a5b019fd2e9ee5,
        0x6922c31ac5aa7675,
        0x8ffbe9a2b1bd1de5,
        0xcad95aa5cf2ae2d5,
    ],
    [
        0x5ffddad20fc31065,
        0xf489a95247577b45,
        0x0fb0140418f84705,
        0x26c2bd8e71e9f765,
    ],
    [
        0x213e3009ed59e0a5,
        0x2edac38e67fba641,
        0xdff3dc15622cf15d,
        0x19bce5dae3d535d1,
    ],
];

fn hash_byte(hash: u64, byte: u8) -> u64 {
    (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
}

fn hash_all_matrix(mode: RgbMode, fans: usize) -> u64 {
    let mut hash = 0xcbf29ce484222325;
    for brightness in 0..=4 {
        for direction in [RgbDirection::Clockwise, RgbDirection::CounterClockwise] {
            let region = RgbRegionConfig {
                effect: RgbEffect {
                    mode,
                    colors: vec![[255, 1, 127], [5, 200, 17], [90, 33, 240], [18, 77, 150]],
                    brightness,
                    direction,
                    speed: 2,
                    scope: RgbScope::All,
                    ..RgbEffect::default()
                },
                flip: false,
            };
            let frames = render(&[region], fans).unwrap().frames;
            for byte in (frames.len() as u32).to_le_bytes() {
                hash = hash_byte(hash, byte);
            }
            for channel in frames.iter().flatten().flatten() {
                hash = hash_byte(hash, *channel);
            }
        }
    }
    hash
}

#[test]
fn all_scope_dispatch_matches_the_extracted_csharp_parameter_matrix() {
    for (mode_index, mode) in MODES.into_iter().enumerate() {
        for fans in 1..=4 {
            assert_eq!(
                hash_all_matrix(mode, fans),
                EXPECTED[mode_index][fans - 1],
                "{mode:?}, {fans} fans"
            );
        }
    }
}

#[test]
fn mixed_regions_match_the_extracted_csharp_composition_fixture() {
    let regions = [
        RgbRegionConfig {
            effect: RgbEffect {
                mode: RgbMode::River,
                scope: RgbScope::Outer,
                colors: vec![[255, 1, 127], [5, 200, 17], [90, 33, 240], [18, 77, 150]],
                brightness: 2,
                direction: RgbDirection::Clockwise,
                speed: 0,
                ..RgbEffect::default()
            },
            flip: false,
        },
        RgbRegionConfig {
            effect: RgbEffect {
                mode: RgbMode::Stack,
                scope: RgbScope::Inner,
                colors: vec![[255, 1, 127], [5, 200, 17], [90, 33, 240], [18, 77, 150]],
                brightness: 4,
                direction: RgbDirection::CounterClockwise,
                speed: 4,
                ..RgbEffect::default()
            },
            flip: false,
        },
    ];
    let animation = render(&regions, 3).unwrap();
    let mut hash = 0xcbf29ce484222325;
    for byte in (animation.frames.len() as u32).to_le_bytes() {
        hash = hash_byte(hash, byte);
    }
    for channel in animation.frames.iter().flatten().flatten() {
        hash = hash_byte(hash, *channel);
    }
    assert_eq!(hash, 0x1b2ef45415decd33);
    assert_eq!(animation.frames.len(), 540);
    assert_eq!(animation.interval_hundredths, 9_000);
    assert_eq!(
        animation.secondary,
        Some(SecondaryTiming {
            interval_ticks: 92,
            frame_count: 528,
            outer_longest: false,
        })
    );
}

#[test]
fn integration_all_scope_reports_vendor_secondary_timing() {
    let animation = render(
        &[RgbRegionConfig {
            effect: RgbEffect {
                mode: RgbMode::Rainbow,
                scope: RgbScope::All,
                brightness: 4,
                speed: 2,
                ..RgbEffect::default()
            },
            flip: false,
        }],
        1,
    )
    .unwrap();
    assert_eq!(animation.frames.len(), 12);
    assert_eq!(animation.interval_hundredths, 5_500);
    assert_eq!(
        animation.secondary,
        Some(SecondaryTiming {
            interval_ticks: 82,
            frame_count: 8,
            outer_longest: false,
        })
    );
}
