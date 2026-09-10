use super::engine::{brightness, palette, place, scale, Color, Plane};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn lottery(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    if plane == Plane::Center {
        let clockwise = !matches!(effect.direction, RgbDirection::CounterClockwise);
        return (0..8)
            .map(|step| {
                let mut segment = [colors[1]; 8];
                segment[if clockwise { 7 - step } else { step }] = colors[0];
                place(&segment.repeat(fans), plane, fans, pn, true)
            })
            .collect();
    }
    let half = fans * plane.track_len() / 2;
    let mut frames = Vec::with_capacity(2 * (half + 2 * fans - 1));
    for pass in 0..2 {
        for step in 0..half + 2 * fans - 1 {
            let mut track = vec![[0; 3]; half * 2];
            for position in 0..half {
                let color = colors[usize::from(!(position <= step && position + 2 * fans > step))];
                let first = if pass == 0 {
                    position
                } else {
                    half - position - 1
                };
                let second = if pass == 0 {
                    half - position - 1
                } else {
                    position
                };
                track[first] = color;
                track[half + second] = color;
            }
            frames.push(place(&track, plane, fans, pn, true));
        }
    }
    frames
}
