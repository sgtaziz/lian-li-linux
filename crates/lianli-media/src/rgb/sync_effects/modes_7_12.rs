use super::engine::{scale, Color, Frames};

pub(super) fn stack(leds: usize, color: Color, bright: u16, reverse: bool) -> Frames {
    let width = 12;
    let rounds = leds.div_ceil(width);
    let mut current = vec![[0; 3]; leds];
    let mut frames = Vec::new();
    for round in 0..rounds {
        let remaining = leds - round * width;
        for step in 0..remaining {
            for position in 0..remaining {
                let lit = position <= step && position + width > step;
                let destination = if reverse {
                    leds - position - 1
                } else {
                    position
                };
                current[destination] = if lit { scale(color, bright) } else { [0; 3] };
            }
            frames.push(current.clone());
        }
    }
    frames
}

pub(super) fn stack_frame_count(leds: usize) -> usize {
    let rounds = leds.div_ceil(12);
    rounds * leds - 12 * rounds * (rounds - 1) / 2
}

pub(super) fn color_cycle(leds: usize, colors: &[Color; 6], bright: u16, reverse: bool) -> Frames {
    const PATTERN: [u8; 48] = [
        1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 2, 2, 2, 2, 2, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 3, 3, 3, 3, 3, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    repeated_pattern(leds, 48, reverse, |shift, position| {
        let index = PATTERN[(shift + position) % 48];
        if index == 0 {
            [0; 3]
        } else {
            scale(colors[index as usize - 1], bright)
        }
    })
}

pub(super) fn cover_cycle(leds: usize, colors: &[Color; 6], bright: u16, reverse: bool) -> Frames {
    let mut frames = Vec::with_capacity(2 * leds);
    for pass in 0..2 {
        for step in 0..leds {
            let mut source = vec![[0; 3]; leds];
            for (position, target) in source.iter_mut().enumerate() {
                let index = if position < step {
                    pass
                } else {
                    usize::from(pass == 0)
                };
                *target = scale(colors[index], bright);
            }
            if reverse {
                source.reverse();
            }
            frames.push(source);
        }
    }
    frames
}

pub(super) fn wave(leds: usize, color: Color, bright: u16, reverse: bool) -> Frames {
    const INTENSITY: [u16; 24] = [
        0, 0, 4, 8, 16, 32, 64, 96, 128, 160, 192, 255, 255, 192, 160, 128, 96, 64, 32, 16, 8, 4,
        0, 0,
    ];
    repeated_pattern(leds, 24, reverse, |shift, position| {
        scale(scale(color, INTENSITY[(shift + position) % 24]), bright)
    })
}

pub(super) fn meteor_shower(
    leds: usize,
    colors: &[Color; 6],
    bright: u16,
    reverse: bool,
) -> Frames {
    const INTENSITY: [u16; 48] = [
        255, 128, 64, 32, 16, 8, 0, 0, 0, 0, 255, 64, 32, 16, 0, 0, 0, 0, 0, 255, 192, 168, 128,
        96, 64, 32, 16, 8, 0, 0, 0, 0, 0, 0, 0, 255, 128, 64, 32, 16, 8, 0, 0, 0, 0, 0, 0, 0,
    ];
    const COLOR: [usize; 48] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
        2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    ];
    repeated_pattern(leds, 48, reverse, |shift, position| {
        let source = (shift + position) % 48;
        scale(scale(colors[COLOR[source]], INTENSITY[source]), bright)
    })
}

fn repeated_pattern(
    leds: usize,
    period: usize,
    reverse: bool,
    color: impl Fn(usize, usize) -> Color,
) -> Frames {
    (0..period)
        .map(|shift| {
            let pattern: Vec<_> = (0..period).map(|position| color(shift, position)).collect();
            (0..leds)
                .map(|position| {
                    let source = if reverse {
                        position % period
                    } else {
                        period - position % period - 1
                    };
                    pattern[source]
                })
                .collect()
        })
        .collect()
}
