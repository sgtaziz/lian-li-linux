use anyhow::{ensure, Result};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

fn scale(color: [u8; 3], brightness: u8) -> [u8; 3] {
    let brightness = [0u16, 64, 128, 192, 255][brightness.min(4) as usize];
    color.map(|channel| ((u16::from(channel) * brightness) >> 8) as u8)
}

fn push_frame(frames: &mut Vec<Vec<[u8; 3]>>, logical: &[[u8; 3]], fans: usize, bottom: bool) {
    let mut frame = vec![[0; 3]; fans * 26];
    for (position, color) in logical.iter().enumerate() {
        let physical = position / 13 * 26 + usize::from(bottom) * 13 + position % 13;
        frame[physical] = *color;
    }
    frames.push(frame);
}

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<[u8; 3]>>> {
    ensure!((1..=4).contains(&fans), "TL fan count must be 1..=4");
    let led_count = fans * 13;
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    let mut logical = vec![[0; 3]; led_count];
    let mut frames = Vec::with_capacity(416 * fans);

    for color in effect
        .colors
        .iter()
        .copied()
        .chain(std::iter::repeat([0; 3]))
        .take(4)
    {
        let color = scale(color, effect.brightness);
        let mut removed = 0;
        for _ in 0..13 {
            let active_leds = led_count - removed;
            for step in 0..active_leds {
                for position in 0..active_leds {
                    let destination = if reverse {
                        led_count - position - 1
                    } else {
                        position
                    };
                    logical[destination] = if position <= step && position + fans > step {
                        color
                    } else {
                        [0; 3]
                    };
                }
                push_frame(&mut frames, &logical, fans, bottom);
            }
            removed += fans;
        }

        for step in 0..led_count {
            for position in 0..led_count {
                let destination = if reverse {
                    led_count - position - 1
                } else {
                    position
                };
                logical[destination] = if position > step { color } else { [0; 3] };
            }
            push_frame(&mut frames, &logical, fans, bottom);
        }
    }

    Ok(frames)
}
