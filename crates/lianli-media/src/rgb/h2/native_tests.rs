use super::*;

const EFFECTS: [RgbMode; 11] = [
    RgbMode::Rainbow,
    RgbMode::RainbowMorph,
    RgbMode::Static,
    RgbMode::Breathing,
    RgbMode::Runway,
    RgbMode::Meteor,
    RgbMode::TaiChi,
    RgbMode::Twinkle,
    RgbMode::Voice,
    RgbMode::Pump,
    RgbMode::Bounce,
];
const EXPECTED: [u64; 11] = [
    0x2ceb1fd33c6e9275,
    0xb48f5bfdbf847985,
    0x079beb6046e212c5,
    0xf3ab73657e090b45,
    0x61f83d4f47e16b15,
    0xbdb8873410f7c379,
    0xcbc2367f5c002295,
    0x95f2a939301888f5,
    0x064c1b3883dfd025,
    0xf320cbb87d04712d,
    0xaf1ca8470a567d51,
];

fn effect(mode: RgbMode, brightness: u8, direction: RgbDirection) -> RgbEffect {
    RgbEffect {
        mode,
        colors: vec![[255, 1, 127], [5, 200, 17], [90, 33, 240], [18, 77, 150]],
        brightness,
        direction,
        ..RgbEffect::default()
    }
}

fn hash_byte(hash: u64, byte: u8) -> u64 {
    (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
}

fn hash_matrix(mode: RgbMode) -> u64 {
    let mut hash = 0xcbf29ce484222325;
    for brightness in 0..=4 {
        for direction in [RgbDirection::Clockwise, RgbDirection::CounterClockwise] {
            let animation = render(&effect(mode, brightness, direction), 24).unwrap();
            for byte in (animation.frames.len() as i32).to_le_bytes() {
                hash = hash_byte(hash, byte);
            }
            for channel in animation.frames.iter().flatten().flatten() {
                hash = hash_byte(hash, *channel);
            }
        }
    }
    hash
}

#[test]
fn every_dispatched_effect_matches_recovered_h2_source() {
    for (index, mode) in EFFECTS.into_iter().enumerate() {
        assert_eq!(hash_matrix(mode), EXPECTED[index], "{mode:?}");
    }
}

#[test]
fn preserves_h2_palette_defaults_and_570_current_limit() {
    let empty = RgbEffect {
        colors: Vec::new(),
        ..RgbEffect::default()
    };
    assert_eq!(
        engine::palette(&empty),
        [[255, 0, 0], [0, 0, 255], [0, 255, 0], [255, 255, 0]]
    );
    let effect = RgbEffect {
        colors: vec![[255, 255, 255]],
        ..RgbEffect::default()
    };
    assert_eq!(engine::palette(&effect)[0], [185, 185, 185]);
}

#[test]
fn uses_recovered_per_mode_intervals() {
    let mut slow_effect = effect(RgbMode::Rainbow, 4, RgbDirection::Clockwise);
    slow_effect.speed = 0;
    let mut morph_effect = effect(RgbMode::RainbowMorph, 4, RgbDirection::Clockwise);
    morph_effect.speed = 0;
    let slow = render(&slow_effect, 24).unwrap();
    let morph = render(&morph_effect, 24).unwrap();
    assert_eq!(slow.interval_hundredths, 14_000);
    assert_eq!(morph.interval_hundredths, 7_700);
}
