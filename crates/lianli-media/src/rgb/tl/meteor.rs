use anyhow::{ensure, Result};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<[u8; 3]>>> {
    ensure!((1..=4).contains(&fans), "TL fan count must be 1..=4");
    let colors =
        std::array::from_fn::<_, 4, _>(|index| effect.colors.get(index).copied().unwrap_or([0; 3]));
    let brightness = [0u16, 64, 128, 192, 255][effect.brightness.min(4) as usize];
    Ok(crate::rgb::effects::chase::meteor(
        fans * 13,
        &colors,
        crate::rgb::effects::chase::METEOR_TAILS[fans - 1],
        brightness,
        effect.direction == RgbDirection::CounterClockwise,
        |track| super::engine::place_side_track(track, fans, bottom),
    ))
}
