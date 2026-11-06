use super::renderers::{
    AsyncCustomH264Renderer, AsyncCustomRenderer, AsyncSensorH264Renderer, AsyncSensorRenderer,
    AsyncVideoPlayer,
};
use super::source_preparation::{PreparedSource, SourceRequest, SourceResult};
use super::DaemonEvent;
use lianli_devices::crypto::PacketBuilder;
use lianli_devices::slv3_lcd::Slv3LcdDevice;
use lianli_devices::traits::LcdDevice;
use lianli_devices::winusb::lcd::WinUsbLcdDevice;
use lianli_devices::wireless::WirelessController;
use lianli_media::{MediaAsset, MediaAssetKind};
use lianli_shared::config::ConfigKey;
use lianli_shared::screen::ScreenInfo;
use lianli_shared::ui_message::{UiMessage, UiText};
use parking_lot::Mutex;
use std::path::PathBuf;
use std::process::ChildStdout;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

pub(super) type SharedHidLcd = Arc<HidLcd>;

pub(super) struct HidLcd {
    attachment: Arc<()>,
    device: Mutex<Box<dyn LcdDevice>>,
    // Recovery and another H.264 producer must wait until the current stream releases its lease.
    streams: Mutex<usize>,
}

impl HidLcd {
    pub(super) fn new(device: Box<dyn LcdDevice>) -> Self {
        Self {
            attachment: Arc::new(()),
            device: Mutex::new(device),
            streams: Mutex::new(0),
        }
    }

    pub(super) fn attachment(&self) -> std::sync::Weak<()> {
        Arc::downgrade(&self.attachment)
    }

    pub(super) fn matches_attachment(&self, attachment: &std::sync::Weak<()>) -> bool {
        std::sync::Weak::ptr_eq(&self.attachment(), attachment)
    }

    fn begin_stream(self: &Arc<Self>) -> Option<HidStreamLease> {
        let mut streams = self.streams.try_lock()?;
        if *streams != 0 {
            return None;
        }
        *streams = 1;
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

fn hid_stream_frame_interval(
    lcd: &SharedHidLcd,
    fps: f32,
    stopped: &dyn Fn() -> bool,
    timeout: Duration,
) -> Option<Duration> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if stopped() {
            return None;
        }
        if let Some(mut guard) = lcd.try_lock_for(Duration::from_millis(50)) {
            if stopped() {
                return None;
            }
            return Some(Duration::from_secs_f32(1.0 / guard.set_stream_fps(fps)));
        }
        if std::time::Instant::now() >= deadline {
            warn!("HID h264 stream not started: LCD remained busy for {timeout:?}");
            return None;
        }
    }
}

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
    transferred: Arc<AtomicBool>,
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
        let Some(frame_interval) =
            hid_stream_frame_interval(&lcd, fps, &halted, Duration::from_secs(3))
        else {
            return;
        };
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
                transferred.store(true, Ordering::Release);
                pace_frame(&mut next_deadline, frame_interval);
                if halted() {
                    return;
                }
            }
        }
        if clean_eof && !halted() && !accum.is_empty() {
            pace_frame(&mut next_deadline, frame_interval);
            if send_h264_au_with_retry(&lcd, &accum, &halted) {
                transferred.store(true, Ordering::Release);
            }
        }
    });
    (handle, halt)
}

pub(super) enum LcdBackend {
    Slv3(Slv3LcdDevice),
    WinUsb(ThreadedWinUsbSender),
    HidLcd(SharedHidLcd),
}

/// Temporary contention defers a frame without counting toward device recovery.
#[derive(Debug)]
struct LcdBusy;

impl std::fmt::Display for LcdBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LCD busy. Retry the frame later.")
    }
}
impl std::error::Error for LcdBusy {}

/// How long a frame send waits for the LCD mutex before deferring.
const LCD_BUSY_WAIT: Duration = Duration::from_millis(100);
const BRIGHTNESS_WRITE_INTERVAL: Duration = Duration::from_millis(250);
/// An H2 ring cycle cannot be abandoned midway and may run for 10 s.
const LCD_SENDER_STOP_WAIT: Duration = Duration::from_secs(12);

impl LcdBackend {
    pub(super) fn pause_for_wireless_image(&self) -> anyhow::Result<()> {
        let Self::WinUsb(sender) = self else {
            anyhow::bail!("H2 wireless image preparation requires its USB LCD worker");
        };
        sender.stream_control.cancel();
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        sender.send_before(LcdThreadMsg::PausePlayback(send), deadline)?;
        receive
            .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .map_err(|_| anyhow::anyhow!("H2 playback did not stop before wireless upload"))?
    }

    pub(super) fn startup_image_ready(&self) -> anyhow::Result<()> {
        match self {
            Self::Slv3(device) => device.startup_image_ready(),
            Self::WinUsb(sender) => sender
                .transport
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("LCD transport unavailable"))?
                .startup_image_ready(sender.shares_cooling),
            Self::HidLcd(_) => Ok(()),
        }
    }
    pub(super) fn upload_startup_image(
        &mut self,
        jpeg: Vec<u8>,
        stop: Arc<AtomicBool>,
        transfer: Arc<lianli_devices::startup_image::Transfer>,
    ) -> anyhow::Result<bool> {
        match self {
            Self::Slv3(device) => device.upload_startup_image(&jpeg, &stop, &transfer),
            Self::WinUsb(sender) => sender.upload_startup_image(jpeg, stop, transfer),
            Self::HidLcd(device) => {
                let _idle = device
                    .recovery_idle()
                    .ok_or_else(|| anyhow::anyhow!("LCD stream has not stopped"))?;
                let mut lcd = device
                    .try_lock_for(Duration::from_millis(100))
                    .ok_or_else(|| anyhow::anyhow!("LCD is busy"))?;
                lcd.upload_startup_image(&jpeg, &stop, &transfer)
            }
        }
    }
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
    transferred: Arc<AtomicBool>,
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
            transferred: Arc::new(AtomicBool::new(false)),
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
            self.transferred.clone(),
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
    pub(super) fn transferred(&self) -> Option<bool> {
        match self {
            Self::HidLcd(_, current) => current.try_lock().map(|current| {
                current
                    .as_ref()
                    .is_some_and(|worker| worker.transferred.load(Ordering::Acquire))
            }),
            Self::WinUsb(..) => None,
        }
    }

    pub(super) fn transfer_interrupted(&self) -> bool {
        match self {
            Self::HidLcd(..) => false,
            Self::WinUsb(_, control, _) => control.transfer_interrupted(),
        }
    }

    /// Called before feeding the encoder so a deferred reader cannot fill its pipe.
    pub(super) fn try_start_pending(&self) -> anyhow::Result<bool> {
        match self {
            Self::HidLcd(_, current) => {
                let mut current = current.lock();
                let Some(worker) = current.as_mut() else {
                    return Ok(true);
                };
                anyhow::ensure!(
                    !worker.handle.as_ref().is_some_and(JoinHandle::is_finished),
                    "HID H.264 worker stopped. Check daemon logs for the transfer error."
                );
                Ok(worker.try_start())
            }
            Self::WinUsb(..) => Ok(true),
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
                *owner = control.submit(Some(&owner), |stream_stop| {
                    tx.try_send(LcdThreadMsg::StreamH264Reader(stdout, fps, stream_stop))
                        .map_err(|e| anyhow::anyhow!("LCD stream restart was not accepted: {e}"))
                })?;
                Ok(())
            }
        }
    }
}

pub(super) enum LcdThreadMsg {
    PausePlayback(std::sync::mpsc::SyncSender<anyhow::Result<()>>),
    StartupImage(
        Vec<u8>,
        Arc<AtomicBool>,
        Arc<lianli_devices::startup_image::Transfer>,
        std::sync::mpsc::SyncSender<anyhow::Result<bool>>,
    ),
    Frame(Vec<u8>, Arc<FrameDelivery>),
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

fn is_transfer_interrupted(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<lianli_devices::winusb::lcd::LcdTransferInterrupted>()
        .is_some()
}

pub(super) fn failure_text(key: &str, prefix: &str, error: impl std::fmt::Display) -> UiText {
    let error = format!("{error:#}");
    UiText::new(
        UiMessage::new(key).param("error", &error),
        format!("{prefix}: {error}"),
    )
}

#[derive(Clone)]
pub(super) struct StreamFailure {
    message: UiText,
    interrupted: bool,
}

impl StreamFailure {
    fn new(message: UiText, interrupted: bool) -> Self {
        Self {
            message: message.truncated(2048),
            interrupted,
        }
    }

    fn from_error(message: UiText, error: &anyhow::Error) -> Self {
        Self::new(message, is_transfer_interrupted(error))
    }
}

impl From<&str> for StreamFailure {
    fn from(message: &str) -> Self {
        Self::new(message.into(), false)
    }
}

#[derive(Default)]
pub(super) struct StreamControl {
    current: Mutex<Arc<AtomicBool>>,
    failure: Mutex<Option<StreamFailure>>,
    transferred: Mutex<Option<Arc<AtomicBool>>>,
}

impl StreamControl {
    fn submit(
        &self,
        expected: Option<&Arc<AtomicBool>>,
        enqueue: impl FnOnce(Arc<AtomicBool>) -> anyhow::Result<()>,
    ) -> anyhow::Result<Arc<AtomicBool>> {
        let mut current = self.current.lock();
        if let Some(expected) = expected {
            anyhow::ensure!(
                Arc::ptr_eq(&current, expected) && !expected.load(Ordering::Acquire),
                "H.264 stream was replaced"
            );
        }
        let replacement = Arc::new(AtomicBool::new(false));
        // The enqueue must be nonblocking. Consumers acquire current before using stream state.
        enqueue(replacement.clone())?;
        current.store(true, Ordering::Release);
        *current = replacement.clone();
        // An interrupted device stays wedged until it is recreated, so a
        // restarted stream must not hide that from the service loop.
        self.failure.lock().take_if(|failure| !failure.interrupted);
        *self.transferred.lock() = Some(Arc::new(AtomicBool::new(false)));
        Ok(replacement)
    }

    fn transfer_interrupted(&self) -> bool {
        self.failure
            .lock()
            .as_ref()
            .is_some_and(|failure| failure.interrupted)
    }

    #[cfg(test)]
    fn next(&self) -> Arc<AtomicBool> {
        self.submit(None, |_| Ok(())).unwrap()
    }

    fn cancel(&self) {
        let current = self.current.lock();
        current.store(true, Ordering::Release);
        *self.failure.lock() = None;
        *self.transferred.lock() = None;
    }

    #[cfg(test)]
    fn restart(&self, expected: &Arc<AtomicBool>) -> Option<Arc<AtomicBool>> {
        self.submit(Some(expected), |_| Ok(())).ok()
    }

    fn transfer_observer(&self, expected: &Arc<AtomicBool>) -> Option<Arc<AtomicBool>> {
        let current = self.current.lock();
        if !Arc::ptr_eq(&current, expected) || expected.load(Ordering::Acquire) {
            return None;
        }
        self.transferred.lock().clone()
    }

    fn transferred(&self) -> Option<bool> {
        let current = self.current.lock();
        if current.load(Ordering::Acquire) {
            return None;
        }
        self.transferred
            .lock()
            .as_ref()
            .map(|flag| flag.load(Ordering::Acquire))
    }

    fn failed(&self, expected: &Arc<AtomicBool>, failure: StreamFailure) {
        let current = self.current.lock();
        if Arc::ptr_eq(&current, expected) && !expected.load(Ordering::Acquire) {
            *self.failure.lock() = Some(failure);
        }
    }

    fn take_failure(&self) -> Option<StreamFailure> {
        let current = self.current.lock();
        let failure = self.failure.lock().take();
        if failure.is_some() {
            current.store(true, Ordering::Release);
        }
        failure
    }
}

#[derive(Default)]
pub(super) struct FrameDelivery {
    retired: AtomicBool,
    failure: Mutex<Option<StreamFailure>>,
}

impl FrameDelivery {
    fn submit(&self, send: impl FnOnce() -> anyhow::Result<()>) {
        if self.retired.load(Ordering::Acquire) || self.failure.lock().is_some() {
            return;
        }
        if let Err(error) = send() {
            *self.failure.lock() =
                Some(StreamFailure::from_error(error.to_string().into(), &error));
        }
    }
}

pub(super) struct ThreadedWinUsbSender {
    shares_cooling: bool,
    transport: Option<lianli_devices::winusb::lcd::SharedTransport>,
    tx: std::sync::mpsc::SyncSender<LcdThreadMsg>,
    stream_control: Arc<StreamControl>,
    frame_delivery: Arc<FrameDelivery>,
    closing: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ThreadedWinUsbSender {
    fn upload_startup_image(
        &self,
        jpeg: Vec<u8>,
        stop: Arc<AtomicBool>,
        transfer: Arc<lianli_devices::startup_image::Transfer>,
    ) -> anyhow::Result<bool> {
        self.stream_control.cancel();
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        self.send_before(
            LcdThreadMsg::StartupImage(jpeg, stop.clone(), transfer, send),
            deadline,
        )?;
        match receive.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
            Ok(result) => result,
            Err(_) => {
                stop.store(true, Ordering::Release);
                anyhow::bail!("Startup upload timed out. Storage state is unknown")
            }
        }
    }
    pub(super) fn new(mut device: WinUsbLcdDevice, index: usize) -> Self {
        let shares_cooling = device.shares_cooling_transport();
        let transport = Some(device.shared_transport());
        let (tx, rx) = std::sync::mpsc::sync_channel::<LcdThreadMsg>(2);
        let stream_control = Arc::new(StreamControl::default());
        let worker_control = stream_control.clone();
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
                    LcdThreadMsg::StartupImage(jpeg, stop, transfer, reply) => {
                        let _ = reply.send(device.upload_startup_image(&jpeg, &stop, &transfer));
                    }
                    LcdThreadMsg::PausePlayback(reply) => {
                        let _ = reply.send(device.pause_for_wireless_image());
                    }
                    LcdThreadMsg::Frame(data, delivery) => {
                        delivery.submit(|| device.send_frame(&data))
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
                        let Some(observer) = worker_control.transfer_observer(&stop) else {
                            continue;
                        };
                        device.observe_h264_transfer(observer);
                        if let Err(e) = device.stream_h264(&path, looping, &stop, fps) {
                            worker_control.failed(
                                &stop,
                                StreamFailure::from_error(
                                    failure_text(
                                        "status.media.h264FileTransferFailed",
                                        "H.264 file transfer failed",
                                        &e,
                                    ),
                                    &e,
                                ),
                            );
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
                        let Some(observer) = worker_control.transfer_observer(&stop) else {
                            continue;
                        };
                        device.observe_h264_transfer(observer);
                        if let Err(e) = device.stream_h264_reader(&mut stdout, &stop, fps) {
                            worker_control.failed(
                                &stop,
                                StreamFailure::from_error(
                                    failure_text(
                                        "status.media.liveH264TransferFailed",
                                        "Live H.264 transfer failed",
                                        &e,
                                    ),
                                    &e,
                                ),
                            );
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
                        return;
                    }
                    LcdThreadMsg::SetBrightness(val) => {
                        if let Err(e) = device.set_brightness_val(val) {
                            warn!("LCD[{index}] set_brightness error: {e}");
                        }
                    }
                    LcdThreadMsg::Shutdown(reply) => {
                        let result =
                            lianli_transport::usb::with_teardown_io(Duration::from_secs(3), || {
                                device.stop_playback()?;
                                device.set_brightness_val(0)
                            });
                        let _ = reply.send(result);
                        return;
                    }
                    LcdThreadMsg::Stop => break,
                }
            }
            if let Err(error) =
                lianli_transport::usb::with_teardown_io(Duration::from_secs(3), || {
                    device.stop_playback()
                })
            {
                warn!("LCD[{index}] playback teardown failed: {error:#}");
            }
        });
        Self {
            transport,
            shares_cooling,
            tx,
            stream_control,
            frame_delivery: Arc::new(FrameDelivery::default()),
            closing,
            thread: Some(thread),
        }
    }

    fn stream_h264(&self, path: PathBuf, looping: bool, fps: f32) -> anyhow::Result<()> {
        self.stream_control.submit(None, |stop| {
            self.tx
                .try_send(LcdThreadMsg::StreamH264 {
                    path,
                    looping,
                    fps,
                    stop,
                })
                .map_err(|e| anyhow::anyhow!("LCD file stream was not accepted: {e}"))
        })?;
        Ok(())
    }

    fn stream_h264_reader(
        &self,
        stdout: std::process::ChildStdout,
        fps: f32,
    ) -> anyhow::Result<()> {
        self.stream_control.submit(None, |stop| {
            self.tx
                .try_send(LcdThreadMsg::StreamH264Reader(stdout, fps, stop))
                .map_err(|e| anyhow::anyhow!("LCD live stream was not accepted: {e}"))
        })?;
        Ok(())
    }

    fn set_brightness(&self, brightness: u8) -> anyhow::Result<()> {
        match self.tx.try_send(LcdThreadMsg::SetBrightness(brightness)) {
            Ok(()) => Ok(()),
            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                anyhow::bail!("LCD busy. Brightness change was not accepted.")
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                anyhow::bail!("LCD sender thread exited")
            }
        }
    }

    fn send_frame(&self, frame: &[u8]) -> anyhow::Result<()> {
        self.stream_control.cancel();
        match self.tx.try_send(LcdThreadMsg::Frame(
            frame.to_vec(),
            self.frame_delivery.clone(),
        )) {
            Ok(()) => Ok(()),
            Err(std::sync::mpsc::TrySendError::Full(_)) => Err(LcdBusy.into()),
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                anyhow::bail!("LCD sender thread exited")
            }
        }
    }

    fn reset_frame_delivery(&mut self) {
        self.frame_delivery.retired.store(true, Ordering::Release);
        self.frame_delivery = Arc::new(FrameDelivery::default());
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

    fn join_before(&mut self, deadline: std::time::Instant) -> bool {
        if let Some(worker) = self.thread.take() {
            while !worker.is_finished() && std::time::Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            if worker.is_finished() {
                if worker.join().is_err() {
                    warn!("LCD sender panicked");
                    return false;
                }
            } else {
                // The worker retains its transport until the current I/O returns.
                warn!("LCD sender did not stop before deadline; detaching");
                return false;
            }
        }
        true
    }

    fn shutdown(&mut self) -> anyhow::Result<()> {
        self.closing.store(true, Ordering::Release);
        self.stream_control.cancel();
        let deadline = std::time::Instant::now() + LCD_SENDER_STOP_WAIT;
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let result = self
            .send_before(LcdThreadMsg::Shutdown(tx), deadline)
            .and_then(|()| {
                rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                    .map_err(|_| anyhow::anyhow!("LCD shutdown acknowledgement timed out"))?
            });
        anyhow::ensure!(
            self.join_before(deadline),
            "LCD sender did not finish shutdown"
        );
        result
    }

    fn stop(&mut self) {
        if self.thread.is_none() {
            return;
        }
        self.closing.store(true, Ordering::Release);
        self.stream_control.cancel();
        let deadline = std::time::Instant::now() + LCD_SENDER_STOP_WAIT;
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
    retired_media: Option<RetiredSource>,
    removal: Option<Arc<()>>,
    playback_asset: Arc<MediaAsset>,
    media_pending: bool,
    media_paused: bool,
    source_failed: bool,
    source_selection: Arc<()>,
    media_stage: lianli_shared::ipc::MediaRuntimeStage,
    media_fallback: Option<UiText>,
    media_tx: Option<Sender<DaemonEvent>>,
    pub(super) lcd: LcdBackend,
    pub(super) asset: Arc<MediaAsset>,
    pub(super) screen: ScreenInfo,
    pub(super) custom_h264: bool,
    pub(super) frame_counter: u64,
    pub(super) consecutive_errors: u32,
    recovery_stop: Arc<AtomicBool>,
    recovery_thread: Option<JoinHandle<()>>,
    initialization: LcdInitialization,
    /// Set when the device definitively does not support recovery, so the
    /// periodic retry stops probing it.
    recovery_unsupported: bool,
    // Latest value waiting for initialization, the device lock, or HID write spacing.
    pending_brightness: Option<u8>,
    brightness_status: Option<lianli_shared::ipc::LcdBrightnessStatus>,
    brightness_retries: u8,
    next_brightness_attempt: Option<Instant>,
    transfer_recovery: TransferRecovery,
}

enum LcdInitialization {
    Pending,
    Ready,
    Failed(String),
}

const TRANSFER_RECOVERY_ATTEMPTS: u32 = 5;
const TRANSFER_RECOVERY_HEALTHY_RESET: Duration = Duration::from_secs(300);

/// Recreation budget for a WinUSB LCD whose packet transfer was interrupted.
/// It outlives the target it belongs to, so the replacement continues the backoff.
#[derive(Default)]
pub(super) struct TransferRecovery {
    attempts: u32,
    last_failure: Option<Instant>,
    due: Option<Instant>,
    pending: bool,
}

impl TransferRecovery {
    /// Returns the attempt number and delay, or `None` once the budget is spent.
    fn schedule(&mut self, now: Instant) -> Option<(u32, Duration)> {
        self.pending = true;
        if let Some(due) = self.due {
            return Some((self.attempts, due.saturating_duration_since(now)));
        }
        if self
            .last_failure
            .is_some_and(|at| now.saturating_duration_since(at) > TRANSFER_RECOVERY_HEALTHY_RESET)
        {
            self.attempts = 0;
        }
        self.last_failure = Some(now);
        if self.attempts >= TRANSFER_RECOVERY_ATTEMPTS {
            return None;
        }
        self.attempts += 1;
        let delay = Duration::from_secs(1 << (self.attempts - 1));
        self.due = Some(now + delay);
        Some((self.attempts, delay))
    }

    pub(super) fn expired(&self, now: Instant) -> bool {
        self.last_failure
            .is_none_or(|at| now.saturating_duration_since(at) > TRANSFER_RECOVERY_HEALTHY_RESET)
    }

    fn take_due(&mut self, now: Instant) -> bool {
        if self.due.is_some_and(|due| now >= due) {
            self.due = None;
            return true;
        }
        false
    }

    fn retry_now(&mut self, now: Instant) -> bool {
        if !self.pending {
            return false;
        }
        self.attempts = 0;
        self.due = Some(now);
        true
    }

    fn for_replacement(self) -> Self {
        Self {
            due: None,
            pending: false,
            ..self
        }
    }
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
        device_identity: String,
        lcd: LcdBackend,
        asset: Arc<MediaAsset>,
        screen: ScreenInfo,
        custom_h264: bool,
        tx: Option<Sender<DaemonEvent>>,
    ) -> Self {
        let key = asset.config_key.clone();
        let media: Box<dyn FrameSource> = Box::new(NoopFrameSource);
        let recovery_stop = Arc::new(AtomicBool::new(false));
        Self {
            index,
            key,
            device_identity,
            lcd,
            media,
            playback_asset: asset.clone(),
            media_pending: true,
            media_paused: false,
            retired_media: None,
            removal: None,
            source_failed: false,
            source_selection: Arc::new(()),
            media_stage: lianli_shared::ipc::MediaRuntimeStage::StartingSource,
            media_fallback: None,
            media_tx: tx,
            asset,
            screen,
            custom_h264,
            frame_counter: 0,
            consecutive_errors: 0,
            recovery_stop,
            recovery_thread: None,
            initialization: LcdInitialization::Ready,
            recovery_unsupported: false,
            pending_brightness: None,
            brightness_status: None,
            brightness_retries: 0,
            next_brightness_attempt: None,
            transfer_recovery: TransferRecovery::default(),
        }
    }

    pub(super) fn maybe_start_recovery(&mut self, tx: Option<Sender<DaemonEvent>>, wait: Duration) {
        if !self.is_initialized()
            || self.removal.is_some()
            || self.recovery_thread.is_some()
            || self.recovery_unsupported
        {
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
            if firmware_known {
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

    pub(super) fn wait_for_initialization(&mut self) {
        self.initialization = LcdInitialization::Pending;
    }

    pub(super) fn is_initialized(&self) -> bool {
        matches!(self.initialization, LcdInitialization::Ready)
    }

    pub(super) fn is_initializing(&self) -> bool {
        matches!(self.initialization, LcdInitialization::Pending)
    }

    pub(super) fn finish_initialization(&mut self, error: Option<&str>) {
        self.initialization = match error {
            Some(error) => LcdInitialization::Failed(error.chars().take(2048).collect()),
            None => LcdInitialization::Ready,
        };
        if let LcdInitialization::Failed(error) = &self.initialization {
            self.media_stage = lianli_shared::ipc::MediaRuntimeStage::Failed;
            self.media_fallback = Some(UiText::new(
                UiMessage::new("status.media.initializationFailed").param("error", error),
                format!(
                    "LCD initialization failed: {error}. Restart the daemon to retry initialization."
                ),
            ));
        }
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
        if self.removal.is_some() {
            return;
        }
        if self.media.is_autonomous() && matches!(&self.lcd, LcdBackend::WinUsb(_)) {
            self.pause_source();
            self.media_pending = true;
            self.source_selection = Arc::new(());
        }
        self.pending_brightness = Some(brightness);
        self.brightness_status = Some(lianli_shared::ipc::LcdBrightnessStatus {
            request_id: None,
            brightness,
            pending: true,
            error: None,
            error_ui: None,
        });
        self.brightness_retries = 3;
        self.flush_pending_brightness(wireless, builder);
    }

    pub(super) fn request_brightness(
        &mut self,
        wireless: Option<&WirelessController>,
        builder: &mut PacketBuilder,
        brightness: u8,
        request_id: Option<String>,
    ) -> Result<bool, String> {
        if self.removal.is_some() {
            return Err("LCD is being removed".into());
        }
        self.apply_brightness(wireless, builder, brightness);
        if let Some(status) = &mut self.brightness_status {
            status.request_id = request_id;
        }
        if let Some(error) = self
            .brightness_status
            .as_ref()
            .and_then(|status| status.error.as_ref())
        {
            return Err(error.clone());
        }
        // WinUSB acknowledges queue acceptance, not completion of the write.
        Ok(self.pending_brightness.is_none() && !matches!(&self.lcd, LcdBackend::WinUsb(_)))
    }

    pub(super) fn brightness_status(&self) -> Option<&lianli_shared::ipc::LcdBrightnessStatus> {
        self.brightness_status.as_ref()
    }

    fn update_brightness_status(&mut self, brightness: u8, pending: bool, error: Option<UiText>) {
        let request_id = self
            .brightness_status
            .as_ref()
            .and_then(|status| status.request_id.clone());
        let (error, error_ui) = UiText::split(error);
        self.brightness_status = Some(lianli_shared::ipc::LcdBrightnessStatus {
            request_id,
            brightness,
            pending,
            error,
            error_ui,
        });
    }

    pub(super) fn flush_pending_brightness(
        &mut self,
        wireless: Option<&WirelessController>,
        builder: &mut PacketBuilder,
    ) {
        let Some(brightness) = self.pending_brightness else {
            return;
        };
        if let LcdInitialization::Failed(error) = &self.initialization {
            let error = UiText::new(
                UiMessage::new("status.brightness.initializationFailed").param("error", error),
                format!("LCD initialization failed: {error}"),
            );
            self.update_brightness_status(brightness, false, Some(error));
            self.pending_brightness = None;
            return;
        }
        if !self.is_initialized() {
            return;
        }
        let result = if let LcdBackend::HidLcd(device) = &self.lcd {
            if self
                .next_brightness_attempt
                .is_some_and(|next| Instant::now() < next)
            {
                return;
            }
            // Leave USB time for video and telemetry between slider writes.
            self.next_brightness_attempt = Some(Instant::now() + BRIGHTNESS_WRITE_INTERVAL);
            let Some(guard) = device.try_lock_for(LCD_BUSY_WAIT) else {
                return;
            };
            let result = guard.set_brightness(brightness);
            self.next_brightness_attempt = Some(Instant::now() + BRIGHTNESS_WRITE_INTERVAL);
            result
        } else {
            self.lcd.set_brightness(wireless, builder, brightness)
        };
        match result {
            Ok(()) => {
                self.pending_brightness = None;
                self.update_brightness_status(brightness, false, None);
            }
            Err(error) => {
                self.brightness_retries = self.brightness_retries.saturating_sub(1);
                if self.brightness_retries == 0 {
                    self.pending_brightness = None;
                    warn!(
                        "LCD[{}] brightness could not be applied after three attempts: {error:#}",
                        self.index
                    );
                }
                self.update_brightness_status(
                    brightness,
                    self.pending_brightness.is_some(),
                    Some(failure_text(
                        "status.brightness.writeFailed",
                        "LCD brightness write failed",
                        &error,
                    )),
                );
            }
        }
    }

    pub(super) fn matches(&self, identity: &str, key: &ConfigKey) -> bool {
        self.device_identity == identity && key == &self.key
    }

    pub(super) fn swap_media(
        &mut self,
        asset: Arc<MediaAsset>,
        custom_h264: bool,
        tx: Option<Sender<DaemonEvent>>,
    ) {
        self.key = asset.config_key.clone();
        if self.media_stage == lianli_shared::ipc::MediaRuntimeStage::Failed {
            self.media_stage = lianli_shared::ipc::MediaRuntimeStage::StartingSource;
        }
        self.asset = Arc::clone(&asset);
        self.custom_h264 = custom_h264;
        self.media_pending = true;
        self.source_failed = false;
        self.source_selection = Arc::new(());
        self.media_tx = tx;
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

    pub(super) fn source_request(&self) -> Option<SourceRequest> {
        (self.is_initialized()
            && self.media_pending
            && self.removal.is_none()
            && self.retired_media.is_none()
            && self.pending_brightness.is_none()
            && self.needs_source_preparation())
        .then(|| SourceRequest {
            index: self.index,
            selection: Arc::downgrade(&self.source_selection),
            asset: self.asset.clone(),
            screen: self.screen,
            custom_h264: self.custom_h264,
            tx: self.media_tx.clone(),
        })
    }

    fn needs_source_preparation(&self) -> bool {
        matches!(
            &self.asset.kind,
            MediaAssetKind::Sensor { .. } | MediaAssetKind::Custom { .. }
        )
    }

    pub(super) fn accepts_source(&self, result: &SourceResult) -> bool {
        self.is_initialized()
            && self.index == result.index
            && self.media_pending
            && self.removal.is_none()
            && self.retired_media.is_none()
            && self.pending_brightness.is_none()
            && std::sync::Weak::ptr_eq(&result.selection, &Arc::downgrade(&self.source_selection))
    }

    pub(super) fn install_source(&mut self, result: SourceResult) {
        if !self.accepts_source(&result) {
            return;
        }
        let mut fallback = None;
        let media = result.result.try_accept(|prepared| {
            let prepared = match prepared {
                Ok(prepared) => prepared,
                Err(error) => return Err((Err(error.clone()), anyhow::anyhow!(error))),
            };
            attach_prepared_source(prepared, &self.lcd, &mut fallback)
                .map_err(|(prepared, error)| (Ok(prepared), error))
        });
        self.finish_source_install(media, fallback);
    }

    fn finish_source_install(
        &mut self,
        media: anyhow::Result<Box<dyn FrameSource>>,
        fallback: Option<UiText>,
    ) {
        let media = match media {
            Ok(media) => media,
            Err(error) => {
                self.media_pending = false;
                self.source_failed = true;
                self.media_fallback = Some(
                    failure_text(
                        "status.media.attachmentFailed",
                        "Media attachment failed",
                        &error,
                    )
                    .truncated(2048),
                );
                return;
            }
        };
        self.begin_source_install(media.is_autonomous());
        self.media = media;
        self.media_fallback = fallback;
        self.media_pending = false;
    }

    fn begin_source_install(&mut self, autonomous: bool) {
        assert!(self.retired_media.is_none());
        self.media.request_stop();
        self.retired_media = Some(RetiredSource {
            source: std::mem::replace(&mut self.media, Box::new(NoopFrameSource)),
            _asset: self.playback_asset.clone(),
        });
        if let LcdBackend::WinUsb(sender) = &mut self.lcd {
            if !autonomous {
                sender.stream_control.cancel();
            }
            sender.reset_frame_delivery();
        }
        self.playback_asset = self.asset.clone();
        self.media_paused = false;
        self.source_failed = false;
        self.media_stage = lianli_shared::ipc::MediaRuntimeStage::StartingSource;
        self.media_fallback = None;
        self.frame_counter = 0;
    }

    fn pause_source(&mut self) {
        self.media.request_stop();
        self.media_paused = true;
        if let LcdBackend::WinUsb(sender) = &self.lcd {
            sender.stream_control.cancel();
        }
    }

    pub(super) fn playback_failure_key(&self) -> Option<ConfigKey> {
        (!self.media_pending && !self.source_failed).then(|| self.playback_asset.config_key.clone())
    }

    pub(super) fn send_frame(
        &mut self,
        wireless: Option<&WirelessController>,
        builder: &mut PacketBuilder,
    ) -> Result<bool, SendError> {
        if !self.is_initialized()
            || self.removal.is_some()
            || self.media_stage == lianli_shared::ipc::MediaRuntimeStage::Failed
        {
            return Ok(false);
        }
        if self.media_pending && self.retired_media.is_none() && !self.needs_source_preparation() {
            if self.pending_brightness.is_some() {
                return Ok(false);
            }
            let media = make_frame_source(Arc::clone(&self.asset), self.media_tx.clone())
                .map_err(SendError::Other)?;
            self.begin_source_install(media.is_autonomous());
            self.media = media;
            self.media_pending = false;
        }
        if self.media_paused {
            return Ok(false);
        }
        let frame_failure = match &self.lcd {
            LcdBackend::WinUsb(sender) => sender.frame_delivery.failure.lock().clone(),
            _ => None,
        };
        if let Some(failure) = frame_failure {
            warn!(
                "LCD[{}] JPEG transfer failed: {}",
                self.index, failure.message
            );
            let error = failure_text(
                "status.media.jpegTransferFailed",
                "JPEG transfer failed",
                &failure.message,
            );
            if failure.interrupted {
                return Err(self.interrupt_transfer(error));
            }
            self.media_stage = lianli_shared::ipc::MediaRuntimeStage::Failed;
            self.pause_source();
            return Err(SendError::Stopped(error));
        }
        // H.264 / autonomous sources: kick off streaming on the first call,
        // then short-circuit (their threads push frames directly to the LCD).
        if self.media.is_autonomous() {
            let stream_failure = match &self.lcd {
                LcdBackend::WinUsb(sender) => sender.stream_control.take_failure(),
                _ => None,
            };
            if let Some(failure) = stream_failure {
                if failure.interrupted {
                    return Err(self.interrupt_transfer(failure.message));
                }
                self.media_stage = lianli_shared::ipc::MediaRuntimeStage::Failed;
                self.pause_source();
                return Err(SendError::Stopped(failure.message));
            }
            if self.media.has_exited() {
                self.media_stage = lianli_shared::ipc::MediaRuntimeStage::Failed;
                self.pause_source();
                return Err(SendError::Stopped(UiText::new(
                    UiMessage::new("status.media.h264PlaybackFailed"),
                    "H.264 playback failed. Check daemon logs, then use Retry failed media in Installation Health.",
                )));
            }
            if let Err(error) = self.media.start(&self.lcd) {
                if self.media.has_exited() {
                    self.media_stage = lianli_shared::ipc::MediaRuntimeStage::Failed;
                    self.pause_source();
                    return Err(SendError::Stopped(error.to_string().into()));
                }
                return Err(SendError::Other(error));
            }
            self.media_stage = lianli_shared::ipc::MediaRuntimeStage::AutonomousSourceConfigured;
            return Ok(true);
        }

        if self.media.has_exited() {
            self.media_stage = lianli_shared::ipc::MediaRuntimeStage::Failed;
            self.pause_source();
            return Err(SendError::Stopped(UiText::new(
                UiMessage::new("status.media.jpegRenderingFailed"),
                "JPEG rendering failed. Check daemon logs, then use Retry failed media in Installation Health.",
            )));
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
            Err(e) if e.downcast_ref::<LcdBusy>().is_some() => {
                debug!("[devices] LCD[{}] busy, deferring frame send", self.index);
                return Ok(false);
            }
            Err(err) if is_transfer_interrupted(&err) => {
                return Err(self.interrupt_transfer(failure_text(
                    "status.media.frameTransferFailed",
                    "LCD frame transfer failed",
                    &err,
                )));
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
        self.media_stage = lianli_shared::ipc::MediaRuntimeStage::FrameSubmitted;
        Ok(true)
    }

    pub(super) fn media_status(&self) -> lianli_shared::ipc::MediaRuntimeStatus {
        lianli_shared::ipc::MediaRuntimeStatus {
            stage: if matches!(self.initialization, LcdInitialization::Failed(_))
                || self.source_failed
                || self.removal.is_some()
            {
                lianli_shared::ipc::MediaRuntimeStage::Failed
            } else {
                self.media_stage
            },
            fps_limit: self.playback_asset.stream_fps,
            hardware_video_allowed: self.playback_asset.hardware_video,
            fallback_reason: self
                .media_fallback
                .as_ref()
                .map(|text| text.english.clone()),
            fallback_reason_ui: self
                .media_fallback
                .as_ref()
                .and_then(|text| text.ui.clone()),
            h264_transfer_started: match &self.lcd {
                LcdBackend::WinUsb(sender) if self.media.is_autonomous() => {
                    sender.stream_control.transferred()
                }
                _ => self.media.transferred(),
            },
            encoder: match &self.playback_asset.kind {
                MediaAssetKind::H264Stream { encoder, .. } => encoder.clone(),
                _ => self.media.encoder_status(),
            },
        }
    }

    fn interrupt_transfer(&mut self, error: UiText) -> SendError {
        self.media_stage = lianli_shared::ipc::MediaRuntimeStage::Failed;
        self.pause_source();
        let error = match self.transfer_recovery.schedule(Instant::now()) {
            Some((attempt, delay)) => {
                warn!(
                    "LCD[{}] transfer interrupted; recreating the LCD in {}s (attempt {attempt}/{TRANSFER_RECOVERY_ATTEMPTS})",
                    self.index,
                    delay.as_secs()
                );
                error
            }
            None => {
                warn!(
                    "LCD[{}] transfer interrupted; automatic recovery gave up after {TRANSFER_RECOVERY_ATTEMPTS} attempts",
                    self.index
                );
                UiText::new(
                    UiMessage::new("status.media.recoveryGaveUp")
                        .param("attempts", TRANSFER_RECOVERY_ATTEMPTS)
                        .param("error", &error),
                    format!(
                        "Automatic LCD recovery gave up after {TRANSFER_RECOVERY_ATTEMPTS} attempts. Use Retry failed media in Installation Health. {error}"
                    ),
                )
            }
        };
        let error = error.truncated(2048);
        self.media_fallback = Some(error.clone());
        SendError::Interrupted(error)
    }

    pub(super) fn due_transfer_recovery(&mut self, now: Instant) -> Option<DaemonEvent> {
        if self.removal.is_some() || !self.transfer_recovery.take_due(now) {
            return None;
        }
        info!(
            "[devices] LCD[{}] recreating the LCD after an interrupted transfer",
            self.index
        );
        let error = self.media_fallback.clone().unwrap_or_else(|| {
            UiText::new(
                UiMessage::new("status.media.transferInterrupted"),
                "LCD packet transfer was interrupted",
            )
        });
        self.removal_event(error)
    }

    /// The budget a replacement must continue, if any recreation has been spent.
    pub(super) fn take_transfer_recovery(&mut self) -> Option<TransferRecovery> {
        let recovery = std::mem::take(&mut self.transfer_recovery).for_replacement();
        (recovery.attempts > 0).then_some(recovery)
    }

    pub(super) fn restore_transfer_recovery(&mut self, recovery: TransferRecovery) {
        self.transfer_recovery = recovery;
    }

    /// Explicit user retry: recreate an interrupted LCD now with a fresh budget.
    pub(super) fn retry_interrupted_transfer(&mut self) {
        if !self.is_initialized() || self.removal.is_some() {
            return;
        }
        if self.transfer_recovery.retry_now(Instant::now()) {
            info!(
                "[devices] LCD[{}] retrying after an interrupted transfer",
                self.index
            );
        }
    }

    /// A wedged LCD is left to its recreation backoff; restarting media in
    /// place cannot reach it.
    pub(super) fn retry_failed_source(&mut self) {
        if !self.is_initialized() || self.removal.is_some() || self.transfer_recovery.pending {
            return;
        }
        if self.source_failed || self.media_stage == lianli_shared::ipc::MediaRuntimeStage::Failed {
            if let LcdBackend::WinUsb(sender) = &mut self.lcd {
                sender.reset_frame_delivery();
            }
            self.media_stage = lianli_shared::ipc::MediaRuntimeStage::StartingSource;
            self.media_pending = true;
            self.source_failed = false;
            self.source_selection = Arc::new(());
            self.media_fallback = None;
        }
    }

    pub(super) fn removal_event(&mut self, error: UiText) -> Option<DaemonEvent> {
        let removal = self.request_removal()?;
        let error = error.truncated(2048);
        self.media_fallback = Some(error.clone());
        Some(DaemonEvent::RemoveFailedLcd {
            target_index: self.index,
            removal,
            key: self.asset.config_key.clone(),
            error,
        })
    }

    fn request_removal(&mut self) -> Option<std::sync::Weak<()>> {
        if self.removal.is_some() {
            return None;
        }
        let removal = Arc::new(());
        let token = Arc::downgrade(&removal);
        self.removal = Some(removal);
        self.media_stage = lianli_shared::ipc::MediaRuntimeStage::Failed;
        self.media_pending = false;
        self.media.request_stop();
        self.recovery_stop.store(true, Ordering::Relaxed);
        self.source_selection = Arc::new(());
        if self.pending_brightness.is_some() {
            if let Some(status) = &mut self.brightness_status {
                status.pending = false;
                status.error = Some("LCD removed before brightness delivery".into());
                status.error_ui = Some(UiMessage::new("status.brightness.lcdRemoved"));
            }
        }
        self.pending_brightness = None;
        if let LcdBackend::WinUsb(sender) = &self.lcd {
            sender.stream_control.cancel();
        }
        Some(token)
    }

    pub(super) fn matches_removal(&self, token: &std::sync::Weak<()>) -> bool {
        self.removal
            .as_ref()
            .is_some_and(|removal| std::sync::Weak::ptr_eq(token, &Arc::downgrade(removal)))
    }

    pub(super) fn take_finished_retirement(&mut self) -> Option<RetiredSource> {
        if self.retired_media.as_ref()?.source.retirement_complete() {
            self.retired_media.take()
        } else {
            None
        }
    }

    pub(super) fn return_retirement(&mut self, source: RetiredSource) {
        assert!(self.retired_media.is_none());
        self.retired_media = Some(source);
    }

    pub(super) fn stop(&mut self) {
        self.recovery_stop.store(true, Ordering::Relaxed);
        self.media_pending = false;
        self.source_selection = Arc::new(());
        let retired = self
            .retired_media
            .as_mut()
            .map(|source| source.source.as_mut());
        if !stop_frame_sources(self.media.as_mut(), retired, Duration::from_secs(3)) {
            warn!(
                "LCD[{}] media workers are still finishing shutdown",
                self.index
            );
        }
        self.media = Box::new(NoopFrameSource);
        drop(self.retired_media.take());
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
            } else if t.join().is_err() {
                warn!(
                    "LCD[{}] recovery thread panicked during shutdown",
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

pub(super) struct RetiredSource {
    source: Box<dyn FrameSource>,
    _asset: Arc<MediaAsset>,
}

/// A source of JPEG frames to push to an LCD, or an autonomous H.264
/// pipeline that streams directly to the device.
pub(super) trait FrameSource: Send {
    fn request_stop(&mut self) {}

    fn retirement_complete(&self) -> bool {
        true
    }

    fn transferred(&self) -> Option<bool> {
        None
    }
    fn encoder_status(&self) -> Option<lianli_shared::ipc::MediaEncoderStatus> {
        None
    }
    fn has_exited(&self) -> bool {
        false
    }
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

fn stop_frame_sources(
    current: &mut dyn FrameSource,
    mut retired: Option<&mut (dyn FrameSource + 'static)>,
    timeout: Duration,
) -> bool {
    current.request_stop();
    if let Some(source) = &mut retired {
        source.request_stop();
    }
    let deadline = std::time::Instant::now() + timeout;
    while !current.retirement_complete()
        || retired
            .as_ref()
            .is_some_and(|source| !source.retirement_complete())
    {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return false;
        }
        thread::sleep(remaining.min(Duration::from_millis(10)));
    }
    true
}

struct NoopFrameSource;
impl FrameSource for NoopFrameSource {}

// ─── JPEG sources ──────────────────────────────────────────────────────

struct StaticSource {
    frame: Arc<lianli_media::Retained<Vec<u8>>>,
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
    frames: Arc<lianli_media::Retained<Vec<Vec<u8>>>>,
    sent_index: usize,
    pending_index: usize,
}
impl FrameSource for VideoSource {
    fn request_stop(&mut self) {
        self.player.request_stop();
    }

    fn retirement_complete(&self) -> bool {
        self.player.retirement_complete()
    }

    fn next_frame(&mut self) -> Option<&[u8]> {
        let idx = self.player.get_frame_index();
        if idx <= self.sent_index || self.frames.is_empty() {
            return None;
        }
        let ret = Some(self.frames[idx % self.frames.len()].as_slice());
        self.pending_index = idx;
        ret
    }
    fn mark_sent(&mut self) {
        self.sent_index = self.pending_index;
    }
}

struct SensorSource {
    renderer: Arc<AsyncSensorRenderer>,
    cached: Vec<u8>,
    sent_index: usize,
    pending_index: usize,
}
impl FrameSource for SensorSource {
    fn request_stop(&mut self) {
        self.renderer.request_stop();
    }

    fn retirement_complete(&self) -> bool {
        self.renderer.retirement_complete()
    }

    fn has_exited(&self) -> bool {
        self.renderer.has_exited()
    }

    fn next_frame(&mut self) -> Option<&[u8]> {
        let idx = self.renderer.get_frame_index();
        if idx <= self.sent_index {
            return None;
        }
        if self.pending_index != idx {
            self.cached = self.renderer.get_current_frame();
            self.pending_index = idx;
        }
        Some(self.cached.as_slice())
    }
    fn mark_sent(&mut self) {
        self.sent_index = self.pending_index;
    }
}

struct CustomSource {
    renderer: Arc<AsyncCustomRenderer>,
    cached: Vec<u8>,
    sent_index: usize,
    pending_index: usize,
}
impl FrameSource for CustomSource {
    fn request_stop(&mut self) {
        self.renderer.request_stop();
    }

    fn retirement_complete(&self) -> bool {
        self.renderer.retirement_complete()
    }

    fn has_exited(&self) -> bool {
        self.renderer.has_exited()
    }

    fn next_frame(&mut self) -> Option<&[u8]> {
        let idx = self.renderer.get_frame_index();
        if idx <= self.sent_index {
            return None;
        }
        if self.pending_index != idx {
            self.cached = self.renderer.get_current_frame();
            self.pending_index = idx;
        }
        Some(self.cached.as_slice())
    }
    fn mark_sent(&mut self) {
        self.sent_index = self.pending_index;
    }
}

// ─── H.264 autonomous sources ──────────────────────────────────────────

struct H264FileSource {
    transferred: Arc<AtomicBool>,
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
            transferred: Arc::new(AtomicBool::new(false)),
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
    fn request_stop(&mut self) {
        self.hid_stop.store(true, Ordering::Relaxed);
        if let Some(worker) = &self.hid_thread {
            worker.thread().unpark();
        }
    }

    fn retirement_complete(&self) -> bool {
        self.hid_thread.as_ref().is_none_or(JoinHandle::is_finished)
    }

    fn transferred(&self) -> Option<bool> {
        self.hid_completed
            .as_ref()
            .map(|_| self.transferred.load(Ordering::Acquire))
    }
    fn has_exited(&self) -> bool {
        self.started
            && self.hid_completed.is_some()
            && self.hid_thread.as_ref().is_none_or(JoinHandle::is_finished)
            && !self
                .hid_completed
                .as_ref()
                .is_some_and(|completed| completed.load(Ordering::Acquire))
    }
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
                    anyhow::bail!("HID H.264 playback failed. Save media settings to retry.");
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
                let transferred = self.transferred.clone();
                let Some(lease) = lcd.begin_stream() else {
                    return Ok(());
                };
                self.hid_thread = Some(thread::spawn(move || {
                    let _lease = lease;
                    if stream_h264_file_to_hid(lcd, file, looping, fps, stop, transferred) {
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
    renderer: Arc<AsyncCustomH264Renderer>,
}
impl FrameSource for CustomH264Source {
    fn request_stop(&mut self) {
        self.renderer.request_stop();
    }

    fn retirement_complete(&self) -> bool {
        self.renderer.retirement_complete()
    }

    fn transferred(&self) -> Option<bool> {
        self.renderer.transferred()
    }
    fn encoder_status(&self) -> Option<lianli_shared::ipc::MediaEncoderStatus> {
        Some(self.renderer.encoder_status())
    }
    fn has_exited(&self) -> bool {
        self.renderer.has_exited()
    }
    fn is_autonomous(&self) -> bool {
        true
    }
}

struct SensorH264Source {
    renderer: Arc<AsyncSensorH264Renderer>,
}
impl FrameSource for SensorH264Source {
    fn request_stop(&mut self) {
        self.renderer.request_stop();
    }

    fn retirement_complete(&self) -> bool {
        self.renderer.retirement_complete()
    }

    fn transferred(&self) -> Option<bool> {
        self.renderer.transferred()
    }
    fn encoder_status(&self) -> Option<lianli_shared::ipc::MediaEncoderStatus> {
        Some(self.renderer.encoder_status())
    }
    fn has_exited(&self) -> bool {
        self.renderer.has_exited()
    }
    fn is_autonomous(&self) -> bool {
        true
    }
}

/// Construct the appropriate `FrameSource` for a given media asset + LCD combo.
pub(super) fn make_jpeg_source(
    asset: Arc<MediaAsset>,
    tx: Option<Sender<DaemonEvent>>,
    screen: &ScreenInfo,
) -> Option<Box<dyn FrameSource>> {
    match &asset.kind {
        MediaAssetKind::Sensor { asset: sensor } => {
            let renderer = Arc::new(AsyncSensorRenderer::new(
                tx,
                sensor.clone(),
                asset.clone(),
                screen.needs_keepalive,
            ));
            Some(Box::new(SensorSource {
                cached: renderer.get_current_frame(),
                renderer,
                sent_index: 0,
                pending_index: 0,
            }))
        }
        MediaAssetKind::Custom { asset: custom } => {
            let renderer = Arc::new(AsyncCustomRenderer::new(
                tx,
                custom.clone(),
                asset.clone(),
                screen.needs_keepalive,
            ));
            Some(Box::new(CustomSource {
                cached: renderer.get_current_frame(),
                renderer,
                sent_index: 0,
                pending_index: 0,
            }))
        }
        _ => None,
    }
}

fn attach_prepared_source(
    prepared: PreparedSource,
    lcd: &LcdBackend,
    fallback: &mut Option<UiText>,
) -> Result<Box<dyn FrameSource>, (PreparedSource, anyhow::Error)> {
    match prepared {
        PreparedSource::Sensor(source) => source
            .start(lcd)
            .map(|renderer| {
                Box::new(SensorH264Source {
                    renderer: Arc::new(renderer),
                }) as Box<dyn FrameSource>
            })
            .map_err(|(source, error)| (PreparedSource::Sensor(source), error)),
        PreparedSource::Custom(source) => source
            .start(lcd)
            .map(|renderer| {
                Box::new(CustomH264Source {
                    renderer: Arc::new(renderer),
                }) as Box<dyn FrameSource>
            })
            .map_err(|(source, error)| (PreparedSource::Custom(source), error)),
        PreparedSource::Jpeg {
            source,
            fallback: reason,
        } => {
            *fallback = reason;
            Ok(source)
        }
    }
}

fn make_frame_source(
    asset: Arc<MediaAsset>,
    tx: Option<Sender<DaemonEvent>>,
) -> anyhow::Result<Box<dyn FrameSource>> {
    Ok(match &asset.kind {
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
                pending_index: 0,
            })
        }
        MediaAssetKind::H264Stream {
            path, looping, fps, ..
        } => Box::new(H264FileSource::new(path.clone(), *looping, *fps)),
        MediaAssetKind::Sensor { .. } | MediaAssetKind::Custom { .. } => {
            anyhow::bail!("Media source requires background preparation")
        }
    })
}

/// True when the file finished on its own and the worker must not restart
fn stream_h264_file_to_hid(
    lcd: SharedHidLcd,
    mut file: std::fs::File,
    looping: bool,
    fps: f32,
    stop: Arc<AtomicBool>,
    transferred: Arc<AtomicBool>,
) -> bool {
    use lianli_devices::hydroshift_lcd::{find_au_split, pace_frame};
    use std::io::{Read, Seek, SeekFrom};
    use std::time::Instant;

    let stopped = || stop.load(Ordering::Relaxed);
    let Some(frame_interval) =
        hid_stream_frame_interval(&lcd, fps, &stopped, Duration::from_secs(3))
    else {
        return false;
    };
    let mut read_buf = vec![0u8; 64 * 1024];
    let mut next_deadline = Instant::now() + frame_interval;
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
                transferred.store(true, Ordering::Release);
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
            transferred.store(true, Ordering::Release);
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
    Stopped(UiText),
    /// The device stays wedged until the target recreates it.
    Interrupted(UiText),
    Usb(lianli_transport::TransportError),
    Other(anyhow::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn stream_failure_belongs_only_to_its_uncancelled_generation() {
        let control = StreamControl::default();
        let old = control.next();
        control.failed(&old, "old transfer failure".into());
        assert_eq!(
            control
                .take_failure()
                .map(|failure| failure.message.english)
                .as_deref(),
            Some("old transfer failure")
        );
        assert!(control.restart(&old).is_none());
        let current = control.next();
        assert!(control.take_failure().is_none());
        control.failed(&old, "late old failure".into());
        assert!(control.take_failure().is_none());
        control.failed(&current, "界".repeat(3000).as_str().into());
        assert_eq!(
            control
                .take_failure()
                .unwrap()
                .message
                .english
                .chars()
                .count(),
            2048
        );
        control.cancel();
        control.failed(&current, "cancelled transfer".into());
        assert!(control.take_failure().is_none());
    }

    fn interrupted_transfer() -> StreamFailure {
        let error =
            anyhow::Error::from(lianli_transport::TransportError::Usb(rusb::Error::Timeout))
                .context(lianli_devices::winusb::lcd::LcdTransferInterrupted)
                .context("streaming H.264");
        StreamFailure::from_error(
            failure_text(
                "status.media.liveH264TransferFailed",
                "Live H.264 transfer failed",
                &error,
            ),
            &error,
        )
    }

    #[test]
    fn interrupted_transfer_survives_stream_restarts_and_blocks_encoder_restart() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(1);
        let control = Arc::new(StreamControl::default());
        let owner = control.next();
        let restarter = StreamRestarter::WinUsb(tx, control.clone(), Mutex::new(owner.clone()));
        control.failed(&owner, "encoder pipe closed".into());
        assert!(!restarter.transfer_interrupted());
        let owner = control.restart(&owner).unwrap();
        assert!(control.take_failure().is_none());

        control.failed(&owner, interrupted_transfer());
        assert!(restarter.transfer_interrupted());
        control.restart(&owner).unwrap();
        control.next();
        assert!(restarter.transfer_interrupted());
        let failure = control.take_failure().unwrap();
        assert!(failure.interrupted);
        assert!(failure.message.english.contains("USB error"));
        assert!(!restarter.transfer_interrupted());
    }

    #[test]
    fn transfer_recovery_backoff_is_bounded_and_resets_after_a_healthy_period() {
        let start = Instant::now();
        let mut recovery = TransferRecovery::default();
        assert!(!recovery.retry_now(start));
        let mut now = start;
        for (attempt, seconds) in (1..).zip([1, 2, 4, 8, 16]) {
            let delay = Duration::from_secs(seconds);
            assert_eq!(recovery.schedule(now), Some((attempt, delay)));
            assert_eq!(recovery.schedule(now), Some((attempt, delay)));
            assert!(!recovery.take_due(now + delay - Duration::from_millis(1)));
            assert!(recovery.take_due(now + delay));
            assert!(!recovery.take_due(now + delay));
            recovery = std::mem::take(&mut recovery).for_replacement();
            now += delay + Duration::from_secs(1);
        }
        assert_eq!(recovery.schedule(now), None);
        assert!(!recovery.take_due(now + Duration::from_secs(3600)));

        let mut replacement = std::mem::take(&mut recovery).for_replacement();
        assert_eq!(replacement.schedule(now + Duration::from_secs(1)), None);
        let healthy = now + TRANSFER_RECOVERY_HEALTHY_RESET + Duration::from_secs(2);
        assert_eq!(
            replacement.schedule(healthy),
            Some((1, Duration::from_secs(1)))
        );
    }

    #[test]
    fn transfer_recovery_outlives_a_brief_disappearance_but_not_a_healthy_period() {
        let now = Instant::now();
        let mut recovery = TransferRecovery::default();
        assert!(recovery.expired(now));
        recovery.schedule(now);
        assert!(!recovery.expired(now + Duration::from_secs(5)));
        assert!(recovery.expired(now + TRANSFER_RECOVERY_HEALTHY_RESET + Duration::from_secs(1)));
    }

    #[test]
    fn interrupted_stream_recreates_the_lcd_after_backoff_or_on_manual_retry() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(1);
        let asset = Arc::new(MediaAsset {
            kind: MediaAssetKind::Static {
                frame: lianli_media::Retained::frame(vec![1]).unwrap(),
            },
            config_key: "interrupted".into(),
            stream_fps: 30.0,
            hardware_video: false,
        });
        let mut target = ActiveTarget::new(
            0,
            "us88".into(),
            LcdBackend::WinUsb(ThreadedWinUsbSender {
                shares_cooling: false,
                transport: None,
                tx,
                stream_control: Arc::new(StreamControl::default()),
                frame_delivery: Arc::new(FrameDelivery::default()),
                closing: Arc::new(AtomicBool::new(false)),
                thread: None,
            }),
            asset,
            ScreenInfo::TLLCD,
            true,
            None,
        );
        struct LiveStream;
        impl FrameSource for LiveStream {
            fn is_autonomous(&self) -> bool {
                true
            }
        }
        let fail_stream = |target: &mut ActiveTarget| {
            target.media = Box::new(LiveStream);
            target.media_pending = false;
            target.media_stage = lianli_shared::ipc::MediaRuntimeStage::AutonomousSourceConfigured;
            let LcdBackend::WinUsb(sender) = &target.lcd else {
                unreachable!()
            };
            let owner = sender.stream_control.next();
            sender.stream_control.failed(&owner, interrupted_transfer());
            target.send_frame(None, &mut PacketBuilder::new())
        };

        let failed_at = Instant::now();
        assert!(matches!(
            fail_stream(&mut target),
            Err(SendError::Interrupted(_))
        ));
        assert_eq!(
            target.media_status().stage,
            lianli_shared::ipc::MediaRuntimeStage::Failed
        );
        assert!(target.due_transfer_recovery(failed_at).is_none());
        let Some(DaemonEvent::RemoveFailedLcd { removal, .. }) =
            target.due_transfer_recovery(Instant::now() + Duration::from_secs(1))
        else {
            panic!("interrupted transfer must recreate the LCD")
        };
        assert!(target.matches_removal(&removal));
        assert!(target
            .due_transfer_recovery(Instant::now() + Duration::from_secs(60))
            .is_none());

        let mut replacement = ActiveTarget::new(
            0,
            "us88".into(),
            LcdBackend::WinUsb(ThreadedWinUsbSender {
                shares_cooling: false,
                transport: None,
                tx: std::sync::mpsc::sync_channel(1).0,
                stream_control: Arc::new(StreamControl::default()),
                frame_delivery: Arc::new(FrameDelivery::default()),
                closing: Arc::new(AtomicBool::new(false)),
                thread: None,
            }),
            target.asset.clone(),
            ScreenInfo::TLLCD,
            true,
            None,
        );
        replacement.retry_interrupted_transfer();
        assert!(replacement.due_transfer_recovery(Instant::now()).is_none());
        assert!(replacement.take_transfer_recovery().is_none());
        replacement.restore_transfer_recovery(target.take_transfer_recovery().unwrap());
        assert!(matches!(
            fail_stream(&mut replacement),
            Err(SendError::Interrupted(_))
        ));

        replacement.retry_failed_source();
        assert_eq!(
            replacement.media_status().stage,
            lianli_shared::ipc::MediaRuntimeStage::Failed
        );
        assert!(replacement
            .due_transfer_recovery(Instant::now() + Duration::from_millis(1500))
            .is_none());
        assert_eq!(replacement.transfer_recovery.attempts, 2);

        replacement.retry_interrupted_transfer();
        assert_eq!(replacement.transfer_recovery.attempts, 0);
        assert!(matches!(
            replacement.due_transfer_recovery(Instant::now()),
            Some(DaemonEvent::RemoveFailedLcd { .. })
        ));
    }

    #[test]
    fn transfer_success_cannot_leak_into_a_replacement_or_cancelled_stream() {
        let control = StreamControl::default();
        assert_eq!(control.transferred(), None);
        let old = control.next();
        let old_observer = control.transfer_observer(&old).unwrap();
        assert_eq!(control.transferred(), Some(false));
        old_observer.store(true, Ordering::Release);
        assert_eq!(control.transferred(), Some(true));
        let current = control.next();
        old_observer.store(true, Ordering::Release);
        assert_eq!(control.transferred(), Some(false));
        assert!(control.transfer_observer(&old).is_none());
        let current_observer = control.transfer_observer(&current).unwrap();
        current_observer.store(true, Ordering::Release);
        let restarted = control.restart(&current).unwrap();
        assert_eq!(control.transferred(), Some(false));
        current_observer.store(true, Ordering::Release);
        assert_eq!(control.transferred(), Some(false));
        let observer = control.transfer_observer(&restarted).unwrap();
        control.cancel();
        observer.store(true, Ordering::Release);
        assert_eq!(control.transferred(), None);
        assert!(control.transfer_observer(&restarted).is_none());
    }

    #[test]
    fn prepared_source_rejects_replaced_selections_and_recreated_targets() {
        let asset = Arc::new(MediaAsset {
            kind: MediaAssetKind::Static {
                frame: lianli_media::Retained::frame(vec![1]).unwrap(),
            },
            config_key: "same-key".into(),
            stream_fps: 30.0,
            hardware_video: false,
        });
        let make_target = || {
            let (tx, _) = std::sync::mpsc::sync_channel(1);
            ActiveTarget::new(
                0,
                "same-device".into(),
                LcdBackend::WinUsb(ThreadedWinUsbSender {
                    shares_cooling: false,
                    transport: None,
                    tx,
                    stream_control: Arc::new(StreamControl::default()),
                    frame_delivery: Arc::new(FrameDelivery::default()),
                    closing: Arc::new(AtomicBool::new(false)),
                    thread: None,
                }),
                asset.clone(),
                ScreenInfo::TLLCD,
                false,
                None,
            )
        };
        let mut target = make_target();
        let result = SourceResult {
            index: 0,
            selection: Arc::downgrade(&target.source_selection),
            result: Err("fixture failure".into()).into(),
        };
        assert!(target.accepts_source(&result));
        target.swap_media(asset.clone(), false, None);
        assert!(!target.accepts_source(&result));
        let current = SourceResult {
            index: 0,
            selection: Arc::downgrade(&target.source_selection),
            result: Err("fixture failure".into()).into(),
        };
        assert!(target.accepts_source(&current));
        assert!(!make_target().accepts_source(&current));
        target.pending_brightness = Some(50);
        assert!(!target.accepts_source(&current));
    }

    #[test]
    fn full_winusb_queue_defers_frames_without_advancing_submission_state() {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        tx.send(LcdThreadMsg::SetBrightness(40)).unwrap();
        let sender = ThreadedWinUsbSender {
            shares_cooling: false,
            transport: None,
            tx,
            stream_control: Arc::new(StreamControl::default()),
            frame_delivery: Arc::new(FrameDelivery::default()),
            closing: Arc::new(AtomicBool::new(false)),
            thread: None,
        };
        let asset = Arc::new(MediaAsset {
            kind: MediaAssetKind::Static {
                frame: lianli_media::Retained::frame(vec![1, 2, 3]).unwrap(),
            },
            config_key: "queue-test".into(),
            stream_fps: 20.0,
            hardware_video: false,
        });
        let mut target = ActiveTarget::new(
            0,
            "test".into(),
            LcdBackend::WinUsb(sender),
            asset,
            ScreenInfo::TLLCD,
            false,
            None,
        );
        struct PendingFrame(bool);
        impl FrameSource for PendingFrame {
            fn next_frame(&mut self) -> Option<&[u8]> {
                (!self.0).then_some(&[1, 2, 3])
            }
            fn mark_sent(&mut self) {
                self.0 = true;
            }
        }
        target.media = Box::new(PendingFrame(false));
        target.media_pending = false;
        assert!(matches!(
            target.send_frame(None, &mut PacketBuilder::new()),
            Ok(false)
        ));
        assert_eq!(target.frame_counter, 0);
        assert_eq!(
            target.media_status().stage,
            lianli_shared::ipc::MediaRuntimeStage::StartingSource
        );
        assert!(matches!(
            rx.recv().unwrap(),
            LcdThreadMsg::SetBrightness(40)
        ));
        assert!(matches!(
            target.send_frame(None, &mut PacketBuilder::new()),
            Ok(true)
        ));
        assert_eq!(target.frame_counter, 1);
        let LcdThreadMsg::Frame(bytes, delivery) = rx.recv().unwrap() else {
            panic!("expected queued JPEG")
        };
        assert_eq!(bytes, [1, 2, 3]);
        assert!(matches!(
            target.send_frame(None, &mut PacketBuilder::new()),
            Ok(false)
        ));
        delivery.submit(|| Err(anyhow::anyhow!("USB transfer timed out")));
        assert!(matches!(target.send_frame(None, &mut PacketBuilder::new()),
            Err(SendError::Stopped(message)) if message.english.contains("USB transfer timed out")));
        assert_eq!(
            target.media_status().stage,
            lianli_shared::ipc::MediaRuntimeStage::Failed
        );
        assert!(matches!(
            target.send_frame(None, &mut PacketBuilder::new()),
            Ok(false)
        ));
        delivery.submit(|| panic!("failed source must not send queued frames"));
        target.retry_failed_source();
        let LcdBackend::WinUsb(sender) = &target.lcd else {
            unreachable!()
        };
        assert!(sender.frame_delivery.failure.lock().is_none());
        assert!(delivery.retired.load(Ordering::Acquire));
    }

    #[test]
    fn retired_jpeg_transfers_cannot_fail_replacement_media() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(2);
        let mut sender = ThreadedWinUsbSender {
            shares_cooling: false,
            transport: None,
            tx,
            stream_control: Arc::new(StreamControl::default()),
            frame_delivery: Arc::new(FrameDelivery::default()),
            closing: Arc::new(AtomicBool::new(false)),
            thread: None,
        };
        let old = sender.frame_delivery.clone();
        let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
        let (finish_tx, finish_rx) = std::sync::mpsc::sync_channel(1);
        let worker_state = old.clone();
        let worker = thread::spawn(move || {
            worker_state.submit(|| {
                started_tx.send(()).unwrap();
                finish_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                Err(anyhow::anyhow!("late failure"))
            })
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        sender.reset_frame_delivery();
        finish_tx.send(()).unwrap();
        worker.join().unwrap();
        assert_eq!(
            old.failure
                .lock()
                .as_ref()
                .map(|failure| failure.message.english.as_str()),
            Some("late failure")
        );
        assert!(sender.frame_delivery.failure.lock().is_none());
        old.submit(|| panic!("retired JPEG must not touch the device"));
        sender.frame_delivery.submit(|| Ok(()));
        assert!(sender.frame_delivery.failure.lock().is_none());
        sender
            .frame_delivery
            .submit(|| Err(anyhow::anyhow!("x".repeat(4096))));
        assert_eq!(
            sender
                .frame_delivery
                .failure
                .lock()
                .as_ref()
                .unwrap()
                .message
                .english
                .len(),
            2048
        );
    }

    #[test]
    fn rejected_stream_submission_preserves_current_owner_and_transfer_status() {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let sender = ThreadedWinUsbSender {
            shares_cooling: false,
            transport: None,
            tx,
            stream_control: Arc::new(StreamControl::default()),
            frame_delivery: Arc::new(FrameDelivery::default()),
            closing: Arc::new(AtomicBool::new(false)),
            thread: None,
        };
        sender.stream_h264("current".into(), true, 20.0).unwrap();
        let current = sender.stream_control.current.lock().clone();
        let observer = sender.stream_control.transfer_observer(&current).unwrap();
        observer.store(true, Ordering::Release);
        sender
            .stream_control
            .failed(&current, "existing failure".into());
        assert!(sender.stream_h264("rejected".into(), true, 20.0).is_err());
        assert!(!current.load(Ordering::Acquire));
        assert_eq!(sender.stream_control.transferred(), Some(true));
        assert_eq!(
            sender
                .stream_control
                .failure
                .lock()
                .as_ref()
                .map(|failure| failure.message.english.as_str()),
            Some("existing failure")
        );
        assert!(sender
            .stream_control
            .submit(Some(&current), |_| anyhow::bail!("rejected restart"))
            .is_err());
        assert!(Arc::ptr_eq(&sender.stream_control.current.lock(), &current));
        drop(rx);
        assert!(sender
            .stream_h264("disconnected".into(), true, 20.0)
            .is_err());
        assert!(!current.load(Ordering::Acquire));
        assert_eq!(sender.stream_control.transferred(), Some(true));
    }

    #[test]
    fn replacing_queued_stream_keeps_old_cancellation_and_brightness_order() {
        let (tx, rx) = std::sync::mpsc::sync_channel(3);
        let sender = ThreadedWinUsbSender {
            shares_cooling: false,
            transport: None,
            tx,
            stream_control: Arc::new(StreamControl::default()),
            frame_delivery: Arc::new(FrameDelivery::default()),
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
            shares_cooling: false,
            transport: None,
            tx,
            stream_control: Arc::new(StreamControl::default()),
            frame_delivery: Arc::new(FrameDelivery::default()),
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
            hardware_video: false,
        });
        let mut target = ActiveTarget::new(
            0,
            "test".into(),
            LcdBackend::WinUsb(sender),
            asset.clone(),
            screen,
            true,
            None,
        );
        target.wait_for_initialization();
        assert!(target.source_request().is_none());
        let premature = SourceResult {
            index: 0,
            selection: Arc::downgrade(&target.source_selection),
            result: Err("must not install while initializing".into()).into(),
        };
        assert!(!target.accepts_source(&premature));
        target.install_source(premature);
        assert!(!target.source_failed);
        target.finish_initialization(None);
        assert!(target.source_request().is_some());
        assert!(rx.try_recv().is_err());
        target.apply_brightness(None, &mut PacketBuilder::new(), 100);
        assert!(matches!(
            rx.try_recv().unwrap(),
            LcdThreadMsg::SetBrightness(100)
        ));
        target.swap_media(asset, true, None);
        let status = target.media_status();
        assert_eq!(
            status.stage,
            lianli_shared::ipc::MediaRuntimeStage::StartingSource
        );
        assert_eq!(status.fps_limit, 20.0);
        assert!(!status.hardware_video_allowed);
        assert!(status.fallback_reason.is_none());
        struct RetainedSource(Arc<AtomicUsize>, Arc<AtomicBool>, Arc<AtomicBool>);
        impl FrameSource for RetainedSource {
            fn request_stop(&mut self) {
                self.1.store(true, Ordering::Release);
            }
            fn retirement_complete(&self) -> bool {
                self.2.load(Ordering::Acquire)
            }
            fn is_autonomous(&self) -> bool {
                true
            }
        }
        impl Drop for RetainedSource {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        let retired = Arc::new(AtomicUsize::new(0));
        let stopping = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        target.media = Box::new(RetainedSource(
            retired.clone(),
            stopping.clone(),
            finished.clone(),
        ));
        let replacement = Arc::new(MediaAsset {
            kind: target.asset.kind.clone(),
            config_key: "replacement".into(),
            stream_fps: 15.0,
            hardware_video: true,
        });
        target.swap_media(replacement, true, None);
        assert_eq!(retired.load(Ordering::Relaxed), 0);
        assert!(target.source_request().is_some());
        assert!(matches!(
            target.send_frame(None, &mut PacketBuilder::new()),
            Ok(true)
        ));
        assert_eq!(target.media_status().fps_limit, 20.0);
        assert!(!target.media_status().hardware_video_allowed);
        assert_eq!(target.playback_asset.config_key, "sensor-test");
        assert!(target.playback_failure_key().is_none());
        assert_eq!(retired.load(Ordering::Relaxed), 0);
        target.install_source(SourceResult {
            index: target.index,
            selection: Arc::downgrade(&target.source_selection),
            result: Err("Initial JPEG rendering failed".into()).into(),
        });
        assert_eq!(
            target.media_status().stage,
            lianli_shared::ipc::MediaRuntimeStage::Failed
        );
        assert!(!target.media_pending);
        assert_eq!(retired.load(Ordering::Relaxed), 0);
        assert!(matches!(
            target.send_frame(None, &mut PacketBuilder::new()),
            Ok(true)
        ));
        assert_eq!(
            target.media_status().stage,
            lianli_shared::ipc::MediaRuntimeStage::Failed
        );
        target.retry_failed_source();
        assert!(target.source_request().is_some());
        assert_eq!(retired.load(Ordering::Relaxed), 0);
        target.finish_source_install(Err(anyhow::anyhow!("sender queue full")), None);
        assert_eq!(retired.load(Ordering::Relaxed), 0);
        assert_eq!(target.media_status().fps_limit, 20.0);
        assert!(target
            .media_status()
            .fallback_reason
            .unwrap()
            .contains("sender queue full"));
        assert!(matches!(
            target.send_frame(None, &mut PacketBuilder::new()),
            Ok(true)
        ));
        target.retry_failed_source();
        let accepted = match &target.lcd {
            LcdBackend::WinUsb(sender) => sender.stream_control.next(),
            _ => unreachable!(),
        };
        target.finish_source_install(
            Ok(Box::new(RetainedSource(
                retired.clone(),
                Arc::new(AtomicBool::new(false)),
                finished.clone(),
            ))),
            None,
        );
        assert!(!accepted.load(Ordering::Acquire));
        assert!(stopping.load(Ordering::Acquire));
        assert_eq!(retired.load(Ordering::Relaxed), 0);
        assert!(target.take_finished_retirement().is_none());
        target.swap_media(target.asset.clone(), true, None);
        assert!(target.source_request().is_none());
        assert!(matches!(
            target.send_frame(None, &mut PacketBuilder::new()),
            Ok(true)
        ));
        finished.store(true, Ordering::Release);
        let completed = target.take_finished_retirement().unwrap();
        assert_eq!(retired.load(Ordering::Relaxed), 0);
        assert!(target.source_request().is_some());
        drop(completed);
        assert_eq!(retired.load(Ordering::Relaxed), 1);
        assert_eq!(target.media_status().fps_limit, 15.0);
        assert!(target.media_status().hardware_video_allowed);
        assert_eq!(target.playback_asset.config_key, "replacement");
        target.media_pending = false;
        assert_eq!(
            target.playback_failure_key().as_deref(),
            Some("replacement")
        );
        struct StoppedSource(bool);
        impl FrameSource for StoppedSource {
            fn is_autonomous(&self) -> bool {
                self.0
            }
            fn has_exited(&self) -> bool {
                true
            }
        }
        target.media = Box::new(StoppedSource(true));
        target.media_pending = false;
        assert!(matches!(
            target.send_frame(None, &mut PacketBuilder::new()),
            Err(SendError::Stopped(_))
        ));
        assert!(matches!(
            target.send_frame(None, &mut PacketBuilder::new()),
            Ok(false)
        ));
        assert_eq!(
            target.media_status().stage,
            lianli_shared::ipc::MediaRuntimeStage::Failed
        );
        target.retry_failed_source();
        assert!(target.media_pending);
        target.media = Box::new(StoppedSource(false));
        target.media_paused = false;
        target.media_pending = false;
        assert!(matches!(
            target.send_frame(None, &mut PacketBuilder::new()),
            Err(SendError::Stopped(message)) if message.english.contains("JPEG rendering failed")
        ));
        assert!(matches!(
            target.send_frame(None, &mut PacketBuilder::new()),
            Ok(false)
        ));
        target.retry_failed_source();
        assert!(target.media_pending);
        assert_eq!(
            target.media_status().stage,
            lianli_shared::ipc::MediaRuntimeStage::StartingSource
        );
        assert!(rx.try_recv().is_err());
        let brightness_drops = Arc::new(AtomicUsize::new(0));
        let brightness_stop = Arc::new(AtomicBool::new(false));
        target.media = Box::new(RetainedSource(
            brightness_drops.clone(),
            brightness_stop.clone(),
            Arc::new(AtomicBool::new(true)),
        ));
        target.apply_brightness(None, &mut PacketBuilder::new(), 37);
        assert!(matches!(
            rx.try_recv().unwrap(),
            LcdThreadMsg::SetBrightness(37)
        ));
        assert!(brightness_stop.load(Ordering::Acquire));
        assert_eq!(brightness_drops.load(Ordering::Relaxed), 0);
        assert!(target.source_request().is_some());
        assert!(matches!(
            target.send_frame(None, &mut PacketBuilder::new()),
            Ok(false)
        ));
        assert!(target.playback_failure_key().is_none());
        target.finish_source_install(
            Ok(Box::new(RetainedSource(
                brightness_drops.clone(),
                Arc::new(AtomicBool::new(false)),
                Arc::new(AtomicBool::new(true)),
            ))),
            None,
        );
        assert!(!target.media_paused);
        assert_eq!(brightness_drops.load(Ordering::Relaxed), 0);
        drop(target.take_finished_retirement().unwrap());
        assert_eq!(brightness_drops.load(Ordering::Relaxed), 1);
        if let LcdBackend::WinUsb(sender) = &target.lcd {
            let owner = sender.stream_control.next();
            sender
                .stream_control
                .failed(&owner, "transfer stopped".into());
        }
        assert!(matches!(
            target.send_frame(None, &mut PacketBuilder::new()),
            Err(SendError::Stopped(error)) if error.english == "transfer stopped"
        ));
        assert_eq!(brightness_drops.load(Ordering::Relaxed), 1);
        target.retry_failed_source();
        assert!(target.source_request().is_some());
        assert!(matches!(
            target.send_frame(None, &mut PacketBuilder::new()),
            Ok(false)
        ));
        let stale = Arc::downgrade(&Arc::new(()));
        assert!(!target.matches_removal(&stale));
        let Some(DaemonEvent::RemoveFailedLcd {
            removal,
            key,
            error,
            ..
        }) = target.removal_event("USB unavailable".into())
        else {
            panic!("removal event missing")
        };
        assert!(target.matches_removal(&removal));
        assert!(!target.matches_removal(&stale));
        assert_eq!(key, "replacement");
        assert_eq!(error.english, "USB unavailable");
        assert!(target.removal_event("duplicate".into()).is_none());
        target.swap_media(target.asset.clone(), true, None);
        target.retry_failed_source();
        assert!(target.matches_removal(&removal));
        assert!(target.source_request().is_none());
        assert!(matches!(
            target.send_frame(None, &mut PacketBuilder::new()),
            Ok(false)
        ));
        target.apply_brightness(None, &mut PacketBuilder::new(), 80);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn file_retirement_waits_for_worker_completion_after_stop() {
        let (release, released) = std::sync::mpsc::channel();
        let mut source = H264FileSource::new(PathBuf::new(), true, 30.0);
        source.hid_thread = Some(thread::spawn(move || {
            released.recv_timeout(Duration::from_secs(2)).unwrap();
        }));
        source.request_stop();
        assert!(source.hid_stop.load(Ordering::Acquire));
        assert!(!source.retirement_complete());
        release.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !source.retirement_complete() && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert!(source.retirement_complete());
        source.hid_thread.take().unwrap().join().unwrap();
    }

    #[test]
    fn shutdown_signals_both_sources_and_observes_completion_with_one_deadline() {
        struct Source {
            stopped: bool,
            finished: bool,
        }
        impl FrameSource for Source {
            fn request_stop(&mut self) {
                self.stopped = true;
            }
            fn retirement_complete(&self) -> bool {
                self.finished
            }
        }
        let mut current = Source {
            stopped: false,
            finished: false,
        };
        let mut retired = Source {
            stopped: false,
            finished: false,
        };
        assert!(!stop_frame_sources(
            &mut current,
            Some(&mut retired),
            Duration::ZERO
        ));
        assert!(current.stopped && retired.stopped);
        current.finished = true;
        assert!(!stop_frame_sources(
            &mut current,
            Some(&mut retired),
            Duration::ZERO
        ));
        retired.finished = true;
        assert!(stop_frame_sources(
            &mut current,
            Some(&mut retired),
            Duration::ZERO
        ));
        assert!(stop_frame_sources(&mut current, None, Duration::ZERO));
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
            failure: Mutex::new(None),
            transferred: Mutex::new(None),
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
            shares_cooling: false,
            transport: None,
            tx,
            stream_control,
            frame_delivery: Arc::new(FrameDelivery::default()),
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
        brightness_failures: AtomicUsize,
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
            let failure = self.brightness_failures.try_update(
                Ordering::Relaxed,
                Ordering::Relaxed,
                |remaining| remaining.checked_sub(1),
            );
            anyhow::ensure!(failure.is_err(), "injected brightness write failure");
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

    fn brightness_target() -> (ActiveTarget, SharedHidLcd, Arc<AtomicUsize>) {
        brightness_target_with_failures(0)
    }

    fn brightness_target_with_failures(
        failures: usize,
    ) -> (ActiveTarget, SharedHidLcd, Arc<AtomicUsize>) {
        let brightness = Arc::new(AtomicUsize::new(80));
        let device = Arc::new(HidLcd::new(Box::new(TestLcd {
            brightness: brightness.clone(),
            brightness_failures: AtomicUsize::new(failures),
            sends: Arc::new(AtomicUsize::new(0)),
            fail_on: 0,
            fail_count: 0,
        })));
        let asset = Arc::new(MediaAsset {
            kind: MediaAssetKind::Static {
                frame: lianli_media::Retained::frame(vec![1]).unwrap(),
            },
            config_key: "brightness-test".into(),
            stream_fps: 24.0,
            hardware_video: false,
        });
        let target = ActiveTarget::new(
            0,
            "lcd".into(),
            LcdBackend::HidLcd(device.clone()),
            asset,
            ScreenInfo::AIO_LCD_480,
            false,
            None,
        );
        (target, device, brightness)
    }

    #[test]
    fn brightness_waits_for_an_in_flight_frame() {
        let (mut target, device, brightness) = brightness_target();
        let (held_tx, held_rx) = std::sync::mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let _guard = device.lock();
            held_tx.send(()).unwrap();
            thread::sleep(Duration::from_millis(20));
        });
        held_rx.recv().unwrap();
        assert!(target
            .request_brightness(None, &mut PacketBuilder::new(), 30, None)
            .unwrap());
        assert_eq!(brightness.load(Ordering::Relaxed), 30);
        assert!(target.pending_brightness.is_none());
        worker.join().unwrap();
    }

    #[test]
    fn brightness_write_failure_reaches_requester_and_recovery_clears_it() {
        let (mut target, _, brightness) = brightness_target_with_failures(1);
        let mut builder = PacketBuilder::new();
        assert!(target
            .request_brightness(None, &mut builder, 30, Some("failed-request".into()))
            .unwrap_err()
            .contains("injected brightness write failure"));
        let status = target.brightness_status().unwrap();
        assert_eq!(status.brightness, 30);
        assert!(status.pending);
        assert_eq!(status.request_id.as_deref(), Some("failed-request"));
        assert!(status.error.is_some());
        assert_eq!(brightness.load(Ordering::Relaxed), 80);
        thread::sleep(BRIGHTNESS_WRITE_INTERVAL);
        target.flush_pending_brightness(None, &mut builder);
        let status = target.brightness_status().unwrap();
        assert!(!status.pending);
        assert!(status.error.is_none());
        assert_eq!(status.request_id.as_deref(), Some("failed-request"));
        assert_eq!(brightness.load(Ordering::Relaxed), 30);
    }

    #[test]
    fn deferred_brightness_exhaustion_remains_visible_until_a_new_request() {
        let (mut target, device, brightness) = brightness_target_with_failures(3);
        let mut builder = PacketBuilder::new();
        let guard = device.lock();
        assert!(!target
            .request_brightness(None, &mut builder, 30, Some("deferred-request".into()))
            .unwrap());
        drop(guard);
        for _ in 0..3 {
            thread::sleep(BRIGHTNESS_WRITE_INTERVAL);
            target.flush_pending_brightness(None, &mut builder);
        }
        let status = target.brightness_status().unwrap();
        assert!(!status.pending);
        assert_eq!(status.request_id.as_deref(), Some("deferred-request"));
        assert!(status
            .error
            .as_ref()
            .unwrap()
            .contains("injected brightness write failure"));
        assert_eq!(brightness.load(Ordering::Relaxed), 80);
        thread::sleep(BRIGHTNESS_WRITE_INTERVAL);
        target.flush_pending_brightness(None, &mut builder);
        assert!(target.brightness_status().unwrap().error.is_some());
        assert!(target
            .request_brightness(None, &mut builder, 60, Some("retry-request".into()))
            .unwrap());
        assert_eq!(brightness.load(Ordering::Relaxed), 60);
        assert!(target.brightness_status().unwrap().error.is_none());
        assert_eq!(
            target.brightness_status().unwrap().request_id.as_deref(),
            Some("retry-request")
        );
    }

    #[test]
    fn initialization_failure_ends_deferred_brightness_and_rejects_new_requests() {
        let (mut target, _, brightness) = brightness_target();
        let mut builder = PacketBuilder::new();
        target.wait_for_initialization();
        assert!(!target
            .request_brightness(None, &mut builder, 30, None)
            .unwrap());
        target.finish_initialization(Some("device disconnected"));
        target.flush_pending_brightness(None, &mut builder);
        let status = target.brightness_status().unwrap();
        assert!(!status.pending);
        assert!(status
            .error
            .as_ref()
            .unwrap()
            .contains("device disconnected"));
        assert!(target
            .request_brightness(None, &mut builder, 60, None)
            .is_err());
        assert_eq!(brightness.load(Ordering::Relaxed), 80);
    }

    #[test]
    fn removal_ends_pending_brightness_with_an_error() {
        let (mut target, device, brightness) = brightness_target();
        let guard = device.lock();
        assert!(!target
            .request_brightness(None, &mut PacketBuilder::new(), 30, None)
            .unwrap());
        drop(guard);
        target.request_removal();
        let status = target.brightness_status().unwrap();
        assert!(!status.pending);
        assert!(status.error.as_ref().unwrap().contains("removed"));
        assert_eq!(brightness.load(Ordering::Relaxed), 80);
    }

    #[test]
    fn brightness_reports_deferred_and_rejects_removed_targets() {
        let (mut target, device, brightness) = brightness_target();
        let guard = device.lock();
        assert!(!target
            .request_brightness(None, &mut PacketBuilder::new(), 30, None)
            .unwrap());
        assert_eq!(brightness.load(Ordering::Relaxed), 80);
        drop(guard);
        thread::sleep(BRIGHTNESS_WRITE_INTERVAL);
        target.flush_pending_brightness(None, &mut PacketBuilder::new());
        assert_eq!(brightness.load(Ordering::Relaxed), 30);
        target.request_removal();
        assert!(target
            .request_brightness(None, &mut PacketBuilder::new(), 80, None)
            .is_err());
        assert_eq!(brightness.load(Ordering::Relaxed), 30);
    }

    #[test]
    fn brightness_bursts_keep_the_latest_value_and_leave_time_between_writes() {
        let (mut target, _, brightness) = brightness_target();
        let mut builder = PacketBuilder::new();
        assert!(target
            .request_brightness(None, &mut builder, 10, None)
            .unwrap());
        for value in 11..=100 {
            assert!(!target
                .request_brightness(None, &mut builder, value, None)
                .unwrap());
        }
        target.flush_pending_brightness(None, &mut builder);
        assert_eq!(brightness.load(Ordering::Relaxed), 10);
        assert_eq!(target.pending_brightness, Some(100));
        thread::sleep(BRIGHTNESS_WRITE_INTERVAL);
        target.flush_pending_brightness(None, &mut builder);
        assert_eq!(brightness.load(Ordering::Relaxed), 100);
        assert!(target.pending_brightness.is_none());
    }

    #[test]
    fn pending_aio_defers_media_and_brightness_without_blocking_ready_targets() {
        let brightness = Arc::new(AtomicUsize::new(75));
        let device = Arc::new(HidLcd::new(Box::new(TestLcd {
            brightness: brightness.clone(),
            brightness_failures: AtomicUsize::new(0),
            sends: Arc::new(AtomicUsize::new(0)),
            fail_on: 0,
            fail_count: 0,
        })));
        let asset = Arc::new(MediaAsset {
            kind: MediaAssetKind::Static {
                frame: lianli_media::Retained::frame(vec![1]).unwrap(),
            },
            config_key: "init-test".into(),
            stream_fps: 20.0,
            hardware_video: false,
        });
        let mut pending = ActiveTarget::new(
            0,
            "pending".into(),
            LcdBackend::HidLcd(device.clone()),
            asset.clone(),
            ScreenInfo::AIO_LCD_480,
            false,
            None,
        );
        let mut ready = ActiveTarget::new(
            1,
            "ready".into(),
            LcdBackend::HidLcd(lcd(0).0),
            asset,
            ScreenInfo::AIO_LCD_480,
            false,
            None,
        );
        pending.wait_for_initialization();
        let guard = device.lock();
        let mut builder = PacketBuilder::new();
        pending.apply_brightness(None, &mut builder, 42);
        pending.maybe_start_recovery(None, Duration::ZERO);
        assert!(matches!(pending.send_frame(None, &mut builder), Ok(false)));
        assert!(pending.media_pending);
        assert!(pending.recovery_thread.is_none());
        assert_eq!(pending.pending_brightness, Some(42));
        assert_eq!(brightness.load(Ordering::Relaxed), 75);
        assert!(matches!(ready.send_frame(None, &mut builder), Ok(true)));
        assert_eq!(ready.frame_counter, 1);
        drop(guard);
        pending.finish_initialization(None);
        assert!(matches!(pending.send_frame(None, &mut builder), Ok(false)));
        pending.flush_pending_brightness(None, &mut builder);
        assert_eq!(brightness.load(Ordering::Relaxed), 42);
        assert!(matches!(pending.send_frame(None, &mut builder), Ok(true)));
        assert_eq!(pending.frame_counter, 1);
    }

    #[test]
    fn failed_initialization_cannot_be_bypassed_by_media_retry_or_replacement() {
        let asset = Arc::new(MediaAsset {
            kind: MediaAssetKind::Static {
                frame: lianli_media::Retained::frame(vec![1]).unwrap(),
            },
            config_key: "init-failed".into(),
            stream_fps: 20.0,
            hardware_video: false,
        });
        let mut target = ActiveTarget::new(
            0,
            "failed".into(),
            LcdBackend::HidLcd(lcd(0).0),
            asset.clone(),
            ScreenInfo::AIO_LCD_480,
            false,
            None,
        );
        target.wait_for_initialization();
        target.finish_initialization(Some("device disconnected"));
        target.retry_failed_source();
        target.swap_media(asset, true, None);
        assert_eq!(
            target.media_status().stage,
            lianli_shared::ipc::MediaRuntimeStage::Failed
        );
        assert!(matches!(
            target.send_frame(None, &mut PacketBuilder::new()),
            Ok(false)
        ));
        assert!(target
            .media_status()
            .fallback_reason
            .unwrap()
            .contains("device disconnected"));
    }

    #[test]
    fn h264_startup_lock_wait_can_expire_or_cancel_without_transferring() {
        let (lcd, sends) = lcd(0);
        let guard = lcd.lock();
        assert!(
            hid_stream_frame_interval(&lcd, 20.0, &|| false, Duration::from_millis(75)).is_none()
        );
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker_lcd = lcd.clone();
        let (entered, waiting) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            hid_stream_frame_interval(
                &worker_lcd,
                20.0,
                &|| {
                    entered.send(()).unwrap();
                    worker_stop.load(Ordering::Relaxed)
                },
                Duration::from_secs(3),
            )
        });
        waiting.recv_timeout(Duration::from_secs(1)).unwrap();
        stop.store(true, Ordering::Relaxed);
        assert!(worker.join().unwrap().is_none());
        drop(guard);
        assert_eq!(sends.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn file_playback_survives_startup_lock_contention() {
        let path = std::env::temp_dir().join(format!(
            "lianli-startup-contention-{}.h264",
            std::process::id()
        ));
        std::fs::write(&path, [0, 0, 0, 1, 5, 128]).unwrap();
        let (lcd, sends) = lcd(0);
        let guard = lcd.lock();
        let mut source = H264FileSource::new(path.clone(), false, 20.0);
        source.start(&LcdBackend::HidLcd(lcd.clone())).unwrap();
        thread::sleep(Duration::from_millis(250));
        assert!(!source.has_exited());
        assert_eq!(sends.load(Ordering::Relaxed), 0);
        drop(guard);
        wait_for_file_worker(&source);
        assert!(!source.has_exited());
        assert_eq!(source.transferred(), Some(true));
        assert_eq!(sends.load(Ordering::Relaxed), 1);
        assert!(lcd.recovery_idle().is_some());
        drop(source);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn init_completion_cannot_match_a_replacement_attachment() {
        let (original, _) = lcd(usize::MAX);
        let completion = original.attachment();
        let same_attachment = Arc::clone(&original);
        assert!(same_attachment.matches_attachment(&completion));
        let (replacement, _) = lcd(usize::MAX);
        assert!(!replacement.matches_attachment(&completion));
        drop(original);
        drop(same_attachment);
        assert!(completion.upgrade().is_none());
        assert!(!replacement.matches_attachment(&completion));
    }

    fn lcd_with_failures(fail_on: usize, fail_count: usize) -> (SharedHidLcd, Arc<AtomicUsize>) {
        let sends = Arc::new(AtomicUsize::new(0));
        (
            Arc::new(HidLcd::new(Box::new(TestLcd {
                brightness: Arc::new(AtomicUsize::new(100)),
                brightness_failures: AtomicUsize::new(0),
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
                brightness_failures: AtomicUsize::new(0),
                sends: Arc::new(AtomicUsize::new(0)),
                fail_on: 0,
                fail_count: 0,
            })));
            let asset = Arc::new(MediaAsset {
                kind: MediaAssetKind::Static {
                    frame: lianli_media::Retained::frame(vec![1]).unwrap(),
                },
                config_key: "shutdown-test".into(),
                stream_fps: 20.0,
                hardware_video: false,
            });
            let mut target = ActiveTarget::new(
                0,
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
    fn stream_replacement_waits_for_retirement_and_preserves_the_new_lease() {
        let (lcd, _) = lcd(0);
        let old = lcd.begin_stream().unwrap();
        assert!(lcd.begin_stream().is_none());
        let _device = lcd.lock();
        assert!(lcd.recovery_idle().is_none());
        old.release();
        let new = lcd.begin_stream().unwrap();
        drop(old);
        assert!(lcd.recovery_idle().is_none());
        assert!(lcd.begin_stream().is_none());
        drop(new);
        assert!(lcd.recovery_idle().is_some());
    }

    #[test]
    fn replacement_hid_worker_stays_pending_until_the_old_producer_retires() {
        let (lcd, sends) = lcd(usize::MAX);
        let old = lcd.begin_stream().unwrap();
        let mut replacement = HidStreamWorker::new(
            lcd.clone(),
            Box::new(std::io::Cursor::new(Vec::<u8>::new())),
            Arc::new(AtomicBool::new(false)),
            30.0,
        );
        assert!(replacement.handle.is_none());
        assert!(!replacement.try_start());
        assert_eq!(sends.load(Ordering::Relaxed), 0);
        drop(old);
        assert!(replacement.try_start());
        replacement.handle.take().unwrap().join().unwrap();
        drop(replacement);
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
        assert!(!restarter.try_start_pending().unwrap());
        drop(idle);
        let device = lcd.lock();
        assert!(restarter.try_start_pending().unwrap());
        assert!(lcd.recovery_idle().is_none());
        drop(device);
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while restarter.try_start_pending().is_ok() && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert!(restarter.try_start_pending().is_err());
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
            let transferred = Arc::new(AtomicBool::new(false));
            let (worker, _) = spawn_hid_h264_stream(
                Arc::clone(&lcd),
                reader,
                Arc::new(AtomicBool::new(stopped)),
                30.0,
                lcd.begin_stream().unwrap(),
                transferred.clone(),
            );
            assert_eq!(worker.join().is_err(), panics);
            assert_eq!(
                transferred.load(Ordering::Acquire),
                !stopped && !panics && sends.load(Ordering::Relaxed) > 0
            );
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
    fn failed_file_flush_releases_gate_and_explicit_retry_takes_a_new_lease() {
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
            assert_eq!(source.transferred(), Some(true));
            assert!(!source
                .hid_completed
                .as_ref()
                .unwrap()
                .load(Ordering::Acquire));
            assert!(lcd.recovery_idle().is_some());

            assert!(source.has_exited());
            assert!(source.start(&backend).is_err());
            assert!(source.has_exited());
            source = H264FileSource::new(path.clone(), false, 30.0);
            assert!(!source.transferred.load(Ordering::Acquire));
            let idle = lcd.recovery_idle().unwrap();
            source.start(&backend).unwrap();
            assert!(!source.started);
            assert!(source.hid_thread.is_none());
            drop(idle);
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
            assert!(!source.has_exited());
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
        let transferred = Arc::new(AtomicBool::new(false));
        let (worker, _) = spawn_hid_h264_stream(
            Arc::clone(&lcd),
            Box::new(std::io::Cursor::new(vec![0, 0, 0, 1, 5, 128])),
            Arc::new(AtomicBool::new(false)),
            30.0,
            lcd.begin_stream().unwrap(),
            transferred.clone(),
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
        assert!(!transferred.load(Ordering::Acquire));
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
        let frame = lianli_media::Retained::frame(vec![0xDE, 0xAD, 0xBE, 0xEF]).unwrap();
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
