use super::media::lcd_id_matches;
use super::{DaemonEvent, ServiceManager};
use crate::pixel_cleaner::{pixel_cleaner_asset_path, PixelCleanSession, SavedTargetState};
use lianli_media::MediaAsset;
use lianli_shared::config::LcdConfig;
use lianli_shared::screen::ScreenInfo;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, warn};

impl ServiceManager {
    pub(super) fn start_pixel_cleaning(
        &mut self,
        target_dev_id: Option<String>,
        minutes: u32,
    ) {
        let cleaner_path = pixel_cleaner_asset_path();
        if !cleaner_path.exists() {
            warn!(
                "Pixel cleaner asset does not exist at {}",
                cleaner_path.display()
            );
            return;
        }

        // If a session is already active, restore previous first to avoid clobbering original state
        if self.pixel_clean_session.is_some() {
            self.stop_pixel_cleaning(None);
        }

        let target_info: Vec<(usize, String, ScreenInfo, bool)> = {
            let targets = self.targets.lock();
            targets
                .iter()
                .filter_map(|(idx, target)| {
                    if let Some(ref target_id) = target_dev_id {
                        if !lcd_id_matches(target_id, &target.device_identity)
                            && &target.device_identity != target_id
                        {
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
            warn!("No active LCD targets found matching {:?}", target_dev_id);
            return;
        }

        let mut saved_targets = Vec::new();

        for (idx, device_identity, screen, custom_h264) in target_info {
            let clean_cfg = LcdConfig {
                index: Some(idx),
                serial: Some(device_identity.clone()),
                media_type: lianli_shared::media::MediaType::Video,
                path: Some(cleaner_path.clone()),
                fps: Some(30.0),
                update_interval_ms: None,
                rgb: None,
                orientation: 0.0,
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
            let clean_asset = Arc::new(MediaAsset {
                kind: asset_kind,
                config_key: format!("pixel_cleaner_{idx}"),
                stream_fps,
            });

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
            let clean_until = Instant::now() + Duration::from_secs(minutes as u64 * 60);
            self.pixel_clean_session = Some(PixelCleanSession {
                original_targets: saved_targets,
                clean_until,
            });
            if let Some(ref tx) = self.tx {
                tx.send(DaemonEvent::FrameFinished).ok();
            }
        }
    }

    pub(super) fn stop_pixel_cleaning(&mut self, target_dev_id: Option<String>) {
        let Some(mut session) = self.pixel_clean_session.take() else {
            return;
        };

        let mut remaining = Vec::new();
        for saved in session.original_targets {
            let matches = match &target_dev_id {
                Some(id) => {
                    lcd_id_matches(id, &saved.device_identity) || &saved.device_identity == id
                }
                None => true,
            };

            if matches {
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

        if !remaining.is_empty() {
            session.original_targets = remaining;
            self.pixel_clean_session = Some(session);
        }

        if let Some(ref tx) = self.tx {
            tx.send(DaemonEvent::FrameFinished).ok();
        }
    }
}
