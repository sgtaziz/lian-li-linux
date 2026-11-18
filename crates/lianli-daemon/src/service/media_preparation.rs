use super::DaemonEvent;
use lianli_media::{prepare_media_asset, MediaAsset, MediaAssetKind, PreparationControl};
use lianli_shared::config::{ConfigKey, LcdConfig};
use lianli_shared::screen::ScreenInfo;
use lianli_shared::template::LcdTemplate;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MediaTarget {
    pub selection: String,
    pub device_id: String,
    pub screen: ScreenInfo,
}

pub(super) struct MediaJob {
    pub index: usize,
    pub config: LcdConfig,
    pub key: ConfigKey,
    pub target: MediaTarget,
    pub h264: bool,
}

pub(super) struct PreparationRequest {
    pub generation: u64,
    pub jobs: Vec<MediaJob>,
    pub templates: Vec<LcdTemplate>,
    pub default_fps: f32,
    pub hardware_video: bool,
    pub catalog_runtime: Arc<crate::catalog_references::RuntimeReferences>,
}

pub(super) struct PreparedMedia {
    pub generation: u64,
    pub index: usize,
    pub config: LcdConfig,
    pub result: Result<Arc<MediaAsset>, String>,
    pub target: MediaTarget,
    pub h264: bool,
}

struct RunningPreparation {
    control: PreparationControl,
    results: Receiver<PreparedMedia>,
    thread: JoinHandle<()>,
}

#[derive(Default)]
pub(super) struct MediaPreparation {
    generation: u64,
    pending: Option<PreparationRequest>,
    running: Option<RunningPreparation>,
}

impl MediaPreparation {
    pub fn submit(&mut self, mut request: PreparationRequest) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        request.generation = self.generation;
        self.cancel_running();
        self.pending = Some(request);
        self.generation
    }

    pub fn is_busy(&self) -> bool {
        self.running.is_some() || self.pending.is_some()
    }

    pub fn poll(&mut self, notify: &Sender<DaemonEvent>) -> Vec<PreparedMedia> {
        let mut results = Vec::new();
        if let Some(running) = &self.running {
            if let Ok(result) = running.results.try_recv() {
                if result.generation == self.generation {
                    results.push(result);
                }
            }
        }
        if self
            .running
            .as_ref()
            .is_some_and(|r| r.thread.is_finished())
        {
            let running = self.running.take().unwrap();
            let failed = running.thread.join().is_err();
            if let Ok(result) = running.results.try_recv() {
                if result.generation == self.generation {
                    results.push(result);
                }
            }
            if failed {
                tracing::error!("Media preparation worker panicked");
            }
        }
        if self.running.is_none() {
            if let Some(request) = self.pending.take() {
                if !request.jobs.is_empty() {
                    let control = PreparationControl::new(request.hardware_video);
                    let worker_control = control.clone();
                    let (sender, receiver) = mpsc::sync_channel(1);
                    let notify = notify.clone();
                    match thread::Builder::new()
                        .name("media-prepare".into())
                        .spawn(move || {
                            prepare(request, &worker_control, &sender, &notify);
                            let _ = notify.send(DaemonEvent::MediaPrepared);
                        }) {
                        Ok(thread) => {
                            self.running = Some(RunningPreparation {
                                control,
                                results: receiver,
                                thread,
                            })
                        }
                        Err(error) => tracing::error!("Cannot start media preparation: {error}"),
                    }
                }
            }
        }
        results
    }

    pub fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.pending = None;
        self.cancel_running();
    }

    fn cancel_running(&self) {
        if let Some(running) = &self.running {
            running.control.cancel();
        }
    }
}

impl Drop for MediaPreparation {
    fn drop(&mut self) {
        self.cancel();
        if let Some(running) = self.running.take() {
            let deadline = Instant::now() + Duration::from_secs(2);
            while !running.thread.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            if running.thread.is_finished() {
                if running.thread.join().is_err() {
                    tracing::warn!("Media preparation worker panicked during shutdown");
                }
            } else {
                // Image decoding and filesystem reads cannot be interrupted mid-call.
                // This worker owns no device handles and is detached only at shutdown.
                tracing::warn!("Media preparation is still finishing a cancelled file operation");
            }
        }
    }
}

fn prepare(
    request: PreparationRequest,
    control: &PreparationControl,
    sender: &SyncSender<PreparedMedia>,
    notify: &Sender<DaemonEvent>,
) {
    if control.check().is_err() {
        return;
    }
    let Ok(usage) = request.catalog_runtime.enter(|| {
        control.check()?;
        Ok(())
    }) else {
        return;
    };
    let sensors = if request.jobs.iter().any(|job| {
        matches!(
            job.config.media_type,
            lianli_shared::media::MediaType::Sensor | lianli_shared::media::MediaType::Custom
        )
    }) {
        lianli_shared::sensors::enumerate_sensors()
    } else {
        Vec::new()
    };
    for job in request.jobs {
        let control = control.next_asset();
        if control.check().is_err() {
            return;
        }
        let screen = ScreenInfo {
            h264: job.h264,
            ..job.target.screen
        };
        let dependencies =
            lianli_shared::media_dependencies::lcd_dependencies(&job.config, &request.templates)
                .unwrap_or_else(|_| {
                    lianli_shared::media_dependencies::stored_lcd_dependencies(&job.config)
                });
        usage.record(&dependencies);
        let result = prepare_media_asset(
            &job.config,
            request.default_fps,
            &screen,
            screen.h264,
            &sensors,
            &request.templates,
            &control,
        )
        .and_then(|kind| {
            control.check()?;
            let stream_fps = match &kind {
                MediaAssetKind::Custom { asset } => asset.render_fps(),
                _ => job
                    .config
                    .fps
                    .unwrap_or(request.default_fps)
                    .min(request.default_fps)
                    .min(screen.max_fps as f32)
                    .max(1.0),
            };
            Ok(Arc::new(MediaAsset {
                kind,
                config_key: job.key,
                stream_fps,
                hardware_video: request.hardware_video,
            }))
        })
        .map_err(|error| error.to_string().chars().take(2048).collect());
        usage.record(&dependencies);
        if !deliver(
            sender,
            PreparedMedia {
                generation: request.generation,
                index: job.index,
                config: job.config,
                target: job.target,
                h264: job.h264,
                result,
            },
            &control,
        ) {
            return;
        }
        if notify.send(DaemonEvent::MediaPrepared).is_err() {
            return;
        }
    }
}

fn deliver(
    sender: &SyncSender<PreparedMedia>,
    mut result: PreparedMedia,
    control: &PreparationControl,
) -> bool {
    loop {
        if control.check().is_err() {
            return false;
        }
        match sender.try_send(result) {
            Ok(()) => return true,
            Err(TrySendError::Disconnected(_)) => return false,
            Err(TrySendError::Full(returned)) => result = returned,
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> MediaTarget {
        MediaTarget {
            selection: "index:0".into(),
            device_id: "panel".into(),
            screen: ScreenInfo::TLLCD,
        }
    }

    fn request() -> PreparationRequest {
        PreparationRequest {
            catalog_runtime: Default::default(),
            generation: 0,
            jobs: Vec::new(),
            templates: Vec::new(),
            default_fps: 30.0,
            hardware_video: false,
        }
    }

    #[test]
    fn multiple_edits_keep_only_the_latest_pending_request() {
        let mut worker = MediaPreparation::default();
        let old = worker.submit(request());
        let mut newest = request();
        newest.hardware_video = true;
        let new = worker.submit(newest);
        assert_ne!(old, new);
        assert_eq!(worker.pending.as_ref().unwrap().generation, new);
        assert!(worker.pending.as_ref().unwrap().hardware_video);
        worker.cancel();
        assert!(!worker.is_busy());
    }

    #[test]
    fn cancelled_output_is_not_delivered_even_with_free_queue_space() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let control = PreparationControl::new(false);
        control.cancel();
        let config =
            serde_json::from_value(serde_json::json!({ "type": "color", "rgb": [0,0,0] })).unwrap();
        assert!(!deliver(
            &sender,
            PreparedMedia {
                generation: 1,
                index: 0,
                config,
                result: Err("old".into()),
                target: target(),
                h264: false,
            },
            &control
        ));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn a_full_result_queue_does_not_prevent_cancellation() {
        let (sender, _receiver) = mpsc::sync_channel(1);
        let control = PreparationControl::new(false);
        let worker_control = control.clone();
        let config: LcdConfig =
            serde_json::from_value(serde_json::json!({ "type": "color", "rgb": [0,0,0] })).unwrap();
        sender
            .send(PreparedMedia {
                generation: 1,
                index: 0,
                config: config.clone(),
                result: Err("first".into()),
                target: target(),
                h264: false,
            })
            .ok()
            .unwrap();
        let worker = thread::spawn(move || {
            deliver(
                &sender,
                PreparedMedia {
                    generation: 1,
                    index: 1,
                    config,
                    result: Err("second".into()),
                    target: target(),
                    h264: false,
                },
                &worker_control,
            )
        });
        thread::sleep(Duration::from_millis(50));
        control.cancel();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !worker.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(worker.is_finished());
        assert!(!worker.join().unwrap());
    }
}
