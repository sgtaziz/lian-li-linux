use super::effect_patterns::{WING_2, WING_3, WING_4, WING_5};
use super::engine::{brightness, palette, place, scale, Color, Plane};
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
