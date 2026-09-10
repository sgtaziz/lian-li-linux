use super::engine::{brightness, palette, place, scale, Color, Plane};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn runway(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    let len = fans * plane.track_len();
    let span = len + 2 * fans - 1;
    let mut frames = Vec::with_capacity(span * 2);
    for pass in 0..2 {
        for step in 0..span {
            let mut track = vec![[0; 3]; len];
            for position in 0..len {
                let color = usize::from(!(position <= step && position + 2 * fans > step));
                let target = if pass == 0 {
                    position
                } else {
                    len - position - 1
                };
                track[target] = colors[color];
            }
            frames.push(place(&track, plane, fans, pn, true));
        }
    }
    frames
}

pub(super) fn meteor(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    const TAILS: [[u16; 8]; 4] = [
        [32, 255, 0, 0, 0, 0, 0, 0],
        [16, 64, 128, 255, 0, 0, 0, 0],
        [8, 16, 32, 64, 128, 255, 0, 0],
        [6, 10, 16, 32, 64, 96, 168, 255],
    ];
    let colors = palette(effect, plane);
    let bright = brightness(effect);
    let len = fans * plane.track_len();
    let span = len + 2 * fans - 1;
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    let mut frames = Vec::with_capacity(span * 4);
    for color in colors {
        for step in 0..span {
            let mut tail = 0;
            let mut track = vec![[0; 3]; len];
            for position in 0..len {
                if position <= step && position + 2 * fans > step {
                    let target = if reverse {
                        len - position - 1
                    } else {
                        position
                    };
                    track[target] = scale(scale(color, TAILS[fans - 1][tail]), bright);
                    tail += 1;
                }
            }
            frames.push(place(&track, plane, fans, pn, true));
        }
    }
    frames
}

pub(super) fn tai_chi(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    let width = plane.track_len();
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    (0..width)
        .map(|frame| {
            let mut segment = vec![[0; 3]; width];
            for position in 0..width {
                let color = usize::from((frame + position) % width >= width / 2);
                let target = if reverse {
                    width - position - 1
                } else {
                    position
                };
                segment[target] = colors[color];
            }
            let track = segment.repeat(fans);
            place(&track, plane, fans, pn, true)
        })
        .collect()
}
