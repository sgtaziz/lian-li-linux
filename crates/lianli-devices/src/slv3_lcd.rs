use crate::crypto::PacketBuilder;
use anyhow::{bail, Context, Result};
use lianli_shared::screen::ScreenInfo;
use lianli_transport::usb::{RusbBulk, LCD_WRITE_TIMEOUT, USB_TIMEOUT};
use rusb::{Device, GlobalContext};
use tracing::{debug, info};

/// SLV3/TLV2 wireless LCD fan — USB bulk with DES-encrypted headers.
pub struct Slv3LcdDevice {
    transport: RusbBulk,
    bus: u8,
    address: u8,
    serial: String,
    initialized: bool,
    screen: ScreenInfo,
    firmware: Option<String>,
}

impl Slv3LcdDevice {
    pub fn new(device: Device<GlobalContext>) -> Result<Self> {
        let bus = device.bus_number();
        let address = device.address();

        let desc = device
            .device_descriptor()
            .context("reading device descriptor")?;
        let serial = device
            .open()
            .and_then(|h| h.read_serial_number_string_ascii(&desc))
            .unwrap_or_else(|_| format!("bus{bus}-addr{address}"));

        let mut transport = RusbBulk::open_device(device).context("opening LCD device")?;
        transport
            .detach_and_configure("LCD")
            .context("configuring LCD device")?;

        Ok(Self {
            transport,
            bus,
            address,
            serial,
            initialized: false,
            screen: ScreenInfo::WIRELESS_LCD,
            firmware: None,
        })
    }

    pub fn bus(&self) -> u8 {
        self.bus
    }

    pub fn address(&self) -> u8 {
        self.address
    }

    pub fn serial(&self) -> &str {
        &self.serial
    }

    pub fn screen_info(&self) -> &ScreenInfo {
        &self.screen
    }

    fn send_init(&mut self, builder: &mut PacketBuilder) -> Result<()> {
        if self.initialized {
            return Ok(());
        }
        debug!("LCD[bus {} addr {}] init sequence", self.bus, self.address);

        // Init sequence:
        // 1. CheckNewLcd (0x80) — probe LCD hardware revision
        let mut buf = [0u8; 511];
        let check = builder.header(0, 0x80, false);
        self.transport.write(&check, LCD_WRITE_TIMEOUT)?;
        let _ = self.transport.read(&mut buf, USB_TIMEOUT);

        // 3. SetFrameRate(120) — hardcoded 120 for wireless LCDs
        let fps = builder.frame_rate_header(120);
        self.transport.write(&fps, LCD_WRITE_TIMEOUT)?;
        let _ = self.transport.read(&mut buf, USB_TIMEOUT);

        // 4. GetVer (0x0A) — read firmware version
        let ver = builder.header(0, 0x0A, false);
        self.transport.write(&ver, LCD_WRITE_TIMEOUT)?;
        let n = self.transport.read(&mut buf, USB_TIMEOUT).unwrap_or(0);
        if n > 8 {
            let end = buf[8..]
                .iter()
                .position(|&b| b == 0)
                .map(|p| 8 + p)
                .unwrap_or(n.min(40));
            let fw = String::from_utf8_lossy(&buf[8..end]).trim().to_string();
            if !fw.is_empty() {
                info!("SLV3/TLV2 LCD firmware: {fw}");
                self.firmware = Some(fw);
            }
        }

        self.initialized = true;
        Ok(())
    }

    /// Set LCD brightness (0-100). Uses legacy DES path opcode 0x0E.
    pub fn set_brightness(&mut self, builder: &mut PacketBuilder, brightness: u8) -> Result<()> {
        self.send_init(builder)?;
        let mapped = slv3_brightness_lut(brightness);
        let header = builder.brightness_header(mapped);
        self.transport.write(&header, LCD_WRITE_TIMEOUT)?;
        let mut buf = [0u8; 511];
        let _ = self.transport.read(&mut buf, USB_TIMEOUT);
        debug!("SLV3/TLV2 LCD brightness: {brightness} -> {mapped}");
        Ok(())
    }

    /// Reboot the LCD MCU. Uses legacy DES path opcode 0x0B.
    pub fn reboot(&mut self, builder: &mut PacketBuilder) -> Result<()> {
        let header = builder.header(0, 0x0B, false);
        self.transport.write(&header, LCD_WRITE_TIMEOUT)?;
        debug!("SLV3/TLV2 LCD reboot sent");
        Ok(())
    }

    pub fn firmware_str(&self) -> Option<&str> {
        self.firmware.as_deref()
    }

    pub fn send_frame(&mut self, builder: &mut PacketBuilder, frame: &[u8]) -> Result<()> {
        if frame.len() > self.screen.max_payload {
            bail!(
                "frame payload {} exceeds LCD payload limit {}",
                frame.len(),
                self.screen.max_payload
            );
        }

        self.send_init(builder)?;

        let header = builder.header(frame.len(), 0x65, true);
        let mut packet = vec![0u8; 102_400];
        packet[..512].copy_from_slice(&header);
        packet[512..512 + frame.len()].copy_from_slice(frame);

        self.transport
            .write(&packet, LCD_WRITE_TIMEOUT)
            .context("writing LCD frame data")?;

        let mut buf = [0u8; 511];
        let _ = self.transport.read(&mut buf, USB_TIMEOUT);
        Ok(())
    }
}

/// Brightness LUT for SLV3/TLV2 wireless LCD firmware. Maps 0–100 percent to
/// the expected byte via 5 anchor points with linear interpolation.
fn slv3_brightness_lut(value: u8) -> u8 {
    const ANCHORS: &[(u8, u8)] = &[(0, 0), (25, 10), (50, 30), (75, 40), (100, 100)];
    let v = value.min(100);
    if let Ok(idx) = ANCHORS.binary_search_by_key(&v, |&(in_v, _)| in_v) {
        return ANCHORS[idx].1;
    }
    let pos = ANCHORS
        .iter()
        .position(|&(in_v, _)| in_v > v)
        .unwrap_or(ANCHORS.len());
    let (lo_in, lo_out) = ANCHORS[pos - 1];
    let (hi_in, hi_out) = ANCHORS[pos];
    let span = (hi_in - lo_in) as u32;
    if span == 0 {
        return lo_out;
    }
    let num = (v - lo_in) as u32;
    let step_lo = (hi_out as u32).saturating_sub(lo_out as u32);
    lo_out.saturating_add(((num * step_lo + span / 2) / span) as u8)
}

#[cfg(test)]
mod tests {
    use super::slv3_brightness_lut;

    #[test]
    fn lut_anchors_match_vendor() {
        assert_eq!(slv3_brightness_lut(0), 0);
        assert_eq!(slv3_brightness_lut(25), 10);
        assert_eq!(slv3_brightness_lut(50), 30);
        assert_eq!(slv3_brightness_lut(75), 40);
        assert_eq!(slv3_brightness_lut(100), 100);
    }

    #[test]
    fn lut_interpolates_linearly() {
        // Between 25 -> 10 and 50 -> 30: midpoint ~37 -> 20
        assert_eq!(slv3_brightness_lut(37), 20);
        // Between 50 -> 30 and 75 -> 40: midpoint ~62 -> 35
        assert_eq!(slv3_brightness_lut(62), 35);
    }

    #[test]
    fn lut_clamps_above_100() {
        assert_eq!(slv3_brightness_lut(200), 100);
    }
}
