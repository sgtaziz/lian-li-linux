use super::classic;
use super::engine::{geometry, Color, Frame};
use super::mode_13;
use super::mode_14;
use super::mode_3;
use super::mode_34;
use super::mode_6;
use super::mode_7;
use super::mode_8;
use super::modes_15_17;
use super::modes_16_21;
use super::modes_18_19;
use super::modes_1_2;
use super::modes_20_22;
use super::modes_23_27;
use super::modes_4_5;
use super::modes_9_10;

const COLORS: [Color; 6] = [
    [201, 33, 17],
    [5, 144, 220],
    [67, 199, 88],
    [155, 55, 200],
    [99, 188, 44],
    [222, 77, 133],
];

fn add(hash: u64, byte: u8) -> u64 {
    (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
}

fn hash_modes_1_2(mode: u8) -> u64 {
    let mut hash = 0xcbf29ce484222325;
    for led_count in [88, 116, 132, 174] {
        let geometry = geometry(led_count).unwrap();
        for brightness in [64, 255] {
            for reverse in [false, true] {
                let (frames, base_ticks) = if mode == 1 {
                    (
                        modes_1_2::color_transfer(geometry, &COLORS, brightness, reverse),
                        24u32,
                    )
                } else {
                    (modes_1_2::fade_out(geometry, &COLORS, brightness), 11)
                };
                for byte in (frames.len() as i32).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for byte in (base_ticks * 5 * 100).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for frame in frames {
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

fn hash_contest() -> u64 {
    let mut hash = 0xcbf29ce484222325;
    for led_count in [88, 116, 132, 174] {
        let geometry = geometry(led_count).unwrap();
        for brightness in [64, 255] {
            for reverse in [false, true] {
                let frames = mode_3::contest(geometry, &COLORS, brightness, reverse);
                for byte in (frames.len() as i32).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for byte in (11 * 5 * 100i32).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for frame in frames {
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

fn hash_modes_4_5(mode: u8) -> u64 {
    let mut hash = 0xcbf29ce484222325;
    for led_count in [88, 116, 132, 174] {
        let geometry = geometry(led_count).unwrap();
        for brightness in [64, 255] {
            for reverse in [false, true] {
                let (frames, base_hundredths) = if mode == 4 {
                    (
                        modes_4_5::cross_over(geometry, &COLORS, brightness, reverse),
                        1_100i32,
                    )
                } else {
                    (
                        modes_4_5::bullet_stack(geometry, &COLORS, brightness, reverse),
                        1_130,
                    )
                };
                for byte in (frames.len() as i32).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for byte in (base_hundredths * 5).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for frame in frames {
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

fn hash_modes_18_19(mode: u8) -> u64 {
    let mut hash = 0xcbf29ce484222325;
    for led_count in [88, 116, 132, 174] {
        let geometry = geometry(led_count).unwrap();
        for brightness in [64, 255] {
            for reverse in [false, true] {
                let frames = if mode == 18 {
                    modes_18_19::transformation(geometry, &COLORS, brightness, reverse)
                } else {
                    modes_18_19::gradient_ribbon(geometry, brightness, reverse)
                };
                for byte in (frames.len() as i32).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for byte in (1_100 * 5i32).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for frame in frames {
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

fn hash_modes_23_27(mode: u8) -> u64 {
    let mut hash = 0xcbf29ce484222325;
    for led_count in [88, 116, 132, 174] {
        let geometry = geometry(led_count).unwrap();
        for brightness in [64, 255] {
            for reverse in [false, true] {
                let (frames, base_hundredths) = match mode {
                    23 => (
                        modes_23_27::ping_pong(geometry, &COLORS, brightness),
                        1_200i32,
                    ),
                    24 => (modes_23_27::runway(geometry, &COLORS, brightness), 1_200),
                    25 => (modes_23_27::tide(geometry, &COLORS, brightness), 1_100),
                    26 => (modes_23_27::blow_up(geometry, &COLORS, brightness), 1_550),
                    27 => (
                        modes_23_27::meteor(geometry, &COLORS, brightness, reverse),
                        1_500,
                    ),
                    _ => unreachable!(),
                };
                for byte in (frames.len() as i32).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for byte in (base_hundredths * 5).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for frame in frames {
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

fn hash_stack() -> u64 {
    let mut hash = 0xcbf29ce484222325;
    for led_count in [88, 116, 132, 174] {
        let geometry = geometry(led_count).unwrap();
        for brightness in [64, 255] {
            for reverse in [false, true] {
                let frames = mode_34::stack(geometry, &COLORS, brightness, reverse);
                for byte in (frames.len() as i32).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for byte in (1_100 * 5i32).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for frame in frames {
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

fn hash_modes_9_10_16_21(mode: u8) -> u64 {
    let mut hash = 0xcbf29ce484222325;
    for led_count in [88, 116, 132, 174] {
        let geometry = geometry(led_count).unwrap();
        for brightness in [64, 255] {
            for _reverse in [false, true] {
                let frames = match mode {
                    9 => modes_9_10::ripple(geometry, &COLORS, brightness),
                    10 => modes_9_10::voice(geometry, &COLORS, brightness),
                    16 => modes_16_21::pioneer(geometry, COLORS[0], brightness),
                    21 => modes_16_21::snooker(geometry, &COLORS, brightness),
                    _ => unreachable!(),
                };
                for byte in (frames.len() as i32).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for byte in (1_100 * 5i32).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for frame in frames {
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

fn hash_modes_6_7(mode: u8) -> u64 {
    let mut hash = 0xcbf29ce484222325;
    for led_count in [88, 116, 132, 174] {
        let geometry = geometry(led_count).unwrap();
        for brightness in [64, 255] {
            for reverse in [false, true] {
                let frames = if mode == 6 {
                    mode_6::twinkle(geometry, &COLORS, brightness)
                } else {
                    mode_7::parallel(geometry, &COLORS, brightness, reverse)
                };
                for byte in (frames.len() as i32).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for byte in (1_100 * 5i32).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for frame in frames {
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

fn hash_modes_15_17(mode: u8) -> u64 {
    let mut hash = 0xcbf29ce484222325;
    for led_count in [88, 116, 132, 174] {
        let geometry = geometry(led_count).unwrap();
        for brightness in [64, 255] {
            for _reverse in [false, true] {
                let frames = if mode == 15 {
                    modes_15_17::hourglass(geometry, &COLORS, brightness)
                } else {
                    modes_15_17::electric_current(geometry, &COLORS, brightness)
                };
                for byte in (frames.len() as i32).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for byte in (1_100 * 5i32).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for frame in frames {
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

fn hash_modes_8_20_22(mode: u8) -> u64 {
    let mut hash = 0xcbf29ce484222325;
    for led_count in [88, 116, 132, 174] {
        let geometry = geometry(led_count).unwrap();
        for brightness in [64, 255] {
            for reverse in [false, true] {
                let (frames, base_hundredths) = match mode {
                    8 => (
                        mode_8::shock_wave(geometry, &COLORS, brightness, reverse),
                        1_100i32,
                    ),
                    20 => (
                        modes_20_22::rainbow_wave(geometry, brightness, reverse),
                        1_100,
                    ),
                    22 => (modes_20_22::mixing(geometry, &COLORS, brightness), 850),
                    _ => unreachable!(),
                };
                for byte in (frames.len() as i32).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for byte in (base_hundredths * 5).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for frame in frames {
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

fn hash_river() -> u64 {
    let mut hash = 0xcbf29ce484222325;
    for led_count in [88, 116, 132, 174] {
        let geometry = geometry(led_count).unwrap();
        for brightness in [64, 255] {
            for reverse in [false, true] {
                let frames = mode_14::river(geometry, &COLORS, brightness, reverse);
                for byte in (frames.len() as i32).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for byte in (1_100 * 5i32).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for frame in frames {
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

fn hash_shuttle_run() -> u64 {
    let mut hash = 0xcbf29ce484222325;
    for led_count in [88, 116, 132, 174] {
        let geometry = geometry(led_count).unwrap();
        for brightness in [64, 255] {
            for _reverse in [false, true] {
                let frames = mode_13::shuttle_run(geometry, &COLORS, brightness);
                for byte in (frames.len() as i32).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for byte in (1_100 * 5i32).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for frame in frames {
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

fn hash_modes(regional: bool, mode: u8) -> u64 {
    let mut hash = 0xcbf29ce484222325;
    for led_count in [88, 116, 132, 174] {
        let geometry = geometry(led_count).unwrap();
        for brightness in [64, 255] {
            for reverse in [false, true] {
                let (frames, base_ticks): (Vec<Frame>, u32) = match (regional, mode) {
                    (_, 1 | 28) => (
                        classic::rainbow(geometry, brightness, reverse),
                        if regional { 20 } else { 22 },
                    ),
                    (true, 2) => (classic::wave(geometry, &COLORS, brightness, reverse), 20),
                    (false, 29) => (classic::wave(geometry, &COLORS, brightness, reverse), 22),
                    (true, 3) => (classic::solid(geometry, COLORS[0], brightness, 1), 20),
                    (false, 30) => (classic::solid(geometry, COLORS[0], brightness, 30), 20),
                    (true, 4) => (
                        classic::breathing(geometry, COLORS[0], brightness, true),
                        20,
                    ),
                    (false, 31) => (
                        classic::breathing(geometry, COLORS[0], brightness, false),
                        11,
                    ),
                    (true, 5) => (classic::morph(geometry, brightness, true), 20),
                    (false, 32) => (classic::morph(geometry, brightness, false), 11),
                    (_, 6 | 33) => (
                        classic::paint(geometry, &COLORS, brightness),
                        if regional { 20 } else { 11 },
                    ),
                    _ => unreachable!(),
                };
                for byte in (frames.len() as i32).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for byte in (base_ticks * 5 * 100).to_le_bytes() {
                    hash = add(hash, byte);
                }
                for frame in frames {
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
fn whole_classic_modes_match_recovered_source_for_every_layout() {
    for (mode, expected) in [
        (28, 0xece9eb89836e3251),
        (29, 0x63a7f3a7000e9115),
        (30, 0x124c464cc3c26419),
        (31, 0x49a34a784f4e4849),
        (32, 0x3a2e2878e168f1ad),
        (33, 0xf2393608f7b6e3ed),
    ] {
        assert_eq!(hash_modes(false, mode), expected, "mode {mode}");
    }
}

#[test]
fn segment_modes_match_recovered_source_for_every_layout() {
    for (mode, expected) in [
        (1, 0x1ece37676611fa05),
        (2, 0x995c5e8320aef145),
        (3, 0xc399964f0199a195),
        (4, 0x1139ac89d2c6dc95),
        (5, 0x1cd3202559b98d9d),
        (6, 0x466b422675f3b1a5),
    ] {
        assert_eq!(hash_modes(true, mode), expected, "single mode {mode}");
    }
}

#[test]
fn first_whole_modes_match_recovered_source_for_every_layout() {
    assert_eq!(hash_modes_1_2(1), 0xaf03f89b40d3224d);
    assert_eq!(hash_modes_1_2(2), 0x8eca0798cfeef37d);
    assert_eq!(hash_contest(), 0xa1fa89abebb265e9);
    assert_eq!(hash_modes_4_5(4), 0xb9d5c39304fd1ae9);
    assert_eq!(hash_modes_4_5(5), 0x0bc9dcb30297d6b1);
    assert_eq!(hash_modes_18_19(18), 0x1a9d6d4a4d2d1d51);
    assert_eq!(hash_modes_18_19(19), 0x5350af6fcfefd3ad);
    for (mode, expected) in [
        (23, 0xfeea001d688b62d5),
        (24, 0xea6b7d0b06ddb9bd),
        (25, 0x5ff4e3ac359a28fd),
        (26, 0xa40f11e303149f09),
        (27, 0x5d975e08a81f163d),
    ] {
        assert_eq!(hash_modes_23_27(mode), expected, "mode {mode}");
    }
    assert_eq!(hash_stack(), 0xf8647065ee4df4c5);
    assert_eq!(hash_river(), 0xe285d200d0084af5);
    assert_eq!(hash_shuttle_run(), 0x409611c5ee8af7b1);
    for (mode, expected) in [
        (9, 0xf4a71b0aa33778ed),
        (10, 0xc90339d32c111fdd),
        (16, 0x88315d1a4e5905fd),
        (21, 0x09a45c8b13392845),
    ] {
        assert_eq!(hash_modes_9_10_16_21(mode), expected, "mode {mode}");
    }
    assert_eq!(hash_modes_6_7(6), 0x48b60c2d4907dc59);
    assert_eq!(hash_modes_6_7(7), 0x4d226805d0684855);
    assert_eq!(hash_modes_15_17(15), 0xbb76669541152065);
    assert_eq!(hash_modes_15_17(17), 0xc65b6f1fc6235cc1);
    assert_eq!(hash_modes_8_20_22(8), 0xb66229f6822734a5);
    assert_eq!(hash_modes_8_20_22(20), 0x4cc82daeb641120b);
    assert_eq!(hash_modes_8_20_22(22), 0x034808ae2ae99b4d);
}

fn region(
    scope: lianli_shared::rgb::RgbScope,
    mode: lianli_shared::rgb::RgbMode,
    speed: u8,
) -> lianli_shared::rgb::RgbRegionConfig {
    lianli_shared::rgb::RgbRegionConfig {
        effect: lianli_shared::rgb::RgbEffect {
            mode,
            colors: COLORS.to_vec(),
            speed,
            brightness: 4,
            scope,
            ..Default::default()
        },
        flip: false,
    }
}

#[test]
fn segment_composition_uses_segment_one_timing_and_source_defaults() {
    use lianli_shared::rgb::{RgbMode, RgbScope};

    let regions = vec![
        region(RgbScope::Segment3, RgbMode::Paint, 4),
        region(RgbScope::Segment1, RgbMode::Static, 0),
        region(RgbScope::Segment2, RgbMode::Breathing, 2),
    ];
    let animation = super::render(&regions, 88).unwrap();
    assert_eq!(animation.frames.len(), 264);
    assert_eq!(animation.interval_hundredths, 14_000);

    let without_segment_one = super::render(&regions[..1], 88).unwrap();
    assert_eq!(without_segment_one.interval_hundredths, 8_000);
    assert!(without_segment_one.frames[0][..44]
        .iter()
        .any(|color| *color != [0; 3]));

    assert!(super::render(&[region(RgbScope::Segment5, RgbMode::Static, 2)], 88).is_err());
}

#[test]
fn sync_key_matches_the_vendor_whole_and_segment_comparison_fields() {
    use lianli_shared::rgb::{RgbMode, RgbScope};

    let whole = super::sync_key(&[region(RgbScope::All, RgbMode::River, 4)], 116).unwrap();
    assert_eq!(whole.modes, vec![RgbMode::River]);
    assert_eq!(whole.compared_speed, 4);

    let segments = super::sync_key(
        &[
            region(RgbScope::Segment4, RgbMode::Paint, 0),
            region(RgbScope::Segment2, RgbMode::Static, 4),
        ],
        88,
    )
    .unwrap();
    assert_eq!(
        segments.modes,
        vec![
            RgbMode::Rainbow,
            RgbMode::Static,
            RgbMode::Rainbow,
            RgbMode::Paint,
            RgbMode::Rainbow,
            RgbMode::Rainbow,
        ]
    );
    assert_eq!(segments.compared_speed, 2);
}

#[test]
fn capability_menus_separate_whole_and_segment_effects_by_layout() {
    use lianli_shared::rgb::{RgbMode, RgbScope};

    for led_count in [88, 116, 132, 174] {
        let whole = super::parameters::for_scope(led_count, RgbScope::All);
        assert_eq!(whole.len(), 29);
        assert_eq!(whole.first().unwrap().mode, RgbMode::Off);
        assert_eq!(whole.last().unwrap().mode, RgbMode::Stack);
        for backend_only in [
            RgbMode::Rainbow,
            RgbMode::Wave,
            RgbMode::Static,
            RgbMode::Breathing,
            RgbMode::RainbowMorph,
            RgbMode::Paint,
        ] {
            assert!(!whole.iter().any(|entry| entry.mode == backend_only));
        }
        let voice = whole
            .iter()
            .find(|entry| entry.mode == RgbMode::Voice)
            .unwrap();
        assert_eq!(voice.max_colors, if led_count < 132 { 4 } else { 6 });
        assert_eq!(
            super::parameters::for_scope(led_count, RgbScope::Segment5).len(),
            if led_count < 132 { 0 } else { 7 }
        );
    }

    let segment = super::parameters::for_scope(174, RgbScope::Segment6);
    assert_eq!(
        segment.iter().map(|entry| entry.mode).collect::<Vec<_>>(),
        super::SEGMENT_MODES
    );
    assert!(super::parameters::for_scope(174, RgbScope::Inner).is_empty());
}
