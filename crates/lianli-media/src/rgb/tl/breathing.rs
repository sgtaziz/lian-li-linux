use anyhow::{ensure, Result};
use lianli_shared::rgb::RgbEffect;

fn scale(c: [u8; 3], v: usize, brightness: u8) -> [u8; 3] {
    let b = [0u16, 64, 128, 192, 255][brightness.min(4) as usize];
    c.map(|x| ((((u16::from(x) * v as u16) >> 8) * b) >> 8) as u8)
}

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<[u8; 3]>>> {
    ensure!((1..=4).contains(&fans), "TL fan count must be 1..=4");
    let colors = (0..fans * 13)
        .map(|i| effect.colors.get(i / 13).copied().unwrap_or([0; 3]))
        .collect::<Vec<_>>();
    let mut frames = Vec::with_capacity(170);
    for i in 0..170 {
        let v = if i < 85 { i * 3 } else { (170 - i) * 3 };
        let mut frame = vec![[0; 3]; fans * 26];
        for fan in 0..fans {
            let base = fan * 26 + usize::from(bottom) * 13;
            for led in 0..13 {
                frame[base + led] = scale(colors[fan * 13 + led], v, effect.brightness);
            }
        }
        frames.push(frame);
    }
    Ok(frames)
}
