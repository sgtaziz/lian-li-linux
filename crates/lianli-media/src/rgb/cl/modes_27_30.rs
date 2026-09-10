use super::engine::{brightness, palette, place, place_outer_banks, scale, Color, Plane};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

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

pub(super) fn electric_current(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    let half = fans * 4;
    let end = half + 2 * fans - 1;
    let mut frames = Vec::with_capacity(4 * (end + 9 + end.div_ceil(2)));
    for color in colors {
        let steps = std::iter::repeat_n(0, 10)
            .chain(1..end)
            .chain((0..end).step_by(2));
        for (frame_in_color, step) in steps.enumerate() {
            let reverse_halves = frame_in_color >= end + 9;
            let mut track = vec![[0; 3]; fans * 8];
            for position in 0..half {
                if position < step && position + 2 * fans > step {
                    let first = if reverse_halves {
                        half - position - 1
                    } else {
                        position
                    };
                    let second = if reverse_halves {
                        position
                    } else {
                        half - position - 1
                    };
                    track[first] = color;
                    track[half + second] = color;
                }
            }
            frames.push(place(&track, plane, fans));
        }
    }
    frames
}

pub(super) fn reflect(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    const TAIL: [u8; 8] = [255, 192, 168, 128, 96, 64, 32, 16];
    let colors = palette(effect, plane);
    let bright = brightness(effect);
    let half = fans * 4;
    let end = half + 2 * fans - 1;
    let mut frames = Vec::with_capacity(8 * end);
    for color in colors {
        for pass in 0..2 {
            for step in 0..end {
                let mut track = vec![[0; 3]; fans * 8];
                if pass == 0 {
                    for position in 0..half {
                        if position <= step && position + 2 * fans > step {
                            let color =
                                scale(scale(color, u16::from(TAIL[step - position])), bright);
                            track[position] = color;
                            track[fans * 8 - position - 1] = color;
                        }
                    }
                }
                frames.push(place(&track, plane, fans));
            }
        }
    }
    frames
}

pub(super) fn gradient_ribbon(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    const RAINBOW: [[u8; 48]; 3] = [
        [
            255, 240, 224, 208, 192, 176, 160, 144, 128, 112, 96, 80, 64, 48, 32, 16, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 16, 32, 48, 64, 80, 96, 112, 128, 144, 160, 176,
            192, 208, 224, 240,
        ],
        [
            0, 16, 32, 48, 64, 80, 96, 112, 128, 144, 160, 176, 192, 208, 224, 240, 255, 240, 224,
            208, 192, 176, 160, 144, 128, 112, 96, 80, 64, 48, 32, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0,
        ],
        [
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 16, 32, 48, 64, 80, 96, 112, 128,
            144, 160, 176, 192, 208, 224, 240, 255, 240, 224, 208, 192, 176, 160, 144, 128, 112,
            96, 80, 64, 48, 32, 16,
        ],
    ];
    let bright = brightness(effect);
    let reverse = !matches!(effect.direction, RgbDirection::CounterClockwise);
    (0..48)
        .map(|frame| {
            let make_track = |offset: usize| {
                let mut track = Vec::with_capacity(32);
                for group in 0..8 {
                    let source = (frame + offset + group) % 48;
                    let color = scale(
                        [RAINBOW[0][source], RAINBOW[1][source], RAINBOW[2][source]],
                        bright,
                    );
                    track.extend(std::iter::repeat_n(color, 4));
                }
                if reverse {
                    track.reverse();
                }
                track.truncate(fans * 8);
                track
            };
            match plane {
                Plane::Center => place(&make_track(8), plane, fans),
                Plane::Outer => place_outer_banks(&make_track(0), &make_track(16), fans),
            }
        })
        .collect()
}
