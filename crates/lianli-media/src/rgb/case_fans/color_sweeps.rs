use super::{
    palette::{scale, Color, Frame},
    Layout,
};
use lianli_shared::rgb::RgbScope;

pub(crate) fn warning(layout: Layout, palettes: &[[Color; 4]; 4], brightness: u8) -> Vec<Frame> {
    let mut frames = Vec::with_capacity(64);
    for colors in (0..4).map(|index| [palettes[0][index], palettes[1][index]]) {
        for (side, &color) in colors.iter().enumerate() {
            for _ in 0..8 {
                let mut output = layout.frame();
                let active = if side == 0 {
                    0..layout.front_leds
                } else {
                    layout.front_leds..layout.led_count()
                };
                for led in active {
                    output[led] = scale(color, brightness);
                }
                frames.push(output);
            }
        }
    }
    frames
}

pub(crate) fn mixing(
    layout: Layout,
    scope: RgbScope,
    palettes: &[[Color; 4]; 4],
    brightness: u8,
) -> Vec<Frame> {
    let side = usize::from(scope == RgbScope::Rear);
    let half = if side == 0 { layout.front_leds / 2 } else { 8 };
    let start = if side == 0 { 0 } else { layout.front_leds };
    let head = if side == 0 { 16 } else { 2 };
    let colors = palettes[side];
    let mut mixed =
        std::array::from_fn(|channel| colors[0][channel].saturating_add(colors[1][channel]));
    if mixed.iter().map(|&channel| u16::from(channel)).sum::<u16>() > 536 {
        mixed = scale(mixed, 178);
    }
    let mut frames = Vec::new();
    for phase in 0..3 {
        let mut step = 0;
        while step < half + head - 1 {
            let mut output = layout.frame();
            for position in 0..half {
                let completed =
                    (phase == 1 && position < step) || (phase == 2 && position + head > step);
                let moving = position < step && position + head > step;
                let left = if completed {
                    mixed
                } else if moving {
                    colors[0]
                } else {
                    [0; 3]
                };
                let right = if completed {
                    mixed
                } else if moving {
                    colors[1]
                } else {
                    [0; 3]
                };
                let left_destination = if phase == 0 {
                    position
                } else {
                    half - position - 1
                };
                let right_destination = if phase == 0 {
                    half - position - 1
                } else {
                    position
                };
                output[start + left_destination] = scale(left, brightness);
                output[start + half + right_destination] = scale(right, brightness);
            }
            frames.push(output);
            step += if phase == 0 { 1 } else { 2 };
        }
    }
    frames
}

pub(crate) fn tide(
    layout: Layout,
    scope: RgbScope,
    palettes: &[[Color; 4]; 4],
    brightness: u8,
) -> Vec<Frame> {
    let side = usize::from(scope == RgbScope::Rear);
    let half = if side == 0 { layout.front_leds / 2 } else { 8 };
    let start = if side == 0 { 0 } else { layout.front_leds };
    let head = if side == 0 { 16 } else { 2 };
    let colors = palettes[side];
    let mut frames = Vec::with_capacity(4 * (half + head - 1));
    for (color_index, &color) in colors.iter().enumerate() {
        let previous = if color_index == 0 { 3 } else { color_index - 1 };
        for step in 0..half + head - 1 {
            let mut output = layout.frame();
            for position in 0..half {
                let pixel = scale(
                    if position <= step {
                        color
                    } else {
                        colors[previous]
                    },
                    brightness,
                );
                output[start + position] = pixel;
                output[start + half * 2 - position - 1] = pixel;
            }
            frames.push(output);
        }
    }
    frames
}

pub(crate) fn double_meteor(
    layout: Layout,
    scope: RgbScope,
    palettes: &[[Color; 4]; 4],
    brightness: u8,
) -> Vec<Frame> {
    let side = usize::from(scope == RgbScope::Rear);
    let half = if side == 0 { layout.front_leds / 2 } else { 8 };
    let start = if side == 0 { 0 } else { layout.front_leds };
    let head = if side == 0 { 8 } else { 1 };
    let colors = palettes[side];
    let mut frames = Vec::with_capacity(4 * half);
    for (color_index, &color) in colors.iter().enumerate() {
        for step in 0..half {
            let mut output = layout.frame();
            for position in 0..half {
                let pixel = if position <= step && position + head > step {
                    scale(color, brightness)
                } else {
                    [0; 3]
                };
                let left = if color_index == 1 || color_index == 3 {
                    half - position - 1
                } else {
                    position
                };
                let right = if color_index == 1 || color_index == 3 {
                    position
                } else {
                    half - position - 1
                };
                output[start + left] = pixel;
                output[start + half + right] = pixel;
            }
            frames.push(output);
        }
    }
    frames
}
