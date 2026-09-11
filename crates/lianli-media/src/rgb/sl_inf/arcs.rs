use super::effect_patterns::{BOOM_10, BOOM_3, BOOM_4, BOOM_5, BOOM_6, BOOM_7, BOOM_8, BOOM_9};
use super::engine::{brightness, center_order, palette, place, scale, Color, Plane};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn return_arc(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    arc(effect, fans, plane, pn, false)
}

fn arc(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8, double: bool) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    let width = plane.track_len();
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    let mut frames = Vec::with_capacity(8 * width);
    for color in colors {
        for pass in 0..2 {
            for step in 0..width {
                let mut segment = vec![[0; 3]; width];
                for position in 0..width {
                    let lit =
                        (pass == 0 && position < step) || (pass == 1 && position < width - step);
                    let logical = if reverse {
                        width - position - 1
                    } else {
                        position
                    };
                    let target = if double && plane == Plane::Center {
                        center_order(logical, pn)
                    } else {
                        logical
                    };
                    segment[target] = if lit { color } else { [0; 3] };
                }
                let track = (0..fans)
                    .flat_map(|_| segment.iter().copied())
                    .collect::<Vec<_>>();
                frames.push(place(
                    &track,
                    plane,
                    fans,
                    pn,
                    !double || plane != Plane::Center,
                ));
            }
        }
    }
    frames
}

pub(super) fn double_arc(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    arc(effect, fans, plane, pn, true)
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
