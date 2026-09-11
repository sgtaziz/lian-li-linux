use super::engine::{
    brightness, center_order, drumming_palette, palette, place, scale, Color, Plane,
};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn warning(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    let mut frames = Vec::with_capacity(64);
    for color in colors {
        for pulse in 0..2 {
            let lit = (plane == Plane::Center) == (pulse == 0);
            let track = vec![if lit { color } else { [0; 3] }; fans * plane.track_len()];
            let frame = place(&track, plane, fans, pn, true);
            frames.extend(std::iter::repeat_n(frame, 8));
        }
    }
    frames
}

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

const HEARTBEAT: [u16; 96] = [
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 24, 32, 40, 48, 56, 64,
    72, 80, 88, 96, 104, 112, 120, 128, 136, 144, 152, 160, 168, 176, 184, 192, 200, 208, 216, 224,
    232, 240, 248, 255, 248, 240, 232, 224, 216, 208, 200, 192, 184, 176, 168, 160, 152, 144, 136,
    128, 120, 112, 104, 96, 88, 80, 72, 64, 56, 48, 40, 32, 24, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
];

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
