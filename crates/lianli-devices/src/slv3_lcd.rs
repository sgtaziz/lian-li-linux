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
        // 1. Rotate(0)
        let header = builder.header(0, 0x0D, false);
        self.transport.write(&header, LCD_WRITE_TIMEOUT)?;
        let mut buf = [0u8; 511];
        let _ = self.transport.read(&mut buf, USB_TIMEOUT);

        // 2. CheckNewLcd (0x80) — probe LCD hardware revision
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
        let header = builder.brightness_header(brightness);
        self.transport.write(&header, LCD_WRITE_TIMEOUT)?;
        let mut buf = [0u8; 511];
        let _ = self.transport.read(&mut buf, USB_TIMEOUT);
        debug!("SLV3/TLV2 LCD brightness: {brightness}");
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
