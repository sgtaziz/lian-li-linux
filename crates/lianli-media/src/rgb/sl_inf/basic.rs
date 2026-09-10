use super::engine::{brightness, center_order, palette, place, scale, Color, Plane};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

const LONG_DESC: [u8; 40] = [
    255, 248, 242, 235, 229, 222, 216, 209, 203, 196, 190, 183, 177, 170, 164, 157, 151, 144, 138,
    131, 125, 118, 112, 105, 99, 92, 86, 79, 73, 66, 60, 53, 47, 40, 34, 27, 21, 14, 8, 2,
];

pub(super) fn rainbow(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    let len = fans * plane.track_len();
    let step = [12, 6, 4, 3][fans - 1];
    let table_len = if plane == Plane::Inner { 120 } else { 96 };
    let clockwise = !matches!(effect.direction, RgbDirection::CounterClockwise);
    let bright = brightness(effect);
    (0..len)
        .map(|frame| {
            let mut sample = frame * step;
            let mut track = vec![[0; 3]; len];
            for position in 0..len {
                let logical = if clockwise {
                    len - position - 1
                } else {
                    position
                };
                let target = if plane == Plane::Center {
                    center_order(logical, pn)
                } else {
                    logical
                };
                track[target] = scale(rainbow_color(sample, table_len), bright);
                sample = (sample + step) % table_len;
            }
            place(&track, plane, fans, pn, plane != Plane::Center)
        })
        .collect()
}

fn rainbow_color(index: usize, table_len: usize) -> Color {
    let pulse = |position: usize| -> u8 {
        let position = position % table_len;
        if table_len == 96 {
            match position {
                0 => 255,
                1..=31 => (256 - position * 8) as u8,
                32..=64 => 0,
                _ => ((position - 64) * 8) as u8,
            }
        } else {
            match position {
                0..=39 => LONG_DESC[position],
                40..=80 => 0,
                _ => LONG_DESC[120 - position],
            }
        }
    };
    let third = table_len / 3;
    [pulse(index), pulse(index + third * 2), pulse(index + third)]
}

pub(super) fn rainbow_morph(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let len = fans * plane.track_len();
    let bright = brightness(effect);
    let mut rgb = [255u8, 0, 0];
    let mut frames = Vec::with_capacity(127);
    for frame in 0usize..255 {
        if frame < 254 && frame.is_multiple_of(2) {
            frames.push(place(&vec![scale(rgb, bright); len], plane, fans, 1, false));
        }
        if frame < 85 {
            rgb = [rgb[0].wrapping_sub(3), rgb[1].wrapping_add(3), 0];
        } else if frame < 170 {
            rgb = [0, rgb[1].wrapping_sub(3), rgb[2].wrapping_add(3)];
        } else {
            rgb = [rgb[0].wrapping_add(3), 0, rgb[2].wrapping_sub(3)];
        }
    }
    frames
}

pub(super) fn static_color(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane);
    let bright = brightness(effect);
    let mut track = Vec::with_capacity(fans * plane.track_len());
    for color in colors.iter().take(fans) {
        track.extend(std::iter::repeat_n(
            scale(*color, bright),
            plane.track_len(),
        ));
    }
    vec![place(&track, plane, fans, 1, false); 30]
}

pub(super) fn breathing(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane);
    let bright = brightness(effect);
    let mut level = 0u32;
    (0..170)
        .map(|frame| {
            let intensity = ((level * 3) & 0xff) as u16;
            let mut track = Vec::with_capacity(fans * plane.track_len());
            for color in colors.iter().take(fans) {
                track.extend(std::iter::repeat_n(
                    scale(scale(*color, intensity), bright),
                    plane.track_len(),
                ));
            }
            level = if frame >= 85 { level - 1 } else { level + 1 };
            place(&track, plane, fans, 1, false)
        })
        .collect()
}
