use super::engine::{brightness, palette, place, scale, Color, Plane};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

const TAILS: [[u16; 8]; 4] = [
    [32, 255, 0, 0, 0, 0, 0, 0],
    [16, 64, 128, 255, 0, 0, 0, 0],
    [8, 16, 32, 64, 128, 255, 0, 0],
    [6, 10, 16, 32, 64, 96, 168, 255],
];

pub(super) fn color_cycle(
    effect: &RgbEffect,
    fans: usize,
    plane: Plane,
    pn: u8,
) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    let len = fans * plane.track_len();
    let mut frames = Vec::with_capacity(4 * len);
    for color in 0..4 {
        let previous = if color == 0 { 3 } else { color - 1 };
        for step in 0..len {
            let mut track = vec![[0; 3]; len];
            for position in 0..len {
                let target = if reverse {
                    len - position - 1
                } else {
                    position
                };
                track[target] = colors[if position <= step { color } else { previous }];
            }
            frames.push(place(&track, plane, fans, pn, true));
        }
    }
    frames
}

pub(super) fn mop_up(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    let len = fans * plane.track_len();
    let mut frames = Vec::with_capacity(4 * (len + 2 * fans));
    for (color_index, color) in colors.into_iter().enumerate() {
        for step in 0..len + 2 * fans {
            let mut track = vec![[0; 3]; len];
            for position in 0..len {
                if position <= step && position + 2 * fans > step {
                    let target = if color_index == 1 || color_index == 3 {
                        len - position - 1
                    } else {
                        position
                    };
                    track[target] = color;
                }
            }
            frames.push(place(&track, plane, fans, pn, true));
        }
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

pub(super) fn lottery(effect: &RgbEffect, fans: usize, plane: Plane, pn: u8) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    if plane == Plane::Center {
        let clockwise = !matches!(effect.direction, RgbDirection::CounterClockwise);
        return (0..8)
            .map(|step| {
                let mut segment = [colors[1]; 8];
                segment[if clockwise { 7 - step } else { step }] = colors[0];
                place(&segment.repeat(fans), plane, fans, pn, true)
            })
            .collect();
    }
    let half = fans * plane.track_len() / 2;
    let mut frames = Vec::with_capacity(2 * (half + 2 * fans - 1));
    for pass in 0..2 {
        for step in 0..half + 2 * fans - 1 {
            let mut track = vec![[0; 3]; half * 2];
            for position in 0..half {
                let color = colors[usize::from(!(position <= step && position + 2 * fans > step))];
                let first = if pass == 0 {
                    position
                } else {
                    half - position - 1
                };
                let second = if pass == 0 {
                    half - position - 1
                } else {
                    position
                };
                track[first] = color;
                track[half + second] = color;
            }
            frames.push(place(&track, plane, fans, pn, true));
        }
    }
    frames
}

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
