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
            let mut track = vec![background; len];
            for position in 0..half {
                if position <= step {
                    track[position] = advancing;
                    track[len - position - 1] = advancing;
                }
            }
            frames.push(place(&track, side, fans));
        }
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_shared::rgb::RgbMode;

    fn effect(mode: RgbMode) -> RgbEffect {
        RgbEffect {
            mode,
            colors: vec![[255, 0, 0], [0, 255, 0]],
            brightness: 4,
            ..Default::default()
        }
    }

    #[test]
    fn staggered_alternates_fans_then_colors() {
        let frames = staggered(&effect(RgbMode::Staggered), 2, Side::Outer);
        assert_eq!(frames.len(), 136);
        assert_eq!(frames[17][12], [253, 0, 0]);
        assert_eq!(frames[17][52], [0; 3]);
        assert_eq!(frames[51][12], [0; 3]);
        assert_eq!(frames[51][52], [253, 0, 0]);
        assert_eq!(frames[85][12], [0, 253, 0]);
    }

    #[test]
    fn tide_advances_symmetrically_from_both_ends() {
        let frames = tide(&effect(RgbMode::Tide), 1, Side::Inner);
        assert_eq!(frames.len(), 14);
        assert_eq!(frames[0][0], [254, 0, 0]);
        assert_eq!(frames[0][11], [254, 0, 0]);
        assert_eq!(frames[0][1], [0, 254, 0]);
    }
}
