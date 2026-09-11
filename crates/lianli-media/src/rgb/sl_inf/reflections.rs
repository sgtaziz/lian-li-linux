use super::engine::{brightness, center_order, palette, place, scale, Color, Plane};
use lianli_shared::rgb::RgbEffect;

pub(super) fn electric_current(
    effect: &RgbEffect,
    fans: usize,
    plane: Plane,
    pn: u8,
) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    let half = fans * plane.track_len() / 2;
    let span = half + 2 * fans - 1;
    let mut frames = Vec::with_capacity(4 * (span + 9 + span.div_ceil(2)));
    for color in colors {
        for pass in 0..2 {
            let steps: Vec<_> = if pass == 0 {
                std::iter::repeat_n(0, 10).chain(1..span).collect()
            } else {
                (0..span).step_by(2).collect()
            };
            for step in steps {
                let mut track = vec![[0; 3]; half * 2];
                if plane == Plane::Center {
                    let row = pass * span + step;
                    let pattern = electric_pattern(fans, pn);
                    let track_len = track.len();
                    for (position, target) in track.iter_mut().enumerate() {
                        let bit = row * track_len + position;
                        if pattern[bit / 8] & (1 << (bit % 8)) != 0 {
                            *target = color;
                        }
                    }
                } else {
                    for position in 0..half {
                        if position < step && position + 2 * fans > step {
                            let first = if pass == 0 {
                                position
                            } else {
                                half - position - 1
                            };
                            let second = if pass == 0 {
                                half * 2 - position - 1
                            } else {
                                half + position
                            };
                            track[first] = color;
                            track[second] = color;
                        }
                    }
                }
                frames.push(place(&track, plane, fans, pn, plane != Plane::Center));
            }
        }
    }
    frames
}

fn electric_pattern(fans: usize, pn: u8) -> &'static [u8] {
    use super::electric_patterns::{FOUR_LEFT, FOUR_RIGHT, ONE, THREE_LEFT, THREE_RIGHT, TWO};
    match (fans, pn) {
        (1, _) => ONE,
        (2, _) => TWO,
        (3, 0) => THREE_LEFT,
        (3, _) => THREE_RIGHT,
        (4, 0) => FOUR_LEFT,
        (4, _) => FOUR_RIGHT,
        _ => unreachable!(),
    }
}

pub(super) fn reflect(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    const TAIL: [u16; 8] = [255, 192, 168, 128, 96, 64, 32, 16];
    let colors = palette(effect, plane);
    let bright = brightness(effect);
    let len = fans * plane.track_len();
    let half = len / 2;
    let span = half + 2 * fans - 1;
    let mut frames = Vec::with_capacity(4 * (span + span.div_ceil(2)));
    for color in colors {
        for pass in 0..2 {
            let increment = if pass == 0 { 1 } else { 2 };
            for step in (0..span).step_by(increment) {
                let mut track = vec![[0; 3]; len];
                if pass == 0 {
                    for position in 0..half {
                        if position <= step && position + 2 * fans > step {
                            let value = scale(scale(color, TAIL[step - position]), bright);
                            let first = if plane == Plane::Center {
                                center_order(position, pn)
                            } else {
                                position
                            };
                            let second_logical = len - position - 1;
                            let second = if plane == Plane::Center {
                                center_order(second_logical, pn)
                            } else {
                                second_logical
                            };
                            track[first] = value;
                            track[second] = value;
                        }
                    }
                }
                frames.push(place(&track, plane, fans, pn, plane != Plane::Center));
            }
        }
    }
    frames
}
