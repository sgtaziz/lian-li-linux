use super::geometry::{frame, map_16, map_25, map_31, scale, Color};

pub(super) fn paint(led_count: usize, colors: &[Color], brightness: u8) -> Vec<Vec<Color>> {
    let mut frames = Vec::with_capacity(colors.len() * 31);
    for color_index in 0..colors.len() {
        for step in 0..31 {
            let mut path = [[0; 3]; 31];
            for position in 0..31 {
                let source = if position > step {
                    (color_index + colors.len() - 1) % colors.len()
                } else {
                    color_index
                };
                let destination = if color_index % 2 == 1 {
                    30 - position
                } else {
                    position
                };
                path[destination] = scale(colors[source], brightness);
            }
            frames.push(map_31(&path, led_count));
        }
    }
    frames
}

pub(super) fn runway(led_count: usize, colors: &[Color; 6], brightness: u8) -> Vec<Vec<Color>> {
    let mut frames = Vec::with_capacity(70);
    for reverse in [false, true] {
        for step in 0..35 {
            let mut path = [[0; 3]; 31];
            for position in 0..31 {
                let source = usize::from(position <= step && position + 4 > step);
                let destination = if reverse { 30 - position } else { position };
                path[destination] = scale(colors[source], brightness);
            }
            frames.push(map_31(&path, led_count));
        }
    }
    frames
}

pub(super) fn tide(led_count: usize, colors: &[Color], brightness: u8) -> Vec<Vec<Color>> {
    let mut frames = Vec::with_capacity(colors.len() * 16);
    for color_index in 0..colors.len() {
        for step in 0..16 {
            let mut path = [[0; 3]; 16];
            for (position, target) in path.iter_mut().enumerate() {
                let source = if position > step {
                    (color_index + colors.len() - 1) % colors.len()
                } else {
                    color_index
                };
                *target = scale(colors[source], brightness);
            }
            frames.push(map_16(&path, led_count));
        }
    }
    frames
}

pub(super) fn blow_up(led_count: usize, colors: &[Color], brightness: u8) -> Vec<Vec<Color>> {
    let mut frames = Vec::with_capacity(colors.len() * 51);
    for color in colors {
        for step in 0..16 {
            let mut path = [[0; 3]; 16];
            for position in 0..16 {
                if position <= step {
                    path[15 - position] = scale(*color, brightness);
                }
            }
            frames.push(map_16(&path, led_count));
        }
        for step in 0..35 {
            let intensity = if step < 25 { (24 - step) * 10 } else { 0 } as u8;
            let color = scale(scale(*color, intensity), brightness);
            let mut output = frame(led_count);
            output[..60].fill(color);
            frames.push(output);
        }
    }
    frames
}

pub(super) fn meteor(
    led_count: usize,
    colors: &[Color],
    brightness: u8,
    reverse: bool,
) -> Vec<Vec<Color>> {
    const TAIL: [u8; 5] = [16, 32, 64, 128, 255];
    let mut frames = Vec::with_capacity(colors.len() * 30);
    for color in colors {
        for step in 0..30 {
            let mut path = [[0; 3]; 25];
            let mut tail_index = 0;
            for position in 0..25 {
                let intensity = if position <= step && position + 5 > step {
                    let value = TAIL[tail_index];
                    tail_index += 1;
                    value
                } else {
                    0
                };
                let destination = if reverse { 24 - position } else { position };
                path[destination] = scale(scale(*color, intensity), brightness);
            }
            frames.push(map_25(&path, led_count));
        }
    }
    frames
}

pub(super) fn snooker(led_count: usize, colors: &[Color], brightness: u8) -> Vec<Vec<Color>> {
    let mut frames = Vec::with_capacity(colors.len() * 28);
    for color in colors {
        for reverse in [false, true] {
            for step in 1..15 {
                let mut path = [[0; 3]; 16];
                for position in 0..16 {
                    let destination = if reverse { 15 - position } else { position };
                    if position <= step && position + 3 > step {
                        path[destination] = scale(*color, brightness);
                    }
                }
                frames.push(map_16(&path, led_count));
            }
        }
    }
    frames
}

pub(super) fn mixing(led_count: usize, colors: &[Color; 6], brightness: u8) -> Vec<Vec<Color>> {
    let mixed = std::array::from_fn(|channel| {
        u16::from(colors[0][channel])
            .saturating_add(u16::from(colors[1][channel]))
            .min(255) as u8
    });
    let mixed = if mixed.iter().map(|&value| u16::from(value)).sum::<u16>() > 536 {
        scale(mixed, 178)
    } else {
        mixed
    };
    let mut frames = Vec::with_capacity(32);
    for fill in [false, true] {
        for step in 0..16 {
            let mut path = [[0; 3]; 32];
            for position in 0..16 {
                let first = if fill && position <= step {
                    mixed
                } else if position <= step && position + 3 > step {
                    colors[0]
                } else {
                    [0; 3]
                };
                let second = if fill && position <= step {
                    mixed
                } else if position <= step && position + 3 > step {
                    colors[1]
                } else {
                    [0; 3]
                };
                let first_destination = if fill { 15 - position } else { position };
                let second_destination = if fill { 16 + position } else { 31 - position };
                path[first_destination] = scale(first, brightness);
                path[second_destination] = scale(second, brightness);
            }
            frames.push(super::geometry::map_32(&path, led_count));
        }
    }
    frames
}

pub(super) fn ping_pong(led_count: usize, colors: &[Color], brightness: u8) -> Vec<Vec<Color>> {
    let mut frames = Vec::with_capacity(colors.len() * 58);
    for color in colors {
        for reverse in [false, true] {
            for step in 0..29 {
                let mut path = [[0; 3]; 25];
                for position in 0..25 {
                    let destination = if reverse { 24 - position } else { position };
                    if position <= step && position + 4 > step {
                        path[destination] = scale(*color, brightness);
                    }
                }
                frames.push(map_25(&path, led_count));
            }
        }
    }
    frames
}

pub(super) fn bullet_stack(led_count: usize, brightness: u8, reverse: bool) -> Vec<Vec<Color>> {
    const COLORS: [Color; 7] = [
        [153, 0, 204],
        [255, 51, 204],
        [255, 153, 0],
        [255, 255, 0],
        [0, 255, 102],
        [51, 255, 255],
        [66, 87, 248],
    ];
    let mut frames = Vec::with_capacity(325);
    let mut path = [[0; 3]; 25];
    let mut color_index = 0;
    for stacked in 0..25 {
        color_index = (color_index + 1) % COLORS.len();
        for step in 0..25 - stacked {
            for position in 0..25 - stacked {
                let destination = if reverse { 24 - position } else { position };
                path[destination] = if position == step {
                    scale(COLORS[color_index], brightness)
                } else {
                    [0; 3]
                };
            }
            for offset in 0..stacked {
                let previous = (color_index + COLORS.len() - 1) % COLORS.len();
                let position = 25 - stacked + offset;
                let destination = if reverse { 24 - position } else { position };
                path[destination] = scale(COLORS[previous], brightness);
            }
            frames.push(map_25(&path, led_count));
        }
    }
    frames
}

pub(super) fn river(
    led_count: usize,
    colors: &[Color; 6],
    brightness: u8,
    reverse: bool,
) -> Vec<Vec<Color>> {
    const INDEX: [usize; 6] = [0, 0, 0, 0, 1, 1];
    (0..6)
        .map(|shift| {
            let mut segment = [[0; 3]; 6];
            for position in 0..6 {
                let destination = if reverse { position } else { 5 - position };
                segment[destination] = scale(colors[INDEX[(shift + position) % 6]], brightness);
            }
            let mut output = frame(led_count);
            for repeat in 0..10 {
                output[repeat * 6..repeat * 6 + 6].copy_from_slice(&segment);
            }
            output
        })
        .collect()
}

pub(super) fn rainbow_wave(led_count: usize, brightness: u8, reverse: bool) -> Vec<Vec<Color>> {
    const MASK: [bool; 60] = [
        false, false, false, false, false, true, true, true, true, true, true, true, false, false,
        false, false, false, true, true, true, true, true, true, true, false, false, false, false,
        false, true, true, true, true, true, true, true, false, false, false, false, false, true,
        true, true, true, true, true, true, false, false, false, false, false, true, true, true,
        true, true, true, true,
    ];
    const TABLE: [Color; 60] = [
        [255, 0, 0],
        [242, 8, 0],
        [229, 21, 0],
        [216, 34, 0],
        [203, 47, 0],
        [190, 60, 0],
        [177, 73, 0],
        [164, 86, 0],
        [151, 99, 0],
        [138, 112, 0],
        [125, 125, 0],
        [112, 138, 0],
        [99, 151, 0],
        [86, 164, 0],
        [73, 177, 0],
        [60, 190, 0],
        [47, 203, 0],
        [34, 216, 0],
        [21, 229, 0],
        [8, 242, 0],
        [0, 255, 0],
        [0, 242, 8],
        [0, 229, 21],
        [0, 216, 34],
        [0, 203, 47],
        [0, 190, 60],
        [0, 177, 73],
        [0, 164, 86],
        [0, 151, 99],
        [0, 138, 112],
        [0, 125, 125],
        [0, 112, 138],
        [0, 99, 151],
        [0, 86, 164],
        [0, 73, 177],
        [0, 60, 190],
        [0, 47, 203],
        [0, 34, 216],
        [0, 21, 229],
        [0, 8, 242],
        [0, 0, 255],
        [8, 0, 242],
        [21, 0, 229],
        [34, 0, 216],
        [47, 0, 203],
        [60, 0, 190],
        [73, 0, 177],
        [86, 0, 164],
        [99, 0, 151],
        [112, 0, 138],
        [125, 0, 125],
        [138, 0, 112],
        [151, 0, 99],
        [164, 0, 86],
        [177, 0, 73],
        [190, 0, 60],
        [203, 0, 47],
        [216, 0, 34],
        [229, 0, 21],
        [242, 0, 8],
    ];
    (0..60)
        .map(|shift| {
            let mut generated = [[0; 3]; 60];
            for position in 0..60 {
                let color = scale(TABLE[(shift + position * 2) % 60], brightness);
                generated[position] = if MASK[(shift + position) % 60] {
                    color
                } else {
                    [0; 3]
                };
            }
            let mut output = frame(led_count);
            for (position, target) in output.iter_mut().take(60).enumerate() {
                let source = if reverse { position } else { 59 - position };
                *target = generated[source];
            }
            output
        })
        .collect()
}
