use super::engine::{brightness, palette, place, scale, Color, Side};
use lianli_shared::rgb::RgbEffect;

pub(super) fn ripple(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let half = fans * side.leds_per_track() / 2;
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    let mut frames = Vec::with_capacity(4 * (12 * fans + half));
    for color in colors {
        for step in 0..12 * fans + half {
            let mut track = vec![[0; 3]; half * 2];
            for position in 0..half {
                let lit = [0, 3, 6, 9].into_iter().any(|pulse| {
                    let start = pulse * fans + position;
                    start <= step && start + fans > step
                });
                if lit {
                    track[half - position - 1] = color;
                    track[half + position] = color;
                }
            }
            frames.push(place(&track, side, fans));
        }
    }
    frames
}

pub(super) fn collide(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let half = fans * side.leds_per_track() / 2;
    let span = half + 2 * fans - 1;
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    let mut frames = Vec::with_capacity(8 * span);
    for initial_color in 0..4 {
        let mut color = initial_color;
        for pass in 0..2 {
            for step in 0..span {
                if pass == 0 && step == half {
                    color = (color + 1) % 4;
                }
                let mut track = vec![[0; 3]; half * 2];
                for position in 0..half {
                    if position < step && position + 2 * fans > step {
                        let first = if pass == 0 {
                            position
                        } else {
                            half - position - 1
                        };
                        let second = if pass == 0 {
                            half * 2 - position - 1
                        } else {
                            half + position
                        };
                        track[first] = colors[color];
                        track[second] = colors[color];
                    }
                }
                frames.push(place(&track, side, fans));
            }
        }
    }
    frames
}

pub(super) fn reflect(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let half = fans * side.leds_per_track() / 2;
    let span = half + 2 * fans - 1;
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    let mut frames = Vec::with_capacity(8 * span);
    for color in colors {
        for pass in 0..2 {
            for step in 0..span {
                let mut track = vec![[0; 3]; half * 2];
                if pass == 0 {
                    for position in 0..half {
                        if position <= step && position + 2 * fans > step {
                            track[position] = color;
                            track[half * 2 - position - 1] = color;
                        }
                    }
                }
                frames.push(place(&track, side, fans));
            }
        }
    }
    frames
}

pub(super) fn electric_current(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let half = fans * side.leds_per_track() / 2;
    let span = half + 2 * fans - 1;
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    let mut frames = Vec::new();
    for color in colors {
        let mut delay = 0;
        for pass in 0..2 {
            let mut step = 0;
            while step < span {
                if delay < 10 {
                    delay += 1;
                    step = 0;
                }
                let mut track = vec![[0; 3]; half * 2];
                for position in 0..half {
                    if position < step && position + 2 * fans > step {
                        let first = if pass == 0 {
                            position
                        } else {
                            half - position - 1
                        };
                        let second = if pass == 0 {
                            half * 2 - position - 1
                        } else {
                            half + position
                        };
                        track[first] = color;
                        track[second] = color;
                    }
                }
                frames.push(place(&track, side, fans));
                step += if pass == 0 { 1 } else { 2 };
            }
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
            colors: vec![[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]],
            brightness: 4,
            ..Default::default()
        }
    }

    #[test]
    fn ripple_emits_four_inward_pairs_per_color() {
        let frames = ripple(&effect(RgbMode::Ripple), 1, Side::Outer);
        assert_eq!(frames.len(), 64);
        assert_eq!(frames[0][15], [254, 0, 0]);
        assert_eq!(frames[0][16], [254, 0, 0]);
        assert_eq!(frames[1][15], [0; 3]);
        assert_eq!(frames[3][15], [254, 0, 0]);
    }

    #[test]
    fn collide_changes_color_at_the_native_turnaround() {
        let frames = collide(&effect(RgbMode::Collide), 1, Side::Outer);
        assert_eq!(frames.len(), 40);
        assert!(frames[0].iter().all(|color| *color == [0; 3]));
        assert_eq!(frames[1][12], [254, 0, 0]);
        assert_eq!(frames[4][15], [0, 254, 0]);
        assert_eq!(frames[4][16], [0, 254, 0]);
    }

    #[test]
    fn reflect_keeps_the_native_blank_return_pass() {
        let frames = reflect(&effect(RgbMode::Reflect), 1, Side::Inner);
        assert_eq!(frames.len(), 56);
        assert_eq!(frames[0][0], [254, 0, 0]);
        assert_eq!(frames[0][11], [254, 0, 0]);
        assert!(frames[7..14]
            .iter()
            .all(|frame| frame.iter().all(|color| *color == [0; 3])));
    }

    #[test]
    fn electric_current_repeats_the_startup_hold_for_each_color() {
        let frames = electric_current(&effect(RgbMode::ElectricCurrent), 1, Side::Outer);
        assert_eq!(frames.len(), 68);
        assert!(frames[..10]
            .iter()
            .all(|frame| frame.iter().all(|color| *color == [0; 3])));
        assert_eq!(frames[10][12], [254, 0, 0]);
        assert_eq!(frames[10][19], [254, 0, 0]);
        assert!(frames[17..27]
            .iter()
            .all(|frame| frame.iter().all(|color| *color == [0; 3])));
    }
}
