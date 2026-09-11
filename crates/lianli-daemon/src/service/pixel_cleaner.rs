use super::media::lcd_id_matches;
use super::{DaemonEvent, ServiceManager};
use crate::pixel_cleaner::{pixel_cleaner_asset_path, PixelCleanSession, SavedTargetState};
use lianli_media::MediaAsset;
use lianli_shared::config::LcdConfig;
use lianli_shared::screen::ScreenInfo;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, warn};

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

/// Helper to check if a requested target string matches an active LCD target.
/// Supports:
/// - Compound device-and-index format: `"<device_id>#<idx>"` (used by GUI cards)
/// - Config index: `"0"`, `"1"`, `"index:0"`, `"lcd:0"`
/// - Config-based serial or device_id: `"serial:XYZ"`, `"XYZ"`
/// - USB device identity: `"hid:1-2:1.0"`, `"1-2:1.0"`
pub(super) fn target_matches(
    target_id: &str,
    idx: usize,
    device_identity: &str,
    config: Option<&lianli_shared::config::AppConfig>,
) -> bool {
    // 1. Compound target with index: "<device_id>#<idx>"
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

    // 2. Direct index matching: "0", "1", "index:0", "lcd:0"
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

    // 3. Config-based device ID or serial matching
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

    // 4. Device identity matching
    lcd_id_matches(target_id, device_identity) || device_identity == target_id
}

impl ServiceManager {
    pub(super) fn start_pixel_cleaning(
        &mut self,
        target_dev_id: Option<String>,
        minutes: u16,
    ) -> Result<u64, String> {
        let original_minutes = minutes;
        let minutes = minutes.clamp(1, lianli_shared::ipc::MAX_CLEAN_MINUTES);
        if minutes != original_minutes {
            info!("Pixel cleaner duration {original_minutes} clamped to {minutes} minutes");
        }

        let cleaner_path = pixel_cleaner_asset_path();
        if !cleaner_path.exists() {
            let msg = format!(
                "Pixel cleaner asset does not exist at {}",
                cleaner_path.display()
            );
            warn!("{msg}");
            return Err(msg);
        }

        let target_info: Vec<(usize, String, ScreenInfo, bool)> = {
            let targets = self.targets.lock();
            let cfg = self.config.as_ref();
            targets
                .iter()
                .filter_map(|(idx, target)| {
                    if let Some(ref target_id) = target_dev_id {
                        if !target_matches(target_id, *idx, &target.device_identity, cfg) {
                            return None;
                        }
                    }
                    Some((
                        *idx,
                        target.device_identity.clone(),
                        target.screen,
                        target.custom_h264,
                    ))
                })
                .collect()
        };

        if target_info.is_empty() {
            let msg = format!("No active LCD targets found matching {:?}", target_dev_id);
            warn!("{msg}");
            return Err(msg);
        }

        let mut prepared = Vec::new();
        let mut asset_cache: std::collections::HashMap<(u32, u32, bool, i32), Arc<MediaAsset>> =
            std::collections::HashMap::new();

        for (idx, device_identity, screen, custom_h264) in target_info {
            let orig_orientation = self
                .config
                .as_ref()
                .and_then(|cfg| cfg.lcds.get(idx).map(|l| l.orientation))
                .unwrap_or(0.0);

            let cache_key = (
                screen.width,
                screen.height,
                screen.h264,
                (orig_orientation * 10.0) as i32,
            );

            let clean_asset = if let Some(cached) = asset_cache.get(&cache_key) {
                Arc::clone(cached)
            } else {
                let clean_cfg = LcdConfig {
                    index: Some(idx),
                    serial: Some(device_identity.clone()),
                    media_type: lianli_shared::media::MediaType::Video,
                    path: Some(cleaner_path.clone()),
                    fps: Some(30.0),
                    update_interval_ms: None,
                    rgb: None,
                    orientation: orig_orientation,
                    sensor: None,
                    sensor_source_1: Default::default(),
                    sensor_source_2: Default::default(),
                    doublegauge: None,
                    template_id: None,
                    smooth_edges: None,
                    custom_h264: Some(true),
                    aio_512_frame: None,
                    brightness: Some(75),
                };

                let asset_kind = match lianli_media::prepare_media_asset(
                    &clean_cfg,
                    30.0,
                    &screen,
                    screen.h264,
                    &[],
                    &[],
                ) {
                    Ok(k) => k,
                    Err(e) => {
                        warn!(
                            "Failed to prepare pixel cleaner asset for target {}: {e}",
                            device_identity
                        );
                        continue;
                    }
                };

                let stream_fps = match &asset_kind {
                    lianli_media::MediaAssetKind::Custom { asset } => asset.render_fps(),
                    _ => 30.0_f32.min(screen.max_fps as f32).max(1.0),
                };
                let asset = Arc::new(MediaAsset {
                    kind: asset_kind,
                    config_key: format!("pixel_cleaner_{idx}"),
                    stream_fps,
                });
                asset_cache.insert(cache_key, Arc::clone(&asset));
                asset
            };

            prepared.push((idx, device_identity, screen, custom_h264, clean_asset));
        }

        if prepared.is_empty() {
            let msg = "Failed to prepare pixel cleaner asset for target LCDs".to_string();
            warn!("{msg}");
            return Err(msg);
        }

        // If any of the requested targets already have an active conditioning session,
        // stop that target from its existing session first without affecting other targets.
        for (idx, _, _, _, _) in &prepared {
            self.stop_target_from_sessions(*idx);
        }

        let session_id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
        let mut saved_targets = Vec::new();

        for (idx, device_identity, screen, custom_h264, clean_asset) in prepared {
            self.media_assets.insert(idx, Arc::clone(&clean_asset));

            let orig_brightness = self
                .config
                .as_ref()
                .and_then(|cfg| {
                    cfg.lcds
                        .iter()
                        .find(|l| {
                            l.serial
                                .as_deref()
                                .map_or(false, |s| lcd_id_matches(s, &device_identity))
                        })
                        .and_then(|l| l.brightness)
                        .or_else(|| cfg.lcds.get(idx).and_then(|l| l.brightness))
                        .or_else(|| cfg.aio.get(&device_identity).map(|a| a.brightness))
                })
                .unwrap_or(75);

            let mut targets = self.targets.lock();
            if let Some(target) = targets.get_mut(&idx) {
                saved_targets.push(SavedTargetState {
                    target_index: idx,
                    device_identity: device_identity.clone(),
                    media_asset: Arc::clone(&target.asset),
                    custom_h264,
                    original_brightness: Some(orig_brightness),
                });

                target.swap_media(clean_asset, screen.h264, self.tx.clone());
                target.apply_brightness(Some(&self.wireless), &mut self.packet_builder, 75);
                info!(
                    "Started pixel cleaner on LCD[{device_identity}] for {minutes} minutes at 75% brightness"
                );
            }
        }

        if !saved_targets.is_empty() {
            let clean_until = Instant::now()
                .checked_add(Duration::from_secs((minutes as u64).saturating_mul(60)))
                .unwrap_or_else(|| Instant::now() + Duration::from_secs(u32::MAX as u64));
            self.pixel_clean_sessions.push(PixelCleanSession {
                session_id,
                target_id: target_dev_id,
                duration_minutes: minutes,
                original_targets: saved_targets,
                clean_until,
            });
            self.sync_cleaner_ipc_state();
            if let Some(ref tx) = self.tx {
                tx.send(DaemonEvent::FrameFinished).ok();
            }
            Ok(session_id)
        } else {
            let msg = "Failed to initialize pixel cleaner on target LCDs".to_string();
            warn!("{msg}");
            Err(msg)
        }
    }

    fn stop_target_from_sessions(&mut self, target_idx: usize) {
        let mut empty_indices = Vec::new();
        for (s_idx, session) in self.pixel_clean_sessions.iter_mut().enumerate() {
            let mut remaining = Vec::new();
            for saved in session.original_targets.drain(..) {
                if saved.target_index == target_idx {
                    self.media_assets
                        .insert(saved.target_index, Arc::clone(&saved.media_asset));
                    let mut targets = self.targets.lock();
                    if let Some(target) = targets.get_mut(&saved.target_index) {
                        target.swap_media(saved.media_asset, saved.custom_h264, self.tx.clone());
                        if let Some(orig_b) = saved.original_brightness {
                            target.apply_brightness(
                                Some(&self.wireless),
                                &mut self.packet_builder,
                                orig_b,
                            );
                        }
                        info!(
                            "Restored previous media and brightness on LCD[{}]",
                            target.device_identity
                        );
                    }
                } else {
                    remaining.push(saved);
                }
            }
            session.original_targets = remaining;
            if session.original_targets.is_empty() {
                empty_indices.push(s_idx);
            }
        }

        for &s_idx in empty_indices.iter().rev() {
            self.pixel_clean_sessions.remove(s_idx);
        }
        self.sync_cleaner_ipc_state();
    }

    fn sync_cleaner_ipc_state(&self) {
        let mut ipc_state = self.ipc.state.lock();
        ipc_state.pixel_clean_states.clear();
        ipc_state
            .pixel_clean_states
            .extend(self.pixel_clean_sessions.iter().map(|s| crate::ipc::PixelCleanState {
                session_id: s.session_id,
                device_id: s.target_id.clone(),
                duration_minutes: s.duration_minutes,
                clean_until: s.clean_until,
            }));
        ipc_state.telemetry.pixel_clean_statuses = ipc_state.pixel_clean_statuses();
    }

    pub(super) fn stop_pixel_cleaning(
        &mut self,
        target_dev_id: Option<String>,
        session_id: Option<u64>,
    ) -> bool {
        if self.pixel_clean_sessions.is_empty() {
            self.sync_cleaner_ipc_state();
            return false;
        }

        let cfg = self.config.as_ref();
        let mut stopped_any = false;
        let mut empty_indices = Vec::new();
        let mut new_split_sessions = Vec::new();

        for (s_idx, session) in self.pixel_clean_sessions.iter_mut().enumerate() {
            if let Some(sid) = session_id {
                if session.session_id != sid {
                    continue;
                }
            }

            let initial_count = session.original_targets.len();
            let is_global = session.target_id.is_none();
            let mut remaining = Vec::new();
            for saved in session.original_targets.drain(..) {
                let matches = match &target_dev_id {
                    Some(id) => {
                        target_matches(id, saved.target_index, &saved.device_identity, cfg)
                    }
                    None => true,
                };

                if matches {
                    stopped_any = true;
                    self.media_assets
                        .insert(saved.target_index, Arc::clone(&saved.media_asset));
                    let mut targets = self.targets.lock();
                    if let Some(target) = targets.get_mut(&saved.target_index) {
                        target.swap_media(saved.media_asset, saved.custom_h264, self.tx.clone());
                        if let Some(orig_b) = saved.original_brightness {
                            target.apply_brightness(
                                Some(&self.wireless),
                                &mut self.packet_builder,
                                orig_b,
                            );
                        }
                        info!(
                            "Restored previous media and brightness on LCD[{}]",
                            target.device_identity
                        );
                    }
                } else {
                    remaining.push(saved);
                }
            }

            if remaining.is_empty() {
                empty_indices.push(s_idx);
            } else if is_global && remaining.len() < initial_count {
                // Global session partially stopped: split remaining targets into targeted individual sessions
                // so "all" is no longer broadcast to the stopped screen.
                empty_indices.push(s_idx);
                for target in remaining {
                    new_split_sessions.push(PixelCleanSession {
                        session_id: NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed),
                        target_id: Some(format!("{}#{}", target.device_identity, target.target_index)),
                        duration_minutes: session.duration_minutes,
                        original_targets: vec![target],
                        clean_until: session.clean_until,
                    });
                }
            } else {
                session.original_targets = remaining;
            }
        }

        for &s_idx in empty_indices.iter().rev() {
            self.pixel_clean_sessions.remove(s_idx);
        }
        self.pixel_clean_sessions.extend(new_split_sessions);

        self.sync_cleaner_ipc_state();
        if let Some(ref tx) = self.tx {
            tx.send(DaemonEvent::FrameFinished).ok();
        }
        stopped_any
    }

    #[allow(dead_code)]
    pub(super) fn force_stop_pixel_cleaning(&mut self, target_dev_id: Option<String>) -> bool {
        self.stop_pixel_cleaning(target_dev_id, None)
    }

    pub(super) fn check_pixel_clean_sessions(&mut self) {
        if self.pixel_clean_sessions.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut expired = Vec::new();
        self.pixel_clean_sessions.retain_mut(|session| {
            if now >= session.clean_until {
                expired.append(&mut session.original_targets);
                false
            } else {
                true
            }
        });

        if !expired.is_empty() {
            for saved in expired {
                self.media_assets
                    .insert(saved.target_index, Arc::clone(&saved.media_asset));
                let mut targets = self.targets.lock();
                if let Some(target) = targets.get_mut(&saved.target_index) {
                    target.swap_media(saved.media_asset, saved.custom_h264, self.tx.clone());
                    if let Some(orig_b) = saved.original_brightness {
                        target.apply_brightness(
                            Some(&self.wireless),
                            &mut self.packet_builder,
                            orig_b,
                        );
                    }
                    info!(
                        "Pixel cleaner elapsed: restored previous media and brightness on LCD[{}]",
                        target.device_identity
                    );
                }
            }
            self.sync_cleaner_ipc_state();
            if let Some(ref tx) = self.tx {
                tx.send(DaemonEvent::FrameFinished).ok();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::pixel_cleaner::PixelCleanSession;
    use crate::service::ServiceManager;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    #[test]
    fn test_stop_pixel_cleaning_mismatched_session_returns_false() {
        let mut service = ServiceManager::new(
            PathBuf::from("/tmp/test_config.json"),
            PathBuf::from("/tmp/test_socket.sock"),
        )
        .expect("failed to instantiate ServiceManager");

        service.pixel_clean_sessions.push(PixelCleanSession {
            session_id: 12345,
            target_id: None,
            duration_minutes: 1,
            original_targets: Vec::new(),
            clean_until: Instant::now() + Duration::from_secs(60),
        });

        // Attempting to stop with a mismatched session ID should return false
        let stopped = service.stop_pixel_cleaning(None, Some(99999));
        assert!(!stopped);

        // Active session should remain untouched
        assert!(!service.pixel_clean_sessions.is_empty());
        assert_eq!(
            service.pixel_clean_sessions[0].session_id,
            12345
        );
    }

    #[test]
    fn test_stop_pixel_cleaning_no_active_session_returns_false() {
        let mut service = ServiceManager::new(
            PathBuf::from("/tmp/test_config.json"),
            PathBuf::from("/tmp/test_socket.sock"),
        )
        .expect("failed to instantiate ServiceManager");

        assert!(service.pixel_clean_sessions.is_empty());
        let stopped = service.stop_pixel_cleaning(None, Some(12345));
        assert!(!stopped);
    }

    #[test]
    fn test_start_pixel_cleaning_clamps_zero_minutes_without_error() {
        let mut service = ServiceManager::new(
            PathBuf::from("/tmp/test_config.json"),
            PathBuf::from("/tmp/test_socket.sock"),
        )
        .expect("failed to instantiate ServiceManager");

        let result_zero = service.start_pixel_cleaning(None, 0);
        assert!(result_zero.is_err());
        assert!(!result_zero.unwrap_err().contains("Duration"));
    }

    #[test]
    fn test_start_pixel_cleaning_no_matching_target_preserves_active_session() {
        let mut service = ServiceManager::new(
            PathBuf::from("/tmp/test_config.json"),
            PathBuf::from("/tmp/test_socket.sock"),
        )
        .expect("failed to instantiate ServiceManager");

        service.pixel_clean_sessions.push(PixelCleanSession {
            session_id: 8888,
            target_id: Some("0".into()),
            duration_minutes: 5,
            original_targets: Vec::new(),
            clean_until: Instant::now() + Duration::from_secs(300),
        });

        // Requesting for an LCD device that does not exist in targets should fail
        // and leave the existing session untouched
        let res = service.start_pixel_cleaning(Some("non_existent_lcd_target".into()), 30);
        assert!(res.is_err());

        assert_eq!(service.pixel_clean_sessions.len(), 1);
        assert_eq!(
            service.pixel_clean_sessions[0].session_id,
            8888
        );
    }

    #[test]
    fn test_stop_pixel_cleaning_concurrent_sessions() {
        let mut service = ServiceManager::new(
            PathBuf::from("/tmp/test_config.json"),
            PathBuf::from("/tmp/test_socket.sock"),
        )
        .expect("failed to instantiate ServiceManager");

        service.pixel_clean_sessions.push(PixelCleanSession {
            session_id: 101,
            target_id: Some("0".into()),
            duration_minutes: 15,
            original_targets: Vec::new(),
            clean_until: Instant::now() + Duration::from_secs(900),
        });

        service.pixel_clean_sessions.push(PixelCleanSession {
            session_id: 102,
            target_id: Some("1".into()),
            duration_minutes: 30,
            original_targets: Vec::new(),
            clean_until: Instant::now() + Duration::from_secs(1800),
        });

        assert_eq!(service.pixel_clean_sessions.len(), 2);

        // Stopping session 101 removes that session and keeps session 102
        let _ = service.stop_pixel_cleaning(None, Some(101));
        // original_targets was empty, but session 101 is dropped
        assert_eq!(service.pixel_clean_sessions.len(), 1);
        assert_eq!(service.pixel_clean_sessions[0].session_id, 102);
    }

    #[test]
    fn test_target_matches_compound_index() {
        use super::target_matches;

        // Compound identifier with index: e.g. "hid:1-2:1.0#0" vs "hid:1-2:1.0#1"
        assert!(target_matches("hid:1-2:1.0#0", 0, "hid:1-2:1.0", None));
        assert!(!target_matches("hid:1-2:1.0#0", 1, "hid:1-2:1.0", None));

        assert!(!target_matches("hid:1-2:1.0#1", 0, "hid:1-2:1.0", None));
        assert!(target_matches("hid:1-2:1.0#1", 1, "hid:1-2:1.0", None));

        // Bare #0 or #1
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

        // When device identity without # is provided, matches device
        assert!(target_matches("hid:1-2:1.0", 0, "hid:1-2:1.0", None));
        assert!(target_matches("1-2:1.0", 0, "hid:1-2:1.0", None));
        assert!(!target_matches("hid:9-9:1.0", 0, "hid:1-2:1.0", None));
    }

    #[test]
    fn test_target_matches_compound_invalid_prefix_returns_false() {
        use super::target_matches;

        // Compound identifier with nonexistent prefix should return false even if index matches
        assert!(!target_matches("nonexistent#0", 0, "hid:1-2:1.0", None));
        assert!(!target_matches("other_device#0", 0, "hid:1-2:1.0", None));
    }

    #[test]
    fn test_stop_pixel_cleaning_global_session_partial_stop_splits() {
        use std::path::PathBuf;
        use std::sync::Arc;
        use lianli_media::{MediaAsset, MediaAssetKind};
        use crate::pixel_cleaner::SavedTargetState;

        let mut service = ServiceManager::new(
            PathBuf::from("/tmp/test_config.json"),
            PathBuf::from("/tmp/test_socket.sock"),
        )
        .expect("failed to instantiate ServiceManager");

        let dummy_asset = Arc::new(MediaAsset {
            kind: MediaAssetKind::Static {
                frame: Arc::new(vec![0u8; 16]),
            },
            config_key: "dummy".to_string(),
            stream_fps: 30.0,
        });

        // Push a global session (target_id: None) with 2 targets
        service.pixel_clean_sessions.push(PixelCleanSession {
            session_id: 10,
            target_id: None,
            duration_minutes: 30,
            original_targets: vec![
                SavedTargetState {
                    target_index: 0,
                    device_identity: "hid:devA".to_string(),
                    media_asset: Arc::clone(&dummy_asset),
                    custom_h264: false,
                    original_brightness: Some(100),
                },
                SavedTargetState {
                    target_index: 1,
                    device_identity: "hid:devB".to_string(),
                    media_asset: Arc::clone(&dummy_asset),
                    custom_h264: false,
                    original_brightness: Some(100),
                },
            ],
            clean_until: Instant::now() + Duration::from_secs(1800),
        });

        service.sync_cleaner_ipc_state();
        let statuses_before = service.ipc.state.lock().pixel_clean_statuses();
        assert!(statuses_before.contains_key("all"));

        // Partially stop target 0
        let stopped = service.stop_pixel_cleaning(Some("hid:devA#0".into()), None);
        assert!(stopped);

        // Global session should be replaced by a targeted session for devB#1
        let statuses_after = service.ipc.state.lock().pixel_clean_statuses();
        assert!(!statuses_after.contains_key("all"), "all should no longer be broadcast");
        assert!(!statuses_after.contains_key("hid:devA#0"), "devA#0 was stopped");
        assert!(statuses_after.contains_key("hid:devB#1"), "devB#1 should still be cleaning");
    }
}

