//! TURZX desktop-mode USB display protocol.
//!
//! Decoded from the Windows `LIANLI_display_driver.dll` — see `target/TURZX.md`
//! for the full protocol reference and the packet-builder provenance.

mod device;
mod edid;
mod framing;
mod vendor_caps;

pub use device::{DeviceIdentity, TurzxDisplay};
pub use edid::{build_edid, patch_edid_serial};
pub use framing::{
    build_config_packet, build_power_off, fragment_stream_a, pack_fragment, pack_frame, tlv,
};
pub use vendor_caps::{parse_vendor_desc, pick_format, pick_mode, Mode, VendorCaps};

pub const VID: u16 = 0x1A86;
pub const PID_RANGE: std::ops::RangeInclusive<u16> = 0xAD10..=0xAD3F;

pub const MAGIC: u8 = 0xAF;
pub const CTRL_OP: u8 = 0x20;

pub const FMT_MJPEG: u16 = 0x0111;
pub const FMT_H264: u16 = 0x0112;

pub const STREAM_A_FRAG: u8 = 0x6C;
pub const STREAM_A_FINAL: u8 = 0x6D;
pub const STREAM_B_FRAG: u8 = 0x68;
pub const STREAM_B_FINAL: u8 = 0x69;
pub const COMMIT: u8 = 0x66;

pub fn is_turzx(vid: u16, pid: u16) -> bool {
    vid == VID && PID_RANGE.contains(&pid)
}
