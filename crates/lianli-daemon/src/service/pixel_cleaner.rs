use super::media::lcd_id_matches;
use super::ServiceManager;
use crate::pixel_cleaner::{PixelCleanSession, SavedTargetState};
use lianli_media::MediaAsset;
use lianli_shared::config::LcdConfig;
use lianli_shared::screen::ScreenInfo;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tracing::warn;

fn next_session_id() -> Result<u64, String> {
    use std::io::Read;
    let mut bytes = [0_u8; 8];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|e| format!("Creating pixel cleaner session ID: {e}"))?;
    Ok((u64::from_ne_bytes(bytes) & ((1 << 53) - 1)).max(1))
}
const MAX_TARGETS: usize = 16;
const PREPARATION_LIFETIME: Duration = Duration::from_secs(90);

#[derive(Clone)]
struct PlannedTarget {
    index: usize,
    identity: String,
    screen: ScreenInfo,
    payload_limit: usize,
    orientation: f32,
    previous: Arc<MediaAsset>,
}

pub(super) struct PixelCleanPreparation {
    id: u64,
    minutes: u16,
    targets: Vec<PlannedTarget>,
    cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<Result<Vec<Arc<MediaAsset>>, String>>>,
    assets: Option<Vec<Arc<MediaAsset>>>,
    expires: Instant,
}

impl Drop for PixelCleanPreparation {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let until = Instant::now() + Duration::from_secs(2);
            while !worker.is_finished() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(10));
            }
            if worker.is_finished() {
                if worker.join().is_err() {
                    warn!("Pixel cleaner preparation panicked");
                }
            } else {
                // It owns only temporary media; cancellation prevents activation or device I/O.
                warn!("Pixel cleaner preparation is still stopping; detaching");
            }
        }
    }
}

pub(super) fn target_matches(
    target_id: &str,
    idx: usize,
    device_identity: &str,
    config: Option<&lianli_shared::config::AppConfig>,
) -> bool {
    if let Some((prefix, idx_str)) = target_id.rsplit_once('#') {
        if let Ok(target_idx) = idx_str.parse::<usize>() {
            if target_idx != idx {
                return false;
            }
            if prefix.is_empty() {
                return true;
            }
            if lcd_id_matches(prefix, device_identity) || device_identity == prefix {
                return true;
            }
            if let Some(cfg) = config {
                if let Some(lcd_cfg) = cfg.lcds.get(idx) {
                    if lcd_cfg.device_id() == prefix {
                        return true;
                    }
                    if let Some(ref serial) = lcd_cfg.serial {
                        if lcd_id_matches(prefix, serial) || serial == prefix {
                            return true;
                        }
                    }
                }
            }
            return false;
        }
    }

    if let Ok(target_idx) = target_id.parse::<usize>() {
        return target_idx == idx;
    }
    if let Some(rest) = target_id
        .strip_prefix("index:")
        .or_else(|| target_id.strip_prefix("lcd:"))
    {
        if let Ok(target_idx) = rest.parse::<usize>() {
            return target_idx == idx;
        }
    }

    if let Some(cfg) = config {
        if let Some(lcd_cfg) = cfg.lcds.get(idx) {
            if lcd_cfg.device_id() == target_id {
                return true;
            }
            if let Some(ref serial) = lcd_cfg.serial {
                if lcd_id_matches(target_id, serial) || serial == target_id {
                    return true;
                }
            }
        }
    }

    lcd_id_matches(target_id, device_identity) || device_identity == target_id
}

impl ServiceManager {
    pub(super) fn start_pixel_cleaning(
        &mut self,
        target_id: Option<String>,
        minutes: u16,
    ) -> Result<u64, String> {
        if minutes == 0 {
            return Err("Duration must be positive".into());
        }
        if self.pixel_clean_preparation.is_some() {
            return Err("Another pixel cleaner preparation is pending".into());
        }
        let targets = self
            .targets
            .try_lock_for(Duration::from_millis(100))
            .ok_or("LCD targets are busy; retry shortly")?;
        let planned: Vec<_> = targets
            .iter()
            .filter(|(index, target)| {
                target_id.as_ref().is_none_or(|id| {
                    target_matches(id, **index, &target.device_identity, self.config.as_ref())
                })
            })
            .map(|(&index, target)| PlannedTarget {
                index,
                identity: target.device_identity.clone(),
                screen: target.screen,
                payload_limit: target.cleaner_payload_limit(),
                orientation: self
                    .config
                    .as_ref()
                    .and_then(|c| c.lcds.get(index))
                    .map_or(0.0, |c| c.orientation),
                previous: Arc::clone(&target.asset),
            })
            .collect();
        let cached: Vec<_> = targets
            .values()
            .filter_map(|target| {
                let payload_limit = target.cleaner_payload_limit();
                if !target.asset.config_key.starts_with("pixel_cleaner:")
                    || !target
                        .asset
                        .config_key
                        .ends_with(&format!(":{payload_limit}"))
                {
                    return None;
                }
                Some((
                    target.screen,
                    payload_limit,
                    self.config
                        .as_ref()
                        .and_then(|c| c.lcds.get(target.index))
                        .map_or(0.0, |c| c.orientation)
                        .to_bits(),
                    target.asset.clone(),
                ))
            })
            .collect();
        let mut affected: std::collections::HashSet<_> = self
            .pixel_clean_sessions
            .iter()
            .flat_map(|session| session.original_targets.iter().map(|t| t.target_index))
            .collect();
        affected.extend(planned.iter().map(|t| t.index));
        drop(targets);
        if affected.len() > MAX_TARGETS {
            return Err("At most 16 LCDs can be cleaned concurrently".into());
        }
        if planned.is_empty() {
            return Err("No active LCD targets match the request".into());
        }
        if planned.len() > MAX_TARGETS {
            return Err("Pixel cleaning supports at most 16 targets per request".into());
        }
        let id = next_session_id()?;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let worker_targets = planned.clone();
        let worker = std::thread::Builder::new()
            .name("pixel-cleaner-prepare".into())
            .spawn(move || {
                let mut cache = cached;
                let mut seen = std::collections::HashSet::new();
                cache.retain(|(_, _, _, asset)| seen.insert(Arc::as_ptr(asset)));
                let mut prepared = Vec::new();
                let mut total_bytes = cache
                    .iter()
                    .map(|(_, _, _, asset)| match &asset.kind {
                        lianli_media::MediaAssetKind::Video { frames, .. } => {
                            frames.iter().map(Vec::len).sum::<usize>()
                        }
                        lianli_media::MediaAssetKind::H264Stream { path, .. } => {
                            std::fs::metadata(path).map_or(0, |m| m.len() as usize)
                        }
                        _ => 0,
                    })
                    .sum::<usize>();
                for target in worker_targets {
                    if worker_cancel.load(Ordering::Relaxed) {
                        return Err("Preparation cancelled".into());
                    }
                    let rotation = target.orientation.to_bits();
                    let asset = if let Some((_, _, _, asset)) =
                        cache.iter().find(|(screen, limit, orientation, _)| {
                            *screen == target.screen
                                && *limit == target.payload_limit
                                && *orientation == rotation
                        }) {
                        Arc::clone(asset)
                    } else {
                        let kind = lianli_media::pixel_cleaner::prepare_asset(
                            &target.screen,
                            target.orientation,
                            target.payload_limit,
                            &worker_cancel,
                        )
                        .map_err(|e| format!("Preparing {}: {e}", target.identity))?;
                        total_bytes += match &kind {
                            lianli_media::MediaAssetKind::Video { frames, .. } => {
                                frames.iter().map(Vec::len).sum::<usize>()
                            }
                            lianli_media::MediaAssetKind::H264Stream { path, .. } => {
                                std::fs::metadata(path).map_err(|e| e.to_string())?.len() as usize
                            }
                            _ => 0,
                        };
                        if total_bytes > 64 * 1024 * 1024 {
                            return Err("Pixel cleaner preparation exceeds 64 MiB budget".into());
                        }
                        let asset = Arc::new(MediaAsset {
                            kind,
                            config_key: format!(
                                "pixel_cleaner:{id}:{}:{}",
                                target.index, target.payload_limit
                            ),
                            stream_fps: lianli_media::pixel_cleaner::FPS as f32,
                        });
                        cache.push((
                            target.screen,
                            target.payload_limit,
                            rotation,
                            Arc::clone(&asset),
                        ));
                        asset
                    };
                    prepared.push(asset);
                }
                Ok(prepared)
            })
            .map_err(|e| format!("Starting pixel cleaner preparation: {e}"))?;
        self.pixel_clean_preparation = Some(PixelCleanPreparation {
            id,
            minutes: minutes.min(lianli_shared::ipc::MAX_CLEAN_MINUTES),
            targets: planned,
            cancel,
            worker: Some(worker),
            assets: None,
            expires: Instant::now() + PREPARATION_LIFETIME,
        });
        self.ipc.state.lock().pixel_clean_preparation = Some((id, false, None));
        Ok(id)
    }

    pub(super) fn activate_pixel_cleaning(&mut self, id: u64) -> Result<u64, String> {
        self.poll_pixel_clean_preparation();
        let pending = self
            .pixel_clean_preparation
            .as_ref()
            .filter(|p| p.id == id)
            .ok_or("Unknown pixel cleaner preparation")?;
        let assets = pending
            .assets
            .as_ref()
            .ok_or("Pixel cleaner is not ready")?;
        let mut targets = self
            .targets
            .try_lock_for(Duration::from_millis(100))
            .ok_or("LCD targets are busy; retry shortly")?;
        for plan in &pending.targets {
            let current = targets
                .get(&plan.index)
                .ok_or("LCD disappeared while preparing; previous session preserved")?;
            if current.device_identity != plan.identity
                || !Arc::ptr_eq(&current.asset, &plan.previous)
                || current.cleaner_payload_limit() != plan.payload_limit
            {
                return Err("LCD changed while preparing; previous session preserved".into());
            }
        }
        let mut saved = Vec::new();
        for (plan, asset) in pending.targets.iter().zip(assets) {
            let current = targets.get_mut(&plan.index).expect("validated target");
            let prior = self.pixel_clean_sessions.iter_mut().find_map(|session| {
                session
                    .original_targets
                    .iter()
                    .position(|s| {
                        s.target_index == plan.index && s.device_identity == plan.identity
                    })
                    .map(|i| session.original_targets.remove(i))
            });
            saved.push(prior.unwrap_or_else(|| {
                SavedTargetState {
                    target_index: plan.index,
                    device_identity: plan.identity.clone(),
                    media_asset: current.asset.clone(),
                    custom_h264: current.custom_h264,
                    original_brightness: Some(
                        self.config
                            .as_ref()
                            .and_then(|c| c.lcds.get(plan.index))
                            .map_or(100, LcdConfig::brightness),
                    ),
                }
            }));
            current.swap_media(Arc::clone(asset), plan.screen.h264, self.tx.clone());
            current.apply_brightness(Some(&self.wireless), &mut self.packet_builder, 75);
            self.media_assets.insert(plan.index, Arc::clone(asset));
        }
        drop(targets);
        self.pixel_clean_sessions
            .retain(|s| !s.original_targets.is_empty());
        self.pixel_clean_sessions.push(PixelCleanSession {
            session_id: id,
            duration_minutes: pending.minutes,
            clean_until: Instant::now() + Duration::from_secs(u64::from(pending.minutes) * 60),
            original_targets: saved,
        });
        self.pixel_clean_preparation = None;
        self.ipc.state.lock().pixel_clean_preparation = None;
        self.sync_cleaner_ipc_state();
        Ok(id)
    }

    fn poll_pixel_clean_preparation(&mut self) {
        let Some(pending) = self.pixel_clean_preparation.as_mut() else {
            return;
        };
        if Instant::now() >= pending.expires {
            self.cancel_pixel_clean_preparation();
            return;
        }
        if pending.worker.as_ref().is_some_and(JoinHandle::is_finished) {
            let worker = pending.worker.take().expect("finished worker");
            match worker
                .join()
                .unwrap_or_else(|_| Err("Pixel cleaner preparation panicked".into()))
            {
                Ok(assets) => {
                    pending.assets = Some(assets);
                    self.ipc.state.lock().pixel_clean_preparation = Some((pending.id, true, None));
                }
                Err(error) => {
                    self.ipc.state.lock().pixel_clean_preparation =
                        Some((pending.id, false, Some(error)));
                }
            }
        }
    }

    pub(super) fn cancel_pixel_clean_preparation(&mut self) {
        self.pixel_clean_preparation = None;
        self.ipc.state.lock().pixel_clean_preparation = None;
    }

    fn sync_cleaner_ipc_state(&self) {
        let mut state = self.ipc.state.lock();
        state.pixel_clean_states = self
            .pixel_clean_sessions
            .iter()
            .flat_map(|session| {
                session
                    .original_targets
                    .iter()
                    .map(|target| crate::ipc::PixelCleanState {
                        session_id: session.session_id,
                        device_id: Some(format!(
                            "{}#{}",
                            target.device_identity, target.target_index
                        )),
                        duration_minutes: session.duration_minutes,
                        clean_until: session.clean_until,
                    })
            })
            .collect();
        state.telemetry.pixel_clean_statuses = state.pixel_clean_statuses();
    }

    pub(super) fn stop_pixel_cleaning(
        &mut self,
        target_id: Option<String>,
        session_id: Option<u64>,
    ) -> bool {
        let Some(id) = session_id else { return false };
        if self
            .pixel_clean_preparation
            .as_ref()
            .is_some_and(|p| p.id == id)
        {
            self.cancel_pixel_clean_preparation();
            return true;
        }
        self.restore_cleaner_targets(|session, saved, cfg| {
            session == id
                && target_id.as_ref().is_none_or(|target| {
                    target_matches(target, saved.target_index, &saved.device_identity, cfg)
                })
        })
        .unwrap_or(false)
    }

    fn restore_cleaner_targets(
        &mut self,
        matches: impl Fn(u64, &SavedTargetState, Option<&lianli_shared::config::AppConfig>) -> bool,
    ) -> Result<bool, String> {
        if self.pixel_clean_sessions.is_empty() {
            return Ok(false);
        }
        let targets_handle = Arc::clone(&self.targets);
        let mut targets = targets_handle
            .try_lock_for(Duration::from_millis(100))
            .ok_or("LCD targets are busy; cleaner restoration deferred")?;
        let mut restore = Vec::new();
        for session in &mut self.pixel_clean_sessions {
            let mut retained = Vec::new();
            for saved in session.original_targets.drain(..) {
                if matches(session.session_id, &saved, self.config.as_ref()) {
                    restore.push(saved);
                } else {
                    retained.push(saved);
                }
            }
            session.original_targets = retained;
        }
        self.pixel_clean_sessions
            .retain(|s| !s.original_targets.is_empty());
        let mut stopped = false;
        for saved in restore {
            if let Some(target) = targets.get_mut(&saved.target_index) {
                if target.device_identity != saved.device_identity {
                    continue;
                }
                target.swap_media(
                    saved.media_asset.clone(),
                    saved.custom_h264,
                    self.tx.clone(),
                );
                target.apply_brightness(
                    Some(&self.wireless),
                    &mut self.packet_builder,
                    saved.original_brightness.unwrap_or(100),
                );
                stopped = true;
            }
            self.media_assets
                .insert(saved.target_index, saved.media_asset);
        }
        drop(targets);
        self.sync_cleaner_ipc_state();
        Ok(stopped)
    }

    pub(super) fn force_stop_pixel_cleaning(&mut self, target_id: Option<String>) -> bool {
        self.cancel_pixel_clean_preparation();
        self.restore_cleaner_targets(|_, saved, cfg| {
            target_id.as_ref().is_none_or(|id| {
                target_matches(id, saved.target_index, &saved.device_identity, cfg)
            })
        })
        .is_ok()
    }

    pub(super) fn check_pixel_clean_sessions(&mut self) {
        self.poll_pixel_clean_preparation();
        if self.pixel_clean_sessions.is_empty() {
            return;
        }
        let now = Instant::now();
        let expired: Vec<_> = self
            .pixel_clean_sessions
            .iter()
            .filter(|s| now >= s.clean_until)
            .map(|s| s.session_id)
            .collect();
        let Some(targets) = self.targets.try_lock() else {
            return;
        };
        let current: Vec<_> = targets
            .iter()
            .map(|(&index, target)| (index, target.device_identity.clone()))
            .collect();
        drop(targets);
        if self.pixel_clean_sessions.iter().any(|s| {
            expired.contains(&s.session_id)
                || s.original_targets.iter().any(|saved| {
                    !current.contains(&(saved.target_index, saved.device_identity.clone()))
                })
        }) {
            let _ = self.restore_cleaner_targets(|id, saved, _| {
                expired.contains(&id)
                    || !current.contains(&(saved.target_index, saved.device_identity.clone()))
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::runtime::{ActiveTarget, HidLcd, LcdBackend};
    use lianli_devices::traits::LcdDevice;
    use lianli_media::MediaAssetKind;

    struct TestLcd;
    impl LcdDevice for TestLcd {
        fn screen_info(&self) -> &ScreenInfo {
            &ScreenInfo::TLLCD
        }
        fn send_jpeg_frame(&mut self, _: &[u8]) -> anyhow::Result<()> {
            Ok(())
        }
        fn set_brightness(&self, _: u8) -> anyhow::Result<()> {
            Ok(())
        }
        fn set_rotation(&self, _: u16) -> anyhow::Result<()> {
            Ok(())
        }
        fn initialize(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
    }

    fn asset(key: &str) -> Arc<MediaAsset> {
        Arc::new(MediaAsset {
            kind: MediaAssetKind::Static {
                frame: Arc::new(vec![1]),
            },
            config_key: key.into(),
            stream_fps: 20.0,
        })
    }

    fn service() -> ServiceManager {
        let mut service = ServiceManager::new(
            "/tmp/unused-clean-config".into(),
            "/tmp/unused-clean-socket".into(),
        )
        .unwrap();
        for index in 0..2 {
            let asset = asset(&format!("original-{index}"));
            let target = ActiveTarget::new(
                index,
                asset.config_key.clone(),
                format!("hid:device-{index}"),
                LcdBackend::HidLcd(Arc::new(HidLcd::new(Box::new(TestLcd)))),
                asset.clone(),
                ScreenInfo::TLLCD,
                false,
                None,
            );
            service.targets.lock().insert(index, target);
            service.media_assets.insert(index, asset);
        }
        service
    }

    fn ready(service: &mut ServiceManager, id: u64, indices: &[usize]) {
        let targets = service.targets.lock();
        let planned = indices
            .iter()
            .map(|&index| {
                let target = &targets[&index];
                PlannedTarget {
                    index,
                    identity: target.device_identity.clone(),
                    screen: target.screen,
                    payload_limit: target.cleaner_payload_limit(),
                    orientation: 0.0,
                    previous: target.asset.clone(),
                }
            })
            .collect();
        service.pixel_clean_preparation = Some(PixelCleanPreparation {
            id,
            minutes: 1,
            targets: planned,
            cancel: Arc::new(AtomicBool::new(false)),
            worker: None,
            assets: Some(
                indices
                    .iter()
                    .map(|_| asset(&format!("clean-{id}")))
                    .collect(),
            ),
            expires: Instant::now() + PREPARATION_LIFETIME,
        });
    }

    #[test]
    fn partial_stop_keeps_original_token_and_reports_only_remaining_targets() {
        let mut service = service();
        ready(&mut service, 10, &[0, 1]);
        service.activate_pixel_cleaning(10).unwrap();
        assert!(!service.stop_pixel_cleaning(None, None));
        assert!(!service.stop_pixel_cleaning(None, Some(99)));
        assert!(!service.stop_pixel_cleaning(Some("nonexistent".into()), Some(10)));
        assert!(service.stop_pixel_cleaning(Some("index:0".into()), Some(10)));
        let statuses = service.ipc.state.lock().pixel_clean_statuses();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses["hid:device-1#1"].session_id, Some(10));
        assert!(service.stop_pixel_cleaning(None, Some(10)));
        assert!(service.ipc.state.lock().pixel_clean_statuses().is_empty());
        assert_eq!(service.targets.lock()[&0].asset.config_key, "original-0");
        assert_eq!(service.targets.lock()[&1].asset.config_key, "original-1");
    }

    #[test]
    fn replacement_inherits_original_media_without_claiming_other_targets() {
        let mut service = service();
        ready(&mut service, 10, &[0, 1]);
        service.activate_pixel_cleaning(10).unwrap();
        ready(&mut service, 11, &[0]);
        service.activate_pixel_cleaning(11).unwrap();
        let statuses = service.ipc.state.lock().pixel_clean_statuses();
        assert_eq!(statuses["hid:device-0#0"].session_id, Some(11));
        assert_eq!(statuses["hid:device-1#1"].session_id, Some(10));
        assert!(service.stop_pixel_cleaning(None, Some(10)));
        assert_eq!(service.targets.lock()[&0].asset.config_key, "clean-11");
        assert!(service.stop_pixel_cleaning(None, Some(11)));
        assert_eq!(service.targets.lock()[&0].asset.config_key, "original-0");
    }

    #[test]
    fn missing_target_during_replacement_preserves_previous_session() {
        let mut service = service();
        ready(&mut service, 10, &[0]);
        service.activate_pixel_cleaning(10).unwrap();
        ready(&mut service, 11, &[0, 1]);
        service.targets.lock().remove(&1);
        assert!(service.activate_pixel_cleaning(11).is_err());
        assert_eq!(service.pixel_clean_sessions[0].session_id, 10);
        assert_eq!(service.targets.lock()[&0].asset.config_key, "clean-10");
    }

    #[test]
    fn expired_or_cancelled_preparation_cannot_activate() {
        let mut service = service();
        ready(&mut service, 10, &[0]);
        service.pixel_clean_preparation.as_mut().unwrap().expires = Instant::now();
        assert!(service.activate_pixel_cleaning(10).is_err());
        ready(&mut service, 11, &[0]);
        assert!(service.stop_pixel_cleaning(None, Some(11)));
        assert!(service.activate_pixel_cleaning(11).is_err());
        assert_eq!(service.targets.lock()[&0].asset.config_key, "original-0");
    }

    #[test]
    fn restored_media_is_never_sent_to_a_reused_slot() {
        let mut service = service();
        ready(&mut service, 10, &[0]);
        service.activate_pixel_cleaning(10).unwrap();
        service.targets.lock().get_mut(&0).unwrap().device_identity = "hid:replacement".into();
        assert!(!service.stop_pixel_cleaning(None, Some(10)));
        assert_eq!(service.targets.lock()[&0].asset.config_key, "clean-10");
        assert!(service.pixel_clean_sessions.is_empty());
    }

    #[test]
    fn changed_payload_budget_rejects_activation_without_replacing_the_session() {
        let mut service = service();
        ready(&mut service, 10, &[0]);
        service.activate_pixel_cleaning(10).unwrap();
        ready(&mut service, 11, &[0]);
        service
            .targets
            .lock()
            .get_mut(&0)
            .unwrap()
            .screen
            .max_payload /= 2;
        assert!(service.activate_pixel_cleaning(11).is_err());
        assert_eq!(service.pixel_clean_sessions[0].session_id, 10);
        assert_eq!(service.pixel_clean_sessions[0].original_targets.len(), 1);
    }

    #[test]
    fn busy_stop_keeps_session_and_saved_media_for_retry() {
        let mut service = service();
        ready(&mut service, 10, &[0]);
        service.activate_pixel_cleaning(10).unwrap();
        let targets = service.targets.clone();
        let guard = targets.lock();
        let started = Instant::now();
        assert!(!service.stop_pixel_cleaning(None, Some(10)));
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(service.pixel_clean_sessions[0].session_id, 10);
        assert_eq!(service.pixel_clean_sessions[0].original_targets.len(), 1);
        assert!(!service.force_stop_pixel_cleaning(None));
        drop(guard);
        assert!(service.stop_pixel_cleaning(None, Some(10)));
        assert_eq!(service.targets.lock()[&0].asset.config_key, "original-0");
    }

    #[test]
    fn force_stop_for_reload_clears_preparation_and_active_sessions() {
        let mut service = service();
        ready(&mut service, 10, &[0]);
        service.activate_pixel_cleaning(10).unwrap();
        ready(&mut service, 11, &[1]);
        service.force_stop_pixel_cleaning(None);
        assert!(service.pixel_clean_preparation.is_none());
        assert!(service.pixel_clean_sessions.is_empty());
        assert_eq!(service.targets.lock()[&0].asset.config_key, "original-0");
    }

    #[test]
    fn invalid_start_never_changes_an_active_session() {
        let mut service = service();
        ready(&mut service, 10, &[0]);
        service.activate_pixel_cleaning(10).unwrap();
        assert!(service
            .start_pixel_cleaning(None, 0)
            .unwrap_err()
            .contains("Duration"));
        assert!(service
            .start_pixel_cleaning(Some("missing".into()), 1)
            .is_err());
        assert_eq!(service.pixel_clean_sessions[0].session_id, 10);
    }

    #[test]
    fn test_target_matches_compound_index() {
        use super::target_matches;

        assert!(target_matches("hid:1-2:1.0#0", 0, "hid:1-2:1.0", None));
        assert!(!target_matches("hid:1-2:1.0#0", 1, "hid:1-2:1.0", None));

        assert!(!target_matches("hid:1-2:1.0#1", 0, "hid:1-2:1.0", None));
        assert!(target_matches("hid:1-2:1.0#1", 1, "hid:1-2:1.0", None));

        assert!(target_matches("#0", 0, "hid:1-2:1.0", None));
        assert!(!target_matches("#0", 1, "hid:1-2:1.0", None));
    }

    #[test]
    fn test_target_matches_direct_index() {
        use super::target_matches;

        assert!(target_matches("0", 0, "hid:1-2:1.0", None));
        assert!(!target_matches("0", 1, "hid:1-2:1.0", None));

        assert!(target_matches("index:1", 1, "hid:1-2:1.0", None));
        assert!(!target_matches("index:1", 0, "hid:1-2:1.0", None));

        assert!(target_matches("lcd:2", 2, "hid:1-2:1.0", None));
        assert!(!target_matches("lcd:2", 1, "hid:1-2:1.0", None));
    }

    #[test]
    fn test_target_matches_device_identity() {
        use super::target_matches;

        assert!(target_matches("hid:1-2:1.0", 0, "hid:1-2:1.0", None));
        assert!(target_matches("1-2:1.0", 0, "hid:1-2:1.0", None));
        assert!(!target_matches("hid:9-9:1.0", 0, "hid:1-2:1.0", None));
    }

    #[test]
    fn test_target_matches_compound_invalid_prefix_returns_false() {
        use super::target_matches;

        assert!(!target_matches("nonexistent#0", 0, "hid:1-2:1.0", None));
        assert!(!target_matches("other_device#0", 0, "hid:1-2:1.0", None));
    }
}
