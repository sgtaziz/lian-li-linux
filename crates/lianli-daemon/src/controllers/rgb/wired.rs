use anyhow::{ensure, Result};
use lianli_devices::traits::{RgbDevice, RgbFrameDelivery};
use lianli_shared::rgb::RgbPlaybackTiming;
use parking_lot::{Condvar, Mutex};
use std::collections::HashMap;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::warn;

struct Playback {
    clock: Option<Arc<super::sync_clock::SyncClock>>,
    device: Arc<dyn RgbDevice>,
    frames: Arc<Vec<Vec<[u8; 3]>>>,
    interval: Duration,
    timing: RgbPlaybackTiming,
    started: Instant,
    next: Option<Instant>,
    last_frame: Option<usize>,
    failures: u32,
}

#[derive(Default)]
struct State {
    devices: HashMap<String, Playback>,
    stopped: bool,
}

pub(super) struct WiredRenderer {
    shared: Arc<(Mutex<State>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}

impl WiredRenderer {
    pub fn new() -> Self {
        let shared = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let worker = shared.clone();
        let thread = thread::spawn(move || loop {
            let (id, device, frames, index, interval, timing) = {
                let mut state = worker.0.lock();
                loop {
                    if state.stopped {
                        return;
                    }
                    let now = Instant::now();
                    let due = state
                        .devices
                        .iter()
                        .filter_map(|(id, p)| {
                            p.next
                                .filter(|&next| next <= now)
                                .map(|next| (id.clone(), next))
                        })
                        .min_by_key(|(_, next)| *next);
                    if let Some((id, _)) = due {
                        let p = state.devices.get_mut(&id).unwrap();
                        if p.device.rf_owned() {
                            p.next = None;
                            continue;
                        }
                        let index = if p.device.software_frame_delivery()
                            == Some(RgbFrameDelivery::Streaming)
                        {
                            Some(p.clock.as_ref().map_or_else(
                                || {
                                    (p.started.elapsed().as_nanos() / p.interval.as_nanos())
                                        as usize
                                        % p.frames.len()
                                },
                                |clock| {
                                    clock.frame_index(p.timing.interval_hundredths, p.frames.len())
                                },
                            ))
                        } else {
                            None
                        };
                        if let (Some(last), Some(index)) = (p.last_frame, index) {
                            if p.frames[last] == p.frames[index] {
                                p.next = Some(now + p.interval);
                                continue;
                            }
                        }
                        break (
                            id,
                            p.device.clone(),
                            p.frames.clone(),
                            index,
                            p.interval,
                            p.timing,
                        );
                    }
                    if let Some(next) = state.devices.values().filter_map(|p| p.next).min() {
                        worker
                            .1
                            .wait_for(&mut state, next.saturating_duration_since(now));
                    } else {
                        worker.1.wait(&mut state);
                    }
                }
            };
            let result = if let Some(index) = index {
                device.set_software_animation(&frames[index..index + 1], timing)
            } else {
                device.set_software_animation(&frames, timing)
            };
            let mut state = worker.0.lock();
            if let Some(p) = state
                .devices
                .get_mut(&id)
                .filter(|p| Arc::ptr_eq(&p.frames, &frames))
            {
                match result {
                    Ok(()) => {
                        p.failures = 0;
                        p.last_frame = index;
                        p.next = if index.is_some() && frames.len() > 1 {
                            Some(Instant::now() + interval)
                        } else {
                            None
                        };
                    }
                    Err(error) => {
                        p.failures = p.failures.saturating_add(1);
                        p.next = Some(
                            Instant::now()
                                + Duration::from_millis(500 * u64::from(p.failures.min(20))),
                        );
                        if p.failures == 1 {
                            warn!("Software RGB failed for {id}; will retry: {error}");
                        }
                    }
                }
            }
        });
        Self {
            shared,
            thread: Some(thread),
        }
    }

    pub fn submit(
        &self,
        id: &str,
        device: Arc<dyn RgbDevice>,
        frames: Vec<Vec<[u8; 3]>>,
        interval_ms: u16,
    ) -> Result<()> {
        self.submit_animation(
            id,
            device,
            frames,
            RgbPlaybackTiming::from_millis(interval_ms),
        )
    }

    pub fn submit_animation(
        &self,
        id: &str,
        device: Arc<dyn RgbDevice>,
        frames: Vec<Vec<[u8; 3]>>,
        timing: RgbPlaybackTiming,
    ) -> Result<()> {
        self.submit_timed(id, device, frames, timing, None)
    }

    pub fn submit_timed(
        &self,
        id: &str,
        device: Arc<dyn RgbDevice>,
        mut frames: Vec<Vec<[u8; 3]>>,
        timing: RgbPlaybackTiming,
        clock: Option<Arc<super::sync_clock::SyncClock>>,
    ) -> Result<()> {
        ensure!(
            !frames.is_empty() && frames.len() <= 2048,
            "invalid software RGB frame count"
        );
        ensure!(
            (100..=6_553_599).contains(&timing.interval_hundredths),
            "invalid software RGB playback interval"
        );
        let led_count = usize::from(device.total_led_count());
        ensure!(
            led_count > 0 && frames.iter().all(|f| f.len() == led_count),
            "software RGB frame does not match device LED layout"
        );
        device.validate_software_animation(&frames, timing)?;
        if device.software_frame_delivery() == Some(RgbFrameDelivery::Streaming)
            && frames.iter().all(|frame| frame == &frames[0])
        {
            frames.truncate(1);
        }
        let mut state = self.shared.0.lock();
        ensure!(!state.stopped, "RGB renderer is stopped");
        ensure!(
            state.devices.contains_key(id) || state.devices.len() < 64,
            "RGB renderer device limit reached"
        );
        let now = Instant::now();
        state.devices.insert(
            id.to_owned(),
            Playback {
                clock,
                device,
                frames: Arc::new(frames),
                interval: Duration::from_nanos(u64::from(timing.interval_hundredths) * 6_250),
                timing,
                started: now,
                next: Some(now),
                last_frame: None,
                failures: 0,
            },
        );
        self.shared.1.notify_one();
        Ok(())
    }

    pub fn clear(&self) {
        self.shared.0.lock().devices.clear();
        self.shared.1.notify_one();
    }

    pub fn remove(&self, id: &str) {
        self.shared.0.lock().devices.remove(id);
        self.shared.1.notify_one();
    }

    pub fn stop(&mut self) {
        {
            let mut state = self.shared.0.lock();
            state.stopped = true;
            state.devices.clear();
            self.shared.1.notify_one();
        }
        if let Some(thread) = self.thread.take() {
            if thread.join().is_err() {
                warn!("Wired RGB renderer panicked");
            }
        }
    }
}

impl Drop for WiredRenderer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    type Frames = Vec<Vec<[u8; 3]>>;
    use super::*;
    use lianli_shared::rgb::{RgbEffect, RgbMode, RgbZoneInfo};
    use std::sync::mpsc::{self, Receiver, Sender};

    struct RecordingDevice {
        delivery: RgbFrameDelivery,
        frames: Sender<Vec<Vec<[u8; 3]>>>,
        failures_left: std::sync::atomic::AtomicUsize,
    }

    impl RecordingDevice {
        fn new(delivery: RgbFrameDelivery) -> (Arc<Self>, Receiver<Frames>) {
            let (frames, received) = mpsc::channel();
            (
                Arc::new(Self {
                    delivery,
                    frames,
                    failures_left: Default::default(),
                }),
                received,
            )
        }
    }

    impl RgbDevice for RecordingDevice {
        fn device_name(&self) -> String {
            "recording device".to_owned()
        }

        fn supported_modes(&self) -> Vec<RgbMode> {
            vec![RgbMode::Static]
        }

        fn zone_info(&self) -> Vec<RgbZoneInfo> {
            vec![RgbZoneInfo {
                name: "test".to_owned(),
                led_count: 1,
            }]
        }

        fn set_zone_effect(&self, _zone: u8, _effect: &RgbEffect) -> Result<()> {
            Ok(())
        }

        fn software_frame_delivery(&self) -> Option<RgbFrameDelivery> {
            Some(self.delivery)
        }

        fn set_software_animation(
            &self,
            frames: &[Vec<[u8; 3]>],
            _timing: RgbPlaybackTiming,
        ) -> Result<()> {
            if self
                .failures_left
                .fetch_update(
                    std::sync::atomic::Ordering::Relaxed,
                    std::sync::atomic::Ordering::Relaxed,
                    |remaining| remaining.checked_sub(1),
                )
                .is_ok()
            {
                anyhow::bail!("temporary transfer failure");
            }
            self.frames
                .send(frames.to_vec())
                .map_err(|_| anyhow::anyhow!("recording receiver dropped"))
        }
    }

    fn frame(value: u8) -> Vec<[u8; 3]> {
        vec![[value, 0, 0]]
    }

    #[test]
    fn transient_failure_does_not_permanently_disable_playback() {
        let (device, received) = RecordingDevice::new(RgbFrameDelivery::LoopUpload);
        device
            .failures_left
            .store(3, std::sync::atomic::Ordering::Relaxed);
        let renderer = WiredRenderer::new();
        renderer
            .submit("recover", device, vec![frame(9)], 100)
            .unwrap();
        assert_eq!(
            received.recv_timeout(Duration::from_secs(6)).unwrap(),
            vec![frame(9)]
        );
    }

    #[test]
    fn loop_upload_static_frame_is_sent_once() {
        let (device, received) = RecordingDevice::new(RgbFrameDelivery::LoopUpload);
        let renderer = WiredRenderer::new();
        renderer
            .submit("static", device, vec![frame(1)], 100)
            .unwrap();

        assert_eq!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            vec![frame(1)]
        );
        assert!(received.recv_timeout(Duration::from_millis(150)).is_err());
    }

    #[test]
    fn identical_streaming_frames_become_idle_without_changing_uploaded_loops() {
        for delivery in [RgbFrameDelivery::Streaming, RgbFrameDelivery::LoopUpload] {
            let (device, received) = RecordingDevice::new(delivery);
            let renderer = WiredRenderer::new();
            renderer
                .submit("static", device, vec![frame(7); 30], 10)
                .unwrap();
            let expected = if delivery == RgbFrameDelivery::Streaming {
                1
            } else {
                30
            };
            assert_eq!(
                received.recv_timeout(Duration::from_secs(1)).unwrap(),
                vec![frame(7); expected]
            );
            assert!(received.recv_timeout(Duration::from_millis(50)).is_err());
            let state = renderer.shared.0.lock();
            assert!(state.devices["static"].next.is_none());
            assert_eq!(state.devices["static"].frames.len(), expected);
        }
    }

    #[test]
    fn replacing_playback_cancels_old_animation() {
        let (recording, received) = RecordingDevice::new(RgbFrameDelivery::Streaming);
        let device: Arc<dyn RgbDevice> = recording;
        let renderer = WiredRenderer::new();
        renderer
            .submit(
                "replace",
                Arc::clone(&device),
                vec![frame(1), frame(2)],
                100,
            )
            .unwrap();
        assert_eq!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            vec![frame(1)]
        );

        renderer
            .submit("replace", device, vec![frame(3)], 100)
            .unwrap();
        assert_eq!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            vec![frame(3)]
        );
        assert!(received.recv_timeout(Duration::from_millis(250)).is_err());
    }

    #[test]
    fn stop_joins_worker_and_rejects_new_playback() {
        let mut renderer = WiredRenderer::new();
        renderer.stop();
        let (device, _received) = RecordingDevice::new(RgbFrameDelivery::LoopUpload);
        assert!(renderer
            .submit("after-stop", device, vec![frame(1)], 100)
            .is_err());
    }
}
