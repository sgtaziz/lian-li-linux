use lianli_shared::rgb::{RgbEffect, RgbMode};

pub(super) fn prepare(effect: &RgbEffect, bottom: bool) -> RgbEffect {
    let mut prepared = effect.clone();
    for color in &mut prepared.colors {
        *color = crate::rgb::color::limit_current(*color, 600);
    }
    if matches!(
        effect.mode,
        RgbMode::Static
            | RgbMode::Breathing
            | RgbMode::Runway
            | RgbMode::Meteor
            | RgbMode::ColorCycle
            | RgbMode::Stack
            | RgbMode::Twinkle
    ) {
        let mut palette = if bottom {
            vec![[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]]
        } else {
            vec![[255, 0, 0], [0, 0, 255], [0, 255, 0], [255, 255, 0]]
        };
        if prepared
            .colors
            .iter()
            .flatten()
            .any(|&channel| channel != 0)
        {
            for (target, color) in palette.iter_mut().zip(&prepared.colors) {
                *target = *color;
            }
        }
        prepared.colors = palette;
    }
    prepared
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_palette_current_with_repeated_channel_rounding() {
        let effect = RgbEffect {
            colors: vec![[255; 3]],
            ..Default::default()
        };
        assert_eq!(prepare(&effect, false).colors[0], [195; 3]);
    }

    #[test]
    fn unfilled_native_slots_retain_side_specific_defaults() {
        let effect = RgbEffect {
            colors: vec![[10, 20, 30]],
            ..Default::default()
        };
        assert_eq!(prepare(&effect, false).colors[1], [0, 0, 255]);
        assert_eq!(prepare(&effect, true).colors[1], [0, 255, 0]);
        let black = RgbEffect {
            colors: vec![[0; 3]; 4],
            ..Default::default()
        };
        assert_eq!(prepare(&black, false).colors[0], [255, 0, 0]);
    }
}
