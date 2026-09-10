use super::engine::{brightness, palette, place, scale, Color, Side};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) fn runway(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    crate::rgb::track_effects::runway(
        fans * side.leds_per_track(),
        2 * fans,
        [colors[0], colors[1]],
        |track| place(track, side, fans),
    )
}

pub(super) fn meteor(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    crate::rgb::track_effects::meteor(
        fans * side.leds_per_track(),
        &palette(effect, side),
        crate::rgb::track_effects::METEOR_TAILS[fans - 1],
        brightness(effect),
        matches!(effect.direction, RgbDirection::CounterClockwise),
        |track| place(track, side, fans),
    )
}

pub(super) fn color_cycle(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let len = fans * side.leds_per_track();
    let short_track = matches!(side, Side::Outer);
    let lit = (if short_track {
        [3, 4, 9, 13]
    } else {
        [1, 4, 8, 11]
    })[fans - 1];
    let gap = (if short_track {
        [2, 3, 4, 5]
    } else {
        [3, 4, 5, 6]
    })[fans - 1];
    let colors = palette(effect, side).map(|color| scale(color, brightness(effect)));
    let mut pattern = Vec::new();
    for color in colors.iter().take(3) {
        pattern.extend(std::iter::repeat_n(*color, lit));
        pattern.extend(std::iter::repeat_n([0; 3], gap));
        if short_track && (fans == 1 || fans == 4) {
            pattern.pop();
        }
        if short_track && fans == 2 {
            pattern.push([0; 3]);
        }
    }
    pattern.resize(len, [0; 3]);
    let clockwise = !matches!(effect.direction, RgbDirection::CounterClockwise);
    (0..len)
        .map(|shift| {
            let mut track = vec![[0; 3]; len];
            for position in 0..len {
                let target = if clockwise {
                    len - position - 1
                } else {
                    position
                };
                track[target] = pattern[(position + shift) % len];
            }
            place(&track, side, fans)
        })
        .collect()
}
