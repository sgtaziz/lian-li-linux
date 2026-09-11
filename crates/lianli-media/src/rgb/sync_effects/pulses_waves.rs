use super::engine::{scale, solid_frame, Color, Frames};

pub(super) fn disco(leds: usize, colors: &[Color; 6], bright: u16, reverse: bool) -> Frames {
    const INDEX: [usize; 12] = [0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3];
    (0..12)
        .map(|shift| {
            (0..leds)
                .map(|position| {
                    let source = if reverse {
                        position % 12
                    } else {
                        11 - position % 12
                    };
                    scale(colors[INDEX[(shift + source) % 12]], bright)
                })
                .collect()
        })
        .collect()
}

pub(super) fn blow_up(leds: usize, colors: &[Color; 6], bright: u16) -> Frames {
    let half = leds.div_ceil(2);
    let width = 12;
    let mixed = mixed_color(colors[0], colors[1]);
    let mut frames = Vec::new();
    for phase in 0..3 {
        let mut step = 0;
        while step < half + width - 1 {
            let mut frame = vec![[0; 3]; leds];
            for position in 0..half {
                let first = phase_color(phase, position, step, width, colors[0], mixed);
                let second = phase_color(phase, position, step, width, colors[1], mixed);
                let first_destination = if phase == 0 {
                    position
                } else {
                    half - position - 1
                };
                let second_destination = if phase == 0 {
                    2 * half - position - 1
                } else {
                    half + position
                };
                if first_destination < leds {
                    frame[first_destination] = scale(first, bright);
                }
                if second_destination < leds {
                    frame[second_destination] = scale(second, bright);
                }
            }
            frames.push(frame);
            step += if phase == 0 { 1 } else { 2 };
        }
    }
    frames
}

fn phase_color(
    phase: usize,
    position: usize,
    step: usize,
    width: usize,
    incoming: Color,
    mixed: Color,
) -> Color {
    if (phase == 1 && position < step) || (phase == 2 && position + width > step) {
        mixed
    } else if position < step && position + width > step {
        incoming
    } else {
        [0; 3]
    }
}

fn mixed_color(first: Color, second: Color) -> Color {
    let mut mixed = std::array::from_fn(|channel| {
        if u16::from(first[channel]) + u16::from(second[channel]) >= 255 {
            255
        } else {
            first[channel] + second[channel]
        }
    });
    if mixed.iter().map(|&channel| u16::from(channel)).sum::<u16>() > 536 {
        mixed = scale(mixed, 178);
    }
    mixed
}

pub(super) fn heart_beat(leds: usize, colors: &[Color; 6], bright: u16) -> Frames {
    let mut frames = Vec::with_capacity(2 * leds);
    for pass in 0..2 {
        for step in 0..leds {
            let mut frame = vec![[0; 3]; leds];
            for position in 0..leds {
                let index = if position <= step { pass + 1 } else { pass };
                let destination = if pass == 1 {
                    leds - position - 1
                } else {
                    position
                };
                frame[destination] = scale(colors[index], bright);
            }
            frames.push(frame);
        }
    }
    frames
}

pub(super) fn warning(leds: usize, colors: &[Color; 6], bright: u16) -> Frames {
    let half = leds.div_ceil(2);
    let width = 6;
    let mut frames = Vec::with_capacity(4 * half.saturating_sub(width));
    for color in colors.iter().take(2) {
        for reverse in [false, true] {
            for step in 0..half.saturating_sub(width) {
                let mut frame = vec![[0; 3]; leds];
                for position in 0..half {
                    let lit = position >= step && position < step + width;
                    let first = if reverse {
                        half - position - 1
                    } else {
                        position
                    };
                    let second = if reverse {
                        half + position
                    } else {
                        2 * half - position - 1
                    };
                    if first < leds {
                        frame[first] = if lit { scale(*color, bright) } else { [0; 3] };
                    }
                    if second < leds {
                        frame[second] = if lit { scale(*color, bright) } else { [0; 3] };
                    }
                }
                frames.push(frame);
            }
        }
    }
    frames
}

pub(super) fn sea_flow(leds: usize, color: Color, bright: u16, reverse: bool) -> Frames {
    let lengths = [leds, leds * 2 / 3, leds / 3];
    let full = scale(color, bright);
    let dim = full.map(|channel| channel / 8);
    let mut frames = Vec::with_capacity(2 * lengths.iter().sum::<usize>());
    for length in lengths {
        for dim_after in [true, false] {
            for step in 0..length {
                let mut frame = vec![[0; 3]; leds];
                for position in 0..leds {
                    let destination = if reverse {
                        leds - position - 1
                    } else {
                        position
                    };
                    let use_dim = if dim_after {
                        position > step
                    } else {
                        position > length - step
                    };
                    frame[destination] = if use_dim { dim } else { full };
                }
                frames.push(frame);
            }
        }
    }
    frames
}

pub(super) fn ripple(leds: usize, colors: &[Color; 6], bright: u16) -> Frames {
    let half = leds.div_ceil(2);
    let expansion_frames = half.div_ceil(2);
    let mut frames = Vec::with_capacity(2 * (expansion_frames + 35));
    for color in colors.iter().take(2) {
        for step in (0..half).step_by(2) {
            let mut frame = vec![[0; 3]; leds];
            for position in 0..half {
                let value = if position <= step {
                    scale(*color, bright)
                } else {
                    [0; 3]
                };
                let first = half - position - 1;
                let second = half + position;
                if first < leds {
                    frame[first] = value;
                }
                if second < leds {
                    frame[second] = value;
                }
            }
            frames.push(frame);
        }
        for step in 0..35 {
            let intensity = if step < 25 { (24 - step) * 10 } else { 0 } as u16;
            frames.push(solid_frame(leds, scale(scale(*color, intensity), bright)));
        }
    }
    frames
}

pub(super) fn echo(leds: usize, colors: &[Color; 6], bright: u16) -> Frames {
    const INDEX: [u8; 32] = [
        0, 1, 0, 2, 0, 1, 0, 2, 0, 1, 0, 2, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 2, 0, 2, 0, 2, 0, 2,
        0, 2,
    ];
    const REPEAT: [usize; 32] = [
        8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 2, 2, 2, 2, 2, 2, 2, 2, 2, 8, 2, 2, 2, 2, 2, 2, 2,
        2, 2,
    ];
    let mut frames = Vec::with_capacity(REPEAT.iter().sum());
    for (index, repeat) in INDEX.into_iter().zip(REPEAT) {
        let color = if index == 0 {
            [0; 3]
        } else {
            scale(colors[index as usize - 1], bright)
        };
        frames.extend(std::iter::repeat_n(solid_frame(leds, color), repeat));
    }
    frames
}
