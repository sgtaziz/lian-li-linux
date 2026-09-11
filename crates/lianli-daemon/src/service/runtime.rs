use super::renderers::{
    AsyncCustomH264Renderer, AsyncCustomRenderer, AsyncSensorH264Renderer, AsyncSensorRenderer,
    AsyncVideoPlayer,
};
use super::DaemonEvent;
use lianli_devices::crypto::PacketBuilder;
use lianli_devices::slv3_lcd::Slv3LcdDevice;
use lianli_devices::traits::LcdDevice;
use lianli_devices::winusb::lcd::WinUsbLcdDevice;
use lianli_devices::wireless::WirelessController;
use lianli_media::{MediaAsset, MediaAssetKind};
use lianli_shared::config::ConfigKey;
use lianli_shared::screen::ScreenInfo;
use parking_lot::Mutex;
use std::path::PathBuf;
use std::process::ChildStdout;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tracing::{debug, info, warn};

pub(super) type SharedHidLcd = Arc<HidLcd>;

pub(super) struct HidLcd {
    device: Mutex<Box<dyn LcdDevice>>,
    // Separate from the LCD mutex: recovery must not even take that mutex
    // while a worker is feeding H.264. Count overlapping replacement workers.
    //
    // Ungated device access is limited to non autonomous frame sends,
    // brightness applies and the init worker, only brightness can overlap
    // a live stream
    streams: Mutex<usize>,
}

impl HidLcd {
    pub(super) fn new(device: Box<dyn LcdDevice>) -> Self {
        Self {
            device: Mutex::new(device),
            streams: Mutex::new(0),
        }
    }

    /// Recovery may hold the gate for seconds; let the caller retry next tick.
    fn begin_stream(self: &Arc<Self>) -> Option<HidStreamLease> {
        *self.streams.try_lock()? += 1;
        Some(HidStreamLease {
            lcd: Arc::clone(self),
            released: Arc::new(AtomicBool::new(false)),
        })
    }

    pub(super) fn recovery_idle(&self) -> Option<parking_lot::MutexGuard<'_, usize>> {
        let streams = self.streams.try_lock()?;
        (*streams == 0).then_some(streams)
    }
}

impl std::ops::Deref for HidLcd {
    type Target = Mutex<Box<dyn LcdDevice>>;

    fn deref(&self) -> &Self::Target {
        &self.device
    }
}

/// Registered before spawning and owned by the worker until every exit path,
/// including unwinding. A detached old worker cannot clear a new one's state.
/// Release is idempotent so stop() can drop the gate early, a halted worker
/// never touches the device again.
struct HidStreamLease {
    lcd: SharedHidLcd,
    released: Arc<AtomicBool>,
}

impl Clone for HidStreamLease {
    fn clone(&self) -> Self {
        Self {
            lcd: Arc::clone(&self.lcd),
            released: Arc::clone(&self.released),
        }
    }
}

impl HidStreamLease {
    fn release(&self) {
        if !self.released.swap(true, Ordering::AcqRel) {
            *self.lcd.streams.lock() -= 1;
        }
    }
}

impl Drop for HidStreamLease {
    fn drop(&mut self) {
        self.release();
    }
}

// lock per access unit only, never for the whole stream
// no sane access unit comes close; malformed streams without a second
// boundary would otherwise grow `accum` without limit
const MAX_AU_BYTES: usize = 4 * 1024 * 1024;

/// Sends one access unit with three attempts, false when aborted or failed
fn send_h264_au_with_retry(lcd: &SharedHidLcd, au: &[u8], aborted: &dyn Fn() -> bool) -> bool {
    let mut last_err = None;
    for attempt in 1..=3 {
        if aborted() {
            return false;
        }
        let result = {
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            let mut guard = loop {
                if aborted() {
                    return false;
                }
                if let Some(guard) = lcd.try_lock_for(Duration::from_millis(50)) {
                    break guard;
                }
                if std::time::Instant::now() >= deadline {
                    warn!("HID h264 stream stopped: LCD remained busy for 3s");
                    return false;
                }
            };
            if aborted() {
                return false;
            }
            guard.send_h264_frame(au)
        };
        match result {
            Ok(()) => return true,
            Err(e) => {
                debug!("HID h264 send error (attempt {attempt}/3): {e:#}");
                last_err = Some(e);
                thread::sleep(Duration::from_millis(150));
            }
        }
    }
    if let Some(e) = last_err {
        warn!("HID h264 send failed after retries: {e:#}");
    }
    false
}

/// Returns the join handle plus the worker's private halt flag so a
/// replacement stream can stop this one promptly.
fn spawn_hid_h264_stream(
    lcd: SharedHidLcd,
    mut reader: Box<dyn std::io::Read + Send>,
    stop: Arc<AtomicBool>,
    fps: f32,
    lease: HidStreamLease,
) -> (JoinHandle<()>, Arc<AtomicBool>) {
    use lianli_devices::hydroshift_lcd::{find_au_split, pace_frame};
    use std::io::Read;
    use std::time::Instant;

    let halt = Arc::new(AtomicBool::new(false));
    let worker_halt = Arc::clone(&halt);
    let worker_stop = Arc::clone(&stop);
    let handle = thread::spawn(move || {
        let _lease = lease;
        let halted = || worker_stop.load(Ordering::Relaxed) || worker_halt.load(Ordering::Relaxed);
        let fps = loop {
            if halted() {
                return;
            }
            let Some(mut guard) = lcd.try_lock_for(Duration::from_millis(50)) else {
                continue;
            };
            if halted() {
                return;
            }
            break guard.set_stream_fps(fps);
        };
        let frame_interval = Duration::from_secs_f32(1.0 / fps);
        let mut read_buf = vec![0u8; 64 * 1024];
        let mut accum: Vec<u8> = Vec::with_capacity(256 * 1024);
        let mut next_deadline = Instant::now() + frame_interval;
        // residual data is only flushed on a clean EOF, never after a stop
        // or error exit where it may be a partial access unit
        let mut clean_eof = false;
        loop {
            if halted() {
                break;
            }
            let n = match reader.read(&mut read_buf) {
                Ok(n) => n,
                Err(e) => {
                    warn!("HID h264 stream read error: {e:#}");
                    break;
                }
            };
            if n == 0 {
                clean_eof = true;
                break;
            }
            accum.extend_from_slice(&read_buf[..n]);
            if accum.len() > MAX_AU_BYTES {
                warn!(
                    "HID h264 stream: no AU boundary within {} bytes, aborting",
                    MAX_AU_BYTES
                );
                return;
            }
            while let Some(split) = find_au_split(&accum) {
                let au: Vec<u8> = accum.drain(..split).collect();
                if au.is_empty() {
                    continue;
                }
                if !send_h264_au_with_retry(&lcd, &au, &halted) {
                    return;
                }
                pace_frame(&mut next_deadline, frame_interval);
                if halted() {
                    return;
                }
            }
        }
        if clean_eof && !halted() && !accum.is_empty() {
            pace_frame(&mut next_deadline, frame_interval);
            send_h264_au_with_retry(&lcd, &accum, &halted);
        }
    });
    (handle, halt)
}

pub(super) enum LcdBackend {
    Slv3(Slv3LcdDevice),
    WinUsb(ThreadedWinUsbSender),
    HidLcd(SharedHidLcd),
}

/// The LCD mutex could not be taken within the bounded wait, meaning the
/// init worker is holding it across its long settle and firmware retries.
/// Frame sends treat it as a retry later rather than an error so the
/// streaming thread never tears down a target that is merely initializing.
#[derive(Debug)]
struct LcdBusy;

impl std::fmt::Display for LcdBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LCD busy (initializing)")
    }
}
impl std::error::Error for LcdBusy {}

/// How long a frame send waits for the LCD mutex before deferring.
const LCD_BUSY_WAIT: Duration = Duration::from_millis(100);

impl LcdBackend {
    fn send_frame(
        &mut self,
        wireless: Option<&WirelessController>,
        builder: &mut PacketBuilder,
        frame: &[u8],
    ) -> anyhow::Result<()> {
        match self {
            Self::Slv3(d) => {
                if let Some(w) = wireless {
                    w.ensure_video_mode()?;
                }
                d.send_frame(builder, frame)
            }
            Self::WinUsb(d) => d.send_frame(frame),
            Self::HidLcd(d) => {
                let Some(mut guard) = d.try_lock_for(LCD_BUSY_WAIT) else {
                    return Err(LcdBusy.into());
                };
                guard.send_jpeg_frame(frame)
            }
        }
    }

    fn send_frame_verified(
        &mut self,
        wireless: Option<&WirelessController>,
        builder: &mut PacketBuilder,
        frame: &[u8],
    ) -> anyhow::Result<()> {
        match self {
            Self::WinUsb(d) => d.send_frame_verified(frame),
            Self::HidLcd(d) => {
                let Some(mut guard) = d.try_lock_for(LCD_BUSY_WAIT) else {
                    return Err(LcdBusy.into());
                };
                guard.send_static_frame(frame)
            }
            _ => self.send_frame(wireless, builder, frame),
        }
    }

    pub(super) fn set_brightness(
        &mut self,
        wireless: Option<&WirelessController>,
        builder: &mut PacketBuilder,
        brightness: u8,
    ) -> anyhow::Result<()> {
        match self {
            Self::Slv3(d) => {
                if let Some(w) = wireless {
                    w.ensure_video_mode()?;
                }
                d.set_brightness(builder, brightness)
            }
            Self::WinUsb(sender) => sender.set_brightness(brightness),
            Self::HidLcd(d) => d.lock().set_brightness(brightness),
        }
    }

    pub(super) fn start_h264_stream(
        &self,
        stdout: ChildStdout,
        stop: Arc<AtomicBool>,
        fps: f32,
    ) -> anyhow::Result<Option<HidStreamWorker>> {
        match self {
            Self::HidLcd(lcd) => Ok(Some(HidStreamWorker::new(
                Arc::clone(lcd),
                Box::new(stdout),
                stop,
                fps,
            ))),
            Self::WinUsb(sender) => {
                sender.stream_h264_reader(stdout, fps)?;
                Ok(None)
            }
            _ => anyhow::bail!("h264 streaming not supported on this backend"),
        }
    }

    /// Restart-capable handle for render threads; takes the initial worker
    /// so the first restart can stop it.
    pub(super) fn stream_restarter(
        &self,
        initial: Option<HidStreamWorker>,
    ) -> Option<StreamRestarter> {
        match self {
            Self::HidLcd(lcd) => Some(StreamRestarter::HidLcd(
                Arc::clone(lcd),
                Mutex::new(initial),
            )),
            Self::WinUsb(sender) => Some(StreamRestarter::WinUsb(
                sender.tx.clone(),
                Arc::clone(&sender.stream_control),
                Mutex::new(Arc::clone(&sender.stream_control.current.lock())),
            )),
            Self::Slv3(_) => None,
        }
    }
}

pub(super) struct HidStreamWorker {
    handle: Option<JoinHandle<()>>,
    halt: Arc<AtomicBool>,
    lease: Option<HidStreamLease>,
    pending: Option<PendingHidStream>,
}

struct PendingHidStream {
    lcd: SharedHidLcd,
    reader: Box<dyn std::io::Read + Send + Sync>,
    stop: Arc<AtomicBool>,
    fps: f32,
}

impl HidStreamWorker {
    fn new(
        lcd: SharedHidLcd,
        reader: Box<dyn std::io::Read + Send + Sync>,
        stop: Arc<AtomicBool>,
        fps: f32,
    ) -> Self {
        let mut worker = Self {
            handle: None,
            halt: Arc::new(AtomicBool::new(false)),
            lease: None,
            pending: Some(PendingHidStream {
                lcd,
                reader,
                stop,
                fps,
            }),
        };
        worker.try_start();
        worker
    }

    /// Keep the reader for the next render tick when recovery owns the gate.
    pub(super) fn try_start(&mut self) -> bool {
        let Some(pending) = self.pending.as_ref() else {
            return true;
        };
        let Some(lease) = pending.lcd.begin_stream() else {
            return false;
        };
        let pending = self.pending.take().unwrap();
        let (handle, halt) = spawn_hid_h264_stream(
            pending.lcd,
            pending.reader,
            pending.stop,
            pending.fps,
            lease.clone(),
        );
        self.handle = Some(handle);
        self.halt = halt;
        self.lease = Some(lease);
        true
    }

    /// The worker may be parked reading an encoder stdout that only EOFs
    /// once the caller replaces the encoder, so join is bounded.
    fn stop(&self, timeout: Duration) {
        self.halt.store(true, Ordering::Relaxed);
        // A halted worker never touches the device again, release the gate
        // now so a detached thread parked in its read cannot defer recovery
        if let Some(lease) = &self.lease {
            lease.release();
        }
        // No handle means the worker never spawned, the pending reader drops with it
        let Some(handle) = &self.handle else {
            return;
        };
        let deadline = std::time::Instant::now() + timeout;
        while !handle.is_finished() && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(25));
        }
        if !handle.is_finished() {
            debug!("h264 stream worker did not stop within {timeout:?}, detaching");
        }
    }
}

/// Cloneable handle for restarting an h264 stream from inside a render thread.
pub(super) enum StreamRestarter {
    HidLcd(SharedHidLcd, Mutex<Option<HidStreamWorker>>),
    WinUsb(
        std::sync::mpsc::SyncSender<LcdThreadMsg>,
        Arc<StreamControl>,
        Mutex<Arc<AtomicBool>>,
    ),
}

impl StreamRestarter {
    /// Called before feeding the encoder so a deferred reader cannot fill its pipe.
    pub(super) fn try_start_pending(&self) -> bool {
        match self {
            Self::HidLcd(_, current) => current
                .lock()
                .as_mut()
                .is_none_or(HidStreamWorker::try_start),
            Self::WinUsb(..) => true,
        }
    }

    /// Start a new h264 stream reading from the given stdout. The old
    /// stream is halted and joined (bounded) first.
    pub(super) fn start_stream(
        &self,
        stdout: ChildStdout,
        stop: Arc<AtomicBool>,
        fps: f32,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(!stop.load(Ordering::Acquire), "H.264 renderer stopped");
        match self {
            Self::HidLcd(lcd, current) => {
                let mut current = current.lock();
                if let Some(old) = current.take() {
                    old.stop(Duration::from_secs(1));
                }
                *current = Some(HidStreamWorker::new(
                    Arc::clone(lcd),
                    Box::new(stdout),
                    stop,
                    fps,
                ));
                Ok(())
            }
            Self::WinUsb(tx, control, owner) => {
                let mut owner = owner.lock();
                let stream_stop = control
                    .restart(&owner)
                    .ok_or_else(|| anyhow::anyhow!("H.264 stream was replaced"))?;
                *owner = stream_stop.clone();
                tx.try_send(LcdThreadMsg::StreamH264Reader(stdout, fps, stream_stop))
                    .map_err(|e| anyhow::anyhow!("LCD stream restart was not accepted: {e}"))
            }
        }
    }
}

pub(super) enum LcdThreadMsg {
    Frame(Vec<u8>),
    FrameVerified(Vec<u8>, std::sync::mpsc::SyncSender<anyhow::Result<()>>),
    StreamH264 {
        path: PathBuf,
        looping: bool,
        fps: f32,
        stop: Arc<AtomicBool>,
    },
    StreamH264Reader(std::process::ChildStdout, f32, Arc<AtomicBool>),
    SwitchDesktop(std::sync::mpsc::SyncSender<anyhow::Result<()>>),
    SetBrightness(u8),
    Shutdown(std::sync::mpsc::SyncSender<anyhow::Result<()>>),
    Stop,
}

#[derive(Default)]
pub(super) struct StreamControl {
    current: Mutex<Arc<AtomicBool>>,
}

impl StreamControl {
    fn next(&self) -> Arc<AtomicBool> {
        let mut current = self.current.lock();
        current.store(true, Ordering::Release);
        *current = Arc::new(AtomicBool::new(false));
        Arc::clone(&current)
    }

    fn cancel(&self) {
        self.current.lock().store(true, Ordering::Release);
    }

    fn restart(&self, expected: &Arc<AtomicBool>) -> Option<Arc<AtomicBool>> {
        let mut current = self.current.lock();
        if !Arc::ptr_eq(&current, expected) || expected.load(Ordering::Acquire) {
            return None;
        }
        current.store(true, Ordering::Release);
        *current = Arc::new(AtomicBool::new(false));
        Some(Arc::clone(&current))
    }
}

pub(super) struct ThreadedWinUsbSender {
    transport: Option<lianli_devices::winusb::lcd::SharedTransport>,
    tx: std::sync::mpsc::SyncSender<LcdThreadMsg>,
    stream_control: Arc<StreamControl>,
    closing: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ThreadedWinUsbSender {
    pub(super) fn new(mut device: WinUsbLcdDevice, index: usize) -> Self {
        let transport = Some(device.shared_transport());
        let (tx, rx) = std::sync::mpsc::sync_channel::<LcdThreadMsg>(2);
        let stream_control = Arc::new(StreamControl::default());
        let closing = Arc::new(AtomicBool::new(false));
        let closing_clone = Arc::clone(&closing);
        let thread = thread::spawn(move || {
            for msg in rx {
                if closing_clone.load(Ordering::Acquire)
                    && !matches!(msg, LcdThreadMsg::Shutdown(_) | LcdThreadMsg::Stop)
                {
                    continue;
                }
                match msg {
                    LcdThreadMsg::Frame(data) => {
                        if let Err(e) = device.send_frame(&data) {
                            if lianli_transport::usb::shutting_down() {
                                debug!("LCD[{index}] frame send refused during shutdown: {e:#}");
                            } else {
                                warn!("LCD[{index}] sender thread frame error: {e}");
                            }
                        }
                    }
                    LcdThreadMsg::FrameVerified(data, reply) => {
                        let result = device.send_frame_verified(&data);
                        let _ = reply.send(result);
                    }
                    LcdThreadMsg::StreamH264 {
                        path,
                        looping,
                        fps,
                        stop,
                    } => {
                        if stop.load(Ordering::Acquire) || closing_clone.load(Ordering::Acquire) {
                            continue;
                        }
                        if let Err(e) = device.stream_h264(&path, looping, &stop, fps) {
                            if lianli_transport::usb::shutting_down() {
                                debug!("LCD[{index}] h264 stream ended by shutdown: {e:#}");
                            } else {
                                warn!("LCD[{index}] h264 stream error: {e}");
                            }
                        }
                    }
                    LcdThreadMsg::StreamH264Reader(mut stdout, fps, stop) => {
                        if stop.load(Ordering::Acquire) || closing_clone.load(Ordering::Acquire) {
                            continue;
                        }
                        if let Err(e) = device.stream_h264_reader(&mut stdout, &stop, fps) {
                            if lianli_transport::usb::shutting_down() {
                                debug!("LCD[{index}] h264 live stream ended by shutdown: {e:#}");
                            } else {
                                warn!("LCD[{index}] h264 live stream error: {e}");
                            }
                        }
                    }
                    LcdThreadMsg::SwitchDesktop(reply) => {
                        let result = device.switch_to_desktop_mode();
                        let _ = reply.send(result);
                        break;
                    }
                    LcdThreadMsg::SetBrightness(val) => {
                        if let Err(e) = device.set_brightness_val(val) {
                            warn!("LCD[{index}] set_brightness error: {e}");
                        }
                    }
                    LcdThreadMsg::Shutdown(reply) => {
                        let result =
                            lianli_transport::usb::with_teardown_io(Duration::from_secs(3), || {
                                device.set_brightness_val(0)
                            });
                        device.transport_release();
                        let _ = reply.send(result);
                        return;
                    }
                    LcdThreadMsg::Stop => break,
                }
            }
            device.transport_release();
        });
        Self {
            transport,
            tx,
            stream_control,
            closing,
            thread: Some(thread),
        }
    }

    fn stream_h264(&self, path: PathBuf, looping: bool, fps: f32) -> anyhow::Result<()> {
        let stop = self.stream_control.next();
        self.tx
            .try_send(LcdThreadMsg::StreamH264 {
                path,
                looping,
                fps,
                stop,
            })
            .map_err(|e| anyhow::anyhow!("LCD file stream was not accepted: {e}"))?;
        Ok(())
    }

    fn stream_h264_reader(
        &self,
        stdout: std::process::ChildStdout,
        fps: f32,
    ) -> anyhow::Result<()> {
        let stop = self.stream_control.next();
        self.tx
            .try_send(LcdThreadMsg::StreamH264Reader(stdout, fps, stop))
            .map_err(|e| anyhow::anyhow!("LCD live stream was not accepted: {e}"))?;
        Ok(())
    }

    fn set_brightness(&self, brightness: u8) -> anyhow::Result<()> {
        match self.tx.try_send(LcdThreadMsg::SetBrightness(brightness)) {
            Ok(()) => Ok(()),
            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                anyhow::bail!("LCD sender busy; brightness was not accepted")
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                anyhow::bail!("LCD sender thread exited")
            }
        }
    }

    fn send_frame(&self, frame: &[u8]) -> anyhow::Result<()> {
        self.stream_control.cancel();
        match self.tx.try_send(LcdThreadMsg::Frame(frame.to_vec())) {
            Ok(()) => Ok(()),
            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                debug!("LCD sender busy, dropping frame");
                Ok(())
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                anyhow::bail!("LCD sender thread exited")
            }
        }
    }

    pub(super) fn switch_to_desktop_mode(&mut self) -> anyhow::Result<()> {
        self.stream_control.cancel();
        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
        self.tx
            .send(LcdThreadMsg::SwitchDesktop(reply_tx))
            .map_err(|_| anyhow::anyhow!("LCD sender thread exited"))?;
        let result = reply_rx
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| anyhow::anyhow!("LCD sender thread timeout"))?;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        result
    }

    fn send_frame_verified(&self, frame: &[u8]) -> anyhow::Result<()> {
        self.stream_control.cancel();
        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
        self.tx
            .send(LcdThreadMsg::FrameVerified(frame.to_vec(), reply_tx))
            .map_err(|_| anyhow::anyhow!("LCD sender thread exited"))?;
        reply_rx
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| anyhow::anyhow!("LCD sender thread timeout"))?
    }

    fn send_before(
        &self,
        mut message: LcdThreadMsg,
        deadline: std::time::Instant,
    ) -> anyhow::Result<()> {
        loop {
            match self.tx.try_send(message) {
                Ok(()) => return Ok(()),
                Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                    anyhow::bail!("LCD sender thread exited")
                }
                Err(std::sync::mpsc::TrySendError::Full(returned)) => {
                    if std::time::Instant::now() >= deadline {
                        anyhow::bail!("LCD sender queue timed out")
                    }
                    message = returned;
                    thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }

    fn join_before(&mut self, deadline: std::time::Instant) {
        if let Some(worker) = self.thread.take() {
            while !worker.is_finished() && std::time::Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            if worker.is_finished() {
                if worker.join().is_err() {
                    warn!("LCD sender panicked");
                }
            } else {
                // The worker retains its transport until the current I/O returns.
                warn!("LCD sender did not stop before deadline; detaching");
            }
        }
    }

    fn shutdown(&mut self) -> anyhow::Result<()> {
        self.closing.store(true, Ordering::Release);
        self.stream_control.cancel();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let result = self
            .send_before(LcdThreadMsg::Shutdown(tx), deadline)
            .and_then(|()| {
                rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                    .map_err(|_| anyhow::anyhow!("LCD shutdown acknowledgement timed out"))?
            });
        self.join_before(deadline);
        result
    }

    fn stop(&mut self) {
        if self.thread.is_none() {
            return;
        }
        self.closing.store(true, Ordering::Release);
        self.stream_control.cancel();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        if let Err(e) = self.send_before(LcdThreadMsg::Stop, deadline) {
            warn!("Failed to stop LCD sender: {e}");
        }
        self.join_before(deadline);
    }
}

impl Drop for ThreadedWinUsbSender {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(crate) struct ActiveTarget {
    pub(super) index: usize,
    pub(super) key: ConfigKey,
    pub(super) device_identity: String,
    // `media` must drop before `lcd`: tearing down a live h264 pipeline closes
    // the encoder's stdin, ffmpeg flushes its trailer to stdout, and the WinUsb
    // thread (owned by `lcd`) needs to still be alive to drain it.
    media: Box<dyn FrameSource>,
    media_pending: bool,
    media_tx: Option<Sender<DaemonEvent>>,
    pub(super) lcd: LcdBackend,
    pub(super) asset: Arc<MediaAsset>,
    pub(super) screen: ScreenInfo,
    pub(super) custom_h264: bool,
    // This variable contains the last seen frame version. Each renderer holds a frame version counter which gets increased each time it actually writes into the frame. The first time it writes into the frame sets the frame version to 1
    // By using this mechanism we are able to detect whether we actually need to send the frame via USB bus to the LCD, and thus we can save quite a lot of time by not sending frames which are already displayed.
    pub(super) frame_counter: u64,
    pub(super) consecutive_errors: u32,
    recovery_stop: Arc<AtomicBool>,
    recovery_thread: Option<JoinHandle<()>>,
    /// Set once the init worker reports LcdInitComplete for this device.
    /// Before it, a false answer from supports_c_command only means the
    /// firmware is not known yet and must be retried later.
    init_complete: bool,
    /// Set when the device definitively does not support recovery, so the
    /// periodic retry stops probing it.
    recovery_unsupported: bool,
    /// Brightness that could not be applied because the init worker held
    /// the LCD. Applied when init completes.
    pending_brightness: Option<u8>,
    brightness_retries: u8,
}

fn spawn_recovery_thread(
    lcd: SharedHidLcd,
    stop: Arc<AtomicBool>,
    index: usize,
    device_id: String,
    tx: Option<Sender<DaemonEvent>>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        use lianli_devices::traits::RecoveryAction;
        while !stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_secs(2));
            if stop.load(Ordering::Relaxed) {
                break;
            }
            // Hold the gate through the probe so a worker cannot start between
            // the idle check and LCD access. Active streams never take this path.
            let Some(_idle) = lcd.recovery_idle() else {
                continue;
            };
            let Some(mut guard) = lcd.try_lock_for(Duration::from_secs(2)) else {
                continue;
            };
            if stop.load(Ordering::Relaxed) {
                break;
            }
            match guard.check_and_recover_lcd(&stop) {
                Ok(RecoveryAction::Recovered) => {
                    if let Some(tx) = &tx {
                        if !stop.load(Ordering::Relaxed) {
                            tx.send(DaemonEvent::RecreateMedia {
                                target_index: index,
                                device_id: device_id.clone(),
                            })
                            .ok();
                        }
                    }
                }
                Ok(RecoveryAction::NoChange) => {}
                Err(e) => {
                    debug!("LCD[{index}] health check error: {e:#}");
                }
            }
        }
    })
}

impl ActiveTarget {
    pub(super) fn new(
        index: usize,
        key: ConfigKey,
        device_identity: String,
        lcd: LcdBackend,
        asset: Arc<MediaAsset>,
        screen: ScreenInfo,
        custom_h264: bool,
        tx: Option<Sender<DaemonEvent>>,
    ) -> Self {
        let media: Box<dyn FrameSource> = Box::new(NoopFrameSource);
        let recovery_stop = Arc::new(AtomicBool::new(false));
        let recovery_thread = match &lcd {
            LcdBackend::HidLcd(d) => {
                // bounded: the init worker holds this mutex across the 10s
                // settle on some paths
                // Not gated on recovery_idle, renderer targets already hold
                // a lease here. The thread idles itself while streams run
                let supports = d
                    .try_lock_for(Duration::from_millis(200))
                    .is_some_and(|guard| guard.supports_c_command());
                if supports {
                    Some(spawn_recovery_thread(
                        Arc::clone(d),
                        Arc::clone(&recovery_stop),
                        index,
                        device_identity.clone(),
                        tx.clone(),
                    ))
                } else {
                    None
                }
            }
            _ => None,
        };
        Self {
            index,
            key,
            device_identity,
            lcd,
            media,
            media_pending: true,
            media_tx: tx,
            asset,
            screen,
            custom_h264,
            frame_counter: 0,
            consecutive_errors: 0,
            recovery_stop,
            recovery_thread,
            init_complete: false,
            recovery_unsupported: false,
            pending_brightness: None,
            brightness_retries: 0,
        }
    }

    /// Start the recovery thread if it is missing and the device now
    /// reports c-command support. Called from new, where firmware may
    /// already be known, after LcdInitComplete, and from the periodic
    /// device poll with a zero wait so a busy LCD is retried later
    /// instead of disabling recovery for the whole session.
    pub(super) fn maybe_start_recovery(&mut self, tx: Option<Sender<DaemonEvent>>, wait: Duration) {
        if self.recovery_thread.is_some() || self.recovery_unsupported {
            return;
        }
        let LcdBackend::HidLcd(d) = &self.lcd else {
            return;
        };
        let Some(_idle) = d.recovery_idle() else {
            return;
        };
        let Some(guard) = d.try_lock_for(wait) else {
            if wait > Duration::ZERO {
                debug!(
                    "[devices] LCD[{}] busy, will retry starting recovery thread",
                    self.device_identity
                );
            }
            return;
        };
        let supports = guard.supports_c_command();
        // Only a device that actually answered its firmware query gives a
        // definitive no. When the read never succeeded the capability is
        // unknown, and a later successful read by the firmware tracker
        // must still be able to start recovery, so the retries continue.
        let firmware_known = guard.firmware_version_str().is_some();
        drop(guard);
        if !supports {
            // Before init completes this only means the firmware is not
            // known yet. After it, the answer is definitive.
            if self.init_complete && firmware_known {
                self.recovery_unsupported = true;
                debug!(
                    "[devices] LCD[{}] firmware does not support recovery, stopping retries",
                    self.device_identity
                );
            }
            return;
        }
        info!(
            "[devices] LCD[{}] starting recovery thread after init",
            self.device_identity
        );
        self.recovery_thread = Some(spawn_recovery_thread(
            Arc::clone(d),
            Arc::clone(&self.recovery_stop),
            self.index,
            self.device_identity.clone(),
            tx,
        ));
    }

    /// The init worker finished, so answers from the device are now
    /// definitive and deferred work can be applied.
    pub(super) fn mark_init_complete(&mut self) {
        self.init_complete = true;
    }

    pub(super) fn cleaner_payload_limit(&self) -> usize {
        match &self.lcd {
            LcdBackend::WinUsb(sender) if self.screen.h264 => sender
                .transport
                .as_ref()
                .map_or(self.screen.max_payload, |transport| {
                    transport.h264_chunk_size()
                }),
            _ => self.screen.max_payload,
        }
    }

    pub(super) fn apply_brightness(
        &mut self,
        wireless: Option<&WirelessController>,
        builder: &mut PacketBuilder,
        brightness: u8,
    ) {
        if !self.media_pending && self.media.is_autonomous() {
            if let LcdBackend::WinUsb(sender) = &self.lcd {
                sender.stream_control.cancel();
                self.media = Box::new(NoopFrameSource);
                self.media_pending = true;
            }
        }
        self.pending_brightness = Some(brightness);
        self.brightness_retries = 3;
        self.flush_pending_brightness(wireless, builder);
    }

    pub(super) fn flush_pending_brightness(
        &mut self,
        wireless: Option<&WirelessController>,
        builder: &mut PacketBuilder,
    ) {
        let Some(brightness) = self.pending_brightness else {
            return;
        };
        let result = if let LcdBackend::HidLcd(device) = &self.lcd {
            let Some(guard) = device.try_lock() else {
                return;
            };
            guard.set_brightness(brightness)
        } else {
            self.lcd.set_brightness(wireless, builder, brightness)
        };
        match result {
            Ok(()) => self.pending_brightness = None,
            Err(error) => {
                self.brightness_retries = self.brightness_retries.saturating_sub(1);
                if self.brightness_retries == 0 {
                    self.pending_brightness = None;
                    warn!(
                        "LCD[{}] brightness could not be applied after three attempts: {error:#}",
                        self.index
                    );
                }
            }
        }
    }

    pub(super) fn matches(&self, identity: &str, key: &ConfigKey) -> bool {
        self.device_identity == identity && key == &self.key
    }

    /// Replace the media asset without reopening the LCD transport.
    pub(super) fn swap_media(
        &mut self,
        asset: Arc<MediaAsset>,
        custom_h264: bool,
        tx: Option<Sender<DaemonEvent>>,
    ) {
        self.media = Box::new(NoopFrameSource);
        self.key = asset.config_key.clone();
        self.asset = Arc::clone(&asset);
        self.custom_h264 = custom_h264;
        if let LcdBackend::WinUsb(sender) = &self.lcd {
            sender.stream_control.cancel();
        }
        self.media_pending = true;
        self.media_tx = tx;
        self.frame_counter = 0;
        info!(
            "[devices] LCD[{}] media swapped (keeping transport)",
            self.index
        );
    }

    /// Apply a `custom_h264` toggle change without reloading media or
    /// reopening the transport. Rebuilds only the frame source so the live
    /// H.264 pipeline engages/disengages immediately on save.
    pub(super) fn update_custom_h264(
        &mut self,
        custom_h264: bool,
        tx: Option<Sender<DaemonEvent>>,
    ) {
        if self.custom_h264 == custom_h264 {
            return;
        }
        self.swap_media(Arc::clone(&self.asset), custom_h264, tx);
    }

    pub(super) fn send_frame(
        &mut self,
        wireless: Option<&WirelessController>,
        builder: &mut PacketBuilder,
    ) -> Result<bool, SendError> {
        if self.media_pending {
            if self.pending_brightness.is_some() {
                return Ok(false);
            }
            self.media = make_frame_source(
                Arc::clone(&self.asset),
                self.media_tx.clone(),
                &self.lcd,
                &self.screen,
                self.custom_h264,
            );
            self.media_pending = false;
        }
        // H.264 / autonomous sources: kick off streaming on the first call,
        // then short-circuit (their threads push frames directly to the LCD).
        if self.media.is_autonomous() {
            self.media.start(&self.lcd).map_err(SendError::Other)?;
            return Ok(true);
        }

        let is_static = self.media.is_static();
        let frame = match self.media.next_frame() {
            Some(bytes) => bytes,
            None => return Ok(false),
        };

        let result = if is_static {
            self.lcd.send_frame_verified(wireless, builder, frame)
        } else {
            self.lcd.send_frame(wireless, builder, frame)
        };
        match result {
            Ok(()) => {}
            // The init worker holds the LCD for its whole settle window.
            // Report nothing sent so the version check retries next tick
            // instead of counting an error toward target recreation.
            Err(e) if e.downcast_ref::<LcdBusy>().is_some() => {
                debug!(
                    "[devices] LCD[{}] initializing, deferring frame send",
                    self.index
                );
                return Ok(false);
            }
            Err(err) => {
                let send_err = match err.downcast::<lianli_transport::TransportError>() {
                    Ok(usb) => SendError::Usb(usb),
                    Err(other) => SendError::Other(other),
                };
                return Err(send_err);
            }
        }

        self.frame_counter += 1;
        self.media.mark_sent();
        Ok(true)
    }

    pub(super) fn stop(&mut self) {
        self.recovery_stop.store(true, Ordering::Relaxed);
        self.media_pending = false;
        // dropping media sets the renderer stop flags, which unblock stop()
        self.media = Box::new(NoopFrameSource);
        if let Some(t) = self.recovery_thread.take() {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !t.is_finished() && std::time::Instant::now() < deadline {
                thread::sleep(Duration::from_millis(50));
            }
            if !t.is_finished() {
                warn!(
                    "LCD[{}] recovery thread did not stop in 5s — detaching it",
                    self.index
                );
            }
        }
    }

    pub(super) fn shutdown(
        &mut self,
        wireless: Option<&WirelessController>,
        builder: &mut PacketBuilder,
        turn_off: bool,
    ) -> anyhow::Result<()> {
        self.stop();
        if !turn_off {
            if let LcdBackend::WinUsb(sender) = &mut self.lcd {
                sender.stop();
            }
            return Ok(());
        }
        match &mut self.lcd {
            LcdBackend::WinUsb(sender) => sender.shutdown(),
            LcdBackend::HidLcd(lcd) => {
                let guard = lcd
                    .try_lock_for(Duration::from_millis(500))
                    .ok_or_else(|| anyhow::anyhow!("LCD is busy during shutdown"))?;
                lianli_transport::usb::with_teardown_io(Duration::from_secs(3), || {
                    guard.set_brightness(0)
                })
            }
            LcdBackend::Slv3(lcd) => {
                lianli_transport::usb::with_teardown_io(Duration::from_secs(3), || {
                    if let Some(wireless) = wireless {
                        wireless.ensure_video_mode()?;
                    }
                    lcd.set_brightness(builder, 0)
                })
            }
        }
    }
}

impl Drop for ActiveTarget {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A source of JPEG frames to push to an LCD, or an autonomous H.264
/// pipeline that streams directly to the device.
///
/// Every `MediaRuntime` variant is now one of these. The trait lets
/// `ActiveTarget::send_frame` dispatch without a 7-arm match.
trait FrameSource: Send {
    /// Called on the first `send_frame` after the source is attached. For
    /// H.264 file streaming, this kicks off the streaming thread.
    fn start(&mut self, _lcd: &LcdBackend) -> anyhow::Result<()> {
        Ok(())
    }

    /// Poll for the next JPEG frame. Returns `None` when no new frame has
    /// been rendered since the last call, or when the source is autonomous.
    fn next_frame(&mut self) -> Option<&[u8]> {
        None
    }

    /// Mark the current frame as successfully sent over USB.
    fn mark_sent(&mut self) {}

    /// `true` if the source produces a single unchanging frame (uses the
    /// verified-send path that tolerates a dropped USB write).
    fn is_static(&self) -> bool {
        false
    }

    /// `true` if the source pushes frames on its own thread and `send_frame`
    /// should skip the JPEG path entirely.
    fn is_autonomous(&self) -> bool {
        false
    }
}

struct NoopFrameSource;
impl FrameSource for NoopFrameSource {}

// ─── JPEG sources ──────────────────────────────────────────────────────

struct StaticSource {
    frame: Arc<Vec<u8>>,
    sent: bool,
}
impl FrameSource for StaticSource {
    fn next_frame(&mut self) -> Option<&[u8]> {
        if self.sent {
            return None;
        }
        Some(self.frame.as_slice())
    }
    fn mark_sent(&mut self) {
        self.sent = true;
    }
    fn is_static(&self) -> bool {
        true
    }
}

struct VideoSource {
    player: Arc<AsyncVideoPlayer>,
    frames: Arc<Vec<Vec<u8>>>,
    sent_index: usize,
}
impl FrameSource for VideoSource {
    fn next_frame(&mut self) -> Option<&[u8]> {
        let idx = self.player.get_frame_index();
        if idx <= self.sent_index || self.frames.is_empty() {
            return None;
        }
        let ret = Some(self.frames[idx % self.frames.len()].as_slice());
        self.sent_index = idx;
        ret
    }
}

struct SensorSource {
    renderer: Arc<AsyncSensorRenderer>,
    cached: Vec<u8>,
    sent_index: usize,
}
impl FrameSource for SensorSource {
    fn next_frame(&mut self) -> Option<&[u8]> {
        let idx = self.renderer.get_frame_index();
        if idx <= self.sent_index {
            return None;
        }
        self.cached = self.renderer.get_current_frame();
        self.sent_index = idx;
        Some(self.cached.as_slice())
    }
}

struct CustomSource {
    renderer: Arc<AsyncCustomRenderer>,
    cached: Vec<u8>,
    sent_index: usize,
}
impl FrameSource for CustomSource {
    fn next_frame(&mut self) -> Option<&[u8]> {
        let idx = self.renderer.get_frame_index();
        if idx <= self.sent_index {
            return None;
        }
        self.cached = self.renderer.get_current_frame();
        self.sent_index = idx;
        Some(self.cached.as_slice())
    }
}

// ─── H.264 autonomous sources ──────────────────────────────────────────

struct H264FileSource {
    path: PathBuf,
    looping: bool,
    fps: f32,
    started: bool,
    hid_thread: Option<JoinHandle<()>>,
    hid_stop: Arc<AtomicBool>,
    /// Set by the worker when it ended without a stop request
    hid_completed: Option<Arc<AtomicBool>>,
    /// start() runs every streaming tick; back off after an open failure
    /// so a missing file retries periodically instead of once or per-tick.
    retry_after: Option<std::time::Instant>,
}

const FILE_OPEN_RETRY: Duration = Duration::from_secs(5);

impl H264FileSource {
    fn new(path: PathBuf, looping: bool, fps: f32) -> Self {
        Self {
            path,
            looping,
            fps,
            started: false,
            hid_thread: None,
            hid_stop: Arc::new(AtomicBool::new(false)),
            hid_completed: None,
            retry_after: None,
        }
    }
}

impl FrameSource for H264FileSource {
    fn start(&mut self, lcd: &LcdBackend) -> anyhow::Result<()> {
        if self.started {
            if let Some(ref t) = self.hid_thread {
                if t.is_finished() {
                    let completed = self
                        .hid_completed
                        .as_ref()
                        .is_some_and(|c| c.load(Ordering::Acquire));
                    if let Some(t) = self.hid_thread.take() {
                        let _ = t.join();
                    }
                    if completed {
                        return Ok(());
                    }
                    warn!("HID h264 stream thread ended; resetting for restart");
                    self.hid_stop = Arc::new(AtomicBool::new(false));
                    self.started = false;
                    // Back off so a wedged device cannot spin the restart loop
                    self.retry_after = Some(std::time::Instant::now() + FILE_OPEN_RETRY);
                } else {
                    return Ok(());
                }
            } else {
                return Ok(());
            }
        }
        if let Some(t) = self.retry_after {
            if std::time::Instant::now() < t {
                return Ok(());
            }
        }
        match lcd {
            LcdBackend::WinUsb(sender) => {
                sender.stream_h264(self.path.clone(), self.looping, self.fps)?;
            }
            LcdBackend::HidLcd(hid) => {
                // open before marking started so a missing file can retry
                let file = match std::fs::File::open(&self.path) {
                    Ok(f) => f,
                    Err(e) => {
                        warn!("HID h264 file open failed for {:?}: {e:#}", self.path);
                        self.retry_after = Some(std::time::Instant::now() + FILE_OPEN_RETRY);
                        return Ok(());
                    }
                };
                self.retry_after = None;
                let lcd = Arc::clone(hid);
                let (looping, fps) = (self.looping, self.fps);
                let stop = Arc::clone(&self.hid_stop);
                let completed = Arc::new(AtomicBool::new(false));
                let done = Arc::clone(&completed);
                self.hid_completed = Some(completed);
                let Some(lease) = lcd.begin_stream() else {
                    return Ok(());
                };
                self.hid_thread = Some(thread::spawn(move || {
                    let _lease = lease;
                    if stream_h264_file_to_hid(lcd, file, looping, fps, stop) {
                        done.store(true, Ordering::Release);
                    }
                }));
            }
            _ => {}
        }
        self.started = true;
        Ok(())
    }
    fn is_autonomous(&self) -> bool {
        true
    }
}

impl Drop for H264FileSource {
    fn drop(&mut self) {
        self.hid_stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.hid_thread.take() {
            let deadline = std::time::Instant::now() + Duration::from_millis(100);
            while !worker.is_finished() && std::time::Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            if worker.is_finished() {
                let _ = worker.join();
            } else {
                // The worker retains its stream lease and checks stop after locking the device.
                warn!("HID H.264 file worker is stopping; detaching until its transfer returns");
            }
        }
    }
}

struct CustomH264Source {
    #[allow(dead_code)]
    renderer: Arc<AsyncCustomH264Renderer>,
}
impl FrameSource for CustomH264Source {
    fn is_autonomous(&self) -> bool {
        true
    }
}

struct SensorH264Source {
    #[allow(dead_code)]
    renderer: Arc<AsyncSensorH264Renderer>,
}
impl FrameSource for SensorH264Source {
    fn is_autonomous(&self) -> bool {
        true
    }
}

/// Construct the appropriate `FrameSource` for a given media asset + LCD combo.
fn make_frame_source(
    asset: Arc<MediaAsset>,
    tx: Option<Sender<DaemonEvent>>,
    lcd: &LcdBackend,
    screen: &ScreenInfo,
    custom_h264: bool,
) -> Box<dyn FrameSource> {
    match &asset.kind {
        MediaAssetKind::Static { frame } => Box::new(StaticSource {
            frame: Arc::clone(frame),
            sent: false,
        }),
        MediaAssetKind::Video { frames, .. } => {
            let player = Arc::new(AsyncVideoPlayer::new(tx, Arc::clone(&asset)));
            Box::new(VideoSource {
                player,
                frames: Arc::clone(frames),
                sent_index: 0,
            })
        }
        MediaAssetKind::Sensor {
            asset: sensor_asset,
        } => {
            if screen.h264 {
                match AsyncSensorH264Renderer::new(
                    Arc::clone(sensor_asset),
                    lcd,
                    screen,
                    asset.stream_fps,
                ) {
                    Ok(renderer) => {
                        info!("Sensor mode using live h264 pipeline");
                        return Box::new(SensorH264Source {
                            renderer: Arc::new(renderer),
                        });
                    }
                    Err(e) => {
                        warn!("Sensor h264 pipeline unavailable, falling back to JPEG: {e}");
                    }
                }
            }
            let renderer = Arc::new(AsyncSensorRenderer::new(
                tx,
                Arc::clone(sensor_asset),
                Arc::clone(&asset),
                screen.needs_keepalive,
            ));
            let cached = renderer.get_current_frame();
            Box::new(SensorSource {
                renderer,
                cached,
                sent_index: 0,
            })
        }
        MediaAssetKind::H264Stream {
            path, looping, fps, ..
        } => Box::new(H264FileSource::new(path.clone(), *looping, *fps)),
        MediaAssetKind::Custom {
            asset: custom_asset,
        } => {
            if custom_h264 && screen.h264 {
                match AsyncCustomH264Renderer::new(
                    Arc::clone(custom_asset),
                    lcd,
                    screen,
                    custom_asset.canvas_width(),
                    custom_asset.canvas_height(),
                    custom_asset.total_rotation_deg(),
                    asset.stream_fps,
                ) {
                    Ok(renderer) => {
                        info!("Custom mode using live h264 pipeline");
                        return Box::new(CustomH264Source {
                            renderer: Arc::new(renderer),
                        });
                    }
                    Err(e) => {
                        warn!("Custom h264 pipeline unavailable, falling back to JPEG: {e}");
                    }
                }
            }
            let renderer = Arc::new(AsyncCustomRenderer::new(
                tx,
                Arc::clone(custom_asset),
                Arc::clone(&asset),
                screen.needs_keepalive,
            ));
            let cached = renderer.get_current_frame();
            Box::new(CustomSource {
                renderer,
                cached,
                sent_index: 0,
            })
        }
    }
}

/// True when the file finished on its own and the worker must not restart
fn stream_h264_file_to_hid(
    lcd: SharedHidLcd,
    mut file: std::fs::File,
    looping: bool,
    fps: f32,
    stop: Arc<AtomicBool>,
) -> bool {
    use lianli_devices::hydroshift_lcd::{find_au_split, pace_frame};
    use std::io::{Read, Seek, SeekFrom};
    use std::time::Instant;

    let frame_interval = {
        let Some(mut guard) = lcd.try_lock_for(Duration::from_millis(100)) else {
            return false;
        };
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        Duration::from_secs_f32(1.0 / guard.set_stream_fps(fps))
    };
    let mut read_buf = vec![0u8; 64 * 1024];
    let mut next_deadline = Instant::now() + frame_interval;
    let stopped = || stop.load(Ordering::Relaxed);
    // a single-AU file never yields a split boundary, its only frame rides
    // the EOF flush, allow it exactly one looping pass
    let mut first_pass = true;
    let mut saw_boundary = false;
    loop {
        if stopped() {
            return false;
        }
        let mut accum: Vec<u8> = Vec::with_capacity(256 * 1024);
        let mut sent_any = false;
        loop {
            if stopped() {
                return false;
            }
            let n = match file.read(&mut read_buf) {
                Ok(n) => n,
                Err(e) => {
                    warn!("HID h264 file read error: {e:#}");
                    return false;
                }
            };
            if n == 0 {
                break;
            }
            accum.extend_from_slice(&read_buf[..n]);
            if accum.len() > MAX_AU_BYTES {
                warn!(
                    "HID h264 file: no AU boundary within {} bytes, aborting",
                    MAX_AU_BYTES
                );
                return false;
            }
            while let Some(split) = find_au_split(&accum) {
                if stopped() {
                    return false;
                }
                let au: Vec<u8> = accum.drain(..split).collect();
                if au.is_empty() {
                    continue;
                }
                if !send_h264_au_with_retry(&lcd, &au, &stopped) {
                    return false;
                }
                saw_boundary = true;
                sent_any = true;
                pace_frame(&mut next_deadline, frame_interval);
            }
        }
        // reached only via the EOF break (every other exit is a return),
        // residual flush is paced like a regular AU
        if !stopped() && !accum.is_empty() {
            pace_frame(&mut next_deadline, frame_interval);
            if !send_h264_au_with_retry(&lcd, &accum, &stopped) {
                return false;
            }
            sent_any = true;
        }
        if stopped() {
            return false;
        }
        if !looping {
            return true;
        }
        // only loop when real AU boundaries were found: a boundary-less
        // file's flush would otherwise re-send identical data forever
        if !sent_any || (first_pass && !saw_boundary) {
            warn!("HID h264 file produced no complete access units, stopping");
            return true;
        }
        first_pass = false;
        if let Err(e) = file.seek(SeekFrom::Start(0)) {
            warn!("HID h264 file seek failed: {e:#}");
            return false;
        }
    }
}

pub(super) enum SendError {
    Usb(lianli_transport::TransportError),
    Other(anyhow::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn replacing_queued_stream_keeps_old_cancellation_and_brightness_order() {
        let (tx, rx) = std::sync::mpsc::sync_channel(3);
        let sender = ThreadedWinUsbSender {
            transport: None,
            tx,
            stream_control: Arc::new(StreamControl::default()),
            closing: Arc::new(AtomicBool::new(false)),
            thread: None,
        };
        sender.stream_h264("old".into(), true, 20.0).unwrap();
        sender.stream_control.cancel();
        sender.set_brightness(37).unwrap();
        sender.stream_h264("new".into(), true, 20.0).unwrap();
        let LcdThreadMsg::StreamH264 {
            path, stop: old, ..
        } = rx.try_recv().unwrap()
        else {
            panic!("expected old stream")
        };
        assert_eq!(path, PathBuf::from("old"));
        assert!(old.load(Ordering::Acquire));
        assert!(matches!(
            rx.try_recv().unwrap(),
            LcdThreadMsg::SetBrightness(37)
        ));
        let LcdThreadMsg::StreamH264 {
            path, stop: new, ..
        } = rx.try_recv().unwrap()
        else {
            panic!("expected replacement stream")
        };
        assert_eq!(path, PathBuf::from("new"));
        assert!(!new.load(Ordering::Acquire));
        assert!(sender.stream_control.restart(&old).is_none());
        assert!(!new.load(Ordering::Acquire));
        sender.stream_control.cancel();
        assert!(new.load(Ordering::Acquire));
        assert!(sender.stream_control.restart(&new).is_none());
    }

    #[test]
    fn live_sources_do_not_queue_streaming_before_brightness() {
        let (tx, rx) = std::sync::mpsc::sync_channel(2);
        let sender = ThreadedWinUsbSender {
            transport: None,
            tx,
            stream_control: Arc::new(StreamControl::default()),
            closing: Arc::new(AtomicBool::new(false)),
            thread: None,
        };
        let screen = ScreenInfo {
            h264: true,
            ..ScreenInfo::TLLCD
        };
        let descriptor = serde_json::from_value(serde_json::json!({
            "label": "Test", "unit": "%", "source": { "type": "constant", "value": 25 }
        }))
        .unwrap();
        let sensor =
            lianli_media::SensorAsset::new(&descriptor, 0.0, &screen, &[], None, 1000).unwrap();
        let asset = Arc::new(MediaAsset {
            kind: MediaAssetKind::Sensor { asset: sensor },
            config_key: "sensor-test".into(),
            stream_fps: 20.0,
        });
        let mut target = ActiveTarget::new(
            0,
            asset.config_key.clone(),
            "test".into(),
            LcdBackend::WinUsb(sender),
            asset.clone(),
            screen,
            true,
            None,
        );
        assert!(rx.try_recv().is_err());
        target.apply_brightness(None, &mut PacketBuilder::new(), 100);
        assert!(matches!(
            rx.try_recv().unwrap(),
            LcdThreadMsg::SetBrightness(100)
        ));
        target.swap_media(asset, true, None);
        assert!(rx.try_recv().is_err());
        target.apply_brightness(None, &mut PacketBuilder::new(), 37);
        assert!(matches!(
            rx.try_recv().unwrap(),
            LcdThreadMsg::SetBrightness(37)
        ));
    }

    #[test]
    fn stopping_file_source_does_not_wait_for_a_busy_worker() {
        let (release, blocked) = std::sync::mpsc::channel();
        let (finished, done) = std::sync::mpsc::channel();
        let mut source = H264FileSource::new("unused".into(), true, 20.0);
        let stop = source.hid_stop.clone();
        source.hid_thread = Some(thread::spawn(move || {
            blocked.recv().unwrap();
            assert!(stop.load(Ordering::Relaxed));
            finished.send(()).unwrap();
        }));
        let started = std::time::Instant::now();
        drop(source);
        assert!(started.elapsed() < Duration::from_secs(1));
        release.send(()).unwrap();
        done.recv_timeout(Duration::from_secs(1)).unwrap();
    }

    #[test]
    fn cancelled_access_unit_never_writes_after_waiting_for_the_device() {
        let (lcd, sends) = lcd(0);
        let guard = lcd.lock();
        let worker_lcd = lcd.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let (entered, waiting) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            let first = AtomicBool::new(true);
            send_h264_au_with_retry(&worker_lcd, &[1, 2, 3], &|| {
                let cancelled = worker_stop.load(Ordering::Relaxed);
                if first.swap(false, Ordering::Relaxed) {
                    entered.send(()).unwrap();
                }
                cancelled
            })
        });
        waiting.recv_timeout(Duration::from_secs(1)).unwrap();
        stop.store(true, Ordering::Relaxed);
        drop(guard);
        assert!(!worker.join().unwrap());
        assert_eq!(sends.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn shutdown_reports_the_sender_result_after_stopping_production() {
        let (tx, rx) = std::sync::mpsc::sync_channel(2);
        let stop = Arc::new(AtomicBool::new(false));
        let closing = Arc::new(AtomicBool::new(false));
        let stream_control = Arc::new(StreamControl {
            current: Mutex::new(stop.clone()),
        });
        let worker_stop = stop.clone();
        let worker_closing = closing.clone();
        let worker = thread::spawn(move || {
            let LcdThreadMsg::Shutdown(reply) = rx.recv().unwrap() else {
                panic!("expected shutdown")
            };
            assert!(worker_stop.load(Ordering::Relaxed));
            assert!(worker_closing.load(Ordering::Acquire));
            reply
                .send(Err(anyhow::anyhow!("brightness transfer failed")))
                .unwrap();
        });
        let mut sender = ThreadedWinUsbSender {
            transport: None,
            tx,
            stream_control,
            closing,
            thread: Some(worker),
        };
        assert!(sender
            .shutdown()
            .unwrap_err()
            .to_string()
            .contains("brightness transfer failed"));
        assert!(sender.thread.is_none());
    }

    struct TestLcd {
        brightness: Arc<AtomicUsize>,
        sends: Arc<AtomicUsize>,
        fail_on: usize,
        fail_count: usize,
    }

    impl LcdDevice for TestLcd {
        fn screen_info(&self) -> &ScreenInfo {
            &ScreenInfo::AIO_LCD_480
        }
        fn send_jpeg_frame(&mut self, _: &[u8]) -> anyhow::Result<()> {
            Ok(())
        }
        fn set_brightness(&self, value: u8) -> anyhow::Result<()> {
            self.brightness.store(usize::from(value), Ordering::Relaxed);
            Ok(())
        }
        fn set_rotation(&self, _: u16) -> anyhow::Result<()> {
            Ok(())
        }
        fn initialize(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        fn send_h264_frame(&mut self, _: &[u8]) -> anyhow::Result<()> {
            let n = self.sends.fetch_add(1, Ordering::Relaxed) + 1;
            anyhow::ensure!(
                !(self.fail_on..self.fail_on + self.fail_count).contains(&n),
                "injected send failure"
            );
            Ok(())
        }
    }

    fn lcd(fail_on: usize) -> (SharedHidLcd, Arc<AtomicUsize>) {
        lcd_with_failures(fail_on, 1)
    }

    fn lcd_with_failures(fail_on: usize, fail_count: usize) -> (SharedHidLcd, Arc<AtomicUsize>) {
        let sends = Arc::new(AtomicUsize::new(0));
        (
            Arc::new(HidLcd::new(Box::new(TestLcd {
                brightness: Arc::new(AtomicUsize::new(100)),
                sends: Arc::clone(&sends),
                fail_on,
                fail_count,
            }))),
            sends,
        )
    }

    #[test]
    fn shutdown_setting_controls_brightness_but_always_stops_media() {
        for turn_off in [false, true] {
            let brightness = Arc::new(AtomicUsize::new(75));
            let device = Arc::new(HidLcd::new(Box::new(TestLcd {
                brightness: brightness.clone(),
                sends: Arc::new(AtomicUsize::new(0)),
                fail_on: 0,
                fail_count: 0,
            })));
            let asset = Arc::new(MediaAsset {
                kind: MediaAssetKind::Static {
                    frame: Arc::new(vec![1]),
                },
                config_key: "shutdown-test".into(),
                stream_fps: 20.0,
            });
            let mut target = ActiveTarget::new(
                0,
                asset.config_key.clone(),
                "test".into(),
                LcdBackend::HidLcd(device),
                asset,
                ScreenInfo::AIO_LCD_480,
                false,
                None,
            );
            target
                .shutdown(None, &mut PacketBuilder::new(), turn_off)
                .unwrap();
            assert_eq!(
                brightness.load(Ordering::Relaxed),
                if turn_off { 0 } else { 75 }
            );
            assert!(!target.media_pending);
            assert!(target.recovery_stop.load(Ordering::Relaxed));
        }
    }

    #[test]
    fn lock_contention_preserves_all_three_h264_send_attempts() {
        let (lcd, sends) = lcd_with_failures(1, 2);
        let guard = lcd.lock();
        let worker_lcd = Arc::clone(&lcd);
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            tx.send(()).unwrap();
            send_h264_au_with_retry(&worker_lcd, &[1, 2, 3], &|| false)
        });
        rx.recv().unwrap();
        thread::sleep(Duration::from_millis(250));
        drop(guard);
        assert!(worker.join().unwrap());
        assert_eq!(sends.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn prolonged_h264_lock_contention_exits_without_sending() {
        let (lcd, sends) = lcd(0);
        let _guard = lcd.lock();
        let started = std::time::Instant::now();
        assert!(!send_h264_au_with_retry(&lcd, &[1, 2, 3], &|| false));
        assert!(started.elapsed() >= Duration::from_secs(3));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(sends.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn overlapping_workers_exclude_recovery_without_lcd_access() {
        let (lcd, _) = lcd(0);
        let old = lcd.begin_stream().unwrap();
        let new = lcd.begin_stream().unwrap();
        let _device = lcd.lock();
        assert!(lcd.recovery_idle().is_none());
        drop(old);
        assert!(lcd.recovery_idle().is_none());
        drop(new);
        assert!(lcd.recovery_idle().is_some());
    }

    #[test]
    fn recovery_gate_defers_stream_start_without_blocking() {
        let (lcd, _) = lcd(0);
        let idle = lcd.recovery_idle().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker_lcd = Arc::clone(&lcd);
        let caller = thread::spawn(move || {
            let worker = HidStreamWorker::new(
                worker_lcd,
                Box::new(std::io::empty()),
                Arc::new(AtomicBool::new(false)),
                30.0,
            );
            tx.send(worker).unwrap();
        });
        let result = rx.recv_timeout(Duration::from_secs(1));
        drop(idle); // Release even on failure so the test cannot strand the caller.
        caller.join().unwrap();
        let worker = result.expect("stream start blocked on recovery");
        assert!(worker.handle.is_none());
        let restarter = StreamRestarter::HidLcd(Arc::clone(&lcd), Mutex::new(Some(worker)));
        let idle = lcd.recovery_idle().unwrap();
        assert!(!restarter.try_start_pending());
        drop(idle);
        let device = lcd.lock();
        assert!(restarter.try_start_pending());
        assert!(lcd.recovery_idle().is_none());
        drop(device);
        let StreamRestarter::HidLcd(_, current) = restarter else {
            unreachable!()
        };
        current
            .into_inner()
            .unwrap()
            .handle
            .unwrap()
            .join()
            .unwrap();
        assert!(lcd.recovery_idle().is_some());
    }

    #[test]
    fn live_worker_releases_recovery_on_eof_error_stop_and_panic() {
        struct PanicReader;
        impl std::io::Read for PanicReader {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                panic!("injected reader panic")
            }
        }
        struct ErrorReader;
        impl std::io::Read for ErrorReader {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("injected read error"))
            }
        }
        // Two slice NALs: one normal send and one EOF flush.
        let data = vec![0, 0, 0, 1, 5, 128, 0, 0, 0, 1, 1, 128];
        for (fail_on, stopped, reader, panics) in [
            (
                0,
                false,
                Box::new(std::io::Cursor::new(data.clone())) as Box<dyn std::io::Read + Send>,
                false,
            ),
            (
                1,
                false,
                Box::new(std::io::Cursor::new(data.clone())),
                false,
            ),
            (
                2,
                false,
                Box::new(std::io::Cursor::new(data.clone())),
                false,
            ),
            (0, true, Box::new(std::io::Cursor::new(data)), false),
            (0, false, Box::new(ErrorReader), false),
            (0, false, Box::new(PanicReader), true),
        ] {
            let (lcd, sends) = lcd(fail_on);
            let (worker, _) = spawn_hid_h264_stream(
                Arc::clone(&lcd),
                reader,
                Arc::new(AtomicBool::new(stopped)),
                30.0,
                lcd.begin_stream().unwrap(),
            );
            assert_eq!(worker.join().is_err(), panics);
            assert!(lcd.recovery_idle().is_some());
            if fail_on > 0 {
                assert_eq!(sends.load(Ordering::Relaxed), 3);
            }
            if stopped {
                assert_eq!(sends.load(Ordering::Relaxed), 0);
            }
        }
    }

    fn wait_for_file_worker(source: &H264FileSource) {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while !source.hid_thread.as_ref().unwrap().is_finished() {
            assert!(
                std::time::Instant::now() < deadline,
                "file worker did not exit"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn file_flush_retries_release_gate_and_restart_with_a_new_lease() {
        let path =
            std::env::temp_dir().join(format!("lianli-recovery-test-{}.h264", std::process::id()));
        std::fs::write(&path, [0, 0, 0, 1, 5, 128, 0, 0, 0, 1, 1, 128]).unwrap();
        for looping in [false, true] {
            // First AU succeeds; all three attempts at the EOF flush fail.
            let (lcd, sends) = lcd_with_failures(2, 3);
            let backend = LcdBackend::HidLcd(Arc::clone(&lcd));
            let mut source = H264FileSource::new(path.clone(), looping, 30.0);
            let idle = lcd.recovery_idle().unwrap();
            source.start(&backend).unwrap();
            assert!(!source.started);
            assert!(source.hid_thread.is_none());
            drop(idle);
            source.start(&backend).unwrap();
            assert!(lcd.recovery_idle().is_none());
            wait_for_file_worker(&source);
            assert_eq!(sends.load(Ordering::Relaxed), 4);
            assert!(!source
                .hid_completed
                .as_ref()
                .unwrap()
                .load(Ordering::Acquire));
            assert!(lcd.recovery_idle().is_some());

            source.looping = false;
            let idle = lcd.recovery_idle().unwrap();
            source.start(&backend).unwrap();
            assert!(!source.started);
            assert!(source.hid_thread.is_none());
            drop(idle);
            // Clear the restart backoff the dead worker armed
            source.retry_after = None;
            let device = lcd.lock();
            source.start(&backend).unwrap();
            assert!(lcd.recovery_idle().is_none());
            drop(device);
            wait_for_file_worker(&source);
            assert_eq!(sends.load(Ordering::Relaxed), 6);
            assert!(source
                .hid_completed
                .as_ref()
                .unwrap()
                .load(Ordering::Acquire));
            assert!(lcd.recovery_idle().is_some());
            source.start(&backend).unwrap();
            assert!(
                source.hid_thread.is_none(),
                "normal completion must not restart"
            );
            assert_eq!(sends.load(Ordering::Relaxed), 6);
        }
        // A single transient EOF-flush failure succeeds on retry and counts
        // as normal completion, without restarting the worker.
        let (lcd, sends) = lcd(2);
        let backend = LcdBackend::HidLcd(Arc::clone(&lcd));
        let mut source = H264FileSource::new(path.clone(), false, 30.0);
        source.start(&backend).unwrap();
        wait_for_file_worker(&source);
        assert_eq!(sends.load(Ordering::Relaxed), 3);
        assert!(source
            .hid_completed
            .as_ref()
            .unwrap()
            .load(Ordering::Acquire));
        assert!(lcd.recovery_idle().is_some());
        source.start(&backend).unwrap();
        assert!(source.hid_thread.is_none());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn stream_lease_covers_retry_sleeps_and_exhausted_failures() {
        let (lcd, sends) = lcd_with_failures(1, 3);
        let (worker, _) = spawn_hid_h264_stream(
            Arc::clone(&lcd),
            Box::new(std::io::Cursor::new(vec![0, 0, 0, 1, 5, 128])),
            Arc::new(AtomicBool::new(false)),
            30.0,
            lcd.begin_stream().unwrap(),
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while sends.load(Ordering::Relaxed) == 0 {
            assert!(std::time::Instant::now() < deadline);
            thread::yield_now();
        }
        // The failed send releases the LCD mutex during upstream's retry sleep,
        // but recovery must remain excluded by the worker's lease.
        let device = lcd.lock();
        assert!(lcd.recovery_idle().is_none());
        drop(device);
        worker.join().unwrap();
        assert_eq!(sends.load(Ordering::Relaxed), 3);
        assert!(lcd.recovery_idle().is_some());
    }

    #[test]
    fn stop_releases_the_lease_while_the_worker_stays_parked() {
        // A reader that blocks until released, like an encoder stdout whose
        // child has not exited yet
        struct ParkedReader {
            go: Arc<AtomicBool>,
            in_read: Arc<AtomicBool>,
        }
        impl std::io::Read for ParkedReader {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                self.in_read.store(true, Ordering::Release);
                while !self.go.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(5));
                }
                Ok(0)
            }
        }

        let (lcd, _) = lcd(0);
        let go = Arc::new(AtomicBool::new(false));
        let in_read = Arc::new(AtomicBool::new(false));
        let mut worker = HidStreamWorker::new(
            Arc::clone(&lcd),
            Box::new(ParkedReader {
                go: Arc::clone(&go),
                in_read: Arc::clone(&in_read),
            }),
            Arc::new(AtomicBool::new(false)),
            30.0,
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while !in_read.load(Ordering::Acquire) {
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        assert!(lcd.recovery_idle().is_none());

        worker.stop(Duration::from_millis(50));
        // The thread stays parked inside its read, yet the gate is free
        assert!(!worker.handle.as_ref().unwrap().is_finished());
        assert!(lcd.recovery_idle().is_some());

        // Let the thread exit, its own lease drop must not double decrement
        go.store(true, Ordering::Release);
        worker.handle.take().unwrap().join().unwrap();
        assert_eq!(*lcd.streams.lock(), 0);
        assert!(lcd.recovery_idle().is_some());
    }

    #[test]
    fn static_source_only_yields_frame_until_marked_sent() {
        let frame = Arc::new(vec![0xDE, 0xAD, 0xBE, 0xEF]);
        let mut source = StaticSource {
            frame: Arc::clone(&frame),
            sent: false,
        };
        assert!(source.is_static());
        assert!(!source.is_autonomous());

        // Yields frame before being marked sent
        assert_eq!(source.next_frame(), Some(frame.as_slice()));
        assert_eq!(source.next_frame(), Some(frame.as_slice()));

        // Once marked sent, yields None
        source.mark_sent();
        assert_eq!(source.next_frame(), None);
        assert_eq!(source.next_frame(), None);
    }
}
