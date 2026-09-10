use super::engine::{brightness, center_order, mixed_color, palette, place, scale, Color, Plane};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn voice(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    let len = fans * plane.track_len();
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    let bands = [len, len * 2 / 3, len / 3];
    let mut source = Vec::with_capacity(8 * bands.iter().sum::<usize>());
    for color in colors {
        for band in bands {
            for pass in 0..2 {
                for step in 0..band {
                    let mut track = vec![[0; 3]; len];
                    for position in 0..len {
                        let logical = if reverse {
                            len - position - 1
                        } else {
                            position
                        };
                        let target = if plane == Plane::Center {
                            center_order(logical, pn)
                        } else {
                            logical
                        };
                        let dim =
                            (pass == 0 && position > step) || (pass == 1 && position > band - step);
                        track[target] = if dim {
                            color.map(|channel| channel / 16)
                        } else {
                            color
                        };
                    }
                    source.push(place(&track, plane, fans, pn, plane != Plane::Center));
                }
            }
        }
    }
    source.into_iter().step_by(2).collect()
}

pub(super) fn mixing(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane);
    let mixed = mixed_color(colors[0], colors[1]);
    let bright = brightness(effect);
    let half = fans * plane.track_len() / 2;
    let end = half + 2 * fans - 1;
    let mut frames = Vec::with_capacity(end + 2 * end.div_ceil(2));
    for phase in 0..3 {
        let increment = if phase == 0 { 1 } else { 2 };
        for step in (0..end).step_by(increment) {
            let mut track = vec![[0; 3]; half * 2];
            for position in 0..half {
                let settled =
                    (phase == 1 && position < step) || (phase == 2 && position + 2 * fans > step);
                let moving = position < step && position + 2 * fans > step;
                let first = if settled {
                    mixed
                } else if moving {
                    colors[0]
                } else {
                    [0; 3]
                };
                let second = if settled {
                    mixed
                } else if moving {
                    colors[1]
                } else {
                    [0; 3]
                };
                let first_target = if phase == 0 {
                    position
                } else {
                    half - position - 1
                };
                let second_target = if phase == 0 {
                    half - position - 1
                } else {
                    position
                };
                track[first_target] = scale(first, bright);
                track[half + second_target] = scale(second, bright);
            }
            frames.push(place(&track, plane, fans, pn, plane != Plane::Center));
        }
    }
    frames
}

pub(super) fn tide(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    let half = fans * plane.track_len() / 2;
    let mut frames = Vec::with_capacity(4 * (half + 2 * fans - 1));
    for color in 0..4 {
        let previous = if color == 0 { 3 } else { color - 1 };
        for step in 0..half + 2 * fans - 1 {
            let mut track = vec![[0; 3]; half * 2];
            for position in 0..half {
                let value = colors[if position <= step { color } else { previous }];
                let first = if plane == Plane::Center {
                    center_order(position, pn)
                } else {
                    position
                };
                let logical_second = half * 2 - position - 1;
                let second = if plane == Plane::Center {
                    center_order(logical_second, pn)
                } else {
                    logical_second
                };
                track[first] = value;
                track[second] = value;
            }
            frames.push(place(&track, plane, fans, pn, plane != Plane::Center));
        }
    }
    frames
}

pub(super) fn scan(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    let color = scale(palette(effect, plane)[0], brightness(effect));
    let len = fans * plane.track_len();
    let mut frames = Vec::with_capacity(2 * (len + 2 * fans));
    for pass in 0..2 {
        for step in 0..len + 2 * fans {
            let mut track = vec![[0; 3]; len];
            for position in 0..len {
                let logical = if pass == 0 {
                    position
                } else {
                    len - position - 1
                };
                let target = if plane == Plane::Center {
                    center_order(logical, pn)
                } else {
                    logical
                };
                if position <= step && position + fans > step {
                    track[target] = color;
                }
            }
            frames.push(place(&track, plane, fans, pn, plane != Plane::Center));
        }
    }
    frames
}
