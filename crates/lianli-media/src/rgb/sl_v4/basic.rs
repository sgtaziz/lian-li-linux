use super::engine::{brightness, palette, place, scale, Color, Side};
use lianli_shared::rgb::RgbEffect;

pub(super) fn rainbow(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let len = fans * side.leds_per_track();
    let step = [4, 2, 1, 1][fans - 1];
    let table: &[Color] = if len == 39 { &RAINBOW_39 } else { &RAINBOW_52 };
    let clockwise = !matches!(
        effect.direction,
        lianli_shared::rgb::RgbDirection::CounterClockwise
    );
    let bright = brightness(effect);
    (0..len)
        .map(|frame| {
            let mut source = frame * step;
            let mut track = vec![[0; 3]; len];
            for position in 0..len {
                let target = if clockwise {
                    len - position - 1
                } else {
                    position
                };
                track[target] = scale(table[source], bright);
                source = (source + step) % table.len();
            }
            place(&track, side, fans)
        })
        .collect()
}

pub(super) fn rainbow_color(index: usize, table_len: usize) -> Color {
    let pulse = |position: usize| -> u8 {
        let position = position % table_len;
        match position {
            0 => 255,
            1..=31 => (256 - position * 8) as u8,
            32..=64 => 0,
            _ => ((position - 64) * 8) as u8,
        }
    };
    let third = table_len / 3;
    [pulse(index), pulse(index + third * 2), pulse(index + third)]
}

const RAINBOW_39: [Color; 39] = transpose([
    [
        255, 235, 215, 195, 175, 155, 135, 115, 95, 75, 55, 35, 15, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 15, 35, 55, 75, 95, 115, 135, 155, 175, 195, 215, 235,
    ],
    [
        0, 15, 35, 55, 75, 95, 115, 135, 155, 195, 215, 235, 255, 235, 215, 195, 175, 155, 135,
        115, 95, 75, 55, 35, 15, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 15, 35, 55, 75, 95, 115, 135, 155, 175, 195, 215,
        235, 255, 235, 215, 195, 175, 155, 135, 115, 95, 75, 55, 35, 15,
    ],
]);

const RAINBOW_52: [Color; 52] = transpose([
    [
        255, 239, 223, 207, 191, 175, 159, 143, 127, 111, 95, 79, 63, 47, 31, 15, 5, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 5, 15, 31, 47, 63, 79, 95, 111, 127, 143, 159, 175,
        191, 207, 223, 239, 250,
    ],
    [
        0, 15, 31, 47, 63, 79, 95, 111, 127, 143, 159, 175, 191, 207, 223, 239, 250, 255, 239, 223,
        207, 191, 175, 159, 143, 127, 111, 95, 79, 63, 47, 31, 15, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 15, 31, 47, 63, 79, 95, 111, 127,
        143, 159, 175, 191, 207, 223, 239, 250, 255, 250, 239, 223, 207, 191, 175, 159, 143, 127,
        111, 95, 79, 63, 47, 31, 15, 5,
    ],
]);

const fn transpose<const N: usize>(channels: [[u8; N]; 3]) -> [Color; N] {
    let mut colors = [[0; 3]; N];
    let mut index = 0;
    while index < N {
        colors[index] = [channels[0][index], channels[1][index], channels[2][index]];
        index += 1;
    }
    colors
}

pub(super) fn rainbow_morph(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let track_len = fans * side.leds_per_track();
    let bright = brightness(effect);
    let mut red = 255u8;
    let mut green = 0u8;
    let mut blue = 0u8;
    let mut source = Vec::with_capacity(255);
    for frame in 0..255 {
        source.push(place(
            &vec![scale([red, green, blue], bright); track_len],
            side,
            fans,
        ));
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

pub(super) fn static_color(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let colors = palette(effect, side);
    let bright = brightness(effect);
    let per_fan = side.leds_per_track();
    let mut track = Vec::with_capacity(fans * per_fan);
    for color in colors.iter().take(fans) {
        track.extend(std::iter::repeat_n(scale(*color, bright), per_fan));
    }
    vec![place(&track, side, fans)]
}

pub(super) fn breathing(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let colors = palette(effect, side);
    let bright = brightness(effect);
    let per_fan = side.leds_per_track();
    let mut level = 0u32;
    let mut frames = Vec::with_capacity(170);
    for frame in 0..170 {
        let intensity = ((level * 3) & 0xff) as u16;
        let mut track = Vec::with_capacity(fans * per_fan);
        for color in colors.iter().take(fans) {
            track.extend(std::iter::repeat_n(
                scale(scale(*color, intensity), bright),
                per_fan,
            ));
        }
        frames.push(place(&track, side, fans));
        level = if frame >= 85 { level - 1 } else { level + 1 };
    }
    frames
}
