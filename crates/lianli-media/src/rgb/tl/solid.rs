use anyhow::{ensure, Result};
use lianli_shared::rgb::RgbEffect;

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<[u8; 3]>>> {
    ensure!((1..=4).contains(&fans), "TL fan count must be 1..=4");
    let scale = [0u16, 64, 128, 192, 255][effect.brightness.min(4) as usize];
    let mut out = vec![[0; 3]; fans * 26];
    for fan in 0..fans {
        let color = effect.colors.get(fan).copied().unwrap_or([0; 3]);
        let color = color.map(|v| ((u16::from(v) * scale) >> 8) as u8);
        let base = fan * 26 + usize::from(bottom) * 13;
        out[base..base + 13].fill(color);
    }
    Ok(vec![out; 30])
}
