use crate::crypto::PacketBuilder;
use anyhow::{bail, Context, Result};
use lianli_shared::screen::ScreenInfo;
use lianli_transport::usb::{RusbBulk, EP_IN, EP_OUT};
use parking_lot::Mutex;
use rusb::{Device, GlobalContext};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

pub type SharedTransport = Arc<Mutex<RusbBulk>>;

const REOPEN_DELAY: Duration = Duration::from_millis(100);
/// Chunk writes slower than this are logged: the panel NAKed bulk OUT.
const SLOW_CHUNK_WRITE: Duration = Duration::from_millis(100);
const WAIT_BUFFER_POLL: Duration = Duration::from_millis(50);
const WAIT_BUFFER_NO_STOP_CAP: u32 = 600;

pub(crate) struct WinUsbLcdCore {
    transport: SharedTransport,
    builder: PacketBuilder,
    screen: ScreenInfo,
    write_timeout: Duration,
    read_timeout: Duration,
    name: String,
    raw_device: Option<Device<GlobalContext>>,
    pub(crate) initialized: bool,
    pub(crate) consecutive_failures: u32,
    pub(crate) h264_chunk_size: usize,
    pub(crate) device_gone: bool,
    pub(crate) firmware: Option<String>,
}

impl WinUsbLcdCore {
    pub(crate) fn open(
        device: Device<GlobalContext>,
        screen: ScreenInfo,
        name: &str,
        write_timeout: Duration,
        read_timeout: Duration,
    ) -> Result<Self> {
        let bus = device.bus_number();
        let address = device.address();
        let desc = device
            .device_descriptor()
            .context("reading device descriptor")?;
        let serial = device
            .open()
            .and_then(|h| h.read_serial_number_string_ascii(&desc))
            .unwrap_or_else(|_| format!("bus{bus}-addr{address}"));

        let mut transport = RusbBulk::open_device(device.clone()).context("opening WinUSB LCD")?;
        transport
            .detach_and_configure(name)
            .context("configuring WinUSB LCD")?;

        info!(
            "{name} opened: {}x{} at bus {bus} addr {address} serial {serial}",
            screen.width, screen.height
        );

        Ok(Self {
            transport: Arc::new(Mutex::new(transport)),
            builder: PacketBuilder::new(),
            screen,
            write_timeout,
            read_timeout,
            name: name.to_string(),
            raw_device: Some(device),
            initialized: false,
            consecutive_failures: 0,
            h264_chunk_size: 202_752,
            device_gone: false,
            firmware: None,
        })
    }

    pub(crate) fn from_shared(
        transport: SharedTransport,
        screen: ScreenInfo,
        name: String,
        write_timeout: Duration,
        read_timeout: Duration,
    ) -> Self {
        Self {
            transport,
            builder: PacketBuilder::new(),
            screen,
            write_timeout,
            read_timeout,
            name,
            raw_device: None,
            initialized: false,
            consecutive_failures: 0,
            h264_chunk_size: 202_752,
            device_gone: false,
            firmware: None,
        }
    }

    pub(crate) fn screen(&self) -> &ScreenInfo {
        &self.screen
    }

    pub(crate) fn builder_mut(&mut self) -> &mut PacketBuilder {
        &mut self.builder
    }

    pub(crate) fn shared_transport(&self) -> SharedTransport {
        Arc::clone(&self.transport)
    }

    pub(crate) fn firmware_str(&self) -> Option<&str> {
        self.firmware.as_deref()
    }

    pub(crate) fn transport_release(&self) {}

    #[inline]
    fn tx_write_full(
        &self,
        data: &[u8],
    ) -> std::result::Result<(), lianli_transport::TransportError> {
        self.transport.lock().write_full(data, self.write_timeout)
    }

    #[inline]
    fn tx_read(
        &self,
        buf: &mut [u8],
    ) -> std::result::Result<usize, lianli_transport::TransportError> {
        self.transport.lock().read(buf, self.read_timeout)
    }

    #[inline]
    fn tx_read_flush(&self) {
        self.transport.lock().read_flush();
    }

    #[inline]
    fn tx_clear_halt(&self, ep: u8) -> std::result::Result<(), lianli_transport::TransportError> {
        self.transport.lock().clear_halt(ep)
    }

    fn note_write_success(&mut self) {
        self.consecutive_failures = 0;
    }

    /// Vendor-faithful recovery: close the handle and reopen it from the raw
    /// device (ReInitDev), so a stalled endpoint is recovered within the
    /// session without a USB port reset (which would take down composite
    /// siblings like the LED MCU). Falls back to clear_halt when no raw device
    /// is available (shared-transport path).
    fn try_recover(&mut self) -> Result<()> {
        if self.device_gone {
            bail!("device handle is stale; re-discovery required");
        }
        self.consecutive_failures += 1;

        if let Some(raw) = &self.raw_device {
            std::thread::sleep(REOPEN_DELAY);
            match RusbBulk::open_device(raw.clone()) {
                Ok(mut t) => {
                    if t.detach_and_configure(&self.name).is_ok() {
                        *self.transport.lock() = t;
                        self.consecutive_failures = 0;
                        debug!("recovered via close+reopen");
                        return Ok(());
                    }
                }
                Err(e) => warn!("reopen failed: {e}"),
            }
        }

        let out_ok = self.tx_clear_halt(EP_OUT).is_ok();
        let _ = self.tx_clear_halt(EP_IN);
        if out_ok && self.consecutive_failures <= 5 {
            debug!("recovered EP_OUT stall via clear_halt");
            return Ok(());
        }

        self.device_gone = true;
        bail!("device unresponsive after recovery attempts; re-discovery required")
    }

    fn read_response(&mut self, context: &str) -> Option<[u8; 512]> {
        let mut buf = [0u8; 512];
        match self.tx_read(&mut buf) {
            Ok(n) if n > 0 => {
                debug!(
                    "Response for {context} ({n} bytes): {:02x?}",
                    &buf[..n.min(32)]
                );
                self.tx_read_flush();
                return Some(buf);
            }
            Ok(_) => debug!("No response for {context} (timeout)"),
            Err(e) => warn!("Read after {context} failed: {e}"),
        }
        self.tx_read_flush();
        None
    }

    pub(crate) fn send_command(&mut self, header: Vec<u8>, label: &str) {
        match self.tx_write_full(&header) {
            Ok(_) => self.note_write_success(),
            Err(e) => {
                warn!("{label} write failed: {e}");
                if let Err(rec_err) = self.try_recover() {
                    warn!("{label} recovery skipped: {rec_err}");
                    return;
                }
                if let Err(e2) = self.tx_write_full(&header) {
                    warn!("{label} write retry failed: {e2}");
                    return;
                }
                self.note_write_success();
            }
        }
        self.read_response(label);
    }

    pub(crate) fn read_firmware(&mut self) {
        let ver = self.builder.get_ver_header_winusb();
        match self.tx_write_full(&ver) {
            Ok(_) => self.note_write_success(),
            Err(e) => warn!("GetVer write failed: {e}"),
        }
        if let Some(resp) = self.read_response("GetVer") {
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
    }

    pub(crate) fn query_h264_block(&mut self) {
        let h264_block = self.builder.get_h264_block_header_winusb();
        if self.tx_write_full(&h264_block).is_ok() {
            if let Some(resp) = self.read_response("GetH264Block") {
                if resp.len() >= 12 {
                    let size = u32::from_be_bytes([resp[8], resp[9], resp[10], resp[11]]) as usize;
                    if size > 0 {
                        self.h264_chunk_size = size;
                        debug!("H264 chunk size from device: {size}");
                    }
                }
            }
        }
    }

    pub(crate) fn clear_png_cmd(&mut self) {
        let h = self.builder.clear_png_header_winusb();
        self.send_command(h, "ClearPng");
    }

    pub(crate) fn stop_clock_resp(&mut self) -> Option<[u8; 512]> {
        let h = self.builder.stop_clock_header_winusb();
        match self.tx_write_full(&h) {
            Ok(_) => self.note_write_success(),
            Err(e) => {
                warn!("StopClock write failed: {e}");
                return None;
            }
        }
        self.read_response("StopClock")
    }

    pub(crate) fn clear_jpg_layer(&mut self) {
        use image::{ImageBuffer, Rgb};
        let jpg_img =
            ImageBuffer::from_pixel(self.screen.width, self.screen.height, Rgb([0u8, 0, 0]));
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
        if let Err(e) = self.tx_write_full(&packet) {
            warn!("ClearJpgLayer failed: {e}");
        } else {
            self.read_response("ClearJpgLayer");
        }
    }

    pub(crate) fn clear_layers(&mut self) {
        use image::{ImageBuffer, Rgb, Rgba};
        use std::io::Cursor;

        let w = self.screen.width;
        let h = self.screen.height;

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
            if let Err(e) = self.tx_write_full(&packet) {
                warn!("ClearPngLayer failed: {e}");
            } else {
                self.read_response("ClearPngLayer");
            }
        }

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
        if let Err(e) = self.tx_write_full(&packet) {
            warn!("ClearJpgLayer failed: {e}");
        } else {
            self.read_response("ClearJpgLayer");
        }
    }

    pub(crate) fn send_frame(&mut self, frame: &[u8]) -> Result<()> {
        if frame.len() > self.screen.max_payload {
            bail!(
                "frame payload {} exceeds LCD limit {}",
                frame.len(),
                self.screen.max_payload
            );
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

        match self.tx_write_full(&packet) {
            Ok(_) => self.note_write_success(),
            Err(e) => {
                warn!("Frame write failed: {e}");
                self.try_recover()
                    .with_context(|| format!("recovering from frame write error: {e}"))?;
                self.tx_write_full(&packet)
                    .context("writing LCD frame after recovery")?;
                self.note_write_success();
            }
        }

        let resp = self.read_response("frame ack");
        if let Some(buf) = resp {
            if buf[8] > 3 {
                self.wait_buffer(2, None);
            }
        }
        Ok(())
    }

    pub(crate) fn send_frame_verified(&mut self, frame: &[u8]) -> Result<()> {
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

    pub(crate) fn set_brightness_val(&mut self, brightness: u8) -> Result<()> {
        let header = self.builder.brightness_header_winusb(brightness);
        self.tx_write_full(&header).context("setting brightness")?;
        self.read_response("brightness");
        debug!("Set brightness to {}", brightness.min(100));
        Ok(())
    }

    pub(crate) fn set_frame_rate(&mut self, fps: u8) -> Result<()> {
        let header = self.builder.frame_rate_header_winusb(fps);
        self.tx_write_full(&header).context("setting frame rate")?;
        self.read_response("frame rate");
        debug!("Set frame rate to {fps}");
        Ok(())
    }

    pub(crate) fn apply_stream_fps(&mut self, fps: f32) -> Result<()> {
        let clamped = fps.round().clamp(1.0, self.screen.max_fps as f32) as u8;
        self.set_frame_rate(clamped)
    }

    pub(crate) fn switch_to_desktop_mode(&mut self) -> Result<()> {
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

    fn query_buffer_level(&mut self) -> Option<u8> {
        let header = self.builder.query_buffer_level_header_winusb();
        self.tx_write_full(&header).ok()?;
        let mut buf = [0u8; 512];
        match self.tx_read(&mut buf) {
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

    /// Vendor-faithful: poll QueryBlock every 50ms until the buffer drains to
    /// `threshold` or less. When a `stop` flag is supplied (H264 streaming) it
    /// is honoured as the cancellation token; otherwise a safety cap prevents
    /// an indefinite hang on a wedged device.
    pub(crate) fn wait_buffer(&mut self, threshold: u8, stop: Option<&AtomicBool>) {
        let mut iter = 0u32;
        loop {
            if let Some(s) = stop {
                if s.load(Ordering::Relaxed) {
                    return;
                }
            } else if iter >= WAIT_BUFFER_NO_STOP_CAP {
                debug!("Buffer wait capped after {} polls", WAIT_BUFFER_NO_STOP_CAP);
                return;
            }
            iter += 1;
            match self.query_buffer_level() {
                Some(level) if level <= threshold => return,
                Some(_) => std::thread::sleep(WAIT_BUFFER_POLL),
                None => {
                    debug!("Buffer wait aborted (no response)");
                    return;
                }
            }
        }
    }

    fn send_h264_chunk(
        &mut self,
        data: &[u8],
        is_last: bool,
        play_count: u8,
        play_tick: u32,
        stop: &AtomicBool,
    ) -> Result<()> {
        let header =
            self.builder
                .start_play_header_winusb(data.len(), is_last, play_count, play_tick);
        let mut packet = vec![0u8; 512 + data.len()];
        packet[..512].copy_from_slice(&header);
        packet[512..512 + data.len()].copy_from_slice(data);

        let write_started = Instant::now();
        let write_result = self.tx_write_full(&packet);
        let write_took = write_started.elapsed();
        if write_took > SLOW_CHUNK_WRITE {
            // Device NAK stall: the panel is back-pressuring. Visible at WARN
            // so field logs show stalls that the write timeout absorbed.
            warn!(
                "H264 chunk write stalled {} ms ({} bytes, result {:?})",
                write_took.as_millis(),
                packet.len(),
                write_result.as_ref().map(|_| ()).map_err(|e| e.to_string())
            );
        }
        match write_result {
            Ok(_) => self.note_write_success(),
            Err(e) => {
                warn!("H264 chunk write failed: {e}");
                self.try_recover()
                    .with_context(|| format!("recovering from h264 write error: {e}"))?;
                self.tx_write_full(&packet)
                    .context("h264 chunk write after recovery")?;
                self.note_write_success();
            }
        }

        let resp = self.read_response("h264 chunk");
        if let Some(buf) = resp {
            if buf[8] > 3 {
                self.wait_buffer(2, Some(stop));
            }
        }
        Ok(())
    }

    pub(crate) fn stream_h264(
        &mut self,
        path: &std::path::Path,
        looping: bool,
        stop: &AtomicBool,
        fps: f32,
        play_count: u8,
        play_tick: u32,
    ) -> Result<()> {
        use std::io::{Read, Seek};

        let mut file = std::fs::File::open(path).context("opening h264 file")?;
        let mut file_buf = vec![0u8; self.h264_chunk_size];
        let interval = chunk_interval(fps);
        let mut next_deadline = Instant::now() + interval;

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
            self.send_h264_chunk(&file_buf[..n], is_last, play_count, play_tick, stop)?;
            sleep_until(&mut next_deadline, interval);
        }

        self.tx_read_flush();
        self.initialized = false;
        Ok(())
    }

    pub(crate) fn stream_h264_reader(
        &mut self,
        reader: &mut dyn std::io::Read,
        stop: &AtomicBool,
        play_count: u8,
        play_tick: u32,
    ) -> Result<()> {
        let mut buf = vec![0u8; self.h264_chunk_size];
        loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let n = reader
                .read(&mut buf)
                .context("WinUSB LCD: read h264 stream")?;
            if n == 0 {
                break;
            }
            self.send_h264_chunk(&buf[..n], false, play_count, play_tick, stop)?;
        }
        self.tx_read_flush();
        self.initialized = false;
        Ok(())
    }

    pub(crate) fn init_logging(&self) {
        info!(
            "Initializing LCD ({}x{}, quality {})",
            self.screen.width, self.screen.height, self.screen.jpeg_quality
        );
    }

    pub(crate) fn reset_failure_state(&mut self) {
        self.device_gone = false;
        self.consecutive_failures = 0;
        self.tx_read_flush();
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
