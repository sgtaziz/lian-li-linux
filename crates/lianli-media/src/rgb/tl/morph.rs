use anyhow::{ensure, Result};
use lianli_shared::rgb::RgbEffect;

fn scale(c: [u8; 3], brightness: u8) -> [u8; 3] {
    let b = [0u16, 64, 128, 192, 255][brightness.min(4) as usize];
    c.map(|v| ((u16::from(v) * b) >> 8) as u8)
}

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<[u8; 3]>>> {
    ensure!((1..=4).contains(&fans), "TL fan count must be 1..=4");
    let mut source = Vec::with_capacity(127);
    let (mut r, mut g, mut b) = (255i16, 0i16, 0i16);
    for i in 0..255 {
        if i % 2 == 0 && i < 254 {
            let c = scale([r as u8, g as u8, b as u8], effect.brightness);
            let mut frame = vec![c; fans * 26];
            for fan in 0..fans {
                if bottom {
                    frame[fan * 26..fan * 26 + 13].fill([0; 3]);
                } else {
                    frame[fan * 26 + 13..fan * 26 + 26].fill([0; 3]);
                }
            }
            source.push(frame);
        }
        if i < 85 {
            r -= 3;
            g += 3;
        } else if i < 170 {
            g -= 3;
            b += 3;
        } else {
            r += 3;
            b -= 3;
        }
    }
    Ok(source)
}
