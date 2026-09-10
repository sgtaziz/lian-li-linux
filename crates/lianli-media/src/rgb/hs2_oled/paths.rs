use super::geometry::{centered_15, frame, mirrored_18, scale, Color, ACTIVE_LEDS};

pub(super) fn paint(colors: &[Color; 6], brightness: u8) -> Vec<Vec<Color>> {
    let mut frames = Vec::with_capacity(108);
    for color_index in 0..6 {
        for step in 0..18 {
            let mut path = [[0; 3]; 18];
            for position in 0..18 {
                let source = if position > step {
                    (color_index + 5) % 6
                } else {
                    color_index
                };
                let destination = if color_index % 2 == 1 {
                    17 - position
                } else {
                    position
                };
                path[destination] = scale(colors[source], brightness);
            }
            frames.push(mirrored_18(&path));
        }
    }
    frames
}

pub(super) fn runway(colors: &[Color; 6], brightness: u8) -> Vec<Vec<Color>> {
    let mut frames = Vec::with_capacity(44);
    for reverse in [false, true] {
        for step in 0..22 {
            let mut path = [[0; 3]; 18];
            for position in 0..18 {
                let source = usize::from(!(position <= step && position + 4 > step));
                let destination = if reverse { 17 - position } else { position };
                path[destination] = scale(colors[source], brightness);
            }
            frames.push(mirrored_18(&path));
        }
    }
    frames
}

pub(super) fn tide(colors: &[Color; 6], brightness: u8, reverse: bool) -> Vec<Vec<Color>> {
    let mut frames = Vec::with_capacity(108);
    for color_index in 0..6 {
        for step in 0..18 {
            let mut path = [[0; 3]; 18];
            for position in 0..18 {
                let source = if position > step {
                    (color_index + 5) % 6
                } else {
                    color_index
                };
                let destination = if reverse { 17 - position } else { position };
                path[destination] = scale(colors[source], brightness);
            }
            frames.push(mirrored_18(&path));
        }
    }
    frames
}

pub(super) fn blow_up(colors: &[Color; 6], brightness: u8) -> Vec<Vec<Color>> {
    let mut frames = Vec::with_capacity(258);
    for color in colors {
        for step in 0..8 {
            let mut segment = [[0; 3]; 8];
            for (position, target) in segment.iter_mut().enumerate() {
                if position <= step {
                    *target = scale(*color, brightness);
                }
            }
            let mut output = frame();
            for position in 0..8 {
                output[7 - position] = segment[position];
            }
            for position in 0..7 {
                output[position + 7] = segment[position];
                output[27 - position] = segment[position];
                output[position + 14] = segment[7];
            }
            output[27..35].copy_from_slice(&segment);
            frames.push(output);
        }
        for step in 0..35 {
            let intensity = if step < 25 { (24 - step) * 10 } else { 0 } as u8;
            let color = scale(scale(*color, intensity), brightness);
            let mut output = frame();
            output[..ACTIVE_LEDS].fill(color);
            frames.push(output);
        }
    }
    frames
}

pub(super) fn meteor(colors: &[Color; 6], brightness: u8, reverse: bool) -> Vec<Vec<Color>> {
    const TAIL: [u8; 5] = [16, 32, 64, 128, 255];
    let mut frames = Vec::with_capacity(138);
    for color in colors {
        for step in 0..23 {
            let mut path = [[0; 3]; 18];
            let mut tail_index = 0;
            for position in 0..18 {
                let intensity = if position <= step && position + 5 > step {
                    let value = TAIL[tail_index];
                    tail_index += 1;
                    value
                } else {
                    0
                };
                let destination = if reverse { 17 - position } else { position };
                path[destination] = scale(scale(*color, intensity), brightness);
            }
            frames.push(mirrored_18(&path));
        }
    }
    frames
}

pub(super) fn snooker(colors: &[Color; 6], brightness: u8) -> Vec<Vec<Color>> {
    let mut frames = Vec::with_capacity(192);
    for color in colors {
        for reverse in [false, true] {
            for step in 1..17 {
                let mut path = [[0; 3]; 18];
                for position in 0..18 {
                    let destination = if reverse { 17 - position } else { position };
                    if position <= step && position + 3 > step {
                        path[destination] = scale(*color, brightness);
                    }
                }
                frames.push(mirrored_18(&path));
            }
        }
    }
    frames
}

pub(super) fn mixing(colors: &[Color; 6], brightness: u8) -> Vec<Vec<Color>> {
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
    let mut frames = Vec::with_capacity(36);
    for fill in [false, true] {
        for step in 0..18 {
            let mut path = [[0; 3]; 36];
            for position in 0..18 {
                let first = if fill && position <= step {
                    mixed
                } else if !fill && position <= step && position + 3 > step {
                    colors[0]
                } else {
                    [0; 3]
                };
                let second = if fill && position <= step {
                    mixed
                } else if !fill && position <= step && position + 3 > step {
                    colors[1]
                } else {
                    [0; 3]
                };
                let first_destination = if fill { 17 - position } else { position };
                let second_destination = if fill { 18 + position } else { 35 - position };
                path[first_destination] = scale(first, brightness);
                path[second_destination] = scale(second, brightness);
            }
            let mut output = frame();
            output[..18].copy_from_slice(&path[..18]);
            output[17..35].copy_from_slice(&path[18..36]);
            frames.push(output);
        }
    }
    frames
}

pub(super) fn ping_pong(colors: &[Color; 6], brightness: u8) -> Vec<Vec<Color>> {
    let mut frames = Vec::with_capacity(228);
    for color in colors {
        for reverse in [false, true] {
            for step in 0..19 {
                let mut path = [[0; 3]; 15];
                for position in 0..15 {
                    let destination = if reverse { 14 - position } else { position };
                    if position <= step && position + 4 > step {
                        path[destination] = scale(*color, brightness);
                    }
                }
                frames.push(centered_15(&path));
            }
        }
    }
    frames
}

pub(super) fn bullet_stack(brightness: u8, reverse: bool) -> Vec<Vec<Color>> {
    const COLORS: [Color; 7] = [
        [153, 0, 204],
        [255, 51, 204],
        [255, 153, 0],
        [255, 255, 0],
        [0, 255, 102],
        [51, 255, 255],
        [66, 87, 248],
    ];
    let mut frames = Vec::with_capacity(120);
    let mut path = [[0; 3]; 15];
    let mut color_index = COLORS.len() - 1;
    for stacked in 0..15 {
        color_index = (color_index + 1) % COLORS.len();
        for step in 0..15 - stacked {
            for position in 0..15 - stacked {
                let destination = if reverse { 14 - position } else { position };
                path[destination] = if position == step {
                    scale(COLORS[color_index], brightness)
                } else {
                    [0; 3]
                };
            }
            for offset in 0..stacked {
                let previous = (color_index + COLORS.len() - 1) % COLORS.len();
                let position = 15 - stacked + offset;
                let destination = if reverse { 14 - position } else { position };
                path[destination] = scale(COLORS[previous], brightness);
            }
            frames.push(centered_15(&path));
        }
    }
    frames
}

pub(super) fn river(colors: &[Color; 6], brightness: u8, reverse: bool) -> Vec<Vec<Color>> {
    const COLOR_INDEX: [usize; 6] = [0, 0, 0, 0, 1, 1];
    (0..6)
        .map(|shift| {
            let mut segment = [[0; 3]; 6];
            for position in 0..6 {
                let destination = if reverse { position } else { 5 - position };
                segment[destination] =
                    scale(colors[COLOR_INDEX[(shift + position) % 6]], brightness);
            }
            let mut output = frame();
            for position in 0..ACTIVE_LEDS {
                output[position] = segment[position % 6];
            }
            output
        })
        .collect()
}

pub(super) fn hourglass(colors: &[Color; 6], brightness: u8) -> Vec<Vec<Color>> {
    let mut frames = Vec::with_capacity(388);
    for color in colors.iter().take(4) {
        for pass in 0..4 {
            for step in 0..18 {
                let mut output = frame();
                for position in 0..18 {
                    if pass == 3 && position >= 15 {
                        break;
                    }
                    let destination = if pass == 3 { 14 - position } else { position };
                    if position < step || position >= 18 - pass {
                        output[destination] = scale(*color, brightness);
                    }
                }
                for position in 0..18 {
                    output[position + 17] = output[17 - position];
                }
                frames.push(output);
            }
        }
        for step in 0..25 {
            let intensity = ((24 - step) * 10) as u8;
            let color = scale(scale(*color, intensity), brightness);
            let mut output = frame();
            output[..ACTIVE_LEDS].fill(color);
            frames.push(output);
        }
    }
    frames
}

pub(super) fn electric_current(colors: &[Color; 6], brightness: u8) -> Vec<Vec<Color>> {
    let mut frames = Vec::with_capacity(160);
    for color in colors.iter().take(4) {
        let mut warmup = 0;
        for reverse in [false, true] {
            let mut step = 0;
            while step < 23 {
                if warmup < 10 {
                    warmup += 1;
                    step = 0;
                }
                let mut output = frame();
                for position in 0..18 {
                    if position < step && position + 6 >= step {
                        let first = if reverse { 17 - position } else { position };
                        let second = if reverse { position } else { 17 - position };
                        let color = scale(*color, brightness);
                        output[first] = color;
                        output[second + 17] = color;
                    }
                }
                frames.push(output);
                step += if reverse { 3 } else { 1 };
            }
        }
    }
    frames
}
