pub(super) use crate::rgb::color::scale_byte as scale;
use lianli_shared::rgb::RgbEffect;

pub(super) type Color = [u8; 3];

pub(super) fn palette(effect: &RgbEffect) -> [Color; 4] {
    let mut colors = [[255, 0, 0], [0, 0, 255], [0, 255, 0], [255, 255, 0]];
    if effect.colors.iter().flatten().any(|&channel| channel != 0) {
        for (target, source) in colors.iter_mut().zip(&effect.colors) {
            *target = clamp_color(*source);
        }
    }
    colors
}

fn clamp_color(color: Color) -> Color {
    crate::rgb::color::limit_current(color, 570)
}

pub(super) fn frame() -> Vec<Color> {
    vec![[0; 3]; 24]
}
