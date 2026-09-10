pub mod animation;
#[cfg(test)]
mod capability_tests;
pub mod cl;
pub mod family;
pub mod h2;
pub mod hs2_oled;
pub mod lancool217;
pub mod lancool_v150;
pub mod p28;
pub mod parameters;
pub mod sl_inf;
pub mod sl_v4;
pub mod sl_wireless;
pub mod strimer;
pub mod tl;
pub mod universal;
pub use animation::{Animation, SecondaryTiming};

pub const FRAME_INTERVAL_MS: u16 = 50;
