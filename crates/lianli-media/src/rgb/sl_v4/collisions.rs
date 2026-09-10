use super::engine::{brightness, palette, place, scale, Color, Side};
use lianli_shared::rgb::RgbEffect;

pub(super) fn ripple(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    crate::rgb::effects::collisions::ripple(fans * side.leds_per_track(), fans, colors, |track| {
        place(track, side, fans)
    })
}

pub(super) fn collide(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    crate::rgb::effects::collisions::collide(side.mirrored_track_len(fans), fans, colors, |track| {
        place(&track[..fans * 13], side, fans)
    })
}

pub(super) fn reflect(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    crate::rgb::effects::collisions::reflect(side.mirrored_track_len(fans), fans, colors, |track| {
        place(&track[..fans * 13], side, fans)
    })
}

pub(super) fn electric_current(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    crate::rgb::effects::collisions::electric_current(
        side.mirrored_track_len(fans),
        fans,
        colors,
        |track| place(&track[..fans * 13], side, fans),
    )
}
