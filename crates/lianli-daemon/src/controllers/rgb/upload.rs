use anyhow::{ensure, Result};
use lianli_devices::wireless::{WirelessController, WirelessRgbUpload};
use parking_lot::{Condvar, Mutex};
use std::collections::VecDeque;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::warn;

const MAX_PENDING_DEVICES: usize = 64;

#[derive(Clone)]
pub(super) enum Command {
    Upload(Arc<WirelessRgbUpload>),
    MotherboardSync(bool),
}

struct Job {
    wireless: Arc<WirelessController>,
    mac: [u8; 6],
    command: Command,
    attempts: u8,
    ready: Instant,
}

#[derive(Default)]
struct Pending {
    jobs: VecDeque<Job>,
    stopped: bool,
    generation: u64,
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
            let (mut job, generation) = {
                let (lock, wake) = &*shared;
                let mut pending = lock.lock();
                loop {
                    if pending.stopped {
                        return;
                    }
                    if let Some(index) = pending.jobs.iter().position(|j| j.ready <= Instant::now())
                    {
                        break (pending.jobs.remove(index).unwrap(), pending.generation);
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
            let result = match &job.command {
                Command::Upload(upload) => job.wireless.send_rgb_upload(
                    &job.mac,
                    upload,
                    if upload.frame_count() == 1 { 2 } else { 4 },
                ),
                Command::MotherboardSync(enabled) => {
                    job.wireless.set_mb_rgb_sync(&job.mac, *enabled)
                }
            };
            if let Err(error) = result {
                job.attempts += 1;
                let mut pending = shared.0.lock();
                if pending.stopped
                    || pending.generation != generation
                    || pending.jobs.iter().any(|j| j.mac == job.mac)
                {
                    continue;
                }
                if job.attempts < 3 && pending.jobs.len() < MAX_PENDING_DEVICES {
                    job.ready =
                        Instant::now() + Duration::from_millis(500 * u64::from(job.attempts));
                    pending.jobs.push_back(job);
                } else {
                    warn!(mac = ?job.mac, "Wireless RGB upload failed after {} attempts: {error}", job.attempts);
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
        let existing = pending.jobs.iter().position(|job| job.mac == mac);
        ensure!(
            existing.is_some() || pending.jobs.len() < MAX_PENDING_DEVICES,
            "RGB upload queue is full"
        );
        if let Some(index) = existing {
            pending.jobs.remove(index);
        }
        pending.jobs.push_back(Job {
            wireless,
            mac,
            command,
            attempts: 0,
            ready: Instant::now(),
        });
        self.pending.1.notify_one();
        Ok(())
    }

    pub(super) fn clear(&self) {
        let mut pending = self.pending.0.lock();
        pending.jobs.clear();
        pending.generation = pending.generation.wrapping_add(1);
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

impl Drop for UploadWorker {
    fn drop(&mut self) {
        self.stop();
    }
}
