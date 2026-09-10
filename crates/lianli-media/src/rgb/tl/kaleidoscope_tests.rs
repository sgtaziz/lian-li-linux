use super::kaleidoscope::render;
use lianli_shared::rgb::{RgbEffect, RgbMode};
#[test]
fn seeded_kaleidoscope_matches_extracted_csharp_frames() {
    let fixtures: &[(usize, u8, usize, u64)] = &[
        (1, 0, 36, 0xce37643269027c85),
        (1, 1, 36, 0x57488b656d8845d3),
        (1, 2, 36, 0x46cfee19f9d50013),
        (1, 3, 36, 0xe072b54866a8d0d3),
        (1, 4, 36, 0x3b1e221a9d105bbb),
        (2, 0, 73, 0x0b569991fd033155),
        (2, 1, 73, 0x20d8c9776e0a7513),
        (2, 2, 73, 0x4ce1df45373011d3),
        (2, 3, 73, 0xd01ef91949f39613),
        (2, 4, 73, 0xd3fd7b75b39ca235),
        (3, 0, 107, 0xd276422876cf05dd),
        (3, 1, 107, 0x0f0430c2e0d47b71),
        (3, 2, 107, 0x23a2cf0eaf213171),
        (3, 3, 107, 0xb34b049bd886d971),
        (3, 4, 107, 0x21fc6260ef03ebbf),
        (4, 0, 144, 0x4e0ada9765f11925),
        (4, 1, 144, 0x8114290dca5c39d3),
        (4, 2, 144, 0x347b48abe43a2f93),
        (4, 3, 144, 0xfdc3b42fc3f97f53),
        (4, 4, 144, 0x01074f6664c250e9),
    ];
    for &(fans, brightness, count, expected) in fixtures {
        let effect = RgbEffect {
            mode: RgbMode::Kaleidoscope,
            brightness,
            colors: vec![],
            ..Default::default()
        };
        let top = render(&effect, fans, false).unwrap();
        let bottom = render(&effect, fans, true).unwrap();
        assert_eq!(top.len(), count);
        assert_eq!(bottom.len(), count);
        let mut hash = 0xcbf29ce484222325u64;
        for (a, b) in top.iter().flatten().zip(bottom.iter().flatten()) {
            for ch in 0..3 {
                hash ^= u64::from(a[ch] | b[ch]);
                hash = hash.wrapping_mul(0x100000001b3);
            }
        }
        assert_eq!(hash, expected, "{fans} fans brightness {brightness}");
    }
}
