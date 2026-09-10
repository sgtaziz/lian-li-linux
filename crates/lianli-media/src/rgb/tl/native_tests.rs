use super::{
    breathing, cycle, door, meteor, morph, rainbow, render, ripple, runway, solid, stack, twinkle,
    voice,
};
use anyhow::Result;
use lianli_shared::rgb::{RgbDirection, RgbEffect, RgbMode};

type Frames = Vec<Vec<[u8; 3]>>;
type Renderer = fn(&RgbEffect, usize, bool) -> Result<Frames>;

const COLORS: [[u8; 3]; 4] = [[255, 1, 127], [5, 200, 17], [90, 33, 240], [18, 77, 150]];

fn effect(brightness: u8, direction: RgbDirection) -> RgbEffect {
    RgbEffect {
        mode: RgbMode::Static,
        colors: COLORS.to_vec(),
        brightness,
        direction,
        ..RgbEffect::default()
    }
}

fn hash_byte(hash: u64, byte: u8) -> u64 {
    (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
}

fn hash_parameter_matrix(renderer: Renderer, fans: usize) -> u64 {
    let mut hash = 0xcbf29ce484222325;
    for brightness in 0..=4 {
        for direction in [RgbDirection::Clockwise, RgbDirection::CounterClockwise] {
            for bottom in [false, true] {
                let frames = renderer(&effect(brightness, direction), fans, bottom).unwrap();
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

fn assert_parameter_matrix(renderer: Renderer, expected: [u64; 4]) {
    let actual = std::array::from_fn(|index| hash_parameter_matrix(renderer, index + 1));
    assert_eq!(actual, expected);
}

#[test]
fn rainbow_matches_native_fixtures() {
    assert_parameter_matrix(
        rainbow::render,
        [
            0xb1749008d25bd8e5,
            0xc74dba525466d18d,
            0x2867dcb5aa98fbc1,
            0xfcf8820fa895eb4d,
        ],
    );
}

#[test]
fn solid_matches_native_fixtures() {
    assert_parameter_matrix(
        solid::render,
        [
            0xac70fe6531190965,
            0xf4d3168cfeff8245,
            0x481da718d95087a5,
            0x03f6f296c9916e15,
        ],
    );
}

#[test]
fn morph_matches_native_fixtures() {
    assert_parameter_matrix(
        morph::render,
        [
            0xd6a231754491c799,
            0x6b842e8a930eec05,
            0x3496b5010950b759,
            0xccee13b8e55f6485,
        ],
    );
}

#[test]
fn breathing_matches_native_fixtures() {
    assert_parameter_matrix(
        breathing::render,
        [
            0xf031a5e75df4f985,
            0xe0eefff2f33ffd59,
            0x3a98d074b864a081,
            0x17d2f03e867dd115,
        ],
    );
}

#[test]
fn runway_matches_native_fixtures() {
    assert_parameter_matrix(
        runway::render,
        [
            0x41da1c1f8441d525,
            0x02cbb9556c6fef25,
            0x3c62b5900100d205,
            0x86f604db38e66265,
        ],
    );
}

#[test]
fn meteor_matches_native_fixtures() {
    assert_parameter_matrix(
        meteor::render,
        [
            0xf300d3123d5553f5,
            0x70df8616c84c355d,
            0x643b985a533f5245,
            0x1c9e380f1a6e8bc9,
        ],
    );
}

#[test]
fn cycle_matches_native_fixtures() {
    assert_parameter_matrix(
        cycle::render,
        [
            0xfdc9d57701eea8c9,
            0x3fb7cf0390278035,
            0x36df582c17b4ced9,
            0x67e609255cb018a5,
        ],
    );
}

#[test]
fn stack_matches_native_fixtures() {
    assert_parameter_matrix(
        stack::render,
        [
            0x3f1cded1d81d8769,
            0x9a5b578f0411c3c5,
            0xbb94e1a2cbe22569,
            0xe5061e34a5cfed2d,
        ],
    );
}

#[test]
fn twinkle_matches_native_fixtures() {
    assert_parameter_matrix(
        twinkle::render,
        [
            0xd3eaf554e0ab9b95,
            0x84cbfc8054f10ab1,
            0x21642b2028ae06f1,
            0xf36bf2334394bc15,
        ],
    );
}

#[test]
fn voice_matches_seeded_native_fixtures() {
    assert_parameter_matrix(
        voice::render,
        [
            0x793f3c2030efc169,
            0xf915ad80a20f9221,
            0xb5b43e106b314e6d,
            0x57d38c8c8f24a169,
        ],
    );
}

#[test]
fn door_matches_native_fixtures() {
    assert_parameter_matrix(
        door::render,
        [
            0x4f19d40ff7619625,
            0x94b7857e175d2145,
            0xaa30ab603e38f049,
            0xf446a5cf00d157a5,
        ],
    );
}

#[test]
fn render_matches_native_fixtures() {
    assert_parameter_matrix(
        render::render,
        [
            0x6dc6e59fefdb9989,
            0x90a01afa568cb225,
            0x94996c6f4c5d66f9,
            0xda8732e5b7eae80d,
        ],
    );
}

#[test]
fn ripple_matches_native_fixtures() {
    assert_parameter_matrix(
        ripple::render,
        [
            0x2c641f5e1a135675,
            0xc597cf1b7e69d925,
            0x2f83ed8e2a5c1121,
            0x8fb2480f267a5359,
        ],
    );
}

#[test]
fn breathing_preserves_native_double_truncation() {
    let frames = breathing::render(&effect(4, RgbDirection::Clockwise), 1, false).unwrap();
    assert_eq!(frames[85][0], [253, 0, 125]);
}

#[test]
fn morph_discards_the_unpaired_last_source_frame() {
    let frames = morph::render(&effect(4, RgbDirection::Clockwise), 1, false).unwrap();
    assert_eq!(frames.len(), 127);
    assert_eq!(frames[126][0], [245, 0, 8]);
}

#[test]
fn cycle_uses_distinct_top_and_bottom_patterns() {
    let effect = effect(4, RgbDirection::CounterClockwise);
    let top = cycle::render(&effect, 1, false).unwrap();
    let bottom = cycle::render(&effect, 1, true).unwrap();
    assert_eq!(top[0][3], [4, 199, 16]);
    assert_eq!(bottom[0][3], [0, 0, 0]);
}
