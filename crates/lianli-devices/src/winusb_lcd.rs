//! Generic WinUSB LCD driver for all VID=0x1CBE direct-connect LCD devices.
//!
//! Shared protocol for:
//!   - HydroShift II LCD Circle (0x1CBE:0xA021) — 480x480
//!   - Lancool 207 Digital      (0x1CBE:0xA065) — 1472x720
//!   - Universal Screen 8.8"    (0x1CBE:0xA088) — 1920x480
//!
//! All use a DES-CBC encrypted 512-byte command header + raw JPEG payload.
//! The H2 packet format differs from SLV3: 500-byte plaintext (vs 504), and
//! the 512-byte header has fixed trailer bytes [510]=0xa1, [511]=0x1a.

use crate::crypto::PacketBuilder;
use crate::traits::LcdDevice;
use anyhow::{bail, Context, Result};
use lianli_shared::screen::ScreenInfo;
use lianli_transport::usb::{UsbTransport, LCD_WRITE_TIMEOUT, USB_TIMEOUT};
use rusb::{Device, GlobalContext};
use tracing::{debug, info};

/// Generic WinUSB LCD device.
///
/// Handles DES-CBC encrypted command headers + raw JPEG payload for any
/// directly-connected VID=0x1CBE LCD device.
pub struct WinUsbLcdDevice {
    transport: UsbTransport,
    builder: PacketBuilder,
    screen: ScreenInfo,
    bus: u8,
    address: u8,
    serial: String,
    initialized: bool,
}

impl WinUsbLcdDevice {
    /// Open a WinUSB LCD device.
    pub fn new(device: Device<GlobalContext>, screen: ScreenInfo, name: &str) -> Result<Self> {
        let bus = device.bus_number();
        let address = device.address();

        let desc = device
            .device_descriptor()
            .context("reading device descriptor")?;
        let serial = device
            .open()
            .and_then(|h| h.read_serial_number_string_ascii(&desc))
            .unwrap_or_else(|_| format!("bus{bus}-addr{address}"));

        let mut transport =
            UsbTransport::open_device(device).context("opening WinUSB LCD device")?;
        transport
            .detach_and_configure(name)
            .context("configuring WinUSB LCD device")?;

        info!(
            "{name} opened: {}x{} at bus {} addr {} serial {}",
            screen.width, screen.height, bus, address, serial
        );

        Ok(Self {
            transport,
            builder: PacketBuilder::new(),
            screen,
            bus,
            address,
            serial,
            initialized: false,
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

    /// Send a JPEG frame to the LCD.
    pub fn send_frame(&mut self, frame: &[u8]) -> Result<()> {
        if frame.len() > self.screen.max_payload {
            bail!(
                "frame payload {} exceeds LCD limit {}",
                frame.len(),
                self.screen.max_payload
            );
        }

        if !self.initialized {
            self.do_init()?;
        }

        let header = self.builder.jpeg_header_h2(frame.len());
        let total = 512 + frame.len();
        let mut packet = vec![0u8; total];
        packet[..512].copy_from_slice(&header);
        packet[512..total].copy_from_slice(frame);

        self.transport
            .write_bulk(&packet, LCD_WRITE_TIMEOUT)
            .context("writing LCD frame")?;

        // Read ack (may timeout, that's fine)
        let mut buf = [0u8; 512];
        let _ = self.transport.read_bulk(&mut buf, USB_TIMEOUT);

        Ok(())
    }

    /// Set LCD brightness (0-100).
    pub fn set_brightness_val(&mut self, brightness: u8) -> Result<()> {
        let header = self.builder.brightness_header_h2(brightness);
        self.transport
            .write_bulk(&header, LCD_WRITE_TIMEOUT)
            .context("setting brightness")?;
        let mut buf = [0u8; 512];
        let _ = self.transport.read_bulk(&mut buf, USB_TIMEOUT);
        debug!("Set brightness to {}", brightness.min(100));
        Ok(())
    }

    /// Set LCD rotation (0=0°, 1=90°, 2=180°, 3=270°).
    pub fn set_rotation_val(&mut self, rotation: u8) -> Result<()> {
        let header = self.builder.rotation_header_h2(rotation);
        self.transport
            .write_bulk(&header, LCD_WRITE_TIMEOUT)
            .context("setting rotation")?;
        let mut buf = [0u8; 512];
        let _ = self.transport.read_bulk(&mut buf, USB_TIMEOUT);
        debug!("Set rotation to {}", rotation);
        Ok(())
    }

    /// Set frame rate.
    pub fn set_frame_rate(&mut self, fps: u8) -> Result<()> {
        let header = self.builder.frame_rate_header_h2(fps);
        self.transport
            .write_bulk(&header, LCD_WRITE_TIMEOUT)
            .context("setting frame rate")?;
        let mut buf = [0u8; 512];
        let _ = self.transport.read_bulk(&mut buf, USB_TIMEOUT);
        debug!("Set frame rate to {fps}");
        Ok(())
    }

    fn do_init(&mut self) -> Result<()> {
        // H2 init (WinUsbH2.cs::InitDev): only set frame rate, no rotation header.
        self.set_frame_rate(30)?;

        self.initialized = true;
        Ok(())
    }
}

impl LcdDevice for WinUsbLcdDevice {
    fn screen_info(&self) -> &ScreenInfo {
        &self.screen
    }

    fn send_jpeg_frame(&mut self, jpeg_data: &[u8]) -> Result<()> {
        self.send_frame(jpeg_data)
    }

    fn set_brightness(&self, _brightness: u8) -> Result<()> {
        // Can't call &mut self methods from &self trait method.
        // Brightness should be set via set_brightness_val() directly.
        Ok(())
    }

    fn set_rotation(&self, _degrees: u16) -> Result<()> {
        // Same limitation — use set_rotation_val() directly.
        Ok(())
    }

    fn initialize(&mut self) -> Result<()> {
        if !self.initialized {
            self.do_init()?;
        }
        Ok(())
    }
}
