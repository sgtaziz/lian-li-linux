use super::h264::{
    encoder_chain, encoder_codec_args_live, finalize_vf, hwaccel_input_args, EncoderKind,
};
use crate::common::MediaError;
use lianli_shared::screen::ScreenInfo;
use parking_lot::Mutex;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::io::AsRawFd;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

const MAX_DIAGNOSTIC_LINE: usize = 2048;
const MAX_PROBE_LINES: usize = 8;
const DIAGNOSTIC_INTERVAL: Duration = Duration::from_secs(5);

fn live_frame_bytes(
    width: u32,
    height: u32,
    rotation: u16,
    screen: &ScreenInfo,
) -> Result<usize, MediaError> {
    let output = match rotation {
        0 | 180 => (width, height),
        90 | 270 => (height, width),
        _ => {
            return Err(MediaError::InvalidConfig(
                "H.264 rotation must be 0, 90, 180 or 270 degrees".into(),
            ))
        }
    };
    if !screen.h264 || output != (screen.width, screen.height) || width == 0 || height == 0 {
        return Err(MediaError::InvalidConfig(
            "Live H.264 dimensions must match a supported panel after rotation".into(),
        ));
    }
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .filter(|bytes| *bytes <= 64 * 1024 * 1024)
        .ok_or_else(|| {
            MediaError::InvalidConfig("Live H.264 frame exceeds the 64 MiB limit".into())
        })
}

struct PendingEncoder(Option<Child>);

impl Drop for PendingEncoder {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            if let Err(error) = child.wait() {
                warn!("Failed to reap rejected FFmpeg encoder: {error}");
            }
        }
    }
}

fn read_diagnostic_line(
    reader: &mut impl BufRead,
    line: &mut Vec<u8>,
    consumed: &mut usize,
) -> std::io::Result<bool> {
    line.clear();
    let mut had_input = false;
    loop {
        let bytes = reader.fill_buf()?;
        if bytes.is_empty() {
            return Ok(had_input);
        }
        had_input = true;
        let end = bytes.iter().position(|byte| *byte == b'\n');
        let count = end.map_or(bytes.len(), |index| index + 1);
        let retained = count.min(MAX_DIAGNOSTIC_LINE - line.len());
        line.extend_from_slice(&bytes[..retained]);
        reader.consume(count);
        *consumed += count;
        if *consumed >= 64 * 1024 {
            thread::sleep(Duration::from_millis(10));
            *consumed = 0;
        }
        if end.is_some() {
            return Ok(true);
        }
    }
}

#[derive(Default)]
struct DiagnosticRate {
    next: Option<Instant>,
    suppressed: u64,
}

impl DiagnosticRate {
    fn observe(&mut self, now: Instant) -> Option<u64> {
        if self.next.is_some_and(|next| now < next) {
            self.suppressed = self.suppressed.saturating_add(1);
            return None;
        }
        self.next = Some(now + DIAGNOSTIC_INTERVAL);
        Some(std::mem::take(&mut self.suppressed))
    }
}

fn retain_probe_line(lines: &mut Vec<String>, line: String) -> bool {
    let failed = line.contains("Error while opening encoder")
        || line.contains("Could not open encoder")
        || line.contains("Failed to initialise")
        || line.contains("Device creation failed");
    if failed {
        lines.clear();
    }
    if lines.len() < MAX_PROBE_LINES {
        lines.push(line);
    }
    failed
}

/// Try to grow ffmpeg's stdin pipe buffer so a full RGBA frame fits in a single
/// kernel-side buffer. With the default 64 KB pipe size, a 3.7 MB frame
/// fragments into ~58 write() syscalls and ffmpeg rate-limits the writer to
/// pipe-empty cadence; a larger pipe lets us write the whole frame in one syscall
/// and queue ahead by a frame, smoothing out the encoder. We try to fit two
/// frames; if the kernel caps us (EPERM at fs.pipe-max-size, default 1 MB), we
/// settle for whatever size we can get and continue.
fn grow_pipe(fd: i32, frame_bytes: usize) {
    let want = (frame_bytes * 2).max(1 << 22) as libc::c_int;
    let n = unsafe { libc::fcntl(fd, libc::F_SETPIPE_SZ, want) };
    if n < 0 {
        let err = std::io::Error::last_os_error();
        debug!(
            "F_SETPIPE_SZ {} failed: {err}; pipe will use default size",
            want
        );
    } else {
        debug!("pipe buffer sized to {} bytes", n);
    }
}

/// A frame starts at a slice NAL (type 1 or 5) whose first_mb_in_slice ue(v) is 0, a leading 1 bit.
#[derive(Default)]
struct FrameStarts {
    zeros: u8,
    state: NalState,
}

#[derive(Default, Clone, Copy)]
enum NalState {
    #[default]
    Scan,
    Header,
    SliceStart,
}

impl FrameStarts {
    fn count(&mut self, bytes: &[u8]) -> u64 {
        let mut frames = 0;
        for &byte in bytes {
            match self.state {
                NalState::Header => {
                    self.state = if matches!(byte & 0x1f, 1 | 5) {
                        NalState::SliceStart
                    } else {
                        NalState::Scan
                    };
                    self.zeros = 0;
                    continue;
                }
                NalState::SliceStart => {
                    frames += u64::from(byte & 0x80 != 0);
                    self.state = NalState::Scan;
                }
                NalState::Scan => {}
            }
            if byte == 0 {
                self.zeros = self.zeros.saturating_add(1);
            } else {
                if byte == 1 && self.zeros >= 2 {
                    self.state = NalState::Header;
                }
                self.zeros = 0;
            }
        }
        frames
    }
}

struct EncoderBacklog {
    written: AtomicU64,
    read: AtomicU64,
}

impl EncoderBacklog {
    fn new() -> Self {
        Self {
            // The startup probe frame.
            written: AtomicU64::new(1),
            read: AtomicU64::new(0),
        }
    }

    /// None until a frame start is seen, so unparsed output never throttles the renderer.
    fn queued(&self) -> Option<u64> {
        let read = self.read.load(Ordering::Acquire);
        (read > 0).then(|| self.written.load(Ordering::Relaxed).saturating_sub(read))
    }
}

fn backlog_limit(fps: u32) -> u64 {
    u64::from(fps / 2).max(3)
}

pub struct LiveH264Output {
    stdout: ChildStdout,
    backlog: Arc<EncoderBacklog>,
    frames: FrameStarts,
}

impl Read for LiveH264Output {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.stdout.read(buf)?;
        let frames = self.frames.count(&buf[..n]);
        if frames > 0 {
            self.backlog.read.fetch_add(frames, Ordering::Release);
        }
        Ok(n)
    }
}

/// Long-running ffmpeg subprocess that consumes raw RGBA frames on stdin and
/// emits a continuous H.264 NAL stream on stdout. Used by Custom mode for live
/// h264 streaming on devices that accept it.
pub struct LiveH264Encoder {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Option<ChildStdout>,
    frame_bytes: usize,
    kind: EncoderKind,
    backlog: Arc<EncoderBacklog>,
    backlog_limit: u64,
    dropped_log: DiagnosticRate,
}

impl LiveH264Encoder {
    pub fn encoder_status(&self, hardware_video: bool) -> lianli_shared::ipc::MediaEncoderStatus {
        self.kind.status(hardware_video)
    }
    pub fn spawn(
        width: u32,
        height: u32,
        fps: f32,
        rotation_deg: u16,
        screen: &ScreenInfo,
        hardware_video: bool,
    ) -> Result<Self, MediaError> {
        let frame_bytes = live_frame_bytes(width, height, rotation_deg, screen)?;
        let fps_int = super::h264::frame_rate(fps, screen);
        let bitrate = super::h264::bitrate(width, height, fps_int);
        let bitrate_str = format!("{bitrate}");
        let fps_str = fps_int.to_string();
        let size_str = format!("{width}x{height}");
        let transpose = match rotation_deg {
            90 => Some("transpose=1"),
            180 => Some("transpose=1,transpose=1"),
            270 => Some("transpose=2"),
            _ => None,
        };

        let mut last_err: Option<String> = None;
        for kind in encoder_chain(hardware_video) {
            match try_spawn(
                *kind,
                &size_str,
                &fps_str,
                &bitrate_str,
                transpose,
                frame_bytes,
            ) {
                Ok(mut encoder) => {
                    info!(
                        "live H.264 encoder: {width}x{height}@{fps_int}fps via {}",
                        kind.name()
                    );
                    encoder.backlog_limit = backlog_limit(fps_int);
                    return Ok(encoder);
                }
                Err(e) => {
                    warn!("h264 encoder {} unavailable, trying next: {e}", kind.name());
                    last_err = Some(e);
                }
            }
        }

        Err(MediaError::Ffmpeg(format!(
            "All live H.264 encoders failed. Last error: {}",
            last_err.unwrap_or_default()
        )))
    }

    /// Push one raw RGBA frame into the encoder, or drop it while the consumer
    /// lags. Returns Err on broken pipe (encoder died) so the caller can tear down.
    pub fn write_frame(&mut self, rgba: &[u8]) -> Result<(), MediaError> {
        if rgba.len() != self.frame_bytes {
            return Err(MediaError::Ffmpeg(format!(
                "frame size mismatch: got {} bytes, expected {}",
                rgba.len(),
                self.frame_bytes
            )));
        }
        if let Some(queued) = self
            .backlog
            .queued()
            .filter(|queued| *queued >= self.backlog_limit)
        {
            if let Some(earlier) = self.dropped_log.observe(Instant::now()) {
                debug!(
                    "live H.264 consumer is {queued} frames behind; dropped {} frames",
                    earlier + 1
                );
            }
            return Ok(());
        }
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| MediaError::Ffmpeg("encoder stdin already closed".into()))?;
        write_pipe(stdin, rgba, Duration::from_secs(2))
            .map_err(|e| MediaError::Ffmpeg(format!("write_frame: {e}")))?;
        self.backlog.written.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Hand the encoder's stdout to the streaming consumer. Callable once.
    pub fn take_stdout(&mut self) -> Option<LiveH264Output> {
        self.stdout.take().map(|stdout| LiveH264Output {
            stdout,
            backlog: Arc::clone(&self.backlog),
            frames: FrameStarts::default(),
        })
    }
}

fn write_pipe(
    pipe: &mut (impl Write + AsRawFd),
    mut bytes: &[u8],
    timeout: Duration,
) -> std::io::Result<()> {
    let deadline = Instant::now() + timeout;
    while !bytes.is_empty() {
        if Instant::now() >= deadline {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        match pipe.write(bytes) {
            Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
            Ok(written) => bytes = &bytes[written..],
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                let mut descriptor = libc::pollfd {
                    fd: pipe.as_raw_fd(),
                    events: libc::POLLOUT,
                    revents: 0,
                };
                let wait = deadline.saturating_duration_since(Instant::now());
                let millis = wait.as_millis().clamp(1, 50) as i32;
                // The descriptor remains owned by pipe throughout this bounded wait.
                if unsafe { libc::poll(&mut descriptor, 1, millis) } < 0 {
                    let error = std::io::Error::last_os_error();
                    if error.kind() != std::io::ErrorKind::Interrupted {
                        return Err(error);
                    }
                }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

impl Drop for LiveH264Encoder {
    fn drop(&mut self) {
        // Closing stdin signals EOF to ffmpeg, which then flushes and exits.
        drop(self.stdin.take());
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => {
                    warn!("ffmpeg wait failed: {e}");
                    break;
                }
            }
        }
        if let Err(e) = self.child.kill() {
            warn!("ffmpeg kill failed: {e}");
        }
        let _ = self.child.wait();
    }
}

fn try_spawn(
    kind: EncoderKind,
    size_str: &str,
    fps_str: &str,
    bitrate_str: &str,
    transpose: Option<&str>,
    frame_bytes: usize,
) -> Result<LiveH264Encoder, String> {
    let mut args: Vec<String> = vec!["-loglevel".into(), "error".into()];
    args.extend(hwaccel_input_args(kind));
    args.extend([
        "-f".into(),
        "rawvideo".into(),
        "-pix_fmt".into(),
        "rgba".into(),
        "-s".into(),
        size_str.into(),
        "-r".into(),
        fps_str.into(),
        "-i".into(),
        "pipe:0".into(),
    ]);
    let base_vf = transpose.unwrap_or("");
    let vf = finalize_vf(kind, base_vf);
    if !vf.is_empty() {
        args.extend(["-vf".into(), vf]);
    }
    args.extend(encoder_codec_args_live(kind, fps_str, bitrate_str));
    args.extend(["-color_range".into(), "pc".into()]);
    args.extend(["-an".into(), "-f".into(), "h264".into(), "pipe:1".into()]);

    debug!("live h264 ffmpeg args: {}", args.join(" "));
    let child = Command::new("ffmpeg")
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn ffmpeg: {e}"))?;
    let mut pending = PendingEncoder(Some(child));
    let child = pending.0.as_mut().unwrap();

    std::thread::sleep(Duration::from_millis(200));
    if let Ok(Some(status)) = child.try_wait() {
        let mut err_buf = String::new();
        if let Some(stderr) = child.stderr.take() {
            use std::io::Read;
            let _ = stderr
                .take((MAX_DIAGNOSTIC_LINE * MAX_PROBE_LINES) as u64)
                .read_to_string(&mut err_buf);
        }
        return Err(format!(
            "ffmpeg exited early ({status}): {}",
            err_buf.trim()
        ));
    }

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "ffmpeg stdin missing".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "ffmpeg stdout missing".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "ffmpeg stderr missing".to_string())?;

    grow_pipe(stdin.as_raw_fd(), frame_bytes);
    grow_pipe(stdout.as_raw_fd(), 1 << 19);
    // A stalled USB consumer must not trap the renderer in an encoder pipe write.
    let flags = unsafe { libc::fcntl(stdin.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe { libc::fcntl(stdin.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        let error = std::io::Error::last_os_error();
        return Err(format!("nonblocking encoder stdin: {error}"));
    }

    let probing = Arc::new(AtomicBool::new(true));
    let open_failed = Arc::new(AtomicBool::new(false));
    let probe_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let probing_clone = Arc::clone(&probing);
    let open_failed_clone = Arc::clone(&open_failed);
    let probe_log_clone = Arc::clone(&probe_log);
    let kind_name = kind.name();
    thread::Builder::new()
        .name("h264-diagnostics".into())
        .spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut bytes = Vec::with_capacity(MAX_DIAGNOSTIC_LINE);
            let mut rate = DiagnosticRate::default();
            let mut consumed = 0;
            loop {
                match read_diagnostic_line(&mut reader, &mut bytes, &mut consumed) {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(error) => {
                        debug!("ffmpeg[{kind_name}] diagnostic pipe closed: {error}");
                        break;
                    }
                }
                let line = String::from_utf8_lossy(&bytes).trim().to_owned();
                if line.is_empty() {
                    continue;
                }
                if probing_clone.load(Ordering::Relaxed) {
                    if retain_probe_line(&mut probe_log_clone.lock(), line) {
                        open_failed_clone.store(true, Ordering::Relaxed);
                    }
                } else if let Some(suppressed) = rate.observe(Instant::now()) {
                    warn!(suppressed, "ffmpeg[{kind_name}]: {line}");
                }
            }
        })
        .map_err(|error| format!("start encoder diagnostics: {error}"))?;

    let probe = vec![0u8; frame_bytes];
    if let Err(e) = write_pipe(&mut stdin, &probe, Duration::from_secs(2)) {
        return Err(format!("probe write: {e}"));
    }

    let probe_deadline = Instant::now() + Duration::from_millis(2000);
    loop {
        if open_failed.load(Ordering::Relaxed) {
            let summary = probe_log
                .lock()
                .iter()
                .find(|l| {
                    !l.contains("Task finished")
                        && !l.contains("Terminating thread")
                        && !l.contains("Nothing was written")
                })
                .cloned()
                .unwrap_or_else(|| format!("{kind_name} encoder open failed"));
            return Err(summary);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                return Err(format!("ffmpeg exited during probe ({status})"));
            }
            Ok(None) => {
                if Instant::now() >= probe_deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("try_wait: {e}")),
        }
    }

    probing.store(false, Ordering::Relaxed);

    Ok(LiveH264Encoder {
        child: pending.0.take().unwrap(),
        stdin: Some(stdin),
        stdout: Some(stdout),
        frame_bytes,
        kind,
        backlog: Arc::new(EncoderBacklog::new()),
        backlog_limit: u64::MAX,
        dropped_log: DiagnosticRate::default(),
    })
}

#[cfg(test)]
mod pipe_tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::net::UnixStream;

    #[test]
    fn live_geometry_validates_rotated_panel_layout_and_allocation_before_spawn() {
        let screen = ScreenInfo::HYDROSHIFT2_OLED_CURVE;
        for rotation in [0, 90, 180, 270] {
            let (width, height) = if rotation % 180 == 0 {
                (screen.width, screen.height)
            } else {
                (screen.height, screen.width)
            };
            assert_eq!(
                live_frame_bytes(width, height, rotation, &screen).unwrap(),
                screen.width as usize * screen.height as usize * 4
            );
        }
        for (width, height, rotation) in [
            (0, 0, 0),
            (screen.height, screen.width, 0),
            (screen.width, screen.height, 45),
            (screen.width + 1, screen.height, 0),
        ] {
            assert!(live_frame_bytes(width, height, rotation, &screen).is_err());
        }
        let unsupported = ScreenInfo {
            h264: false,
            ..screen
        };
        assert!(live_frame_bytes(screen.width, screen.height, 0, &unsupported).is_err());
        let excessive = ScreenInfo {
            width: u32::MAX,
            height: u32::MAX,
            ..screen
        };
        assert!(live_frame_bytes(excessive.width, excessive.height, 0, &excessive).is_err());
        let boundary = ScreenInfo {
            width: 4096,
            height: 4096,
            ..screen
        };
        assert_eq!(
            live_frame_bytes(4096, 4096, 0, &boundary).unwrap(),
            64 * 1024 * 1024
        );
        let excessive = ScreenInfo {
            width: 4097,
            ..boundary
        };
        assert!(live_frame_bytes(4097, 4096, 0, &excessive).is_err());
    }

    #[test]
    fn rejected_encoder_is_reaped_and_success_transfers_child_ownership() {
        let child = Command::new("sleep").arg("30").spawn().unwrap();
        let pid = child.id() as libc::pid_t;
        drop(PendingEncoder(Some(child)));
        let mut status = 0;
        // The fixture was our child and the guard must already have reaped it.
        assert_eq!(
            unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );

        let child = Command::new("sleep").arg("30").spawn().unwrap();
        let mut pending = PendingEncoder(Some(child));
        let mut accepted = pending.0.take().unwrap();
        drop(pending);
        let running = accepted.try_wait().unwrap().is_none();
        accepted.kill().unwrap();
        accepted.wait().unwrap();
        assert!(running);
    }

    #[test]
    fn oversized_lines_are_drained_without_hiding_the_next_error() {
        let mut input = vec![b'x'; 256 * 1024];
        input.extend_from_slice(b"\nDevice creation failed\nlast line");
        let mut reader = BufReader::with_capacity(37, std::io::Cursor::new(input));
        let mut line = Vec::new();
        let mut consumed = 0;
        assert!(read_diagnostic_line(&mut reader, &mut line, &mut consumed).unwrap());
        assert_eq!(line, vec![b'x'; MAX_DIAGNOSTIC_LINE]);
        assert!(read_diagnostic_line(&mut reader, &mut line, &mut consumed).unwrap());
        assert_eq!(line, b"Device creation failed\n");
        assert!(read_diagnostic_line(&mut reader, &mut line, &mut consumed).unwrap());
        assert_eq!(line, b"last line");
        assert!(!read_diagnostic_line(&mut reader, &mut line, &mut consumed).unwrap());
        assert!(line.is_empty());
    }

    #[test]
    fn startup_retention_stays_bounded_and_prioritizes_encoder_failures() {
        let mut lines = Vec::new();
        for _ in 0..100 {
            assert!(!retain_probe_line(&mut lines, "startup detail".into()));
        }
        assert_eq!(lines.len(), MAX_PROBE_LINES);
        assert!(retain_probe_line(
            &mut lines,
            "Device creation failed".into()
        ));
        assert_eq!(lines, ["Device creation failed"]);
    }

    #[test]
    fn runtime_diagnostics_report_suppression_without_per_line_logging() {
        let now = Instant::now();
        let mut rate = DiagnosticRate::default();
        assert_eq!(rate.observe(now), Some(0));
        for _ in 0..1000 {
            assert_eq!(rate.observe(now + Duration::from_secs(1)), None);
        }
        assert_eq!(rate.observe(now + DIAGNOSTIC_INTERVAL), Some(1000));
        assert_eq!(rate.observe(now + 2 * DIAGNOSTIC_INTERVAL), Some(0));
    }

    #[test]
    fn stalled_consumer_times_out_and_healthy_consumer_receives_exact_bytes() {
        let (mut writer, mut reader) = UnixStream::pair().unwrap();
        writer.set_nonblocking(true).unwrap();
        let payload = vec![0x5a; 4 * 1024 * 1024];
        let started = Instant::now();
        assert_eq!(
            write_pipe(&mut writer, &payload, Duration::from_millis(50))
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::TimedOut
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(writer);
        let mut partial = Vec::new();
        reader.read_to_end(&mut partial).unwrap();
        assert!(!partial.is_empty() && partial.len() < payload.len());

        let (mut writer, mut reader) = UnixStream::pair().unwrap();
        writer.set_nonblocking(true).unwrap();
        let consumer = thread::spawn(move || {
            let mut actual = Vec::new();
            reader.read_to_end(&mut actual).unwrap();
            actual
        });
        write_pipe(&mut writer, &payload, Duration::from_secs(2)).unwrap();
        drop(writer);
        assert_eq!(consumer.join().unwrap(), payload);
    }
}

#[cfg(test)]
mod backlog_tests {
    use super::*;

    fn annex_b_stream() -> Vec<u8> {
        [
            &[0, 0, 0, 1, 0x67, 0x42, 0x00, 0x1f][..],
            &[0, 0, 0, 1, 0x68, 0xce, 0x38, 0x80],
            &[0, 0, 1, 0x06, 0x05, 0x10],
            &[0, 0, 0, 1, 0x65, 0x88, 0x84, 0x00, 0x21],
            &[0, 0, 1, 0x65, 0x40, 0x11],
            &[0, 0, 0, 1, 0x41, 0x9a, 0x02],
            &[0, 0, 1, 0x01, 0x80, 0x00, 0x00, 0x03, 0x01],
        ]
        .concat()
    }

    #[test]
    fn counts_one_start_per_frame_across_any_read_split() {
        let stream = annex_b_stream();
        for chunk in 1..=stream.len() {
            let mut starts = FrameStarts::default();
            let frames: u64 = stream.chunks(chunk).map(|part| starts.count(part)).sum();
            assert_eq!(frames, 3, "chunk size {chunk}");
        }
    }

    #[test]
    fn backlog_throttles_only_after_the_consumer_reads_a_frame() {
        let backlog = EncoderBacklog::new();
        backlog.written.store(40, Ordering::Relaxed);
        assert_eq!(backlog.queued(), None);
        backlog.read.store(25, Ordering::Release);
        assert_eq!(backlog.queued(), Some(15));
        assert!(backlog.queued().unwrap() >= backlog_limit(30));
        assert!(backlog.queued().unwrap() < backlog_limit(60));
        assert_eq!(backlog_limit(1), 3);
    }
}
