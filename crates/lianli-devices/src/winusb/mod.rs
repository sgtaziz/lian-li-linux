//! Shared WinUSB transport for VID=0x1CBE LCD panels and LED rings.
//!
//! - [`lcd`] — generic WinUSB LCD driver (DES-CBC encrypted headers + JPEG/H.264).
//! - [`led`] — generic WinUSB LED driver for addressable RGB rings.

pub mod lcd;
pub mod led;
