use super::engine::{brightness, palette, place, scale, Color, Plane};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn lottery(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    if plane == Plane::Center {
        let reverse = !matches!(effect.direction, RgbDirection::CounterClockwise);
        return (0..8)
            .map(|step| {
                let mut fan = [colors[1]; 8];
                let target = if reverse { 7 - step } else { step };
                fan[target] = colors[0];
                place(&fan.repeat(fans), plane, fans)
            })
            .collect();
    }

    let half = fans * 4;
    let mut frames = Vec::with_capacity(2 * (half + 2 * fans - 1));
    for pass in 0..2 {
        for step in 0..half + 2 * fans - 1 {
            let mut track = vec![[0; 3]; fans * 8];
            for position in 0..half {
                let color = if position <= step && position + 2 * fans > step {
                    colors[0]
                } else {
                    colors[1]
                };
                let first = if pass == 1 {
                    half - position - 1
                } else {
                    position
                };
                let second = if pass == 1 {
                    position
                } else {
                    half - position - 1
                };
                track[first] = color;
                track[half + second] = color;
            }
            frames.push(place(&track, plane, fans));
        }
    }
    frames
}
