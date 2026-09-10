pub(super) use crate::rgb::case_fans::palette::{palettes, scale, Color, Frame};
use crate::rgb::case_fans::Layout;
use lianli_shared::rgb::RgbScope;
pub(super) const LAYOUT: Layout = Layout {
    front_leds: 72,
    rear_runway_frames: 164,
};
pub(super) fn frame() -> Frame {
    LAYOUT.frame()
}
pub(super) fn range(scope: RgbScope) -> std::ops::Range<usize> {
    LAYOUT.range(scope)
}
