use super::engine::Plane;
use lianli_shared::rgb::RgbMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ModeParameters {
    pub min_colors: u8,
    pub max_colors: u8,
    pub per_fan_colors: bool,
    pub supports_direction: bool,
}

pub(crate) fn for_mode(mode: RgbMode, plane: Plane) -> Option<ModeParameters> {
    let (min_colors, max_colors, per_fan_colors, supports_direction) = match mode {
        RgbMode::Off | RgbMode::RainbowMorph => (0, 0, false, false),
        RgbMode::Rainbow => (0, 0, false, true),
        RgbMode::Static | RgbMode::Breathing => (1, 4, true, false),
        RgbMode::Runway => (2, 2, false, false),
        RgbMode::Meteor => (1, 4, false, true),
        RgbMode::Twinkle => (1, 4, false, false),
        RgbMode::TaiChi => (2, 2, false, true),
        RgbMode::ColorCycle => (1, 4, false, true),
        RgbMode::MopUp => (1, 4, false, false),
        RgbMode::MeteorRainbow | RgbMode::ColorfulMeteor => (0, 0, false, true),
        RgbMode::Lottery => (2, 2, false, plane == Plane::Center),
        RgbMode::Warning | RgbMode::Voice | RgbMode::Tide => (4, 4, false, mode == RgbMode::Voice),
        RgbMode::Mixing => (2, 2, false, false),
        RgbMode::Scan => (1, 1, false, false),
        RgbMode::DoubleMeteor => (4, 4, false, false),
        RgbMode::MeteorContest => (2, 2, false, true),
        RgbMode::MeteorMix => (2, 2, false, false),
        RgbMode::ReturnArc | RgbMode::DoubleArc => (4, 4, false, true),
        RgbMode::Door => (4, 4, false, false),
        RgbMode::HeartBeat => (1, 1, false, false),
        RgbMode::HeartBeatRunway => (1, 1, false, true),
        RgbMode::Disco => (4, 4, false, true),
        RgbMode::ElectricCurrent | RgbMode::Reflect => (4, 4, false, false),
        RgbMode::GradientRibbon => (0, 0, false, true),
        RgbMode::Wing => (4, 4, false, false),
        RgbMode::Drumming | RgbMode::Boomerang => (2, 2, false, false),
        RgbMode::CandyBox => (0, 0, false, false),
        _ => return None,
    };
    Some(ModeParameters {
        min_colors,
        max_colors,
        per_fan_colors,
        supports_direction,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lottery_direction_is_a_center_plane_parameter() {
        assert!(
            for_mode(RgbMode::Lottery, Plane::Center)
                .unwrap()
                .supports_direction
        );
        assert!(
            !for_mode(RgbMode::Lottery, Plane::Outer)
                .unwrap()
                .supports_direction
        );
    }

    #[test]
    fn final_native_modes_expose_their_recovered_controls() {
        let cases = [
            (RgbMode::Disco, 4, true),
            (RgbMode::ElectricCurrent, 4, false),
            (RgbMode::Reflect, 4, false),
            (RgbMode::GradientRibbon, 0, true),
            (RgbMode::Wing, 4, false),
            (RgbMode::Drumming, 2, false),
            (RgbMode::Boomerang, 2, false),
            (RgbMode::CandyBox, 0, false),
        ];
        for (mode, colors, direction) in cases {
            let parameters = for_mode(mode, Plane::Center).unwrap();
            assert_eq!(parameters.min_colors, colors);
            assert_eq!(parameters.max_colors, colors);
            assert_eq!(parameters.supports_direction, direction);
            assert!(!parameters.per_fan_colors);
        }
    }
}
