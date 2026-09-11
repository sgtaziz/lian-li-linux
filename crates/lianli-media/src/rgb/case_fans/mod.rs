pub(super) mod color_sweeps;
pub(super) mod composition;
pub(super) mod palette;
pub(crate) mod parameters;
pub(super) mod paths;
pub(super) mod twinkle;

use lianli_shared::rgb::RgbScope;
use palette::Frame;

#[derive(Clone, Copy)]
pub(crate) struct Layout {
    pub front_leds: usize,
    pub rear_runway_frames: usize,
}

impl Layout {
    pub const fn led_count(self) -> usize {
        self.front_leds + 16
    }

    pub fn frame(self) -> Frame {
        vec![[0; 3]; self.led_count()]
    }

    pub fn range(self, scope: RgbScope) -> std::ops::Range<usize> {
        match scope {
            RgbScope::Front => 0..self.front_leds,
            RgbScope::Rear => self.front_leds..self.led_count(),
            _ => unreachable!("validated case fan scope"),
        }
    }
}
pub(super) mod classic;
