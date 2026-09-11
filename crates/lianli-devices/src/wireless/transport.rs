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
    let mut handle = arc.lock();
    retry_transport_operation(
        &mut *handle,
        stop,
        |handle| {
            anyhow::ensure!(
                !stop.load(Ordering::Acquire),
                "wireless controller is stopping"
            );
            op(handle)
        },
        |handle, error| {
            warn!("{name} transport op failed ({error}); attempting reopen");
            let replacement = open_any(ids).context(format!("reopening {name} dongle"))?;
            replace_transport(handle, replacement, |handle| {
                handle
                    .detach_and_configure_with_cancel(name, || stop.load(Ordering::Acquire))
                    .context("claiming replacement wireless transport")
            })?;
            info!("{name} transport reopened, retrying");
            Ok(())
        },
    )
}

fn replace_transport<T>(
    current: &mut T,
    replacement: T,
    claim: impl FnOnce(&mut T) -> Result<()>,
) -> Result<()> {
    drop(std::mem::replace(current, replacement));
    claim(current)
}

fn retry_transport_operation<T, R>(
    transport: &mut T,
    stop: &AtomicBool,
    mut operation: impl FnMut(&mut T) -> Result<R>,
    reopen: impl FnOnce(&mut T, &anyhow::Error) -> Result<()>,
) -> Result<R> {
    anyhow::ensure!(
        !stop.load(Ordering::Acquire),
        "wireless controller is stopping"
    );
    match operation(transport) {
        Ok(result) => Ok(result),
        Err(error) => {
            if lianli_transport::usb::shutting_down() || stop.load(Ordering::Acquire) {
                return Err(error).context("shutting down");
            }
            if !needs_reopen(&error) {
                return Err(error);
            }
            reopen(transport, &error)?;
            anyhow::ensure!(
                !stop.load(Ordering::Acquire),
                "wireless controller is stopping"
            );
            operation(transport)
        }
    }
}

fn needs_reopen(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<lianli_transport::TransportError>(),
        Some(lianli_transport::TransportError::Usb(
            rusb::Error::NoDevice | rusb::Error::Io | rusb::Error::Pipe
        ))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn replacement_releases_old_owner_before_claim_even_when_claim_fails() {
        struct Handle<'a>(&'a Cell<bool>);
        impl Drop for Handle<'_> {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }
        let old_dropped = Cell::new(false);
        let new_dropped = Cell::new(false);
        let mut handle = Handle(&old_dropped);
        let result = replace_transport(&mut handle, Handle(&new_dropped), |_| {
            assert!(old_dropped.get());
            assert!(!new_dropped.get());
            anyhow::bail!("interface unavailable")
        });
        assert!(result.is_err());
        assert!(!new_dropped.get());
        drop(handle);
        assert!(new_dropped.get());
    }

    #[test]
    fn protocol_errors_and_timeouts_do_not_reopen_transport() {
        for error in [
            anyhow::anyhow!("missing or invalid GetDev response"),
            lianli_transport::TransportError::Usb(rusb::Error::Timeout).into(),
        ] {
            let mut error = Some(error);
            let result: Result<()> = retry_transport_operation(
                &mut (),
                &AtomicBool::new(false),
                |_| Err(error.take().unwrap()),
                |_, _| panic!("response failure must not reopen a claimed interface"),
            );
            assert!(result.is_err());
        }
    }

    #[test]
    fn failed_reopen_never_retries_operation() {
        let attempts = Cell::new(0);
        let result: Result<()> = retry_transport_operation(
            &mut (),
            &AtomicBool::new(false),
            |_| {
                attempts.set(attempts.get() + 1);
                Err(lianli_transport::TransportError::Usb(rusb::Error::NoDevice).into())
            },
            |_, _| anyhow::bail!("replacement unavailable"),
        );
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("replacement unavailable"));
        assert_eq!(attempts.get(), 1);
    }

    #[test]
    fn stopped_controller_never_starts_an_operation() {
        let stop = AtomicBool::new(true);
        let result: Result<()> = retry_transport_operation(
            &mut (),
            &stop,
            |_| panic!("operation must not start"),
            |_, _| panic!("transport must not reopen"),
        );
        assert!(result.is_err());
    }

    #[test]
    fn stop_during_failed_operation_prevents_reopen() {
        let stop = AtomicBool::new(false);
        let result: Result<()> = retry_transport_operation(
            &mut (),
            &stop,
            |_| {
                stop.store(true, Ordering::Release);
                anyhow::bail!("receiver read failed")
            },
            |_, _| panic!("transport must not reopen after stop"),
        );
        assert!(result.unwrap_err().to_string().contains("shutting down"));
    }

    #[test]
    fn stop_during_reopen_prevents_retry() {
        let stop = AtomicBool::new(false);
        let attempts = Cell::new(0);
        let result: Result<()> = retry_transport_operation(
            &mut (),
            &stop,
            |_| {
                attempts.set(attempts.get() + 1);
                Err(lianli_transport::TransportError::Usb(rusb::Error::NoDevice).into())
            },
            |_, _| {
                stop.store(true, Ordering::Release);
                Ok(())
            },
        );
        assert!(result.is_err());
        assert!(stop.load(Ordering::Acquire));
        assert_eq!(attempts.get(), 1);
    }

    #[test]
    fn running_controller_reopens_once_and_returns_retry_result() {
        let stop = AtomicBool::new(false);
        let attempts = Cell::new(0);
        let reopens = Cell::new(0);
        let result = retry_transport_operation(
            &mut (),
            &stop,
            |_| {
                attempts.set(attempts.get() + 1);
                if attempts.get() == 1 {
                    return Err(lianli_transport::TransportError::Usb(rusb::Error::NoDevice).into());
                }
                Ok(42)
            },
            |_, _| {
                reopens.set(reopens.get() + 1);
                Ok(())
            },
        );
        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempts.get(), 2);
        assert_eq!(reopens.get(), 1);
    }
}
