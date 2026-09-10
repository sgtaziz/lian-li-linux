use super::engine::{brightness, palette, place, scale, Color, Side};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn runway(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let len = fans * side.leds_per_track();
    let span = len + 2 * fans - 1;
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    let mut frames = Vec::with_capacity(span * 2);
    for reverse in [false, true] {
        for step in 0..span {
            let mut track = vec![[0; 3]; len];
            for position in 0..len {
                let color = if position <= step && position + 2 * fans > step {
                    colors[0]
                } else {
                    colors[1]
                };
                track[if reverse {
                    len - position - 1
                } else {
                    position
                }] = color;
            }
            frames.push(place(&track, side, fans));
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
    let len = fans * side.leds_per_track();
    let span = len + 2 * fans - 1;
    let colors = palette(effect, side);
    let bright = brightness(effect);
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    let mut frames = Vec::with_capacity(span * 4);
    for color in colors {
        for step in 0..span {
            let mut tail_index = 0;
            let mut track = vec![[0; 3]; len];
            for position in 0..len {
                let color = if position <= step && position + 2 * fans > step {
                    let color = scale(scale(color, TAILS[fans - 1][tail_index]), bright);
                    tail_index += 1;
                    color
                } else {
                    [0; 3]
                };
                track[if reverse {
                    len - position - 1
                } else {
                    position
                }] = color;
            }
            frames.push(place(&track, side, fans));
        }
    }
    frames
}

pub(super) fn color_cycle(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let len = fans * side.leds_per_track();
    let short_track = matches!(side, Side::Outer);
    let lit = (if short_track {
        [1, 2, 4, 6]
    } else {
        [1, 4, 7, 10]
    })[fans - 1];
    let gap = (if short_track {
        [2, 3, 4, 5]
    } else {
        [3, 4, 5, 6]
    })[fans - 1];
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    let mut pattern = Vec::new();
    for color in colors.iter().take(3) {
        pattern.extend(std::iter::repeat_n(*color, lit));
        pattern.extend(std::iter::repeat_n([0; 3], gap));
        if short_track && (fans == 1 || fans == 4) {
            pattern.pop();
        }
        if short_track && fans == 2 {
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
            place(&track, side, fans)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_shared::rgb::{RgbMode, RgbScope};

    fn effect(mode: RgbMode) -> RgbEffect {
        RgbEffect {
            mode,
            colors: vec![[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]],
            brightness: 4,
            direction: RgbDirection::Clockwise,
            scope: RgbScope::Outer,
            ..Default::default()
        }
    }

    #[test]
    fn runway_keeps_the_vendor_bidirectional_pass_length() {
        let frames = runway(&effect(RgbMode::Runway), 1, Side::Inner);
        assert_eq!(frames.len(), 26);
        assert_eq!(frames[0][0], [254, 0, 0]);
        assert_eq!(frames[0][1], [0, 254, 0]);
    }

    #[test]
    fn meteor_uses_the_fan_count_specific_tail() {
        let frames = meteor(&effect(RgbMode::Meteor), 1, Side::Outer);
        assert_eq!(frames.len(), 36);
        assert_eq!(frames[0][12], [30, 0, 0]);
        assert_eq!(frames[1][13], [253, 0, 0]);
    }

    #[test]
    fn color_cycle_uses_the_native_outer_pattern() {
        let frames = color_cycle(&effect(RgbMode::ColorCycle), 1, Side::Inner);
        assert_eq!(frames.len(), 12);
        assert_eq!(frames[0][31], [254, 0, 0]);
        assert_eq!(frames[0][30], [0; 3]);
    }
}
