use lianli_devices::{traits::RgbDevice, wireless::WirelessController};
use parking_lot::{Condvar, Mutex};
use std::{
    sync::Arc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub(super) struct SyncClock(Mutex<(u32, Instant)>);

impl SyncClock {
    pub fn frame_index(&self, interval_hundredths: u32, frames: usize) -> usize {
        let (ticks, sampled) = *self.0.lock();
        let ticks = ticks.wrapping_add((sampled.elapsed().as_nanos() / 625_000) as u32);
        ((u64::from(ticks) * 100 / u64::from(interval_hundredths)) % frames as u64) as usize
    }
    fn ticks(&self) -> u32 {
        let (ticks, sampled) = *self.0.lock();
        ticks.wrapping_add((sampled.elapsed().as_nanos() / 625_000) as u32)
    }
}

#[derive(Default)]
struct State {
    stopped: bool,
    generation: u64,
    devices: Vec<Arc<dyn RgbDevice>>,
    wireless: Option<Arc<WirelessController>>,
}

pub(super) struct SyncClockWorker {
    state: Arc<(Mutex<State>, Condvar)>,
    pub clock: Arc<SyncClock>,
    thread: Option<JoinHandle<()>>,
}

impl SyncClockWorker {
    pub fn new() -> Self {
        let state = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let clock = Arc::new(SyncClock(Mutex::new((0, Instant::now()))));
        let shared = state.clone();
        let worker_clock = clock.clone();
        let thread = thread::spawn(move || {
            let mut failed = false;
            loop {
                let (devices, wireless, generation) = {
                    let mut state = shared.0.lock();
                    while !state.stopped && state.devices.is_empty() {
                        shared.1.wait(&mut state);
                    }
                    if state.stopped {
                        return;
                    }
                    (
                        state.devices.clone(),
                        state.wireless.clone(),
                        state.generation,
                    )
                };
                let mut error = None;
                if let Some(wireless) = wireless {
                    match wireless.read_rgb_clock() {
                        Ok(snapshot) if snapshot.0 != 0 => *worker_clock.0.lock() = snapshot,
                        Ok(_) => {}
                        Err(err) => error = Some(err),
                    }
                }
                for device in devices {
                    {
                        let state = shared.0.lock();
                        if state.stopped {
                            return;
                        }
                        if state.generation != generation {
                            break;
                        }
                    }
                    if let Err(err) = device.set_rgb_clock(worker_clock.ticks()) {
                        error = Some(err);
                    }
                }
                if let Some(error) = &error {
                    if !failed {
                        tracing::warn!(
                            "RGB synchronization clock update failed; will retry: {error:#}"
                        );
                    }
                }
                failed = error.is_some();
                let mut state = shared.0.lock();
                if state.stopped {
                    return;
                }
                if state.generation == generation {
                    shared.1.wait_for(&mut state, Duration::from_secs(3));
                }
            }
        });
        Self {
            state,
            clock,
            thread: Some(thread),
        }
    }
    pub fn configure(
        &self,
        devices: Vec<Arc<dyn RgbDevice>>,
        wireless: Option<Arc<WirelessController>>,
    ) {
        let mut state = self.state.0.lock();
        state.devices = devices;
        state.wireless = wireless;
        state.generation = state.generation.wrapping_add(1);
        self.state.1.notify_one();
    }
    pub fn clear(&self) {
        self.configure(Vec::new(), None);
    }
    pub fn stop(&mut self) {
        self.state.0.lock().stopped = true;
        self.state.1.notify_all();
        if let Some(thread) = self.thread.take() {
            if thread.join().is_err() {
                tracing::warn!("RGB synchronization clock worker panicked");
            }
        }
    }
}
impl Drop for SyncClockWorker {
    fn drop(&mut self) {
        self.stop();
    }
}
