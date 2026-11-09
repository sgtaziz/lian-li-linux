//! Shared WinUSB transport for VID=0x1CBE LCD panels and LED rings.
//!
//! - [`lcd`] — generic WinUSB LCD driver (DES-CBC encrypted headers + JPEG/H.264).
//! - [`led`] — generic WinUSB LED driver for addressable RGB rings.
//! - [`h2_aio`] — HydroShift II AIO pump/fan controller (SyncPumpFan opcode).

pub mod h2_aio;
pub mod hs2_oled_led;
pub mod lcd;
pub mod led;
pub mod wired_receiver;

fn nul_terminated_text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}
