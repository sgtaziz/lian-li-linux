use super::DaemonEvent;
use lianli_devices::crypto::PacketBuilder;
use lianli_devices::slv3_lcd::Slv3LcdDevice;
use lianli_devices::traits::LcdDevice;
use lianli_devices::winusb_lcd::WinUsbLcdDevice;
use lianli_devices::wireless::WirelessController;
use lianli_media::sensor::FrameInfo;
use lianli_media::video::LiveH264Encoder;
use lianli_media::{CustomAsset, MediaAsset, MediaAssetKind, SensorAsset};
use lianli_shared::config::ConfigKey;
use lianli_shared::screen::ScreenInfo;
use parking_lot::Mutex;
use std::path::PathBuf;
use std::process::ChildStdout;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

pub(super) type SharedHidLcd = Arc<Mutex<Box<dyn LcdDevice>>>;

pub(super) enum LcdBackend {
    Slv3(Slv3LcdDevice),
    WinUsb(ThreadedWinUsbSender),
    HidLcd(SharedHidLcd),
}

impl LcdBackend {
    fn send_frame(
        &mut self,
        wireless: &WirelessController,
        builder: &mut PacketBuilder,
        frame: &[u8],
    ) -> anyhow::Result<()> {
        match self {
            Self::Slv3(d) => {
                wireless.ensure_video_mode()?;
                d.send_frame(builder, frame)
            }
            Self::WinUsb(d) => d.send_frame(frame),
            Self::HidLcd(d) => d.lock().send_jpeg_frame(frame),
        }
    }

    fn send_frame_verified(
        &mut self,
        wireless: &WirelessController,
        builder: &mut PacketBuilder,
        frame: &[u8],
    ) -> anyhow::Result<()> {
        match self {
            Self::WinUsb(d) => d.send_frame_verified(frame),
            Self::HidLcd(d) => d.lock().send_static_frame(frame),
            _ => self.send_frame(wireless, builder, frame),
        }
    }

    fn start_h264_stream(
        &self,
        stdout: ChildStdout,
        stop: Arc<AtomicBool>,
        fps: f32,
    ) -> anyhow::Result<Option<JoinHandle<()>>> {
        match self {
            Self::WinUsb(sender) => {
                sender.stream_h264_reader(stdout, fps)?;
                Ok(None)
            }
            Self::HidLcd(lcd) => {
                let lcd = Arc::clone(lcd);
                let mut stdout = stdout;
                let handle = thread::spawn(move || {
                    let mut guard = lcd.lock();
                    if let Err(e) = guard.stream_h264_reader(&mut stdout, &stop, fps) {
                        warn!("HID h264 stream error: {e:#}");
                    }
                });
                Ok(Some(handle))
            }
            _ => anyhow::bail!("h264 streaming not supported on this backend"),
        }
    }
}

enum LcdThreadMsg {
    Frame(Vec<u8>),
    FrameVerified(Vec<u8>, std::sync::mpsc::SyncSender<anyhow::Result<()>>),
    StreamH264 {
        path: PathBuf,
        looping: bool,
        fps: f32,
    },
    StreamH264Reader(ChildStdout, f32),
    SwitchDesktop(std::sync::mpsc::SyncSender<anyhow::Result<()>>),
    Stop,
}

pub(super) struct ThreadedWinUsbSender {
    tx: std::sync::mpsc::SyncSender<LcdThreadMsg>,
    h264_stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ThreadedWinUsbSender {
    pub(super) fn new(mut device: WinUsbLcdDevice, index: usize) -> Self {
        let (tx, rx) = std::sync::mpsc::sync_channel::<LcdThreadMsg>(2);
        let h264_stop = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&h264_stop);
        let thread = thread::spawn(move || {
            for msg in rx {
                match msg {
                    LcdThreadMsg::Frame(data) => {
                        if let Err(e) = device.send_frame(&data) {
                            warn!("LCD[{index}] sender thread frame error: {e}");
                        }
                    }
                    LcdThreadMsg::FrameVerified(data, reply) => {
                        let result = device.send_frame_verified(&data);
                        let _ = reply.send(result);
                    }
                    LcdThreadMsg::StreamH264 { path, looping, fps } => {
                        stop_clone.store(false, Ordering::Relaxed);
                        if let Err(e) = device.stream_h264(&path, looping, &stop_clone, fps) {
                            warn!("LCD[{index}] h264 stream error: {e}");
                        }
                    }
                    LcdThreadMsg::StreamH264Reader(mut stdout, fps) => {
                        stop_clone.store(false, Ordering::Relaxed);
                        if let Err(e) = device.stream_h264_reader(&mut stdout, &stop_clone, fps) {
                            warn!("LCD[{index}] live h264 stream error: {e}");
                        }
                    }
                    LcdThreadMsg::SwitchDesktop(reply) => {
                        let result = device.switch_to_desktop_mode();
                        let _ = reply.send(result);
                        break;
                    }
                    LcdThreadMsg::Stop => break,
                }
            }
            device.transport_release();
        });
        Self {
            tx,
            h264_stop,
            thread: Some(thread),
        }
    }

    fn stream_h264(&self, path: PathBuf, looping: bool, fps: f32) -> anyhow::Result<()> {
        self.h264_stop.store(true, Ordering::Relaxed);
        self.tx
            .send(LcdThreadMsg::StreamH264 { path, looping, fps })
            .map_err(|_| anyhow::anyhow!("LCD sender thread exited"))?;
        Ok(())
    }

    fn stream_h264_reader(&self, stdout: ChildStdout, fps: f32) -> anyhow::Result<()> {
        self.h264_stop.store(true, Ordering::Relaxed);
        self.tx
            .send(LcdThreadMsg::StreamH264Reader(stdout, fps))
            .map_err(|_| anyhow::anyhow!("LCD sender thread exited"))?;
        Ok(())
    }

    fn send_frame(&self, frame: &[u8]) -> anyhow::Result<()> {
        self.h264_stop.store(true, Ordering::Relaxed);
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
        self.h264_stop.store(true, Ordering::Relaxed);
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
        self.h264_stop.store(true, Ordering::Relaxed);
        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
        self.tx
            .send(LcdThreadMsg::FrameVerified(frame.to_vec(), reply_tx))
            .map_err(|_| anyhow::anyhow!("LCD sender thread exited"))?;
        reply_rx
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| anyhow::anyhow!("LCD sender thread timeout"))?
    }

    fn stop(&mut self) {
        self.h264_stop.store(true, Ordering::Relaxed);
        let _ = self.tx.send(LcdThreadMsg::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
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
        let media =
            make_frame_source(Arc::clone(&asset), tx.clone(), &lcd, &screen, custom_h264);
        let recovery_stop = Arc::new(AtomicBool::new(false));
        let recovery_thread = match &lcd {
            LcdBackend::HidLcd(d) => {
                let lcd = Arc::clone(d);
                let stop = Arc::clone(&recovery_stop);
                let recovery_tx = tx.clone();
                Some(thread::spawn(move || {
                    use lianli_devices::traits::RecoveryAction;
                    while !stop.load(Ordering::Relaxed) {
                        thread::sleep(Duration::from_secs(2));
                        if stop.load(Ordering::Relaxed) {
                            break;
                        }
                        match lcd.lock().check_and_recover_lcd() {
                            Ok(RecoveryAction::Recovered) => {
                                if let Some(tx) = &recovery_tx {
                                    tx.send(DaemonEvent::RecreateMedia {
                                        target_index: index,
                                    })
                                    .ok();
                                }
                            }
                            Ok(RecoveryAction::NoChange) => {}
                            Err(e) => {
                                debug!("LCD[{index}] health check error: {e:#}");
                            }
                        }
                    }
                }))
            }
            _ => None,
        };
        Self {
            index,
            key,
            device_identity,
            lcd,
            media,
            asset,
            screen,
            custom_h264,
            frame_counter: 0,
            consecutive_errors: 0,
            recovery_stop,
            recovery_thread,
        }
    }

    pub(super) fn matches(&self, identity: &str, key: &ConfigKey) -> bool {
        self.device_identity == identity && key == &self.key
    }

    /// Replace the media asset without reopening the LCD transport.
    pub(super) fn swap_media(&mut self, asset: Arc<MediaAsset>, tx: Option<Sender<DaemonEvent>>) {
        self.asset = Arc::clone(&asset);
        self.media = make_frame_source(asset, tx, &self.lcd, &self.screen, self.custom_h264);
        self.frame_counter = 0;
        info!(
            "[devices] LCD[{}] media swapped (keeping transport)",
            self.index
        );
    }

    pub(super) fn send_frame(
        &mut self,
        wireless: &WirelessController,
        builder: &mut PacketBuilder,
    ) -> Result<bool, SendError> {
        // H.264 / autonomous sources: kick off streaming on the first call,
        // then short-circuit (their threads push frames directly to the LCD).
        if self.media.is_autonomous() {
            self.media
                .start(&self.lcd)
                .map_err(SendError::Other)?;
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
        result.map_err(
            |err| match err.downcast::<lianli_transport::TransportError>() {
                Ok(usb) => SendError::Usb(usb),
                Err(other) => SendError::Other(other),
            },
        )?;

        self.frame_counter += 1;
        Ok(true)
    }

    pub(super) fn stop(&mut self) {
        self.recovery_stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.recovery_thread.take() {
            let _ = t.join();
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

// ─── JPEG sources ──────────────────────────────────────────────────────

struct StaticSource {
    frame: Arc<Vec<u8>>,
}
impl FrameSource for StaticSource {
    fn next_frame(&mut self) -> Option<&[u8]> {
        Some(self.frame.as_slice())
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
}

impl H264FileSource {
    fn new(path: PathBuf, looping: bool, fps: f32) -> Self {
        Self {
            path,
            looping,
            fps,
            started: false,
            hid_thread: None,
            hid_stop: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl FrameSource for H264FileSource {
    fn start(&mut self, lcd: &LcdBackend) -> anyhow::Result<()> {
        if self.started {
            return Ok(());
        }
        match lcd {
            LcdBackend::WinUsb(sender) => {
                sender.stream_h264(self.path.clone(), self.looping, self.fps)?;
            }
            LcdBackend::HidLcd(hid) => {
                let lcd = Arc::clone(hid);
                let (path, looping, fps) = (self.path.clone(), self.looping, self.fps);
                let stop = Arc::clone(&self.hid_stop);
                self.hid_thread = Some(thread::spawn(move || {
                    stream_h264_file_to_hid(lcd, path, looping, fps, stop);
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
        if let Some(t) = self.hid_thread.take() {
            let _ = t.join();
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
        }),
        MediaAssetKind::Video { frames, .. } => {
            let player = Arc::new(AsyncVideoPlayer::new(tx, Arc::clone(&asset)));
            Box::new(VideoSource {
                player,
                frames: Arc::clone(frames),
                sent_index: 0,
            })
        }
        MediaAssetKind::Sensor { asset: sensor_asset } => {
            if screen.h264 && matches!(lcd, LcdBackend::WinUsb(_) | LcdBackend::HidLcd(_)) {
                match AsyncSensorH264Renderer::new(Arc::clone(sensor_asset), lcd, screen) {
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
        MediaAssetKind::H264Stream { path, looping, fps, .. } => {
            Box::new(H264FileSource::new(path.clone(), *looping, *fps))
        }
        MediaAssetKind::Custom { asset: custom_asset } => {
            if custom_h264 && screen.h264 {
                if matches!(lcd, LcdBackend::WinUsb(_) | LcdBackend::HidLcd(_)) {
                    match AsyncCustomH264Renderer::new(
                        Arc::clone(custom_asset),
                        lcd,
                        screen,
                        custom_asset.canvas_width(),
                        custom_asset.canvas_height(),
                        custom_asset.total_rotation_deg(),
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

fn stream_h264_file_to_hid(
    lcd: SharedHidLcd,
    path: PathBuf,
    looping: bool,
    fps: f32,
    stop: Arc<AtomicBool>,
) {
    use std::io::{Seek, SeekFrom};
    let mut file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(e) => {
            warn!("HID h264 file open failed: {e:#}");
            return;
        }
    };
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let mut guard = lcd.lock();
        if let Err(e) = guard.stream_h264_reader(&mut file, &stop, fps) {
            warn!("HID h264 stream error: {e:#}");
            break;
        }
        drop(guard);
        if !looping || stop.load(Ordering::Relaxed) {
            break;
        }
        if let Err(e) = file.seek(SeekFrom::Start(0)) {
            warn!("HID h264 file seek failed: {e:#}");
            break;
        }
    }
}

struct AsyncSensorRenderer {
    #[allow(dead_code)] // We'd like to keep the SensorAsset, who knows if we'll need it
    asset: Arc<SensorAsset>,
    current_frame: Arc<Mutex<FrameInfo>>,
    stop_flag: Arc<AtomicBool>,
    _thread: Option<JoinHandle<()>>,
}

impl AsyncSensorRenderer {
    fn new(
        tx: Option<Sender<DaemonEvent>>,
        asset: Arc<SensorAsset>,
        baseasset: Arc<MediaAsset>,
        keep_alive_on_no_change: bool,
    ) -> Self {
        let initial = match asset.render_frame(true) {
            Ok(Some(frame)) => frame,
            Ok(None) => asset.blank_frame(),
            Err(err) => {
                warn!("sensor initial render failed: {err}");
                asset.blank_frame()
            }
        };

        let current_frame = Arc::new(Mutex::new(initial));
        let stop_flag = Arc::new(AtomicBool::new(false));
        let update_interval = asset.update_interval();

        let asset_clone = Arc::clone(&asset);
        let frame_clone = Arc::clone(&current_frame);
        let stop_clone = Arc::clone(&stop_flag);

        let asset_for_thread = Arc::clone(&baseasset);
        let tx_for_thread = tx.clone();

        let thread = thread::spawn(move || {
            while !stop_clone.load(Ordering::Relaxed) {
                thread::sleep(update_interval);
                if stop_clone.load(Ordering::Relaxed) {
                    break;
                }
                match asset_clone.render_frame(keep_alive_on_no_change) {
                    Ok(Some(new_frame)) => {
                        *frame_clone.lock() = new_frame;
                    }
                    Ok(None) => {
                        frame_clone.lock().frame_index += 1;
                    }
                    Err(err) => {
                        warn!("sensor background render failed: {err}");
                        continue;
                    }
                }
                if let Some(ref tx) = tx_for_thread {
                    let event = DaemonEvent::FrameFinished {
                        asset: Arc::clone(&asset_for_thread),
                    };
                    if tx.send(event).is_err() {
                        break;
                    }
                }
            }
        });

        Self {
            asset,
            current_frame,
            stop_flag,
            _thread: Some(thread),
        }
    }

    fn get_frame_index(&self) -> usize {
        self.current_frame.lock().frame_index
    }

    fn get_current_frame(&self) -> Vec<u8> {
        self.current_frame.lock().data.clone()
    }
}

impl Drop for AsyncSensorRenderer {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

struct AsyncVideoPlayer {
    stop_flag: Arc<AtomicBool>,
    _thread: Option<JoinHandle<()>>,
    frame_index: Arc<AtomicUsize>,
}

impl AsyncVideoPlayer {
    fn new(tx: Option<Sender<DaemonEvent>>, asset: Arc<MediaAsset>) -> Self {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop_flag);

        let tx_for_thread = tx.clone();

        let asset_for_thread = Arc::clone(&asset);

        let min_dur = Duration::from_millis(10);
        let std_dur = Duration::from_millis(100);

        let frame_durations: Vec<Duration> = if let MediaAssetKind::Video {
            frame_durations, ..
        } = &asset.kind
        {
            frame_durations.iter().map(|&d| d.max(min_dur)).collect()
        } else {
            vec![min_dur; 1]
        };

        let frame_index: Arc<AtomicUsize> = Arc::new(0.into());
        let frame_index_cloned = frame_index.clone();

        let thread = thread::spawn(move || {
            while !stop_clone.load(Ordering::Relaxed) {
                let mut frame_cnt = 0;
                if let Some(ref tx) = tx_for_thread {
                    frame_cnt = frame_index.fetch_add(1, Ordering::SeqCst);
                    let event = DaemonEvent::FrameFinished {
                        asset: Arc::clone(&asset_for_thread),
                    };
                    if tx.send(event).is_err() {
                        break;
                    }
                }

                if stop_clone.load(Ordering::Relaxed) {
                    break;
                }

                let millis = frame_durations.get(frame_cnt % frame_durations.len());
                thread::sleep(*millis.unwrap_or(&std_dur));
            }
        });

        Self {
            stop_flag,
            _thread: Some(thread),
            frame_index: frame_index_cloned,
        }
    }

    fn get_frame_index(&self) -> usize {
        self.frame_index.load(Ordering::SeqCst)
    }
}

impl Drop for AsyncVideoPlayer {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

struct AsyncCustomRenderer {
    current_frame: Arc<Mutex<FrameInfo>>,
    stop_flag: Arc<AtomicBool>,
    _thread: Option<JoinHandle<()>>,
}

impl AsyncCustomRenderer {
    fn new(
        tx: Option<Sender<DaemonEvent>>,
        asset: Arc<CustomAsset>,
        baseasset: Arc<MediaAsset>,
        keep_alive_on_no_change: bool,
    ) -> Self {
        let initial = match asset.render_frame(true) {
            Ok(Some(frame)) => frame,
            Ok(None) => asset.blank_frame(),
            Err(err) => {
                warn!("Custom initial render failed: {err}");
                asset.blank_frame()
            }
        };

        let current_frame = Arc::new(Mutex::new(initial));
        let stop_flag = Arc::new(AtomicBool::new(false));
        let update_interval = asset.update_interval();

        let asset_clone = Arc::clone(&asset);
        let frame_clone = Arc::clone(&current_frame);
        let stop_clone = Arc::clone(&stop_flag);

        let asset_for_thread = Arc::clone(&baseasset);
        let tx_for_thread = tx.clone();

        let thread = thread::spawn(move || {
            let mut next_deadline = Instant::now() + update_interval;
            while !stop_clone.load(Ordering::Relaxed) {
                let now = Instant::now();
                if now < next_deadline {
                    thread::sleep(next_deadline - now);
                }
                if stop_clone.load(Ordering::Relaxed) {
                    break;
                }
                next_deadline += update_interval;
                if next_deadline < Instant::now() {
                    next_deadline = Instant::now() + update_interval;
                }
                match asset_clone.render_frame(keep_alive_on_no_change) {
                    Ok(Some(new_frame)) => {
                        *frame_clone.lock() = new_frame;
                        if let Some(ref tx) = tx_for_thread {
                            let event = DaemonEvent::FrameFinished {
                                asset: Arc::clone(&asset_for_thread),
                            };
                            if tx.send(event).is_err() {
                                break;
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(err) => {
                        warn!("Custom background render failed: {err}");
                    }
                }
            }
        });

        Self {
            current_frame,
            stop_flag,
            _thread: Some(thread),
        }
    }

    fn get_frame_index(&self) -> usize {
        self.current_frame.lock().frame_index
    }

    fn get_current_frame(&self) -> Vec<u8> {
        self.current_frame.lock().data.clone()
    }
}

impl Drop for AsyncCustomRenderer {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

struct AsyncCustomH264Renderer {
    stop_flag: Arc<AtomicBool>,
    _encoder: Arc<Mutex<LiveH264Encoder>>,
    _thread: Option<JoinHandle<()>>,
    _stream_thread: Option<JoinHandle<()>>,
}

impl AsyncCustomH264Renderer {
    fn new(
        asset: Arc<CustomAsset>,
        lcd: &LcdBackend,
        screen: &ScreenInfo,
        canvas_w: u32,
        canvas_h: u32,
        rotation_deg: u16,
    ) -> anyhow::Result<Self> {
        let fps = screen.max_fps as f32;
        let mut encoder = LiveH264Encoder::spawn(canvas_w, canvas_h, fps, rotation_deg, screen)
            .map_err(|e| anyhow::anyhow!("h264 encoder spawn: {e}"))?;
        let stdout = encoder
            .take_stdout()
            .ok_or_else(|| anyhow::anyhow!("h264 encoder stdout missing"))?;

        let stop_flag = Arc::new(AtomicBool::new(false));
        let stream_thread = lcd.start_h264_stream(stdout, Arc::clone(&stop_flag), fps)?;
        let stop_clone = Arc::clone(&stop_flag);
        let encoder = Arc::new(Mutex::new(encoder));
        let encoder_clone = Arc::clone(&encoder);
        let frame_interval =
            Duration::from_secs_f32(1.0 / fps.max(1.0)).max(Duration::from_millis(16));

        let thread = thread::spawn(move || {
            let mut next_deadline = Instant::now() + frame_interval;
            while !stop_clone.load(Ordering::Relaxed) {
                let now = Instant::now();
                if now < next_deadline {
                    thread::sleep(next_deadline - now);
                }
                if stop_clone.load(Ordering::Relaxed) {
                    break;
                }
                next_deadline += frame_interval;
                if next_deadline < Instant::now() {
                    next_deadline = Instant::now() + frame_interval;
                }

                let outcome = asset.render_frame_rgba_with(true, |rgba| {
                    let mut enc = encoder_clone.lock();
                    enc.write_frame(rgba)
                });
                match outcome {
                    Ok(Some(Ok(()))) => {}
                    Ok(Some(Err(e))) => {
                        warn!("custom h264 encoder write failed: {e}");
                        break;
                    }
                    Ok(None) => {}
                    Err(err) => {
                        warn!("custom h264 render failed: {err}");
                        break;
                    }
                }
            }
        });

        Ok(Self {
            stop_flag,
            _encoder: encoder,
            _thread: Some(thread),
            _stream_thread: stream_thread,
        })
    }
}

impl Drop for AsyncCustomH264Renderer {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(t) = self._thread.take() {
            let _ = t.join();
        }
    }
}

struct AsyncSensorH264Renderer {
    stop_flag: Arc<AtomicBool>,
    _encoder: Arc<Mutex<LiveH264Encoder>>,
    _thread: Option<JoinHandle<()>>,
    _stream_thread: Option<JoinHandle<()>>,
}

impl AsyncSensorH264Renderer {
    fn new(
        asset: Arc<lianli_media::SensorAsset>,
        lcd: &LcdBackend,
        screen: &ScreenInfo,
    ) -> anyhow::Result<Self> {
        let initial = match asset.render_frame_rgba(true)? {
            Some(img) => img,
            None => {
                anyhow::bail!("sensor produced no initial frame");
            }
        };
        let canvas_w = initial.width();
        let canvas_h = initial.height();
        let fps = screen.max_fps as f32;
        let mut encoder = LiveH264Encoder::spawn(canvas_w, canvas_h, fps, 0, screen)
            .map_err(|e| anyhow::anyhow!("h264 encoder spawn: {e}"))?;
        let stdout = encoder
            .take_stdout()
            .ok_or_else(|| anyhow::anyhow!("h264 encoder stdout missing"))?;

        let stop_flag = Arc::new(AtomicBool::new(false));
        let stream_thread = lcd.start_h264_stream(stdout, Arc::clone(&stop_flag), fps)?;
        let stop_clone = Arc::clone(&stop_flag);
        let encoder = Arc::new(Mutex::new(encoder));
        let encoder_clone = Arc::clone(&encoder);
        let frame_interval =
            Duration::from_secs_f32(1.0 / fps.max(1.0)).max(Duration::from_millis(16));

        if let Err(e) = encoder_clone.lock().write_frame(initial.as_raw()) {
            warn!("sensor h264 initial frame write failed: {e}");
        }

        let thread = thread::spawn(move || {
            let mut next_deadline = Instant::now() + frame_interval;
            while !stop_clone.load(Ordering::Relaxed) {
                let now = Instant::now();
                if now < next_deadline {
                    thread::sleep(next_deadline - now);
                }
                if stop_clone.load(Ordering::Relaxed) {
                    break;
                }
                next_deadline += frame_interval;
                if next_deadline < Instant::now() {
                    next_deadline = Instant::now() + frame_interval;
                }

                match asset.render_frame_rgba(true) {
                    Ok(Some(rgba)) => {
                        let mut enc = encoder_clone.lock();
                        if let Err(e) = enc.write_frame(rgba.as_raw()) {
                            warn!("sensor h264 encoder write failed: {e}");
                            break;
                        }
                    }
                    Ok(None) => {}
                    Err(err) => {
                        warn!("sensor h264 render failed: {err}");
                        break;
                    }
                }
            }
        });

        Ok(Self {
            stop_flag,
            _encoder: encoder,
            _thread: Some(thread),
            _stream_thread: stream_thread,
        })
    }
}

impl Drop for AsyncSensorH264Renderer {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(t) = self._thread.take() {
            let _ = t.join();
        }
    }
}


pub(super) enum SendError {
    Usb(lianli_transport::TransportError),
    Other(anyhow::Error),
}

pub(super) fn parse_mac_str(s: &str) -> Option<[u8; 6]> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 6 {
        return None;
    }
    let mut mac = [0u8; 6];
    for (i, part) in parts.iter().enumerate() {
        mac[i] = u8::from_str_radix(part, 16).ok()?;
    }
    Some(mac)
}
