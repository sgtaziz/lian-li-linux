use super::engine::{brightness, clamp_current, palette, place, scale, Color, Side};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn mixing(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let colors = palette(effect, side);
    let mixed = clamp_current(std::array::from_fn(|channel| {
        colors[0][channel].saturating_add(colors[1][channel])
    }));
    let bright = brightness(effect);
    let half = fans * side.leds_per_track() / 2;
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
            let mut track = vec![[0; 3]; half * 2];
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
            frames.push(place(&track, side, fans));
            step += if phase == 0 { 1 } else { 2 };
        }
    }
    frames
}

pub(super) fn render_effect(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    const INNER: [[u8; 8]; 5] = [
        [0, 0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 1, 1, 0, 0, 0],
        [0, 0, 1, 1, 1, 1, 0, 0],
        [0, 1, 1, 1, 1, 1, 1, 0],
        [1, 1, 1, 1, 1, 1, 1, 1],
    ];
    const OUTER: [[u8; 12]; 7] = [
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
    let clockwise = !matches!(effect.direction, RgbDirection::CounterClockwise);
    let stages = if matches!(side, Side::Outer) { 5 } else { 7 };
    let mut frames = Vec::with_capacity(4 * fans * stages);
    for color in 0..4 {
        for frontier in 0..fans {
            for stage in 0..stages {
                let mut track = vec![[0; 3]; fans * per_fan];
                for fan in 0..fans {
                    for led in 0..per_fan {
                        let mask = if matches!(side, Side::Outer) {
                            INNER[stage][led]
                        } else {
                            OUTER[stage][led]
                        };
                        let next = fan < frontier || (fan == frontier && mask == 1);
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
    let width = if matches!(side, Side::Outer) { 2 } else { 3 };
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    let reverse = matches!(effect.direction, RgbDirection::CounterClockwise);
    let mut source = Vec::new();
    let mut track = vec![[0; 3]; len];
    for color in colors {
        let mut stacked = 0;
        for _ in 0..4 * fans {
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
    source.into_iter().step_by(2).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_shared::rgb::RgbMode;

    fn effect(mode: RgbMode) -> RgbEffect {
        RgbEffect {
            mode,
            colors: vec![[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]],
            brightness: 4,
            ..Default::default()
        }
    }

    #[test]
    fn mixing_preserves_the_vendor_delay_and_saturated_sum() {
        let frames = mixing(&effect(RgbMode::Mixing), 1, Side::Outer);
        assert_eq!(frames.len(), 20);
        assert!(frames[..10].iter().all(|frame| frame == &frames[0]));
        assert!(frames.iter().flatten().any(|color| *color == [254, 254, 0]));
    }

    #[test]
    fn render_expands_one_fan_at_a_time() {
        let frames = render_effect(&effect(RgbMode::Render), 2, Side::Outer);
        assert_eq!(frames.len(), 40);
        assert_eq!(frames[0][12], [254, 0, 0]);
        assert_eq!(frames[4][15], [0, 254, 0]);
        assert_eq!(frames[5][12], [0, 254, 0]);
    }

    #[test]
    fn ping_pong_runs_two_opposing_color_passes() {
        let frames = ping_pong(&effect(RgbMode::PingPong), 1, Side::Inner);
        assert_eq!(frames.len(), 26);
        assert_eq!(frames[0][0], [254, 0, 0]);
        assert_eq!(frames[13][11], [0, 254, 0]);
    }

    #[test]
    fn stack_downsamples_the_native_accumulation_sequence() {
        let frames = stack(&effect(RgbMode::Stack), 1, Side::Outer);
        assert_eq!(frames.len(), 56);
        assert_eq!(frames[0][12], [254, 0, 0]);
        assert!(frames[10][18..20].iter().all(|color| *color == [254, 0, 0]));
    }
}
