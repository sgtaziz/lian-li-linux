use super::engine::{brightness, place, scale, Color, Plane};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn meteor_rainbow(
    effect: &RgbEffect,
    fans: usize,
    plane: Plane,
    pn: u8,
) -> Vec<Vec<Color>> {
    let visible_tail = [2, 4, 6, 8][fans - 1];
    let len = fans * plane.track_len();
    let bright = brightness(effect);
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    let mut phase = 0;
    let mut frames = Vec::with_capacity(len + 2 * fans - 1);
    for step in 0..len + 2 * fans - 1 {
        let rainbow: [Color; 45] = std::array::from_fn(|position| {
            let source = (phase + 44 - position) % 45;
            [
                RAINBOW_METEOR[0][source],
                RAINBOW_METEOR[1][source],
                RAINBOW_METEOR[2][source],
            ]
        });
        phase = (phase + 2) % 45;
        let mut tail = 0;
        let mut track = vec![[0; 3]; len];
        for (position, &rainbow_color) in rainbow.iter().take(len).enumerate() {
            let mut color = [0; 3];
            if position <= step && position + 2 * fans > step {
                if tail < visible_tail {
                    color = rainbow_color;
                }
                tail += 1;
            }
            let target = if reverse {
                len - position - 1
            } else {
                position
            };
            track[target] = scale(color, bright);
        }
        frames.push(place(&track, plane, fans, pn, true));
    }
    frames
}

const RAINBOW_METEOR: [[u8; 45]; 3] = [
    [
        255, 128, 64, 0, 0, 0, 0, 64, 128, 255, 128, 64, 0, 0, 0, 0, 64, 128, 255, 128, 64, 0, 0,
        0, 0, 64, 128, 255, 128, 64, 0, 0, 0, 0, 64, 128, 255, 128, 64, 0, 0, 0, 0, 64, 128,
    ],
    [
        0, 64, 128, 255, 128, 64, 0, 0, 0, 0, 64, 128, 255, 128, 64, 0, 0, 0, 0, 64, 128, 255, 128,
        64, 0, 0, 0, 0, 64, 128, 255, 128, 64, 0, 0, 0, 0, 64, 128, 255, 128, 64, 0, 0, 0,
    ],
    [
        0, 0, 0, 0, 64, 128, 255, 128, 64, 0, 0, 0, 0, 64, 128, 255, 128, 64, 0, 0, 0, 0, 64, 128,
        255, 128, 64, 0, 0, 0, 0, 64, 128, 255, 128, 64, 0, 0, 0, 0, 64, 128, 255, 128, 64,
    ],
];

pub(super) fn colorful_meteor(
    effect: &RgbEffect,
    fans: usize,
    plane: Plane,
    pn: u8,
) -> Vec<Vec<Color>> {
    const COLORS: [Color; 6] = [
        [255, 0, 0],
        [0, 255, 0],
        [0, 0, 255],
        [255, 255, 0],
        [0, 255, 255],
        [255, 0, 255],
    ];
    let len = fans * plane.track_len();
    let bright = brightness(effect);
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    let mut color = 0;
    let mut hold = 0;
    let mut frames = Vec::with_capacity(len + 2 * fans - 1);
    for step in 0..len + 2 * fans - 1 {
        let mut tail = 0;
        let mut track = vec![[0; 3]; len];
        for position in 0..len {
            if position <= step && position + 2 * fans > step {
                let target = if reverse {
                    len - position - 1
                } else {
                    position
                };
                track[target] = scale(scale(COLORS[color], TAILS[fans - 1][tail]), bright);
                tail += 1;
            }
        }
        hold += 1;
        if hold == 2 {
            hold = 0;
            color = (color + 1) % COLORS.len();
        }
        frames.push(place(&track, plane, fans, pn, true));
    }
    frames
}

const TAILS: [[u16; 8]; 4] = [
    [32, 255, 0, 0, 0, 0, 0, 0],
    [16, 64, 128, 255, 0, 0, 0, 0],
    [8, 16, 32, 64, 128, 255, 0, 0],
    [6, 10, 16, 32, 64, 96, 168, 255],
];
