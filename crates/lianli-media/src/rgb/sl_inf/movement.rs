use super::engine::{brightness, palette, place, scale, Color, Plane};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn runway(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    crate::rgb::track_effects::runway(
        fans * plane.track_len(),
        2 * fans,
        [colors[0], colors[1]],
        |track| place(track, plane, fans, pn, true),
    )
}

pub(super) fn meteor(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    crate::rgb::track_effects::meteor(
        fans * plane.track_len(),
        &palette(effect, plane),
        crate::rgb::track_effects::METEOR_TAILS[fans - 1],
        brightness(effect),
        matches!(effect.direction, RgbDirection::CounterClockwise),
        |track| place(track, plane, fans, pn, true),
    )
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
