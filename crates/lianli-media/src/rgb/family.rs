use anyhow::{ensure, Result};
use lianli_shared::rgb::{RgbMode, RgbRegionConfig, RgbRenderFamily, RgbRenderProfile, RgbScope};

use super::Animation;

pub fn modes(family: RgbRenderFamily) -> &'static [RgbMode] {
    match family {
        RgbRenderFamily::Tl => super::tl::MODES,
        RgbRenderFamily::Sl => super::sl_wireless::MODES,
        RgbRenderFamily::SlV4 => super::sl_v4::WHOLE_MODES,
        RgbRenderFamily::SlInf | RgbRenderFamily::SlInfV3 => super::sl_inf::MODES,
        RgbRenderFamily::Cl => super::cl::MODES,
        RgbRenderFamily::P28 => super::p28::MODES,
        RgbRenderFamily::HydroShiftII => super::h2::MODES,
        RgbRenderFamily::UniversalScreen => super::universal::MODES,
        RgbRenderFamily::HydroShiftIIOled => super::hs2_oled::MODES,
        RgbRenderFamily::Lancool217 => super::lancool217::MODES,
        RgbRenderFamily::LancoolV150 => super::lancool_v150::MODES,
        RgbRenderFamily::Strimer => super::strimer::WHOLE_MODES,
    }
}

pub fn regions(family: RgbRenderFamily) -> &'static [RgbScope] {
    match family {
        RgbRenderFamily::Tl => &[RgbScope::Top, RgbScope::Bottom],
        RgbRenderFamily::Sl | RgbRenderFamily::SlV4 => &[RgbScope::Inner, RgbScope::Outer],
        RgbRenderFamily::SlInf | RgbRenderFamily::SlInfV3 => super::sl_inf::SCOPES,
        RgbRenderFamily::Cl => &[RgbScope::Center, RgbScope::Outer],
        RgbRenderFamily::P28 => &[RgbScope::All],
        RgbRenderFamily::HydroShiftII => &[RgbScope::Pump, RgbScope::Center, RgbScope::Outer],
        RgbRenderFamily::UniversalScreen | RgbRenderFamily::HydroShiftIIOled => &[RgbScope::All],
        RgbRenderFamily::Lancool217 => super::lancool217::SCOPES,
        RgbRenderFamily::LancoolV150 => super::lancool_v150::SCOPES,
        RgbRenderFamily::Strimer => super::strimer::SCOPES,
    }
}

pub fn render(profile: RgbRenderProfile, settings: &[RgbRegionConfig]) -> Result<Animation> {
    for region in settings {
        let controls = super::parameters::for_scope(profile, region.effect.scope)
            .into_iter()
            .find(|parameters| parameters.mode == region.effect.mode)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "RGB effect {:?} is not supported for {:?} on {:?}",
                    region.effect.mode,
                    region.effect.scope,
                    profile.family
                )
            })?;
        ensure!(
            controls.directions.is_empty()
                || controls.directions.contains(&region.effect.direction),
            "RGB direction {:?} is not supported for {:?} on {:?}",
            region.effect.direction,
            region.effect.mode,
            profile.family
        );
    }
    let animation = match profile.family {
        RgbRenderFamily::HydroShiftII => {
            super::h2::render_regions(settings, usize::from(profile.fan_count))?
        }
        RgbRenderFamily::Cl => super::cl::render(settings, usize::from(profile.fan_count))?,
        RgbRenderFamily::P28 => super::p28::render(settings, usize::from(profile.fan_count))?,
        RgbRenderFamily::Tl => super::tl::render(settings, usize::from(profile.fan_count))?,
        RgbRenderFamily::Sl => {
            super::sl_wireless::render(settings, usize::from(profile.fan_count))?
        }
        RgbRenderFamily::SlV4 => super::sl_v4::render(settings, usize::from(profile.fan_count))?,
        RgbRenderFamily::SlInf | RgbRenderFamily::SlInfV3 => super::sl_inf::render(
            settings,
            usize::from(profile.fan_count),
            profile.right_attach,
            if profile.family == RgbRenderFamily::SlInf {
                super::sl_inf::InfVariant::Original
            } else {
                super::sl_inf::InfVariant::V3
            },
        )?,
        RgbRenderFamily::Lancool217 => {
            super::lancool217::render(settings, usize::from(profile.led_count))?
        }
        RgbRenderFamily::LancoolV150 => {
            super::lancool_v150::render(settings, usize::from(profile.led_count))?
        }
        RgbRenderFamily::UniversalScreen | RgbRenderFamily::HydroShiftIIOled => {
            ensure!(settings.len() == 1, "screen RGB requires one effect");
            let region = &settings[0];
            ensure!(
                region.effect.scope == RgbScope::All && !region.flip,
                "screen RGB supports only the whole ring without flipping"
            );
            if profile.family == RgbRenderFamily::UniversalScreen {
                super::universal::render(&region.effect, usize::from(profile.led_count))?
            } else {
                super::hs2_oled::render(&region.effect, usize::from(profile.led_count))?
            }
        }
        RgbRenderFamily::Strimer => {
            super::strimer::render(settings, usize::from(profile.led_count))?
        }
    };
    ensure!(
        !animation.frames.is_empty(),
        "RGB effect engine returned an empty animation"
    );
    ensure!(
        animation
            .frames
            .iter()
            .all(|f| f.len() == usize::from(profile.led_count)),
        "RGB effect engine returned an incompatible LED layout"
    );
    Ok(animation)
}
