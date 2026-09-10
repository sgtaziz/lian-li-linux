use super::*;
use anyhow::{ensure, Context, Result};
use lianli_media::rgb::{sync_effects, sync_layout::Layout, Animation};
use lianli_shared::rgb::{
    MergeLightingConfig, RgbDirection, RgbRegionConfig, RgbScope, RgbSyncKind,
};

pub(super) const MATCHED_MODES: &[RgbMode] = &[
    RgbMode::Rainbow,
    RgbMode::RainbowMorph,
    RgbMode::Static,
    RgbMode::Breathing,
    RgbMode::Meteor,
    RgbMode::Runway,
];

pub(super) enum PreparedSync {
    Wired {
        id: String,
        device: Arc<dyn RgbDevice>,
        animation: Animation,
    },
    Wireless {
        id: String,
        mac: [u8; 6],
        upload: Arc<WirelessRgbUpload>,
    },
    Hardware {
        id: String,
        device: Arc<dyn RgbDevice>,
        effect: RgbEffect,
    },
}

impl PreparedSync {
    pub fn id(&self) -> &str {
        match self {
            Self::Wired { id, .. } | Self::Wireless { id, .. } | Self::Hardware { id, .. } => id,
        }
    }
}

impl RgbController {
    pub(super) fn sync_ids(&self, config: &RgbAppConfig) -> Vec<String> {
        let Some(sync) = config.merge_lighting.as_ref().filter(|sync| sync.enabled) else {
            return Vec::new();
        };
        sync.device_order
            .iter()
            .filter(|id| {
                !sync.disabled_devices.contains(id)
                    && !config
                        .devices
                        .iter()
                        .any(|d| &d.device_id == *id && d.mb_rgb_sync)
                    && (self.wired.get(*id).is_some_and(|d| !d.rf_owned())
                        || self.wireless_state.contains_key(*id))
                    && (sync.kind == RgbSyncKind::Matched || self.sync_layout(id).is_some())
            })
            .cloned()
            .collect()
    }

    pub(super) fn sync_signature(&self, config: &RgbAppConfig) -> Result<Option<String>> {
        let Some(sync) = config.merge_lighting.as_ref().filter(|sync| sync.enabled) else {
            return Ok(None);
        };
        let devices: Vec<_> = self
            .sync_ids(config)
            .into_iter()
            .map(|id| {
                let reverse = sync
                    .device_order
                    .iter()
                    .position(|other| other == &id)
                    .and_then(|i| sync.directions.get(i))
                    .copied()
                    .unwrap_or_default();
                let profile = self.render_profile(&id);
                (id, reverse, profile)
            })
            .collect();
        Ok(Some(serde_json::to_string(&(
            sync.kind,
            &sync.effect,
            devices,
        ))?))
    }

    pub(super) fn prepare_sync(&self, config: &RgbAppConfig) -> Result<Vec<PreparedSync>> {
        let Some(sync) = config.merge_lighting.as_ref().filter(|sync| sync.enabled) else {
            return Ok(Vec::new());
        };
        validate_settings(sync)?;
        let ids = self.sync_ids(config);
        let native = sync.kind == RgbSyncKind::Matched
            || matches!(sync.effect.mode, RgbMode::RainbowMorph | RgbMode::Twinkle);
        let layouts: Vec<_> = if native {
            Vec::new()
        } else {
            ids.iter()
                .map(|id| {
                    self.render_profile(id)
                        .and_then(Layout::for_profile)
                        .with_context(|| format!("{id} does not support continuous RGB sync"))
                })
                .collect::<Result<_>>()?
        };
        let total: usize = layouts.iter().map(Layout::logical_led_count).sum();
        ensure!(
            total <= 720,
            "continuous RGB sync supports at most 720 logical LEDs"
        );
        let generic = (!native && !ids.is_empty())
            .then(|| sync_effects::render(&sync.effect, total.max(24)))
            .transpose()?;
        let mut offset = 0;
        let mut prepared = Vec::with_capacity(ids.len());
        for (index, id) in ids.into_iter().enumerate() {
            let projected = !native;
            let animation = if let Some(generic) = &generic {
                let layout = &layouts[index];
                let end = offset + layout.logical_led_count();
                let reverse = sync
                    .device_order
                    .iter()
                    .position(|other| other == &id)
                    .and_then(|i| sync.directions.get(i))
                    .is_some_and(|d| *d == RgbDirection::CounterClockwise);
                let frames = generic
                    .frames
                    .iter()
                    .map(|frame| layout.project_frame(frame, offset..end, reverse))
                    .collect::<Result<_>>()?;
                offset = end;
                Animation {
                    frames,
                    interval_hundredths: generic.interval_hundredths,
                    secondary: None,
                }
            } else if let Some(profile) = self.regional_profile(&id) {
                native_animation(profile, &sync.effect)
                    .with_context(|| format!("sync effect is unsupported by {id}"))?
            } else {
                ensure!(
                    sync.kind == RgbSyncKind::Matched,
                    "{id} does not support continuous RGB sync"
                );
                let device = self
                    .wired
                    .get(&id)
                    .context("RGB device unavailable")?
                    .clone();
                ensure!(
                    device.supported_modes().contains(&sync.effect.mode),
                    "sync effect is unsupported by {id}"
                );
                prepared.push(PreparedSync::Hardware {
                    id,
                    device,
                    effect: sync.effect.clone(),
                });
                continue;
            };
            ensure!(
                !animation.frames.is_empty() && animation.frames.len() <= 2048,
                "RGB sync animation exceeds the 2048-frame playback capacity for {id}"
            );
            if let Some(wireless_device) = self.wireless_state.get(&id) {
                let wireless = self
                    .wireless
                    .as_ref()
                    .context("wireless RGB controller unavailable")?;
                let upload = if projected {
                    wireless.prepare_rgb_sync_animation(
                        &wireless_device.mac,
                        &animation.frames,
                        animation.timing(),
                    )?
                } else {
                    wireless.prepare_rgb_animation(
                        &wireless_device.mac,
                        &animation.frames,
                        animation.timing(),
                    )?
                };
                prepared.push(PreparedSync::Wireless {
                    id,
                    mac: wireless_device.mac,
                    upload: Arc::new(upload),
                });
            } else {
                let inner = self
                    .wired
                    .get(&id)
                    .context("RGB device unavailable")?
                    .clone();
                let device: Arc<dyn RgbDevice> = if projected {
                    Arc::new(super::sync_device::SyncDevice {
                        inner,
                        led_count: animation.frames[0].len() as u16,
                    })
                } else {
                    inner
                };
                device.validate_software_animation(&animation.frames, animation.timing())?;
                prepared.push(PreparedSync::Wired {
                    id,
                    device,
                    animation,
                });
            }
        }
        Ok(prepared)
    }
}

fn validate_settings(sync: &MergeLightingConfig) -> Result<()> {
    ensure!(
        sync.device_order.len() <= 64 && sync.disabled_devices.len() <= 64,
        "RGB sync device limit exceeded"
    );
    ensure!(
        sync.directions.len() <= sync.device_order.len(),
        "RGB sync directions exceed device count"
    );
    for (i, id) in sync.device_order.iter().enumerate() {
        ensure!(
            !id.is_empty() && id.len() <= 256 && !sync.device_order[..i].contains(id),
            "invalid or duplicate RGB sync device"
        );
    }
    ensure!(
        sync.effect.colors.len() <= 6
            && sync.effect.speed <= 4
            && (sync.effect.brightness <= 4 || sync.effect.brightness == 255),
        "invalid RGB sync effect parameters"
    );
    ensure!(
        sync.effect.scope == RgbScope::All,
        "RGB sync requires the whole-device scope"
    );
    let modes = if sync.kind == RgbSyncKind::Matched {
        MATCHED_MODES
    } else {
        sync_effects::MODES
    };
    ensure!(
        modes.contains(&sync.effect.mode),
        "unsupported RGB sync effect"
    );
    Ok(())
}

fn native_animation(
    profile: lianli_shared::rgb::RgbRenderProfile,
    effect: &RgbEffect,
) -> Result<Animation> {
    let region = RgbRegionConfig {
        effect: effect.clone(),
        flip: false,
    };
    let mut regions = if profile.family != lianli_shared::rgb::RgbRenderFamily::HydroShiftII
        && lianli_media::rgb::parameters::for_profile(profile)
            .iter()
            .any(|p| p.mode == effect.mode)
    {
        vec![region]
    } else {
        regions::expand_all(profile, &region)
    };
    ensure!(!regions.is_empty(), "unsupported native sync effect");
    for region in &mut regions {
        if let Some(parameters) =
            lianli_media::rgb::parameters::for_scope(profile, region.effect.scope)
                .iter()
                .find(|p| p.mode == effect.mode)
        {
            if parameters.per_fan_colors {
                region.effect.colors = vec![
                    effect.colors.first().copied().unwrap_or([0; 3]);
                    usize::from(parameters.max_colors)
                ];
            }
        }
    }
    lianli_media::rgb::family::render(profile, &regions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_shared::rgb::{RgbRenderFamily as Family, RgbRenderProfile};

    #[test]
    fn native_sync_modes_cover_every_software_family() {
        for (family, fan_count, led_count) in [
            (Family::Tl, 3, 78),
            (Family::Sl, 3, 120),
            (Family::SlInf, 3, 132),
            (Family::SlInfV3, 3, 132),
            (Family::SlV4, 3, 156),
            (Family::Cl, 3, 72),
            (Family::P28, 3, 27),
            (Family::Strimer, 0, 116),
            (Family::HydroShiftII, 3, 96),
            (Family::HydroShiftII, 0, 24),
            (Family::HydroShiftIIOled, 0, 45),
            (Family::UniversalScreen, 0, 60),
            (Family::Lancool217, 0, 96),
            (Family::LancoolV150, 4, 88),
        ] {
            let profile = RgbRenderProfile {
                family,
                fan_count,
                led_count,
                right_attach: false,
            };
            for mode in MATCHED_MODES.iter().copied().chain([RgbMode::Twinkle]) {
                let effect = RgbEffect {
                    mode,
                    colors: vec![[255, 0, 0], [0, 255, 0]],
                    ..Default::default()
                };
                let animation = native_animation(profile, &effect)
                    .unwrap_or_else(|e| panic!("{family:?} {mode:?}: {e:#}"));
                assert!(!animation.frames.is_empty());
                assert!(
                    animation
                        .frames
                        .iter()
                        .all(|frame| frame.len() == usize::from(led_count)),
                    "{family:?} {mode:?}"
                );
            }
        }
    }

    #[test]
    fn sync_configuration_rejects_duplicate_devices_and_invalid_controls() {
        let mut sync = MergeLightingConfig {
            enabled: true,
            device_order: vec!["a".into(), "a".into()],
            ..Default::default()
        };
        assert!(validate_settings(&sync).is_err());
        sync.device_order.pop();
        assert!(validate_settings(&sync).is_ok());
        sync.effect.brightness = 5;
        assert!(validate_settings(&sync).is_err());
        sync.effect.brightness = 255;
        assert!(validate_settings(&sync).is_ok());
        sync.effect.mode = RgbMode::Voice;
        assert!(validate_settings(&sync).is_err());
    }
}
