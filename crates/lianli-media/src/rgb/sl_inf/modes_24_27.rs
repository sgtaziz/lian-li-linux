use super::engine::{brightness, palette, place, scale, Color, Plane};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

const HEARTBEAT: [u16; 96] = [
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 24, 32, 40, 48, 56, 64,
    72, 80, 88, 96, 104, 112, 120, 128, 136, 144, 152, 160, 168, 176, 184, 192, 200, 208, 216, 224,
    232, 240, 248, 255, 248, 240, 232, 224, 216, 208, 200, 192, 184, 176, 168, 160, 152, 144, 136,
    128, 120, 112, 104, 96, 88, 80, 72, 64, 56, 48, 40, 32, 24, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
];

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

pub(super) fn heart_beat(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    let color = palette(effect, plane)[0];
    let bright = brightness(effect);
    let len = fans * plane.track_len();
    (0usize..42)
        .map(|frame| {
            let source = frame * 2;
            let pulse = if plane == Plane::Center {
                source
            } else {
                source.saturating_sub(32)
            };
            let value = scale(scale(color, HEARTBEAT[pulse]), bright);
            place(&vec![value; len], plane, fans, pn, true)
        })
        .collect()
}

pub(super) fn heart_beat_runway(
    effect: &RgbEffect,
    fans: usize,
    plane: Plane,
    pn: u8,
) -> Vec<Vec<Color>> {
    let color = palette(effect, plane)[0];
    let bright = brightness(effect);
    let width = plane.track_len();
    let delay = 72 / fans;
    let total = [96, 144, 160, 168][fans - 1] + delay;
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    let offset = usize::from(plane != Plane::Center) * 64;
    let mut frames = Vec::with_capacity(total);
    for frame in 0..total {
        let phase = frame.saturating_sub(offset);
        let mut track = vec![[0; 3]; width * fans];
        for fan in 0..fans {
            let pulse = phase.saturating_sub(delay * fan);
            let intensity = if pulse < HEARTBEAT.len() {
                HEARTBEAT[pulse]
            } else {
                HEARTBEAT[0]
            };
            let value = scale(scale(color, intensity), bright);
            for position in 0..width {
                let logical = fan * width + position;
                let target = if reverse {
                    track.len() - logical - 1
                } else {
                    logical
                };
                track[target] = value;
            }
        }
        frames.push(place(&track, plane, fans, pn, plane != Plane::Center));
    }
    frames
}

pub(super) fn disco(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    const OUTER: [usize; 8] = [0, 0, 1, 1, 2, 2, 3, 3];
    const INNER: [usize; 10] = [0, 0, 1, 1, 1, 2, 2, 3, 3, 3];
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    let pattern = if plane == Plane::Inner {
        &INNER[..]
    } else {
        &OUTER[..]
    };
    let width = plane.track_len();
    let reverse = matches!(effect.direction, RgbDirection::Clockwise);
    let mut frames = Vec::with_capacity(width);
    for phase in 0..width {
        let mut segment = vec![[0; 3]; width];
        for position in 0..width {
            let target = if reverse {
                width - position - 1
            } else {
                position
            };
            segment[target] = colors[pattern[(phase + position) % width]];
        }
        let mut track = vec![[0; 3]; fans * width];
        track[..width].copy_from_slice(&segment);
        for fan in 1..fans {
            for position in 0..width {
                track[fan * 8 + position] = track[position];
            }
        }
        frames.push(place(&track, plane, fans, pn, true));
    }
    frames
}

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
