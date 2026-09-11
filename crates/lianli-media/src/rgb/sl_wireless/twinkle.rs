use super::engine::{brightness, palette_all, Color, Side};
use crate::rgb::effects::twinkle as twinkle_pattern;
use lianli_shared::rgb::RgbEffect;

pub(super) fn render(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    twinkle_pattern::render(
        fans * 40,
        &palette_all(effect),
        &twinkle_pattern::FAN_COLORS,
        brightness(effect),
        &[],
        |led| {
            let position = led % 40;
            let selected = match side {
                Side::Inner => position < 12 || (20..32).contains(&position),
                Side::Outer => (12..20).contains(&position) || position >= 32,
            };
            [selected.then_some(led), None]
        },
    )
}
