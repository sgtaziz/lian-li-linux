use super::engine::{brightness, palette, place, place_both, scale, Color, Plane};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

const HEARTBEAT: [u8; 96] = [
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 24, 32, 40, 48, 56, 64,
    72, 80, 88, 96, 104, 112, 120, 128, 136, 144, 152, 160, 168, 176, 184, 192, 200, 208, 216, 224,
    232, 240, 248, 255, 248, 240, 232, 224, 216, 208, 200, 192, 184, 176, 168, 160, 152, 144, 136,
    128, 120, 112, 104, 96, 88, 80, 72, 64, 56, 48, 40, 32, 24, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
];

const DOOR_1: [u32; 8] = [0x66, 0x66, 0xff, 0xff, 0xff, 0x66, 0x66, 0];
const DOOR_2: [u32; 16] = [
    0x6006, 0x6006, 0xf00f, 0xf00f, 0xf99f, 0xf99f, 0xffff, 0xffff, 0xffff, 0xffff, 0xf99f, 0xf99f,
    0xf00f, 0xf00f, 0x6006, 0x6006,
];
const DOOR_3: [u32; 24] = [
    0x600006, 0x600006, 0xf0000f, 0xf0000f, 0xf9009f, 0xf9009f, 0xff00ff, 0xff00ff, 0xff66ff,
    0xff66ff, 0xffffff, 0xffffff, 0xffffff, 0xffffff, 0xff66ff, 0xff66ff, 0xff00ff, 0xff00ff,
    0xf9009f, 0xf9009f, 0xf0000f, 0xf0000f, 0x600006, 0x600006,
];
const DOOR_4: [u32; 32] = [
    0x60000006, 0x60000006, 0xf000000f, 0xf000000f, 0xf900009f, 0xf900009f, 0xff0000ff, 0xff0000ff,
    0xff6006ff, 0xff6006ff, 0xfff00fff, 0xfff00fff, 0xfff99fff, 0xfff99fff, 0xffffffff, 0xffffffff,
    0xffffffff, 0xffffffff, 0xfff99fff, 0xfff99fff, 0xfff00fff, 0xfff00fff, 0xff6006ff, 0xff6006ff,
    0xff0000ff, 0xff0000ff, 0xf900009f, 0xf900009f, 0xf000000f, 0xf000000f, 0x60000006, 0x60000006,
];

pub(super) fn door(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    let half = fans * 4;
    let masks: &[u32] = match fans {
        1 => &DOOR_1,
        2 => &DOOR_2,
        3 => &DOOR_3,
        4 => &DOOR_4,
        _ => unreachable!(),
    };
    let mut frames = Vec::with_capacity(fans * 32);
    for color in colors {
        for pass in 0..2 {
            for step in 0..half {
                let track = match plane {
                    Plane::Center => (0..fans * 8)
                        .map(|position| {
                            if masks[pass * half + step] & (1 << position) != 0 {
                                color
                            } else {
                                [0; 3]
                            }
                        })
                        .collect::<Vec<_>>(),
                    Plane::Outer => {
                        let mut track = vec![[0; 3]; fans * 8];
                        for position in 0..half {
                            let lit = (pass == 0 && position < step)
                                || (pass == 1 && position < half - step);
                            if lit {
                                track[position] = color;
                                track[fans * 8 - position - 1] = color;
                            }
                        }
                        track
                    }
                };
                frames.push(place(&track, plane, fans));
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
