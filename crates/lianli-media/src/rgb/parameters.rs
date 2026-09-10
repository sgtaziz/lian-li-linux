use lianli_shared::rgb::{
    RgbDirection, RgbEffectParameters, RgbMode, RgbRenderFamily, RgbRenderProfile,
};

pub fn for_profile(profile: RgbRenderProfile) -> Vec<RgbEffectParameters> {
    for_scope(profile, lianli_shared::rgb::RgbScope::All)
}

pub fn modes(profile: RgbRenderProfile) -> Vec<RgbMode> {
    let mut modes = Vec::new();
    for region in for_regions(profile) {
        for effect in region.effects {
            if !modes.contains(&effect.mode) {
                modes.push(effect.mode);
            }
        }
    }
    modes
}

pub fn scopes(profile: RgbRenderProfile) -> Vec<lianli_shared::rgb::RgbScope> {
    for_regions(profile)
        .into_iter()
        .map(|region| region.scope)
        .collect()
}

pub fn for_regions(profile: RgbRenderProfile) -> Vec<lianli_shared::rgb::RgbRegionParameters> {
    use lianli_shared::rgb::{RgbRegionParameters, RgbScope};
    if profile.family == RgbRenderFamily::HydroShiftII {
        let scopes = if profile.fan_count == 0 {
            vec![RgbScope::Pump]
        } else {
            vec![RgbScope::Pump, RgbScope::Center, RgbScope::Outer]
        };
        return scopes
            .into_iter()
            .map(|scope| RgbRegionParameters {
                scope,
                effects: for_scope(profile, scope),
            })
            .collect();
    }
    if super::family::modes(profile.family).is_empty() {
        return vec![];
    }
    std::iter::once(RgbScope::All)
        .chain(
            super::family::regions(profile.family)
                .iter()
                .copied()
                .filter(|&scope| scope != RgbScope::All),
        )
        .map(|scope| RgbRegionParameters {
            scope,
            effects: for_scope(profile, scope),
        })
        .filter(|region| !region.effects.is_empty())
        .collect()
}

pub fn for_scope(
    profile: RgbRenderProfile,
    scope: lianli_shared::rgb::RgbScope,
) -> Vec<RgbEffectParameters> {
    if profile.family == RgbRenderFamily::Strimer {
        return super::strimer::parameters::for_scope(usize::from(profile.led_count), scope);
    }
    if profile.family == RgbRenderFamily::SlV4 {
        let modes = if scope == lianli_shared::rgb::RgbScope::All {
            super::sl_v4::WHOLE_MODES
        } else {
            super::sl_v4::SIDE_MODES
        };
        return modes
            .iter()
            .filter_map(|&mode| {
                super::sl_v4::capabilities::parameters(mode, scope, profile.fan_count)
            })
            .collect();
    }
    if profile.family == RgbRenderFamily::Tl {
        return super::tl::parameters::for_fans(profile.fan_count);
    }
    if matches!(
        profile.family,
        RgbRenderFamily::SlInf | RgbRenderFamily::SlInfV3
    ) {
        return super::sl_inf::parameters::for_scope(profile.fan_count, scope);
    }
    if matches!(profile.family, RgbRenderFamily::Cl | RgbRenderFamily::P28) {
        let plane = if scope == lianli_shared::rgb::RgbScope::Outer
            && profile.family == RgbRenderFamily::Cl
        {
            super::cl::engine::Plane::Outer
        } else {
            super::cl::engine::Plane::Center
        };
        return super::family::modes(profile.family)
            .iter()
            .filter_map(|&mode| {
                let parameters = super::cl::parameters::for_mode(mode, plane)?;
                Some(RgbEffectParameters {
                    mode,
                    min_colors: if parameters.per_fan_colors {
                        profile.fan_count
                    } else {
                        parameters.min_colors
                    },
                    max_colors: if parameters.per_fan_colors {
                        profile.fan_count
                    } else {
                        parameters.max_colors
                    },
                    per_fan_colors: parameters.per_fan_colors,
                    directions: if parameters.supports_direction {
                        vec![RgbDirection::Clockwise, RgbDirection::CounterClockwise]
                    } else {
                        vec![]
                    },
                    supports_speed: !matches!(mode, RgbMode::Off | RgbMode::Static),
                })
            })
            .collect();
    }
    if profile.family == RgbRenderFamily::HydroShiftII {
        if matches!(
            scope,
            lianli_shared::rgb::RgbScope::Center | lianli_shared::rgb::RgbScope::Outer
        ) {
            return for_scope(
                RgbRenderProfile {
                    family: RgbRenderFamily::Cl,
                    ..profile
                },
                scope,
            );
        }
        return super::h2::MODES
            .iter()
            .map(|&mode| {
                let colors = match mode {
                    RgbMode::Off | RgbMode::Rainbow | RgbMode::RainbowMorph | RgbMode::Twinkle => 0,
                    RgbMode::Runway | RgbMode::TaiChi => 2,
                    RgbMode::Meteor | RgbMode::Bounce => 4,
                    _ => 1,
                };
                RgbEffectParameters {
                    mode,
                    min_colors: colors,
                    max_colors: colors,
                    per_fan_colors: false,
                    directions: if matches!(
                        mode,
                        RgbMode::Rainbow | RgbMode::Meteor | RgbMode::TaiChi | RgbMode::Pump
                    ) {
                        vec![RgbDirection::Clockwise, RgbDirection::CounterClockwise]
                    } else {
                        vec![]
                    },
                    supports_speed: !matches!(mode, RgbMode::Off | RgbMode::Static),
                }
            })
            .collect();
    }
    if profile.family == RgbRenderFamily::Lancool217 {
        return super::lancool217::MODES
            .iter()
            .filter_map(|&mode| super::lancool217::parameters::for_mode(mode))
            .collect();
    }
    if profile.family == RgbRenderFamily::LancoolV150 {
        return super::lancool_v150::MODES
            .iter()
            .filter_map(|&mode| super::lancool_v150::parameters::for_mode(mode))
            .collect();
    }
    super::family::modes(profile.family)
        .iter()
        .filter(|&&mode| {
            profile.family != RgbRenderFamily::Sl
                || super::sl_wireless::capabilities::supports(mode, scope)
        })
        .map(|&mode| {
            let screen = matches!(
                profile.family,
                RgbRenderFamily::UniversalScreen | RgbRenderFamily::HydroShiftIIOled
            );
            let per_fan_colors = !screen && matches!(mode, RgbMode::Static | RgbMode::Breathing);
            let automatic = matches!(
                mode,
                RgbMode::Off | RgbMode::Rainbow | RgbMode::RainbowMorph
            ) || screen
                && matches!(
                    mode,
                    RgbMode::BulletStack | RgbMode::Twinkle | RgbMode::RainbowWave
                );
            let mut max_colors = if automatic {
                0
            } else if per_fan_colors {
                profile.fan_count
            } else if screen && matches!(mode, RgbMode::Static | RgbMode::Breathing) {
                1
            } else if screen {
                6
            } else {
                4
            };
            if profile.family == RgbRenderFamily::Sl {
                max_colors = match mode {
                    RgbMode::Runway
                    | RgbMode::Staggered
                    | RgbMode::Tide
                    | RgbMode::PingPong
                    | RgbMode::Endless
                    | RgbMode::River
                    | RgbMode::Duel
                    | RgbMode::ShuttleRun => 2,
                    RgbMode::Pioneer => 1,
                    RgbMode::GradientRibbon => 0,
                    RgbMode::ColorCycle | RgbMode::Mixing => 3,
                    _ => max_colors,
                };
            } else if screen {
                max_colors = match mode {
                    RgbMode::Runway | RgbMode::Mixing | RgbMode::River => 2,
                    RgbMode::Hourglass | RgbMode::ElectricCurrent => 4,
                    _ => max_colors,
                };
            }
            let direction = if screen {
                matches!(
                    mode,
                    RgbMode::Rainbow
                        | RgbMode::Wave
                        | RgbMode::Meteor
                        | RgbMode::BulletStack
                        | RgbMode::River
                        | RgbMode::RainbowWave
                ) || profile.family == RgbRenderFamily::HydroShiftIIOled && mode == RgbMode::Tide
            } else if profile.family == RgbRenderFamily::Sl {
                matches!(
                    mode,
                    RgbMode::Rainbow
                        | RgbMode::Meteor
                        | RgbMode::ColorCycle
                        | RgbMode::Render
                        | RgbMode::Stack
                        | RgbMode::River
                        | RgbMode::GradientRibbon
                )
            } else {
                false
            };
            RgbEffectParameters {
                mode,
                min_colors: max_colors,
                max_colors,
                per_fan_colors,
                directions: if direction {
                    vec![RgbDirection::Clockwise, RgbDirection::CounterClockwise]
                } else {
                    vec![]
                },
                supports_speed: !matches!(mode, RgbMode::Off | RgbMode::Static),
            }
        })
        .collect()
}
