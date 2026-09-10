use super::engine::{brightness, mixed_color, palette, place, scale, Color, Plane, CENTER_ORDER};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn voice(effect: &RgbEffect, fans: usize, plane: Plane) -> Vec<Vec<Color>> {
    let colors = palette(effect, plane).map(|color| scale(color, brightness(effect)));
    let len = fans * 8;
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    let band_lengths = [len, len * 2 / 3, len / 3];
    let mut frames = Vec::with_capacity(8 * band_lengths.iter().sum::<usize>());
    for color in colors {
        for band in band_lengths {
            for pass in 0..2 {
                for step in 0..band {
                    let mut track = vec![[0; 3]; len];
                    for position in 0..len {
                        let mut target = if reverse {
                            len - position - 1
                        } else {
                            position
                        };
                        if plane == Plane::Center {
                            target = CENTER_ORDER[target] - 1;
                        }
                        let dimmed =
                            (pass == 0 && position > step) || (pass == 1 && position > band - step);
                        track[target] = if dimmed {
                            color.map(|channel| channel / 8)
                        } else {
                            color
                        };
                    }
                    frames.push(place(&track, plane, fans));
                }
            }
        }
    }
    frames
}

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
