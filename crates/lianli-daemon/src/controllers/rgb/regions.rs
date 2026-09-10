use anyhow::{ensure, Context, Result};
use lianli_shared::rgb::{RgbDeviceConfig, RgbMode, RgbRegionConfig, RgbScope};

pub(super) fn expand_all(
    profile: lianli_shared::rgb::RgbRenderProfile,
    all: &RgbRegionConfig,
) -> Vec<RgbRegionConfig> {
    lianli_media::rgb::parameters::scopes(profile)
        .into_iter()
        .filter(|&scope| {
            scope != RgbScope::All
                && lianli_media::rgb::parameters::for_scope(profile, scope)
                    .iter()
                    .any(|parameters| parameters.mode == all.effect.mode)
        })
        .map(|scope| {
            let mut region = all.clone();
            region.effect.scope = scope;
            region
        })
        .collect()
}

pub(super) fn resolve(
    config: &RgbDeviceConfig,
    profile: lianli_shared::rgb::RgbRenderProfile,
) -> Result<Option<Vec<RgbRegionConfig>>> {
    use lianli_shared::rgb::RgbRenderFamily;
    let scopes = lianli_media::rgb::parameters::scopes(profile);
    if let Some(regions) = &config.regions {
        ensure!(
            !regions.is_empty() && regions.len() <= scopes.len(),
            "invalid number of RGB regions"
        );
        for (index, region) in regions.iter().enumerate() {
            ensure!(
                scopes.contains(&region.effect.scope),
                "unsupported RGB region for {:?}",
                profile.family
            );
            ensure!(
                regions[..index]
                    .iter()
                    .all(|r| r.effect.scope != region.effect.scope),
                "duplicate RGB region"
            );
        }
        ensure!(
            regions.len() == 1 || regions.iter().all(|r| r.effect.scope != RgbScope::All),
            "All cannot be combined with individual RGB regions"
        );
        return Ok(Some(regions.clone()));
    }
    match profile.family {
        RgbRenderFamily::HydroShiftII => legacy_aio_regions(config, usize::from(profile.fan_count)),
        RgbRenderFamily::Tl
        | RgbRenderFamily::Sl
        | RgbRenderFamily::SlV4
        | RgbRenderFamily::SlInf
        | RgbRenderFamily::SlInfV3
        | RgbRenderFamily::Cl
        | RgbRenderFamily::P28 => legacy_fan_regions(config, usize::from(profile.fan_count)),
        RgbRenderFamily::UniversalScreen | RgbRenderFamily::HydroShiftIIOled => {
            if config.zones.is_empty()
                || config
                    .zones
                    .iter()
                    .all(|z| z.effect.mode == RgbMode::Direct)
            {
                return Ok(None);
            }
            ensure!(
                config.zones.len() == 1 && config.zones[0].zone_index == 0,
                "screen RGB requires one whole-ring effect"
            );
            let zone = &config.zones[0];
            ensure!(
                !zone.swap_lr && !zone.swap_tb,
                "screen RGB does not support region flipping"
            );
            Ok(Some(vec![RgbRegionConfig {
                effect: zone.effect.clone(),
                flip: false,
            }]))
        }
        RgbRenderFamily::Lancool217 | RgbRenderFamily::LancoolV150 => legacy_whole_region(config),
        RgbRenderFamily::Strimer => {
            let Some(mut regions) = legacy_whole_region(config)? else {
                return Ok(None);
            };
            if !lianli_media::rgb::parameters::for_scope(profile, RgbScope::All)
                .iter()
                .any(|controls| controls.mode == regions[0].effect.mode)
            {
                regions = expand_all(profile, &regions[0]);
                ensure!(
                    !regions.is_empty(),
                    "choose a supported Strimer effect to replace legacy settings"
                );
            }
            Ok(Some(regions))
        }
    }
}

fn legacy_whole_region(config: &RgbDeviceConfig) -> Result<Option<Vec<RgbRegionConfig>>> {
    if config.zones.is_empty()
        || config
            .zones
            .iter()
            .all(|zone| zone.effect.mode == RgbMode::Direct)
    {
        return Ok(None);
    }
    ensure!(
        config.zones.len() == 1 && config.zones[0].zone_index == 0,
        "device RGB requires one whole-device effect"
    );
    let zone = &config.zones[0];
    ensure!(
        zone.effect.scope == RgbScope::All && !zone.swap_lr && !zone.swap_tb,
        "device RGB supports only the whole device without flipping"
    );
    Ok(Some(vec![RgbRegionConfig {
        effect: zone.effect.clone(),
        flip: false,
    }]))
}

fn legacy_aio_regions(
    config: &RgbDeviceConfig,
    fans: usize,
) -> Result<Option<Vec<RgbRegionConfig>>> {
    if config.zones.is_empty()
        || config
            .zones
            .iter()
            .all(|z| z.effect.mode == RgbMode::Direct)
    {
        return Ok(None);
    }
    let pump = config
        .zones
        .iter()
        .find(|zone| zone.zone_index == 0)
        .context("choose pump and fan RGB regions to replace incomplete AIO settings")?;
    ensure!(
        pump.effect.mode != RgbMode::Direct
            && pump.effect.scope == RgbScope::All
            && !pump.swap_lr
            && !pump.swap_tb,
        "choose pump and fan RGB regions to replace legacy AIO settings"
    );
    let mut effect = pump.effect.clone();
    effect.scope = RgbScope::Pump;
    let mut regions = vec![RgbRegionConfig {
        effect,
        flip: false,
    }];
    if fans > 0 {
        let mut fan_config = config.clone();
        fan_config.zones.retain(|zone| zone.zone_index > 0);
        for zone in &mut fan_config.zones {
            zone.zone_index -= 1;
        }
        let fan_regions = legacy_fan_regions(&fan_config, fans)?
            .context("choose fan RGB regions to replace mixed direct and animated AIO settings")?;
        for scope in [RgbScope::Center, RgbScope::Outer] {
            let mut region = fan_regions[0].clone();
            region.effect.scope = scope;
            regions.push(region);
        }
    }
    Ok(Some(regions))
}

fn legacy_fan_regions(
    config: &RgbDeviceConfig,
    fans: usize,
) -> Result<Option<Vec<RgbRegionConfig>>> {
    if config.zones.is_empty()
        || config
            .zones
            .iter()
            .all(|z| z.effect.mode == RgbMode::Direct)
    {
        return Ok(None);
    }
    let first = config
        .zones
        .iter()
        .find(|z| z.zone_index == 0)
        .context("choose a group RGB effect to replace legacy per-fan settings")?;
    ensure!(
        !first.swap_lr,
        "choose a group effect to replace legacy per-fan orientation"
    );
    let mut effect = first.effect.clone();
    let per_fan_colors = matches!(effect.mode, RgbMode::Static | RgbMode::Breathing);
    let mut palette = Vec::with_capacity(fans);
    for fan in 0..fans {
        let zone = config
            .zones
            .iter()
            .find(|z| usize::from(z.zone_index) == fan)
            .context("choose a group RGB effect to replace incomplete per-fan settings")?;
        let mut comparable = zone.effect.clone();
        if per_fan_colors {
            comparable.colors = effect.colors.clone();
        }
        ensure!(comparable == effect && zone.swap_tb == first.swap_tb && zone.swap_lr == first.swap_lr,
            "independent per-fan animations cannot be migrated to group effects; choose a group effect");
        palette.push(zone.effect.colors.first().copied().unwrap_or([0; 3]));
    }
    ensure!(
        effect.scope == RgbScope::All,
        "choose group regions to replace legacy scoped fan effects"
    );
    if per_fan_colors {
        effect.colors = palette;
    }
    Ok(Some(vec![RgbRegionConfig {
        effect,
        flip: first.swap_tb,
    }]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_shared::rgb::{RgbEffect, RgbZoneConfig};

    #[test]
    fn expanding_whole_device_effect_excludes_all_and_unsupported_sides() {
        use lianli_shared::rgb::{RgbRenderFamily, RgbRenderProfile};
        let mut profile = RgbRenderProfile {
            family: RgbRenderFamily::SlInf,
            fan_count: 3,
            led_count: 132,
            right_attach: false,
        };
        let mut all = RgbRegionConfig {
            effect: RgbEffect::default(),
            flip: false,
        };
        let expanded = expand_all(profile, &all);
        assert_eq!(expanded.len(), 3);
        assert!(expanded
            .iter()
            .all(|region| region.effect.scope != RgbScope::All));
        lianli_media::rgb::family::render(profile, &expanded).unwrap();
        profile.family = RgbRenderFamily::Sl;
        profile.led_count = 120;
        all.effect.mode = RgbMode::GradientRibbon;
        assert!(expand_all(profile, &all).is_empty());
    }

    fn config(mode: RgbMode) -> RgbDeviceConfig {
        RgbDeviceConfig {
            device_id: "tl".into(),
            mb_rgb_sync: false,
            active_preset: None,
            regions: None,
            effect_memory: Vec::new(),
            zones: (0..3)
                .map(|zone_index| RgbZoneConfig {
                    zone_index,
                    effect: RgbEffect {
                        mode,
                        ..Default::default()
                    },
                    swap_lr: false,
                    swap_tb: false,
                })
                .collect(),
        }
    }

    #[test]
    fn migration_retains_fan_color_slots() {
        let mut config = config(RgbMode::Breathing);
        config.zones[1].effect.colors = vec![[5, 6, 7]];
        let migrated = legacy_fan_regions(&config, 3).unwrap().unwrap();
        assert_eq!(migrated[0].effect.colors, [[255; 3], [5, 6, 7], [255; 3]]);
    }

    #[test]
    fn migration_rejects_conflicting_fan_animations() {
        let mut config = config(RgbMode::Rainbow);
        config.zones[2].effect.mode = RgbMode::Breathing;
        assert!(legacy_fan_regions(&config, 3).is_err());
        config.zones[2].effect.mode = RgbMode::Rainbow;
        config.zones[1].effect.speed = 4;
        assert!(legacy_fan_regions(&config, 3).is_err());
    }

    #[test]
    fn legacy_strimer_static_migrates_to_only_physical_segments() {
        use lianli_shared::rgb::{RgbRenderFamily, RgbRenderProfile};
        let mut config = config(RgbMode::Static);
        config.zones.truncate(1);
        for (led_count, segments) in [(88, 4), (116, 4), (132, 6), (174, 6)] {
            let profile = RgbRenderProfile {
                family: RgbRenderFamily::Strimer,
                fan_count: 0,
                led_count,
                right_attach: false,
            };
            let regions = resolve(&config, profile).unwrap().unwrap();
            assert_eq!(regions.len(), segments);
            assert!(regions
                .iter()
                .all(|region| region.effect.scope != RgbScope::All));
            let animation = lianli_media::rgb::family::render(profile, &regions).unwrap();
            assert!(animation
                .frames
                .iter()
                .all(|frame| frame.len() == usize::from(led_count)));
            assert!(config.regions.is_none());
        }
    }

    #[test]
    fn case_migration_accepts_only_one_unflipped_whole_device_zone() {
        let mut config = config(RgbMode::Rainbow);
        config.zones.truncate(1);
        assert_eq!(
            legacy_whole_region(&config).unwrap().unwrap()[0]
                .effect
                .mode,
            RgbMode::Rainbow
        );

        config.zones[0].effect.scope = RgbScope::Front;
        assert!(legacy_whole_region(&config).is_err());
        config.zones[0].effect.scope = RgbScope::All;
        config.zones[0].swap_lr = true;
        assert!(legacy_whole_region(&config).is_err());
    }
}
