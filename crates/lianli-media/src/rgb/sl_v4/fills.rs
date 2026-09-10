use super::engine::{brightness, clamp_current, palette, place, scale, Color, Side};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn mixing(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let colors = palette(effect, side);
    let mixed = clamp_current(std::array::from_fn(|channel| {
        colors[0][channel].saturating_add(colors[1][channel])
    }));
    let bright = brightness(effect);
    let len = [14, 26, 40, 52][fans - 1];
    let half = len / 2;
    let span = half + 2 * fans - 1;
    let mut delay = 0;
    let mut frames = Vec::new();
    for phase in 0..3 {
        let mut step = 0;
        while step < span {
            if delay < 10 {
                delay += 1;
                step = 0;
            }
            let mut track = vec![[0; 3]; len];
            for position in 0..half {
                let blended =
                    (phase == 1 && position < step) || (phase == 2 && position + 2 * fans > step);
                let moving = position < step && position + 2 * fans > step;
                let first = if blended {
                    mixed
                } else if moving {
                    colors[0]
                } else {
                    [0; 3]
                };
                let second = if blended {
                    mixed
                } else if moving {
                    colors[if phase == 0 { 1 } else { 2 }]
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
                track[first_target] = scale(first, bright);
                track[half + second_target] = scale(second, bright);
            }
            frames.push(place(&track[..fans * 13], side, fans));
            step += if phase == 0 { 1 } else { 2 };
        }
    }
    frames
}

pub(super) fn render_effect(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    const MASK: [[u8; 13]; 8] = [
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 1, 1, 1, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 1, 1, 1, 1, 1, 0, 0, 0, 0],
        [0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0],
        [0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0],
        [0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0],
        [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    ];
    let colors = palette(effect, side);
    let bright = brightness(effect);
    let per_fan = side.leds_per_track();
    let clockwise = !matches!(effect.direction, RgbDirection::CounterClockwise);
    let mut frames = Vec::with_capacity(4 * fans * MASK.len());
    for color in 0..4 {
        for frontier in 0..fans {
            for mask in MASK {
                let mut track = vec![[0; 3]; fans * per_fan];
                for fan in 0..fans {
                    for (led, &value) in mask.iter().enumerate() {
                        let next = fan < frontier || (fan == frontier && value == 1);
                        let target = fan * per_fan + led;
                        track[target] = scale(colors[(color + usize::from(next)) % 4], bright);
                    }
                }
                if !clockwise {
                    track.reverse();
                }
                frames.push(place(&track, side, fans));
            }
        }
    }
    frames
}

pub(super) fn ping_pong(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let len = fans * side.leds_per_track();
    let span = len + fans;
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    let mut frames = Vec::with_capacity(span * 2);
    for (pass, color) in colors.iter().take(2).enumerate() {
        for step in 0..span {
            let mut track = vec![[0; 3]; len];
            for position in 0..len {
                if position <= step && position + 2 * fans > step {
                    let target = if pass == 0 {
                        position
                    } else {
                        len - position - 1
                    };
                    track[target] = *color;
                }
            }
            frames.push(place(&track, side, fans));
        }
    }
    frames
}

pub(super) fn stack(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let len = fans * side.leds_per_track();
    let width = if fans == 4 { 4 } else { 3 };
    let stack_count = [4, 9, 13, 13][fans - 1];
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    let mut source = Vec::new();
    let mut track = vec![[0; 3]; len];
    for color in colors {
        let mut stacked = 0;
        for _ in 0..stack_count {
            for step in 0..len - stacked {
                for position in 0..len - stacked {
                    let target = if reverse {
                        len - position - 1
                    } else {
                        position
                    };
                    track[target] = if position <= step && position + width > step {
                        color
                    } else {
                        [0; 3]
                    };
                }
                source.push(place(&track, side, fans));
            }
            stacked += width;
        }
        for step in 0..len {
            for position in 0..len {
                let target = if reverse {
                    len - position - 1
                } else {
                    position
                };
                track[target] = if position > step { color } else { [0; 3] };
            }
            source.push(place(&track, side, fans));
        }
    }
    let output_len = source.len() / 3;
    (0..output_len)
        .map(|frame| source[frame * 2].clone())
        .collect()
}
