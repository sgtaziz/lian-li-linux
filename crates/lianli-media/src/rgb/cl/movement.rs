use super::engine::{brightness, palette, place, scale, Color, Plane};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn runway(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    crate::rgb::effects::chase::runway(fans * 8, 2 * fans, [colors[0], colors[1]], |track| {
        place(track, plane, fans)
    })
}

pub(super) fn meteor(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    crate::rgb::effects::chase::meteor(
        fans * 8,
        &palette(effect, plane),
        crate::rgb::effects::chase::METEOR_TAILS[fans - 1],
        brightness(effect),
        matches!(effect.direction, RgbDirection::CounterClockwise),
        |track| place(track, plane, fans),
    )
}

pub(super) fn tai_chi(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    const PATTERN: [usize; 8] = [0, 0, 0, 0, 1, 1, 1, 1];
    let colors = palette(effect, plane);
    let bright = brightness(effect);
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    (0..8)
        .map(|frame| {
            let first = (0..8)
                .map(|position| {
                    let source = (frame + position) % 8;
                    scale(colors[PATTERN[source]], bright)
                })
                .collect::<Vec<_>>();
            let first = if reverse {
                first.into_iter().rev().collect::<Vec<_>>()
            } else {
                first
            };
            let track = first.repeat(fans);
            place(&track, plane, fans)
        })
        .collect()
}

pub(super) fn color_cycle(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane);
    let bright = brightness(effect);
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    let len = fans * 8;
    let mut frames = Vec::with_capacity(4 * len);
    for color_index in 0..4 {
        let previous = if color_index == 0 { 3 } else { color_index - 1 };
        for step in 0..len {
            let mut track = vec![[0; 3]; len];
            for position in 0..len {
                let target = if reverse {
                    len - position - 1
                } else {
                    position
                };
                let color = if position <= step {
                    colors[color_index]
                } else {
                    colors[previous]
                };
                track[target] = scale(color, bright);
            }
            frames.push(place(&track, plane, fans));
        }
    }
    frames
}

pub(super) fn mop_up(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane);
    let bright = brightness(effect);
    let len = fans * 8;
    let mut frames = Vec::with_capacity(4 * (len + 2 * fans));
    for (color_index, color) in colors.into_iter().enumerate() {
        for step in 0..len + 2 * fans {
            let mut track = vec![[0; 3]; len];
            for position in 0..len {
                let target = if color_index == 1 || color_index == 3 {
                    len - position - 1
                } else {
                    position
                };
                if position <= step && position + 2 * fans > step {
                    track[target] = scale(color, bright);
                }
            }
            frames.push(place(&track, plane, fans));
        }
    }
    frames
}
