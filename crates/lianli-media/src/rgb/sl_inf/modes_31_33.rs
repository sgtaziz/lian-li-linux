use super::effect_patterns::*;
use super::engine::{brightness, drumming_palette, palette, place, scale, Color, Plane};
use lianli_shared::rgb::RgbEffect;

pub(super) fn wing(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    let original = palette(effect, Plane::Outer);
    let mapped = match plane {
        Plane::Outer => original,
        Plane::Inner | Plane::Center => [original[1], original[0], original[3], original[2]],
    }
    .map(|color| scale(color, brightness(effect)));
    let mixed = |first: Color, second: Color| {
        scale(
            std::array::from_fn(|channel| first[channel].saturating_add(second[channel])),
            brightness(effect),
        )
    };
    let outer_half = 4 * fans;
    let inner_half = 5 * fans;
    let span = outer_half + 2 * fans - 1;
    let mut frames = Vec::with_capacity(2 * span);
    for pair in 0..2 {
        let mut inner_step = 0;
        for step in 0..span {
            let track = match plane {
                Plane::Center => {
                    let mut track = vec![[0; 3]; outer_half * 2];
                    for half_position in 0..outer_half {
                        for side in 0..2 {
                            let lit = wing_lit(fans, pn, step, half_position, side);
                            track[side * outer_half + half_position] =
                                if lit { mapped[pair * 2 + side] } else { [0; 3] };
                        }
                    }
                    if (fans == 1 || fans == 3) && step < outer_half + fans - 1 {
                        let blend = mixed(original[pair * 2], original[pair * 2 + 1]);
                        let blend_at = match (fans, pn) {
                            (3, 0) => 12,
                            (3, _) => 8,
                            (1, 0) => 4,
                            (1, _) => 0,
                            _ => unreachable!(),
                        };
                        track[blend_at] = blend;
                        if step > 2 {
                            match (fans, pn) {
                                (3, 0) => track[11] = mapped[pair * 2],
                                (3, _) => {
                                    track[15] = mapped[pair * 2];
                                    track[9] = mapped[pair * 2 + 1];
                                }
                                (1, 0) => {
                                    track[3] = mapped[pair * 2];
                                    track[5] = mapped[pair * 2 + 1];
                                }
                                (1, _) => {
                                    track[7] = mapped[pair * 2];
                                    track[1] = mapped[pair * 2 + 1];
                                }
                                _ => unreachable!(),
                            }
                        }
                    }
                    track
                }
                Plane::Outer => moving_pair(outer_half, step, fans, mapped, pair),
                Plane::Inner => {
                    if matches!(step, 2 | 6 | 10 | 14) {
                        inner_step += 1;
                    }
                    let track = moving_pair(inner_half, inner_step, fans, mapped, pair);
                    inner_step += 1;
                    track
                }
            };
            frames.push(place(&track, plane, fans, pn, plane != Plane::Center));
        }
    }
    frames
}

fn moving_pair(
    half: usize,
    step: usize,
    fans: usize,
    colors: [Color; 4],
    pair: usize,
) -> Vec<Color> {
    let mut track = vec![[0; 3]; half * 2];
    for position in 0..half {
        if position <= step && position + fans > step {
            track[half - position - 1] = colors[pair * 2];
            track[half + position] = colors[pair * 2 + 1];
        }
    }
    track
}

fn wing_lit(fans: usize, pn: u8, step: usize, position: usize, side: usize) -> bool {
    if fans == 1 {
        return false;
    }
    let half = 4 * fans;
    let (data, columns, fixed_row, column) = match fans {
        2 => (
            if pn == 0 { WING_4 } else { WING_5 },
            32,
            7,
            position + 8 + side * 8,
        ),
        3 => (
            if pn == 0 { WING_2 } else { WING_3 },
            24,
            11,
            position + side * 12,
        ),
        4 => (
            if pn == 0 { WING_4 } else { WING_5 },
            32,
            15,
            position + side * 16,
        ),
        _ => unreachable!(),
    };
    let (row, active) = if step < half {
        (step, true)
    } else {
        (fixed_row, step < half + fans - 1)
    };
    active && packed_bit(data, row * columns + column)
}

fn packed_bit(data: &[u8], index: usize) -> bool {
    data[index / 8] & (1 << (index % 8)) != 0
}

pub(super) fn drumming(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    const PULSE: [u16; 9] = [51, 102, 153, 204, 255, 204, 153, 102, 51];
    let colors = drumming_palette(effect);
    let bright = brightness(effect);
    let source_slots: &[usize] = match fans {
        1 => &[0],
        2 => &[0, 1],
        3 => &[0, 1, 3],
        4 => &[0, 1, 2, 3],
        _ => unreachable!(),
    };
    let width = plane.track_len();
    let mut frames = Vec::with_capacity(108);
    for frame in 0..108 {
        let block = frame / 9;
        let mut track = vec![[0; 3]; fans * width];
        for (fan, &slot) in source_slots.iter().enumerate() {
            let color_index = match block {
                2..=5 => 1,
                8 | 9 => usize::from(matches!(slot, 1 | 2)),
                10 | 11 => usize::from(matches!(slot, 0 | 3)),
                _ => 0,
            };
            let active = match block {
                0 | 1 | 4 | 5 => matches!(slot, 0 | 3),
                2 | 3 | 6 | 7 => matches!(slot, 1 | 2),
                _ => true,
            };
            let intensity = if plane == Plane::Outer || active {
                PULSE[frame % 9]
            } else {
                0
            };
            let value = scale(scale(colors[color_index], intensity), bright);
            track[fan * width..(fan + 1) * width].fill(value);
        }
        frames.push(place(&track, plane, fans, pn, true));
    }
    frames
}

pub(super) fn boomerang(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    let colors = palette(effect, Plane::Outer).map(|color| scale(color, brightness(effect)));
    let center_len = 8 * fans;
    let inner_len = 10 * fans;
    let span = center_len + 2 * fans - 1;
    let mut frames = Vec::with_capacity(2 * span);
    for pass in 0..2 {
        let mut inner_step = 0;
        for step in 0..span {
            let track = match plane {
                Plane::Center => {
                    let pattern = boomerang_pattern(fans, pn);
                    (0..center_len)
                        .map(
                            |position| match packed_pair(pattern, step * center_len + position) {
                                0 => [0; 3],
                                1 if pass == 0 => colors[0],
                                1 => colors[1],
                                2 if pass == 0 => colors[1],
                                2 => colors[0],
                                _ => unreachable!(),
                            },
                        )
                        .collect()
                }
                Plane::Outer => one_way_tail(center_len, step, fans, pass, colors[0]),
                Plane::Inner => {
                    if matches!(step, 2 | 6 | 10 | 14 | 18 | 22 | 26 | 30) {
                        inner_step += 1;
                    }
                    let track = one_way_tail(inner_len, inner_step, fans, 1 - pass, colors[1]);
                    inner_step += 1;
                    track
                }
            };
            frames.push(place(&track, plane, fans, pn, plane != Plane::Center));
        }
    }
    frames
}

fn one_way_tail(len: usize, step: usize, fans: usize, pass: usize, color: Color) -> Vec<Color> {
    let mut track = vec![[0; 3]; len];
    for position in 0..len {
        if position <= step && position + fans > step {
            let target = if pass == 0 {
                position
            } else {
                len - position - 1
            };
            track[target] = color;
        }
    }
    track
}

fn boomerang_pattern(fans: usize, pn: u8) -> &'static [u8] {
    match (fans, pn) {
        (1, 0) => BOOM_3,
        (1, _) => BOOM_4,
        (2, 0) => BOOM_5,
        (2, _) => BOOM_6,
        (3, 0) => BOOM_7,
        (3, _) => BOOM_8,
        (4, 0) => BOOM_9,
        (4, _) => BOOM_10,
        _ => unreachable!(),
    }
}

fn packed_pair(data: &[u8], index: usize) -> u8 {
    (data[index / 4] >> (2 * (index % 4))) & 3
}
