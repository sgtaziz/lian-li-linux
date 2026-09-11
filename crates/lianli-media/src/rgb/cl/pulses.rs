use super::engine::{
    all_palette, brightness, palette, place, place_both, scale, Color, Plane, CENTER_ORDER,
};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn warning(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    let mut frames = Vec::with_capacity(64);
    for color in colors {
        for pulse in 0..2 {
            let lit = match plane {
                Plane::Center => pulse == 0,
                Plane::Outer => pulse == 1,
            };
            let frame = place(
                &vec![if lit { color } else { [0; 3] }; fans * 8],
                plane,
                fans,
            );
            frames.extend(std::iter::repeat_n(frame, 8));
        }
    }
    frames
}

pub(super) fn voice(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    let len = fans * 8;
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    let band_lengths = [len, len * 2 / 3, len / 3];
    let mut frames = Vec::with_capacity(8 * band_lengths.iter().sum::<usize>());
    for color in colors {
        for band in band_lengths {
            for pass in 0..2 {
                for step in 0..band {
                    let mut track = vec![[0; 3]; len];
                    for position in 0..len {
                        let mut target = if reverse {
                            len - position - 1
                        } else {
                            position
                        };
                        if plane == Plane::Center {
                            target = CENTER_ORDER[target] - 1;
                        }
                        let dimmed =
                            (pass == 0 && position > step) || (pass == 1 && position > band - step);
                        track[target] = if dimmed {
                            color.map(|channel| channel / 8)
                        } else {
                            color
                        };
                    }
                    frames.push(place(&track, plane, fans));
                }
            }
        }
    }
    frames
}

pub(super) fn heart_beat(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let color = palette(effect, plane)[0];
    let bright = brightness(effect);
    (0..128)
        .map(|frame| {
            let pulse_index = match plane {
                Plane::Center if frame < 96 => frame,
                Plane::Center => 0,
                Plane::Outer if frame >= 32 => frame - 32,
                Plane::Outer => 0,
            };
            let color = scale(scale(color, u16::from(HEARTBEAT[pulse_index])), bright);
            place(&vec![color; fans * 8], plane, fans)
        })
        .collect()
}

const HEARTBEAT: [u8; 96] = [
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 24, 32, 40, 48, 56, 64,
    72, 80, 88, 96, 104, 112, 120, 128, 136, 144, 152, 160, 168, 176, 184, 192, 200, 208, 216, 224,
    232, 240, 248, 255, 248, 240, 232, 224, 216, 208, 200, 192, 184, 176, 168, 160, 152, 144, 136,
    128, 120, 112, 104, 96, 88, 80, 72, 64, 56, 48, 40, 32, 24, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
];

pub(super) fn heart_beat_runway(effect: &RgbEffect, fans: usize, _plane: Plane) -> Vec<Vec<Color>> {
    const FRAME_BASE: [usize; 4] = [96, 144, 160, 168];
    let color = palette(effect, Plane::Outer)[0];
    let bright = brightness(effect);
    let delay = 72 / fans;
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    (0..FRAME_BASE[fans - 1] + delay)
        .map(|frame| {
            let make_track = |phase_delay: usize| {
                let phase = frame.saturating_sub(phase_delay);
                let mut track = vec![[0; 3]; fans * 8];
                for fan in 0..fans {
                    let pulse_index = phase.saturating_sub(fan * delay);
                    let pulse_index = if pulse_index >= 96 { 0 } else { pulse_index };
                    let pixel = scale(scale(color, u16::from(HEARTBEAT[pulse_index])), bright);
                    track[fan * 8..(fan + 1) * 8].fill(pixel);
                }
                if reverse {
                    track.reverse();
                }
                track
            };
            let center = make_track(0);
            let outer = make_track(64);
            place_both(&center, &outer, fans)
        })
        .collect()
}

pub(super) fn disco(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    const COLOR_INDEX: [usize; 8] = [0, 0, 1, 1, 2, 2, 3, 3];
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    let reverse = !matches!(effect.direction, RgbDirection::CounterClockwise);
    (0..8)
        .map(|frame| {
            let mut fan = [[0; 3]; 8];
            for position in 0..8 {
                let target = if reverse { 7 - position } else { position };
                fan[target] = colors[COLOR_INDEX[(frame + position) % 8]];
            }
            place(&fan.repeat(fans), plane, fans)
        })
        .collect()
}

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
