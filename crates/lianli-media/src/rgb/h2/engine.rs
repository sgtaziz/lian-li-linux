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

fn clamp_color(mut color: Color) -> Color {
    while color.iter().map(|&channel| u16::from(channel)).sum::<u16>() > 570 {
        color = color.map(|channel| (f64::from(channel) * 0.95) as u8);
    }
    color
}

pub(super) fn scale(color: Color, value: u8) -> Color {
    color.map(|channel| ((u16::from(channel) * u16::from(value)) >> 8) as u8)
}

pub(super) fn frame() -> Vec<Color> {
    vec![[0; 3]; 24]
}
