use anyhow::{Context, Result};
use lianli_transport::usb::RusbBulk;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{debug, info, warn};

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
    stop: Option<&AtomicBool>,
    mut op: F,
) -> Result<R>
where
    F: FnMut(&RusbBulk) -> Result<R>,
{
    let first = {
        let handle = arc.lock();
        anyhow::ensure!(
            !stop.is_some_and(|flag| flag.load(Ordering::Acquire)),
            "wireless controller is stopping"
        );
        op(&handle)
    };
    match first {
        Ok(r) => Ok(r),
        Err(e) => {
            // Failures are expected once shutdown starts, since new
            // transfers are refused. Do not warn or attempt a reopen, the
            // caller is about to be joined anyway.
            if lianli_transport::usb::shutting_down()
                || stop.is_some_and(|flag| flag.load(Ordering::Acquire))
            {
                debug!("{name} transport op failed ({e}) while shutting down, not reopening");
                return Err(e).context("shutting down");
            }
            warn!("{name} transport op failed ({e}); attempting reopen");
            reopen_transport(arc, ids, name).context("reopen after stale handle")?;
            info!("{name} transport reopened, retrying");
            let handle = arc.lock();
            anyhow::ensure!(
                !stop.is_some_and(|flag| flag.load(Ordering::Acquire)),
                "wireless controller is stopping"
            );
            op(&handle)
        }
    }
}
