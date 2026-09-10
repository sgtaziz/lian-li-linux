use super::basic::rainbow_color;
use super::engine::{brightness, palette_all, place, place_banks, scale, Color, Side};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn pioneer(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let outer_half = fans * Side::Outer.leds_per_track() / 2;
    let inner_half = fans * Side::Inner.leds_per_track() / 2;
    let color = scale(palette_all(effect)[0], brightness(effect));
    let mut frames = Vec::with_capacity(outer_half + inner_half + 4 * fans - 2);
    for (phase, half) in [outer_half, inner_half].into_iter().enumerate() {
        let span = half + 2 * fans - 1;
        for step in 0..span {
            if (phase == 0 && matches!(side, Side::Outer))
                || (phase == 1 && matches!(side, Side::Inner))
            {
                let mut track = vec![[0; 3]; half * 2];
                for position in 0..half {
                    let lit = (phase == 0 && position >= step) || (phase == 1 && position < step);
                    if lit {
                        let first = if phase == 0 {
                            position
                        } else {
                            half - position - 1
                        };
                        let second = if phase == 0 {
                            half * 2 - position - 1
                        } else {
                            half + position
                        };
                        track[first] = color;
                        track[second] = color;
                    }
                }
                frames.push(place(&track, side, fans));
            } else {
                frames.push(vec![[0; 3]; fans * 40]);
            }
        }
    }
    frames
}

pub(super) fn shuttle_run(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let per_fan = side.leds_per_track();
    let len = fans * per_fan;
    let width = per_fan / 2;
    let colors = palette_all(effect).map(|color| scale(color, brightness(effect)));
    let mut frames = Vec::with_capacity(2 * (len + per_fan - 1));
    for pass in 0..2 {
        for step in 0..len + per_fan - 1 {
            let mut first = vec![[0; 3]; len];
            let mut second = vec![[0; 3]; len];
            for position in 0..len {
                for (bank, track) in [&mut first, &mut second].into_iter().enumerate() {
                    let even = (position / per_fan).is_multiple_of(2);
                    let label = if (bank == 0) == even { 2 } else { 1 };
                    let delay = if label == 2 { width } else { 0 };
                    if position + delay <= step && position + delay + width > step {
                        let swap = pass == 1 && fans % 2 == 1;
                        let color = usize::from((label == 2) ^ swap);
                        let target = if pass == 0 {
                            position
                        } else {
                            len - position - 1
                        };
                        track[target] = colors[color];
                    }
                }
            }
            frames.push(place_banks(&first, &second, side, fans));
        }
    }
    frames
}

pub(super) fn gradient_ribbon(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let bright = brightness(effect);
    let clockwise = matches!(effect.direction, RgbDirection::Clockwise);
    let (repeat, first_offset, second_offset) = match side {
        Side::Inner => (3, 8, 16),
        Side::Outer => (2, 0, 24),
    };
    let len = fans * side.leds_per_track();
    (0..96)
        .map(|frame| {
            let make_track = |offset: usize| {
                let mut track = Vec::with_capacity(16 * repeat);
                for sample in 0..16 {
                    let color = scale(rainbow_color(frame + offset + sample, 96), bright);
                    track.extend(std::iter::repeat_n(color, repeat));
                }
                if clockwise {
                    track.reverse();
                }
                track
            };
            let first = make_track(first_offset);
            let second = make_track(second_offset);
            place_banks(&first[..len], &second[..len], side, fans)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::twinkle::render as twinkle;
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
    fn pioneer_shrinks_outer_then_grows_inner() {
        let outer = pioneer(&effect(RgbMode::Pioneer), 1, Side::Outer);
        let inner = pioneer(&effect(RgbMode::Pioneer), 1, Side::Inner);
        assert_eq!(outer.len(), 12);
        assert_eq!(outer[0][12..20], [[254, 0, 0]; 8]);
        assert!(outer[5..].iter().all(|frame| frame[12..20] == [[0; 3]; 8]));
        assert!(inner[..5].iter().all(|frame| frame[..12] == [[0; 3]; 12]));
        assert_eq!(inner[6][5], [254, 0, 0]);
        assert_eq!(inner[6][6], [254, 0, 0]);
    }

    #[test]
    fn shuttle_run_preserves_the_two_source_bank_patterns() {
        let frames = shuttle_run(&effect(RgbMode::ShuttleRun), 1, Side::Outer);
        assert_eq!(frames.len(), 30);
        assert_eq!(frames[0][12], [0; 3]);
        assert_eq!(frames[0][32], [254, 0, 0]);
        assert_eq!(frames[4][12], [0, 254, 0]);
        assert_eq!(frames[4][33], [254, 0, 0]);
    }

    #[test]
    fn gradient_ribbon_uses_native_bank_offsets_and_direction() {
        let mut effect = effect(RgbMode::GradientRibbon);
        let clockwise = gradient_ribbon(&effect, 1, Side::Inner);
        effect.direction = RgbDirection::CounterClockwise;
        let counter = gradient_ribbon(&effect, 1, Side::Inner);
        assert_eq!(clockwise.len(), 96);
        assert_eq!(clockwise[0][0], [71, 183, 0]);
        assert_eq!(counter[0][0], [191, 63, 0]);
        assert_eq!(counter[0][20], [127, 127, 0]);
    }

    #[test]
    fn twinkle_matches_extracted_sparse_source_columns() {
        let frames = twinkle(&effect(RgbMode::Twinkle), 4, Side::Outer);
        assert_eq!(frames.len(), 200);
        assert_eq!(frames[35][157], [3, 3, 0]);
        assert_eq!(frames[129][104], [0, 0, 0]);
        assert!(frames.iter().any(|frame| frame[158] != [0; 3]));
    }
}
