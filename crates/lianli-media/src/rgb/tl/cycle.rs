use anyhow::{ensure, Result};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

fn scale(color: [u8; 3], brightness: u8) -> [u8; 3] {
    let brightness = [0u16, 64, 128, 192, 255][brightness.min(4) as usize];
    color.map(|channel| ((u16::from(channel) * brightness) >> 8) as u8)
}

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<[u8; 3]>>> {
    ensure!((1..=4).contains(&fans), "TL fan count must be 1..=4");
    let led_count = fans * 13;
    let lit_leds = [1, 2, 5, 8][fans - 1];
    let gap_leds = [3, 6, 8, 9][fans - 1];
    let mut pattern = Vec::new();
    for color_index in 0..3 {
        pattern.extend(std::iter::repeat_n(color_index, lit_leds));
        pattern.extend(std::iter::repeat_n(4, gap_leds));
        if !bottom && (fans == 1 || fans == 4) {
            pattern.pop();
        }
        if !bottom && fans == 2 {
            pattern.push(4);
        }
    }
    pattern.resize(led_count, 4);

    let mut frames = Vec::with_capacity(led_count);
    for shift in 0..led_count {
        let mut frame = vec![[0; 3]; fans * 26];
        for position in 0..led_count {
            let destination = if !matches!(effect.direction, RgbDirection::CounterClockwise) {
                led_count - position - 1
            } else {
                position
            };
            let color_index = pattern[(position + shift) % led_count];
            let color = if color_index < 4 {
                effect.colors.get(color_index).copied().unwrap_or([0; 3])
            } else {
                [0; 3]
            };
            let physical = destination / 13 * 26 + usize::from(bottom) * 13 + destination % 13;
            frame[physical] = scale(color, effect.brightness);
        }
        frames.push(frame);
    }

    Ok(frames)
}
