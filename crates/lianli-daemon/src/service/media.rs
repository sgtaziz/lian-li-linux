use super::media_preparation::{MediaJob, MediaTarget, PreparationRequest};
use super::runtime::{ActiveTarget, LcdBackend, ThreadedWinUsbSender};
use super::{DaemonEvent, ServiceManager};
use lianli_devices::detect::{create_hid_lcd_device, enumerate_devices, open_hid_lcd_device};
use lianli_devices::slv3_lcd::Slv3LcdDevice;
use lianli_shared::config::{config_identity, ConfigKey, LcdConfig};
use lianli_shared::device_id::DeviceFamily;
use lianli_shared::ipc::{MediaPreparationState, MediaPreparationStatus};
use lianli_shared::media::MediaType;
use lianli_shared::screen::{screen_info_for, ScreenInfo};
use lianli_shared::template::LcdTemplate;
use lianli_shared::ui_message::{UiMessage, UiText};
use rusb::Device;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};

const SERIAL_REWRITE_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

fn asset_cache_key(
    device: &LcdConfig,
    user_templates: &[LcdTemplate],
    default_fps: f32,
    hardware_video: bool,
) -> ConfigKey {
    let base = format!(
        "{}|fps:{default_fps}|hw:{hardware_video}",
        config_identity(device)
    );
    if device.media_type != MediaType::Custom {
        return base;
    }
    let Some(id) = device.template_id.as_deref() else {
        return base;
    };
    let Some(tpl) = user_templates.iter().find(|t| t.id == id).cloned() else {
        return base;
    };
    let body = serde_json::to_string(&tpl).unwrap_or_default();
    format!("{base}|tpl:{body}")
}

impl ServiceManager {
    pub(super) fn prepare_media_assets(&mut self, tx: Sender<DaemonEvent>) {
        if !self.pixel_clean_sessions.is_empty() || self.pixel_clean_preparation.is_some() {
            self.media_reload_pending = true;
            return;
        }
        self.media_reload_pending = false;
        let Some(cfg) = &self.config else {
            return;
        };
        let templates = self.ipc.state.lock().user_templates.clone();
        let targets: HashMap<_, _> = self
            .media_targets
            .iter()
            .filter(|(index, target)| {
                cfg.lcds
                    .get(**index)
                    .is_some_and(|cfg| target.selection == selection_key(cfg))
            })
            .map(|(index, target)| (*index, target.clone()))
            .collect();
        let h264: Vec<_> = (0..cfg.lcds.len())
            .map(|index| {
                targets
                    .get(&index)
                    .is_some_and(|target| target.screen.h264 && cfg.h264_allowed(&target.device_id))
            })
            .collect();
        let keys: Vec<_> = cfg
            .lcds
            .iter()
            .enumerate()
            .map(|(index, device)| {
                let base = asset_cache_key(device, &templates, cfg.default_fps, cfg.hardware_video);
                format!(
                    "{base}|target:{:?}|h264:{}",
                    targets.get(&index),
                    h264[index]
                )
            })
            .collect();
        if self.media_preparation.is_busy() && self.media_requested_keys == keys {
            return;
        }
        self.media_requested_keys = keys;
        for target in self.targets.lock().values_mut() {
            target.retry_failed_source();
        }
        self.media_assets.retain(|index, _| {
            self.media_settings
                .get(index)
                .zip(cfg.lcds.get(*index))
                .is_some_and(|(old, new)| same_selection(old, new))
                && self
                    .media_asset_targets
                    .get(index)
                    .zip(targets.get(index))
                    .is_some_and(|(old, new)| old == new)
        });
        self.media_settings
            .retain(|index, _| self.media_assets.contains_key(index));
        self.media_asset_targets
            .retain(|index, _| self.media_assets.contains_key(index));
        self.media_asset_h264
            .retain(|index, _| self.media_assets.contains_key(index));
        let jobs: Vec<_> = cfg
            .lcds
            .iter()
            .enumerate()
            .filter_map(|(index, device)| {
                let target = targets.get(&index)?.clone();
                let key = self.media_requested_keys[index].clone();
                if self
                    .media_assets
                    .get(&index)
                    .is_some_and(|asset| asset.config_key == key)
                {
                    return None;
                }
                Some(MediaJob {
                    index,
                    config: device.clone(),
                    key,
                    target,
                    h264: h264[index],
                })
            })
            .collect();
        let preparing: HashSet<_> = jobs.iter().map(|job| job.index).collect();
        let generation = self.media_preparation.submit(PreparationRequest {
            catalog_runtime: self.ipc.state.lock().catalog_runtime.clone(),
            generation: 0,
            jobs,
            templates,
            default_fps: cfg.default_fps,
            hardware_video: cfg.hardware_video,
        });
        self.ipc.state.lock().telemetry.media_preparation = cfg
            .lcds
            .iter()
            .enumerate()
            .map(|(index, cfg)| {
                let recovery = cfg.serial.as_ref().is_some_and(|serial| {
                    self.startup_image_quarantine
                        .iter()
                        .any(|id| lcd_id_matches(serial, id))
                });
                let (error, error_ui) =
                    UiText::split(recovery.then(super::startup_image::recovery_text));
                (
                    index,
                    MediaPreparationStatus {
                        startup_recovery_required: recovery,
                        runtime: None,
                        last_playback_error: None,
                        last_playback_error_ui: None,
                        generation,
                        device_id: cfg.device_id(),
                        state: if recovery {
                            MediaPreparationState::Failed
                        } else if !targets.contains_key(&index) {
                            MediaPreparationState::WaitingForDevice
                        } else if preparing.contains(&index) {
                            MediaPreparationState::Preparing
                        } else {
                            MediaPreparationState::Ready
                        },
                        error,
                        error_ui,
                    },
                )
            })
            .collect();
        let results = self.media_preparation.poll(&tx);
        self.apply_prepared_results(results);
    }

    pub(super) fn poll_prepared_media(&mut self) {
        let Some(tx) = self.tx.clone() else {
            return;
        };
        let retry_ready =
            self.ipc.state.lock().media_retry_pending && !self.media_preparation.is_busy();
        if retry_ready {
            self.media_reload_pending = true;
            for target in self.targets.lock().values_mut() {
                target.retry_interrupted_transfer();
            }
        }
        if self.media_reload_pending
            && self.pixel_clean_sessions.is_empty()
            && self.pixel_clean_preparation.is_none()
        {
            self.prepare_media_assets(tx.clone());
            if retry_ready {
                self.ipc.state.lock().media_retry_pending = false;
            }
        }
        let results = self.media_preparation.poll(&tx);
        self.apply_prepared_results(results);
        if !self.media_preparation.is_busy() {
            for status in self
                .ipc
                .state
                .lock()
                .telemetry
                .media_preparation
                .values_mut()
            {
                if status.state == MediaPreparationState::Preparing {
                    status.state = MediaPreparationState::Failed;
                    status.error = Some(
                        "Media preparation stopped before producing a result. Save again to retry."
                            .into(),
                    );
                    status.error_ui = Some(UiMessage::new("status.media.preparationStopped"));
                }
            }
        }
    }

    fn apply_prepared_results(&mut self, results: Vec<super::media_preparation::PreparedMedia>) {
        for result in results {
            let current = self
                .ipc
                .state
                .lock()
                .telemetry
                .media_preparation
                .get(&result.index)
                .is_some_and(|status| status.generation == result.generation);
            if !current {
                continue;
            }
            let (new_state, error) = match result.result {
                Ok(asset) => {
                    self.media_assets.insert(result.index, asset);
                    self.media_settings.insert(result.index, result.config);
                    self.media_asset_targets.insert(result.index, result.target);
                    self.media_asset_h264.insert(result.index, result.h264);
                    (MediaPreparationState::Ready, None)
                }
                Err(error) => {
                    warn!(
                        "LCD[{}] media preparation failed: {error}",
                        result.config.device_id()
                    );
                    (MediaPreparationState::Failed, Some(error))
                }
            };
            if let Some(status) = self
                .ipc
                .state
                .lock()
                .telemetry
                .media_preparation
                .get_mut(&result.index)
            {
                status.state = new_state;
                status.error = error;
                status.error_ui = None;
            }
        }
    }

    pub(super) fn record_playback_failure(&self, index: usize, key: &str, error: UiText) {
        if self
            .media_requested_keys
            .get(index)
            .is_none_or(|current| current != key)
        {
            return;
        }
        if let Some(status) = self
            .ipc
            .state
            .lock()
            .telemetry
            .media_preparation
            .get_mut(&index)
        {
            let error = error.truncated(2048);
            status.last_playback_error = Some(error.english);
            status.last_playback_error_ui = error.ui;
        }
    }

    fn take_target(&self, index: usize) -> Option<ActiveTarget> {
        self.targets.lock().remove(&index)
    }

    pub(super) fn refresh_targets(&mut self) {
        if self.startup_image_job.is_some() || self.lcd_group_recovery.busy() {
            return;
        }
        if self.config.as_ref().is_none_or(|cfg| cfg.lcds.is_empty())
            && self.targets.lock().is_empty()
        {
            self.ipc
                .state
                .lock()
                .state_health
                .retain_lcds(&HashSet::new());
            return;
        }

        struct LcdCandidate {
            family: DeviceFamily,
            device_id: String,
            usb_device: Option<Device<lianli_transport::UsbContext>>,
            vid: u16,
            pid: u16,
            bus: u8,
            address: u8,
        }

        let mut candidates: Vec<LcdCandidate> = Vec::new();

        self.mode_switch_suppression
            .retain(|_, until| Instant::now() < *until);

        let usb_devs = match enumerate_devices() {
            Ok(devices) => devices,
            Err(error) => {
                debug!("LCD enumeration unavailable: {error}");
                return;
            }
        };
        for det in usb_devs {
            if !is_streamable_lcd(det.family) {
                continue;
            }
            let device_id = det.device_id();
            if self.startup_image_quarantine.contains(&device_id) {
                continue;
            }
            if self.mode_switch_suppressed(&device_id) {
                debug!("LCD candidate skipped (recent mode switch): {device_id}");
                continue;
            }
            let transport = if lianli_shared::device_id::uses_hid(det.family) {
                "HID"
            } else {
                "USB bulk"
            };
            debug!(
                "LCD candidate: {} ({:04x}:{:04x}) id={device_id} ({transport})",
                det.name, det.vid, det.pid
            );
            candidates.push(LcdCandidate {
                family: det.family,
                device_id,
                usb_device: Some(det.device),
                vid: det.vid,
                pid: det.pid,
                bus: det.bus,
                address: det.address,
            });
        }

        let present: HashSet<String> = candidates
            .iter()
            .map(|candidate| candidate.device_id.clone())
            .collect();
        let now = Instant::now();
        self.lcd_transfer_recovery
            .retain(|device_id, recovery| present.contains(device_id) || !recovery.expired(now));
        self.ipc.state.lock().state_health.retain_lcds(&present);
        let mut new_targets = HashMap::new();
        let mut new_media_targets = HashMap::new();
        let mut canonicalize: Vec<(String, String)> = Vec::new();

        if let Some(cfg) = &self.config {
            let mut claimed: HashSet<usize> = HashSet::new();
            for (cfg_idx, device_cfg) in cfg.lcds.iter().enumerate() {
                let matched = if let Some(serial) = &device_cfg.serial {
                    candidates
                        .iter()
                        .enumerate()
                        .find(|(idx, candidate)| {
                            !claimed.contains(idx) && lcd_id_matches(serial, &candidate.device_id)
                        })
                        .map(|(idx, candidate)| {
                            claimed.insert(idx);
                            candidate
                        })
                } else if let Some(index) = device_cfg.index {
                    candidates
                        .get(index)
                        .filter(|_| !claimed.contains(&index))
                        .inspect(|_c| {
                            claimed.insert(index);
                        })
                } else {
                    None
                };

                let candidate = match matched {
                    Some(c) => c,
                    None => {
                        if let Some(mut existing) = self.take_target(cfg_idx) {
                            info!("[devices] LCD[{}] detached", device_cfg.device_id());
                            existing.stop();
                        }
                        continue;
                    }
                };

                let Some(screen) = screen_info_for(candidate.family) else {
                    continue;
                };
                let media_target = MediaTarget {
                    selection: selection_key(device_cfg),
                    device_id: candidate.device_id.clone(),
                    screen,
                };
                new_media_targets.insert(cfg_idx, media_target.clone());
                let asset = match self
                    .media_assets
                    .get(&cfg_idx)
                    .filter(|_| self.media_asset_targets.get(&cfg_idx) == Some(&media_target))
                {
                    Some(asset) => Arc::clone(asset),
                    None => {
                        if let Some(mut existing) = self.take_target(cfg_idx) {
                            existing.stop();
                        }
                        continue;
                    }
                };
                let applied_cfg = self.media_settings.get(&cfg_idx).unwrap_or(device_cfg);
                let playback_screen = ScreenInfo {
                    h264: self
                        .media_asset_h264
                        .get(&cfg_idx)
                        .copied()
                        .unwrap_or_else(|| screen.h264 && cfg.h264_allowed(&candidate.device_id)),
                    ..screen
                };

                // Form rewrites only: the alias fallback may have matched a
                // different physical device, which must not be persisted.
                if let Some(serial) = &device_cfg.serial {
                    if serial != &candidate.device_id
                        && hid_id_norm(serial) == hid_id_norm(&candidate.device_id)
                    {
                        canonicalize.push((serial.clone(), candidate.device_id.clone()));
                    }
                }

                let cfg_key = asset.config_key.clone();
                if let Some(mut existing) = self.take_target(cfg_idx) {
                    if existing.matches(&candidate.device_id, &cfg_key) {
                        // Media is unchanged, but the custom_h264 toggle may have
                        // flipped — rebuild the frame source so the H.264 pipeline
                        // engages/disengages without a daemon restart.
                        if existing.screen == playback_screen {
                            existing.update_custom_h264(applied_cfg.custom_h264(), self.tx.clone());
                        } else {
                            existing.screen = playback_screen;
                            existing.swap_media(
                                Arc::clone(&asset),
                                applied_cfg.custom_h264(),
                                self.tx.clone(),
                            );
                        }
                        new_targets.insert(cfg_idx, existing);
                        continue;
                    } else if existing.device_identity == candidate.device_id {
                        // Same device, different config — reuse the USB transport,
                        // just swap the media asset. Reopening the device can leave
                        // some firmware in a bad state.
                        existing.screen = playback_screen;
                        existing.swap_media(
                            Arc::clone(&asset),
                            applied_cfg.custom_h264(),
                            self.tx.clone(),
                        );
                        existing.key = cfg_key;
                        new_targets.insert(cfg_idx, existing);
                        if let Some(ref tx) = self.tx {
                            tx.send(DaemonEvent::FrameFinished).ok();
                        }
                        continue;
                    } else {
                        existing.stop();
                    }
                }

                let backend_result: anyhow::Result<LcdBackend> =
                    match lcd_backend_kind(candidate.family) {
                        Some(LcdBackendKind::Slv3) => {
                            let device = Device::clone(candidate.usb_device.as_ref().unwrap());
                            Slv3LcdDevice::new(device).map(LcdBackend::Slv3)
                        }
                        Some(LcdBackendKind::WinUsbShared) => {
                            if let Some(transport) =
                                self.registry.usb_backends.get(&candidate.device_id)
                            {
                                lianli_devices::winusb::lcd::WinUsbLcdDevice::from_shared_transport(
                                    Arc::clone(transport),
                                    candidate.pid,
                                )
                                .map(|d| LcdBackend::WinUsb(ThreadedWinUsbSender::new(d, cfg_idx)))
                            } else {
                                let device = Device::clone(candidate.usb_device.as_ref().unwrap());
                                lianli_devices::winusb::lcd::WinUsbLcdDevice::open(
                                    device,
                                    candidate.pid,
                                )
                                .map(|d| LcdBackend::WinUsb(ThreadedWinUsbSender::new(d, cfg_idx)))
                            }
                        }
                        Some(LcdBackendKind::WinUsb) => {
                            let device = Device::clone(candidate.usb_device.as_ref().unwrap());
                            lianli_devices::winusb::lcd::WinUsbLcdDevice::open(
                                device,
                                candidate.pid,
                            )
                            .map(|d| LcdBackend::WinUsb(ThreadedWinUsbSender::new(d, cfg_idx)))
                        }
                        Some(LcdBackendKind::HidAio) => {
                            if let Some(d) =
                                self.registry.aio_lcd_devices.remove(&candidate.device_id)
                            {
                                Ok(LcdBackend::HidLcd(Arc::new(super::runtime::HidLcd::new(d))))
                            } else if let Some(backend) =
                                self.registry.hid_backends.get(&candidate.device_id)
                            {
                                match create_hid_lcd_device(
                                    candidate.family,
                                    candidate.pid,
                                    Arc::clone(backend),
                                ) {
                                    Some(result) => result.map(|d| {
                                        LcdBackend::HidLcd(Arc::new(super::runtime::HidLcd::new(d)))
                                    }),
                                    None => Err(anyhow::anyhow!("Not an LCD device")),
                                }
                            } else {
                                Err(anyhow::anyhow!(
                                    "AIO LCD '{}' not opened yet; deferring attach",
                                    candidate.device_id
                                ))
                            }
                        }
                        Some(LcdBackendKind::HidTl) => {
                            if let Some(backend) =
                                self.registry.hid_backends.get(&candidate.device_id)
                            {
                                match create_hid_lcd_device(
                                    candidate.family,
                                    candidate.pid,
                                    Arc::clone(backend),
                                ) {
                                    Some(result) => result.map(|d| {
                                        LcdBackend::HidLcd(Arc::new(super::runtime::HidLcd::new(d)))
                                    }),
                                    None => Err(anyhow::anyhow!("Not an LCD device")),
                                }
                            } else {
                                let device = Device::clone(candidate.usb_device.as_ref().unwrap());
                                let det = lianli_devices::detect::DetectedDevice {
                                    device,
                                    family: candidate.family,
                                    name: "TL LCD",
                                    vid: candidate.vid,
                                    pid: candidate.pid,
                                    bus: candidate.bus,
                                    address: candidate.address,
                                    serial: Some(candidate.device_id.clone()),
                                    hid_usage_page: None,
                                };
                                match open_hid_lcd_device(&det, self.hid_backend()) {
                                    Some(result) => result.map(|d| {
                                        LcdBackend::HidLcd(Arc::new(super::runtime::HidLcd::new(d)))
                                    }),
                                    None => Err(anyhow::anyhow!("Not an LCD device")),
                                }
                            }
                        }
                        None => Err(anyhow::anyhow!(
                            "no LCD backend for family {:?}",
                            candidate.family
                        )),
                    };

                match backend_result {
                    Ok(lcd) => {
                        self.ipc
                            .state
                            .lock()
                            .state_health
                            .lcd_initialization(&candidate.device_id, None);
                        info!(
                            "[devices] LCD[{}] attached (serial: {}, orientation: {:.0}°)",
                            device_cfg.device_id(),
                            candidate.device_id,
                            device_cfg.orientation
                        );
                        let mut init_pending = false;
                        let mut init_error = None;
                        if let LcdBackend::HidLcd(ref hid) = lcd {
                            // hydroshift init sleeps 10s, keep it off the main loop
                            if is_wired_aio_lcd(candidate.family) {
                                init_pending = true;
                                let hid_init = std::sync::Arc::clone(hid);
                                let enable_512 = device_cfg.aio_512_frame_for(candidate.family);
                                let device_id = candidate.device_id.clone();
                                let init_tx = self.tx.clone();
                                let spawn_err_id = device_id.clone();
                                let initialization_task = hid.lock().initialization_task();
                                if let Err(e) = std::thread::Builder::new()
                                    .name(format!("lcd-init-{device_id}"))
                                    .spawn(move || {
                                        let result =
                                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                                                || match initialization_task {
                                                    Some(initialize) => initialize(),
                                                    None => hid_init.lock().initialize(),
                                                },
                                            ))
                                            .unwrap_or_else(|_| {
                                                Err(anyhow::anyhow!(
                                                    "LCD initialization worker panicked"
                                                ))
                                            });
                                        let error = result.err().map(|e| {
                                            warn!("AIO LCD init failed for {device_id}: {e:#}");
                                            format!("{e:#}").chars().take(2048).collect()
                                        });
                                        if error.is_none() {
                                            hid_init.lock().set_use_c_command(enable_512);
                                        }
                                        if let Some(tx) = init_tx {
                                            tx.send(DaemonEvent::LcdInitComplete {
                                                device_id,
                                                attachment: hid_init.attachment(),
                                                error,
                                            })
                                            .ok();
                                        }
                                    })
                                {
                                    warn!(
                                        "Failed to spawn AIO LCD init thread for {spawn_err_id}: {e}"
                                    );
                                    self.ipc.state.lock().state_health.lcd_initialization(
                                        &spawn_err_id,
                                        Some(&format!("Could not start LCD initialization: {e}")),
                                    );
                                    init_error =
                                        Some(format!("Could not start LCD initialization: {e}"));
                                }
                                self.aio_lcd_firmware
                                    .record(&candidate.device_id, None, false);
                                if !self.aio_lcd_firmware.should_skip(&candidate.device_id) {
                                    self.aio_lcd_firmware.schedule(
                                        &candidate.device_id,
                                        std::time::Duration::from_secs(10),
                                        device_cfg.aio_512_frame_for(candidate.family),
                                    );
                                }
                            } else {
                                let result = hid.lock().initialize();
                                if let Err(e) = result {
                                    warn!(
                                        "AIO LCD basic init failed for {}: {e:#}",
                                        candidate.device_id
                                    );
                                    self.ipc.state.lock().state_health.lcd_initialization(
                                        &candidate.device_id,
                                        Some(&format!("{e:#}")),
                                    );
                                    init_error = Some(format!("{e:#}"));
                                }
                            }
                        }
                        let mut target = ActiveTarget::new(
                            cfg_idx,
                            candidate.device_id.clone(),
                            lcd,
                            Arc::clone(&asset),
                            playback_screen,
                            applied_cfg.custom_h264(),
                            self.tx.clone(),
                        );
                        if let Some(recovery) =
                            self.lcd_transfer_recovery.remove(&candidate.device_id)
                        {
                            target.restore_transfer_recovery(recovery);
                        }
                        if init_pending {
                            target.wait_for_initialization();
                        }
                        if let Some(error) = init_error {
                            target.finish_initialization(Some(&error));
                        }
                        target.maybe_start_recovery(self.tx.clone(), std::time::Duration::ZERO);
                        new_targets.insert(cfg_idx, target);
                        {
                            let brightness = device_cfg.brightness();
                            if let Some(t) = new_targets.get_mut(&cfg_idx) {
                                t.apply_brightness(
                                    Some(&self.wireless),
                                    &mut self.packet_builder,
                                    brightness,
                                );
                            }
                        }
                        if let Some(ref tx) = self.tx {
                            tx.send(DaemonEvent::FrameFinished).ok();
                        }
                    }
                    Err(err) => {
                        self.ipc
                            .state
                            .lock()
                            .state_health
                            .lcd_initialization(&candidate.device_id, Some(&format!("{err:#}")));
                        warn!(
                            "[devices] LCD[{}] unavailable during attach: {err}",
                            device_cfg.device_id()
                        );
                    }
                }
            }
        }

        // This runs every device poll. Without backoff a failing write would
        // be retried at 1 Hz while holding the IPC state lock.
        let backoff_expired = self
            .serial_rewrite_backoff
            .is_none_or(|t| t.elapsed() >= SERIAL_REWRITE_RETRY_INTERVAL);
        if !canonicalize.is_empty() && backoff_expired {
            let mut ipc_state = self.ipc.state.lock();
            if let Some(mut cfg) = ipc_state.config.clone().or_else(|| self.config.clone()) {
                let mut changed = false;
                for (old, canonical) in &canonicalize {
                    for lcd in &mut cfg.lcds {
                        if lcd.serial.as_deref() == Some(old.as_str()) {
                            lcd.serial = Some(canonical.clone());
                            changed = true;
                        }
                    }
                }
                if changed {
                    if let Err(e) = crate::persistence::write_config(&self.config_path, &cfg) {
                        self.serial_rewrite_backoff = Some(Instant::now());
                        warn!("Failed to persist canonicalized LCD serials: {e}");
                    } else {
                        self.serial_rewrite_backoff = None;
                        self.config = Some(cfg.clone());
                        ipc_state.config = Some(cfg);
                    }
                }
            }
        }

        let old_targets = std::mem::take(&mut *self.targets.lock());
        for (_, mut target) in old_targets {
            target.stop();
        }
        self.targets.lock().extend(new_targets);
        if new_media_targets != self.media_targets {
            self.media_targets = new_media_targets;
            if let Some(tx) = self.tx.clone() {
                self.prepare_media_assets(tx);
            }
        }
    }
}

/// Whether the daemon can stream media to this family over USB.
fn is_streamable_lcd(family: DeviceFamily) -> bool {
    family.has_lcd() && !family.is_desktop_mode()
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum LcdBackendKind {
    Slv3,
    WinUsbShared,
    WinUsb,
    HidAio,
    HidTl,
}

/// Single source of truth: maps an LCD family to its backend type.
fn lcd_backend_kind(family: DeviceFamily) -> Option<LcdBackendKind> {
    use DeviceFamily::*;
    Some(match family {
        Slv3Lcd | Tlv2Lcd => LcdBackendKind::Slv3,
        HydroShift2Lcd => LcdBackendKind::WinUsbShared,
        HydroShift2OledCurveLcd
        | Lancool207
        | UniversalScreen
        | Vision9p2
        | TlFlexLcd
        | SlInfFlexLcd => LcdBackendKind::WinUsb,
        HydroShiftLcd | Galahad2Lcd => LcdBackendKind::HidAio,
        TlLcd => LcdBackendKind::HidTl,
        _ => return None,
    })
}

fn hid_id_norm(s: &str) -> &str {
    s.strip_prefix("hid:").unwrap_or(s)
}

fn same_selection(old: &LcdConfig, new: &LcdConfig) -> bool {
    match (&old.serial, &new.serial) {
        (Some(old), Some(new)) => lcd_id_matches(old, new),
        (None, None) => old.index == new.index,
        _ => false,
    }
}

fn selection_key(config: &LcdConfig) -> String {
    config
        .serial
        .as_deref()
        .map(|serial| format!("serial:{}", hid_id_norm(serial)))
        .unwrap_or_else(|| config.device_id())
}

pub(super) fn lcd_id_matches(serial: &str, device_id: &str) -> bool {
    hid_id_norm(serial) == hid_id_norm(device_id)
}

fn is_wired_aio_lcd(family: DeviceFamily) -> bool {
    matches!(
        family,
        DeviceFamily::HydroShiftLcd
            | DeviceFamily::Galahad2Lcd
            | DeviceFamily::HydroShift2Lcd
            | DeviceFamily::HydroShift2OledCurveLcd
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_media::{MediaAsset, MediaAssetKind};
    use lianli_shared::device_id::KNOWN_DEVICES;

    fn config(serial: &str, rgb: [u8; 3]) -> LcdConfig {
        serde_json::from_value(serde_json::json!({ "type": "color", "serial": serial, "rgb": rgb }))
            .unwrap()
    }

    fn asset(key: &str) -> Arc<MediaAsset> {
        Arc::new(MediaAsset {
            config_key: key.into(),
            kind: MediaAssetKind::Static {
                frame: lianli_media::Retained::frame(vec![1]).unwrap(),
            },
            stream_fps: 30.0,
            hardware_video: false,
        })
    }

    fn target() -> MediaTarget {
        MediaTarget {
            selection: "serial:panel-a".into(),
            device_id: "hid:panel-a".into(),
            screen: ScreenInfo::AIO_LCD_480,
        }
    }

    #[test]
    fn stale_or_failed_preparation_preserves_the_working_asset() {
        let root = tempfile::tempdir().unwrap();
        let mut service = ServiceManager::new(
            root.path().join("config.json"),
            root.path().join("daemon.sock"),
            lianli_shared::daemon::DaemonMode::User,
        )
        .unwrap();
        let old = asset("old");
        let old_config = config("hid:panel-a", [0, 0, 0]);
        let new_config = config("hid:panel-a", [255, 0, 0]);
        service.media_assets.insert(0, old.clone());
        service.media_settings.insert(0, old_config);
        service.ipc.state.lock().telemetry.media_preparation.insert(
            0,
            MediaPreparationStatus {
                startup_recovery_required: false,
                runtime: None,
                last_playback_error: None,
                last_playback_error_ui: None,
                generation: 2,
                device_id: "hid:panel-a".into(),
                state: MediaPreparationState::Preparing,
                error: None,
                error_ui: None,
            },
        );
        service.media_requested_keys = vec!["new".into()];
        service.record_playback_failure(0, "old", "stale failure".into());
        assert!(service.ipc.state.lock().telemetry.media_preparation[&0]
            .last_playback_error
            .is_none());
        service.record_playback_failure(0, "new", "界".repeat(3000).into());
        assert_eq!(
            service.ipc.state.lock().telemetry.media_preparation[&0]
                .last_playback_error
                .as_ref()
                .unwrap()
                .chars()
                .count(),
            2048
        );
        assert!(service.targets.lock().is_empty());
        service.apply_prepared_results(vec![super::super::media_preparation::PreparedMedia {
            generation: 1,
            index: 0,
            config: new_config.clone(),
            result: Ok(asset("stale")),
            target: target(),
            h264: true,
        }]);
        assert!(Arc::ptr_eq(&service.media_assets[&0], &old));
        service.apply_prepared_results(vec![super::super::media_preparation::PreparedMedia {
            generation: 2,
            index: 0,
            config: new_config.clone(),
            result: Err("unreadable child video".into()),
            target: target(),
            h264: true,
        }]);
        assert!(Arc::ptr_eq(&service.media_assets[&0], &old));
        assert_eq!(
            service.ipc.state.lock().telemetry.media_preparation[&0].state,
            MediaPreparationState::Failed
        );
        let new = asset("new");
        service.apply_prepared_results(vec![super::super::media_preparation::PreparedMedia {
            generation: 2,
            index: 0,
            config: new_config,
            result: Ok(new.clone()),
            target: target(),
            h264: true,
        }]);
        assert!(Arc::ptr_eq(&service.media_assets[&0], &new));
        assert_eq!(service.media_settings[&0].rgb, Some([255, 0, 0]));
        assert_eq!(
            service.ipc.state.lock().telemetry.media_preparation[&0].state,
            MediaPreparationState::Ready
        );
    }

    #[test]
    fn retention_requires_the_same_physical_selection() {
        let old = config("hid:panel-a", [0, 0, 0]);
        assert!(same_selection(&old, &config("panel-a", [255, 0, 0])));
        assert!(!same_selection(&old, &config("hid:panel-b", [0, 0, 0])));
    }

    #[test]
    fn offline_media_waits_and_then_uses_the_resolved_panel_dimensions() {
        let root = tempfile::tempdir().unwrap();
        let mut service = ServiceManager::new(
            root.path().join("config.json"),
            root.path().join("daemon.sock"),
            lianli_shared::daemon::DaemonMode::User,
        )
        .unwrap();
        let config: LcdConfig = serde_json::from_value(
            serde_json::json!({ "type": "color", "index": 0, "rgb": [255, 0, 0] }),
        )
        .unwrap();
        service.config = Some(lianli_shared::config::AppConfig {
            lcds: vec![config],
            ..Default::default()
        });
        let (tx, _rx) = std::sync::mpsc::channel();
        service.tx = Some(tx.clone());
        service.prepare_media_assets(tx.clone());
        assert!(!service.media_preparation.is_busy());
        assert!(service.media_assets.is_empty());
        assert_eq!(
            service.ipc.state.lock().telemetry.media_preparation[&0].state,
            MediaPreparationState::WaitingForDevice
        );

        service.media_targets.insert(
            0,
            MediaTarget {
                selection: "index:0".into(),
                ..target()
            },
        );
        service
            .pixel_clean_sessions
            .push(crate::pixel_cleaner::PixelCleanSession {
                session_id: 1,
                duration_minutes: 1,
                original_targets: Vec::new(),
                clean_until: Instant::now() + std::time::Duration::from_secs(60),
            });
        service.prepare_media_assets(tx);
        assert!(!service.media_preparation.is_busy());
        assert!(service.media_reload_pending);
        service.ipc.state.lock().media_retry_pending = true;
        service.poll_prepared_media();
        assert!(service.ipc.state.lock().media_retry_pending);
        service.pixel_clean_sessions.clear();
        service.poll_prepared_media();
        assert!(!service.ipc.state.lock().media_retry_pending);
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        while service.media_preparation.is_busy() && Instant::now() < deadline {
            service.poll_prepared_media();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!service.media_preparation.is_busy());
        let MediaAssetKind::Static { frame } = &service.media_assets[&0].kind else {
            panic!("expected static frame");
        };
        let image = image::load_from_memory(frame).unwrap();
        assert_eq!((image.width(), image.height()), (480, 480));
        assert_eq!(service.media_assets[&0].stream_fps, 24.0);
        assert_eq!(
            service.media_asset_targets[&0].screen,
            ScreenInfo::AIO_LCD_480
        );
        let healthy = service.media_assets[&0].clone();
        service.ipc.state.lock().media_retry_pending = true;
        service.poll_prepared_media();
        assert!(!service.ipc.state.lock().media_retry_pending);
        assert!(Arc::ptr_eq(&healthy, &service.media_assets[&0]));
        assert!(!service.media_preparation.is_busy());
    }

    #[test]
    fn repaired_media_retries_without_changing_saved_settings() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("repaired.png");
        let mut service = ServiceManager::new(
            root.path().join("config.json"),
            root.path().join("daemon.sock"),
            lianli_shared::daemon::DaemonMode::User,
        )
        .unwrap();
        let lcd = serde_json::from_value(serde_json::json!({
            "type": "image", "serial": "panel-a", "path": path,
        }))
        .unwrap();
        service.config = Some(lianli_shared::config::AppConfig {
            lcds: vec![lcd],
            ..Default::default()
        });
        let saved = serde_json::to_value(&service.config).unwrap();
        service.media_targets.insert(0, target());
        let (tx, _rx) = std::sync::mpsc::channel();
        service.tx = Some(tx.clone());
        let finish = |service: &mut ServiceManager| {
            let deadline = Instant::now() + std::time::Duration::from_secs(3);
            while service.media_preparation.is_busy() && Instant::now() < deadline {
                service.poll_prepared_media();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(!service.media_preparation.is_busy());
        };
        service.prepare_media_assets(tx);
        finish(&mut service);
        assert_eq!(
            service.ipc.state.lock().telemetry.media_preparation[&0].state,
            MediaPreparationState::Failed
        );
        image::RgbImage::from_pixel(8, 8, image::Rgb([40, 80, 120]))
            .save(&path)
            .unwrap();
        service.ipc.state.lock().media_retry_pending = true;
        service.poll_prepared_media();
        finish(&mut service);
        assert_eq!(
            service.ipc.state.lock().telemetry.media_preparation[&0].state,
            MediaPreparationState::Ready
        );
        assert!(service.media_assets.contains_key(&0));
        assert_eq!(serde_json::to_value(&service.config).unwrap(), saved);
        assert!(!root.path().join("config.json").exists());
    }

    #[test]
    fn hardware_video_changes_media_identity_in_both_directions() {
        let device = serde_json::from_value(serde_json::json!({
            "type": "video", "path": "/example/video.mp4"
        }))
        .unwrap();
        let software = super::asset_cache_key(&device, &[], 30.0, false);
        let hardware = super::asset_cache_key(&device, &[], 30.0, true);
        assert_ne!(software, hardware);
        assert_eq!(software, super::asset_cache_key(&device, &[], 30.0, false));
    }

    #[test]
    fn disabling_h264_reprepares_the_panel_media_without_it() {
        let root = tempfile::tempdir().unwrap();
        let mut service = ServiceManager::new(
            root.path().join("config.json"),
            root.path().join("daemon.sock"),
            lianli_shared::daemon::DaemonMode::User,
        )
        .unwrap();
        let lcd = config("hid:1cbe:a021:1-3.2", [255, 0, 0]);
        service.config = Some(lianli_shared::config::AppConfig {
            lcds: vec![lcd],
            ..Default::default()
        });
        service.media_targets.insert(
            0,
            MediaTarget {
                selection: "serial:1cbe:a021:1-3.2".into(),
                device_id: "hid:1cbe:a021:1-3.2".into(),
                screen: ScreenInfo::HYDROSHIFT2,
            },
        );
        let (tx, _rx) = std::sync::mpsc::channel();
        service.tx = Some(tx.clone());
        let prepare = |service: &mut ServiceManager| {
            service.prepare_media_assets(tx.clone());
            let deadline = Instant::now() + std::time::Duration::from_secs(3);
            while service.media_preparation.is_busy() && Instant::now() < deadline {
                service.poll_prepared_media();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(!service.media_preparation.is_busy());
            (
                service.media_assets[&0].config_key.clone(),
                service.media_asset_h264[&0],
            )
        };
        let (h264_key, h264) = prepare(&mut service);
        assert!(h264);
        service
            .config
            .as_mut()
            .unwrap()
            .h264_disabled
            .insert("usb:1-3.2".into());
        let (jpeg_key, h264) = prepare(&mut service);
        assert!(!h264);
        assert_ne!(h264_key, jpeg_key);
        assert_eq!(service.media_targets[&0].screen, ScreenInfo::HYDROSHIFT2);
    }

    #[test]
    fn all_streamable_lcds_have_backends() {
        let mut seen = std::collections::HashSet::new();
        for entry in KNOWN_DEVICES {
            if !seen.insert(entry.family) {
                continue;
            }
            if super::is_streamable_lcd(entry.family) {
                assert!(screen_info_for(entry.family).is_some());
                assert!(
                    super::lcd_backend_kind(entry.family).is_some(),
                    "{:?} is streamable but lcd_backend_kind returns None",
                    entry.family
                );
            }
        }
    }
}
