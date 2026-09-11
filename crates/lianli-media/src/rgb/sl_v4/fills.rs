use super::engine::{brightness, clamp_current, palette, place, scale, Color, Side};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn mixing(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let colors = palette(effect, side);
    let mixed = clamp_current(std::array::from_fn(|channel| {
        colors[0][channel].saturating_add(colors[1][channel])
    }));
    crate::rgb::effects::fills::mixing(
        side.mirrored_track_len(fans),
        2 * fans,
        colors,
        mixed,
        brightness(effect),
        |track| place(&track[..fans * 13], side, fans),
    )
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
        if fans == 4 { 4 } else { 3 },
        [4, 9, 13, 13][fans - 1],
        colors,
        effect.direction == RgbDirection::CounterClockwise,
        |track| place(track, side, fans),
    );
    let output_len = source.len() / 3;
    (0..output_len)
        .map(|frame| source[frame * 2].clone())
        .collect()
}
