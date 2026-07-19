use crate::crypto::PacketBuilder;
use anyhow::{bail, Context, Result};
use lianli_shared::screen::ScreenInfo;
use lianli_transport::usb::{RusbBulk, LCD_WRITE_TIMEOUT, USB_TIMEOUT};
use rusb::{Device, GlobalContext};
use tracing::debug;

/// SLV3/TLV2 wireless LCD fan — USB bulk with DES-encrypted headers.
pub struct Slv3LcdDevice {
    transport: RusbBulk,
    bus: u8,
    address: u8,
    serial: String,
    initialized: bool,
    screen: ScreenInfo,
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
        debug!(
            "LCD[bus {} addr {}] sending 0x0d init header",
            self.bus, self.address
        );
        let header = builder.header(0, 0x0D, false);
        self.transport
            .write(&header, LCD_WRITE_TIMEOUT)
            .context("writing LCD init header")?;
        let mut buf = [0u8; 511];
        let _ = self.transport.read(&mut buf, USB_TIMEOUT);
        self.initialized = true;
        Ok(())
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
