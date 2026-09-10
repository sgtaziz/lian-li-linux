pub mod animation;
#[cfg(test)]
mod capability_tests;
mod case_fans;
pub mod cl;
pub mod family;
pub mod h2;
pub mod hs2_oled;
pub mod lancool217;
pub mod lancool_v150;
pub mod p28;
pub mod parameters;
#[cfg(test)]
mod screen_palette_tests;
pub mod sl_inf;
pub mod sl_v4;
pub mod sl_wireless;
pub mod strimer;
pub mod sync_effects;
pub mod sync_layout;
pub mod tl;
mod track_effects;
mod twinkle_pattern;
pub mod universal;
pub use animation::{Animation, SecondaryTiming};

pub const FRAME_INTERVAL_MS: u16 = 50;
