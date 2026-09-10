use anyhow::{ensure, Result};
use lianli_shared::rgb::RgbEffect;

fn scale(color: [u8; 3], brightness: u8) -> [u8; 3] {
    let brightness = [0u16, 64, 128, 192, 255][brightness.min(4) as usize];
    color.map(|channel| ((u16::from(channel) * brightness) >> 8) as u8)
}

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<[u8; 3]>>> {
    ensure!((1..=4).contains(&fans), "TL fan count must be 1..=4");
    let led_count = fans * 13;
    let pass_frames = led_count + 2 * fans - 1;
    let runway_color = effect.colors.first().copied().unwrap_or([0; 3]);
    let background_color = effect.colors.get(1).copied().unwrap_or([0; 3]);
    let mut frames = Vec::with_capacity(pass_frames * 2);

    for reverse in [false, true] {
        for step in 0..pass_frames {
            let mut frame = vec![[0; 3]; fans * 26];
            for position in 0..led_count {
                let color = if position <= step && position + 2 * fans > step {
                    runway_color
                } else {
                    background_color
                };
                let destination = if reverse {
                    led_count - position - 1
                } else {
                    position
                };
                let physical = destination / 13 * 26 + usize::from(bottom) * 13 + destination % 13;
                frame[physical] = scale(color, effect.brightness);
            }
            frames.push(frame);
        }
    }

    Ok(frames)
}
