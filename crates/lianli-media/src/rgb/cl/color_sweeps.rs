use super::engine::{brightness, mixed_color, palette, place, scale, Color, Plane, CENTER_ORDER};
use lianli_shared::rgb::RgbEffect;

pub(super) fn mixing(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane);
    let mixed = mixed_color(colors[0], colors[1]);
    let bright = brightness(effect);
    let half = fans * 4;
    let end = half + 2 * fans - 1;
    let mut frames = Vec::with_capacity(end + 2 * end.div_ceil(2));
    for phase in 0..3 {
        let increment = if phase == 0 { 1 } else { 2 };
        for step in (0..end).step_by(increment) {
            let mut track = vec![[0; 3]; fans * 8];
            for position in 0..half {
                let first_color = if (phase == 1 && position < step)
                    || (phase == 2 && position + 2 * fans > step)
                {
                    mixed
                } else if position < step && position + 2 * fans > step {
                    colors[0]
                } else {
                    [0; 3]
                };
                let second_color = if (phase == 1 && position < step)
                    || (phase == 2 && position + 2 * fans > step)
                {
                    mixed
                } else if position < step && position + 2 * fans > step {
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
                track[first_target] = scale(first_color, bright);
                track[half + second_target] = scale(second_color, bright);
            }
            frames.push(place(&track, plane, fans));
        }
    }
    frames
}

pub(super) fn tide(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    let half = fans * 4;
    let mut frames = Vec::with_capacity(4 * (half + 2 * fans - 1));
    for color_index in 0..4 {
        let previous = if color_index == 0 { 3 } else { color_index - 1 };
        for step in 0..half + 2 * fans - 1 {
            let mut track = vec![[0; 3]; fans * 8];
            for position in 0..half {
                let color = if position <= step {
                    colors[color_index]
                } else {
                    colors[previous]
                };
                track[position] = color;
                let mut target = half - position - 1 + half;
                if plane == Plane::Center {
                    target = CENTER_ORDER[target] - 1;
                }
                track[target] = color;
            }
            frames.push(place(&track, plane, fans));
        }
    }
    frames
}

pub(super) fn scan(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let color = scale(palette(effect, plane)[0], brightness(effect));
    let len = fans * 8;
    let mut frames = Vec::with_capacity(2 * (len + 2 * fans));
    for pass in 0..2 {
        for step in 0..len + 2 * fans {
            let mut track = vec![[0; 3]; len];
            for position in 0..len {
                let mut target = if pass == 1 {
                    len - position - 1
                } else {
                    position
                };
                if plane == Plane::Center {
                    target = CENTER_ORDER[target] - 1;
                }
                if position <= step && position + fans > step {
                    track[target] = color;
                }
            }
            frames.push(place(&track, plane, fans));
        }
    }
    frames
}

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
