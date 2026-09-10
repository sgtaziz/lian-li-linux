use lianli_shared::rgb::{RgbDirection, RgbEffectParameters, RgbMode, RgbScope};

pub fn supports(mode: RgbMode, scope: RgbScope) -> bool {
    match scope {
        RgbScope::All => super::WHOLE_MODES.contains(&mode),
        RgbScope::Inner | RgbScope::Outer => super::SIDE_MODES.contains(&mode),
        _ => false,
    }
}

pub fn parameters(mode: RgbMode, scope: RgbScope, fans: u8) -> Option<RgbEffectParameters> {
    if !supports(mode, scope) {
        return None;
    }
    let per_fan_colors = matches!(mode, RgbMode::Static | RgbMode::Breathing);
    let colors = match mode {
        RgbMode::Off | RgbMode::Rainbow | RgbMode::RainbowMorph => 0,
        RgbMode::GradientRibbon | RgbMode::Twinkle => 0,
        RgbMode::Pioneer => 1,
        RgbMode::Runway
        | RgbMode::Staggered
        | RgbMode::Tide
        | RgbMode::Mixing
        | RgbMode::PingPong
        | RgbMode::Endless
        | RgbMode::Duel
        | RgbMode::ShuttleRun => 2,
        RgbMode::ColorCycle | RgbMode::River => 3,
        _ if per_fan_colors => fans,
        _ => 4,
    };
    let direction = matches!(
        mode,
        RgbMode::Rainbow
            | RgbMode::Runway
            | RgbMode::Meteor
            | RgbMode::ColorCycle
            | RgbMode::Render
            | RgbMode::PingPong
            | RgbMode::Stack
            | RgbMode::River
            | RgbMode::GradientRibbon
    );
    Some(RgbEffectParameters {
        mode,
        min_colors: colors,
        max_colors: colors,
        per_fan_colors,
        directions: if direction {
            vec![RgbDirection::Clockwise, RgbDirection::CounterClockwise]
        } else {
            vec![]
        },
        supports_speed: !matches!(mode, RgbMode::Off | RgbMode::Static),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_menus_and_controls_match_the_sl_flex_source() {
        assert!(supports(RgbMode::Endless, RgbScope::All));
        assert!(!supports(RgbMode::Endless, RgbScope::Inner));
        assert!(supports(RgbMode::Mixing, RgbScope::Outer));
        assert!(!supports(RgbMode::Mixing, RgbScope::All));
        assert_eq!(
            parameters(RgbMode::Mixing, RgbScope::Outer, 4)
                .unwrap()
                .max_colors,
            2
        );
        assert_eq!(
            parameters(RgbMode::Static, RgbScope::All, 4)
                .unwrap()
                .max_colors,
            4
        );
        assert!(
            parameters(RgbMode::River, RgbScope::All, 4)
                .unwrap()
                .directions
                .len()
                == 2
        );
    }
}
