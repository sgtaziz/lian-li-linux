//! Bounded execution for the external ffmpeg/ffprobe helpers.
//!
//! Media preparation runs on the daemon's main event loop, so a helper process
//! that never exits does not just fail one encode: it wedges the daemon for
//! good. Nothing after it in the loop runs again - no config reloads, so RGB
//! and fan changes are persisted but never applied, and no device polling, so
//! wireless devices that drop their binding are never re-bound.
//!
//! `std::process::Command::output()` waits without a deadline, so every helper
//! invocation goes through [`output_with_timeout`] instead.

use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Deadline for a one-shot transcode (a still image or short clip to H.264).
/// Generous: large sources on a slow CPU are legitimately slow.
pub(crate) const ENCODE_TIMEOUT: Duration = Duration::from_secs(120);

/// Deadline for a metadata probe, which reads headers and should return
/// almost immediately.
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

/// How often to check whether the child has exited.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Run `cmd` to completion, killing it if it outlives `timeout`.
///
/// Both pipes are drained on their own threads: a child that fills a pipe
/// buffer would otherwise block before it could ever reach the deadline.
///
/// On timeout the child is killed and reaped, so it cannot linger as an
/// orphan holding the pipes (or the GPU) open.
pub(crate) fn output_with_timeout(mut cmd: Command, timeout: Duration) -> Result<Output, TimedOut> {
    let program = cmd.get_program().to_string_lossy().into_owned();

    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|source| TimedOut::Spawn {
        program: program.clone(),
        source,
    })?;

    let mut child_stdout = child.stdout.take().expect("stdout was piped");
    let mut child_stderr = child.stderr.take().expect("stderr was piped");
    let stdout_reader = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = child_stdout.read_to_end(&mut buf);
        buf
    });
    let stderr_reader = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = child_stderr.read_to_end(&mut buf);
        buf
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    // SIGKILL: the whole point is that the child is wedged and
                    // may not be servicing signals it could catch.
                    let _ = child.kill();
                    let _ = child.wait();
                    // The pipes close as the child dies, so the readers finish.
                    let _ = stdout_reader.join();
                    let _ = stderr_reader.join();
                    return Err(TimedOut::Deadline { program, timeout });
                }
                thread::sleep(POLL_INTERVAL);
            }
            Err(source) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(TimedOut::Spawn { program, source });
            }
        }
    };

    Ok(Output {
        status,
        stdout: stdout_reader.join().unwrap_or_default(),
        stderr: stderr_reader.join().unwrap_or_default(),
    })
}

impl From<TimedOut> for crate::common::MediaError {
    fn from(err: TimedOut) -> Self {
        match err {
            // A helper that could not be started is an ordinary I/O failure,
            // as it was before this call became bounded.
            TimedOut::Spawn { source, .. } => Self::Io(source),
            deadline => Self::HelperTimedOut(deadline.to_string()),
        }
    }
}

/// Why a bounded run did not produce an exit status.
#[derive(Debug)]
pub(crate) enum TimedOut {
    /// The process could not be started, or could not be waited on.
    Spawn {
        program: String,
        source: std::io::Error,
    },
    /// The process outlived its deadline and was killed.
    Deadline { program: String, timeout: Duration },
}

impl std::fmt::Display for TimedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn { program, source } => write!(f, "could not run {program}: {source}"),
            Self::Deadline { program, timeout } => write!(
                f,
                "{program} did not exit within {}s and was killed",
                timeout.as_secs()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A child that exits normally still yields its output.
    #[test]
    fn returns_output_of_a_command_that_exits() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "printf out; printf err >&2; exit 3"]);

        let out = output_with_timeout(cmd, Duration::from_secs(30)).expect("should not time out");

        assert_eq!(out.status.code(), Some(3));
        assert_eq!(out.stdout, b"out");
        assert_eq!(out.stderr, b"err");
    }

    /// The regression this module exists for: a child that never exits must not
    /// block the caller forever.
    #[test]
    fn kills_a_child_that_never_exits() {
        let mut cmd = Command::new("sleep");
        cmd.arg("600");

        let started = Instant::now();
        let err = output_with_timeout(cmd, Duration::from_millis(300))
            .expect_err("a child that outlives the deadline must be reported");

        assert!(
            matches!(err, TimedOut::Deadline { .. }),
            "expected a deadline error, got {err:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "returned after {:?}; it should return at the deadline, not wait for the child",
            started.elapsed()
        );
    }

    /// A child that outlives the deadline while holding its pipes open (the
    /// shape of the hung ffmpeg) is still cut off at the deadline.
    #[test]
    fn kills_a_child_that_holds_its_pipes_open() {
        let mut cmd = Command::new("sh");
        // Writes, then hangs without closing stdout - so waiting for EOF on the
        // pipes, as `Command::output()` does, would never return.
        cmd.args(["-c", "printf partial; sleep 600"]);

        let started = Instant::now();
        let err = output_with_timeout(cmd, Duration::from_millis(300))
            .expect_err("a child holding its pipes open must still be cut off");

        assert!(matches!(err, TimedOut::Deadline { .. }), "got {err:?}");
        assert!(started.elapsed() < Duration::from_secs(20));
    }

    /// A missing binary is reported rather than treated as a timeout.
    #[test]
    fn reports_a_command_that_cannot_be_spawned() {
        let cmd = Command::new("lianli-no-such-binary-should-exist");

        let err = output_with_timeout(cmd, Duration::from_secs(30))
            .expect_err("spawning a missing binary must fail");

        assert!(matches!(err, TimedOut::Spawn { .. }), "got {err:?}");
    }
}
