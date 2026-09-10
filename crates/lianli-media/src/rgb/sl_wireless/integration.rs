use super::basic::rainbow_color;
use super::engine::{brightness, palette, place_banks, scale, Color, Side};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

fn place_continuous(track: &[Color], side: Side, fans: usize) -> Vec<Color> {
    let len = fans * side.leds_per_track();
    let second: Vec<Color> = track[len..].iter().copied().rev().collect();
    place_banks(&track[..len], &second, side, fans)
}

fn place_mirrored(track: &[Color], side: Side, fans: usize) -> Vec<Color> {
    let second: Vec<Color> = track.iter().copied().rev().collect();
    place_banks(track, &second, side, fans)
}

pub(super) fn rainbow(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let len = fans * side.leds_per_track();
    let step = match side {
        Side::Outer => [12, 6, 4, 3][fans - 1],
        Side::Inner => [8, 4, 1, 2][fans - 1],
    };
    let table_len = if len == 36 { 36 } else { 96 };
    let clockwise = !matches!(effect.direction, RgbDirection::CounterClockwise);
    let bright = brightness(effect);
    (0..len)
        .map(|frame| {
            let mut source = frame * step;
            let mut track = vec![[0; 3]; len];
            for position in 0..len {
                let target = if clockwise {
                    len - position - 1
                } else {
                    position
                };
                track[target] = scale(rainbow_color(source, table_len), bright);
                source = (source + step) % table_len;
            }
            place_mirrored(&track, side, fans)
        })
        .collect()
}

pub(super) fn runway(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let len = 2 * fans * side.leds_per_track();
    let span = len + 2 * fans - 1;
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    let mut frames = Vec::with_capacity(2 * span);
    for pass in 0..2 {
        for step in 0..span {
            let mut track = vec![[0; 3]; len];
            for position in 0..len {
                let color = if position <= step && position + 2 * fans > step {
                    colors[0]
                } else {
                    colors[1]
                };
                let target = if pass == 0 {
                    position
                } else {
                    len - position - 1
                };
                track[target] = color;
            }
            frames.push(place_continuous(&track, side, fans));
        }
    }
    frames
}

pub(super) fn meteor(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    const TAILS: [[u16; 8]; 4] = [
        [32, 255, 0, 0, 0, 0, 0, 0],
        [16, 64, 128, 255, 0, 0, 0, 0],
        [8, 16, 32, 64, 128, 255, 0, 0],
        [6, 10, 16, 32, 64, 96, 168, 255],
    ];
    let len = 2 * fans * side.leds_per_track();
    let span = len + 2 * fans - 1;
    let colors = palette(effect, side);
    let bright = brightness(effect);
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    let mut frames = Vec::with_capacity(4 * span);
    for color in colors {
        for step in 0..span {
            let mut tail = 0;
            let mut track = vec![[0; 3]; len];
            for position in 0..len {
                if position <= step && position + 2 * fans > step {
                    let target = if reverse {
                        len - position - 1
                    } else {
                        position
                    };
                    track[target] = scale(scale(color, TAILS[fans - 1][tail]), bright);
                    tail += 1;
                }
            }
            frames.push(place_continuous(&track, side, fans));
        }
    }
    frames
}

pub(super) fn color_cycle(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let len = 2 * fans * side.leds_per_track();
    let short = matches!(side, Side::Outer);
    let lit = 2 * (if short { [1, 2, 4, 6] } else { [1, 4, 7, 10] })[fans - 1];
    let gap = 2 * (if short { [2, 3, 4, 5] } else { [3, 4, 5, 6] })[fans - 1];
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    let mut pattern = Vec::new();
    for color in colors.iter().take(3) {
        pattern.extend(std::iter::repeat_n(*color, lit));
        pattern.extend(std::iter::repeat_n([0; 3], gap));
        if short && (fans == 1 || fans == 4) {
            pattern.pop();
        }
        if short && fans == 2 {
            pattern.push([0; 3]);
        }
    }
    pattern.resize(len, [0; 3]);
    let clockwise = !matches!(effect.direction, RgbDirection::CounterClockwise);
    (0..len)
        .map(|shift| {
            let mut track = vec![[0; 3]; len];
            for position in 0..len {
                let target = if clockwise {
                    len - position - 1
                } else {
                    position
                };
                track[target] = pattern[(position + shift) % len];
            }
            place_continuous(&track, side, fans)
        })
        .collect()
}

pub(super) fn render_effect(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    const SHORT: [[u8; 8]; 5] = [
        [0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 1, 1, 0, 0, 0],
        [0, 0, 1, 1, 1, 1, 0, 0],
        [0, 1, 1, 1, 1, 1, 1, 0],
        [1, 1, 1, 1, 1, 1, 1, 1],
    ];
    const LONG: [[u8; 12]; 7] = [
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0],
        [0, 0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 0],
        [0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0],
        [0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0],
        [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    ];
    let colors = palette(effect, side);
    let bright = brightness(effect);
    let per_fan = side.leds_per_track();
    let stages = if matches!(side, Side::Outer) { 5 } else { 7 };
    let virtual_fans = 2 * fans;
    let clockwise = !matches!(effect.direction, RgbDirection::CounterClockwise);
    let mut frames = Vec::with_capacity(4 * virtual_fans * stages);
    for color in 0..4 {
        for frontier in 0..virtual_fans {
            for stage in 0..stages {
                let mut track = vec![[0; 3]; virtual_fans * per_fan];
                for fan in 0..virtual_fans {
                    for led in 0..per_fan {
                        let mask = if matches!(side, Side::Outer) {
                            SHORT[stage][led]
                        } else {
                            LONG[stage][led]
                        };
                        let next = fan < frontier || (fan == frontier && mask == 1);
                        track[fan * per_fan + led] =
                            scale(colors[(color + usize::from(next)) % 4], bright);
                    }
                }
                if !clockwise {
                    track.reverse();
                }
                frames.push(place_continuous(&track, side, fans));
            }
        }
    }
    frames
}
