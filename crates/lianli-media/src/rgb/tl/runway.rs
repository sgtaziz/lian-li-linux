use anyhow::{ensure, Result};
use lianli_shared::rgb::RgbEffect;

fn scale(color: [u8; 3], brightness: u8) -> [u8; 3] {
    let brightness = [0u16, 64, 128, 192, 255][brightness.min(4) as usize];
    color.map(|channel| ((u16::from(channel) * brightness) >> 8) as u8)
}

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<[u8; 3]>>> {
    ensure!((1..=4).contains(&fans), "TL fan count must be 1..=4");
    let colors = [
        effect.colors.first().copied().unwrap_or([0; 3]),
        effect.colors.get(1).copied().unwrap_or([0; 3]),
    ]
    .map(|color| scale(color, effect.brightness));
    Ok(crate::rgb::effects::chase::runway(
        fans * 13,
        2 * fans,
        colors,
        |track| super::engine::place_side_track(track, fans, bottom),
    ))
}
