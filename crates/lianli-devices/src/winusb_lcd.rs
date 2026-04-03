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
use lianli_transport::usb::{UsbTransport, LCD_READ_TIMEOUT, LCD_WRITE_TIMEOUT};
use rusb::{Device, GlobalContext};
use std::time::Duration;
use tracing::{debug, info, warn};

/// Generic WinUSB LCD device.
///
/// Handles DES-CBC encrypted command headers + raw JPEG payload for any
/// directly-connected VID=0x1CBE LCD device.
pub struct WinUsbLcdDevice {
    transport: UsbTransport,
    builder: PacketBuilder,
    screen: ScreenInfo,
    name: String,
    bus: u8,
    address: u8,
    serial: String,
    initialized: bool,
    last_read_ok: bool,
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
            name: name.to_string(),
            bus,
            address,
            serial,
            initialized: false,
            last_read_ok: false,
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

        let header = self.builder.jpeg_header_winusb(frame.len());
        let total = 512 + frame.len();
        let mut packet = vec![0u8; total];
        packet[..512].copy_from_slice(&header);
        packet[512..total].copy_from_slice(frame);

        if let Err(e) = self.transport.write(&packet, LCD_WRITE_TIMEOUT) {
            warn!("Frame write failed: {e}, resetting transport");
            self.reinit_transport();
            self.transport
                .write(&packet, LCD_WRITE_TIMEOUT)
                .context("writing LCD frame (retry)")?;
        }

        self.read_response("frame ack", LCD_READ_TIMEOUT);

        Ok(())
    }

    /// Send a JPEG frame, retrying up to 3 times if the device doesn't ack.
    pub fn send_frame_verified(&mut self, frame: &[u8]) -> Result<()> {
        for attempt in 0..3u32 {
            match self.send_frame(frame) {
                Ok(()) if self.last_read_ok => return Ok(()),
                Ok(()) => {
                    warn!("Frame ack missing (attempt {}), reinitializing", attempt + 1);
                    self.initialized = false;
                }
                Err(e) if attempt < 2 => {
                    warn!("Frame send failed (attempt {}): {e}, reinitializing", attempt + 1);
                    self.initialized = false;
                }
                Err(e) => return Err(e),
            }
        }
        warn!("Frame delivery unconfirmed after 3 attempts, proceeding anyway");
        Ok(())
    }

    /// Set LCD brightness (0-100).
    pub fn set_brightness_val(&mut self, brightness: u8) -> Result<()> {
        let header = self.builder.brightness_header_winusb(brightness);
        self.transport
            .write(&header, LCD_WRITE_TIMEOUT)
            .context("setting brightness")?;
        self.read_response("brightness", LCD_READ_TIMEOUT);
        debug!("Set brightness to {}", brightness.min(100));
        Ok(())
    }

    /// Set LCD rotation (0=0°, 1=90°, 2=180°, 3=270°).
    pub fn set_rotation_val(&mut self, rotation: u8) -> Result<()> {
        let header = self.builder.rotation_header_winusb(rotation);
        self.transport
            .write(&header, LCD_WRITE_TIMEOUT)
            .context("setting rotation")?;
        self.read_response("rotation", LCD_READ_TIMEOUT);
        debug!("Set rotation to {}", rotation);
        Ok(())
    }

    /// Set frame rate.
    pub fn set_frame_rate(&mut self, fps: u8) -> Result<()> {
        let header = self.builder.frame_rate_header_winusb(fps);
        self.transport
            .write(&header, LCD_WRITE_TIMEOUT)
            .context("setting frame rate")?;
        self.read_response("frame rate", LCD_READ_TIMEOUT);
        debug!("Set frame rate to {fps}");
        Ok(())
    }

    /// Switch the device from LCD mode to desktop mode.
    ///
    /// Sends StopPlay → SwitchToDesktop (0x96) → Reboot (0x0B).
    /// The device reboots and re-enumerates as a CH340 device (VID=0x1A86).
    pub fn switch_to_desktop_mode(&mut self) -> Result<()> {
        let stop = self.builder.stop_play_header_winusb();
        self.send_command(stop, "StopPlay");

        let switch_cmd = self.builder.switch_to_desktop_header_winusb();
        self.send_command(switch_cmd, "SwitchToDesktop");

        let reboot = self.builder.reboot_header_winusb();
        self.send_command(reboot, "Reboot");

        info!("Sent SwitchToDesktop + Reboot — device will reboot into desktop mode");
        self.initialized = false;
        Ok(())
    }

    fn do_init(&mut self) -> Result<()> {
        self.transport.read_flush();

        let ver = self.builder.get_ver_header_winusb();
        self.send_command(ver, "GetVer");
        let stop_play = self.builder.stop_play_header_winusb();
        self.send_command(stop_play, "StopPlay");
        let stop_clock = self.builder.stop_clock_header_winusb();
        self.send_command(stop_clock, "StopClock");
        self.clear_layers();
        self.set_frame_rate(30)?;

        self.initialized = true;
        Ok(())
    }

    fn clear_layers(&mut self) {
        use image::{ImageBuffer, Rgb, Rgba};
        use std::io::Cursor;

        let w = self.screen.width as u32;
        let h = self.screen.height as u32;

        // Clear PNG overlay layer
        let png_img = ImageBuffer::from_pixel(w, h, Rgba([0u8, 0, 0, 0]));
        let mut png_buf = Vec::new();
        if png_img
            .write_to(&mut Cursor::new(&mut png_buf), image::ImageFormat::Png)
            .is_ok()
        {
            let header = self.builder.png_header_winusb(png_buf.len());
            let mut packet = vec![0u8; 512 + png_buf.len()];
            packet[..512].copy_from_slice(&header);
            packet[512..].copy_from_slice(&png_buf);
            if let Err(e) = self.transport.write(&packet, LCD_WRITE_TIMEOUT) {
                warn!("ClearPngLayer failed: {e}");
            } else {
                self.read_response("ClearPngLayer", LCD_READ_TIMEOUT);
            }
        }

        // Clear JPG background layer
        let jpg_img = ImageBuffer::from_pixel(w, h, Rgb([0u8, 0, 0]));
        let mut jpg_buf = Vec::new();
        {
            let mut encoder =
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg_buf, 50);
            if let Err(e) = encoder.encode_image(&jpg_img) {
                warn!("Failed to encode blank JPEG: {e}");
                return;
            }
        }
        let header = self.builder.jpeg_header_winusb(jpg_buf.len());
        let mut packet = vec![0u8; 512 + jpg_buf.len()];
        packet[..512].copy_from_slice(&header);
        packet[512..].copy_from_slice(&jpg_buf);
        if let Err(e) = self.transport.write(&packet, LCD_WRITE_TIMEOUT) {
            warn!("ClearJpgLayer failed: {e}");
        } else {
            self.read_response("ClearJpgLayer", LCD_READ_TIMEOUT);
        }
    }

    fn send_command(&mut self, header: Vec<u8>, label: &str) {
        if let Err(e) = self.transport.write(&header, LCD_WRITE_TIMEOUT) {
            warn!("{label} write failed: {e}, resetting transport");
            self.reinit_transport();
            if let Err(e2) = self.transport.write(&header, LCD_WRITE_TIMEOUT) {
                warn!("{label} write retry failed: {e2}");
                return;
            }
        }
        self.read_response(label, LCD_READ_TIMEOUT);
    }

    fn reinit_transport(&mut self) {
        let _ = self.transport.reset();
        std::thread::sleep(std::time::Duration::from_millis(500));
        if let Err(e) = self.transport.detach_and_configure(&self.name) {
            warn!("Transport reinit failed: {e}");
        }
    }

    fn read_response(&mut self, context: &str, timeout: Duration) {
        let mut buf = [0u8; 512];
        match self.transport.read(&mut buf, timeout) {
            Ok(n) if n > 0 => {
                debug!("Response for {context} ({n} bytes): {:02x?}", &buf[..n.min(32)]);
                self.last_read_ok = true;
            }
            Ok(_) => {
                debug!("No response for {context} (timeout)");
                self.last_read_ok = false;
            }
            Err(e) => {
                warn!("Read after {context} failed: {e}");
                self.last_read_ok = false;
            }
        }
        self.transport.read_flush();
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
