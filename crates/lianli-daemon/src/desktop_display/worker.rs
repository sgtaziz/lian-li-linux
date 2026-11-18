use super::policy::{FramePacer, SharedVideoPolicy};
use super::session::{CaptureConnection, SessionClient};
use super::{DesktopDisplayHandle, TurzxDeviceMatch, HEALTHY_UPTIME};
use anyhow::{ensure, Context, Result};
use lianli_devices::turzx::{self, Mode as TurzxMode, TurzxDisplay, FMT_H264, FMT_MJPEG};
use lianli_display::buffer::EncodedBuffer;
use lianli_shared::display::{
    CaptureCommand, CaptureReply, DisplayCodec, DisplayMode, OutputRequest, MAX_ENCODED_BYTES,
};
use lianli_shared::ui_message::{UiError, UiMessage, UiText};
use std::os::fd::{AsFd, AsRawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

// These firmware revisions acknowledge H.264 but cannot reliably display it.
const JPEG_FORCE_PIDS: &[u16] = &[0xACD1, 0xAD11, 0xAD26];
// H.264 keyframes visibly pulse on this panel every 0.5 s, and it has the bandwidth for JPEG.
const JPEG_PREFERRED_PIDS: &[u16] = &[0xAD22];

pub(super) fn spawn_worker(
    target: TurzxDeviceMatch,
    video_policy: Arc<SharedVideoPolicy>,
    session: SessionClient,
    status: Arc<super::status::StreamStatus>,
) -> Result<DesktopDisplayHandle> {
    let pid = target.pid;
    let key = target.key;
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();
    let healthy = Arc::new(AtomicBool::new(false));
    let healthy_clone = healthy.clone();
    let join = thread::Builder::new()
        .name(format!("turzx-bridge-{pid:04x}"))
        .spawn(move || {
            status.starting();
            if let Err(error) = run_worker(
                target,
                &stop_clone,
                &healthy_clone,
                &video_policy,
                session,
                &status,
            ) {
                if stop_clone.load(Ordering::Relaxed) {
                    debug!("TURZX {pid:04x} at {key:?} worker stopped: {error:#}");
                } else {
                    status.failed(UiText::from_error(&error));
                    error!("TURZX {pid:04x} at {key:?} worker exited: {error:#}");
                }
            }
        })
        .context("Spawning desktop USB worker")?;
    Ok(DesktopDisplayHandle {
        stop,
        healthy,
        join: Some(join),
        pid,
    })
}

fn run_worker(
    target: TurzxDeviceMatch,
    stop: &AtomicBool,
    healthy: &AtomicBool,
    video_policy: &SharedVideoPolicy,
    session: SessionClient,
    status: &super::status::StreamStatus,
) -> Result<()> {
    let pid = target.pid;
    ensure!(session.ready(), "Waiting for the active desktop worker");
    let mut display = TurzxDisplay::open_device(target.device, || stop.load(Ordering::Relaxed))?;
    let caps = display.caps();
    let convert = |mode: TurzxMode| DisplayMode {
        width: u32::from(mode.width),
        height: u32::from(mode.height),
        refresh_hz: u32::from(mode.refresh_hz),
    };
    let jpeg_preferred = JPEG_PREFERRED_PIDS.contains(&pid) && caps.supports_mjpeg;
    let output = OutputRequest {
        edid: display.edid().to_vec(),
        preferred: convert(turzx::pick_mode(caps).context("Device advertises no modes")?),
        modes: caps.modes.iter().copied().map(convert).collect(),
        max_width: u32::from(caps.max_w),
        max_height: u32::from(caps.max_h),
        max_frame_bytes: jpeg_preferred
            .then(|| caps.max_transfer.checked_sub(turzx::FRAME_OVERHEAD as u32))
            .flatten()
            .filter(|bytes| *bytes > 0)
            .map(|bytes| bytes.min(MAX_ENCODED_BYTES as u32)),
    };
    let codec = if JPEG_FORCE_PIDS.contains(&pid) || jpeg_preferred {
        DisplayCodec::Jpeg
    } else {
        DisplayCodec::H264
    };
    let format = if codec == DisplayCodec::Jpeg {
        FMT_MJPEG
    } else {
        FMT_H264
    };
    let mut connection = session.open(output.clone(), codec, stop)?;
    let mut ready = await_ready(&mut connection, stop)?;
    let buffer = match ready.message {
        CaptureReply::Ready {
            backend,
            fallback_reason,
            buffer_bytes,
        } => {
            ensure!(
                ready.descriptors.len() == 1,
                "Capture worker did not provide one frame buffer"
            );
            info!("TURZX {pid:04x} connected to {backend} session capture");
            status.backend(&backend, fallback_reason.as_deref());
            if let Some(reason) = fallback_reason {
                info!("TURZX {pid:04x} desktop fallback: {reason}");
            }
            EncodedBuffer::from_descriptor(ready.descriptors.pop().unwrap(), buffer_bytes)?
        }
        CaptureReply::Failed { reason } => anyhow::bail!(UiError::new(
            UiMessage::new("status.desktop.setupFailed").param("reason", &reason),
            format!("Desktop setup failed: {reason}"),
        )),
        _ => anyhow::bail!("Unexpected display startup response"),
    };
    let mut requested = video_policy.load();
    let mut generation = 1u64;
    let mut sequence = 0u64;
    let mut pending: Option<(u64, Instant)> = None;
    let mut pause_pending = false;
    let mut paused = false;
    let mut streaming = false;
    let mut current_mode: Option<DisplayMode> = None;
    let mut pacer = FramePacer::new();
    let mut bytes = Vec::new();
    let mut first_frame = None;
    let mut reported_generation = None;
    let mut metrics = super::metrics::StreamMetrics::new();
    while !stop.load(Ordering::Relaxed) {
        let permitted = connection.permitted.load(Ordering::Acquire);
        let policy = video_policy.load();
        if policy != requested {
            requested = policy;
            generation = generation
                .checked_add(1)
                .context("Video policy generation exhausted")?;
            if let Some(mode) = current_mode {
                pacer.set_fps(policy.fps(mode.refresh_hz));
            }
            if pending.is_some() && !pause_pending {
                connection.channel.send(
                    &CaptureCommand::Pause,
                    &[],
                    Duration::from_millis(200),
                    stop,
                )?;
                pause_pending = true;
            }
        }
        if !permitted {
            if streaming {
                power_off(&mut display);
                streaming = false;
                status.paused();
                reported_generation = None;
            }
            if !paused && !pause_pending {
                connection.channel.send(
                    &CaptureCommand::Pause,
                    &[],
                    Duration::from_millis(200),
                    stop,
                )?;
                pause_pending = true;
            }
        }
        if let Some(reply) = connection.channel.try_receive::<CaptureReply>()? {
            ensure!(
                reply.descriptors.is_empty(),
                "Unexpected encoded-frame descriptors"
            );
            match reply.message {
                CaptureReply::Frame {
                    sequence: received,
                    generation: applied,
                    mode,
                    bytes: length,
                    encoding,
                } => {
                    ensure!(
                        pending.map(|(sequence, _)| sequence) == Some(received),
                        "Unexpected display frame sequence"
                    );
                    let request_elapsed = pending.take().unwrap().1.elapsed();
                    output.validate_mode(mode)?;
                    if !permitted || pause_pending || applied != generation {
                        continue;
                    }
                    // Copy before the next request grants the producer permission to reuse its storage.
                    buffer.read(length, &mut bytes)?;
                    if !connection.permitted.load(Ordering::Acquire) {
                        continue;
                    }
                    if !streaming || current_mode != Some(mode) {
                        display.start_streaming(
                            TurzxMode {
                                width: mode.width as u16,
                                height: mode.height as u16,
                                refresh_hz: mode.refresh_hz as u8,
                            },
                            format,
                        )?;
                        streaming = true;
                    }
                    current_mode = Some(mode);
                    pacer.set_fps(requested.fps(mode.refresh_hz));
                    request_frame(
                        &connection.channel,
                        &mut pacer,
                        &mut pending,
                        &mut sequence,
                        generation,
                        requested,
                        stop,
                    )?;
                    let usb_started = Instant::now();
                    let sent = if codec == DisplayCodec::Jpeg {
                        display.send_jpeg_frame(&bytes)
                    } else {
                        display.send_stream_a(&bytes)
                    };
                    // A missing H.264 packet invalidates subsequent dependent frames; restart cleanly.
                    sent.context("Sending desktop frame")?;
                    metrics.delivered(pid, bytes.len(), request_elapsed, usb_started.elapsed());
                    if reported_generation != Some((applied, encoding)) {
                        status.applied(applied, requested, encoding);
                        reported_generation = Some((applied, encoding));
                    }
                    let first = first_frame.get_or_insert_with(Instant::now);
                    if first.elapsed() >= HEALTHY_UPTIME {
                        healthy.store(true, Ordering::Release);
                    }
                }
                CaptureReply::Idle { sequence: received } => {
                    ensure!(
                        pending.map(|(sequence, _)| sequence) == Some(received),
                        "Unexpected empty display frame sequence"
                    );
                    pending = None;
                }
                CaptureReply::Paused => {
                    ensure!(pause_pending, "Unexpected display pause acknowledgement");
                    pending = None;
                    pause_pending = false;
                    paused = true;
                    status.paused();
                    reported_generation = None;
                }
                CaptureReply::Power { powered: false } => {
                    if streaming {
                        power_off(&mut display);
                        streaming = false;
                        status.paused();
                        reported_generation = None;
                    }
                }
                CaptureReply::Power { powered: true } => {}
                CaptureReply::Failed { reason } => {
                    anyhow::bail!("Session capture failed: {reason}")
                }
                CaptureReply::Ready { .. } => anyhow::bail!("Duplicate display startup response"),
            }
        }
        if permitted && !pause_pending {
            request_frame(
                &connection.channel,
                &mut pacer,
                &mut pending,
                &mut sequence,
                generation,
                requested,
                stop,
            )?;
            paused = false;
        }
        let wait = if permitted && !pause_pending && pending.is_none() {
            pacer
                .remaining(Instant::now())
                .min(Duration::from_millis(200))
        } else {
            Duration::from_millis(200)
        };
        let mut fd = libc::pollfd {
            fd: connection.channel.as_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let result = unsafe { libc::poll(&mut fd, 1, wait.as_millis().clamp(1, 200) as i32) };
        if result < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    power_off(&mut display);
    Ok(())
}

fn power_off(display: &mut TurzxDisplay) {
    if let Err(error) = lianli_transport::usb::with_teardown_io(Duration::from_millis(200), || {
        display.send_power_off()
    }) {
        warn!("TURZX power-off failed: {error:#}");
    }
}

fn await_ready(
    connection: &mut CaptureConnection,
    stop: &AtomicBool,
) -> Result<lianli_display::channel::Received<CaptureReply>> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        ensure!(!stop.load(Ordering::Relaxed), "Display startup cancelled");
        ensure!(
            connection.permitted.load(Ordering::Acquire),
            "Desktop session locked or disappeared during startup"
        );
        ensure!(
            Instant::now() < deadline,
            "Desktop capture startup timed out"
        );
        match connection.channel.receive(Duration::from_millis(50), stop) {
            Ok(reply) => {
                ensure!(
                    connection.permitted.load(Ordering::Acquire),
                    "Desktop session changed before capture was ready"
                );
                return Ok(reply);
            }
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::TimedOut) => {}
            Err(error) => return Err(error),
        }
    }
}

fn request_frame(
    channel: &lianli_display::channel::PacketChannel,
    pacer: &mut FramePacer,
    pending: &mut Option<(u64, Instant)>,
    sequence: &mut u64,
    generation: u64,
    policy: lianli_shared::display::DisplayVideoPolicy,
    stop: &AtomicBool,
) -> Result<()> {
    if pending.is_some() || !pacer.ready(Instant::now()) {
        return Ok(());
    }
    let next = sequence
        .checked_add(1)
        .context("Display frame sequence exhausted")?;
    let requested_at = Instant::now();
    channel.send(
        &CaptureCommand::Next {
            sequence: next,
            generation,
            policy,
        },
        &[],
        Duration::from_millis(200),
        stop,
    )?;
    pacer.started(Instant::now());
    *sequence = next;
    *pending = Some((next, requested_at));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_display::channel::PacketChannel;

    #[test]
    fn capture_overlap_keeps_one_request_in_flight_and_obeys_pacing() {
        let (daemon, mut session) = PacketChannel::pair().unwrap();
        let stop = AtomicBool::new(false);
        let mut pacer = FramePacer::new();
        pacer.set_fps(1);
        let mut pending = None;
        let mut sequence = 0;
        let policy = lianli_shared::display::DisplayVideoPolicy {
            hardware_video: true,
            fps_limit: 60,
        };
        request_frame(
            &daemon,
            &mut pacer,
            &mut pending,
            &mut sequence,
            7,
            policy,
            &stop,
        )
        .unwrap();
        let reply = session.try_receive::<CaptureCommand>().unwrap().unwrap();
        assert!(matches!(
            reply.message,
            CaptureCommand::Next {
                sequence: 1,
                generation: 7,
                ..
            }
        ));
        pacer.started(Instant::now() - Duration::from_secs(2));
        request_frame(
            &daemon,
            &mut pacer,
            &mut pending,
            &mut sequence,
            7,
            policy,
            &stop,
        )
        .unwrap();
        assert!(session.try_receive::<CaptureCommand>().unwrap().is_none());
        pending = None;
        pacer.started(Instant::now());
        request_frame(
            &daemon,
            &mut pacer,
            &mut pending,
            &mut sequence,
            7,
            policy,
            &stop,
        )
        .unwrap();
        assert!(session.try_receive::<CaptureCommand>().unwrap().is_none());
        pacer.started(Instant::now() - Duration::from_secs(2));
        request_frame(
            &daemon,
            &mut pacer,
            &mut pending,
            &mut sequence,
            8,
            policy,
            &stop,
        )
        .unwrap();
        let reply = session.try_receive::<CaptureCommand>().unwrap().unwrap();
        assert!(matches!(
            reply.message,
            CaptureCommand::Next {
                sequence: 2,
                generation: 8,
                ..
            }
        ));
    }

    #[test]
    fn a_ready_worker_cannot_authorize_capture_after_its_session_is_revoked() {
        let (helper, daemon) = PacketChannel::pair().unwrap();
        let stop = AtomicBool::new(false);
        helper
            .send(
                &CaptureReply::Ready {
                    backend: "test".into(),
                    fallback_reason: None,
                    buffer_bytes: 0,
                },
                &[],
                Duration::from_secs(1),
                &stop,
            )
            .unwrap();
        let mut connection = CaptureConnection {
            channel: daemon,
            permitted: Arc::new(AtomicBool::new(false)),
        };
        assert!(await_ready(&mut connection, &stop).is_err());
    }
}
