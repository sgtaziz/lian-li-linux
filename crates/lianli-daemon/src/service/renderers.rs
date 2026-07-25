use super::runtime::{LcdBackend, StreamRestarter};
use super::DaemonEvent;
use lianli_media::sensor::FrameInfo;
use lianli_media::video::LiveH264Encoder;
use lianli_media::{CustomAsset, MediaAsset, MediaAssetKind, SensorAsset};
use lianli_shared::screen::ScreenInfo;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{info, warn};

// ── Encoder restart policy ───────────────────────────────────────────────────
/// Don't restart if the encoder crashed within this period — likely systemic.
const MIN_HEALTHY_UPTIME: Duration = Duration::from_secs(10);
/// Max restart attempts before giving up.
const MAX_RESTARTS: u32 = 3;
/// Reset the restart counter after this long healthy streak.
const HEALTHY_RESET: Duration = Duration::from_secs(300);

/// Attempt to respawn the encoder and restart the h264 stream after a write
/// failure. Returns `true` if the caller should continue the render loop.
fn try_restart_encoder(
    encoder: &Arc<Mutex<LiveH264Encoder>>,
    restarter: &StreamRestarter,
    stop: &Arc<AtomicBool>,
    canvas_w: u32,
    canvas_h: u32,
    fps: f32,
    rotation_deg: u16,
    screen: &ScreenInfo,
    restart_count: &mut u32,
    started_at: &mut Instant,
) -> bool {
    if *restart_count >= MAX_RESTARTS {
        warn!("h264 encoder exceeded max restarts ({MAX_RESTARTS}), giving up");
        return false;
    }

    // Reset counter if the encoder was healthy for a long stretch.
    if started_at.elapsed() > HEALTHY_RESET {
        *restart_count = 0;
    }

    if started_at.elapsed() < MIN_HEALTHY_UPTIME {
        warn!(
            "h264 encoder only ran {:?} (< {MIN_HEALTHY_UPTIME:?}), not restarting",
            started_at.elapsed()
        );
        return false;
    }

    *restart_count += 1;
    let backoff_secs = 2u64.pow(restart_count.saturating_sub(1)).min(30);
    info!(
        "restarting h264 encoder (attempt {}/{MAX_RESTARTS}) after {backoff_secs}s backoff",
        *restart_count
    );

    // Backoff sleep — interruptible by stop flag.
    let mut remaining = Duration::from_secs(backoff_secs);
    while remaining > Duration::ZERO {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        let step = remaining.min(Duration::from_millis(100));
        thread::sleep(step);
        remaining -= step;
    }

    let mut new_encoder =
        match LiveH264Encoder::spawn(canvas_w, canvas_h, fps, rotation_deg, screen) {
            Ok(enc) => enc,
            Err(e) => {
                warn!("h264 encoder respawn failed: {e}");
                return false;
            }
        };

    if let Some(stdout) = new_encoder.take_stdout() {
        if let Err(e) = restarter.start_stream(stdout, Arc::clone(stop), fps) {
            warn!("h264 stream restart failed: {e}");
            return false;
        }
    }

    // Replacing the old encoder drops it, closing ffmpeg's stdin → stdout EOFs
    // → old stream thread exits naturally.
    *encoder.lock() = new_encoder;
    *started_at = Instant::now();
    info!("h264 encoder restarted successfully");
    true
}

pub(super) struct AsyncSensorRenderer {
    current_frame: Arc<Mutex<FrameInfo>>,
    stop_flag: Arc<AtomicBool>,
    _thread: Option<JoinHandle<()>>,
}

impl AsyncSensorRenderer {
    pub(super) fn new(
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

        let _asset_for_thread = Arc::clone(&baseasset);
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
                    let event = DaemonEvent::FrameFinished;
                    if tx.send(event).is_err() {
                        break;
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

    pub(super) fn get_frame_index(&self) -> usize {
        self.current_frame.lock().frame_index
    }

    pub(super) fn get_current_frame(&self) -> Vec<u8> {
        self.current_frame.lock().data.clone()
    }
}

impl Drop for AsyncSensorRenderer {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

pub(super) struct AsyncVideoPlayer {
    stop_flag: Arc<AtomicBool>,
    _thread: Option<JoinHandle<()>>,
    frame_index: Arc<AtomicUsize>,
}

impl AsyncVideoPlayer {
    pub(super) fn new(tx: Option<Sender<DaemonEvent>>, asset: Arc<MediaAsset>) -> Self {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop_flag);

        let tx_for_thread = tx.clone();

        let _asset_for_thread = Arc::clone(&asset);

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
                    let event = DaemonEvent::FrameFinished;
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

    pub(super) fn get_frame_index(&self) -> usize {
        self.frame_index.load(Ordering::SeqCst)
    }
}

impl Drop for AsyncVideoPlayer {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

pub(super) struct AsyncCustomRenderer {
    current_frame: Arc<Mutex<FrameInfo>>,
    stop_flag: Arc<AtomicBool>,
    _thread: Option<JoinHandle<()>>,
}

impl AsyncCustomRenderer {
    pub(super) fn new(
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

        let _asset_for_thread = Arc::clone(&baseasset);
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
                            let event = DaemonEvent::FrameFinished;
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

    pub(super) fn get_frame_index(&self) -> usize {
        self.current_frame.lock().frame_index
    }

    pub(super) fn get_current_frame(&self) -> Vec<u8> {
        self.current_frame.lock().data.clone()
    }
}

impl Drop for AsyncCustomRenderer {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

pub(super) struct AsyncCustomH264Renderer {
    stop_flag: Arc<AtomicBool>,
    _encoder: Arc<Mutex<LiveH264Encoder>>,
    _thread: Option<JoinHandle<()>>,
    _stream_thread: Option<JoinHandle<()>>,
}

impl AsyncCustomH264Renderer {
    pub(super) fn new(
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

        let restarter = lcd
            .stream_restarter()
            .ok_or_else(|| anyhow::anyhow!("h264 streaming not supported on this backend"))?;
        let screen_clone = screen.clone();

        let thread = thread::spawn(move || {
            let mut next_deadline = Instant::now() + frame_interval;
            let mut restart_count = 0u32;
            let mut encoder_started_at = Instant::now();
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
                        if !try_restart_encoder(
                            &encoder_clone,
                            &restarter,
                            &stop_clone,
                            canvas_w,
                            canvas_h,
                            fps,
                            rotation_deg,
                            &screen_clone,
                            &mut restart_count,
                            &mut encoder_started_at,
                        ) {
                            break;
                        }
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

pub(super) struct AsyncSensorH264Renderer {
    stop_flag: Arc<AtomicBool>,
    _encoder: Arc<Mutex<LiveH264Encoder>>,
    _thread: Option<JoinHandle<()>>,
    _stream_thread: Option<JoinHandle<()>>,
}

impl AsyncSensorH264Renderer {
    pub(super) fn new(
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

        let restarter = lcd
            .stream_restarter()
            .ok_or_else(|| anyhow::anyhow!("h264 streaming not supported on this backend"))?;
        let screen_clone = screen.clone();

        let thread = thread::spawn(move || {
            let mut next_deadline = Instant::now() + frame_interval;
            let mut restart_count = 0u32;
            let mut encoder_started_at = Instant::now();
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
                            drop(enc);
                            if !try_restart_encoder(
                                &encoder_clone,
                                &restarter,
                                &stop_clone,
                                canvas_w,
                                canvas_h,
                                fps,
                                0,
                                &screen_clone,
                                &mut restart_count,
                                &mut encoder_started_at,
                            ) {
                                break;
                            }
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
