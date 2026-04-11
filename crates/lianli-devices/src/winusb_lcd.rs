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
use lianli_transport::usb::{UsbTransport, EP_OUT, LCD_READ_TIMEOUT, LCD_WRITE_TIMEOUT};
use rusb::{Device, GlobalContext};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

const RESET_COOLDOWN: Duration = Duration::from_secs(5);

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
    consecutive_failures: u32,
    last_reset: Option<Instant>,
    device_gone: bool,
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
            consecutive_failures: 0,
            last_reset: None,
            device_gone: false,
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

    pub fn transport_release(&self) {
        self.transport.release();
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

        match self.transport.write(&packet, LCD_WRITE_TIMEOUT) {
            Ok(_) => self.note_write_success(),
            Err(e) => {
                warn!("Frame write failed: {e}");
                self.try_recover()
                    .with_context(|| format!("recovering from frame write error: {e}"))?;
                self.transport
                    .write(&packet, LCD_WRITE_TIMEOUT)
                    .context("writing LCD frame after recovery")?;
                self.note_write_success();
            }
        }

        let resp = self.read_response("frame ack", Duration::from_millis(200));

        // Flow control: if device buffer is getting full, wait for it to drain
        if let Some(buf) = resp {
            if buf[8] > 3 {
                self.wait_buffer(2);
            }
        }

        Ok(())
    }

    /// Send a JPEG frame, retrying up to 3 times if the device doesn't ack.
    pub fn send_frame_verified(&mut self, frame: &[u8]) -> Result<()> {
        for attempt in 0..3u32 {
            match self.send_frame(frame) {
                Ok(()) => return Ok(()),
                Err(e) if attempt < 2 => {
                    warn!(
                        "Frame send failed (attempt {}): {e}, reinitializing",
                        attempt + 1
                    );
                    self.initialized = false;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    const H264_CHUNK_SIZE: usize = 202_752;

    /// Stream a raw H264 file in chunks via StartPlay (0x79).
    /// Loops the file if `looping` is true. Runs until `stop` is set.
    pub fn stream_h264(
        &mut self,
        path: &std::path::Path,
        looping: bool,
        stop: &std::sync::atomic::AtomicBool,
    ) -> Result<()> {
        use std::io::{Read, Seek};
        use std::sync::atomic::Ordering;

        if !self.initialized {
            self.do_init()?;
        }

        let mut file = std::fs::File::open(path).context("opening h264 file")?;
        let mut file_buf = vec![0u8; Self::H264_CHUNK_SIZE];

        loop {
            let n = file.read(&mut file_buf).context("reading h264 chunk")?;
            if n == 0 {
                if looping && !stop.load(Ordering::Relaxed) {
                    file.seek(std::io::SeekFrom::Start(0))?;
                    continue;
                }
                break;
            }
            if stop.load(Ordering::Relaxed) {
                break;
            }

            let is_last = {
                let pos = file.stream_position()?;
                let len = file.metadata()?.len();
                pos >= len
            };

            let header = self.builder.start_play_header_winusb(n, is_last);
            let mut packet = vec![0u8; 512 + n];
            packet[..512].copy_from_slice(&header);
            packet[512..512 + n].copy_from_slice(&file_buf[..n]);

            match self.transport.write(&packet, LCD_WRITE_TIMEOUT) {
                Ok(_) => self.note_write_success(),
                Err(e) => {
                    warn!("H264 chunk write failed: {e}");
                    self.try_recover()
                        .with_context(|| format!("recovering from h264 write error: {e}"))?;
                    self.transport
                        .write(&packet, LCD_WRITE_TIMEOUT)
                        .context("h264 chunk write after recovery")?;
                    self.note_write_success();
                }
            }

            let resp = self.read_response("h264 chunk", LCD_READ_TIMEOUT);

            std::thread::sleep(Duration::from_millis(30));

            if let Some(buf) = resp {
                if buf[8] > 3 {
                    self.wait_buffer(2);
                }
            }
        }

        self.transport.read_flush();
        self.initialized = false;
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
        info!(
            "Initializing LCD ({}x{}, quality {})",
            self.screen.width, self.screen.height, self.screen.jpeg_quality
        );
        self.transport.read_flush();

        let ver = self.builder.get_ver_header_winusb();
        self.send_command(ver, "GetVer");
        let stop_play = self.builder.stop_play_header_winusb();
        self.send_command(stop_play, "StopPlay");

        let sync = self.builder.sync_clock_header_winusb(2);
        self.send_command(sync, "SyncClock");
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
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut jpg_buf,
                self.screen.jpeg_quality,
            );
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
            self.read_response("ClearJpgLayer", Duration::from_millis(200));
        }
    }

    fn send_command(&mut self, header: Vec<u8>, label: &str) {
        match self.transport.write(&header, LCD_WRITE_TIMEOUT) {
            Ok(_) => self.note_write_success(),
            Err(e) => {
                warn!("{label} write failed: {e}");
                if let Err(rec_err) = self.try_recover() {
                    warn!("{label} recovery skipped: {rec_err}");
                    return;
                }
                if let Err(e2) = self.transport.write(&header, LCD_WRITE_TIMEOUT) {
                    warn!("{label} write retry failed: {e2}");
                    return;
                }
                self.note_write_success();
            }
        }
        self.read_response(label, LCD_READ_TIMEOUT);
    }

    fn note_write_success(&mut self) {
        self.consecutive_failures = 0;
    }

    fn try_recover(&mut self) -> Result<()> {
        if self.device_gone {
            bail!("device handle is stale; re-discovery required");
        }

        self.consecutive_failures += 1;

        if self.consecutive_failures <= 2 {
            match self.transport.clear_halt(EP_OUT) {
                Ok(()) => {
                    debug!("recovered EP_OUT stall via clear_halt");
                    return Ok(());
                }
                Err(e) => warn!("clear_halt(EP_OUT) failed: {e}"),
            }
        }

        let now = Instant::now();
        if let Some(last) = self.last_reset {
            let since = now.saturating_duration_since(last);
            if since < RESET_COOLDOWN {
                bail!(
                    "USB reset on cooldown ({:.1}s remaining)",
                    (RESET_COOLDOWN - since).as_secs_f32()
                );
            }
        }
        self.last_reset = Some(now);

        match self.transport.reset() {
            Ok(()) => {
                std::thread::sleep(Duration::from_millis(300));
                if let Err(e) = self.transport.detach_and_configure(&self.name) {
                    warn!("post-reset detach_and_configure failed: {e}");
                    bail!("recovery failed: {e}");
                }
                self.initialized = false;
                info!("USB reset + reconfigure succeeded");
                Ok(())
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("not found") || msg.contains("No such device") {
                    self.device_gone = true;
                    bail!("device disappeared during reset: {e}");
                }
                bail!("USB reset failed: {e}");
            }
        }
    }

    fn read_response(&mut self, context: &str, timeout: Duration) -> Option<[u8; 512]> {
        let mut buf = [0u8; 512];
        match self.transport.read(&mut buf, timeout) {
            Ok(n) if n > 0 => {
                debug!(
                    "Response for {context} ({n} bytes): {:02x?}",
                    &buf[..n.min(32)]
                );
                self.last_read_ok = true;
                self.transport.read_flush();
                return Some(buf);
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
        None
    }

    /// Query device buffer level. Returns None on communication failure.
    fn query_block(&mut self) -> Option<u8> {
        let header = self.builder.query_block_header_winusb();
        self.transport.write(&header, LCD_WRITE_TIMEOUT).ok()?;
        let resp = self.read_response("QueryBlock", Duration::from_millis(200))?;
        Some(resp[8])
    }

    /// Wait until the device buffer drains to an acceptable level.
    /// Reference polls QueryBlock every 50ms until buf[8] <= threshold.
    fn wait_buffer(&mut self, threshold: u8) {
        for _ in 0..40 {
            match self.query_block() {
                Some(level) if level <= threshold => return,
                Some(_) => std::thread::sleep(std::time::Duration::from_millis(50)),
                None => return,
            }
        }
        debug!("Buffer wait timed out after 2s");
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
