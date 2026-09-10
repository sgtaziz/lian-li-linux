use super::engine::{brightness, palette, place, scale, Color, Side};
use lianli_shared::rgb::RgbEffect;

pub(super) fn ripple(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let len = fans * side.leds_per_track();
    let half = len / 2;
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    let mut frames = Vec::with_capacity(4 * (12 * fans + half));
    for color in colors {
        for step in 0..12 * fans + half {
            let mut track = vec![[0; 3]; len];
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
    let internal_len = [14, 26, 40, 52][fans - 1];
    let half = internal_len / 2;
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
                let mut track = vec![[0; 3]; internal_len];
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
                frames.push(place(&track[..fans * 13], side, fans));
            }
        }
    }
    frames
}

pub(super) fn reflect(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let internal_len = [14, 26, 40, 52][fans - 1];
    let half = internal_len / 2;
    let span = half + 2 * fans - 1;
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    let mut frames = Vec::with_capacity(8 * span);
    for color in colors {
        for pass in 0..2 {
            for step in 0..span {
                let mut track = vec![[0; 3]; internal_len];
                if pass == 0 {
                    for position in 0..half {
                        if position <= step && position + 2 * fans > step {
                            track[position] = color;
                            track[half * 2 - position - 1] = color;
                        }
                    }
                }
                frames.push(place(&track[..fans * 13], side, fans));
            }
        }
    }
    frames
}

pub(super) fn electric_current(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let internal_len = [14, 26, 40, 52][fans - 1];
    let half = internal_len / 2;
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
                let mut track = vec![[0; 3]; internal_len];
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
                frames.push(place(&track[..fans * 13], side, fans));
                step += if pass == 0 { 1 } else { 2 };
            }
        }
    }
    frames
}
