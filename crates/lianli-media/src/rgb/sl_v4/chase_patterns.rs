use super::basic::rainbow_color;
use super::engine::{brightness, palette_all, place, place_banks, scale, Color, Side};
use super::shuttle_masks::{FIRST_BANK_MASKS, SECOND_BANK_MASKS};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn pioneer(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let internal_len = [14, 26, 40, 52][fans - 1];
    let half = internal_len / 2;
    let output_len = fans * 13;
    let color = scale(palette_all(effect)[0], brightness(effect));
    let mut frames = Vec::with_capacity(2 * (half + 2 * fans - 1));
    for phase in 0..2 {
        let span = half + 2 * fans - 1;
        for step in 0..span {
            if (phase == 0 && matches!(side, Side::Outer))
                || (phase == 1 && matches!(side, Side::Inner))
            {
                let mut track = vec![[0; 3]; internal_len];
                for position in 0..half {
                    let lit = (phase == 0 && position >= step) || (phase == 1 && position < step);
                    if lit {
                        let first = if phase == 0 {
                            position
                        } else {
                            half - position - 1
                        };
                        let second = if phase == 0 {
                            internal_len - position - 1
                        } else {
                            half + position
                        };
                        track[first] = color;
                        track[second] = color;
                    }
                }
                frames.push(place(&track[..output_len], side, fans));
            } else {
                frames.push(vec![[0; 3]; fans * 52]);
            }
        }
    }
    frames
}

pub(super) fn shuttle_run(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let len = fans * 13;
    let colors = palette_all(effect).map(|color| scale(color, brightness(effect)));
    let mut frames = Vec::with_capacity(2 * (fans + 1) * 13);
    for pass in 0..2 {
        for step in 0..(fans + 1) * 13 {
            let mut first = vec![[0; 3]; len];
            let mut second = vec![[0; 3]; len];
            for position in 0..len {
                let target = if pass == 0 {
                    position
                } else {
                    len - position - 1
                };
                for (masks, track) in [
                    (FIRST_BANK_MASKS, &mut first),
                    (SECOND_BANK_MASKS, &mut second),
                ] {
                    let (ones, twos) = masks[step];
                    let label = if ones & (1 << position) != 0 {
                        Some(0)
                    } else if twos & (1 << position) != 0 {
                        Some(1)
                    } else {
                        None
                    };
                    if let Some(mut color) = label {
                        if pass == 1 && fans % 2 == 1 {
                            color = 1 - color;
                        }
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
    let (first_offset, second_offset) = match side {
        Side::Inner => (8, 16),
        Side::Outer => (0, 24),
    };
    let len = fans * side.leds_per_track();
    (0..96)
        .map(|frame| {
            let make_track = |offset: usize| {
                let mut track = Vec::with_capacity(54);
                for sample in 0..18 {
                    let color = scale(rainbow_color(frame + offset + sample, 96), bright);
                    track.extend(std::iter::repeat_n(color, 3));
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
