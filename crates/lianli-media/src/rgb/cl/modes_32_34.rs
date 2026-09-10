use super::engine::{all_palette, brightness, palette, place, place_both, scale, Color, Plane};
use lianli_shared::rgb::RgbEffect;

const DRUM_PULSE: [u8; 9] = [51, 102, 153, 204, 255, 204, 153, 102, 51];
const DRUM_COLORS: [[usize; 4]; 12] = [
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [1, 1, 1, 1],
    [1, 1, 1, 1],
    [1, 1, 1, 1],
    [1, 1, 1, 1],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 1, 1, 0],
    [0, 1, 1, 0],
    [1, 0, 0, 1],
    [1, 0, 0, 1],
];

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

pub(super) fn drumming(effect: &RgbEffect, fans: usize, _plane: Plane) -> Vec<Vec<Color>> {
    let colors = all_palette(effect);
    let bright = brightness(effect);
    let columns: &[usize] = match fans {
        1 => &[0],
        2 => &[0, 1],
        3 => &[0, 1, 3],
        4 => &[0, 1, 2, 3],
        _ => unreachable!(),
    };
    (0..108)
        .map(|frame| {
            let pulse = DRUM_PULSE[frame % 9];
            let block = frame / 9;
            let mut center = Vec::with_capacity(fans * 8);
            let mut outer = Vec::with_capacity(fans * 8);
            for &column in columns {
                let center_color = DRUM_COLORS[block][column];
                let active = if frame >= 72 {
                    true
                } else if (frame / 18).is_multiple_of(2) {
                    column == 0 || column == 3
                } else {
                    column == 1 || column == 2
                };
                let center_pixel = if active {
                    scale(scale(colors[center_color], u16::from(pulse)), bright)
                } else {
                    [0; 3]
                };
                let outer_pixel = scale(
                    scale(colors[usize::from(center_color == 0)], u16::from(pulse)),
                    bright,
                );
                center.extend(std::iter::repeat_n(center_pixel, 8));
                outer.extend(std::iter::repeat_n(outer_pixel, 8));
            }
            place_both(&center, &outer, fans)
        })
        .collect()
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

pub(super) fn candy_box(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    const OUTER: [[Color; 4]; 2] = [
        [[127, 0, 255], [0, 255, 0], [127, 0, 255], [0, 255, 255]],
        [[255, 95, 0], [255, 0, 255], [207, 255, 0], [0, 255, 0]],
    ];
    const CENTER: [[Color; 4]; 2] = [
        [[255, 0, 47], [0, 0, 255], [255, 0, 255], [0, 255, 0]],
        [[255, 255, 0], [0, 255, 0], [255, 31, 0], [255, 0, 255]],
    ];
    let bright = brightness(effect);
    let mut frames = Vec::with_capacity(340);
    for phase in 0..2 {
        for frame in 0..170 {
            let intensity = if frame > 85 {
                (170 - frame) * 3
            } else {
                frame * 3
            } as u16;
            let colors = if plane == Plane::Center {
                CENTER[phase]
            } else {
                OUTER[phase]
            };
            let mut track = Vec::with_capacity(fans * 8);
            for color in colors.into_iter().take(fans) {
                let color = scale(scale(color, intensity), bright);
                track.extend(std::iter::repeat_n(color, 8));
            }
            frames.push(place(&track, plane, fans));
        }
    }
    frames
}
