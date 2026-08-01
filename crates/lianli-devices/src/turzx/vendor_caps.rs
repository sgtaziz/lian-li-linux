use super::{FMT_H264, FMT_MJPEG};
use anyhow::{bail, Context, Result};

#[derive(Default, Debug, Clone)]
pub struct VendorCaps {
    pub min_w: u16,
    pub min_h: u16,
    pub max_w: u16,
    pub max_h: u16,
    pub max_transfer: u32,
    pub supports_mjpeg: bool,
    pub supports_h264: bool,
    pub mjpeg_fmt: u8,
    pub h264_fmt: u8,
    pub modes: Vec<Mode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mode {
    pub width: u16,
    pub height: u16,
    pub refresh_hz: u8,
}

pub fn parse_vendor_desc(buf: &[u8]) -> Result<VendorCaps> {
    if buf.len() < 5 {
        bail!("vendor descriptor too short ({} bytes)", buf.len());
    }
    let total_len = buf[0] as usize;
    if total_len > buf.len() {
        bail!(
            "vendor descriptor header len {} > returned {}",
            total_len,
            buf.len()
        );
    }
    if buf[1] != 0x5F {
        bail!("vendor descriptor magic {:#04x} (want 0x5F)", buf[1]);
    }
    if !(buf[2] == 0x01 || buf[2] == 0x02) {
        bail!(
            "vendor descriptor version {:02x}{:02x} (unsupported)",
            buf[2],
            buf[3]
        );
    }
    if buf[4] as usize != total_len.saturating_sub(2) {
        bail!(
            "vendor descriptor payload len {} != header-2 {}",
            buf[4],
            total_len - 2
        );
    }

    let mut caps = VendorCaps::default();
    let mut off = 5;
    while off + 3 <= total_len {
        let etype = u16::from_le_bytes([buf[off], buf[off + 1]]);
        let elen = buf[off + 2] as usize;
        let pstart = off + 3;
        let pend = pstart + elen;
        if pend > total_len {
            break;
        }
        let p = &buf[pstart..pend];
        match (etype, elen) {
            (0x0001, 4) => {
                caps.min_w = u16::from_le_bytes([p[0], p[1]]);
                caps.min_h = u16::from_le_bytes([p[2], p[3]]);
            }
            (0x0002, 4) => {
                caps.max_w = u16::from_le_bytes([p[0], p[1]]);
                caps.max_h = u16::from_le_bytes([p[2], p[3]]);
            }
            (0x0003, 4) => {
                caps.max_transfer = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
            }
            (0x0100, 4) => match p[0] {
                1 => {
                    caps.supports_mjpeg = true;
                    caps.mjpeg_fmt = p[3];
                }
                2 => {
                    caps.supports_h264 = true;
                    caps.h264_fmt = p[3] & 0x7F;
                }
                _ => {}
            },
            (0x0200, 5) => {
                let w = u16::from_le_bytes([p[0], p[1]]);
                let h = u16::from_le_bytes([p[2], p[3]]);
                let refresh = p[4] & 0x7F;
                caps.modes.push(Mode {
                    width: w,
                    height: h,
                    refresh_hz: refresh,
                });
            }
            _ => {}
        }
        off = pend;
    }
    Ok(caps)
}

pub fn pick_format(caps: &VendorCaps, forced: Option<&str>) -> Result<u16> {
    match forced.map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("mjpeg") if caps.supports_mjpeg => Ok(FMT_MJPEG),
        Some("mjpeg") => bail!("device does not advertise MJPEG support"),
        Some("h264") if caps.supports_h264 => Ok(FMT_H264),
        Some("h264") => bail!("device does not advertise H.264 support"),
        Some(other) => bail!("unknown format '{other}' (use mjpeg or h264)"),
        None if caps.supports_h264 => Ok(FMT_H264),
        None if caps.supports_mjpeg => Ok(FMT_MJPEG),
        None => bail!("device advertises no supported codec"),
    }
}

pub fn pick_mode(caps: &VendorCaps) -> Result<Mode> {
    caps.modes
        .iter()
        .copied()
        .max_by_key(|m| (m.refresh_hz as u32, m.width as u32 * m.height as u32))
        .context("device advertises no display modes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_device_descriptor() {
        let raw = [
            0x38, 0x5f, 0x01, 0x00, 0x36, 0x01, 0x00, 0x04, 0xf0, 0x00, 0xf0, 0x00, 0x02, 0x00,
            0x04, 0xff, 0x07, 0xff, 0x07, 0x03, 0x00, 0x04, 0xe0, 0xff, 0x01, 0x00, 0x00, 0x01,
            0x04, 0x01, 0x00, 0x00, 0x40, 0x00, 0x01, 0x04, 0x02, 0x00, 0x00, 0x40, 0x00, 0x02,
            0x05, 0x80, 0x07, 0xe0, 0x01, 0x3c, 0x00, 0x02, 0x05, 0x80, 0x07, 0xe0, 0x01, 0x1e,
        ];
        let caps = parse_vendor_desc(&raw).unwrap();
        assert_eq!(caps.min_w, 240);
        assert_eq!(caps.min_h, 240);
        assert_eq!(caps.max_w, 2047);
        assert_eq!(caps.max_h, 2047);
        assert_eq!(caps.max_transfer, 131040);
        assert!(caps.supports_mjpeg && caps.supports_h264);
        assert_eq!(caps.modes.len(), 2);
        assert!(caps
            .modes
            .iter()
            .any(|m| m.width == 1920 && m.height == 480 && m.refresh_hz == 60));
        assert!(caps
            .modes
            .iter()
            .any(|m| m.width == 1920 && m.height == 480 && m.refresh_hz == 30));
    }
}
