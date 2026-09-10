use lianli_shared::rgb::{RgbDirection, RgbEffectParameters, RgbMode, RgbScope};

pub fn for_scope(fans: u8, scope: RgbScope) -> Vec<RgbEffectParameters> {
    if !(1..=4).contains(&fans) || !super::SCOPES.contains(&scope) {
        return vec![];
    }
    super::MODES
        .iter()
        .filter(|&&mode| {
            scope != RgbScope::All
                || !matches!(
                    mode,
                    RgbMode::ColorCycle
                        | RgbMode::ColorfulMeteor
                        | RgbMode::Lottery
                        | RgbMode::DoubleMeteor
                        | RgbMode::MeteorContest
                        | RgbMode::MeteorMix
                        | RgbMode::ReturnArc
                        | RgbMode::DoubleArc
                )
        })
        .map(|&mode| {
            let per_fan_colors = matches!(mode, RgbMode::Static | RgbMode::Breathing);
            let (min_colors, max_colors) = match mode {
                RgbMode::Off | RgbMode::Rainbow | RgbMode::RainbowMorph | RgbMode::Twinkle => {
                    (0, 0)
                }
                RgbMode::Static | RgbMode::Breathing => (fans, fans),
                RgbMode::Runway | RgbMode::TaiChi => (2, 2),
                RgbMode::Meteor => (1, 4),
                RgbMode::ColorCycle | RgbMode::MopUp => (1, 4),
                RgbMode::MeteorRainbow | RgbMode::ColorfulMeteor => (0, 0),
                RgbMode::Lottery | RgbMode::Mixing => (2, 2),
                RgbMode::Warning | RgbMode::Voice | RgbMode::Tide => (4, 4),
                RgbMode::Scan => (1, 1),
                RgbMode::DoubleMeteor | RgbMode::ReturnArc | RgbMode::DoubleArc => (4, 4),
                RgbMode::MeteorContest | RgbMode::MeteorMix => (2, 2),
                RgbMode::Door | RgbMode::Disco | RgbMode::ElectricCurrent | RgbMode::Reflect => {
                    (4, 4)
                }
                RgbMode::HeartBeat | RgbMode::HeartBeatRunway => (1, 1),
                RgbMode::GradientRibbon => (0, 0),
                RgbMode::Wing => (4, 4),
                RgbMode::Drumming | RgbMode::Boomerang => (2, 2),
                RgbMode::CandyBox => (0, 0),
                _ => unreachable!(),
            };
            let directions = if matches!(
                mode,
                RgbMode::Rainbow
                    | RgbMode::Meteor
                    | RgbMode::TaiChi
                    | RgbMode::ColorCycle
                    | RgbMode::MeteorRainbow
                    | RgbMode::ColorfulMeteor
                    | RgbMode::Voice
                    | RgbMode::MeteorContest
                    | RgbMode::ReturnArc
                    | RgbMode::DoubleArc
                    | RgbMode::HeartBeatRunway
                    | RgbMode::Disco
                    | RgbMode::GradientRibbon
            ) || (mode == RgbMode::Lottery && scope == RgbScope::Center)
            {
                vec![RgbDirection::Clockwise, RgbDirection::CounterClockwise]
            } else {
                vec![]
            };
            RgbEffectParameters {
                mode,
                min_colors,
                max_colors,
                per_fan_colors,
                directions,
                supports_speed: !matches!(mode, RgbMode::Off | RgbMode::Static),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_modes_match_the_profile_controls() {
        for scope in super::super::SCOPES {
            let effects = for_scope(3, *scope);
            let parameters = |mode| effects.iter().find(|entry| entry.mode == mode).unwrap();
            assert_eq!(parameters(RgbMode::Static).max_colors, 3);
            assert!(parameters(RgbMode::Static).per_fan_colors);
            assert_eq!(parameters(RgbMode::Runway).min_colors, 2);
            assert_eq!(parameters(RgbMode::Meteor).max_colors, 4);
            assert_eq!(parameters(RgbMode::Twinkle).max_colors, 0);
            assert_eq!(parameters(RgbMode::TaiChi).directions.len(), 2);
            assert_eq!(parameters(RgbMode::GradientRibbon).directions.len(), 2);
            assert_eq!(parameters(RgbMode::Wing).max_colors, 4);
            assert_eq!(parameters(RgbMode::Drumming).max_colors, 2);
            assert_eq!(parameters(RgbMode::CandyBox).max_colors, 0);
            assert!(!parameters(RgbMode::Static).supports_speed);
        }

        let all = for_scope(3, RgbScope::All);
        assert_eq!(all.len(), 27);
        for unavailable in [
            RgbMode::ColorCycle,
            RgbMode::ColorfulMeteor,
            RgbMode::Lottery,
            RgbMode::DoubleMeteor,
            RgbMode::MeteorContest,
            RgbMode::MeteorMix,
            RgbMode::ReturnArc,
            RgbMode::DoubleArc,
        ] {
            assert!(!all.iter().any(|entry| entry.mode == unavailable));
        }
        assert_eq!(for_scope(3, RgbScope::Inner).len(), 35);
    }

    #[test]
    fn rejects_non_profile_scopes_and_invalid_counts() {
        assert!(for_scope(0, RgbScope::All).is_empty());
        assert!(for_scope(1, RgbScope::Top).is_empty());
    }
}
