use super::engine::{brightness, center_order, mixed_color, palette, place, scale, Color, Plane};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn double_meteor(
    effect: &RgbEffect,
    fans: usize,
    plane: Plane,
    pn: u8,
) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    let width = plane.track_len();
    let half = width / 2;
    let mut work = vec![[0; 3]; 176];
    let mut frames = Vec::with_capacity(4 * half);
    for (color_index, color) in colors.into_iter().enumerate() {
        for step in 0..half {
            for position in 0..half {
                let target = if color_index.is_multiple_of(2) {
                    position
                } else {
                    half - position - 1
                };
                work[target] = if position == step { color } else { [0; 3] };
            }
            for position in 0..half {
                let target = if color_index.is_multiple_of(2) {
                    width - position - 1
                } else {
                    half + position
                };
                work[target] = if position == step { color } else { [0; 3] };
            }
            work[..width].rotate_right(1);
            for fan in 1..fans {
                let start = fan * 8;
                for position in 0..width {
                    work[start + position] = work[position];
                }
            }
            frames.push(place(&work[..fans * width], plane, fans, pn, true));
        }
    }
    frames
}

pub(super) fn meteor_contest(
    effect: &RgbEffect,
    fans: usize,
    plane: Plane,
    pn: u8,
) -> Vec<Vec<Color>> {
    const OUTER: [u16; 8] = [0, 0, 0, 255, 0, 0, 0, 254];
    const INNER: [u16; 10] = [0, 0, 0, 0, 255, 0, 0, 0, 0, 254];
    let colors = palette(effect, plane);
    let bright = brightness(effect);
    let width = plane.track_len();
    let pattern = if plane == Plane::Inner {
        &INNER[..]
    } else {
        &OUTER[..]
    };
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    let mut frames = Vec::with_capacity(width);
    for phase in 0..width {
        let mut segment = vec![[0; 3]; width];
        for position in 0..width {
            let intensity = pattern[(phase + position) % width];
            let color = colors[usize::from(intensity == 254)];
            let target = if reverse {
                width - position - 1
            } else {
                position
            };
            segment[target] = scale(scale(color, intensity), bright);
        }
        let track = (0..fans)
            .flat_map(|_| segment.iter().copied())
            .collect::<Vec<_>>();
        frames.push(place(&track, plane, fans, pn, true));
    }
    frames
}

pub(super) fn meteor_mix(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane);
    let mixed = mixed_color(colors[0], colors[1]);
    let bright = brightness(effect);
    let width = plane.track_len();
    let half = width / 2;
    let mut work = vec![[0; 3]; 176];
    let mut frames = Vec::with_capacity(2 * half);
    for pass in 0..2 {
        for step in 0..half {
            for position in 0..half {
                let first = if pass == 1 && plane != Plane::Center {
                    mixed
                } else {
                    colors[0]
                };
                let second = if pass == 1 && plane != Plane::Center {
                    mixed
                } else {
                    colors[1]
                };
                let first_target = if pass == 0 {
                    position
                } else {
                    half - position - 1
                };
                let second_target = if pass == 0 {
                    width - position - 1
                } else {
                    half + position
                };
                work[first_target] = if position == step {
                    scale(first, bright)
                } else {
                    [0; 3]
                };
                work[second_target] = if position == step {
                    scale(second, bright)
                } else {
                    [0; 3]
                };
            }
            work[..width].rotate_right(1);
            for fan in 1..fans {
                let start = fan * 8;
                for position in 0..width {
                    work[start + position] = work[position];
                }
            }
            frames.push(place(
                &work[..fans * width],
                plane,
                fans,
                pn,
                plane != Plane::Center,
            ));
        }
    }
    frames
}

pub(super) fn return_arc(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    arc(effect, fans, plane, pn, false)
}

pub(super) fn double_arc(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    arc(effect, fans, plane, pn, true)
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
