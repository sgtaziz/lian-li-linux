use super::meteor_shower::render;
use lianli_shared::rgb::{RgbDirection, RgbEffect, RgbMode};
#[test]
fn seeded_meteor_shower_matches_extracted_csharp_frames() {
    let fixtures: &[(u8, usize, u8, usize, u64)] = &[
        (0, 1, 0, 76, 0x2f8f7ff69b0a2845),
        (0, 1, 1, 76, 0x00dd8fbfd16610f2),
        (0, 1, 2, 76, 0xbdff5f9435ce7f72),
        (0, 1, 3, 76, 0x0b5a87a912462bf2),
        (0, 1, 4, 76, 0x7ba009fc422cdb81),
        (0, 2, 0, 164, 0x879df3e7228901e5),
        (0, 2, 1, 164, 0x6883d893fd952b6a),
        (0, 2, 2, 164, 0xf9fc7f0ae3211d8a),
        (0, 2, 3, 164, 0x805d9e975a36ae2a),
        (0, 2, 4, 164, 0xbb93a718d5ea8f30),
        (0, 3, 0, 237, 0x10faef051a99e06d),
        (0, 3, 1, 237, 0x7ec24cbddb5a72dd),
        (0, 3, 2, 237, 0x14ccfbb7f7fbcbdd),
        (0, 3, 3, 237, 0x74ddf36447091bdd),
        (0, 3, 4, 237, 0x8850bfa2c0fe2ac9),
        (0, 4, 0, 332, 0x517fe6b873304fa5),
        (0, 4, 1, 332, 0x5c4903b1cc47cae3),
        (0, 4, 2, 332, 0x3dcd2978d3741ce3),
        (0, 4, 3, 332, 0xbb82caf984d12063),
        (0, 4, 4, 332, 0xf71baa11d787a2b5),
        (1, 1, 0, 76, 0x2f8f7ff69b0a2845),
        (1, 1, 1, 76, 0xaad1bc111c0d0100),
        (1, 1, 2, 76, 0x4bf10d6b1af27bc0),
        (1, 1, 3, 76, 0x5759fb8f041abd80),
        (1, 1, 4, 76, 0xb558c534cf7aa561),
        (1, 2, 0, 164, 0x879df3e7228901e5),
        (1, 2, 1, 164, 0x58c1e91239084f46),
        (1, 2, 2, 164, 0xc860fc4c8cec8146),
        (1, 2, 3, 164, 0x38d1363f49fdd9c6),
        (1, 2, 4, 164, 0x8ed3fc17915b09d7),
        (1, 3, 0, 237, 0x10faef051a99e06d),
        (1, 3, 1, 237, 0x510ac20c2d59a82a),
        (1, 3, 2, 237, 0xf342bba9dcc53f4a),
        (1, 3, 3, 237, 0x1de39761f9f783ea),
        (1, 3, 4, 237, 0xac37edb84c370f2a),
        (1, 4, 0, 332, 0x517fe6b873304fa5),
        (1, 4, 1, 332, 0x8154333f85a4b9a1),
        (1, 4, 2, 332, 0x464c481fd6500fc1),
        (1, 4, 3, 332, 0x743ddd1bd05ece61),
        (1, 4, 4, 332, 0x64d2733b212e60d8),
    ];
    for &(direction, fans, brightness, count, expected) in fixtures {
        let effect = RgbEffect {
            mode: RgbMode::MeteorShower,
            brightness,
            direction: if direction == 0 {
                RgbDirection::Clockwise
            } else {
                RgbDirection::CounterClockwise
            },
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
        assert_eq!(
            hash, expected,
            "{fans} fans brightness {brightness} direction {direction}"
        );
    }
}
