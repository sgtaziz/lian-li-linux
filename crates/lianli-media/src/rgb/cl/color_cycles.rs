use super::engine::{brightness, place, place_outer_banks, scale, Color, Plane};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

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

pub(super) fn candy_box(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    const OUTER: [[Color; 4]; 2] = [
        [[127, 0, 255], [0, 255, 0], [127, 0, 255], [0, 255, 255]],
        [[255, 95, 0], [255, 0, 255], [207, 255, 0], [0, 255, 0]],
    ];
    const CENTER: [[Color; 4]; 2] = [
        [[255, 0, 47], [0, 0, 255], [255, 0, 255], [0, 255, 0]],
        [[255, 255, 0], [0, 255, 0], [255, 31, 0], [255, 0, 255]],
    ];
    let bright = brightness(effect);
    let mut frames = Vec::with_capacity(340);
    for phase in 0..2 {
        for frame in 0..170 {
            let intensity = if frame > 85 {
                (170 - frame) * 3
            } else {
                frame * 3
            } as u16;
            let colors = if plane == Plane::Center {
                CENTER[phase]
            } else {
                OUTER[phase]
            };
            let mut track = Vec::with_capacity(fans * 8);
            for color in colors.into_iter().take(fans) {
                let color = scale(scale(color, intensity), bright);
                track.extend(std::iter::repeat_n(color, 8));
            }
            frames.push(place(&track, plane, fans));
        }
    }
    frames
}
