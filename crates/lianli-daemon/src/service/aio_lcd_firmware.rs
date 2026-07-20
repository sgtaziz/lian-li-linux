//! AIO LCD firmware bookkeeping extracted from `ServiceManager`.
//!
//! Three small maps that always move together: the daemon schedules deferred
//! firmware reads when an AIO LCD attaches, processes them on a later poll,
//! and remembers which devices previously failed so they aren't retried
//! forever.

use std::collections::HashMap;
use std::time::Instant;

/// Per-device AIO LCD firmware state.
///
/// - `info` — populated when a deferred firmware read succeeds.
/// - `pending` — devices with a scheduled future firmware read.
/// - `skip` — devices whose firmware read previously failed; suppress retries.
#[derive(Default)]
pub struct AioLcdFirmwareTracker {
    pub info: HashMap<String, (Option<String>, bool)>,
    pub pending: HashMap<String, (Instant, bool)>,
    pub skip: HashMap<String, Instant>,
}

impl AioLcdFirmwareTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Schedule a deferred firmware read for `device_id` after `delay`.
    pub fn schedule(
        &mut self,
        device_id: impl Into<String>,
        delay: std::time::Duration,
        enable_512: bool,
    ) {
        self.pending
            .insert(device_id.into(), (Instant::now() + delay, enable_512));
    }

    /// `true` if `device_id` previously failed a firmware read and should be skipped.
    pub fn should_skip(&self, device_id: &str) -> bool {
        self.skip.contains_key(device_id)
    }

    /// Mark `device_id` as failed; further attempts will be skipped.
    pub fn mark_failed(&mut self, device_id: impl Into<String>) {
        self.skip.insert(device_id.into(), Instant::now());
    }

    /// Record a successful firmware read.
    pub fn record(
        &mut self,
        device_id: impl Into<String>,
        firmware: Option<String>,
        supports_c_command: bool,
    ) {
        self.info
            .insert(device_id.into(), (firmware, supports_c_command));
    }

    /// Look up the firmware + C-command support for a device.
    pub fn get(&self, device_id: &str) -> Option<(Option<String>, bool)> {
        self.info.get(device_id).cloned()
    }

    /// Drain the entries whose deadline has passed, returning
    /// `(device_id, enable_512)` pairs ready to be processed.
    pub fn drain_due(&mut self) -> Vec<(String, bool)> {
        let now = Instant::now();
        let due: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, (deadline, _))| *deadline <= now)
            .map(|(id, _)| id.clone())
            .collect();
        due.into_iter()
            .map(|id| {
                let (_, enable_512) = self.pending.remove(&id).unwrap();
                (id, enable_512)
            })
            .collect()
    }
}
