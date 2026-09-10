use super::engine::{brightness, palette, place, scale, Color, Plane};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn return_arc(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    let mut frames = Vec::with_capacity(64);
    for color in colors {
        for pass in 0..2 {
            for step in 0..8 {
                let mut fan = [color; 8];
                for position in 0..8 {
                    let target = if reverse { 7 - position } else { position };
                    if (pass == 0 && position >= step) || (pass == 1 && position > 7 - step) {
                        fan[target] = [0; 3];
                    }
                }
                frames.push(place(&fan.repeat(fans), plane, fans));
            }
        }
    }
    frames
}

pub(super) fn double_arc(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    let mut frames = Vec::with_capacity(32);
    for color in colors {
        for pass in 0..2 {
            for step in 0..4 {
                let mut half = [color; 4];
                for position in 0..4 {
                    let target = if reverse { 3 - position } else { position };
                    if (pass == 0 && position >= step) || (pass == 1 && position > 3 - step) {
                        half[target] = [0; 3];
                    }
                }
                let mut fan = [[0; 3]; 8];
                fan[..4].copy_from_slice(&half);
                for (target, color) in fan[4..].iter_mut().zip(half.iter().rev()) {
                    *target = *color;
                }
                frames.push(place(&fan.repeat(fans), plane, fans));
            }
        }
    }
    frames
}

pub(super) fn boomerang(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let colors = palette(effect, Plane::Outer).map(|color| scale(color, brightness(effect)));
    let len = fans * 8;
    let end = len + 2 * fans - 1;
    let mut frames = Vec::with_capacity(2 * end);
    for pass in 0..2 {
        for step in 0..end {
            let mut track = vec![[0; 3]; len];
            match plane {
                Plane::Center => {
                    let (first, second) = boom_masks(fans, step);
                    for (position, target) in track.iter_mut().enumerate() {
                        let color_index = if first & (1 << position) != 0 {
                            pass
                        } else if second & (1 << position) != 0 {
                            1 - pass
                        } else {
                            continue;
                        };
                        *target = colors[color_index];
                    }
                }
                Plane::Outer => {
                    for position in 0..len {
                        if position <= step && position + fans > step {
                            let target = if pass == 0 {
                                position
                            } else {
                                len - position - 1
                            };
                            track[target] = colors[pass];
                        }
                    }
                }
            }
            frames.push(place(&track, plane, fans));
        }
    }
    frames
}

fn boom_masks(fans: usize, step: usize) -> (u32, u32) {
    match fans {
        1 => (BOOM_1_FIRST[step], BOOM_1_SECOND[step]),
        2 => (BOOM_2_FIRST[step], BOOM_2_SECOND[step]),
        3 => (BOOM_3_FIRST[step], BOOM_3_SECOND[step]),
        4 => (BOOM_4_FIRST[step], BOOM_4_SECOND[step]),
        _ => unreachable!(),
    }
}

const BOOM_1_FIRST: [u32; 9] = [2, 2, 3, 3, 0x83, 0x83, 0xc3, 0xc3, 0];

const BOOM_1_SECOND: [u32; 9] = [0x20, 0x20, 0x30, 0x30, 0x38, 0x38, 0x3c, 0x3c, 0];

const BOOM_2_FIRST: [u32; 19] = [
    4, 4, 0xc, 0xc, 0x1c, 0x1c, 0x3c, 0x3c, 0x23c, 0x23c, 0x33c, 0x33c, 0x833c, 0x833c, 0xc33c,
    0xc33c, 0xc33c, 0, 0,
];

const BOOM_2_SECOND: [u32; 19] = [
    0x2000, 0x2000, 0x3000, 0x3000, 0x3800, 0x3800, 0x3c00, 0x3c00, 0x3c40, 0x3c40, 0x3cc0, 0x3cc0,
    0x3cc1, 0x3cc1, 0x3cc3, 0x3cc3, 0x3cc3, 0, 0,
];

const BOOM_3_FIRST: [u32; 29] = [
    4, 4, 0xc, 0xc, 0x1c, 0x1c, 0x3c, 0x3c, 0x23c, 0x23c, 0x33c, 0x33c, 0x833c, 0x833c, 0xc33c,
    0xc33c, 0x4c33c, 0x4c33c, 0xcc33c, 0xcc33c, 0x1cc33c, 0x1cc33c, 0x3cc33c, 0x3cc33c, 0x3cc33c,
    0x3cc33c, 0x3cc33c, 0x3cc33c, 0,
];

const BOOM_3_SECOND: [u32; 29] = [
    0x400000, 0x400000, 0xc00000, 0xc00000, 0xc10000, 0xc10000, 0xc30000, 0xc30000, 0xc32000,
    0xc32000, 0xc33000, 0xc33000, 0xc33800, 0xc33800, 0xc33c00, 0xc33c00, 0xc33c40, 0xc33c40,
    0xc33cc0, 0xc33cc0, 0xc33cc1, 0xc33cc1, 0xc33cc3, 0xc33cc3, 0xc33cc3, 0xc33cc3, 0xc33cc3,
    0xc33cc3, 0,
];

const BOOM_4_FIRST: [u32; 39] = [
    4, 4, 0xc, 0xc, 0x1c, 0x1c, 0x3c, 0x3c, 0x23c, 0x23c, 0x33c, 0x33c, 0x833c, 0x833c, 0xc33c,
    0xc33c, 0x4c33c, 0x4c33c, 0xcc33c, 0xcc33c, 0x1cc33c, 0x1cc33c, 0x3cc33c, 0x3cc33c, 0x23cc33c,
    0x23cc33c, 0x33cc33c, 0x33cc33c, 0x833cc33c, 0x833cc33c, 0xc33cc33c, 0xc33cc33c, 0xc33cc33c,
    0xc33cc33c, 0xc33cc33c, 0xc33cc33c, 0, 0, 0,
];

const BOOM_4_SECOND: [u32; 39] = [
    0x20000000, 0x20000000, 0x30000000, 0x30000000, 0x38000000, 0x38000000, 0x3c000000, 0x3c000000,
    0x3c400000, 0x3c400000, 0x3cc00000, 0x3cc00000, 0x3cc10000, 0x3cc10000, 0x3cc30000, 0x3cc30000,
    0x3cc32000, 0x3cc32000, 0x3cc33000, 0x3cc33000, 0x3cc33800, 0x3cc33800, 0x3cc33c00, 0x3cc33c00,
    0x3cc33c40, 0x3cc33c40, 0x3cc33cc0, 0x3cc33cc0, 0x3cc33cc1, 0x3cc33cc1, 0x3cc33cc3, 0x3cc33cc3,
    0x3cc33cc3, 0x3cc33cc3, 0x3cc33cc3, 0x3cc33cc3, 0, 0, 0,
];
