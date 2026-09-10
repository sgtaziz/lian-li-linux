use anyhow::{ensure, Result};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

const TAILS: [[u16; 8]; 4] = [
    [32, 255, 0, 0, 0, 0, 0, 0],
    [16, 64, 128, 255, 0, 0, 0, 0],
    [8, 16, 32, 64, 128, 255, 0, 0],
    [6, 10, 16, 32, 64, 96, 168, 255],
];

fn scale(color: [u8; 3], intensity: u16, brightness: u8) -> [u8; 3] {
    let brightness = [0u16, 64, 128, 192, 255][brightness.min(4) as usize];
    color.map(|channel| ((((u16::from(channel) * intensity) >> 8) * brightness) >> 8) as u8)
}

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<[u8; 3]>>> {
    ensure!((1..=4).contains(&fans), "TL fan count must be 1..=4");
    let led_count = fans * 13;
    let color_frames = led_count + 2 * fans - 1;
    let mut frames = Vec::with_capacity(4 * color_frames);

    for color in effect
        .colors
        .iter()
        .copied()
        .chain(std::iter::repeat([0; 3]))
        .take(4)
    {
        for step in 0..color_frames {
            let mut frame = vec![[0; 3]; fans * 26];
            let mut tail_index = 0;
            for position in 0..led_count {
                let mut intensity = 0;
                if position <= step && position + 2 * fans > step {
                    intensity = TAILS[fans - 1][tail_index];
                    tail_index += 1;
                }
                let destination = if matches!(effect.direction, RgbDirection::CounterClockwise) {
                    led_count - position - 1
                } else {
                    position
                };
                let physical = destination / 13 * 26 + usize::from(bottom) * 13 + destination % 13;
                frame[physical] = scale(color, intensity, effect.brightness);
            }
            frames.push(frame);
        }
    }

    Ok(frames)
}
