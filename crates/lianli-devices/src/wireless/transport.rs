use anyhow::{Context, Result};
use lianli_transport::usb::RusbBulk;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{info, warn};

/// Try to open a USB device matching any of the given VID:PID pairs.
pub(super) fn open_any(ids: &[(u16, u16)]) -> Result<RusbBulk> {
    let mut last_err = None;
    for &(vid, pid) in ids {
        match RusbBulk::open(vid, pid) {
            Ok(transport) => return Ok(transport),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err
        .map(|e| anyhow::anyhow!(e))
        .unwrap_or_else(|| anyhow::anyhow!("no VID:PID pairs to try")))
}

/// Reopen and swap a dongle transport in place after the underlying USB
/// handle goes stale (suspend/resume, hub reset, unplug+replug).
pub(super) fn reopen_transport(
    arc: &Arc<Mutex<RusbBulk>>,
    ids: &[(u16, u16)],
    name: &str,
) -> Result<()> {
    let mut new_transport = open_any(ids).context(format!("reopening {name} dongle"))?;
    new_transport.detach_and_configure(name)?;
    let mut guard = arc.lock();
    *guard = new_transport;
    Ok(())
}

/// Run a USB op on a dongle transport with one-shot reopen + retry on failure.
/// `op` must be safe to call twice (idempotent at the protocol level).
pub(super) fn with_transport_recovery<F, R>(
    arc: &Arc<Mutex<RusbBulk>>,
    ids: &[(u16, u16)],
    name: &str,
    stop: &AtomicBool,
    mut op: F,
) -> Result<R>
where
    F: FnMut(&RusbBulk) -> Result<R>,
{
    retry_transport_operation(
        stop,
        || {
            let handle = arc.lock();
            anyhow::ensure!(
                !stop.load(Ordering::Acquire),
                "wireless controller is stopping"
            );
            op(&handle)
        },
        |error| {
            warn!("{name} transport op failed ({error}); attempting reopen");
            reopen_transport(arc, ids, name).context("reopen after stale handle")?;
            info!("{name} transport reopened, retrying");
            Ok(())
        },
    )
}

fn retry_transport_operation<R>(
    stop: &AtomicBool,
    mut operation: impl FnMut() -> Result<R>,
    reopen: impl FnOnce(&anyhow::Error) -> Result<()>,
) -> Result<R> {
    anyhow::ensure!(
        !stop.load(Ordering::Acquire),
        "wireless controller is stopping"
    );
    match operation() {
        Ok(result) => Ok(result),
        Err(error) => {
            if lianli_transport::usb::SHUTTING_DOWN.load(Ordering::Relaxed)
                || stop.load(Ordering::Acquire)
            {
                return Err(error).context("shutting down");
            }
            reopen(&error)?;
            anyhow::ensure!(
                !stop.load(Ordering::Acquire),
                "wireless controller is stopping"
            );
            operation()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn stopped_controller_never_starts_an_operation() {
        let stop = AtomicBool::new(true);
        let result: Result<()> = retry_transport_operation(
            &stop,
            || panic!("operation must not start"),
            |_| panic!("transport must not reopen"),
        );
        assert!(result.is_err());
    }

    #[test]
    fn stop_during_failed_operation_prevents_reopen() {
        let stop = AtomicBool::new(false);
        let result: Result<()> = retry_transport_operation(
            &stop,
            || {
                stop.store(true, Ordering::Release);
                anyhow::bail!("receiver read failed")
            },
            |_| panic!("transport must not reopen after stop"),
        );
        assert!(result.unwrap_err().to_string().contains("shutting down"));
    }

    #[test]
    fn stop_during_reopen_prevents_retry() {
        let stop = AtomicBool::new(false);
        let attempts = Cell::new(0);
        let result: Result<()> = retry_transport_operation(
            &stop,
            || {
                attempts.set(attempts.get() + 1);
                anyhow::bail!("receiver read failed")
            },
            |_| {
                stop.store(true, Ordering::Release);
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(attempts.get(), 1);
    }

    #[test]
    fn running_controller_reopens_once_and_returns_retry_result() {
        let stop = AtomicBool::new(false);
        let attempts = Cell::new(0);
        let reopens = Cell::new(0);
        let result = retry_transport_operation(
            &stop,
            || {
                attempts.set(attempts.get() + 1);
                if attempts.get() == 1 {
                    anyhow::bail!("receiver read failed");
                }
                Ok(42)
            },
            |_| {
                reopens.set(reopens.get() + 1);
                Ok(())
            },
        );
        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempts.get(), 2);
        assert_eq!(reopens.get(), 1);
    }
}
