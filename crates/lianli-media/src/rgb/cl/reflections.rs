use super::engine::{brightness, palette, place, scale, Color, Plane};
use lianli_shared::rgb::RgbEffect;

pub(super) fn electric_current(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    let half = fans * 4;
    let end = half + 2 * fans - 1;
    let mut frames = Vec::with_capacity(4 * (end + 9 + end.div_ceil(2)));
    for color in colors {
        let steps = std::iter::repeat_n(0, 10)
            .chain(1..end)
            .chain((0..end).step_by(2));
        for (frame_in_color, step) in steps.enumerate() {
            let reverse_halves = frame_in_color >= end + 9;
            let mut track = vec![[0; 3]; fans * 8];
            for position in 0..half {
                if position < step && position + 2 * fans > step {
                    let first = if reverse_halves {
                        half - position - 1
                    } else {
                        position
                    };
                    let second = if reverse_halves {
                        position
                    } else {
                        half - position - 1
                    };
                    track[first] = color;
                    track[half + second] = color;
                }
            }
            frames.push(place(&track, plane, fans));
        }
    }
    frames
}

pub(super) fn reflect(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    const TAIL: [u8; 8] = [255, 192, 168, 128, 96, 64, 32, 16];
    let colors = palette(effect, plane);
    let bright = brightness(effect);
    let half = fans * 4;
    let end = half + 2 * fans - 1;
    let mut frames = Vec::with_capacity(8 * end);
    for color in colors {
        for pass in 0..2 {
            for step in 0..end {
                let mut track = vec![[0; 3]; fans * 8];
                if pass == 0 {
                    for position in 0..half {
                        if position <= step && position + 2 * fans > step {
                            let color =
                                scale(scale(color, u16::from(TAIL[step - position])), bright);
                            track[position] = color;
                            track[fans * 8 - position - 1] = color;
                        }
                    }
                }
                frames.push(place(&track, plane, fans));
            }
        }
    }
    frames
}
