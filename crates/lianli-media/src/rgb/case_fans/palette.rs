pub(crate) use crate::rgb::color::scale_byte as scale;
use lianli_shared::rgb::RgbEffect;

pub(crate) type Color = [u8; 3];
pub(crate) type Frame = Vec<Color>;

pub(crate) fn palettes(effect: &RgbEffect) -> [[Color; 4]; 4] {
    let mut colors = [
        [[255, 0, 0], [0, 0, 255], [0, 255, 0], [255, 255, 0]],
        [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]],
        [[255, 0, 0], [255, 255, 0], [255, 0, 255], [255, 255, 0]],
        [[255, 0, 255], [0, 0, 255], [0, 255, 0], [255, 255, 0]],
    ];
    if effect.colors.iter().flatten().any(|&channel| channel != 0) {
        for palette in &mut colors {
            for (target, source) in palette.iter_mut().zip(&effect.colors) {
                *target = clamp_color(*source);
            }
        }
    }
    colors
}

fn clamp_color(color: Color) -> Color {
    crate::rgb::color::limit_current(color, 600)
}
