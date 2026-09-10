//! `family` dispatches to device renderers; `effects` contains shared logical animations.
//! Device modules own LED mappings, palette defaults, and playback timing; `color` owns color math.

pub mod animation;
#[cfg(test)]
mod capability_tests;
mod case_fans;
pub mod cl;
mod color;
mod effects;
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
pub mod universal;
pub use animation::{Animation, SecondaryTiming};

pub const FRAME_INTERVAL_MS: u16 = 50;
