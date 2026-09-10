use anyhow::{ensure, Result};
use lianli_devices::wireless::{WirelessController, WirelessRgbUpload};
use parking_lot::{Condvar, Mutex};
use std::collections::VecDeque;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{debug, warn};

const MAX_PENDING_DEVICES: usize = 64;
const MB_SYNC_WAIT_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone)]
pub(super) enum Command {
    Upload(Arc<WirelessRgbUpload>),
    MotherboardSync(bool),
}

struct Job {
    wireless: Arc<WirelessController>,
    mac: [u8; 6],
    command: Command,
    attempts: u32,
    ready: Instant,
}

struct Inflight {
    id: u64,
    mac: [u8; 6],
    command: Command,
}

#[derive(Default)]
struct Pending {
    jobs: VecDeque<Job>,
    inflight: Option<Inflight>,
    stopped: bool,
    generation: u64,
    next_id: u64,
}

impl Pending {
    fn submit_job(&mut self, job: Job) -> Result<bool> {
        let mac = job.mac;
        let existing = self.jobs.iter().position(|queued| queued.mac == mac);
        if existing.is_some_and(|index| same_command(&self.jobs[index].command, &job.command)) {
            return Ok(false);
        }
        if let Some(index) = existing {
            self.jobs.remove(index);
        }
        if self.inflight.as_ref().is_some_and(|inflight| {
            inflight.mac == mac && same_command(&inflight.command, &job.command)
        }) {
            return Ok(false);
        }
        ensure!(
            existing.is_some()
                || self
                    .inflight
                    .as_ref()
                    .is_some_and(|inflight| inflight.mac == mac)
                || self.jobs.len() + usize::from(self.inflight.is_some()) < MAX_PENDING_DEVICES,
            "RGB upload queue is full"
        );
        self.jobs.push_back(job);
        Ok(true)
    }

    fn clear(&mut self) {
        self.jobs.clear();
        self.generation = self.generation.wrapping_add(1);
        self.inflight = None;
    }
}

pub(super) struct UploadWorker {
    pending: Arc<(Mutex<Pending>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}

impl UploadWorker {
    pub(super) fn new() -> Self {
        let pending = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
        let shared = pending.clone();
        let thread = thread::spawn(move || loop {
            let (mut job, generation, id) = {
                let (lock, wake) = &*shared;
                let mut pending = lock.lock();
                loop {
                    if pending.stopped {
                        return;
                    }
                    if let Some(index) = pending.jobs.iter().position(|j| j.ready <= Instant::now())
                    {
                        let job = pending.jobs.remove(index).unwrap();
                        pending.next_id = pending.next_id.wrapping_add(1).max(1);
                        let id = pending.next_id;
                        pending.inflight = Some(Inflight {
                            id,
                            mac: job.mac,
                            command: job.command.clone(),
                        });
                        break (job, pending.generation, id);
                    }
                    if let Some(deadline) = pending.jobs.iter().map(|j| j.ready).min() {
                        wake.wait_for(
                            &mut pending,
                            deadline.saturating_duration_since(Instant::now()),
                        );
                    } else {
                        wake.wait(&mut pending);
                    }
                }
            };
            let already_applied = match &job.command {
                Command::Upload(upload) => job
                    .wireless
                    .rgb_upload_applied(&job.mac, upload.effect_index()),
                Command::MotherboardSync(_) => false,
            };
            let (result, waiting_for_mb_sync) = if already_applied {
                (Ok(()), false)
            } else {
                match &job.command {
                    Command::Upload(upload) => match job.wireless.mb_rgb_ready_for_upload(&job.mac)
                    {
                        Ok(true) => (job.wireless.send_rgb_upload(&job.mac, upload, 4), false),
                        Ok(false) => (Ok(()), true),
                        Err(error) => (Err(error), false),
                    },
                    Command::MotherboardSync(enabled) => {
                        (job.wireless.set_mb_rgb_sync(&job.mac, *enabled), false)
                    }
                }
            };
            let observed = if result.is_ok() && !waiting_for_mb_sync {
                match &job.command {
                    Command::Upload(upload) => job
                        .wireless
                        .rgb_upload_applied(&job.mac, upload.effect_index()),
                    Command::MotherboardSync(_) => true,
                }
            } else {
                false
            };
            let mut pending = shared.0.lock();
            if pending
                .inflight
                .as_ref()
                .is_some_and(|inflight| inflight.id == id)
            {
                pending.inflight = None;
            }
            if pending.stopped || pending.generation != generation {
                if let Command::Upload(upload) = &job.command {
                    job.wireless
                        .forget_rgb_target(&job.mac, upload.effect_index());
                }
                continue;
            }
            if waiting_for_mb_sync {
                if pending.jobs.iter().any(|queued| queued.mac == job.mac) {
                    continue;
                }
                job.ready = Instant::now() + MB_SYNC_WAIT_INTERVAL;
                pending.jobs.push_back(job);
                continue;
            }
            if let Err(error) = result {
                if job.attempts == 0 {
                    warn!(mac = ?job.mac, "Wireless RGB transfer failed; will retry: {error}");
                }
                job.attempts = job.attempts.saturating_add(1);
                if pending.stopped
                    || pending.generation != generation
                    || pending.jobs.iter().any(|j| j.mac == job.mac)
                {
                    continue;
                }
                if pending.jobs.len() < MAX_PENDING_DEVICES {
                    let backoff_ms = 500u64.saturating_mul(u64::from(job.attempts).min(20));
                    job.ready = Instant::now() + Duration::from_millis(backoff_ms);
                    pending.jobs.push_back(job);
                } else {
                    warn!(mac = ?job.mac, "Wireless RGB upload retry deferred because queue is full: {error}");
                }
            } else if !observed && matches!(&job.command, Command::Upload(_)) {
                if pending.stopped
                    || pending.generation != generation
                    || pending.jobs.iter().any(|j| j.mac == job.mac)
                {
                    continue;
                }
                job.attempts = 0;
                if pending.jobs.len() < MAX_PENDING_DEVICES {
                    job.ready = Instant::now() + Duration::from_millis(30);
                    pending.jobs.push_back(job);
                }
            }
        });
        Self {
            pending,
            thread: Some(thread),
        }
    }

    // Success means the latest desired state is queued; newer commands for
    // the same device supersede pending work, never an in-flight transaction.
    pub(super) fn submit(
        &self,
        wireless: Arc<WirelessController>,
        mac: [u8; 6],
        command: Command,
    ) -> Result<()> {
        let mut pending = self.pending.0.lock();
        ensure!(!pending.stopped, "RGB uploader is stopped");
        let effect_index = match &command {
            Command::Upload(upload) => Some(upload.effect_index()),
            Command::MotherboardSync(_) => None,
        };
        let changed = pending.submit_job(Job {
            wireless,
            mac,
            command,
            attempts: 0,
            ready: Instant::now(),
        })?;
        if changed {
            debug!(mac = ?mac, effect_index = ?effect_index, "Queued wireless RGB state change");
        }
        self.pending.1.notify_one();
        Ok(())
    }

    pub(super) fn clear(&self) {
        let mut pending = self.pending.0.lock();
        pending.clear();
    }

    pub(super) fn stop(&mut self) {
        {
            let mut pending = self.pending.0.lock();
            pending.stopped = true;
            pending.jobs.clear();
            self.pending.1.notify_one();
        }
        if let Some(thread) = self.thread.take() {
            if thread.join().is_err() {
                warn!("RGB upload worker panicked");
            }
        }
    }
}

fn same_command(left: &Command, right: &Command) -> bool {
    match (left, right) {
        (Command::Upload(a), Command::Upload(b)) => a == b,
        (Command::MotherboardSync(a), Command::MotherboardSync(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{same_command, Command, Inflight, Job, Pending, MAX_PENDING_DEVICES};
    use lianli_devices::wireless::WirelessController;
    use lianli_devices::wireless::WirelessRgbUpload;
    use std::sync::Arc;

    #[test]
    fn identical_uploads_are_deduplicated() {
        let upload = Arc::new(WirelessRgbUpload::new(&[vec![[1, 2, 3]]], 50, None).unwrap());
        assert!(same_command(
            &Command::Upload(upload.clone()),
            &Command::Upload(upload)
        ));
    }

    #[test]
    fn different_commands_are_not_deduplicated() {
        assert!(!same_command(
            &Command::MotherboardSync(true),
            &Command::MotherboardSync(false)
        ));
    }

    fn job(wireless: &std::sync::Arc<WirelessController>, mac: [u8; 6], value: u8) -> Job {
        Job {
            wireless: wireless.clone(),
            mac,
            command: Command::MotherboardSync(value != 0),
            attempts: 0,
            ready: std::time::Instant::now(),
        }
    }

    #[test]
    fn pending_newer_command_supersedes_stale_queue_entry() {
        let wireless = std::sync::Arc::new(WirelessController::new());
        let mut pending = Pending::default();
        pending.submit_job(job(&wireless, [1; 6], 0)).unwrap();
        pending.submit_job(job(&wireless, [1; 6], 1)).unwrap();
        assert_eq!(pending.jobs.len(), 1);
        assert!(matches!(
            pending.jobs[0].command,
            Command::MotherboardSync(true)
        ));
    }

    #[test]
    fn inflight_duplicate_removes_older_pending_entry() {
        let wireless = std::sync::Arc::new(WirelessController::new());
        let mut pending = Pending::default();
        pending.submit_job(job(&wireless, [1; 6], 0)).unwrap();
        pending.inflight = Some(Inflight {
            id: 1,
            mac: [1; 6],
            command: Command::MotherboardSync(true),
        });
        pending.submit_job(job(&wireless, [1; 6], 1)).unwrap();
        assert!(pending.jobs.is_empty());
    }

    #[test]
    fn inflight_counts_against_queue_capacity() {
        let wireless = std::sync::Arc::new(WirelessController::new());
        let mut pending = Pending::default();
        for mac in 0..MAX_PENDING_DEVICES - 1 {
            pending
                .submit_job(job(&wireless, [mac as u8; 6], 0))
                .unwrap();
        }
        pending.inflight = Some(Inflight {
            id: 1,
            mac: [250; 6],
            command: Command::MotherboardSync(true),
        });
        assert!(pending.submit_job(job(&wireless, [251; 6], 0)).is_err());
        pending.submit_job(job(&wireless, [250; 6], 0)).unwrap();
        assert_eq!(pending.jobs.back().unwrap().mac, [250; 6]);
    }

    #[test]
    fn clear_cancels_inflight_deduplication_and_queued_work() {
        let wireless = std::sync::Arc::new(WirelessController::new());
        let mut pending = Pending::default();
        pending.submit_job(job(&wireless, [1; 6], 1)).unwrap();
        pending.inflight = Some(Inflight {
            id: 1,
            mac: [2; 6],
            command: Command::MotherboardSync(true),
        });
        let generation = pending.generation;
        pending.clear();
        assert!(pending.jobs.is_empty());
        assert!(pending.inflight.is_none());
        assert_ne!(pending.generation, generation);
    }
}

impl Drop for UploadWorker {
    fn drop(&mut self) {
        self.stop();
    }
}
