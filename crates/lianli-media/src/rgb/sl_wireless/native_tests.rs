use super::{integration, render_region, Side};
use lianli_shared::rgb::{RgbDirection, RgbEffect, RgbMode, RgbScope};

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
        0x5a81285cbd132e25,
        0x770e93d41470ed75,
        0xabefb0ad95559f75,
        0x36dd8cc29c179085,
    ],
    [
        0x1a8c98a0b10e0035,
        0xd2d0590eb83da065,
        0x9e3a2e900a86d2d5,
        0x0940e82d9712e8c5,
    ],
    [
        0xdbee12f3e87fb065,
        0x8e3ff490f051b305,
        0x2820c92fe461f205,
        0x8c5655d751ee9cc5,
    ],
    [
        0x11075634c2bebc45,
        0xc77390a49a6a3e75,
        0x6504dab0a366bb95,
        0x4a424d800c2a8c05,
    ],
    [
        0xb03e178b4758f9c5,
        0x1023363eecd54365,
        0x626dc5014126b105,
        0x1a9e2a57ba7c5725,
    ],
    [
        0x01859b7dbe9caef5,
        0xa64d46830d6fa865,
        0x88c52b1eb244b465,
        0x3726febb60aafe35,
    ],
    [
        0xf7eb20b3ad8f2645,
        0xa3f1ac2f64e85d85,
        0xdef6af3b9326a145,
        0x93c6dffcb43f5b05,
    ],
    [
        0xccf789459e0b1575,
        0x87964eea7c0e0ac5,
        0x7eed4eca64a96e95,
        0x4d9b1fb325792d25,
    ],
    [
        0xda5ade96e9bc04a5,
        0xc992358dcb7d0a45,
        0x096e7cdcac877a85,
        0xe2ad2ad02f697965,
    ],
    [
        0xe00d9afc7fab7165,
        0xd485dde400636265,
        0xc360237092ccde15,
        0x0271530ab4589e25,
    ],
    [
        0x33538b3eedb551a5,
        0xe9e520ac811dcdc5,
        0xe2eed78c05cd5b85,
        0x8974ae434f486325,
    ],
    [
        0xebf08a206b52eb25,
        0x0342fd5c2b8333a5,
        0xc0dbd0c87fe71fc5,
        0xfadc1950831548b5,
    ],
    [
        0x626a7f878bee2abd,
        0xc842518b66caf845,
        0xcfbd8dc4b74b180d,
        0x7113537bdfa5bcf5,
    ],
    [
        0x46a20064fb3a4c05,
        0x25a258e66a484fe5,
        0xfdb8d8aa3e154b65,
        0xd3beed9274d5c2c5,
    ],
    [
        0xe145caf0e1c7c525,
        0x8b5411e2f7f511a5,
        0xb6d41167130ebaa5,
        0xaa0e192d1385cf25,
    ],
    [
        0xf29af20e89cb7665,
        0xb02ff3818ca72425,
        0x34299b7504bfb255,
        0xbaf8ff24f9b01625,
    ],
    [
        0x315708d516c65215,
        0x245a6133d2c10f85,
        0xaf4639d50ec79d75,
        0xe0137093601fcfa5,
    ],
    [
        0xb76abdab47297a55,
        0x8177fda64ee13305,
        0x7891c5fa5b297f75,
        0xc7be77fb0dafecc5,
    ],
    [
        0x1d51525357f22da5,
        0x506bc3810815fe65,
        0xe92c839b279aa065,
        0xdd14e573ce3715e5,
    ],
    [
        0x2cc28d58073937a5,
        0xbf22df661e4c8d25,
        0x11b2906f2ae991e5,
        0xa4cb05c95ab76825,
    ],
    [
        0xcffcf00fb7e0f965,
        0xc5892862dddd3ad5,
        0x8d6e73a1a3ecc975,
        0x4180380f42b8a705,
    ],
    [
        0x114bce5f6cde4165,
        0xf8c94a48ab8bc7e5,
        0x7fd5a5b9149162e5,
        0x7e137f660c56cc65,
    ],
    [
        0x2bc8a5e75a66a9ed,
        0xd3c4d398c52b390d,
        0xcb735ab08a33340d,
        0x5e8ddc70106d666d,
    ],
    [
        0x2c2b26d4513f7e85,
        0xa20c88980652fe65,
        0x1cfec9328dc1b745,
        0x2a3a7158c7b35b65,
    ],
    [
        0x1bc5737fe88f531d,
        0x85d15c23d90928b1,
        0x25a5f25ec689f905,
        0x53e25ce0df1ddaf9,
    ],
];

fn hash_byte(hash: u64, byte: u8) -> u64 {
    (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
}

fn hash_matrix(mode: RgbMode, fans: usize) -> u64 {
    let mut hash = 0xcbf29ce484222325;
    for brightness in 0..=4 {
        for direction in [RgbDirection::Clockwise, RgbDirection::CounterClockwise] {
            for (scope, side) in [
                (RgbScope::Outer, Side::Outer),
                (RgbScope::Inner, Side::Inner),
            ] {
                let effect = RgbEffect {
                    mode,
                    colors: vec![[255, 1, 127], [5, 200, 17], [90, 33, 240], [18, 77, 150]],
                    brightness,
                    direction,
                    scope,
                    ..RgbEffect::default()
                };
                let frames = render_region(&effect, fans, side).unwrap().frames;
                for byte in (frames.len() as u32).to_le_bytes() {
                    hash = hash_byte(hash, byte);
                }
                for channel in frames.iter().flatten().flatten() {
                    hash = hash_byte(hash, *channel);
                }
            }
        }
    }
    hash
}

fn hash_integration_matrix(mode: RgbMode, fans: usize) -> u64 {
    let mut hash = 0xcbf29ce484222325;
    for brightness in 0..=4 {
        for direction in [RgbDirection::Clockwise, RgbDirection::CounterClockwise] {
            for side in [Side::Outer, Side::Inner] {
                let effect = RgbEffect {
                    mode,
                    colors: vec![[255, 1, 127], [5, 200, 17], [90, 33, 240], [18, 77, 150]],
                    brightness,
                    direction,
                    scope: RgbScope::All,
                    ..RgbEffect::default()
                };
                let frames = match mode {
                    RgbMode::Rainbow => integration::rainbow(&effect, fans, side),
                    RgbMode::Runway => integration::runway(&effect, fans, side),
                    RgbMode::Meteor => integration::meteor(&effect, fans, side),
                    RgbMode::ColorCycle => integration::color_cycle(&effect, fans, side),
                    RgbMode::Render => integration::render_effect(&effect, fans, side),
                    _ => unreachable!(),
                };
                for byte in (frames.len() as u32).to_le_bytes() {
                    hash = hash_byte(hash, byte);
                }
                for channel in frames.iter().flatten().flatten() {
                    hash = hash_byte(hash, *channel);
                }
            }
        }
    }
    hash
}

#[test]
fn every_mode_matches_the_extracted_csharp_parameter_matrix() {
    for (mode_index, mode) in MODES.into_iter().enumerate() {
        for fans in 1..=4 {
            assert_eq!(
                hash_matrix(mode, fans),
                EXPECTED[mode_index][fans - 1],
                "{mode:?}, {fans} fans"
            );
        }
    }
}

#[test]
fn integration_modes_match_the_extracted_csharp_parameter_matrix() {
    let expected = [
        (
            RgbMode::Rainbow,
            [
                0xecce8b5bc9691b65,
                0xd0f007118e00dee5,
                0x075d81df92beaf95,
                0xe3a67b530c3f8af5,
            ],
        ),
        (
            RgbMode::Runway,
            [
                0xe787a19e93ce1425,
                0x7c62fd911cb22b25,
                0x26c36b8d41248125,
                0x32083ff75c2b2065,
            ],
        ),
        (
            RgbMode::Meteor,
            [
                0x69707b7fec753df5,
                0xc454aba1353e6615,
                0xbb30866a08b75cf5,
                0x9d65a32193060825,
            ],
        ),
        (
            RgbMode::ColorCycle,
            [
                0xbf8f018d60fe2325,
                0x5f5da3c460ccf095,
                0x2afd37498041a4ad,
                0x672798910589cd6d,
            ],
        ),
        (
            RgbMode::Render,
            [
                0xdb159af4bb4bcdf5,
                0x99d4f8b7a0b9ecc5,
                0xc3bbfa74e5b07ef5,
                0xd7522e9af22ab225,
            ],
        ),
    ];
    for (mode, hashes) in expected {
        for fans in 1..=4 {
            assert_eq!(
                hash_integration_matrix(mode, fans),
                hashes[fans - 1],
                "{mode:?}, {fans} fans"
            );
        }
    }
}
