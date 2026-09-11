use super::engine::{frame, scale, Color, Frame, Geometry};

const RAINBOW_22: [[u8; 22]; 3] = [
    [
        255, 224, 192, 128, 96, 64, 32, 0, 0, 0, 0, 0, 0, 0, 0, 32, 64, 128, 160, 192, 224, 240,
    ],
    [
        0, 32, 64, 128, 160, 192, 224, 255, 224, 192, 128, 96, 64, 32, 0, 0, 0, 0, 0, 0, 0, 0,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 32, 64, 128, 160, 192, 224, 255, 224, 192, 128, 96, 64, 32, 16,
    ],
];
const RAINBOW_29: [[u8; 29]; 3] = [
    [
        255, 240, 224, 192, 160, 128, 96, 64, 32, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 16, 32, 64,
        96, 128, 160, 192, 224,
    ],
    [
        0, 16, 32, 64, 96, 128, 160, 192, 224, 240, 255, 240, 224, 192, 160, 128, 96, 64, 32, 16,
        0, 0, 0, 0, 0, 0, 0, 0, 0,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 16, 32, 64, 96, 128, 160, 192, 224, 240, 255, 240, 224,
        192, 160, 128, 96, 64, 32,
    ],
];

pub(super) fn rainbow(geometry: Geometry, brightness: u8, reverse: bool) -> Vec<Frame> {
    let width = geometry.lane_length;
    (0..width)
        .map(|shift| {
            let mut output = frame(geometry);
            for lane in 0..geometry.lanes {
                for position in 0..width {
                    let source = (shift + position) % width;
                    let pixel = if width == 22 {
                        [
                            RAINBOW_22[0][source],
                            RAINBOW_22[1][source],
                            RAINBOW_22[2][source],
                        ]
                    } else {
                        [
                            RAINBOW_29[0][source],
                            RAINBOW_29[1][source],
                            RAINBOW_29[2][source],
                        ]
                    };
                    let destination = if reverse {
                        position
                    } else {
                        width - position - 1
                    };
                    output[lane * width + destination] = scale(pixel, brightness);
                }
            }
            output
        })
        .collect()
}

pub(super) fn wave(
    geometry: Geometry,
    colors: &[Color; 6],
    brightness: u8,
    reverse: bool,
) -> Vec<Frame> {
    const LEVELS_22: [u8; 22] = [
        16, 32, 128, 255, 128, 32, 16, 0, 0, 0, 16, 32, 128, 255, 128, 32, 16, 0, 0, 0, 0, 0,
    ];
    const LEVELS_29: [u8; 29] = [
        16, 32, 64, 128, 255, 128, 64, 32, 16, 0, 0, 0, 0, 0, 16, 32, 64, 128, 255, 128, 64, 32,
        16, 0, 0, 0, 0, 0, 0,
    ];
    let width = geometry.lane_length;
    let total = width * 12;
    let mut color_index = 0;
    let mut color_ticks = 0;
    (0..total)
        .map(|shift| {
            color_ticks += 1;
            if color_ticks >= width * 2 {
                color_ticks = 0;
                color_index = (color_index + 1) % 6;
            }
            let mut output = frame(geometry);
            for lane in 0..geometry.lanes {
                for position in 0..width {
                    let source = (shift + position) % width;
                    let level = if width == 22 {
                        LEVELS_22[source]
                    } else {
                        LEVELS_29[source]
                    };
                    let pixel = scale(scale(colors[color_index], level), brightness);
                    let destination = if reverse {
                        position
                    } else {
                        width - position - 1
                    };
                    output[lane * width + destination] = pixel;
                }
            }
            output
        })
        .collect()
}

pub(super) fn solid(geometry: Geometry, color: Color, brightness: u8, frames: usize) -> Vec<Frame> {
    let mut output = vec![frame(geometry); frames];
    output[0].fill(scale(color, brightness));
    output
}

pub(super) fn breathing(
    geometry: Geometry,
    color: Color,
    brightness: u8,
    regional: bool,
) -> Vec<Frame> {
    let (count, peak, multiplier) = if regional && geometry.lane_length == 22 {
        (132, 63, 4)
    } else if regional {
        (174, 85, 3)
    } else {
        (170, 85, 3)
    };
    let mut level = 0u8;
    (0..count)
        .map(|index| {
            let intensity = level * multiplier;
            let output = vec![scale(scale(color, intensity), brightness); geometry.led_count];
            if index < count / 2 {
                level = level.saturating_add(1).min(peak);
            } else {
                level = level.saturating_sub(1);
            }
            output
        })
        .collect()
}

pub(super) fn morph(geometry: Geometry, brightness: u8, regional: bool) -> Vec<Frame> {
    let (count, step, mut red) = if regional && geometry.lane_length == 22 {
        (264, 3u8, 255u8)
    } else if regional {
        (348, 2u8, 232u8)
    } else {
        (255, 3u8, 255u8)
    };
    let phase = count / 3;
    let mut green = 0u8;
    let mut blue = 0u8;
    let mut frames = Vec::with_capacity(count);
    for index in 0..count {
        frames.push(vec![
            scale([red, green, blue], brightness);
            geometry.led_count
        ]);
        if index < phase {
            red = red.saturating_sub(step);
            green = green.saturating_add(step);
            blue = 0;
        } else if index < phase * 2 {
            red = 0;
            green = green.saturating_sub(step);
            blue = blue.saturating_add(step);
        } else {
            red = red.saturating_add(step);
            green = 0;
            blue = blue.saturating_sub(step);
        }
    }
    if regional {
        frames
    } else {
        frames.into_iter().step_by(2).take(count / 2).collect()
    }
}

pub(super) fn paint(geometry: Geometry, colors: &[Color; 6], brightness: u8) -> Vec<Frame> {
    let width = geometry.lane_length;
    let mut frames = Vec::with_capacity(width * 6);
    for color_index in 0..6 {
        for boundary in 0..width {
            let forward = color_index % 2 == 0;
            let previous = if color_index == 0 { 5 } else { color_index - 1 };
            let mut output = frame(geometry);
            for lane in 0..geometry.lanes {
                for position in 0..width {
                    let current = if (forward && position <= boundary)
                        || (!forward && position >= width - boundary - 1)
                    {
                        color_index
                    } else {
                        previous
                    };
                    output[lane * width + position] = scale(colors[current], brightness);
                }
            }
            frames.push(output);
        }
    }
    frames
}
