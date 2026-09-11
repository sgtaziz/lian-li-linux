use super::engine::{brightness, mixed_color, palette, place, scale, Color, Plane};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn double_meteor(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    let mut frames = Vec::with_capacity(16);
    for (color_index, color) in colors.into_iter().enumerate() {
        for step in 0..4 {
            let mut fan = [[0; 3]; 8];
            let first = if color_index % 2 == 0 { step } else { 3 - step };
            let second = if color_index % 2 == 0 {
                7 - step
            } else {
                4 + step
            };
            fan[first] = color;
            fan[second] = color;
            frames.push(place(&fan.repeat(fans), plane, fans));
        }
    }
    frames
}

pub(super) fn meteor_contest(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    const PATTERN: [u8; 8] = [0, 0, 0, 255, 0, 0, 0, 254];
    let colors = palette(effect, plane);
    let bright = brightness(effect);
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    (0..8)
        .map(|frame| {
            let mut fan = [[0; 3]; 8];
            for position in 0..8 {
                let intensity = PATTERN[(frame + position) % 8];
                let color = if intensity == 254 {
                    colors[1]
                } else {
                    colors[0]
                };
                let target = if reverse { 7 - position } else { position };
                fan[target] = scale(scale(color, u16::from(intensity)), bright);
            }
            place(&fan.repeat(fans), plane, fans)
        })
        .collect()
}

pub(super) fn meteor_mix(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane);
    let mixed = mixed_color(colors[0], colors[1]);
    let bright = brightness(effect);
    let mut frames = Vec::with_capacity(8);
    for pass in 0..2 {
        for step in 0..4 {
            let mut fan = [[0; 3]; 8];
            let first = if pass == 0 { step } else { 3 - step };
            let second = if pass == 0 { 7 - step } else { 4 + step };
            let first_color = if pass == 1 && plane == Plane::Outer {
                mixed
            } else {
                colors[0]
            };
            let second_color = if pass == 1 && plane == Plane::Outer {
                mixed
            } else {
                colors[1]
            };
            fan[first] = scale(first_color, bright);
            fan[second] = scale(second_color, bright);
            frames.push(place(&fan.repeat(fans), plane, fans));
        }
    }
    frames
}
