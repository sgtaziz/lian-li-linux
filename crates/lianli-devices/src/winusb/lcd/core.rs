use super::playback::{LcdTransferInterrupted, PlaybackState};
use super::ring::RingDelivery;
use crate::crypto::PacketBuilder;
use anyhow::{bail, Context, Result};
use lianli_shared::screen::ScreenInfo;
use lianli_transport::usb::{RusbBulk, EP_IN, EP_OUT};
use lianli_transport::UsbContext;
use parking_lot::{Mutex, MutexGuard};
use rusb::Device;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
    /// Ring payload identity of a PushRgbData, recorded by the link once the
    /// packet is actually on the wire so identical applies can be skipped.
    pub ring_key: Option<(Vec<u8>, u8)>,
    pub cancelled: Option<Arc<AtomicBool>>,
}

impl PendingCmd {
    fn is_cancelled(&self) -> bool {
        self.cancelled
            .as_ref()
            .is_some_and(|stop| stop.load(Ordering::Acquire))
    }
}

fn remaining_timeout(deadline: Instant, maximum: Duration) -> Result<Duration> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    anyhow::ensure!(!remaining.is_zero(), "H2 transaction deadline expired");
    Ok(remaining.min(maximum))
}

fn valid_firmware_reply(reply: &[u8]) -> bool {
    reply.first() == Some(&0x0a)
        && reply.get(8..40).is_some_and(|version| {
            let text = version.split(|byte| *byte == 0).next().unwrap_or_default();
            !text.is_empty()
                && text
                    .iter()
                    .all(|byte| byte.is_ascii_graphic() || *byte == b' ')
        })
}

fn queue_control(queue: &mut Vec<PendingCmd>, mut command: PendingCmd) {
    if command.is_cancelled() {
        return;
    }
    if let Some(previous) = queue.iter().find(|pending| pending.label == command.label) {
        if command.ring_key.is_some()
            && !previous.is_cancelled()
            && previous.queued_at > command.queued_at
        {
            return;
        }
        // Repeated PWM updates must not postpone the safe-window relaxation deadline.
        if command.play_safe {
            command.queued_at = previous.queued_at;
        }
    }
    queue.retain(|pending| pending.label != command.label);
    queue.push(command);
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
    raw_device: Option<Device<UsbContext>>,
    streaming: AtomicBool,
    h264_chunk_size: NegotiatedH264ChunkSize,
    /// Set by `push_and_recover`: the handle was reopened behind the LCD
    /// driver's back, so it must rerun its init before the next frame.
    needs_init: AtomicBool,
    /// When the last push-and-recover cycle ran, to space cycles out.
    last_hold: Mutex<Option<Instant>>,
    ring: Mutex<RingDelivery>,
    software_cooling: AtomicBool,
    storage_interrupted: AtomicBool,
    pending: Mutex<Vec<PendingCmd>>,
}

impl LcdLink {
    pub fn new(bulk: RusbBulk, raw_device: Option<Device<UsbContext>>) -> Self {
        Self {
            bulk: Mutex::new(bulk),
            raw_device,
            streaming: AtomicBool::new(false),
            h264_chunk_size: NegotiatedH264ChunkSize::default(),
            needs_init: AtomicBool::new(false),
            last_hold: Mutex::new(None),
            ring: Mutex::new(RingDelivery::default()),
            software_cooling: AtomicBool::new(true),
            storage_interrupted: AtomicBool::new(false),
            pending: Mutex::new(Vec::new()),
        }
    }

    pub fn lock(&self) -> MutexGuard<'_, RusbBulk> {
        self.bulk.lock()
    }

    pub fn ensure_storage_ready(&self) -> Result<()> {
        anyhow::ensure!(
            !self.storage_interrupted.load(Ordering::Acquire),
            "LCD storage transfer was interrupted; reconnect the device before further commands"
        );
        Ok(())
    }

    pub fn startup_image_ready(&self, h2: bool) -> Result<()> {
        self.ensure_storage_ready()?;
        anyhow::ensure!(
            !self.ring_recovery_pending(),
            "LCD recovery is still pending"
        );
        anyhow::ensure!(!h2 || !self.software_cooling_active(), "Switch H2 fan and pump control to motherboard or device-managed mode before storing a startup image");
        Ok(())
    }

    pub(crate) fn upload_startup_image(
        &self,
        packet: &[u8],
        stop: &AtomicBool,
        transfer: &crate::startup_image::Transfer,
    ) -> Result<bool> {
        let bulk = self
            .bulk
            .try_lock_for(Duration::from_millis(100))
            .context("LCD transport busy")?;
        crate::startup_image::ensure_not_cancelled(stop)?;
        self.startup_image_ready(false)?;
        anyhow::ensure!(
            !self.is_streaming() && !self.ring_recovery_pending(),
            "LCD playback or recovery is still active"
        );
        self.set_needs_init(true);
        lianli_transport::usb::with_teardown_io(Duration::from_secs(5), || {
            transfer.begin(stop)?;
            self.storage_interrupted.store(true, Ordering::Release);
            anyhow::ensure!(
                bulk.write(packet, Duration::from_secs(3))
                    .context("Startup image transfer interrupted. Storage state is unknown")?
                    == packet.len(),
                "Startup image transfer incomplete. Storage state is unknown"
            );
            let mut reply = [0; 512];
            let response_received = bulk
                .read(&mut reply, Duration::from_secs(1))
                .is_ok_and(|length| length > 0);
            self.storage_interrupted.store(false, Ordering::Release);
            Ok(response_received)
        })
    }

    pub(crate) fn probe_relative_startup_path(&self, command: &[u8]) -> Result<bool> {
        self.ensure_storage_ready()?;
        let bulk = self
            .bulk
            .try_lock_for(Duration::from_millis(100))
            .context("LCD transport busy during startup revision probe")?;
        anyhow::ensure!(
            bulk.write(command, Duration::from_millis(200))? == command.len(),
            "Short LCD revision probe write"
        );
        let mut reply = [0; 512];
        let length = match bulk.read(&mut reply, Duration::from_millis(10)) {
            Ok(length) => length,
            Err(lianli_transport::TransportError::Usb(rusb::Error::Timeout)) => 0,
            Err(error) => return Err(error.into()),
        };
        Ok(crate::startup_image::revision(&reply[..length]).unwrap_or(false))
    }

    /// Last valid device-reported block size; absent until negotiation and after reopen.
    pub fn negotiated_h264_chunk_size(&self) -> Option<usize> {
        self.h264_chunk_size.get()
    }

    /// Negotiated block size, or the established transfer default before negotiation.
    pub fn h264_chunk_size(&self) -> usize {
        self.negotiated_h264_chunk_size()
            .unwrap_or(DEFAULT_H264_CHUNK_SIZE)
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
        (!self.software_cooling_active() || self.ring_recovery_pending())
            && self
                .last_hold
                .lock()
                .is_none_or(|at| at.elapsed() >= HOLD_MIN_INTERVAL)
    }

    pub(crate) fn set_software_cooling_active(&self, active: bool) {
        self.software_cooling.store(active, Ordering::Release);
    }

    pub(crate) fn software_cooling_active(&self) -> bool {
        self.software_cooling.load(Ordering::Acquire)
    }

    pub(crate) fn ring_recovery_pending(&self) -> bool {
        self.ring.lock().attempted.is_some()
    }

    pub(crate) fn ring_retry_delay(&self) -> Duration {
        self.last_hold
            .lock()
            .map_or(Duration::from_millis(250), |at| {
                HOLD_MIN_INTERVAL
                    .saturating_sub(at.elapsed())
                    .max(Duration::from_millis(250))
            })
    }

    /// Close the bulk handle and reopen it from the raw device (the vendor
    /// driver's ReInitDev), swapping the new handle in under the guard the
    /// caller already holds. No USB port reset is involved: that would take
    /// down composite siblings like the LED MCU, and on the HydroShift II
    /// locks the header until a power cycle.
    fn reopen_locked_until(
        &self,
        bulk: &mut RusbBulk,
        name: &str,
        deadline: Instant,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<()> {
        let raw = self
            .raw_device
            .as_ref()
            .context("no raw device handle to reopen from")?;
        anyhow::ensure!(!cancelled(), "LCD reopen cancelled");
        // Release the old claims before opening a replacement, avoiding EBUSY.
        bulk.release();
        std::thread::sleep(remaining_timeout(deadline, REOPEN_DELAY)?);
        let mut t = RusbBulk::open_device(raw.clone()).context("reopening device")?;
        t.detach_and_configure_with_cancel(name, || Instant::now() >= deadline || cancelled())
            .context("configuring reopened device")?;
        if let Err(error) = t.clear_halts() {
            debug!("{name}: clearing endpoint halts after reopen failed: {error}");
        }
        *bulk = t;
        self.h264_chunk_size.clear();
        Ok(())
    }

    /// Reopen the handle from the raw device. Used by the LCD driver's
    /// write-error recovery.
    pub(crate) fn reopen(&self, name: &str, cancelled: &dyn Fn() -> bool) -> Result<()> {
        let mut bulk = self.bulk.lock();
        self.reopen_locked_until(
            &mut bulk,
            name,
            Instant::now() + Duration::from_secs(10),
            cancelled,
        )
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
        preamble: &[(&'static str, Vec<u8>)],
        cmds: &[PendingCmd],
        write_timeout: Duration,
        from_stream_thread: bool,
    ) -> Result<bool> {
        let mut bulk = self
            .bulk
            .try_lock_for(Duration::from_millis(100))
            .context("H2 transport busy; RGB recovery postponed")?;
        if !from_stream_thread && self.is_streaming() {
            return Ok(false);
        }
        let recovering = self.ring_recovery_pending();
        self.ensure_storage_ready()?;
        if !recovering && self.software_cooling_active() {
            return Ok(false);
        }
        if !recovering && cmds.iter().any(PendingCmd::is_cancelled) {
            return Ok(false);
        }
        anyhow::ensure!(
            !lianli_transport::usb::shutting_down(),
            "H2 RGB operation cancelled"
        );
        let deadline = Instant::now() + RING_TRANSACTION_BUDGET;
        *self.last_hold.lock() = Some(Instant::now());
        let mut write_error = None;
        if !recovering {
            for (_, packet) in preamble {
                bulk.write_full(packet, remaining_timeout(deadline, write_timeout)?)?;
                let mut reply = [0; 512];
                let _ = bulk.read(&mut reply, remaining_timeout(deadline, WAKE_REPLY_WAIT)?);
            }
            for cmd in cmds {
                if cmd.is_cancelled() {
                    return Ok(false);
                }
                let key = cmd
                    .ring_key
                    .clone()
                    .context("missing H2 ring payload identity")?;
                let timeout = remaining_timeout(deadline, write_timeout)?;
                self.ring.lock().begin(key);
                match lianli_transport::usb::with_teardown_io(timeout, || {
                    bulk.write_full(&cmd.packet, timeout)
                }) {
                    Ok(()) => self.ring.lock().written(),
                    Err(error) => {
                        write_error = Some(error);
                        break;
                    }
                }
                let mut reply = [0; 512];
                if let Ok(timeout) = remaining_timeout(deadline, cmd.reply_wait) {
                    let _ = bulk.read(&mut reply, timeout);
                }
            }
        }
        self.set_needs_init(true);
        // Once an upload starts, finish its bounded reopen even if its owner stops.
        // Leaving the panel in the known post-upload silent state is not cancellation.
        let recovered = lianli_transport::usb::with_teardown_io(
            deadline.saturating_duration_since(Instant::now()),
            || -> Result<()> {
                self.reopen_locked_until(&mut bulk, name, deadline, &|| false)?;
                bulk.read_flush();
                let mut builder = PacketBuilder::new();
                loop {
                    let request = builder.get_ver_header_winusb();
                    bulk.write_full(&request, remaining_timeout(deadline, write_timeout)?)?;
                    let mut response = [0; 512];
                    let length = bulk
                        .read(&mut response, remaining_timeout(deadline, PANEL_POLL_READ)?)
                        .unwrap_or(0);
                    if valid_firmware_reply(&response[..length]) {
                        self.ring.lock().recovered();
                        return Ok(());
                    }
                    std::thread::sleep(remaining_timeout(deadline, PANEL_POLL_GAP)?);
                }
            },
        );
        recovered.context(
            "H2 panel did not recover after RGB upload; payload will not be resent before recovery",
        )?;
        if let Some(error) = write_error {
            return Err(error).context("H2 RGB write incomplete; transport recovered");
        }
        let applied = self.ring.lock().applied.clone();
        Ok(cmds.iter().all(|cmd| cmd.ring_key == applied))
    }

    pub(crate) fn last_ring_payload(&self) -> Option<(Vec<u8>, u8)> {
        self.ring.lock().applied.clone()
    }

    pub(crate) fn ring_is_queued(&self, key: &(Vec<u8>, u8)) -> bool {
        self.ring.lock().in_flight.contains(key)
            || self
                .pending
                .lock()
                .iter()
                .any(|cmd| !cmd.is_cancelled() && cmd.ring_key.as_ref() == Some(key))
    }

    /// Queue a control command for the stream thread. Latest wins per label:
    /// an older SyncPumpFan still waiting is replaced, not appended.
    pub fn defer(&self, cmd: PendingCmd) {
        queue_control(&mut self.pending.lock(), cmd);
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
        self.pending
            .lock()
            .iter()
            .any(|c| !c.play_safe && !c.is_cancelled())
    }

    /// Take only the commands that need the panel reinitialised first.
    pub(crate) fn take_unsafe(&self) -> Vec<PendingCmd> {
        let mut q = self.pending.lock();
        let (unsafe_cmds, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut *q)
            .into_iter()
            .filter(|c| !c.is_cancelled())
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

const DEFAULT_H264_CHUNK_SIZE: usize = 202_752;
// Keep device-reported allocations within the largest supported LCD payload budget.
const MAX_H264_CHUNK_SIZE: usize = 1_048_576;

fn validate_h264_chunk_size(size: usize) -> Result<usize> {
    anyhow::ensure!(
        (4096..=MAX_H264_CHUNK_SIZE).contains(&size),
        "unsupported H264 block size {size}: streaming requires at least 4096 bytes"
    );
    Ok(size)
}

#[derive(Default)]
struct NegotiatedH264ChunkSize(AtomicUsize);

impl NegotiatedH264ChunkSize {
    fn get(&self) -> Option<usize> {
        let size = self.0.load(Ordering::Acquire);
        (size != 0).then_some(size)
    }

    fn clear(&self) {
        self.0.store(0, Ordering::Release);
    }

    fn update(&self, response: &[u8]) -> Option<usize> {
        let size = response
            .get(8..12)
            .map(|bytes| u32::from_be_bytes(bytes.try_into().expect("four-byte size")) as usize)
            .filter(|&size| size > 0 && size <= MAX_H264_CHUNK_SIZE);
        self.0.store(size.unwrap_or(0), Ordering::Release);
        size
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
/// Reply wait for the wake commands inside a push and recover cycle.
const WAKE_REPLY_WAIT: Duration = Duration::from_millis(100);
/// After a PushRgbData and reopen: how long to poll GetVer for the panel
/// to answer, the gap between polls, and each poll's reply wait.
const RING_TRANSACTION_BUDGET: Duration = Duration::from_secs(10);
const PANEL_POLL_GAP: Duration = Duration::from_millis(250);
const PANEL_POLL_READ: Duration = Duration::from_millis(500);
/// Minimum spacing between push-and-recover cycles while streaming.
const HOLD_MIN_INTERVAL: Duration = Duration::from_secs(3);
/// Chunk writes slower than this are logged: the panel NAKed bulk OUT.
const SLOW_CHUNK_WRITE: Duration = Duration::from_millis(100);
/// A panel that stalls again this soon after a resend escalates to the
/// daemon's bounded recreation instead of looping on reopen and resend.
const CHUNK_RESEND_COOLDOWN: Duration = Duration::from_secs(60);
const WAIT_BUFFER_POLL: Duration = Duration::from_millis(50);
const WAIT_BUFFER_NO_STOP_CAP: u32 = 600;

pub(crate) struct LcdResponse {
    bytes: [u8; 512],
    length: usize,
}

impl std::ops::Deref for LcdResponse {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.bytes[..self.length]
    }
}

fn buffer_level(response: &[u8]) -> Option<u8> {
    response.get(8).copied()
}

pub(crate) struct WinUsbLcdCore {
    pub(crate) h264_transferred: Option<Arc<AtomicBool>>,
    transport: SharedTransport,
    builder: PacketBuilder,
    screen: ScreenInfo,
    write_timeout: Duration,
    read_timeout: Duration,
    name: String,
    pub(crate) initialized: bool,
    pub(crate) consecutive_failures: u32,
    pub(crate) device_gone: bool,
    pub(crate) firmware: Option<String>,
    /// Frame rate the current stream asked for, reapplied after a reinit
    /// cycle since h2_control_init resets the panel to 30.
    stream_fps: Option<f32>,
    playback: PlaybackState,
    stop_play_supported: bool,
    resend_stalled_chunk: bool,
    last_chunk_resend: Option<Instant>,
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
        device: Device<UsbContext>,
        screen: ScreenInfo,
        name: &str,
        write_timeout: Duration,
        read_timeout: Duration,
        stop_play_supported: bool,
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
            h264_transferred: None,
            builder: PacketBuilder::new(),
            screen,
            write_timeout,
            read_timeout,
            name: name.to_string(),
            initialized: false,
            consecutive_failures: 0,
            device_gone: false,
            firmware: None,
            stream_fps: None,
            playback: PlaybackState::default(),
            stop_play_supported,
            resend_stalled_chunk: true,
            last_chunk_resend: None,
        })
    }

    pub(crate) fn from_shared(
        transport: SharedTransport,
        screen: ScreenInfo,
        name: String,
        write_timeout: Duration,
        read_timeout: Duration,
        stop_play_supported: bool,
    ) -> Self {
        Self {
            transport,
            h264_transferred: None,
            builder: PacketBuilder::new(),
            screen,
            write_timeout,
            read_timeout,
            name,
            initialized: false,
            consecutive_failures: 0,
            device_gone: false,
            firmware: None,
            stream_fps: None,
            playback: PlaybackState::default(),
            stop_play_supported,
            resend_stalled_chunk: true,
            last_chunk_resend: None,
        }
    }

    /// The vendor HydroShift II reopen reruns SetFrameRate and restarts its
    /// AIO message queue before replaying, so a bare resend is not equivalent.
    pub(crate) fn without_stalled_chunk_resend(mut self) -> Self {
        self.resend_stalled_chunk = false;
        self
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

    pub(crate) fn stop_playback(&mut self) -> Result<()> {
        let builder = &mut self.builder;
        let transport = &self.transport;
        let supported = self.stop_play_supported;
        let write_timeout = self.write_timeout;
        let read_timeout = self.read_timeout;
        self.playback.stop(|| {
            if !supported {
                return Ok(());
            }
            if lianli_transport::usb::shutting_down() {
                bail!("shutting down; playback teardown requires a bounded I/O permit");
            }
            let header = builder.stop_play_header_winusb();
            let bulk = transport
                .bulk
                .try_lock_for(Duration::from_millis(250))
                .context("LCD transport is busy while stopping playback")?;
            lianli_transport::usb::with_teardown_io(
                Duration::from_millis(2_500),
                || -> Result<()> {
                    bulk.write_full(&header, write_timeout)
                        .context("stopping LCD playback")?;
                    let mut response = [0u8; 512];
                    if let Err(error) = bulk.read(&mut response, read_timeout) {
                        debug!("StopPlay reply unavailable: {error}");
                    }
                    bulk.read_flush();
                    Ok(())
                },
            )?;
            drop(bulk);
            std::thread::sleep(WAKE_STEP);
            Ok(())
        })
    }

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
    fn try_recover(&mut self, stop: Option<&AtomicBool>) -> Result<()> {
        let cancelled = || {
            lianli_transport::usb::shutting_down()
                || stop.is_some_and(|stop| stop.load(Ordering::Acquire))
        };
        if cancelled() {
            bail!("LCD stopping; skipping recovery");
        }
        if self.device_gone {
            bail!("device handle is stale; re-discovery required");
        }
        self.consecutive_failures += 1;

        if self.transport.can_reopen() {
            match self.transport.reopen(&self.name, &cancelled) {
                Ok(()) => {
                    self.consecutive_failures = 0;
                    debug!("recovered via close+reopen");
                    return Ok(());
                }
                Err(e) if cancelled() => {
                    return Err(e.context("LCD stopping; recovery abandoned"));
                }
                Err(e) if device_unplugged(&e) => {
                    self.device_gone = true;
                    return Err(e);
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

    fn read_response(&mut self, context: &str) -> Option<LcdResponse> {
        let mut buf = [0u8; 512];
        match self.tx_read(&mut buf) {
            Ok(n) if n > 0 => {
                debug!(
                    "Response for {context} ({n} bytes): {:02x?}",
                    &buf[..n.min(32)]
                );
                self.tx_read_flush();
                return Some(LcdResponse {
                    bytes: buf,
                    length: n,
                });
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
                if let Err(rec_err) = self.try_recover(None) {
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
    pub(crate) fn h2_control_init(&mut self) -> Result<()> {
        self.transport.ensure_storage_ready()?;
        if self.transport.ring_recovery_pending() {
            anyhow::ensure!(
                self.transport.hold_allowed(),
                "H2 panel recovery is cooling down"
            );
            self.transport
                .push_and_recover(&self.name, &[], &[], self.write_timeout, true)?;
        }
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
        Ok(())
    }

    pub(crate) fn read_firmware(&mut self) {
        let ver = self.builder.get_ver_header_winusb();
        match self.tx_write_full(&ver) {
            Ok(_) => self.note_write_success(),
            Err(e) => warn!("GetVer write failed: {e}"),
        }
        if let Some(resp) = self.read_response("GetVer") {
            let fw_bytes = resp.get(8..40.min(resp.len())).unwrap_or_default();
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
        let header = self.builder.get_h264_block_header_winusb();
        let transport = Arc::clone(&self.transport);
        let bulk = transport.lock();
        transport.h264_chunk_size.clear();
        if bulk.write_full(&header, self.write_timeout).is_err() {
            return;
        }
        let mut response = [0u8; 512];
        match bulk.read(&mut response, self.read_timeout) {
            Ok(length) => match transport.h264_chunk_size.update(&response[..length]) {
                Some(size) => debug!("H264 chunk size from device: {size}"),
                None if response.get(8..12) == Some(&[0, 0, 0, 0]) && length >= 12 => {
                    debug!(
                        "Device requested the default H264 chunk size: {DEFAULT_H264_CHUNK_SIZE}"
                    );
                }
                None => warn!(
                    "Invalid GetH264Block response ({length} bytes, block size {:?})",
                    response[..length]
                        .get(8..12)
                        .map(|bytes| u32::from_be_bytes(bytes.try_into().expect("four-byte size")))
                ),
            },
            Err(error) => warn!("Read after GetH264Block failed: {error}"),
        }
        bulk.read_flush();
    }

    pub(crate) fn clear_png_cmd(&mut self) {
        let h = self.builder.clear_png_header_winusb();
        self.send_command(h, "ClearPng");
    }

    pub(crate) fn stop_clock_resp(&mut self) -> Option<LcdResponse> {
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
        self.stop_playback()?;
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
                self.try_recover(None)
                    .with_context(|| format!("recovering from frame write error: {e}"))?;
                self.tx_write_full(&packet)
                    .context("writing LCD frame after recovery")?;
                self.note_write_success();
            }
        }

        let resp = self.read_response("frame ack");
        if let Some(level) = resp.as_deref().and_then(buffer_level) {
            if level > 3 {
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
        self.stop_playback()?;
        self.stream_fps = Some(fps);
        let clamped = fps.round().clamp(1.0, self.screen.max_fps as f32) as u8;
        self.set_frame_rate(clamped)
    }

    pub(crate) fn switch_to_desktop_mode(&mut self) -> Result<()> {
        self.stop_playback()?;
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
                buffer_level(&buf[..n])
            }
            _ => {
                self.tx_read_flush();
                None
            }
        }
    }

    /// Poll QueryBlock at the vendor cadence, with a total wait bound.
    /// Returns the last observed level; callers must check the threshold.
    pub(crate) fn wait_buffer(&mut self, threshold: u8, stop: Option<&AtomicBool>) -> Option<u8> {
        let mut iter = 0u32;
        let mut last = None;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if lianli_transport::usb::shutting_down()
                || stop.is_some_and(|stop| stop.load(Ordering::Relaxed))
            {
                return last;
            }
            if iter >= WAIT_BUFFER_NO_STOP_CAP || Instant::now() >= deadline {
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
    pub(crate) fn reinit_and_flush_unsafe(&mut self, _force: bool) -> Result<bool> {
        if (!self.transport.has_unsafe_pending() && !self.transport.ring_recovery_pending())
            || !self.transport.hold_allowed()
        {
            return Ok(false);
        }
        if !self.transport.ring_recovery_pending() {
            self.stop_playback()?;
        }
        let cmds = self.transport.take_unsafe();
        self.transport.ring.lock().in_flight =
            cmds.iter().filter_map(|cmd| cmd.ring_key.clone()).collect();
        info!(
            "H2 ring: stopping play for {} queued command(s)",
            cmds.len()
        );
        let started = Instant::now();
        if !self.transport.ring_recovery_pending() {
            let stop_clock = self.builder.stop_clock_header_winusb();
            self.send_command(stop_clock, "StopClock");
            std::thread::sleep(WAKE_STEP);
        }
        let result =
            self.transport
                .push_and_recover(&self.name, &[], &cmds, self.write_timeout, true);
        if !matches!(result, Ok(true)) {
            for cmd in cmds {
                self.transport.defer(cmd);
            }
        }
        self.transport.ring.lock().in_flight.clear();
        result?;
        self.h2_control_init()?;
        // The init just reset the panel rate to 30, restore the stream rate
        if let Some(fps) = self.stream_fps {
            if let Err(e) = self.apply_stream_fps(fps) {
                warn!("reapplying stream fps after reinit failed: {e:#}");
            }
        }
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

        if lianli_transport::usb::shutting_down() {
            return Ok(());
        }
        let transport = Arc::clone(&self.transport);
        let timeout = self.write_timeout;
        let write_packet = || -> Result<()> {
            let bulk = transport.lock();
            let started = Instant::now();
            // Finish an accepted packet even if shutdown starts between short writes.
            let result = lianli_transport::usb::with_teardown_io(timeout, || {
                bulk.write_full(&packet, timeout)
            });
            if started.elapsed() > SLOW_CHUNK_WRITE {
                warn!(
                    "H264 chunk write stalled {} ms ({} bytes)",
                    started.elapsed().as_millis(),
                    packet.len()
                );
            }
            Ok(result?)
        };
        if let Err(error) = self.playback.write(write_packet) {
            if error.is::<LcdTransferInterrupted>() || !wedges_the_lcd(&error) {
                return Err(error);
            }
            if !self.resend_stalled_chunk
                || !resendable_write_error(&error)
                || stop.load(Ordering::Acquire)
            {
                return Err(error.context(LcdTransferInterrupted));
            }
            let now = Instant::now();
            if !chunk_resend_allowed(self.last_chunk_resend, now) {
                warn!(
                    "{}: H.264 packet write failed again within {}s of a resend ({error}); not resending",
                    self.name,
                    CHUNK_RESEND_COOLDOWN.as_secs()
                );
                return Err(error.context(LcdTransferInterrupted));
            }
            self.last_chunk_resend = Some(now);
            warn!(
                "{}: H.264 packet write failed ({error}); reopening the LCD and resending it once",
                self.name
            );
            self.try_recover(Some(stop)).map_err(tag_interrupted)?;
            anyhow::ensure!(
                !stop.load(Ordering::Acquire),
                "H.264 stream stopped during LCD recovery"
            );
            self.playback
                .resend(write_packet)
                .map_err(tag_interrupted)?;
            info!("{}: H.264 packet resent after reopening the LCD", self.name);
        }
        self.note_write_success();

        let resp = self.read_response("h264 chunk");
        if let Some(transferred) = &self.h264_transferred {
            transferred.store(true, Ordering::Release);
        }
        let mut level = resp
            .as_deref()
            .and_then(buffer_level)
            .or_else(|| self.query_buffer_level())
            .context("LCD buffer feedback unavailable or truncated; stopping H.264 playback")?;
        if level > 3 {
            level = self
                .wait_buffer(2, Some(stop))
                .filter(|level| *level <= 2)
                .context("LCD buffer did not drain; stopping H.264 playback")?;
        }
        self.flush_pending_control(Some(level));
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
    fn stream_end(&mut self, clean: bool, stop_playback: bool) -> Result<()> {
        self.stream_fps = None;
        self.initialized = false;
        let stopped = if stop_playback {
            lianli_transport::usb::with_teardown_io(Duration::from_secs(3), || self.stop_playback())
        } else {
            Ok(())
        };
        {
            let _bulk = self.transport.lock();
            self.transport.set_streaming(false);
        }
        if !clean || stopped.is_err() || lianli_transport::usb::shutting_down() {
            // Play-safe commands are resent every tick anyway; a queued
            // ring write is kept for the next stream start or the control
            // channel, so a later identical write is not deduplicated away.
            self.transport.take_play_safe();
            return stopped;
        }
        for cmd in self.transport.take_play_safe() {
            debug!("Sending deferred {} after stream end", cmd.label);
            if let Err(e) = self.send_deferred(&cmd) {
                warn!("Deferred {} write failed: {e}", cmd.label);
            }
        }
        stopped
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
        let mut file_buf = vec![0u8; validate_h264_chunk_size(self.transport.h264_chunk_size())?];
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
        // A finite upload can finish before the panel presents its buffered frames.
        let ended = self.stream_end(
            result.is_ok(),
            looping
                || stop.load(Ordering::Relaxed)
                || lianli_transport::usb::shutting_down()
                || result.is_err(),
        );
        result.and(ended)
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
        use std::io::Seek;
        let length = file.metadata()?.len();
        while !stop.load(Ordering::Relaxed) && !lianli_transport::usb::shutting_down() {
            let Some((n, is_last)) = read_stream_chunk(file, file_buf, looping, length)? else {
                break;
            };
            if stop.load(Ordering::Relaxed) {
                break;
            }
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
        let mut buf = vec![0u8; validate_h264_chunk_size(self.transport.h264_chunk_size())?];
        self.reinit_and_flush_unsafe(false)?;
        self.stream_begin();
        let result = (|| -> Result<()> {
            loop {
                if stop.load(Ordering::Relaxed) || lianli_transport::usb::shutting_down() {
                    break;
                }
                let n = reader
                    .read(&mut buf)
                    .context("WinUSB LCD: read h264 stream")?;
                if n == 0 {
                    break;
                }
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                self.send_h264_chunk(&buf[..n], false, play_count, play_tick, stop)?;
                // Live feed: cannot rewind, the decoder resyncs at the next
                // keyframe after a hold.
                self.hold_for_unsafe_pending()?;
            }
            Ok(())
        })();
        let ended = self.stream_end(result.is_ok(), true);
        self.tx_read_flush();
        self.initialized = false;
        result.and(ended)
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

fn device_unplugged(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<lianli_transport::TransportError>(),
        Some(lianli_transport::TransportError::Usb(rusb::Error::NoDevice))
    )
}

/// An unplugged device takes the detach path, and shutdown recreates nothing.
fn wedges_the_lcd(error: &anyhow::Error) -> bool {
    !lianli_transport::usb::shutting_down() && !device_unplugged(error)
}

fn tag_interrupted(error: anyhow::Error) -> anyhow::Error {
    if wedges_the_lcd(&error) {
        error.context(LcdTransferInterrupted)
    } else {
        error
    }
}

fn chunk_resend_allowed(last_resend: Option<Instant>, now: Instant) -> bool {
    last_resend.is_none_or(|at| now.saturating_duration_since(at) >= CHUNK_RESEND_COOLDOWN)
}

fn resendable_write_error(error: &anyhow::Error) -> bool {
    use lianli_transport::TransportError;
    !lianli_transport::usb::shutting_down()
        && matches!(
            error.downcast_ref::<TransportError>(),
            Some(
                TransportError::Timeout
                    | TransportError::Usb(
                        rusb::Error::Timeout | rusb::Error::Io | rusb::Error::Pipe
                    )
            )
        )
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

fn read_stream_chunk(
    reader: &mut (impl std::io::Read + std::io::Seek),
    buffer: &mut [u8],
    looping: bool,
    length: u64,
) -> Result<Option<(usize, bool)>> {
    let mut count = reader.read(buffer).context("reading H.264 chunk")?;
    if count == 0 {
        if !looping {
            return Ok(None);
        }
        reader.seek(std::io::SeekFrom::Start(0))?;
        count = reader.read(buffer).context("restarting H.264 loop")?;
        anyhow::ensure!(count > 0, "cannot loop an empty H.264 stream");
    }
    let last = !looping && reader.stream_position()? >= length;
    Ok(Some((count, last)))
}

#[cfg(test)]
mod file_stream_tests {
    #[test]
    fn cancelled_and_older_ring_requests_cannot_replace_the_latest_colour() {
        let now = Instant::now();
        let make = |value, queued_at, cancelled| PendingCmd {
            label: "PushRgbData",
            packet: vec![value],
            reply_wait: Duration::ZERO,
            queued_at,
            play_safe: false,
            ring_key: Some((vec![value], 1)),
            cancelled: Some(Arc::new(AtomicBool::new(cancelled))),
        };
        let mut queue = Vec::new();
        queue_control(&mut queue, make(2, now, false));
        queue_control(&mut queue, make(1, now - Duration::from_secs(1), false));
        queue_control(&mut queue, make(3, now + Duration::from_secs(1), true));
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].packet, vec![2]);
    }
    #[test]
    fn ring_recovery_requires_full_firmware_reply_and_a_live_deadline() {
        let mut response = [0; 512];
        response[0] = 0x0a;
        response[8..11].copy_from_slice(b"1.7");
        assert!(super::valid_firmware_reply(&response[..40]));
        assert!(!super::valid_firmware_reply(&response[..39]));
        response[0] = 0xfc;
        assert!(!super::valid_firmware_reply(&response));
        assert!(super::remaining_timeout(
            std::time::Instant::now(),
            std::time::Duration::from_secs(1)
        )
        .is_err());
    }
    #[test]
    fn replacement_cooling_commands_keep_the_original_wait_deadline() {
        use super::{queue_control, PendingCmd, CONTROL_RELAX_AFTER};
        use std::time::{Duration, Instant};
        let now = Instant::now();
        let mut queue = Vec::new();
        for second in 0..5 {
            queue_control(
                &mut queue,
                PendingCmd {
                    label: "SyncPumpFan",
                    packet: vec![second as u8],
                    reply_wait: Duration::from_millis(250),
                    queued_at: now + Duration::from_secs(second),
                    play_safe: true,
                    ring_key: None,
                    cancelled: None,
                },
            );
            assert_eq!(queue.len(), 1);
            assert_eq!(queue[0].packet, vec![second as u8]);
            assert_eq!(queue[0].queued_at, now);
        }
        assert!(now + Duration::from_secs(4) - queue[0].queued_at >= CONTROL_RELAX_AFTER);
    }
    use super::*;
    use std::io::Cursor;

    #[test]
    fn looping_chunks_never_signal_end_of_playback() {
        let mut source = Cursor::new(vec![1, 2, 3]);
        let mut buffer = [0; 2];
        assert_eq!(
            read_stream_chunk(&mut source, &mut buffer, true, 3).unwrap(),
            Some((2, false))
        );
        assert_eq!(buffer, [1, 2]);
        assert_eq!(
            read_stream_chunk(&mut source, &mut buffer, true, 3).unwrap(),
            Some((1, false))
        );
        assert_eq!(buffer[0], 3);
        assert_eq!(
            read_stream_chunk(&mut source, &mut buffer, true, 3).unwrap(),
            Some((2, false))
        );
        assert_eq!(buffer, [1, 2]);
    }

    #[test]
    fn finite_stream_signals_last_chunk_then_finishes() {
        let mut source = Cursor::new(vec![1, 2, 3]);
        let mut buffer = [0; 2];
        assert_eq!(
            read_stream_chunk(&mut source, &mut buffer, false, 3).unwrap(),
            Some((2, false))
        );
        assert_eq!(
            read_stream_chunk(&mut source, &mut buffer, false, 3).unwrap(),
            Some((1, true))
        );
        assert_eq!(
            read_stream_chunk(&mut source, &mut buffer, false, 3).unwrap(),
            None
        );
    }

    #[test]
    fn only_stalled_transfers_are_resent_after_reopen() {
        use lianli_transport::TransportError;
        let resendable = |error: TransportError| resendable_write_error(&error.into());
        assert!(resendable(TransportError::Usb(rusb::Error::Timeout)));
        assert!(resendable(TransportError::Usb(rusb::Error::Io)));
        assert!(resendable(TransportError::Usb(rusb::Error::Pipe)));
        assert!(!resendable(TransportError::Usb(rusb::Error::NoDevice)));
        assert!(!resendable(TransportError::Usb(rusb::Error::Interrupted)));
        assert!(!resendable(TransportError::Write("zero-length".into())));
        assert!(!resendable_write_error(&LcdTransferInterrupted.into()));
    }

    #[test]
    fn unplugged_device_is_not_reported_as_an_interrupted_transfer() {
        use lianli_transport::TransportError;
        let tagged = |error: TransportError| {
            tag_interrupted(anyhow::Error::from(error).context("reopening device"))
                .is::<LcdTransferInterrupted>()
        };
        assert!(!tagged(TransportError::Usb(rusb::Error::NoDevice)));
        assert!(tagged(TransportError::Usb(rusb::Error::Timeout)));
        assert!(tagged(TransportError::Usb(rusb::Error::Pipe)));
    }

    #[test]
    fn a_second_stall_within_the_cooldown_is_not_resent() {
        let resent = Instant::now();
        assert!(chunk_resend_allowed(None, resent));
        assert!(!chunk_resend_allowed(Some(resent), resent));
        assert!(!chunk_resend_allowed(
            Some(resent),
            resent + CHUNK_RESEND_COOLDOWN - Duration::from_millis(1)
        ));
        assert!(chunk_resend_allowed(
            Some(resent),
            resent + CHUNK_RESEND_COOLDOWN
        ));
    }

    #[test]
    fn empty_loop_returns_an_error_instead_of_spinning() {
        assert!(
            read_stream_chunk(&mut Cursor::new(Vec::<u8>::new()), &mut [0; 2], true, 0).is_err()
        );
    }
}

#[cfg(test)]
mod h264_negotiation_tests {
    #[test]
    fn short_replies_do_not_fabricate_empty_buffer_credit() {
        for length in 0..9 {
            let response = super::LcdResponse {
                bytes: [0; 512],
                length,
            };
            assert_eq!(response.len(), length);
            assert_eq!(super::buffer_level(&response), None);
        }
        let mut bytes = [0; 512];
        bytes[8] = 4;
        let response = super::LcdResponse { bytes, length: 9 };
        assert_eq!(super::buffer_level(&response), Some(4));
        bytes[8] = 0;
        assert_eq!(
            super::buffer_level(&super::LcdResponse { bytes, length: 9 }),
            Some(0)
        );
    }
    use super::*;

    fn response(size: u32) -> [u8; 12] {
        let mut response = [0; 12];
        response[8..12].copy_from_slice(&size.to_be_bytes());
        response
    }

    #[test]
    fn negotiated_size_uses_only_complete_bounded_device_values() {
        let state = NegotiatedH264ChunkSize::default();
        assert_eq!(state.get(), None);
        for size in [1, 32_768, DEFAULT_H264_CHUNK_SIZE, MAX_H264_CHUNK_SIZE] {
            assert_eq!(state.update(&response(size as u32)), Some(size));
            assert_eq!(state.get(), Some(size));
        }
        for size in [0, MAX_H264_CHUNK_SIZE as u32 + 1, u32::MAX] {
            assert_eq!(state.update(&response(size)), None);
            assert_eq!(state.get(), None);
        }
        let reply = response(DEFAULT_H264_CHUNK_SIZE as u32);
        for length in 0..12 {
            state.update(&reply);
            assert_eq!(state.update(&reply[..length]), None);
            assert_eq!(state.get(), None);
        }
    }

    #[test]
    fn tiny_negotiated_blocks_are_preserved_but_cannot_start_streaming() {
        let state = NegotiatedH264ChunkSize::default();
        for size in [1, 4, 4095] {
            assert_eq!(state.update(&response(size)), Some(size as usize));
            let effective = state.get().unwrap_or(DEFAULT_H264_CHUNK_SIZE);
            assert_eq!(effective, size as usize);
            assert!(validate_h264_chunk_size(effective).is_err());
        }
        for size in [4096, DEFAULT_H264_CHUNK_SIZE, MAX_H264_CHUNK_SIZE] {
            assert_eq!(validate_h264_chunk_size(size).unwrap(), size);
        }
    }

    #[test]
    fn shared_snapshot_is_invalidated_before_a_new_negotiation() {
        let state = Arc::new(NegotiatedH264ChunkSize::default());
        let publisher = state.clone();
        std::thread::spawn(move || {
            publisher.update(&response(65_536));
        })
        .join()
        .unwrap();
        assert_eq!(state.get(), Some(65_536));
        state.clear();
        assert_eq!(state.get(), None);
        assert_eq!(state.get().unwrap_or(DEFAULT_H264_CHUNK_SIZE), 202_752);
        assert_eq!(state.update(&response(32_768)), Some(32_768));
        assert_eq!(state.get(), Some(32_768));
    }
}
