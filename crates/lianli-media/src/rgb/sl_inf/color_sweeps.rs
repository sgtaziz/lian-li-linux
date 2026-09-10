use super::engine::{brightness, center_order, mixed_color, palette, place, scale, Color, Plane};
use lianli_shared::rgb::RgbEffect;

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

pub(super) fn door(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    let outer_half = 4 * fans;
    let inner_half = 5 * fans;
    let mut frames = Vec::with_capacity(32 * fans);
    for color in colors {
        for pass in 0..2 {
            let mut inner_step = 0;
            for step in 0..outer_half {
                let track = match plane {
                    Plane::Center => {
                        let mut track = vec![[0; 3]; outer_half * 2];
                        let pattern = door_pattern(fans, pn);
                        let row = pass * outer_half + step;
                        let track_len = track.len();
                        for (position, target) in track.iter_mut().enumerate() {
                            let bit = row * track_len + position;
                            if pattern[bit / 8] & (1 << (bit % 8)) != 0 {
                                *target = color;
                            }
                        }
                        track
                    }
                    Plane::Outer => symmetric_door_track(outer_half, step, pass, color),
                    Plane::Inner => {
                        if step % 4 == 2 {
                            inner_step += 1;
                        }
                        let track = symmetric_door_track(inner_half, inner_step, pass, color);
                        inner_step += 1;
                        track
                    }
                };
                frames.push(place(&track, plane, fans, pn, plane != Plane::Center));
            }
        }
    }
    frames
}

fn door_pattern(fans: usize, pn: u8) -> &'static [u8] {
    use super::effect_patterns::{DOOR_3, DOOR_4, DOOR_5, DOOR_6, DOOR_7, DOOR_8, DOOR_9};
    match (fans, pn) {
        (1, _) => DOOR_3,
        (2, 0) => DOOR_4,
        (2, _) => DOOR_5,
        (3, 0) => DOOR_6,
        (3, _) => DOOR_7,
        (4, 0) => DOOR_8,
        (4, _) => DOOR_9,
        _ => unreachable!(),
    }
}

fn symmetric_door_track(half: usize, step: usize, pass: usize, color: Color) -> Vec<Color> {
    let mut track = vec![[0; 3]; half * 2];
    for position in 0..half {
        let lit = if pass == 0 {
            position < step
        } else {
            position <= half.saturating_sub(step + 1)
        };
        if lit {
            track[position] = color;
            track[half * 2 - position - 1] = color;
        }
    }
    track
}
