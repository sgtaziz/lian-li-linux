use super::engine::{Color, Frame, Geometry};
use crate::rgb::effects::twinkle as twinkle_pattern;

pub(super) fn twinkle(geometry: Geometry, colors: &[Color; 6], brightness: u8) -> Vec<Frame> {
    twinkle_pattern::render(
        geometry.led_count,
        colors,
        &twinkle_pattern::STRIMER_COLORS,
        u16::from(brightness),
        &[(172, 199)],
        |led| [Some(led), None],
    )
}
