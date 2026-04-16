//! TURZX desktop-mode USB display protocol.
//!
//! Decoded from the Windows `LIANLI_display_driver.dll` — see `target/TURZX.md`
//! for the full protocol reference and the packet-builder provenance.

use anyhow::{bail, Context, Result};
use lianli_transport::usb::{UsbTransport, LCD_READ_TIMEOUT, LCD_WRITE_TIMEOUT};
use std::time::Duration;
use tracing::{debug, warn};

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
pub const STREAM_C: u8 = 0x6B;
pub const COMMIT: u8 = 0x66;

const READY_POLL_ATTEMPTS: usize = 100;
const READY_POLL_INTERVAL: Duration = Duration::from_millis(100);
const READY_STATUS_BIT: u8 = 0x10;

pub fn is_turzx(vid: u16, pid: u16) -> bool {
    vid == VID && PID_RANGE.contains(&pid)
}

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
        bail!("vendor descriptor header len {} > returned {}", total_len, buf.len());
    }
    if buf[1] != 0x5F {
        bail!("vendor descriptor magic {:#04x} (want 0x5F)", buf[1]);
    }
    if buf[2] != 0x01 || buf[3] != 0x00 {
        bail!("vendor descriptor version {:02x}{:02x} (want 01 00)", buf[2], buf[3]);
    }
    if buf[4] as usize != total_len.saturating_sub(2) {
        bail!("vendor descriptor payload len {} != header-2 {}", buf[4], total_len - 2);
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
                let flag = (p[4] & 0x80) != 0;
                caps.modes.push(Mode { width: w, height: h, refresh_hz: refresh });
                if !flag && refresh == 0x1E {
                    caps.modes.push(Mode { width: w, height: h, refresh_hz: 0x3C });
                }
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

pub fn tlv(buf: &mut Vec<u8>, sub_op: u8, value: u8) {
    buf.extend_from_slice(&[MAGIC, CTRL_OP, sub_op, value]);
}

pub fn build_config_packet(width: u16, height: u16, format: u16) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(28);
    tlv(&mut pkt, 0x00, 0x01);
    tlv(&mut pkt, 0x01, (width >> 8) as u8);
    tlv(&mut pkt, 0x02, width as u8);
    tlv(&mut pkt, 0x03, (height >> 8) as u8);
    tlv(&mut pkt, 0x04, height as u8);
    tlv(&mut pkt, format as u8, (format >> 8) as u8);
    tlv(&mut pkt, 0x1F, 0x01);
    pkt
}

pub fn build_power_off() -> [u8; 4] {
    [MAGIC, CTRL_OP, 0x1F, 0x02]
}

fn write_header(buf: &mut Vec<u8>, opcode: u8, offset: u32, size: u32) {
    buf.push(MAGIC);
    buf.push(opcode);
    buf.push(((offset >> 16) & 0xFF) as u8);
    buf.push(((offset >> 8) & 0xFF) as u8);
    buf.push((offset & 0xFF) as u8);
    buf.push(((size >> 16) & 0xFF) as u8);
    buf.push(((size >> 8) & 0xFF) as u8);
    buf.push((size & 0xFF) as u8);
}

pub fn pack_frame(opcode: u8, offset: u32, payload: &[u8]) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(payload.len() + 10);
    write_header(&mut pkt, opcode, offset, payload.len() as u32);
    pkt.extend_from_slice(payload);
    pkt.push(MAGIC);
    pkt.push(COMMIT);
    pkt
}

pub fn pack_fragment(opcode: u8, offset: u32, payload: &[u8]) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(payload.len() + 8);
    write_header(&mut pkt, opcode, offset, payload.len() as u32);
    pkt.extend_from_slice(payload);
    pkt
}

/// Split a stream A payload into one or more URB-sized buffers following the
/// driver's fragmentation rules: intermediate fragments use opcode 0x6C with
/// no commit trailer; the last URB uses opcode 0x6D and carries the
/// `0xAF 0x66` commit marker.
pub fn fragment_stream_a(packet: &[u8], urb_max: usize) -> Vec<Vec<u8>> {
    let urb_max = urb_max.max(16);
    let budget_final = urb_max.saturating_sub(10);
    let budget_frag = urb_max.saturating_sub(8);

    if packet.len() <= budget_final {
        return vec![pack_frame(STREAM_A_FINAL, 0, packet)];
    }
    let mut out = Vec::new();
    let mut offset = 0usize;
    while offset < packet.len() {
        let remaining = packet.len() - offset;
        if remaining <= budget_final {
            out.push(pack_frame(STREAM_A_FINAL, offset as u32, &packet[offset..]));
            return out;
        }
        let chunk = budget_frag;
        out.push(pack_fragment(
            STREAM_A_FRAG,
            offset as u32,
            &packet[offset..offset + chunk],
        ));
        offset += chunk;
    }
    out
}

pub struct TurzxDisplay {
    transport: UsbTransport,
    pid: u16,
    caps: VendorCaps,
    edid: [u8; 128],
    streaming: bool,
    identity: DeviceIdentity,
}

/// Stable-ish identity for a single TURZX device, used to distinguish
/// multiple identical units in the compositor via EDID serial injection.
#[derive(Debug, Clone)]
pub struct DeviceIdentity {
    pub usb_serial: Option<String>,
    pub port_path: String,
    pub edid_serial: u32,
}

impl TurzxDisplay {
    pub fn open(pid: u16) -> Result<Self> {
        let mut transport =
            UsbTransport::open(VID, pid).with_context(|| format!("opening {VID:04x}:{pid:04x}"))?;
        if let Err(e) = transport.reset() {
            warn!("TURZX {VID:04x}:{pid:04x} reset failed (continuing): {e}");
        }
        std::thread::sleep(Duration::from_millis(300));
        transport
            .detach_and_configure(&format!("turzx-{pid:04x}"))
            .context("claiming interface 0")?;

        let identity = resolve_identity(&transport, pid);
        debug!(
            "TURZX {pid:04x} identity: usb_serial={:?} port={} edid_serial={:#010x}",
            identity.usb_serial, identity.port_path, identity.edid_serial
        );

        let _ = transport.write(&build_power_off(), LCD_WRITE_TIMEOUT);
        std::thread::sleep(Duration::from_millis(100));

        let mut this = Self {
            transport,
            pid,
            caps: VendorCaps::default(),
            edid: [0u8; 128],
            streaming: false,
            identity,
        };
        this.init()?;
        Ok(this)
    }

    fn init(&mut self) -> Result<()> {
        let mut buf = vec![0u8; 512];
        let n = self
            .transport
            .control_in(0x81, 0x06, 0x5F00, 0, &mut buf, LCD_READ_TIMEOUT)
            .context("reading vendor mode descriptor")?;
        buf.truncate(n);
        self.caps = parse_vendor_desc(&buf).context("parsing vendor descriptor")?;
        debug!("TURZX {VID:04x}:{:04x} caps: {:?}", self.pid, self.caps);

        let mut status = [0u8; 1];
        let mut ready = false;
        for _ in 0..READY_POLL_ATTEMPTS {
            let n = self
                .transport
                .control_in(0xC1, 0x01, 0, 0, &mut status, LCD_READ_TIMEOUT)
                .context("status poll")?;
            if n >= 1 && (status[0] & READY_STATUS_BIT) != 0 {
                ready = true;
                break;
            }
            std::thread::sleep(READY_POLL_INTERVAL);
        }
        if !ready {
            bail!("TURZX device never reported ready (bit 0x10 never set)");
        }

        let mut raw_edid = [0u8; 128];
        let n = self
            .transport
            .control_in(0xC1, 0x02, 0, 0, &mut raw_edid, LCD_READ_TIMEOUT)
            .context("reading EDID")?;
        if n != 128 {
            warn!("TURZX EDID returned {n} bytes (expected 128) — ignoring");
        } else {
            debug!("TURZX {:04x} raw device EDID captured (discarded — invalid DTDs)", self.pid);
        }

        // The device ships an EDID with broken Detailed Timing Descriptors
        // (H sync pulse > H blanking) which DRM rejects, leaving the
        // connector with zero usable modes. Build a clean one from the
        // vendor descriptor's advertised modes instead.
        self.edid = build_edid(&self.caps, self.identity.edid_serial);
        Ok(())
    }

    pub fn caps(&self) -> &VendorCaps {
        &self.caps
    }

    pub fn edid(&self) -> &[u8; 128] {
        &self.edid
    }

    pub fn pid(&self) -> u16 {
        self.pid
    }

    pub fn identity(&self) -> &DeviceIdentity {
        &self.identity
    }

    pub fn start_streaming(&mut self, mode: Mode, format: u16) -> Result<()> {
        let pkt = build_config_packet(mode.width, mode.height, format);
        self.transport
            .write(&pkt, LCD_WRITE_TIMEOUT)
            .context("writing start-config packet")?;
        self.streaming = true;
        Ok(())
    }

    /// Send a single JPEG frame as stream B (opcode 0x69), commit included.
    pub fn send_jpeg_frame(&self, jpeg: &[u8]) -> Result<()> {
        let pkt = pack_frame(STREAM_B_FINAL, 0, jpeg);
        self.transport
            .write(&pkt, LCD_WRITE_TIMEOUT)
            .context("writing JPEG frame")?;
        Ok(())
    }

    pub fn send_stream_a(&self, packet: &[u8]) -> Result<()> {
        let advertised = self.caps.max_transfer.max(512) as usize;
        let urb_max = advertised.min(32 * 1024);
        for urb in fragment_stream_a(packet, urb_max) {
            write_full(&self.transport, &urb).context("writing stream A URB")?;
        }
        Ok(())
    }

    pub fn send_power_off(&mut self) -> Result<()> {
        let off = build_power_off();
        let res = self.transport.write(&off, LCD_WRITE_TIMEOUT);
        self.streaming = false;
        res.context("writing power-off packet")?;
        Ok(())
    }
}

const STREAM_WRITE_TIMEOUT: Duration = Duration::from_millis(1_000);

fn write_full(transport: &UsbTransport, data: &[u8]) -> Result<()> {
    let mut offset = 0usize;
    while offset < data.len() {
        let n = transport
            .write(&data[offset..], STREAM_WRITE_TIMEOUT)
            .with_context(|| format!("usb bulk write at offset {offset}/{}", data.len()))?;
        if n == 0 {
            bail!("zero-byte USB write at offset {offset}/{}", data.len());
        }
        offset += n;
    }
    Ok(())
}

pub fn patch_edid_serial(edid: &mut [u8; 128], serial: u32) {
    edid[12..16].copy_from_slice(&serial.to_le_bytes());
    let sum: u32 = edid[..127].iter().map(|&b| b as u32).sum();
    edid[127] = (0u8).wrapping_sub((sum & 0xFF) as u8);
}

/// Build a DRM-valid 128-byte EDID 1.4 block from a device's vendor-descriptor
/// capabilities.
///
/// The device's own EDID ships with broken Detailed Timing Descriptors (H
/// sync pulse exceeds H blanking, etc.) that Linux/DRM rejects — so it
/// exposes zero modes on the connector and compositors can't mode-set.
/// We synthesize fresh DTDs using reduced-blanking CEA-style timings that
/// match the advertised pixel dimensions and refresh rates.
pub fn build_edid(caps: &VendorCaps, serial: u32) -> [u8; 128] {
    let mut edid = [0u8; 128];
    // Header magic.
    edid[..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
    // Manufacturer "TUR" (bytes 8-9) — 3 letters × 5 bits, big-endian.
    edid[8] = 0x52;
    edid[9] = 0xB2;
    // Product code (LE) — distinct from device's 0x0000 so the synthetic EDID
    // is identifiable in logs.
    edid[10] = 0x01;
    edid[11] = 0x00;
    // Serial number (LE).
    edid[12..16].copy_from_slice(&serial.to_le_bytes());
    edid[16] = 1;   // mfg week
    edid[17] = 35;  // mfg year = 1990 + 35 = 2025
    edid[18] = 1;
    edid[19] = 4;   // EDID 1.4

    // Basic display parameters (digital, 8 bpc, HDMI-a).
    edid[20] = 0xA5;
    // Claim a modest physical size in cm (bytes 21-22). Rough 1920×480 aspect.
    edid[21] = 48;
    edid[22] = 12;
    edid[23] = 120; // gamma 2.2
    // Feature support: digital display, active off, no standby/suspend,
    // continuous frequency, sRGB, native format = first DTD, preferred mode.
    edid[24] = 0x06;
    // Chromaticity coords — copy standard sRGB.
    edid[25..35].copy_from_slice(&[0xEE, 0x95, 0xA3, 0x54, 0x4C, 0x99, 0x26, 0x0F, 0x50, 0x54]);
    // Established timings — none.
    edid[35] = 0;
    edid[36] = 0;
    edid[37] = 0;
    // Standard timings (16 bytes) — all "unused" sentinel.
    for i in 0..8 {
        edid[38 + i * 2] = 0x01;
        edid[38 + i * 2 + 1] = 0x01;
    }

    // Deduplicate modes preserving order, highest-refresh first for DTD slot 0.
    let mut modes: Vec<Mode> = Vec::new();
    for m in &caps.modes {
        if !modes.iter().any(|u| {
            u.width == m.width && u.height == m.height && u.refresh_hz == m.refresh_hz
        }) {
            modes.push(*m);
        }
    }
    modes.sort_by(|a, b| {
        b.refresh_hz
            .cmp(&a.refresh_hz)
            .then((b.width as u32 * b.height as u32).cmp(&(a.width as u32 * a.height as u32)))
    });

    // 4 detailed descriptor slots of 18 bytes starting at offset 54.
    // First N get DTDs; then a monitor-name descriptor; rest are dummies.
    for slot in 0..4 {
        let off = 54 + slot * 18;
        if slot < modes.len() {
            let m = modes[slot];
            edid[off..off + 18].copy_from_slice(&build_dtd(m.width, m.height, m.refresh_hz));
        } else if slot == modes.len() {
            edid[off..off + 18].copy_from_slice(&build_monitor_name_descriptor("LianLi TURZX"));
        } else {
            // Dummy descriptor (type 0x10) — valid but inert.
            edid[off..off + 5].copy_from_slice(&[0, 0, 0, 0x10, 0]);
        }
    }

    edid[126] = 0; // extension count
    let sum: u32 = edid[..127].iter().map(|&b| b as u32).sum();
    edid[127] = (0u8).wrapping_sub((sum & 0xFF) as u8);
    edid
}

fn build_dtd(width: u16, height: u16, refresh_hz: u8) -> [u8; 18] {
    // Reduced-blanking timings, CEA-style. Safe for any compositor.
    const H_BLANK: u16 = 160;
    const H_FRONT: u16 = 48;
    const H_SYNC: u16 = 32;
    const V_BLANK: u16 = 45;
    const V_FRONT: u16 = 4;
    const V_SYNC: u16 = 5;

    let pixel_clock_khz = ((width as u32 + H_BLANK as u32)
        * (height as u32 + V_BLANK as u32)
        * refresh_hz as u32)
        / 1000;
    let pixel_clock_10khz = (pixel_clock_khz / 10).min(u16::MAX as u32) as u16;

    let mut dtd = [0u8; 18];
    dtd[..2].copy_from_slice(&pixel_clock_10khz.to_le_bytes());

    // Byte 2-4: H active low, H blank low, (H active high << 4) | H blank high
    dtd[2] = (width & 0xFF) as u8;
    dtd[3] = (H_BLANK & 0xFF) as u8;
    dtd[4] = ((((width >> 8) & 0x0F) << 4) | ((H_BLANK >> 8) & 0x0F)) as u8;

    // Byte 5-7: V active low, V blank low, (V active high << 4) | V blank high
    dtd[5] = (height & 0xFF) as u8;
    dtd[6] = (V_BLANK & 0xFF) as u8;
    dtd[7] = ((((height >> 8) & 0x0F) << 4) | ((V_BLANK >> 8) & 0x0F)) as u8;

    // Byte 8-9: H sync offset low, H sync pulse low
    dtd[8] = (H_FRONT & 0xFF) as u8;
    dtd[9] = (H_SYNC & 0xFF) as u8;

    // Byte 10: (V sync offset << 4) | V sync pulse (both low 4 bits)
    dtd[10] = ((((V_FRONT & 0x0F) << 4) | (V_SYNC & 0x0F)) & 0xFF) as u8;

    // Byte 11: high bits of H/V sync offset/pulse
    dtd[11] = ((((H_FRONT >> 8) & 0x03) << 6)
        | (((H_SYNC >> 8) & 0x03) << 4)
        | (((V_FRONT >> 4) & 0x03) << 2)
        | ((V_SYNC >> 4) & 0x03)) as u8;

    // Byte 12-14: image size (mm). Declare a plausible size for the aspect
    // ratio — compositors use it only for DPI hints.
    let h_mm: u16 = 480;
    let v_mm: u16 = 120;
    dtd[12] = (h_mm & 0xFF) as u8;
    dtd[13] = (v_mm & 0xFF) as u8;
    dtd[14] = ((((h_mm >> 8) & 0x0F) << 4) | ((v_mm >> 8) & 0x0F)) as u8;

    dtd[15] = 0; // H border
    dtd[16] = 0; // V border
    // Byte 17: digital separate sync, positive H + V polarity, non-interlaced.
    dtd[17] = 0x1E;
    dtd
}

fn build_monitor_name_descriptor(name: &str) -> [u8; 18] {
    let mut d = [0u8; 18];
    d[3] = 0xFC; // type: monitor name
    let bytes = name.as_bytes();
    let n = bytes.len().min(13);
    d[5..5 + n].copy_from_slice(&bytes[..n]);
    for b in &mut d[5 + n..18] {
        *b = 0x0A; // line-feed padding, per EDID spec for name
    }
    d
}

fn fnv1a_u32(input: &[u8]) -> u32 {
    let mut hash: u32 = 0x811C_9DC5;
    for &b in input {
        hash ^= b as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

fn resolve_identity(transport: &UsbTransport, pid: u16) -> DeviceIdentity {
    let handle = transport.inner();
    let device = handle.device();
    let desc = device.device_descriptor().ok();

    let usb_serial = desc
        .as_ref()
        .and_then(|d| handle.read_serial_number_string_ascii(d).ok())
        .and_then(|s| {
            let trimmed = s.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        });

    let port_path = device
        .port_numbers()
        .ok()
        .filter(|p| !p.is_empty())
        .map(|ports| {
            let parts: Vec<String> = ports.iter().map(|p| p.to_string()).collect();
            format!("{}-{}", device.bus_number(), parts.join("."))
        })
        .unwrap_or_else(|| format!("{}-{}", device.bus_number(), device.address()));

    let identity_input = match &usb_serial {
        Some(s) => format!("{VID:04x}:{pid:04x}:{s}"),
        None => format!("{VID:04x}:{pid:04x}:{port_path}"),
    };
    let edid_serial = fnv1a_u32(identity_input.as_bytes()).max(1);

    DeviceIdentity { usb_serial, port_path, edid_serial }
}

impl Drop for TurzxDisplay {
    fn drop(&mut self) {
        if self.streaming {
            if let Err(e) = self.transport.write(&build_power_off(), LCD_WRITE_TIMEOUT) {
                debug!("TURZX {VID:04x}:{:04x} Drop power-off failed: {e}", self.pid);
            }
        }
    }
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
        assert_eq!(caps.modes.len(), 3);
        assert!(caps.modes.iter().any(|m| m.width == 1920 && m.height == 480 && m.refresh_hz == 60));
        assert!(caps.modes.iter().any(|m| m.width == 1920 && m.height == 480 && m.refresh_hz == 30));
    }

    #[test]
    fn config_packet_matches_spec() {
        let pkt = build_config_packet(1920, 480, FMT_MJPEG);
        assert_eq!(pkt.len(), 28);
        assert_eq!(
            pkt,
            vec![
                0xAF, 0x20, 0x00, 0x01, 0xAF, 0x20, 0x01, 0x07, 0xAF, 0x20, 0x02, 0x80, 0xAF, 0x20,
                0x03, 0x01, 0xAF, 0x20, 0x04, 0xE0, 0xAF, 0x20, 0x11, 0x01, 0xAF, 0x20, 0x1F, 0x01,
            ]
        );
    }

    #[test]
    fn pack_frame_has_commit() {
        let pkt = pack_frame(STREAM_B_FINAL, 0, &[0xAA, 0xBB]);
        assert_eq!(pkt, vec![0xAF, 0x69, 0, 0, 0, 0, 0, 2, 0xAA, 0xBB, 0xAF, 0x66]);
    }

    #[test]
    fn pack_fragment_has_no_commit() {
        let pkt = pack_fragment(STREAM_A_FRAG, 0x123, &[0xCC]);
        assert_eq!(pkt, vec![0xAF, 0x6C, 0, 0x01, 0x23, 0, 0, 1, 0xCC]);
    }

    #[test]
    fn stream_a_single_urb_when_under_budget() {
        let urbs = fragment_stream_a(&[1, 2, 3, 4], 128);
        assert_eq!(urbs.len(), 1);
        assert_eq!(urbs[0][0..2], [0xAF, STREAM_A_FINAL]);
        let last_two = &urbs[0][urbs[0].len() - 2..];
        assert_eq!(last_two, &[MAGIC, COMMIT]);
    }

    #[test]
    fn stream_a_multi_urb_spans_fragments_and_ends_with_final() {
        // urb_max=32 → frag budget 24, final budget 22
        let payload: Vec<u8> = (0..50).collect();
        let urbs = fragment_stream_a(&payload, 32);
        assert!(urbs.len() >= 2);
        // All but the last start with STREAM_A_FRAG and have no commit trailer
        for urb in urbs.iter().take(urbs.len() - 1) {
            assert_eq!(urb[1], STREAM_A_FRAG);
            let last_two = &urb[urb.len() - 2..];
            assert_ne!(last_two, &[MAGIC, COMMIT]);
        }
        let last = urbs.last().unwrap();
        assert_eq!(last[1], STREAM_A_FINAL);
        let last_two = &last[last.len() - 2..];
        assert_eq!(last_two, &[MAGIC, COMMIT]);
        // Reassemble payload from the urbs and confirm byte-identical.
        let mut reassembled = Vec::new();
        for urb in &urbs {
            let size = u32::from_be_bytes([0, urb[5], urb[6], urb[7]]) as usize;
            reassembled.extend_from_slice(&urb[8..8 + size]);
        }
        assert_eq!(reassembled, payload);
    }

    #[test]
    fn patch_edid_writes_serial_and_fixes_checksum() {
        let mut edid = [0u8; 128];
        edid[..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
        edid[8] = 0x52;
        edid[9] = 0xB2;
        // Seed checksum so block sums to 0 before we patch.
        let sum: u32 = edid[..127].iter().map(|&b| b as u32).sum();
        edid[127] = (0u8).wrapping_sub((sum & 0xFF) as u8);
        let sum_before: u32 = edid.iter().map(|&b| b as u32).sum();
        assert_eq!(sum_before % 256, 0, "precondition");

        patch_edid_serial(&mut edid, 0xDEAD_BEEF);

        assert_eq!(&edid[12..16], &0xDEAD_BEEFu32.to_le_bytes());
        let sum_after: u32 = edid.iter().map(|&b| b as u32).sum();
        assert_eq!(sum_after % 256, 0, "checksum still valid after patch");
    }

    #[test]
    fn synthetic_edid_is_valid() {
        let mut caps = VendorCaps {
            max_w: 2047,
            max_h: 2047,
            ..VendorCaps::default()
        };
        caps.modes.push(Mode { width: 1920, height: 480, refresh_hz: 60 });
        caps.modes.push(Mode { width: 1920, height: 480, refresh_hz: 30 });
        let edid = build_edid(&caps, 0xF73B_8A15);

        // Header magic
        assert_eq!(&edid[..8], &[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
        // "TUR" manufacturer
        assert_eq!(&edid[8..10], &[0x52, 0xB2]);
        // Serial
        assert_eq!(&edid[12..16], &0xF73B_8A15u32.to_le_bytes());
        // EDID 1.4
        assert_eq!(edid[18], 1);
        assert_eq!(edid[19], 4);
        // Checksum
        let sum: u32 = edid.iter().map(|&b| b as u32).sum();
        assert_eq!(sum % 256, 0);

        // First DTD should encode 1920x480 @ 60Hz.
        let dtd = &edid[54..72];
        let px_clock_khz = u16::from_le_bytes([dtd[0], dtd[1]]) as u32 * 10;
        let h_active = (((dtd[4] as u16) & 0xF0) << 4) | dtd[2] as u16;
        let h_blank = (((dtd[4] as u16) & 0x0F) << 8) | dtd[3] as u16;
        let v_active = (((dtd[7] as u16) & 0xF0) << 4) | dtd[5] as u16;
        let v_blank = (((dtd[7] as u16) & 0x0F) << 8) | dtd[6] as u16;
        let h_sync_pulse = (((dtd[11] as u16) & 0x30) << 4) | dtd[9] as u16;
        let h_front = (((dtd[11] as u16) & 0xC0) << 2) | dtd[8] as u16;
        assert_eq!(h_active, 1920);
        assert_eq!(v_active, 480);
        // Critical: sync pulse + front porch must fit inside blanking.
        assert!(
            h_front + h_sync_pulse < h_blank,
            "H sync pulse ({h_sync_pulse}) + front ({h_front}) must fit H blanking ({h_blank})"
        );
        assert!(v_blank > 0);
        // Pixel clock sanity: should land between 30 and 150 MHz for our modes.
        assert!((30_000..=150_000).contains(&px_clock_khz),
            "pixel clock {px_clock_khz} kHz outside sane range");
    }

    #[test]
    fn fnv1a_is_deterministic_and_distinct() {
        let a = fnv1a_u32(b"1a86:ad21:1-8.3");
        let b = fnv1a_u32(b"1a86:ad21:1-8.4");
        let c = fnv1a_u32(b"1a86:ad21:1-8.3");
        assert_eq!(a, c);
        assert_ne!(a, b);
    }

    #[test]
    fn stream_a_offset_advances() {
        let payload: Vec<u8> = (0..30).collect();
        let urbs = fragment_stream_a(&payload, 20);
        let mut expected_offset = 0u32;
        for urb in &urbs {
            let off = u32::from_be_bytes([0, urb[2], urb[3], urb[4]]);
            assert_eq!(off, expected_offset);
            let size = u32::from_be_bytes([0, urb[5], urb[6], urb[7]]);
            expected_offset += size;
        }
        assert_eq!(expected_offset as usize, payload.len());
    }
}
