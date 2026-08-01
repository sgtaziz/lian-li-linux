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
use lianli_transport::usb::{RusbBulk, EP_IN, EP_OUT, LCD_READ_TIMEOUT, LCD_WRITE_TIMEOUT};
use parking_lot::Mutex;
use rusb::{Device, GlobalContext};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

type SharedTransport = Arc<Mutex<RusbBulk>>;

/// Generic WinUSB LCD device.
///
/// Handles DES-CBC encrypted command headers + raw JPEG payload for any
/// directly-connected VID=0x1CBE LCD device.
pub struct WinUsbLcdDevice {
    transport: SharedTransport,
    builder: PacketBuilder,
    screen: ScreenInfo,
    _name: String,
    bus: u8,
    address: u8,
    serial: String,
    initialized: bool,
    last_read_ok: bool,
    consecutive_failures: u32,
    h264_chunk_size: usize,

    device_gone: bool,
    firmware: Option<String>,
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

        let mut transport = RusbBulk::open_device(device).context("opening WinUSB LCD device")?;
        transport
            .detach_and_configure(name)
            .context("configuring WinUSB LCD device")?;

        info!(
            "{name} opened: {}x{} at bus {} addr {} serial {}",
            screen.width, screen.height, bus, address, serial
        );

        Ok(Self {
            transport: Arc::new(Mutex::new(transport)),
            builder: PacketBuilder::new(),
            screen,
            _name: name.to_string(),
            bus,
            address,
            serial,
            initialized: false,
            last_read_ok: false,
            consecutive_failures: 0,
            h264_chunk_size: 202_752,

            device_gone: false,
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

    pub fn firmware_str(&self) -> Option<&str> {
        self.firmware.as_deref()
    }

    pub fn shared_transport(&self) -> SharedTransport {
        Arc::clone(&self.transport)
    }

    pub fn transport_release(&self) {}

    #[inline]
    fn tx_write_full(
        &self,
        data: &[u8],
        timeout: Duration,
    ) -> std::result::Result<(), lianli_transport::TransportError> {
        self.transport.lock().write_full(data, timeout)
    }

    #[inline]
    fn tx_read(
        &self,
        buf: &mut [u8],
        timeout: Duration,
    ) -> std::result::Result<usize, lianli_transport::TransportError> {
        self.transport.lock().read(buf, timeout)
    }

    #[inline]
    fn tx_read_flush(&self) {
        self.transport.lock().read_flush();
    }

    #[inline]
    fn tx_clear_halt(&self, ep: u8) -> std::result::Result<(), lianli_transport::TransportError> {
        self.transport.lock().clear_halt(ep)
    }

    /// Send a frame (PNG overlay layer for `screen.png` devices, else JPEG
    /// background layer) to the LCD.
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

        let header = if self.screen.png {
            self.builder.png_header_winusb(frame.len())
        } else {
            self.builder.jpeg_header_winusb(frame.len())
        };
        let total = 512 + frame.len();
        let mut packet = vec![0u8; total];
        packet[..512].copy_from_slice(&header);
        packet[512..total].copy_from_slice(frame);

        match self.tx_write_full(&packet, LCD_WRITE_TIMEOUT) {
            Ok(_) => self.note_write_success(),
            Err(e) => {
                warn!("Frame write failed: {e}");
                self.try_recover()
                    .with_context(|| format!("recovering from frame write error: {e}"))?;
                self.tx_write_full(&packet, LCD_WRITE_TIMEOUT)
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

    #[allow(dead_code)]
    const DEFAULT_H264_CHUNK_SIZE: usize = 202_752;

    /// Stream a raw H264 file in chunks via StartPlay (0x79).
    /// Loops the file if `looping` is true. Runs until `stop` is set.
    /// Paces each chunk at `max(33ms, 1000ms / fps)` to avoid firmware overrun.
    pub fn stream_h264(
        &mut self,
        path: &std::path::Path,
        looping: bool,
        stop: &std::sync::atomic::AtomicBool,
        fps: f32,
    ) -> Result<()> {
        use std::io::{Read, Seek};
        use std::sync::atomic::Ordering;

        if !self.initialized {
            self.do_init()?;
        }

        let mut file = std::fs::File::open(path).context("opening h264 file")?;
        let mut file_buf = vec![0u8; self.h264_chunk_size];
        let interval = chunk_interval(fps);
        let mut next_deadline = std::time::Instant::now() + interval;

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

            self.send_h264_chunk(&file_buf[..n], is_last)?;
            sleep_until(&mut next_deadline, interval);
        }

        self.tx_read_flush();
        self.initialized = false;
        Ok(())
    }

    /// Stream a live H.264 byte stream (e.g. ffmpeg stdout) in Access-Unit
    /// frames via StartPlay (0x79). Runs until EOF or `stop` is set.
    pub fn stream_h264_reader<R: std::io::Read>(
        &mut self,
        reader: &mut R,
        stop: &std::sync::atomic::AtomicBool,
        fps: f32,
    ) -> Result<()> {
        use std::sync::atomic::Ordering;

        if !self.initialized {
            self.do_init()?;
        }

        let frame_interval = std::time::Duration::from_secs_f32(1.0 / fps.max(1.0));
        let mut read_buf = vec![0u8; 64 * 1024];
        let mut accum: Vec<u8> = Vec::with_capacity(256 * 1024);
        let mut next_deadline = std::time::Instant::now() + frame_interval;
        let mut frame_count: u32 = 0;

        loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let n = reader
                .read(&mut read_buf)
                .context("WinUSB LCD: read h264 stream")?;
            if n == 0 {
                break;
            }
            accum.extend_from_slice(&read_buf[..n]);
            while let Some(split) = crate::hydroshift_lcd::find_au_split(&accum) {
                let au: Vec<u8> = accum.drain(..split).collect();
                if !au.is_empty() {
                    self.send_h264_au(&au)?;

                    frame_count += 1;
                    if frame_count % 30 == 0 {
                        if let Some(level) = self.query_block() {
                            if level > 3 {
                                self.wait_buffer(2);
                            }
                        }
                    }

                    let now = std::time::Instant::now();
                    if now < next_deadline {
                        std::thread::sleep(next_deadline - now);
                    }
                    next_deadline += frame_interval;
                    if next_deadline < std::time::Instant::now() {
                        next_deadline = std::time::Instant::now() + frame_interval;
                    }
                }
            }
        }

        if !accum.is_empty() {
            self.send_h264_au(&accum)?;
        }
        self.tx_read_flush();
        self.initialized = false;
        Ok(())
    }

    fn send_h264_chunk(&mut self, data: &[u8], is_last: bool) -> Result<()> {
        let header = self.builder.start_play_header_winusb(data.len(), is_last);
        let mut packet = vec![0u8; 512 + data.len()];
        packet[..512].copy_from_slice(&header);
        packet[512..512 + data.len()].copy_from_slice(data);

        match self.tx_write_full(&packet, LCD_WRITE_TIMEOUT) {
            Ok(_) => self.note_write_success(),
            Err(e) => {
                warn!("H264 chunk write failed: {e}");
                self.try_recover()
                    .with_context(|| format!("recovering from h264 write error: {e}"))?;
                self.tx_write_full(&packet, LCD_WRITE_TIMEOUT)
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
        Ok(())
    }

    /// Lean per-frame send for live H.264 streaming via StartPlay (0x79).
    /// Writes one packet with full short-write handling and does a
    /// non-blocking ack drain. Buffer backpressure is handled by
    /// `tx_write_full` — if the device can't accept data, the write
    /// times out and `try_recover` kicks in.
    fn send_h264_au(&mut self, data: &[u8]) -> Result<()> {
        let header = self.builder.start_play_header_winusb(data.len(), false);
        let total = 512 + data.len();
        let mut packet = vec![0u8; total];
        packet[..512].copy_from_slice(&header);
        packet[512..].copy_from_slice(data);
        match self.tx_write_full(&packet, LCD_WRITE_TIMEOUT) {
            Ok(_) => self.note_write_success(),
            Err(e) => {
                warn!("H264 AU write failed: {e}");
                self.try_recover()
                    .with_context(|| format!("recovering from h264 AU write error: {e}"))?;
                self.tx_write_full(&packet, LCD_WRITE_TIMEOUT)
                    .context("h264 AU write after recovery")?;
                self.note_write_success();
            }
        }
        self.tx_read_flush();
        Ok(())
    }

    /// Set LCD brightness (0-100).
    pub fn set_brightness_val(&mut self, brightness: u8) -> Result<()> {
        let header = self.builder.brightness_header_winusb(brightness);
        self.tx_write_full(&header, LCD_WRITE_TIMEOUT)
            .context("setting brightness")?;
        self.read_response("brightness", LCD_READ_TIMEOUT);
        debug!("Set brightness to {}", brightness.min(100));
        Ok(())
    }

    /// Set LCD rotation (0=0°, 1=90°, 2=180°, 3=270°).
    pub fn set_rotation_val(&mut self, rotation: u8) -> Result<()> {
        let header = self.builder.rotation_header_winusb(rotation);
        self.tx_write_full(&header, LCD_WRITE_TIMEOUT)
            .context("setting rotation")?;
        self.read_response("rotation", LCD_READ_TIMEOUT);
        debug!("Set rotation to {}", rotation);
        Ok(())
    }

    /// Set frame rate.
    pub fn set_frame_rate(&mut self, fps: u8) -> Result<()> {
        let header = self.builder.frame_rate_header_winusb(fps);
        self.tx_write_full(&header, LCD_WRITE_TIMEOUT)
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
        self.device_gone = false;
        self.consecutive_failures = 0;
        self.tx_read_flush();

        let ver = self.builder.get_ver_header_winusb();
        match self.tx_write_full(&ver, LCD_WRITE_TIMEOUT) {
            Ok(_) => self.note_write_success(),
            Err(e) => warn!("GetVer write failed: {e}"),
        }
        if let Some(resp) = self.read_response("GetVer", LCD_READ_TIMEOUT) {
            // Firmware version string is at bytes 8..40 (32 bytes, UTF-8, null-trimmed).
            let fw_bytes = &resp[8..40.min(resp.len())];
            let end = fw_bytes
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(fw_bytes.len());
            let fw_str = String::from_utf8_lossy(&fw_bytes[..end]).to_string();
            if !fw_str.is_empty() {
                info!("LCD firmware: {fw_str}");
                self.firmware = Some(fw_str);
            }
        }
        let stop_play = self.builder.stop_play_header_winusb();
        self.send_command(stop_play, "StopPlay");

        let h264_block = self.builder.query_block_header_winusb();
        if self.tx_write_full(&h264_block, LCD_WRITE_TIMEOUT).is_ok() {
            if let Some(resp) = self.read_response("GetH264Block", LCD_READ_TIMEOUT) {
                if resp.len() >= 12 {
                    let size = u32::from_be_bytes([resp[8], resp[9], resp[10], resp[11]]) as usize;
                    if size > 0 {
                        self.h264_chunk_size = size;
                        debug!("H264 chunk size from device: {size}");
                    }
                }
            }
        }

        // HS2 OLED renders the firmware thermal-warning overlay over pushed
        // frames; disable it during init so user media shows cleanly.
        if self.screen.png {
            let warn = self.builder.warn_switch_header_winusb(false);
            self.send_command(warn, "WarnSwitch");
        }

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
            if let Err(e) = self.tx_write_full(&packet, LCD_WRITE_TIMEOUT) {
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
        if let Err(e) = self.tx_write_full(&packet, LCD_WRITE_TIMEOUT) {
            warn!("ClearJpgLayer failed: {e}");
        } else {
            self.read_response("ClearJpgLayer", Duration::from_millis(200));
        }
    }

    fn send_command(&mut self, header: Vec<u8>, label: &str) {
        match self.tx_write_full(&header, LCD_WRITE_TIMEOUT) {
            Ok(_) => self.note_write_success(),
            Err(e) => {
                warn!("{label} write failed: {e}");
                if let Err(rec_err) = self.try_recover() {
                    warn!("{label} recovery skipped: {rec_err}");
                    return;
                }
                if let Err(e2) = self.tx_write_full(&header, LCD_WRITE_TIMEOUT) {
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

        // Soft recovery only: try clearing endpoint stalls on both directions.
        // A brief delay lets the firmware drain internal buffers before we retry.
        // USB reset is too destructive on composite devices — it can take down
        // sibling interfaces (TURZX, LED MCU, etc.) on the same physical device.
        if self.consecutive_failures <= 3 {
            std::thread::sleep(Duration::from_millis(10));
            let out_ok = self.tx_clear_halt(EP_OUT).is_ok();
            let _ = self.tx_clear_halt(EP_IN);
            if out_ok {
                debug!("recovered EP_OUT stall via clear_halt");
                return Ok(());
            }
        }

        self.device_gone = true;
        bail!("device unresponsive after clear_halt attempts; re-discovery required");
    }

    fn read_response(&mut self, context: &str, timeout: Duration) -> Option<[u8; 512]> {
        let mut buf = [0u8; 512];
        match self.tx_read(&mut buf, timeout) {
            Ok(n) if n > 0 => {
                debug!(
                    "Response for {context} ({n} bytes): {:02x?}",
                    &buf[..n.min(32)]
                );
                self.last_read_ok = true;
                self.tx_read_flush();
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
        self.tx_read_flush();
        None
    }

    /// Query device buffer level. Returns None on communication failure.
    fn query_block(&mut self) -> Option<u8> {
        let header = self.builder.query_block_header_winusb();
        self.tx_write_full(&header, LCD_WRITE_TIMEOUT).ok()?;
        let mut buf = [0u8; 512];
        match self.tx_read(&mut buf, Duration::from_millis(200)) {
            Ok(n) if n > 0 => {
                self.tx_read_flush();
                Some(buf[8])
            }
            _ => {
                self.tx_read_flush();
                None
            }
        }
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

fn chunk_interval(fps: f32) -> Duration {
    let target = Duration::from_secs_f32(1.0 / fps.max(1.0));
    target.max(Duration::from_millis(30))
}

fn sleep_until(next_deadline: &mut Instant, interval: Duration) {
    let now = Instant::now();
    if now < *next_deadline {
        std::thread::sleep(*next_deadline - now);
    }
    *next_deadline += interval;
    let now = Instant::now();
    if *next_deadline < now {
        *next_deadline = now + interval;
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

/// Driver entry point for the WinUSB LCD family (HydroShift II Circle/Square,
/// Lancool 207, Universal Screen 8.8", Vision 9.2"). The specific variant is
/// selected from the PID via [`screen_for_pid`].
pub struct WinUsbLcdDriver;

impl crate::registry::DeviceDriver for WinUsbLcdDriver {
    fn family(&self) -> lianli_shared::device_id::DeviceFamily {
        // The driver dispatches across multiple PIDs; the per-instance family
        // is resolved from the PID via `screen_for_pid` below.
        lianli_shared::device_id::DeviceFamily::HydroShift2Lcd
    }

    fn open(
        &self,
        ctx: &crate::registry::OpenContext,
    ) -> anyhow::Result<crate::registry::OpenedDevice> {
        let (screen, family, name) = screen_for_pid(ctx.pid)
            .ok_or_else(|| anyhow::anyhow!("unknown WinUSB LCD PID {:#06x}", ctx.pid))?;
        let mut lcd = WinUsbLcdDevice::new(ctx.device.clone(), screen, name)?;
        crate::traits::LcdDevice::initialize(&mut lcd)?;
        let firmware = lcd.firmware_str().map(|s| s.to_string());

        let (fan, aio, rgb) = if matches!(ctx.pid, 0xA021 | 0xA034) {
            let shared = lcd.shared_transport();
            match super::h2_aio::H2AioController::new(shared, ctx.pid) {
                ctrl => {
                    let ctrl = std::sync::Arc::new(ctrl);
                    (
                        Some(Box::new(std::sync::Arc::clone(&ctrl))
                            as Box<dyn crate::traits::FanDevice>),
                        Some(Box::new(std::sync::Arc::clone(&ctrl))
                            as Box<dyn crate::traits::AioDevice>),
                        vec![(
                            String::new(),
                            Arc::new(ctrl) as Arc<dyn crate::traits::RgbDevice>,
                        )],
                    )
                }
            }
        } else {
            (None, None, Vec::new())
        };

        Ok(crate::registry::OpenedDevice {
            id: ctx.device_id(),
            family,
            capabilities: family.capabilities(),
            transport_kind: lianli_shared::device_id::TransportKind::UsbBulk,
            model_name: name.to_string(),
            firmware,
            fan,
            lcd: Some(Box::new(lcd)),
            rgb,
            aio,
            shared_hid: None,
        })
    }
}

/// Open a WinUSB LCD device by PID, resolving the screen info and display
/// name automatically. Used by both the registry driver and the daemon's
/// media-backend path.
pub fn open_for_pid(pid: u16, device: Device<GlobalContext>) -> Result<WinUsbLcdDevice> {
    let (screen, _family, name) = screen_for_pid(pid)
        .ok_or_else(|| anyhow::anyhow!("unknown WinUSB LCD PID {:#06x}", pid))?;
    WinUsbLcdDevice::new(device, screen, name)
}

/// Map a WinUSB LCD PID to its `(ScreenInfo, DeviceFamily, display name)`
/// triple. Returns `None` for unknown PIDs.
fn screen_for_pid(
    pid: u16,
) -> Option<(
    lianli_shared::screen::ScreenInfo,
    lianli_shared::device_id::DeviceFamily,
    &'static str,
)> {
    use lianli_shared::device_id::DeviceFamily;
    use lianli_shared::screen::ScreenInfo;
    match pid {
        0xA021 => Some((
            ScreenInfo::HYDROSHIFT2,
            DeviceFamily::HydroShift2Lcd,
            "HydroShift II LCD Circle",
        )),
        0xA034 => Some((
            ScreenInfo::HYDROSHIFT2,
            DeviceFamily::HydroShift2Lcd,
            "HydroShift II LCD Square",
        )),
        0xA065 => Some((
            ScreenInfo::LANCOOL_207,
            DeviceFamily::Lancool207,
            "Lancool 207 Digital",
        )),
        0xA088 => Some((
            ScreenInfo::UNIVERSAL_SCREEN,
            DeviceFamily::UniversalScreen,
            "Universal Screen 8.8\"",
        )),
        0xA092 => Some((
            ScreenInfo::VISION_9P2,
            DeviceFamily::Vision9p2,
            "Vision 9.2\"",
        )),
        0xA018 => Some((ScreenInfo::FLEX_LCD, DeviceFamily::TlFlexLcd, "TL Flex LCD")),
        0xA019 => Some((
            ScreenInfo::FLEX_LCD,
            DeviceFamily::SlInfFlexLcd,
            "SL Infinity Flex LCD",
        )),
        0xA068 => Some((
            ScreenInfo::HYDROSHIFT2_OLED_CURVE,
            DeviceFamily::HydroShift2OledCurveLcd,
            "HydroShift II OLED Curve",
        )),
        _ => None,
    }
}
