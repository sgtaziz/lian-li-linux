use std::time::{Duration, Instant};

const SAMPLE_TOLERANCE: Duration = Duration::from_millis(25);
const RESUME_THRESHOLD: Duration = Duration::from_millis(500);
const USB_SETTLE: Duration = Duration::from_secs(2);

/// Reports a resume on the caller's own thread. The service's resume event
/// waits for USB to settle, which is too late for the temperature fallback.
pub(crate) struct ResumeWatch {
    offset: Option<Duration>,
}

impl ResumeWatch {
    pub(crate) fn new() -> Self {
        Self {
            offset: suspend_offset(),
        }
    }

    pub(crate) fn resumed(&mut self) -> bool {
        self.observe(suspend_offset())
    }

    fn observe(&mut self, offset: Option<Duration>) -> bool {
        let Some(offset) = offset else {
            return false;
        };
        let previous = self.offset.replace(offset);
        previous.is_some_and(|previous| offset.saturating_sub(previous) >= RESUME_THRESHOLD)
    }
}

pub(super) struct ResumeDetector {
    offset: Option<Duration>,
    ready_at: Option<Instant>,
}

impl ResumeDetector {
    pub(super) fn new() -> Self {
        Self {
            offset: suspend_offset(),
            ready_at: None,
        }
    }

    pub(super) fn poll(&mut self) -> bool {
        self.observe(suspend_offset(), Instant::now())
    }

    fn observe(&mut self, offset: Option<Duration>, now: Instant) -> bool {
        if let Some(offset) = offset {
            if let Some(previous) = self.offset {
                if offset.saturating_sub(previous) >= RESUME_THRESHOLD {
                    tracing::info!("System resume detected; allowing USB devices to settle");
                    self.ready_at = Some(now + USB_SETTLE);
                }
            }
            self.offset = Some(offset);
        }
        if self.ready_at.is_some_and(|deadline| now >= deadline) {
            self.ready_at = None;
            return true;
        }
        false
    }
}

fn clock_time(clock: libc::clockid_t) -> Option<Duration> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // clock_gettime writes only to this initialized timespec.
    if unsafe { libc::clock_gettime(clock, &mut time) } != 0 {
        return None;
    }
    Some(Duration::new(
        time.tv_sec.try_into().ok()?,
        time.tv_nsec.try_into().ok()?,
    ))
}

fn suspend_offset() -> Option<Duration> {
    let before = clock_time(libc::CLOCK_MONOTONIC)?;
    let boot = clock_time(libc::CLOCK_BOOTTIME)?;
    let after = clock_time(libc::CLOCK_MONOTONIC)?;
    sample_offset(before, boot, after)
}

fn sample_offset(before: Duration, boot: Duration, after: Duration) -> Option<Duration> {
    let span = after.checked_sub(before)?;
    if span > SAMPLE_TOLERANCE {
        return None;
    }
    Some(boot.saturating_sub(before + span / 2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_watch_reports_each_suspend_once_and_ignores_bad_samples() {
        let mut watch = ResumeWatch {
            offset: Some(Duration::ZERO),
        };
        assert!(!watch.observe(Some(Duration::from_millis(100))));
        assert!(!watch.observe(None));
        assert!(watch.observe(Some(Duration::from_secs(60))));
        assert!(!watch.observe(Some(Duration::from_secs(60))));
    }

    #[test]
    fn suspend_is_detected_when_monotonic_time_excludes_it() {
        let now = Instant::now();
        let mut detector = ResumeDetector {
            offset: Some(Duration::ZERO),
            ready_at: None,
        };
        let offset = sample_offset(
            Duration::from_secs(10),
            Duration::from_secs(70),
            Duration::from_secs(10),
        );
        assert!(!detector.observe(offset, now));
        assert!(!detector.observe(offset, now + Duration::from_secs(1)));
        assert!(detector.observe(offset, now + USB_SETTLE));
        assert!(!detector.observe(offset, now + USB_SETTLE));
    }

    #[test]
    fn busy_service_and_interrupted_sampling_do_not_look_like_suspend() {
        let now = Instant::now();
        let mut detector = ResumeDetector {
            offset: Some(Duration::ZERO),
            ready_at: None,
        };
        assert!(!detector.observe(
            sample_offset(
                Duration::from_secs(90),
                Duration::from_secs(90),
                Duration::from_secs(90)
            ),
            now + Duration::from_secs(90)
        ));
        assert_eq!(
            sample_offset(
                Duration::ZERO,
                Duration::from_secs(4),
                Duration::from_secs(4)
            ),
            None
        );
        assert!(!detector.observe(None, now + Duration::from_secs(100)));
    }

    #[test]
    fn repeated_resume_extends_one_pending_recovery() {
        let now = Instant::now();
        let mut detector = ResumeDetector {
            offset: Some(Duration::ZERO),
            ready_at: None,
        };
        assert!(!detector.observe(Some(Duration::from_secs(10)), now));
        assert!(!detector.observe(Some(Duration::from_secs(20)), now + Duration::from_secs(1)));
        assert!(!detector.observe(None, now + Duration::from_secs(2)));
        assert!(detector.observe(None, now + Duration::from_secs(3)));
        assert!(!detector.observe(None, now + Duration::from_secs(4)));
    }
}
