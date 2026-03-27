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
use tracing::{debug, info, warn};

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

        self.transport
            .write(&packet, LCD_WRITE_TIMEOUT)
            .context("writing LCD frame")?;

        self.read_response("frame ack");

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
        self.read_response("brightness");
        debug!("Set brightness to {}", brightness.min(100));
        Ok(())
    }

    /// Set LCD rotation (0=0°, 1=90°, 2=180°, 3=270°).
    pub fn set_rotation_val(&mut self, rotation: u8) -> Result<()> {
        let header = self.builder.rotation_header_winusb(rotation);
        self.transport
            .write(&header, LCD_WRITE_TIMEOUT)
            .context("setting rotation")?;
        self.read_response("rotation");
        debug!("Set rotation to {}", rotation);
        Ok(())
    }

    /// Set frame rate.
    pub fn set_frame_rate(&mut self, fps: u8) -> Result<()> {
        let header = self.builder.frame_rate_header_winusb(fps);
        self.transport
            .write(&header, LCD_WRITE_TIMEOUT)
            .context("setting frame rate")?;
        self.read_response("frame rate");
        debug!("Set frame rate to {fps}");
        Ok(())
    }

    /// Switch the device from LCD mode to desktop mode.
    ///
    /// Sends StopPlay + SwitchToDesktop (0x96). The device reboots and
    /// re-enumerates as a CH340 device (VID=0x1A86).
    pub fn switch_to_desktop_mode(&mut self) -> Result<()> {
        let stop = self.builder.stop_play_header_winusb();
        self.send_command(stop, "StopPlay");

        let switch = self.builder.switch_to_desktop_header_winusb();
        self.transport
            .write(&switch, LCD_WRITE_TIMEOUT)
            .context("sending SwitchToDesktop command")?;

        info!("Sent SwitchToDesktop — device will reboot into desktop mode");
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
        self.set_frame_rate(30)?;

        self.initialized = true;
        Ok(())
    }

    fn send_command(&mut self, header: Vec<u8>, label: &str) {
        if let Err(e) = self.transport.write(&header, LCD_WRITE_TIMEOUT) {
            warn!("{label} write failed: {e}");
            return;
        }
        self.read_response(label);
    }

    fn read_response(&mut self, context: &str) {
        let mut buf = [0u8; 512];
        match self.transport.read(&mut buf, USB_TIMEOUT) {
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
