use super::engine::{brightness, palette_all, place, place_banks, scale, Color, Side};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn endless(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let outer_len = fans * Side::Outer.leds_per_track();
    let inner_len = fans * Side::Inner.leds_per_track();
    let colors = palette_all(effect).map(|color| scale(color, brightness(effect)));
    let mut frames = Vec::with_capacity(4 * (inner_len + outer_len));
    for color in colors.into_iter().take(2) {
        for pass in 0..2 {
            for step in 0..outer_len {
                if matches!(side, Side::Outer) {
                    let mut track = vec![[0; 3]; outer_len];
                    for position in 0..outer_len {
                        if position < step {
                            let target = if pass == 0 {
                                outer_len - position - 1
                            } else {
                                position
                            };
                            track[target] = color;
                        }
                    }
                    frames.push(place(&track, side, fans));
                } else {
                    frames.push(vec![[0; 3]; fans * 52]);
                }
            }
            for step in 0..inner_len {
                if matches!(side, Side::Inner) {
                    let mut track = vec![[0; 3]; inner_len];
                    for position in 0..inner_len {
                        if position < step {
                            let target = if pass == 0 {
                                position
                            } else {
                                inner_len - position - 1
                            };
                            track[target] = color;
                        }
                    }
                    frames.push(place(&track, side, fans));
                } else {
                    frames.push(vec![[0; 3]; fans * 52]);
                }
            }
        }
    }
    frames
}

pub(super) fn river(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let len = fans * side.leds_per_track();
    let colors = palette_all(effect).map(|color| scale(color, brightness(effect)));
    let reverse = !matches!(effect.direction, RgbDirection::CounterClockwise);
    let pattern: Vec<Color> = (0..len)
        .map(|position| {
            let lit = match fans {
                1 => position < 2 || (6..9).contains(&position),
                2 => position < 4 || (13..17).contains(&position),
                3 => position < 6 || (19..26).contains(&position),
                4 => position < 7 || (26..33).contains(&position),
                _ => unreachable!(),
            };
            colors[usize::from(lit)]
        })
        .collect();
    (0..len)
        .map(|shift| {
            let mut track = vec![[0; 3]; len];
            for position in 0..len {
                let target = if reverse {
                    len - position - 1
                } else {
                    position
                };
                track[target] = pattern[(position + shift) % len];
            }
            if matches!(side, Side::Inner) {
                let mirrored: Vec<Color> = track.iter().copied().rev().collect();
                place_banks(&track, &mirrored, side, fans)
            } else {
                place(&track, side, fans)
            }
        })
        .collect()
}

pub(super) fn duel(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let internal_len = [14, 26, 40, 52][fans - 1];
    let half = internal_len / 2;
    let span = half + 2 * fans - 1;
    let colors = palette_all(effect).map(|color| scale(color, brightness(effect)));
    let output_len = fans * 13;
    let blank = vec![[0; 3]; output_len];
    let mut frames = Vec::with_capacity(4 * span);
    for bank in 0..2 {
        for pass in 0..2 {
            for step in 0..span {
                let mut track = vec![[0; 3]; internal_len];
                for position in 0..half {
                    if position < step && position + 2 * fans > step {
                        let first = if pass == 0 {
                            position
                        } else {
                            half - position - 1
                        };
                        let second = if pass == 0 {
                            internal_len - position - 1
                        } else {
                            half + position
                        };
                        track[first] = colors[0];
                        track[second] = colors[1];
                    }
                }
                let frame = if bank == 0 {
                    place_banks(&track[..output_len], &blank, side, fans)
                } else {
                    place_banks(&blank, &track[..output_len], side, fans)
                };
                frames.push(frame);
            }
        }
    }
    frames
}

pub(super) fn hourglass(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let internal_len = [14, 26, 40, 52][fans - 1];
    let half = internal_len / 2;
    let output_len = fans * 13;
    let colors = palette_all(effect).map(|color| scale(color, brightness(effect)));
    let mut frames = Vec::new();
    for color in colors {
        let mut inner_scope = vec![[0; 3]; internal_len];
        for center_width in 0..3 {
            for step in 0..=half {
                let mut outer_scope = vec![[0; 3]; internal_len];
                for position in 0..half {
                    if position < step {
                        outer_scope[position] = color;
                        outer_scope[internal_len - position - 1] = color;
                    }
                }
                if step == half {
                    inner_scope[half + center_width] = color;
                    inner_scope[half - center_width - 1] = color;
                }
                frames.push(match side {
                    Side::Outer => place(&outer_scope[..output_len], side, fans),
                    Side::Inner => place(&inner_scope[..output_len], side, fans),
                });
            }
        }
        let edge = half - 3;
        for step in 0..edge {
            for position in 0..edge {
                let lit = if position < step { color } else { [0; 3] };
                inner_scope[edge - position - 1] = lit;
                inner_scope[half + 3 + position] = lit;
            }
            frames.push(match side {
                Side::Outer => vec![[0; 3]; fans * 52],
                Side::Inner => place(&inner_scope[..output_len], side, fans),
            });
        }
    }
    frames
}
