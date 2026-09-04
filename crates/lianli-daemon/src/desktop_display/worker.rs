use super::ensure_ffmpeg_initialized;
use super::DesktopDisplayHandle;
use anyhow::{bail, Context, Result};
use lianli_devices::turzx::{self, Mode as TurzxMode, TurzxDisplay, FMT_H264, FMT_MJPEG};
use lianli_evdi::{EvdiBuffer, EvdiHandle, Event as EvdiEvent};
use lianli_media::video::H264Encoder;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self};
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

/// PIDs known to have buggy H264 implementations — forced to JPEG.
/// Both rev1 (0xACD1) and rev2 (0xAD11) Lancool 207 share the silicon bug.
/// Vision 9.2" (0xAD26) ACKs H264 chunks but never renders them.
const JPEG_FORCE_PIDS: &[u16] = &[0xACD1, 0xAD11, 0xAD26];

pub(super) fn spawn_worker(pid: u16) -> Result<DesktopDisplayHandle> {
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

    let use_jpeg = JPEG_FORCE_PIDS.contains(&pid);
    let fmt = if use_jpeg { FMT_MJPEG } else { FMT_H264 };
    if use_jpeg {
        info!("TURZX {pid:04x} using JPEG mode (forced for this device)");
    }

    let mut evdi = EvdiHandle::open_or_add().context("evdi open_or_add")?;
    let lib_version = EvdiHandle::lib_version();
    info!(
        "evdi library {}.{}.{} connected for TURZX {pid:04x}",
        lib_version.0, lib_version.1, lib_version.2
    );
    let sku_area_limit = (caps.max_w as u32).saturating_mul(caps.max_h as u32).max(1);

    // Pre-register a buffer at the device's preferred mode BEFORE evdi_connect.
    // No fourcc is known before the first ModeChanged event, and nothing is
    // encoded until one arrives, so carry the historical XR24 assumption.
    let preferred = turzx::pick_mode(&caps).context("device advertises no modes")?;
    let preferred_resolved = ResolvedMode {
        width: preferred.width as u32,
        height: preferred.height as u32,
        refresh_hz: preferred.refresh_hz as u32,
        pixel_format: DRM_FORMAT_XRGB8888,
    };
    let mut buffer: Option<EvdiBuffer> = Some(EvdiBuffer::new(
        1,
        preferred_resolved.width as i32,
        preferred_resolved.height as i32,
    ));
    if let Some(buf) = buffer.as_mut() {
        evdi.register_buffer(buf);
    }

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

        let events = evdi.poll_events(timeout).context("evdi poll_events")?;

        if !events.is_empty() {
            debug!("TURZX {pid:04x} got {} evdi event(s)", events.len());
        }

        for ev in events {
            match ev {
                EvdiEvent::ModeChanged(mode) => {
                    let rgb_first =
                        matches!(mode.pixel_format, DRM_FORMAT_ABGR8888 | DRM_FORMAT_XBGR8888);
                    info!(
                        "TURZX {pid:04x} evdi mode: {}×{} @ {}Hz (bpp {}, fmt {:#x}, {} first)",
                        mode.width,
                        mode.height,
                        mode.refresh_hz,
                        mode.bits_per_pixel,
                        mode.pixel_format,
                        if rgb_first { "red" } else { "blue" }
                    );
                    let resolved =
                        ResolvedMode::from_evdi(mode).context("negotiated mode unsupported")?;
                    if let Some(mut old) = buffer.take() {
                        evdi.unregister_buffer(&mut old);
                    }

                    let mut new_buf =
                        EvdiBuffer::new(1, resolved.width as i32, resolved.height as i32);
                    evdi.register_buffer(&mut new_buf);
                    buffer = Some(new_buf);
                    if !use_jpeg {
                        encoder = Some(
                            H264Encoder::new(
                                resolved.width,
                                resolved.height,
                                resolved.refresh_hz,
                                resolved.rgb_byte_order(),
                            )
                            .context("building H264Encoder")?,
                        );
                    }
                    display
                        .start_streaming(
                            TurzxMode {
                                width: resolved.width as u16,
                                height: resolved.height as u16,
                                refresh_hz: resolved.refresh_hz as u8,
                            },
                            fmt,
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
                                fmt,
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

        let Some(buf) = buffer.as_mut() else {
            continue;
        };

        if update_pending {
            update_pending = false;
            let t0 = Instant::now();
            let _rects = evdi.grab_pixels();
            let t1 = Instant::now();

            let encode_and_send = if use_jpeg {
                let m = current_mode.unwrap();
                let tj_image = turbojpeg::Image {
                    pixels: buf.pixels(),
                    width: m.width as usize,
                    height: m.height as usize,
                    pitch: m.width as usize * 4,
                    format: if m.rgb_byte_order() {
                        turbojpeg::PixelFormat::RGBX
                    } else {
                        turbojpeg::PixelFormat::BGRX
                    },
                };
                match turbojpeg::compress(tj_image, 90, turbojpeg::Subsamp::Sub2x2) {
                    Ok(jpeg) => {
                        let jpeg = jpeg.to_vec();
                        let t2 = Instant::now();
                        let packet_len = jpeg.len() as u64;
                        let send_result = display.send_jpeg_frame(&jpeg);
                        let t3 = Instant::now();
                        Some((packet_len, t2, t3, send_result))
                    }
                    Err(e) => {
                        warn!("TURZX {pid:04x} JPEG encode failed: {e:#}");
                        None
                    }
                }
            } else {
                let Some(enc) = encoder.as_mut() else {
                    continue;
                };
                match enc.encode(buf.pixels()) {
                    Ok(packet) if !packet.is_empty() => {
                        let t2 = Instant::now();
                        let packet_len = packet.len() as u64;
                        let send_result = display.send_stream_a(&packet);
                        let t3 = Instant::now();
                        Some((packet_len, t2, t3, send_result))
                    }
                    Ok(_) => None,
                    Err(e) => {
                        warn!("TURZX {pid:04x} H.264 encode failed: {e:#}");
                        None
                    }
                }
            };

            if let Some((packet_len, t2, t3, send_result)) = encode_and_send {
                if let Err(e) = &send_result {
                    if is_device_gone(e) {
                        info!("TURZX {pid:04x} disconnected mid-stream, stopping worker");
                        break;
                    }
                    warn!("TURZX {pid:04x} send failed: {e:#}");
                }
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
    err.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<rusb::Error>(),
            Some(rusb::Error::NoDevice)
        )
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResolvedMode {
    width: u32,
    height: u32,
    refresh_hz: u32,
    /// DRM fourcc of the negotiated framebuffer layout.
    pixel_format: u32,
}

/// Packs a DRM fourcc code the way the kernel does, first character in the
/// lowest byte.
const fn fourcc(code: &[u8; 4]) -> u32 {
    (code[0] as u32) | ((code[1] as u32) << 8) | ((code[2] as u32) << 16) | ((code[3] as u32) << 24)
}

/// DRM_FORMAT_ABGR8888, memory layout R then G then B then A.
const DRM_FORMAT_ABGR8888: u32 = fourcc(b"AB24");
/// DRM_FORMAT_XBGR8888, memory layout R then G then B then X.
const DRM_FORMAT_XBGR8888: u32 = fourcc(b"XB24");
/// DRM_FORMAT_XRGB8888, memory layout B then G then R then X.
const DRM_FORMAT_XRGB8888: u32 = fourcc(b"XR24");
/// DRM_FORMAT_ARGB8888, memory layout B then G then R then A.
const DRM_FORMAT_ARGB8888: u32 = fourcc(b"AR24");

impl ResolvedMode {
    fn from_evdi(m: lianli_evdi::Mode) -> Result<Self> {
        if m.width <= 0 || m.height <= 0 {
            bail!("evdi mode has non-positive dimensions ({:?})", m);
        }
        // Every consumer of the framebuffer assumes four bytes per pixel in
        // one of the two channel orders. A compositor that negotiates
        // anything else would hand the encoders a buffer they cannot read
        // correctly, so refuse the mode instead of producing shifted colors
        // or reading past a smaller buffer.
        if m.bits_per_pixel != 32
            || !matches!(
                m.pixel_format,
                DRM_FORMAT_XRGB8888
                    | DRM_FORMAT_ARGB8888
                    | DRM_FORMAT_ABGR8888
                    | DRM_FORMAT_XBGR8888
            )
        {
            bail!(
                "evdi mode has unsupported format {:#x} at {} bpp",
                m.pixel_format,
                m.bits_per_pixel
            );
        }
        let refresh = m.refresh_hz.max(30) as u32;
        Ok(Self {
            width: m.width as u32,
            height: m.height as u32,
            refresh_hz: refresh,
            pixel_format: m.pixel_format,
        })
    }

    /// True when the negotiated layout stores red in the first byte of each
    /// pixel. AB24 and XB24 do, XR24 and AR24 store blue first. The encoders
    /// read the buffer raw, so they must be told which interpretation to
    /// use rather than assuming one.
    fn rgb_byte_order(&self) -> bool {
        matches!(self.pixel_format, DRM_FORMAT_ABGR8888 | DRM_FORMAT_XBGR8888)
    }
}
