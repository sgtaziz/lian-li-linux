use super::engine::{brightness, palette_all, Color, Side};
use crate::rgb::effects::twinkle as twinkle_pattern;
use lianli_shared::rgb::RgbEffect;

pub(super) fn render(effect: &RgbEffect, fans: usize, _side: Side) -> Vec<Vec<Color>> {
    twinkle_pattern::render(
        fans * 52,
        &palette_all(effect),
        &twinkle_pattern::FAN_COLORS,
        brightness(effect),
        &[],
        |led| [Some(led), Some(led + 174)],
    )
}
