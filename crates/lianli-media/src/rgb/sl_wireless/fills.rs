use super::engine::{brightness, clamp_current, palette, place, scale, Color, Side};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn mixing(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let colors = palette(effect, side);
    let mixed = clamp_current(std::array::from_fn(|channel| {
        colors[0][channel].saturating_add(colors[1][channel])
    }));
    crate::rgb::effects::fills::mixing(
        fans * side.leds_per_track(),
        2 * fans,
        colors,
        mixed,
        brightness(effect),
        |track| place(track, side, fans),
    )
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
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    crate::rgb::effects::chase::ping_pong(
        fans * side.leds_per_track(),
        fans,
        [colors[0], colors[1]],
        |track| place(track, side, fans),
    )
}

pub(super) fn stack(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    let source = crate::rgb::effects::fills::stack(
        fans * side.leds_per_track(),
        if matches!(side, Side::Outer) { 2 } else { 3 },
        4 * fans,
        colors,
        effect.direction == RgbDirection::CounterClockwise,
        |track| place(track, side, fans),
    );
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
