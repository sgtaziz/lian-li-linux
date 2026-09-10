use super::engine::{brightness, palette, place, scale, Color, Plane};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

const METEOR_TAILS: [[u8; 8]; 4] = [
    [32, 255, 0, 0, 0, 0, 0, 0],
    [16, 64, 128, 255, 0, 0, 0, 0],
    [8, 16, 32, 64, 128, 255, 0, 0],
    [6, 10, 16, 32, 64, 96, 168, 255],
];

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

pub(super) fn meteor_rainbow(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    const MASKS: [[u8; 8]; 4] = [
        [255, 255, 0, 0, 0, 0, 0, 0],
        [255, 255, 255, 255, 0, 0, 0, 0],
        [255, 255, 255, 255, 255, 255, 0, 0],
        [255; 8],
    ];
    let len = fans * 8;
    let bright = brightness(effect);
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    let mut phase = 0;
    let mut frames = Vec::with_capacity(len + 2 * fans - 1);
    for step in 0..len + 2 * fans - 1 {
        let rainbow = std::array::from_fn::<Color, 45, _>(|position| {
            let source = (phase + 44 - position) % 45;
            [
                RAINBOW_METEOR[0][source],
                RAINBOW_METEOR[1][source],
                RAINBOW_METEOR[2][source],
            ]
        });
        phase = (phase + 2) % 45;
        let mut mask_index = 0;
        let mut track = vec![[0; 3]; len];
        for (position, &rainbow_color) in rainbow.iter().take(len).enumerate() {
            let color = if position <= step && position + 2 * fans > step {
                let color = if MASKS[fans - 1][mask_index] == 0 {
                    [0; 3]
                } else {
                    rainbow_color
                };
                mask_index += 1;
                color
            } else {
                [0; 3]
            };
            let target = if reverse {
                len - position - 1
            } else {
                position
            };
            track[target] = scale(color, bright);
        }
        frames.push(place(&track, plane, fans));
    }
    frames
}

pub(super) fn colorful_meteor(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    const COLORS: [Color; 6] = [
        [255, 0, 0],
        [0, 255, 0],
        [0, 0, 255],
        [255, 255, 0],
        [0, 255, 255],
        [255, 0, 255],
    ];
    let len = fans * 8;
    let bright = brightness(effect);
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    let mut frames = Vec::with_capacity(len + 2 * fans - 1);
    for step in 0..len + 2 * fans - 1 {
        let color = COLORS[(step / 2) % COLORS.len()];
        let mut tail_index = 0;
        let mut track = vec![[0; 3]; len];
        for position in 0..len {
            let intensity = if position <= step && position + 2 * fans > step {
                let intensity = METEOR_TAILS[fans - 1][tail_index];
                tail_index += 1;
                intensity
            } else {
                0
            };
            let target = if reverse {
                len - position - 1
            } else {
                position
            };
            track[target] = scale(scale(color, u16::from(intensity)), bright);
        }
        frames.push(place(&track, plane, fans));
    }
    frames
}

pub(super) fn lottery(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    if plane == Plane::Center {
        let reverse = !matches!(effect.direction, RgbDirection::CounterClockwise);
        return (0..8)
            .map(|step| {
                let mut fan = [colors[1]; 8];
                let target = if reverse { 7 - step } else { step };
                fan[target] = colors[0];
                place(&fan.repeat(fans), plane, fans)
            })
            .collect();
    }

    let half = fans * 4;
    let mut frames = Vec::with_capacity(2 * (half + 2 * fans - 1));
    for pass in 0..2 {
        for step in 0..half + 2 * fans - 1 {
            let mut track = vec![[0; 3]; fans * 8];
            for position in 0..half {
                let color = if position <= step && position + 2 * fans > step {
                    colors[0]
                } else {
                    colors[1]
                };
                let first = if pass == 1 {
                    half - position - 1
                } else {
                    position
                };
                let second = if pass == 1 {
                    position
                } else {
                    half - position - 1
                };
                track[first] = color;
                track[half + second] = color;
            }
            frames.push(place(&track, plane, fans));
        }
    }
    frames
}

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
