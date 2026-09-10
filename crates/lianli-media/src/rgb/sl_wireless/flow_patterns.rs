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
                    frames.push(vec![[0; 3]; fans * 40]);
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
                    frames.push(vec![[0; 3]; fans * 40]);
                }
            }
        }
    }
    frames
}

pub(super) fn river(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let len = fans * side.leds_per_track();
    let run = len.div_ceil(8);
    let colors = palette_all(effect).map(|color| scale(color, brightness(effect)));
    let reverse = !matches!(effect.direction, RgbDirection::CounterClockwise);
    let pattern: Vec<Color> = (0..len)
        .map(|position| colors[usize::from(position % (len / 2) < run)])
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
    let half = fans * side.leds_per_track() / 2;
    let len = half * 2;
    let span = half + 2 * fans - 1;
    let colors = palette_all(effect).map(|color| scale(color, brightness(effect)));
    let blank = vec![[0; 3]; len];
    let mut frames = Vec::with_capacity(4 * span + 18);
    for bank in 0..2 {
        let mut delay = 0;
        for pass in 0..2 {
            let mut step = 0;
            while step < span {
                if delay < 10 {
                    delay += 1;
                    step = 0;
                }
                let mut track = vec![[0; 3]; len];
                for position in 0..half {
                    if position < step && position + 2 * fans > step {
                        let first = if pass == 0 {
                            position
                        } else {
                            half - position - 1
                        };
                        let second = if pass == 0 {
                            len - position - 1
                        } else {
                            half + position
                        };
                        track[first] = colors[0];
                        track[second] = colors[1];
                    }
                }
                let frame = if bank == 0 {
                    place_banks(&track, &blank, side, fans)
                } else {
                    place_banks(&blank, &track, side, fans)
                };
                frames.push(frame);
                step += 1;
            }
        }
    }
    frames
}

pub(super) fn hourglass(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let outer_half = fans * Side::Outer.leds_per_track() / 2;
    let inner_half = fans * Side::Inner.leds_per_track() / 2;
    let colors = palette_all(effect).map(|color| scale(color, brightness(effect)));
    let mut frames = Vec::new();
    for color in colors {
        let mut inner_scope = vec![[0; 3]; inner_half * 2];
        for center_width in 0..3 {
            for step in 0..=outer_half {
                let mut outer_scope = vec![[0; 3]; outer_half * 2];
                for position in 0..outer_half {
                    if position < step {
                        outer_scope[position] = color;
                        outer_scope[outer_half * 2 - position - 1] = color;
                    }
                }
                if step == outer_half {
                    inner_scope[inner_half + center_width] = color;
                    inner_scope[inner_half - center_width - 1] = color;
                }
                frames.push(match side {
                    Side::Outer => place(&outer_scope, side, fans),
                    Side::Inner => place(&inner_scope, side, fans),
                });
            }
        }
        let edge = inner_half - 3;
        for step in 0..edge {
            for position in 0..edge {
                let lit = if position < step { color } else { [0; 3] };
                inner_scope[edge - position - 1] = lit;
                inner_scope[inner_half + 3 + position] = lit;
            }
            frames.push(match side {
                Side::Outer => vec![[0; 3]; fans * 40],
                Side::Inner => place(&inner_scope, side, fans),
            });
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
    fn endless_keeps_inner_and_outer_phases_in_one_timeline() {
        let frames = endless(&effect(RgbMode::Endless), 1, Side::Outer);
        assert_eq!(frames.len(), 80);
        assert!(frames[0].iter().all(|color| *color == [0; 3]));
        assert_eq!(frames[1][19], [254, 0, 0]);
        assert!(frames[8..20]
            .iter()
            .all(|frame| frame.iter().all(|color| *color == [0; 3])));
    }

    #[test]
    fn river_uses_two_native_runs_and_mirrors_the_second_outer_bank() {
        let frames = river(&effect(RgbMode::River), 1, Side::Inner);
        assert_eq!(frames.len(), 12);
        assert_eq!(frames[0][10], [0, 254, 0]);
        assert_eq!(frames[0][11], [0, 254, 0]);
        assert_eq!(frames[0][20], [0, 254, 0]);
        assert_eq!(frames[0][21], [0, 254, 0]);
        assert_eq!(frames[0][30], [254, 0, 0]);
    }

    #[test]
    fn duel_runs_one_physical_bank_at_a_time() {
        let frames = duel(&effect(RgbMode::Duel), 1, Side::Outer);
        assert_eq!(frames.len(), 38);
        assert!(frames[..10]
            .iter()
            .all(|frame| frame.iter().all(|color| *color == [0; 3])));
        assert_eq!(frames[10][12], [254, 0, 0]);
        assert_eq!(frames[10][19], [0, 254, 0]);
        assert_eq!(frames[10][32], [0; 3]);
        assert_eq!(frames[29][32], [254, 0, 0]);
    }

    #[test]
    fn hourglass_moves_from_inner_edges_into_the_outer_center() {
        let outer = hourglass(&effect(RgbMode::Hourglass), 1, Side::Outer);
        let inner = hourglass(&effect(RgbMode::Hourglass), 1, Side::Inner);
        assert_eq!(inner.len(), 72);
        assert_eq!(outer.len(), 72);
        assert_eq!(outer[4][12], [254, 0, 0]);
        assert_eq!(outer[4][19], [254, 0, 0]);
        assert_eq!(inner[4][5], [254, 0, 0]);
        assert_eq!(inner[4][6], [254, 0, 0]);
        assert_eq!(inner[14][3..9], [[254, 0, 0]; 6]);
    }
}
