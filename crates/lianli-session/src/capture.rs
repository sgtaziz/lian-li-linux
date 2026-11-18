use crate::encoding::{Encoder, FrameRequest};
use crate::watchdog::Watchdog;
use anyhow::{ensure, Context, Result};
use lianli_display::backend::LocalBackend;
use lianli_display::buffer::EncodedBuffer;
use lianli_display::channel::PacketChannel;
use lianli_display::{Event, OutputRequest};
use lianli_media::video::ensure_ffmpeg_initialized;
use lianli_shared::display::{
    CaptureCommand, CaptureReply, DisplayCodec, DisplayVideoPolicy, MAX_ENCODED_BYTES,
};
use std::os::fd::AsFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Default)]
struct FirstFrame {
    deadline: Option<Instant>,
    delivered: bool,
}

impl FirstFrame {
    fn check(&mut self, now: Instant, requested: bool) -> Result<()> {
        if requested && !self.delivered {
            let deadline = self.deadline.get_or_insert(now + Duration::from_secs(10));
            ensure!(
                now < *deadline,
                "Desktop capture did not deliver its first encoded frame within 10 seconds"
            );
        }
        Ok(())
    }

    fn complete(&mut self) {
        self.delivered = true;
        self.deadline = None;
    }
}

pub fn run(
    mut channel: PacketChannel,
    output: OutputRequest,
    codec: DisplayCodec,
    stop: &AtomicBool,
    watch: &Watchdog,
) {
    if let Err(error) = capture(&mut channel, output, codec, stop, watch) {
        if !stop.load(Ordering::Relaxed) {
            tracing::warn!("Session display stopped: {error:#}");
            let reason: String = format!("{error:#}").chars().take(2048).collect();
            if let Err(send_error) = channel.send(
                &CaptureReply::Failed { reason },
                &[],
                Duration::from_millis(100),
                stop,
            ) {
                tracing::debug!("Display failure receiver unavailable: {send_error}");
            }
        }
    }
}

fn capture(
    channel: &mut PacketChannel,
    output: OutputRequest,
    codec: DisplayCodec,
    stop: &AtomicBool,
    watch: &Watchdog,
) -> Result<()> {
    output.validate()?;
    let max_frame_bytes = output.max_frame_bytes;
    ensure_ffmpeg_initialized();
    let backend = LocalBackend::discover()?;
    let opened = backend.open(output, stop)?;
    let mut capture = opened.capture;
    let buffer = EncodedBuffer::create()?;
    channel.send(
        &CaptureReply::Ready {
            backend: opened.backend.into(),
            fallback_reason: opened.fallback_reason,
            buffer_bytes: MAX_ENCODED_BYTES,
        },
        &[buffer.as_fd()],
        Duration::from_secs(1),
        stop,
    )?;
    stream(
        channel,
        capture.as_mut(),
        codec,
        max_frame_bytes,
        &buffer,
        stop,
        watch,
    )
}

fn stream(
    channel: &mut PacketChannel,
    capture: &mut dyn lianli_display::Capture,
    codec: DisplayCodec,
    max_frame_bytes: Option<u32>,
    buffer: &EncodedBuffer,
    stop: &AtomicBool,
    watch: &Watchdog,
) -> Result<()> {
    let mut encoder = Encoder::default();
    let mut policy: Option<DisplayVideoPolicy> = None;
    let mut generation = 0;
    let mut pending = None;
    let mut frame_ready = false;
    let mut cached = false;
    let mut in_flight = false;
    let mut powered = true;
    let mut paused = false;
    let mut mode = None;
    let mut last_frame: Option<Instant> = None;
    let mut last_sequence = None;
    let mut first_frame = FirstFrame::default();
    while !stop.load(Ordering::Relaxed) {
        watch.progress(Instant::now());
        if let Some(command) = channel.try_receive::<CaptureCommand>()? {
            ensure!(
                command.descriptors.is_empty(),
                "Unexpected capture command descriptors"
            );
            match command.message {
                CaptureCommand::Close => return Ok(()),
                CaptureCommand::Pause => {
                    first_frame = FirstFrame::default();
                    pending = None;
                    encoder.reset(capture)?;
                    frame_ready = false;
                    cached = false;
                    in_flight = false;
                    paused = true;
                    capture.invalidate()?;
                    channel.send(&CaptureReply::Paused, &[], Duration::from_millis(100), stop)?;
                }
                CaptureCommand::Next {
                    sequence,
                    generation: next_generation,
                    policy: next_policy,
                } => {
                    ensure!(
                        pending.is_none(),
                        "Only one encoded frame may be outstanding"
                    );
                    ensure!(
                        last_sequence.is_none_or(|last| sequence > last),
                        "Display frame sequence did not advance"
                    );
                    ensure!(
                        (1..=120).contains(&next_policy.fps_limit),
                        "Invalid encoding frame limit"
                    );
                    ensure!(
                        next_generation >= generation,
                        "Display policy generation moved backwards"
                    );
                    last_sequence = Some(sequence);
                    paused = false;
                    if policy != Some(next_policy) || generation != next_generation {
                        first_frame = FirstFrame::default();
                        encoder.reset(capture)?;
                        frame_ready |= cached;
                    }
                    generation = next_generation;
                    policy = Some(next_policy);
                    pending = Some(sequence);
                }
            }
        }
        if paused || (pending.is_none() && powered) {
            wait_control(channel, Duration::from_millis(200))?;
            continue;
        }
        first_frame.check(Instant::now(), powered && pending.is_some())?;
        let fps = policy
            .map(|policy| {
                policy.fps(mode.map_or(
                    30,
                    |(mode, _): (
                        lianli_display::frame::Mode,
                        lianli_display::frame::PixelFormat,
                    )| mode.refresh_hz,
                ))
            })
            .unwrap_or(30);
        let interval = Duration::from_secs_f64(1.0 / f64::from(fps));
        let delay = last_frame
            .map(|last| (last + interval).saturating_duration_since(Instant::now()))
            .unwrap_or_default();
        if powered && frame_ready && pending.is_some() && !delay.is_zero() {
            wait_control(channel, delay.min(Duration::from_millis(200)))?;
            continue;
        }
        if powered && frame_ready && pending.is_some() {
            let sequence = pending.take().unwrap();
            frame_ready = false;
            let (current_mode, format) = mode.context("Frame arrived before its mode")?;
            last_frame = Some(Instant::now());
            let packet = encoder.encode(
                capture,
                FrameRequest {
                    codec,
                    mode: current_mode,
                    format,
                    fps,
                    hardware_video: policy.context("Missing encoder policy")?.hardware_video,
                    max_frame_bytes,
                },
                stop,
            )?;
            if packet.is_empty() {
                channel.send(
                    &CaptureReply::Idle { sequence },
                    &[],
                    Duration::from_millis(100),
                    stop,
                )?;
            } else {
                buffer.write(&packet)?;
                channel.send(
                    &CaptureReply::Frame {
                        sequence,
                        generation,
                        mode: current_mode,
                        bytes: packet.len(),
                        encoding: encoder.status(),
                    },
                    &[],
                    Duration::from_millis(100),
                    stop,
                )?;
                first_frame.complete();
            }
            continue;
        }
        if powered && pending.is_some() && !frame_ready && !in_flight && mode.is_some() {
            frame_ready = capture.request_update()?;
            in_flight = !frame_ready;
            cached |= frame_ready;
            if frame_ready {
                continue;
            }
        }
        for event in capture.poll_events(Duration::from_millis(200), stop)? {
            match event {
                Event::ModeChanged(next_mode, format) => {
                    first_frame.delivered = false;
                    mode = Some((next_mode, format));
                    encoder.reset(capture)?;
                    cached = false;
                    frame_ready = false;
                    in_flight = false;
                }
                Event::FrameReady if mode.is_some() => {
                    frame_ready = true;
                    cached = true;
                    in_flight = false;
                }
                Event::FrameReady => {}
                Event::PowerChanged(next) if next != powered => {
                    first_frame = FirstFrame::default();
                    powered = next;
                    encoder.reset(capture)?;
                    frame_ready = false;
                    cached = false;
                    in_flight = false;
                    capture.invalidate()?;
                    channel.send(
                        &CaptureReply::Power { powered },
                        &[],
                        Duration::from_millis(100),
                        stop,
                    )?;
                }
                Event::PowerChanged(_) => {}
            }
        }
        if powered && pending.is_some() && !frame_ready && !in_flight && mode.is_some() {
            frame_ready = capture.request_update()?;
            in_flight = !frame_ready;
            cached |= frame_ready;
        }
    }
    Ok(())
}

fn wait_control(channel: &PacketChannel, duration: Duration) -> Result<()> {
    use std::os::fd::AsRawFd;
    let mut fd = libc::pollfd {
        fd: channel.as_fd().as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let result = unsafe { libc::poll(&mut fd, 1, duration.as_millis().clamp(1, 200) as i32) };
    if result < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_display::frame::{Frame, Mode, PixelFormat};
    use lianli_display::Capture;

    #[test]
    fn first_frame_timeout_bounds_startup_without_expiring_static_playback() {
        let now = Instant::now();
        let mut startup = FirstFrame::default();
        startup.check(now, false).unwrap();
        let requested = now + Duration::from_secs(60);
        startup.check(requested, true).unwrap();
        startup
            .check(requested + Duration::from_secs(9), true)
            .unwrap();
        assert!(startup
            .check(requested + Duration::from_secs(10), true)
            .is_err());
        startup.complete();
        startup
            .check(requested + Duration::from_secs(3600), true)
            .unwrap();
        startup.delivered = false;
        let changed = requested + Duration::from_secs(3601);
        startup.check(changed, true).unwrap();
        assert!(startup
            .check(changed + Duration::from_secs(10), true)
            .is_err());
    }

    struct StillFrame {
        pixels: Vec<u8>,
        fresh: bool,
        request_driven: bool,
        update_pending: bool,
    }

    impl Capture for StillFrame {
        fn poll_events(&mut self, timeout: Duration, _: &AtomicBool) -> Result<Vec<Event>> {
            if std::mem::take(&mut self.fresh) {
                self.update_pending = false;
                Ok(vec![
                    Event::ModeChanged(mode(), PixelFormat::Xrgb8888),
                    Event::FrameReady,
                ])
            } else {
                ensure!(
                    !self.request_driven,
                    "Waited before requesting the next frame"
                );
                std::thread::sleep(timeout);
                Ok(Vec::new())
            }
        }
        fn request_update(&mut self) -> Result<bool> {
            ensure!(!self.update_pending, "Repeated a pending capture request");
            self.update_pending = true;
            self.fresh |= self.request_driven;
            Ok(false)
        }
        fn frame(&mut self, _: &AtomicBool) -> Result<Frame<'_>> {
            Frame::new(mode(), PixelFormat::Xrgb8888, 16, &self.pixels)
        }
        fn invalidate(&mut self) -> Result<()> {
            self.update_pending = false;
            self.fresh = true;
            Ok(())
        }
    }

    fn mode() -> Mode {
        Mode {
            width: 4,
            height: 4,
            refresh_hz: 30,
        }
    }

    #[test]
    fn waits_for_mode_before_requesting_or_encoding_early_updates() {
        struct DelayedMode {
            inner: StillFrame,
            polls: u8,
        }
        impl Capture for DelayedMode {
            fn invalidate(&mut self) -> Result<()> {
                self.inner.invalidate()
            }
            fn poll_events(&mut self, timeout: Duration, stop: &AtomicBool) -> Result<Vec<Event>> {
                self.polls += 1;
                match self.polls {
                    1 => Ok(vec![Event::FrameReady]),
                    2 => Ok(Vec::new()),
                    _ => self.inner.poll_events(timeout, stop),
                }
            }
            fn request_update(&mut self) -> Result<bool> {
                ensure!(self.polls >= 3, "Requested pixels before mode negotiation");
                self.inner.request_update()
            }
            fn frame(&mut self, stop: &AtomicBool) -> Result<Frame<'_>> {
                ensure!(self.polls >= 3, "Read pixels before mode negotiation");
                self.inner.frame(stop)
            }
        }
        let (mut daemon, mut worker) = PacketChannel::pair().unwrap();
        let stop = AtomicBool::new(false);
        let thread = std::thread::spawn(move || {
            stream(
                &mut worker,
                &mut DelayedMode {
                    inner: StillFrame {
                        pixels: vec![128; 64],
                        fresh: true,
                        request_driven: false,
                        update_pending: false,
                    },
                    polls: 0,
                },
                DisplayCodec::Jpeg,
                None,
                &EncodedBuffer::create().unwrap(),
                &AtomicBool::new(false),
                &Watchdog::new(),
            )
        });
        daemon
            .send(
                &CaptureCommand::Next {
                    sequence: 1,
                    generation: 1,
                    policy: DisplayVideoPolicy {
                        hardware_video: false,
                        fps_limit: 30,
                    },
                },
                &[],
                Duration::from_secs(1),
                &stop,
            )
            .unwrap();
        let reply = daemon.receive::<CaptureReply>(Duration::from_secs(2), &stop);
        let _ = daemon.send(&CaptureCommand::Close, &[], Duration::from_secs(1), &stop);
        thread.join().unwrap().unwrap();
        assert!(matches!(
            reply.unwrap().message,
            CaptureReply::Frame { sequence: 1, .. }
        ));
    }

    #[test]
    fn frames_are_demand_driven_and_pause_discards_an_outstanding_request() {
        let (mut daemon, mut worker) = PacketChannel::pair().unwrap();
        let storage = EncodedBuffer::create().unwrap();
        let reader = EncodedBuffer::from_descriptor(
            storage.as_fd().try_clone_to_owned().unwrap(),
            MAX_ENCODED_BYTES,
        )
        .unwrap();
        let stop = AtomicBool::new(false);
        let thread = std::thread::spawn(move || {
            stream(
                &mut worker,
                &mut StillFrame {
                    pixels: vec![128; 64],
                    fresh: true,
                    request_driven: false,
                    update_pending: false,
                },
                DisplayCodec::Jpeg,
                None,
                &storage,
                &AtomicBool::new(false),
                &Watchdog::new(),
            )
        });
        let policy = DisplayVideoPolicy {
            hardware_video: false,
            fps_limit: 30,
        };
        let send = |daemon: &PacketChannel, command| {
            daemon
                .send(&command, &[], Duration::from_secs(1), &stop)
                .unwrap()
        };
        send(
            &daemon,
            CaptureCommand::Next {
                sequence: 1,
                generation: 1,
                policy,
            },
        );
        let frame = daemon
            .receive::<CaptureReply>(Duration::from_secs(2), &stop)
            .unwrap();
        let CaptureReply::Frame {
            sequence: 1,
            generation: 1,
            bytes,
            encoding,
            ..
        } = frame.message
        else {
            panic!("Expected encoded frame")
        };
        let mut jpeg = Vec::new();
        assert_eq!(
            encoding.unwrap().encoder,
            lianli_shared::display::DesktopEncoder::Turbojpeg
        );
        reader.read(bytes, &mut jpeg).unwrap();
        let image = turbojpeg::decompress(&jpeg, turbojpeg::PixelFormat::RGB).unwrap();
        assert_eq!((image.width, image.height), (4, 4));
        assert!(daemon.try_receive::<CaptureReply>().unwrap().is_none());
        send(
            &daemon,
            CaptureCommand::Next {
                sequence: 2,
                generation: 1,
                policy,
            },
        );
        let idle = daemon
            .receive::<CaptureReply>(Duration::from_millis(450), &stop)
            .err()
            .expect("Static capture must wait for damage");
        assert_eq!(
            idle.downcast_ref::<std::io::Error>()
                .map(std::io::Error::kind),
            Some(std::io::ErrorKind::TimedOut)
        );
        send(&daemon, CaptureCommand::Pause);
        let paused = daemon
            .receive::<CaptureReply>(Duration::from_secs(2), &stop)
            .unwrap();
        assert!(matches!(paused.message, CaptureReply::Paused));
        send(
            &daemon,
            CaptureCommand::Next {
                sequence: 3,
                generation: 2,
                policy,
            },
        );
        let resumed = daemon
            .receive::<CaptureReply>(Duration::from_secs(2), &stop)
            .unwrap();
        assert!(matches!(
            resumed.message,
            CaptureReply::Frame {
                sequence: 3,
                generation: 2,
                ..
            }
        ));
        send(&daemon, CaptureCommand::Close);
        thread.join().unwrap().unwrap();
    }

    #[test]
    fn next_frame_is_requested_before_waiting_for_capture_events() {
        let (mut daemon, mut worker) = PacketChannel::pair().unwrap();
        let stop = AtomicBool::new(false);
        let thread = std::thread::spawn(move || {
            stream(
                &mut worker,
                &mut StillFrame {
                    pixels: vec![128; 64],
                    fresh: true,
                    request_driven: true,
                    update_pending: false,
                },
                DisplayCodec::Jpeg,
                None,
                &EncodedBuffer::create().unwrap(),
                &AtomicBool::new(false),
                &Watchdog::new(),
            )
        });
        for sequence in 1..=2 {
            daemon
                .send(
                    &CaptureCommand::Next {
                        sequence,
                        generation: 1,
                        policy: DisplayVideoPolicy {
                            hardware_video: false,
                            fps_limit: 30,
                        },
                    },
                    &[],
                    Duration::from_secs(1),
                    &stop,
                )
                .unwrap();
            let reply = daemon
                .receive::<CaptureReply>(Duration::from_secs(2), &stop)
                .unwrap();
            assert!(
                matches!(reply.message, CaptureReply::Frame { sequence: received, .. } if received == sequence)
            );
        }
        daemon
            .send(&CaptureCommand::Close, &[], Duration::from_secs(1), &stop)
            .unwrap();
        thread.join().unwrap().unwrap();
    }
}
