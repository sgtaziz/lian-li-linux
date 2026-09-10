use lianli_shared::rgb::RgbEffect;

pub(super) type Color = [u8; 3];

pub(super) const ACTIVE_LEDS: usize = 35;
pub(super) const TOTAL_LEDS: usize = 45;

pub(super) fn palette(effect: &RgbEffect) -> [Color; 6] {
    // Older configurations allowed incomplete palettes; missing slots are not intentional black.
    let mut colors = [effect.colors.last().copied().unwrap_or([0; 3]); 6];
    for (target, source) in colors.iter_mut().zip(&effect.colors) {
        *target = *source;
    }
    colors
}

pub(super) fn scale(color: Color, value: u8) -> Color {
    color.map(|channel| ((u16::from(channel) * u16::from(value)) >> 8) as u8)
}

pub(super) fn frame() -> Vec<Color> {
    vec![[0; 3]; TOTAL_LEDS]
}

pub(super) fn mirrored_18(source: &[Color; 18]) -> Vec<Color> {
    let mut output = frame();
    for (index, color) in source.iter().copied().enumerate() {
        output[17 - index] = color;
        output[index + 17] = color;
    }
    output
}

pub(super) fn centered_15(source: &[Color; 15]) -> Vec<Color> {
    let mut output = frame();
    output[..14].copy_from_slice(&source[..14]);
    output[14..21].fill(source[14]);
    for (index, color) in source[..14].iter().copied().enumerate() {
        output[34 - index] = color;
    }
    output
}
