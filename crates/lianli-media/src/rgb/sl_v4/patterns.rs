use super::engine::{brightness, palette, place, scale, Color, Side};
use lianli_shared::rgb::RgbEffect;

pub(super) fn staggered(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let len = fans * side.leds_per_track();
    let colors = palette(effect, side);
    let bright = brightness(effect);
    let mut frames = Vec::with_capacity(136);
    for phase in 0..4 {
        let mut level = 0u32;
        for frame in 0..34 {
            let color = scale(
                scale(colors[phase / 2], ((level * 15) & 0xff) as u16),
                bright,
            );
            let mut track = vec![color; len];
            let quarters = match (fans, phase.is_multiple_of(2)) {
                (1 | 2, true) => &[(2, 4)][..],
                (1 | 2, false) => &[(0, 2)][..],
                (3, true) => &[(1, 2)][..],
                (3, false) => &[(0, 1), (2, 3)][..],
                (4, true) => &[(1, 2), (3, 4)][..],
                (4, false) => &[(0, 1), (2, 3)][..],
                _ => unreachable!(),
            };
            let divisor = if fans <= 2 { 4 } else { fans };
            for &(start, end) in quarters {
                track[len * start / divisor..len * end / divisor].fill([0; 3]);
            }
            frames.push(place(&track, side, fans));
            level = if frame >= 17 { level - 1 } else { level + 1 };
        }
    }
    frames
}

pub(super) fn tide(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let len = fans * side.leds_per_track();
    let half = len / 2;
    let span = half + 2 * fans - 1;
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    let mut frames = Vec::with_capacity(span * 2);
    for pass in 0..2 {
        for step in 0..span {
            let advancing = colors[pass];
            let background = colors[1 - pass];
            let mut track = vec![[0; 3]; len];
            for position in 0..half {
                let color = if position <= step {
                    advancing
                } else {
                    background
                };
                track[position] = color;
                track[half * 2 - position - 1] = color;
            }
            frames.push(place(&track, side, fans));
        }
    }
    frames
}
