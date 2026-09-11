use super::collision::render;
use lianli_shared::rgb::{RgbEffect, RgbMode};

#[test]
fn collision_modes_match_extracted_csharp_frames_at_every_brightness() {
    let fixtures: &[(RgbMode, usize, u8, usize, u64)] = &[
        (RgbMode::Collide, 1, 0, 35, 0x3f89cb06add8980d),
        (RgbMode::Collide, 1, 1, 35, 0x36583299b8066b75),
        (RgbMode::Collide, 1, 2, 35, 0xc696cd37b4237c85),
        (RgbMode::Collide, 1, 3, 35, 0x2d1d2c462d41aba3),
        (RgbMode::Collide, 1, 4, 35, 0x55c10c00b05b2e07),
        (RgbMode::Collide, 2, 0, 63, 0xd980f8fe6f30fb75),
        (RgbMode::Collide, 2, 1, 63, 0x3bcb01156de146d5),
        (RgbMode::Collide, 2, 2, 63, 0xef933ef1d7b7a74f),
        (RgbMode::Collide, 2, 3, 63, 0x873aa8763a83313d),
        (RgbMode::Collide, 2, 4, 63, 0xe54a9fe90bb31d6b),
        (RgbMode::Collide, 3, 0, 91, 0x170f79dbb2f0855d),
        (RgbMode::Collide, 3, 1, 91, 0x1a16a711855bd221),
        (RgbMode::Collide, 3, 2, 91, 0x0122533edcda1905),
        (RgbMode::Collide, 3, 3, 91, 0x1120a9bcf69d044f),
        (RgbMode::Collide, 3, 4, 91, 0x69d819df02fc40d5),
        (RgbMode::Collide, 4, 0, 126, 0x03ded957a5db2265),
        (RgbMode::Collide, 4, 1, 126, 0xbb6314bf81c75597),
        (RgbMode::Collide, 4, 2, 126, 0xf1aa17e682cc3775),
        (RgbMode::Collide, 4, 3, 126, 0x0d764be45c819431),
        (RgbMode::Collide, 4, 4, 126, 0xbaebf6f4547c3edb),
        (RgbMode::ElectricCurrent, 1, 0, 55, 0xb59011d2b016b6ed),
        (RgbMode::ElectricCurrent, 1, 1, 55, 0x2647c6e132591999),
        (RgbMode::ElectricCurrent, 1, 2, 55, 0x90024d9f6dca2fd1),
        (RgbMode::ElectricCurrent, 1, 3, 55, 0x4d226fb013d0375d),
        (RgbMode::ElectricCurrent, 1, 4, 55, 0xadfcf924c447236b),
        (RgbMode::ElectricCurrent, 2, 0, 106, 0xbca019ba6b8bad05),
        (RgbMode::ElectricCurrent, 2, 1, 106, 0x9c0e098d7aa5c1a5),
        (RgbMode::ElectricCurrent, 2, 2, 106, 0xd46c54f0e1298985),
        (RgbMode::ElectricCurrent, 2, 3, 106, 0xa7455e46d7a2280f),
        (RgbMode::ElectricCurrent, 2, 4, 106, 0x121c279517cb102b),
        (RgbMode::ElectricCurrent, 3, 0, 146, 0xd6c7a647abcb7cb5),
        (RgbMode::ElectricCurrent, 3, 1, 146, 0x0b90e5a3ae5f428b),
        (RgbMode::ElectricCurrent, 3, 2, 146, 0x04edc5d1a683ff2f),
        (RgbMode::ElectricCurrent, 3, 3, 146, 0x4c21991c2b6fca51),
        (RgbMode::ElectricCurrent, 3, 4, 146, 0x1007e24f473bf3ff),
        (RgbMode::ElectricCurrent, 4, 0, 184, 0xdb4cc1d345de8825),
        (RgbMode::ElectricCurrent, 4, 1, 184, 0x248c40a4e9563121),
        (RgbMode::ElectricCurrent, 4, 2, 184, 0x6101336db074ffe9),
        (RgbMode::ElectricCurrent, 4, 3, 184, 0x63bb00db36609479),
        (RgbMode::ElectricCurrent, 4, 4, 184, 0x6f284c5e954f9733),
    ];
    for &(mode, fans, brightness, count, expected) in fixtures {
        let effect = RgbEffect {
            mode,
            brightness,
            colors: vec![[211, 17, 67], [13, 199, 29], [31, 47, 173], [107, 83, 23]],
            ..Default::default()
        };
        let top = render(&effect, fans, false).unwrap();
        let bottom = render(&effect, fans, true).unwrap();
        assert_eq!(top.len(), count);
        assert_eq!(bottom.len(), count);
        let mut hash = 0xcbf29ce484222325u64;
        for (a, b) in top.iter().flatten().zip(bottom.iter().flatten()) {
            for channel in 0..3 {
                hash ^= u64::from(a[channel] | b[channel]);
                hash = hash.wrapping_mul(0x100000001b3);
            }
        }
        assert_eq!(
            hash, expected,
            "{mode:?}, {fans} fans, brightness {brightness}"
        );
    }
}
