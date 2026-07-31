use super::edid::build_edid;
use super::framing::{build_config_packet, build_power_off, fragment_stream_a, pack_frame};
use super::vendor_caps::{parse_vendor_desc, Mode, VendorCaps};
use super::{STREAM_B_FINAL, VID};
use anyhow::{bail, Context, Result};
use lianli_transport::usb::{RusbBulk, LCD_READ_TIMEOUT, LCD_WRITE_TIMEOUT};
use std::time::Duration;
use tracing::{debug, warn};

const READY_POLL_ATTEMPTS: usize = 100;
const READY_POLL_INTERVAL: Duration = Duration::from_millis(100);
const READY_STATUS_BIT: u8 = 0x10;
const STREAM_WRITE_TIMEOUT: Duration = Duration::from_millis(1_000);

pub struct TurzxDisplay {
    transport: RusbBulk,
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
            RusbBulk::open(VID, pid).with_context(|| format!("opening {VID:04x}:{pid:04x}"))?;
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
            debug!(
                "TURZX {:04x} raw device EDID captured (discarded — invalid DTDs)",
                self.pid
            );
        }

        // Device EDID has broken DTDs (H sync pulse > H blanking) that DRM
        // rejects. Build a clean one from the vendor descriptor instead.
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

fn write_full(transport: &RusbBulk, data: &[u8]) -> Result<()> {
    transport
        .write_full(data, STREAM_WRITE_TIMEOUT)
        .with_context(|| format!("usb bulk write {} bytes", data.len()))
}

fn fnv1a_u32(input: &[u8]) -> u32 {
    let mut hash: u32 = 0x811C_9DC5;
    for &b in input {
        hash ^= b as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

fn resolve_identity(transport: &RusbBulk, pid: u16) -> DeviceIdentity {
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

    DeviceIdentity {
        usb_serial,
        port_path,
        edid_serial,
    }
}

impl Drop for TurzxDisplay {
    fn drop(&mut self) {
        if self.streaming {
            if let Err(e) = self.transport.write(&build_power_off(), LCD_WRITE_TIMEOUT) {
                debug!(
                    "TURZX {VID:04x}:{:04x} Drop power-off failed: {e}",
                    self.pid
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fnv1a_u32;

    #[test]
    fn fnv1a_is_deterministic_and_distinct() {
        let a = fnv1a_u32(b"1a86:ad21:1-8.3");
        let b = fnv1a_u32(b"1a86:ad21:1-8.4");
        let c = fnv1a_u32(b"1a86:ad21:1-8.3");
        assert_eq!(a, c);
        assert_ne!(a, b);
    }
}
