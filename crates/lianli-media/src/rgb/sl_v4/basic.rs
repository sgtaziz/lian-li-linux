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
    crate::rgb::effects::basic::rainbow_morph(
        fans * side.leds_per_track(),
        brightness(effect),
        |track| place(track, side, fans),
    )
}

pub(super) fn static_color(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    crate::rgb::effects::basic::static_color(
        &palette(effect, side)[..fans],
        side.leds_per_track(),
        brightness(effect),
        |track| place(track, side, fans),
    )
}

pub(super) fn breathing(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    crate::rgb::effects::basic::breathing(
        &palette(effect, side)[..fans],
        side.leds_per_track(),
        brightness(effect),
        |track| place(track, side, fans),
    )
}
