//! Auto-attaches evdi virtual displays to connected Lian Li TURZX panels.
//!
//! For each `(VID=0x1A86, PID ∈ 0xAD10..0xAD3F)` device on the bus we run a
//! dedicated worker thread. The worker opens the USB panel via
//! [`TurzxDisplay`], spins up an evdi display node fed with the device's own
//! EDID, encodes framebuffer updates to H.264 via libavcodec, and pushes the
//! packets as TURZX stream A.

use anyhow::{anyhow, bail, Context, Result};
use ffmpeg_next as ffmpeg;
use lianli_devices::turzx::{self, Mode as TurzxMode, TurzxDisplay, FMT_H264};
use lianli_evdi::{EvdiBuffer, EvdiHandle, Event as EvdiEvent};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

/// Key identifying a single physical USB attachment (bus + address).
pub type DeviceKey = (u8, u8);

static FFMPEG_INIT: std::sync::Once = std::sync::Once::new();

fn ensure_ffmpeg_initialized() {
    FFMPEG_INIT.call_once(|| {
        if let Err(e) = ffmpeg::init() {
            error!("ffmpeg::init failed: {e}");
        }
    });
}

/// Handle to a running worker. Dropping it signals the worker to stop and
/// waits for it to join.
pub struct DesktopDisplayHandle {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    pid: u16,
}

impl Drop for DesktopDisplayHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(j) = self.join.take() {
            if let Err(e) = j.join() {
                warn!(
                    "TURZX {:04x}:{:04x} worker panicked on shutdown: {e:?}",
                    turzx::VID, self.pid
                );
            }
        }
    }
}

/// Registry of running workers, keyed by USB (bus, address).
#[derive(Default)]
pub struct DesktopDisplayRegistry {
    inner: Mutex<HashMap<DeviceKey, DesktopDisplayHandle>>,
}

impl DesktopDisplayRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sync running workers against the currently-present TURZX devices.
    /// Spawns workers for new devices; drops handles (which stops + joins)
    /// for devices that have disappeared.
    pub fn sync(&self, present: &[TurzxDeviceMatch]) {
        let mut inner = self.inner.lock();
        let present_keys: std::collections::HashSet<DeviceKey> =
            present.iter().map(|m| m.key).collect();

        inner.retain(|key, _| {
            if present_keys.contains(key) {
                true
            } else {
                info!("TURZX {:02x}:{:02x} disappeared — stopping worker", key.0, key.1);
                false
            }
        });

        for m in present {
            if inner.contains_key(&m.key) {
                continue;
            }
            match spawn_worker(m.pid) {
                Ok(h) => {
                    info!(
                        "TURZX {:04x}:{:04x} at bus {}/addr {} — worker spawned",
                        turzx::VID,
                        m.pid,
                        m.key.0,
                        m.key.1
                    );
                    inner.insert(m.key, h);
                }
                Err(e) => warn!(
                    "TURZX {:04x}:{:04x} at bus {}/addr {} — spawn failed: {e:#}",
                    turzx::VID,
                    m.pid,
                    m.key.0,
                    m.key.1
                ),
            }
        }
    }

    /// Stop (and join) any running worker for the given PID. Used when the
    /// user initiates a desktop→LCD mode switch on a device and we want the
    /// stream loop out of the way before the reboot flood.
    pub fn stop_for_pid(&self, pid: u16) {
        let mut inner = self.inner.lock();
        inner.retain(|_, h| {
            if h.pid == pid {
                info!(
                    "TURZX {:04x}:{:04x} — stopping worker for mode switch",
                    turzx::VID,
                    pid
                );
                false
            } else {
                true
            }
        });
    }

    pub fn shutdown(&self) {
        let mut inner = self.inner.lock();
        inner.clear();
        // Ask the kernel to tear down any /dev/dri/cardN we created.
        match lianli_evdi::remove_all_devices() {
            Ok(true) => info!("evdi virtual display nodes removed"),
            Ok(false) => {}
            Err(e) => warn!("remove_all_devices failed: {e:#}"),
        }
    }
}

/// A single detected TURZX device from a bus scan.
#[derive(Debug, Clone, Copy)]
pub struct TurzxDeviceMatch {
    pub pid: u16,
    pub key: DeviceKey,
}

/// Enumerate currently-attached TURZX panels on the USB bus.
pub fn enumerate_turzx() -> Result<Vec<TurzxDeviceMatch>> {
    let devices = rusb::devices().context("rusb::devices")?;
    let mut out = Vec::new();
    for device in devices.iter() {
        let desc = match device.device_descriptor() {
            Ok(d) => d,
            Err(_) => continue,
        };
        let vid = desc.vendor_id();
        let pid = desc.product_id();
        if !turzx::is_turzx(vid, pid) {
            continue;
        }
        out.push(TurzxDeviceMatch {
            pid,
            key: (device.bus_number(), device.address()),
        });
    }
    Ok(out)
}

fn spawn_worker(pid: u16) -> Result<DesktopDisplayHandle> {
    ensure_ffmpeg_initialized();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();
    let join = thread::Builder::new()
        .name(format!("turzx-bridge-{pid:04x}"))
        .spawn(move || {
            if let Err(e) = run_worker(pid, stop_clone) {
                error!("TURZX {:04x}:{pid:04x} worker exited: {e:#}", turzx::VID);
            }
        })
        .context("spawning worker thread")?;
    Ok(DesktopDisplayHandle {
        stop,
        join: Some(join),
        pid,
    })
}

fn run_worker(pid: u16, stop: Arc<AtomicBool>) -> Result<()> {
    let mut display = TurzxDisplay::open(pid)
        .with_context(|| format!("opening TURZX {:04x}:{pid:04x}", turzx::VID))?;
    let caps = display.caps().clone();
    let edid = *display.edid();
    let identity = display.identity().clone();
    info!(
        "TURZX {pid:04x} identity: usb_serial={:?} port={} → EDID serial=0x{:08x}",
        identity.usb_serial, identity.port_path, identity.edid_serial
    );
    debug!("TURZX {pid:04x} caps: {caps:?}");

    let mut evdi = EvdiHandle::open_or_add().context("evdi open_or_add")?;
    let lib_version = EvdiHandle::lib_version();
    info!(
        "evdi library {}.{}.{} connected for TURZX {pid:04x}",
        lib_version.0, lib_version.1, lib_version.2
    );
    let sku_area_limit = (caps.max_w as u32).saturating_mul(caps.max_h as u32).max(1);

    // Pre-register a buffer at the device's preferred mode BEFORE evdi_connect.
    // Some evdi/compositor combinations won't complete a mode-set commit until
    // they see a registered buffer of a compatible size — without this the
    // kernel fires vblank events but never our mode_changed callback.
    let preferred = turzx::pick_mode(&caps).context("device advertises no modes")?;
    let preferred_resolved = ResolvedMode {
        width: preferred.width as u32,
        height: preferred.height as u32,
        refresh_hz: preferred.refresh_hz as u32,
    };
    let mut buffer: Option<EvdiBuffer> = Some(EvdiBuffer::new(
        1,
        preferred_resolved.width as i32,
        preferred_resolved.height as i32,
    ));
    if let Some(buf) = buffer.as_mut() {
        evdi.register_buffer(buf);
    }

    // Pixel-per-second hint for evdi_connect2 — mirror DisplayLinkManager.
    // 1920×480@60Hz peak = ~55 Mpx/s; 80M gives headroom without tripping
    // USB 2.0 HS limits.
    let pixel_per_sec_limit = 80_000_000u32;
    evdi.connect_with_rate(&edid, sku_area_limit, pixel_per_sec_limit)
        .context("evdi_connect2")?;
    let event_fd = evdi.raw_event_fd();
    info!(
        "TURZX {pid:04x} evdi connected (event fd={event_fd}, sku_area_limit={sku_area_limit}, \
         preferred {}×{}@{}Hz buffer pre-registered); waiting for compositor mode-set",
        preferred_resolved.width, preferred_resolved.height, preferred_resolved.refresh_hz
    );

    let mut current_mode: Option<ResolvedMode> = None;
    let mut encoder: Option<H264Encoder> = None;
    let mut streaming = false;
    let mut update_pending = false;
    let mut request_in_flight = false;
    let mut grab_us: u64 = 0;
    let mut encode_us: u64 = 0;
    let mut send_us: u64 = 0;
    let mut timing_frames: u32 = 0;
    let mut timing_bytes: u64 = 0;

    while !stop.load(Ordering::SeqCst) {
        let timeout = current_mode
            .as_ref()
            .map(|m| Duration::from_millis((1000 / m.refresh_hz.max(1)) as u64))
            .unwrap_or_else(|| Duration::from_millis(200));

        let events = evdi
            .poll_events(timeout)
            .context("evdi poll_events")?;

        if !events.is_empty() {
            debug!("TURZX {pid:04x} got {} evdi event(s)", events.len());
        }

        for ev in events {
            match ev {
                EvdiEvent::ModeChanged(mode) => {
                    info!(
                        "TURZX {pid:04x} evdi mode: {}×{} @ {}Hz (bpp {}, fmt {:#x})",
                        mode.width, mode.height, mode.refresh_hz, mode.bits_per_pixel, mode.pixel_format
                    );
                    let resolved = ResolvedMode::from_evdi(mode)
                        .context("negotiated mode unsupported")?;
                    if let Some(mut old) = buffer.take() {
                        evdi.unregister_buffer(&mut old);
                    }

                    let mut new_buf =
                        EvdiBuffer::new(1, resolved.width as i32, resolved.height as i32);
                    evdi.register_buffer(&mut new_buf);
                    buffer = Some(new_buf);
                    encoder = Some(
                        H264Encoder::new(resolved.width, resolved.height, resolved.refresh_hz)
                            .context("building H264Encoder")?,
                    );
                    display
                        .start_streaming(
                            TurzxMode {
                                width: resolved.width as u16,
                                height: resolved.height as u16,
                                refresh_hz: resolved.refresh_hz as u8,
                            },
                            FMT_H264,
                        )
                        .context("TURZX start_streaming")?;
                    streaming = true;
                    current_mode = Some(resolved);
                    update_pending = false;
                    request_in_flight = false;
                }
                EvdiEvent::UpdateReady(_) => {
                    update_pending = true;
                    request_in_flight = false;
                }
                EvdiEvent::DpmsChanged(mode) => {
                    debug!("TURZX {pid:04x} DPMS changed: {mode}");
                    if mode != 0 && streaming {
                        // Non-zero DPMS modes = display off/suspend. Power off the panel.
                        if let Err(e) = display.send_power_off() {
                            warn!("TURZX {pid:04x} power_off (DPMS) failed: {e:#}");
                        }
                        streaming = false;
                    } else if mode == 0 && !streaming {
                        if let Some(m) = current_mode {
                            if let Err(e) = display.start_streaming(
                                TurzxMode {
                                    width: m.width as u16,
                                    height: m.height as u16,
                                    refresh_hz: m.refresh_hz as u8,
                                },
                                FMT_H264,
                            ) {
                                warn!("TURZX {pid:04x} DPMS resume failed: {e:#}");
                            } else {
                                streaming = true;
                            }
                        }
                    }
                }
                EvdiEvent::CrtcStateChanged(state) => {
                    debug!("TURZX {pid:04x} crtc state: {state}");
                }
            }
        }

        if !streaming {
            continue;
        }

        let (Some(buf), Some(enc)) = (buffer.as_mut(), encoder.as_mut()) else {
            continue;
        };

        if update_pending {
            update_pending = false;
            let t0 = Instant::now();
            let _rects = evdi.grab_pixels();
            let t1 = Instant::now();
            match enc.encode(buf.pixels()) {
                Ok(packet) if !packet.is_empty() => {
                    let t2 = Instant::now();
                    let packet_len = packet.len() as u64;
                    if let Err(e) = display.send_stream_a(&packet) {
                        if is_device_gone(&e) {
                            info!("TURZX {pid:04x} disconnected mid-stream, stopping worker");
                            break;
                        }
                        warn!("TURZX {pid:04x} send_stream_a failed: {e:#}");
                    }
                    let t3 = Instant::now();
                    grab_us += (t1 - t0).as_micros() as u64;
                    encode_us += (t2 - t1).as_micros() as u64;
                    send_us += (t3 - t2).as_micros() as u64;
                    timing_frames += 1;
                    timing_bytes += packet_len;
                    if timing_frames >= 60 {
                        let n = timing_frames as u64;
                        debug!(
                            "TURZX {pid:04x} timings over {} frames: grab {:.2}ms enc {:.2}ms send {:.2}ms, avg packet {} B",
                            n,
                            grab_us as f64 / n as f64 / 1000.0,
                            encode_us as f64 / n as f64 / 1000.0,
                            send_us as f64 / n as f64 / 1000.0,
                            timing_bytes / n,
                        );
                        grab_us = 0;
                        encode_us = 0;
                        send_us = 0;
                        timing_frames = 0;
                        timing_bytes = 0;
                    }
                }
                Ok(_) => {}
                Err(e) => warn!("TURZX {pid:04x} H.264 encode failed: {e:#}"),
            }
        }

        if !request_in_flight {
            if evdi.request_update(buf.id) {
                update_pending = true;
            } else {
                request_in_flight = true;
            }
        }
    }

    if let Err(e) = display.send_power_off() {
        debug!("TURZX {pid:04x} final power_off ignored: {e:#}");
    }
    Ok(())
}

fn is_device_gone(err: &anyhow::Error) -> bool {
    err.chain()
        .any(|cause| matches!(cause.downcast_ref::<rusb::Error>(), Some(rusb::Error::NoDevice)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResolvedMode {
    width: u32,
    height: u32,
    refresh_hz: u32,
}

impl ResolvedMode {
    fn from_evdi(m: lianli_evdi::Mode) -> Result<Self> {
        if m.width <= 0 || m.height <= 0 {
            bail!("evdi mode has non-positive dimensions ({:?})", m);
        }
        let refresh = m.refresh_hz.max(30) as u32;
        Ok(Self {
            width: m.width as u32,
            height: m.height as u32,
            refresh_hz: refresh,
        })
    }
}

/// libavcodec H.264 encoder specialised for BGRA(=XRGB8888) framebuffers
/// arriving from evdi. Kept persistent across frames.
struct H264Encoder {
    encoder: ffmpeg::encoder::Video,
    scaler: ffmpeg::software::scaling::Context,
    frame_in: ffmpeg::frame::Video,
    frame_out: ffmpeg::frame::Video,
    width: u32,
    height: u32,
    start: Instant,
    packet: ffmpeg::Packet,
}

impl H264Encoder {
    fn new(width: u32, height: u32, fps: u32) -> Result<Self> {
        ensure_ffmpeg_initialized();

        let gop = (fps / 2).max(1);
        let mut last_err: Option<anyhow::Error> = None;
        for name in ["h264_nvenc", "h264_amf", "libx264"] {
            match try_open_encoder(name, width, height, fps, gop) {
                Ok(encoder) => {
                    info!("H.264 encoder: {name}");
                    let scaler = ffmpeg::software::scaling::Context::get(
                        ffmpeg::util::format::Pixel::BGRA,
                        width,
                        height,
                        ffmpeg::util::format::Pixel::YUV420P,
                        width,
                        height,
                        ffmpeg::software::scaling::Flags::BILINEAR,
                    )
                    .context("building sws scaler BGRA→YUV420P")?;
                    let frame_in = ffmpeg::frame::Video::new(
                        ffmpeg::util::format::Pixel::BGRA,
                        width,
                        height,
                    );
                    let frame_out = ffmpeg::frame::Video::new(
                        ffmpeg::util::format::Pixel::YUV420P,
                        width,
                        height,
                    );
                    return Ok(Self {
                        encoder,
                        scaler,
                        frame_in,
                        frame_out,
                        width,
                        height,
                        start: Instant::now(),
                        packet: ffmpeg::Packet::empty(),
                    });
                }
                Err(e) => {
                    debug!("H.264 encoder {name} unavailable: {e:#}");
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow!("no H.264 encoder available")))
    }

    fn encode(&mut self, bgra: &[u8]) -> Result<Vec<u8>> {
        self.copy_pixels_in(bgra)?;
        self.scaler
            .run(&self.frame_in, &mut self.frame_out)
            .context("sws scale BGRA→YUV420P")?;
        self.frame_out
            .set_pts(Some(self.start.elapsed().as_micros() as i64));
        self.encoder
            .send_frame(&self.frame_out)
            .context("encoder.send_frame")?;

        let mut out = Vec::new();
        while self.encoder.receive_packet(&mut self.packet).is_ok() {
            if let Some(data) = self.packet.data() {
                out.extend_from_slice(data);
            }
        }
        Ok(out)
    }

    fn copy_pixels_in(&mut self, bgra: &[u8]) -> Result<()> {
        let expected = (self.width as usize) * 4 * (self.height as usize);
        if bgra.len() < expected {
            bail!("BGRA buffer too small: {} < {}", bgra.len(), expected);
        }
        let stride = self.frame_in.stride(0);
        let row_bytes = (self.width as usize) * 4;
        if stride == row_bytes {
            self.frame_in.data_mut(0)[..expected].copy_from_slice(&bgra[..expected]);
        } else {
            let dst = self.frame_in.data_mut(0);
            for y in 0..self.height as usize {
                let src_off = y * row_bytes;
                let dst_off = y * stride;
                dst[dst_off..dst_off + row_bytes]
                    .copy_from_slice(&bgra[src_off..src_off + row_bytes]);
            }
        }
        Ok(())
    }
}

fn try_open_encoder(
    name: &str,
    width: u32,
    height: u32,
    fps: u32,
    gop: u32,
) -> Result<ffmpeg::encoder::Video> {
    let codec = ffmpeg::encoder::find_by_name(name)
        .ok_or_else(|| anyhow!("codec {name} not built into libavcodec"))?;
    let ctx = ffmpeg::codec::context::Context::new_with_codec(codec);

    let mut opts = ffmpeg::Dictionary::new();
    match name {
        "h264_nvenc" => {
            opts.set("preset", "p1");
            opts.set("tune", "ull");
            opts.set("rc", "cbr");
            opts.set("zerolatency", "1");
            opts.set("delay", "0");
        }
        "h264_amf" => {
            opts.set("usage", "ultralowlatency");
            opts.set("quality", "speed");
            opts.set("rc", "cbr");
        }
        _ => {
            opts.set("preset", "ultrafast");
            opts.set("tune", "zerolatency");
            opts.set("x264-params", "bframes=0");
        }
    }

    let mut enc = ctx.encoder().video()?;
    enc.set_width(width);
    enc.set_height(height);
    enc.set_format(ffmpeg::util::format::Pixel::YUV420P);
    enc.set_time_base(ffmpeg::Rational(1, 1_000_000));
    enc.set_frame_rate(Some(ffmpeg::Rational(fps as i32, 1)));
    enc.set_bit_rate(5_000_000);
    enc.set_gop(gop);
    enc.set_max_b_frames(0);
    Ok(enc.open_with(opts)?)
}
