use crate::rgb::color::{scale, Color};

pub(crate) fn rainbow_morph(
    track_len: usize,
    bright: u16,
    project: impl Fn(&[Color]) -> Vec<Color>,
) -> Vec<Vec<Color>> {
    let mut red = 255u8;
    let mut green = 0u8;
    let mut blue = 0u8;
    let mut source = Vec::with_capacity(255);
    for frame in 0..255 {
        source.push(project(&vec![scale([red, green, blue], bright); track_len]));
        if frame < 85 {
            red = red.wrapping_sub(3);
            green = green.wrapping_add(3);
            blue = 0;
        } else if frame < 170 {
            red = 0;
            green = green.wrapping_sub(3);
            blue = blue.wrapping_add(3);
        } else {
            red = red.wrapping_add(3);
            green = 0;
            blue = blue.wrapping_sub(3);
        }
    }
    source
}

pub(crate) fn static_color(
    colors: &[Color],
    per_fan: usize,
    bright: u16,
    project: impl Fn(&[Color]) -> Vec<Color>,
) -> Vec<Vec<Color>> {
    let mut track = Vec::with_capacity(colors.len() * per_fan);
    for color in colors.iter() {
        track.extend(std::iter::repeat_n(scale(*color, bright), per_fan));
    }
    vec![project(&track)]
}

pub(crate) fn breathing(
    colors: &[Color],
    per_fan: usize,
    bright: u16,
    project: impl Fn(&[Color]) -> Vec<Color>,
) -> Vec<Vec<Color>> {
    let mut level = 0u32;
    let mut frames = Vec::with_capacity(170);
    for frame in 0..170 {
        let intensity = ((level * 3) & 0xff) as u16;
        let mut track = Vec::with_capacity(colors.len() * per_fan);
        for color in colors.iter() {
            track.extend(std::iter::repeat_n(
                scale(scale(*color, intensity), bright),
                per_fan,
            ));
        }
        frames.push(project(&track));
        level = if frame >= 85 { level - 1 } else { level + 1 };
    }
    frames
}
