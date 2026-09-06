use crate::crypto::PacketBuilder;
use anyhow::{bail, Context, Result};
use lianli_shared::screen::ScreenInfo;
use lianli_transport::usb::{RusbBulk, EP_IN, EP_OUT};
use parking_lot::{Mutex, MutexGuard};
use rusb::{Device, GlobalContext};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// A control command (SyncPumpFan, PushRgbData, …) that the H2 AIO channel
/// handed to the LCD stream thread because the panel was busy ingesting
/// H.264. Sent verbatim at the next safe point; the reply is read and
/// discarded after `reply_wait`.
pub struct PendingCmd {
    pub label: &'static str,
    pub packet: Vec<u8>,
    pub reply_wait: Duration,
    pub queued_at: Instant,
    /// May this go out between chunks once the panel reports headroom? False
    /// for commands that hang the panel in play mode regardless of buffer
    /// level (PushRgbData — tested at levels 4 and 1, 2026-08-23) and also
    /// after it (2026-09-06); those only go out once the stream thread has
    /// stopped play and reinitialised the panel (see `reinit_and_flush_unsafe`).
    pub play_safe: bool,
}

/// USB bulk handle shared by the LCD stream and the HydroShift II control
/// channel (pump/fan/ring RGB), plus the coordination that keeps control
/// commands off the wire while the panel's ingest buffer is full.
///
/// Field evidence (usbmon, 2026-08-22/23): a SyncPumpFan or PushRgbData write
/// landing while the panel reports buffer level 3–4 mid-stream hangs the MCU
/// (bulk IN goes silent; sometimes EP0 dies too and only a power cycle helps).
/// So while `streaming` is set, control writers queue their packet here and the
/// stream thread — the only writer — flushes the queue once the panel reports
/// headroom.
pub struct LcdLink {
    bulk: Mutex<RusbBulk>,
    /// The rusb device the handle was opened from, so any core on this
    /// transport can close and reopen it (vendor ReInitDev).
    raw_device: Option<Device<GlobalContext>>,
    streaming: AtomicBool,
    /// Set by `push_and_recover`: the handle was reopened behind the LCD
    /// driver's back, so it must rerun its init before the next frame.
    needs_init: AtomicBool,
    /// When the last push-and-recover cycle ran, to space cycles out.
    last_hold: Mutex<Option<Instant>>,
    pending: Mutex<Vec<PendingCmd>>,
}

impl LcdLink {
    pub fn new(bulk: RusbBulk, raw_device: Option<Device<GlobalContext>>) -> Self {
        Self {
            bulk: Mutex::new(bulk),
            raw_device,
            streaming: AtomicBool::new(false),
            needs_init: AtomicBool::new(false),
            last_hold: Mutex::new(None),
            pending: Mutex::new(Vec::new()),
        }
    }

    pub fn lock(&self) -> MutexGuard<'_, RusbBulk> {
        self.bulk.lock()
    }

    /// True while an H.264 stream is feeding the panel.
    pub fn is_streaming(&self) -> bool {
        self.streaming.load(Ordering::Acquire)
    }

    pub(crate) fn set_streaming(&self, on: bool) {
        self.streaming.store(on, Ordering::Release);
    }

    /// True after `push_and_recover` reopened the handle: the LCD driver
    /// must rerun its init sequence before it streams or draws again.
    pub(crate) fn needs_init(&self) -> bool {
        self.needs_init.load(Ordering::Acquire)
    }

    pub(crate) fn set_needs_init(&self, on: bool) {
        self.needs_init.store(on, Ordering::Release);
    }

    pub(crate) fn can_reopen(&self) -> bool {
        self.raw_device.is_some()
    }

    /// Enough time since the last push-and-recover cycle for another one.
    /// A colour drag in the GUI queues a write per tick; `defer` keeps only
    /// the latest, and this keeps the stream from being stopped for each.
    pub(crate) fn hold_allowed(&self) -> bool {
        self.last_hold
            .lock()
            .is_none_or(|at| at.elapsed() >= HOLD_MIN_INTERVAL)
    }

    /// Close the bulk handle and reopen it from the raw device (the vendor
    /// driver's ReInitDev), swapping the new handle in under the guard the
    /// caller already holds. No USB port reset is involved: that would take
    /// down composite siblings like the LED MCU, and on the HydroShift II
    /// locks the header until a power cycle.
    fn reopen_locked(&self, bulk: &mut RusbBulk, name: &str) -> Result<()> {
        let raw = self
            .raw_device
            .as_ref()
            .context("no raw device handle to reopen from")?;
        // FIX: release the interfaces the current handle holds *before*
        // reopening. The replacement only lands once the new open succeeds,
        // so without this the old handle still owns interface 0 and every
        // claim_interface(0) returns EBUSY — the recovery path could never
        // succeed, it just retried 20 times against a device already in an
        // error state.
        bulk.release();
        std::thread::sleep(REOPEN_DELAY);
        let mut t = RusbBulk::open_device(raw.clone()).context("reopening device")?;
        t.detach_and_configure(name)
            .context("configuring reopened device")?;
        *bulk = t;
        Ok(())
    }

    /// Reopen the handle from the raw device. Used by the LCD driver's
    /// write-error recovery.
    pub(crate) fn reopen(&self, name: &str) -> Result<()> {
        let mut bulk = self.bulk.lock();
        self.reopen_locked(&mut bulk, name)
    }

    /// Write PushRgbData (or any command the panel goes quiet after) and
    /// bring the panel back: the packet goes out, the handle is closed and
    /// reopened, then GetVer is polled until the panel answers. Everything
    /// happens under one bulk guard so no stream chunk can interleave.
    ///
    /// Measured 2026-09-06 on a HydroShift II Square (fw 1.7): after the
    /// packet the panel stops answering bulk IN and does not come back on
    /// its own within 80 s, but answers GetVer 0.5 s after a reopen. Six
    /// cycles in a row all recovered in 3.1 s with the reopen at 2.5 s.
    ///
    /// `from_stream_thread` says the caller is the stream thread itself;
    /// any other caller is refused (`Ok(false)`) if a stream has begun in
    /// the meantime, and must queue the command instead.
    pub(crate) fn push_and_recover(
        &self,
        name: &str,
        cmds: &[PendingCmd],
        write_timeout: Duration,
        from_stream_thread: bool,
    ) -> Result<bool> {
        let mut bulk = self.bulk.lock();
        if !from_stream_thread && self.is_streaming() {
            return Ok(false);
        }
        let started = Instant::now();
        for cmd in cmds {
            debug!(
                "H2 ring: sending {} ({} bytes, queued {} ms ago)",
                cmd.label,
                cmd.packet.len(),
                cmd.queued_at.elapsed().as_millis()
            );
            bulk.write_full(&cmd.packet, write_timeout)
                .with_context(|| format!("H2 ring: {} write", cmd.label))?;
            let mut buf = [0u8; 512];
            match bulk.read(&mut buf, cmd.reply_wait) {
                Ok(n) if n > 0 => debug!(
                    "H2 ring: reply to {} ({n} bytes): {:02x?}",
                    cmd.label,
                    &buf[..n.min(8)]
                ),
                Ok(_) => debug!("H2 ring: no reply to {} (timeout)", cmd.label),
                Err(e) => debug!("H2 ring: no reply to {}: {e}", cmd.label),
            }
        }
        let pushed_at = Instant::now();
        *self.last_hold.lock() = Some(Instant::now());
        // The handle is released before the reopen, so on failure there is
        // nothing to poll; the driver's write-error recovery retries later.
        if let Err(e) = self.reopen_locked(&mut bulk, name) {
            self.set_needs_init(true);
            return Err(e.context("H2 ring: reopen after push"));
        }
        debug!("H2 ring: handle reopened");
        bulk.read_flush();
        let mut builder = PacketBuilder::new();
        let mut answered = None;
        while pushed_at.elapsed() < PANEL_SILENCE_BUDGET {
            let ver = builder.get_ver_header_winusb();
            if let Err(e) = bulk.write_full(&ver, write_timeout) {
                debug!("H2 ring: GetVer poll write failed: {e}");
            } else {
                let mut buf = [0u8; 512];
                if matches!(bulk.read(&mut buf, PANEL_POLL_READ), Ok(n) if n > 0) {
                    bulk.read_flush();
                    answered = Some(pushed_at.elapsed());
                    break;
                }
            }
            std::thread::sleep(PANEL_POLL_GAP);
        }
        self.set_needs_init(true);
        match answered {
            Some(after) => info!(
                "H2 ring: panel answering {} ms after the push, cycle took {} ms",
                after.as_millis(),
                started.elapsed().as_millis()
            ),
            None => warn!(
                "H2 ring: panel still silent {} s after the push and reopen",
                pushed_at.elapsed().as_secs()
            ),
        }
        Ok(true)
    }

    /// Queue a control command for the stream thread. Latest wins per label:
    /// an older SyncPumpFan still waiting is replaced, not appended.
    pub fn defer(&self, cmd: PendingCmd) {
        let mut q = self.pending.lock();
        q.retain(|c| c.label != cmd.label);
        q.push(cmd);
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.lock().is_empty()
    }

    /// Take only the commands that may be sent mid-stream.
    pub(crate) fn take_play_safe(&self) -> Vec<PendingCmd> {
        let mut q = self.pending.lock();
        let (safe, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut *q)
            .into_iter()
            .partition(|c| c.play_safe);
        *q = rest;
        safe
    }

    pub(crate) fn has_play_safe_pending(&self) -> bool {
        self.pending.lock().iter().any(|c| c.play_safe)
    }

    /// True if a command that must wait for a panel reinit is queued.
    pub(crate) fn has_unsafe_pending(&self) -> bool {
        self.pending.lock().iter().any(|c| !c.play_safe)
    }

    /// Take only the commands that need the panel reinitialised first.
    pub(crate) fn take_unsafe(&self) -> Vec<PendingCmd> {
        let mut q = self.pending.lock();
        let (unsafe_cmds, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut *q)
            .into_iter()
            .partition(|c| !c.play_safe);
        *q = rest;
        unsafe_cmds
    }

    pub(crate) fn oldest_play_safe_age(&self) -> Option<Duration> {
        self.pending
            .lock()
            .iter()
            .filter(|c| c.play_safe)
            .map(|c| c.queued_at.elapsed())
            .max()
    }
}

pub type SharedTransport = Arc<LcdLink>;

/// Buffer level at or below which queued control commands go out right away.
const CONTROL_SAFE_LEVEL: u8 = 1;
/// If the panel never drains that far, accept this level once a command has
/// waited `CONTROL_RELAX_AFTER`.
const CONTROL_RELAXED_LEVEL: u8 = 2;
const CONTROL_RELAX_AFTER: Duration = Duration::from_secs(3);

const REOPEN_DELAY: Duration = Duration::from_millis(100);
/// Gap between the wake-preamble commands (StopPlay, StopClock, GetVer).
const WAKE_STEP: Duration = Duration::from_millis(150);
/// After a PushRgbData and reopen: how long to poll GetVer for the panel
/// to answer, the gap between polls, and each poll's reply wait.
const PANEL_SILENCE_BUDGET: Duration = Duration::from_secs(10);
const PANEL_POLL_GAP: Duration = Duration::from_millis(250);
const PANEL_POLL_READ: Duration = Duration::from_millis(500);
/// Minimum spacing between push-and-recover cycles while streaming.
const HOLD_MIN_INTERVAL: Duration = Duration::from_secs(3);
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
    pub(crate) initialized: bool,
    pub(crate) consecutive_failures: u32,
    pub(crate) h264_chunk_size: usize,
    pub(crate) device_gone: bool,
    pub(crate) firmware: Option<String>,
}

/// Read the serial the kernel cached at enumeration, matching on bus/device
/// number. Preferred over a live EP0 read: it cannot stall.
fn sysfs_serial(bus: u8, address: u8) -> Option<String> {
    let entries = std::fs::read_dir("/sys/bus/usb/devices").ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let rd = |f: &str| std::fs::read_to_string(path.join(f)).ok();
        let (Some(b), Some(d)) = (rd("busnum"), rd("devnum")) else {
            continue;
        };
        if b.trim().parse::<u8>().ok() != Some(bus) || d.trim().parse::<u8>().ok() != Some(address)
        {
            continue;
        }
        let serial = rd("serial")?;
        let serial = serial.trim();
        if !serial.is_empty() {
            tracing::debug!("using kernel-cached serial {serial}");
            return Some(serial.to_string());
        }
    }
    None
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
        // FIX: some units stop answering EP0 string-descriptor requests while
        // their bulk pipe keeps working, so the live read fails and the device
        // silently gets a positional id. That breaks config lookup (which keys
        // on the serial) and makes the daemon re-open an already-claimed
        // interface. The kernel cached the serial at enumeration time, so fall
        // back to sysfs before giving up on identity.
        // Ask the kernel first. It read the serial at enumeration, so this is a
        // cheap file read that always works. Going to the device instead costs
        // ~5s of EP0 timeouts on units that stop answering string-descriptor
        // requests, and that delay widens the window in which a second open
        // thread races this one and hits EBUSY on interface 0.
        let serial = sysfs_serial(bus, address)
            .or_else(|| {
                device
                    .open()
                    .and_then(|h| h.read_serial_number_string_ascii(&desc))
                    .ok()
            })
            .unwrap_or_else(|| format!("bus{bus}-addr{address}"));

        let mut transport = RusbBulk::open_device(device.clone()).context("opening WinUSB LCD")?;
        transport
            .detach_and_configure(name)
            .context("configuring WinUSB LCD")?;

        info!(
            "{name} opened: {}x{} at bus {bus} addr {address} serial {serial}",
            screen.width, screen.height
        );

        Ok(Self {
            transport: Arc::new(LcdLink::new(transport, Some(device.clone()))),
            builder: PacketBuilder::new(),
            screen,
            write_timeout,
            read_timeout,
            name: name.to_string(),
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

    /// `tx_write_full`, timing only the USB write itself. The lock is taken
    /// first so a busy `SharedTransport` is not misreported as panel NAK
    /// backpressure.
    #[inline]
    fn tx_write_full_timed(
        &self,
        data: &[u8],
        what: &str,
    ) -> std::result::Result<(), lianli_transport::TransportError> {
        let tx = self.transport.lock();
        let started = Instant::now();
        let result = tx.write_full(data, self.write_timeout);
        let took = started.elapsed();
        if took > SLOW_CHUNK_WRITE {
            // Device NAK stall: the panel is back-pressuring. Visible at WARN
            // so field logs show stalls that the write timeout absorbed.
            warn!(
                "{what} stalled {} ms ({} bytes, result {:?})",
                took.as_millis(),
                data.len(),
                result.as_ref().map(|_| ()).map_err(|e| e.to_string())
            );
        }
        result
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

    /// True after the control channel reopened the handle (see
    /// `LcdLink::push_and_recover`); the driver must rerun its init.
    pub(crate) fn needs_init(&self) -> bool {
        self.transport.needs_init()
    }

    /// Vendor-faithful recovery: close the handle and reopen it from the raw
    /// device (ReInitDev), so a stalled endpoint is recovered within the
    /// session without a USB port reset (which would take down composite
    /// siblings like the LED MCU). Falls back to clear_halt when no raw device
    /// is available (shared-transport path).
    fn try_recover(&mut self) -> Result<()> {
        if lianli_transport::usb::shutting_down() {
            bail!("shutting down; skipping recovery");
        }
        if self.device_gone {
            bail!("device handle is stale; re-discovery required");
        }
        self.consecutive_failures += 1;

        if self.transport.can_reopen() {
            match self.transport.reopen(&self.name) {
                Ok(()) => {
                    self.consecutive_failures = 0;
                    debug!("recovered via close+reopen");
                    return Ok(());
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

    /// HydroShift II control-channel init: GetVer, frame rate, SyncClock,
    /// StopClock. Run by `H2WinUsbLcd::do_init` after enumeration and again
    /// by `reinit_and_flush_unsafe` after a stream.
    pub(crate) fn h2_control_init(&mut self) {
        self.read_firmware();
        // FIX: this AIO never answers GetVer and set_frame_rate can fail
        // transiently. The `?` aborted do_init and left the shared control
        // channel unusable, taking fans and RGB down with it. Degrade instead.
        if let Err(e) = self.set_frame_rate(30) {
            warn!("set_frame_rate failed, continuing anyway: {e:#}");
        }
        let sync = self.builder.sync_clock_header_winusb(2);
        self.send_command(sync, "SyncClock");
        let stop_clock = self.builder.stop_clock_header_winusb();
        self.send_command(stop_clock, "StopClock");
        self.transport.set_needs_init(false);
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
    ///
    /// Returns the last buffer level read, if any.
    pub(crate) fn wait_buffer(&mut self, threshold: u8, stop: Option<&AtomicBool>) -> Option<u8> {
        let mut iter = 0u32;
        let mut last = None;
        loop {
            if let Some(s) = stop {
                if s.load(Ordering::Relaxed) {
                    return last;
                }
            } else if iter >= WAIT_BUFFER_NO_STOP_CAP {
                debug!("Buffer wait capped after {} polls", WAIT_BUFFER_NO_STOP_CAP);
                return last;
            }
            iter += 1;
            match self.query_buffer_level() {
                Some(level) if level <= threshold => return Some(level),
                Some(level) => {
                    last = Some(level);
                    std::thread::sleep(WAIT_BUFFER_POLL)
                }
                None => {
                    debug!("Buffer wait aborted (no response)");
                    return last;
                }
            }
        }
    }

    /// Send play-safe control commands queued by the H2 AIO channel while we
    /// stream. Only called from the stream thread, which is the sole writer
    /// while `streaming` is set, so each reply here belongs to the command
    /// just sent.
    fn flush_pending_control(&mut self, level: Option<u8>) {
        if !self.transport.has_play_safe_pending() {
            return;
        }
        let safe = match level {
            Some(l) if l <= CONTROL_SAFE_LEVEL => true,
            Some(l) if l <= CONTROL_RELAXED_LEVEL => self
                .transport
                .oldest_play_safe_age()
                .is_some_and(|age| age >= CONTROL_RELAX_AFTER),
            _ => false,
        };
        if !safe {
            return;
        }
        for cmd in self.transport.take_play_safe() {
            debug!(
                "Sending deferred {} ({} bytes, waited {} ms, level {:?})",
                cmd.label,
                cmd.packet.len(),
                cmd.queued_at.elapsed().as_millis(),
                level
            );
            if let Err(e) = self.send_deferred(&cmd) {
                warn!("Deferred {} write failed: {e}", cmd.label);
            }
        }
    }

    /// Write one deferred control packet and discard its reply, holding the
    /// transport across both halves. Taking the lock twice would let another
    /// exchange in between, and the reply read here would consume an answer
    /// belonging to that command — the failure GetH2Params was made atomic for.
    fn send_deferred(
        &self,
        cmd: &PendingCmd,
    ) -> std::result::Result<(), lianli_transport::TransportError> {
        let transport = self.transport.lock();
        transport.write_full(&cmd.packet, self.write_timeout)?;
        let mut buf = [0u8; 512];
        match transport.read(&mut buf, cmd.reply_wait) {
            Ok(n) if n > 0 => debug!(
                "Reply to deferred {} ({n} bytes): {:02x?}",
                cmd.label,
                &buf[..n.min(16)]
            ),
            Ok(_) => debug!("No reply to deferred {} (timeout)", cmd.label),
            Err(e) => debug!("Reply read after deferred {} failed: {e}", cmd.label),
        }
        Ok(())
    }

    /// Send the queued PushRgbData mid-stream and bring the panel back,
    /// from the stream thread. StopPlay and StopClock
    /// first, then `LcdLink::push_and_recover` (push, reopen, wait for
    /// GetVer), then the H2 init again. The caller resumes the stream.
    /// Cycles are spaced by `HOLD_MIN_INTERVAL`; a command that arrives
    /// sooner stays queued for the next chunk.
    pub(crate) fn reinit_and_flush_unsafe(&mut self, force: bool) -> Result<bool> {
        if !self.transport.has_unsafe_pending() || !(force || self.transport.hold_allowed()) {
            return Ok(false);
        }
        let cmds = self.transport.take_unsafe();
        info!(
            "H2 ring: stopping play for {} queued command(s)",
            cmds.len()
        );
        let started = Instant::now();
        let stop = self.builder.stop_play_header_winusb();
        self.send_command(stop, "StopPlay");
        std::thread::sleep(WAKE_STEP);
        let stop_clock = self.builder.stop_clock_header_winusb();
        self.send_command(stop_clock, "StopClock");
        std::thread::sleep(WAKE_STEP);
        if let Err(e) = self
            .transport
            .push_and_recover(&self.name, &cmds, self.write_timeout, true)
        {
            // Nothing confirmed delivered: put the commands back so the
            // next stream start or control-channel write retries them.
            for cmd in cmds {
                self.transport.defer(cmd);
            }
            return Err(e);
        }
        self.h2_control_init();
        info!(
            "H2 ring: push and reinit done in {} ms",
            started.elapsed().as_millis()
        );
        Ok(true)
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

        match self.tx_write_full_timed(&packet, "H264 chunk write") {
            Ok(_) => self.note_write_success(),
            Err(e) => {
                // The write is refused on purpose once shutdown starts, so
                // unwinding quietly is the expected outcome, not a fault
                // that recovery should chase.
                if lianli_transport::usb::shutting_down() {
                    debug!("H264 chunk write refused during shutdown: {e}");
                    bail!("shutting down");
                }
                warn!("H264 chunk write failed: {e}");
                self.try_recover()
                    .with_context(|| format!("recovering from h264 write error: {e}"))?;
                self.tx_write_full_timed(&packet, "H264 chunk write retry")
                    .context("h264 chunk write after recovery")?;
                self.note_write_success();
            }
        }

        let resp = self.read_response("h264 chunk");
        let mut level = resp.map(|buf| buf[8]);
        if let Some(buf) = resp {
            if buf[8] > 3 {
                level = self.wait_buffer(2, Some(stop)).or(level);
            }
        }
        self.flush_pending_control(level);
        Ok(())
    }

    /// Mark the start of an H.264 stream: control writers defer to us.
    /// The flag is flipped while holding the bulk mutex, and control
    /// writers recheck it under the same mutex, so a writer that observed
    /// not streaming either finishes its write before any stream chunk or
    /// sees the flag flip and queues instead. It can never land a control
    /// packet mid stream.
    fn stream_begin(&self) {
        let _bulk = self.transport.lock();
        self.transport.set_streaming(true);
    }

    /// Mark the end of a stream and flush the play-safe commands the control
    /// channel queued while it ran. After an error the queue is dropped
    /// rather than hammering a device that just stopped answering.
    ///
    /// Commands that are unsafe in play mode go through the reinit
    /// sequence instead of straight onto the wire: the host stopping the
    /// feed does not idle the panel, and a PushRgbData sent here wedged
    /// the MCU twice on 2026-09-06, once straight after the feed stopped
    /// and once after an acknowledged StopPlay.
    fn stream_end(&mut self, clean: bool) {
        {
            let _bulk = self.transport.lock();
            self.transport.set_streaming(false);
        }
        if !clean {
            // Play-safe commands are resent every tick anyway; a queued
            // ring write is kept for the next stream start or the control
            // channel, so a later identical write is not deduplicated away.
            self.transport.take_play_safe();
            return;
        }
        for cmd in self.transport.take_play_safe() {
            debug!("Sending deferred {} after stream end", cmd.label);
            if let Err(e) = self.send_deferred(&cmd) {
                warn!("Deferred {} write failed: {e}", cmd.label);
            }
        }
        // No stream left to protect, so the spacing does not apply here.
        if let Err(e) = self.reinit_and_flush_unsafe(true) {
            warn!("Panel reinit at stream end failed: {e:#}");
        }
    }

    /// Mid-stream hold: a command that cannot go out in play mode is
    /// waiting, so stop play, reinitialise, send it, and let the caller
    /// carry on streaming. Returns true if a hold happened.
    fn hold_for_unsafe_pending(&mut self) -> Result<bool> {
        self.reinit_and_flush_unsafe(false)
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
        let mut file = std::fs::File::open(path).context("opening h264 file")?;
        let mut file_buf = vec![0u8; self.h264_chunk_size];
        let interval = chunk_interval(fps);
        let mut next_deadline = Instant::now() + interval;

        // A ring write queued while the panel sat idle after an earlier
        // stream goes out now, before play starts, via the same reinit path.
        self.reinit_and_flush_unsafe(false)?;
        self.stream_begin();
        let result = self.stream_h264_inner(
            &mut file,
            &mut file_buf,
            looping,
            stop,
            interval,
            &mut next_deadline,
            play_count,
            play_tick,
        );
        self.stream_end(result.is_ok());
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn stream_h264_inner(
        &mut self,
        file: &mut std::fs::File,
        file_buf: &mut [u8],
        looping: bool,
        stop: &AtomicBool,
        interval: Duration,
        next_deadline: &mut Instant,
        play_count: u8,
        play_tick: u32,
    ) -> Result<()> {
        use std::io::{Read, Seek};
        loop {
            let n = file.read(file_buf).context("reading h264 chunk")?;
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
            if self.hold_for_unsafe_pending()? {
                // Play was stopped and the panel reinitialised; restart the
                // clip from its first keyframe rather than resuming mid-GOP.
                file.seek(std::io::SeekFrom::Start(0))?;
                *next_deadline = Instant::now() + interval;
                continue;
            }
            sleep_until(next_deadline, interval);
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
        self.reinit_and_flush_unsafe(false)?;
        self.stream_begin();
        let result = (|| -> Result<()> {
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
                // Live feed: cannot rewind, the decoder resyncs at the next
                // keyframe after a hold.
                self.hold_for_unsafe_pending()?;
            }
            Ok(())
        })();
        self.stream_end(result.is_ok());
        self.tx_read_flush();
        self.initialized = false;
        result
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
