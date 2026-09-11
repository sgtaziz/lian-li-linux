use lianli_shared::rgb::{RgbMode, RgbScope};

pub fn supports(mode: RgbMode, scope: RgbScope) -> bool {
    if !super::MODES.contains(&mode) {
        return false;
    }
    match scope {
        RgbScope::All => !matches!(
            mode,
            RgbMode::Staggered
                | RgbMode::Tide
                | RgbMode::Mixing
                | RgbMode::PingPong
                | RgbMode::Ripple
                | RgbMode::Collide
                | RgbMode::Reflect
        ),
        RgbScope::Inner | RgbScope::Outer => !matches!(
            mode,
            RgbMode::Endless
                | RgbMode::River
                | RgbMode::Duel
                | RgbMode::Hourglass
                | RgbMode::Pioneer
                | RgbMode::ShuttleRun
                | RgbMode::GradientRibbon
                | RgbMode::Twinkle
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_group_and_individual_region_menus_are_distinct() {
        assert!(supports(RgbMode::GradientRibbon, RgbScope::All));
        assert!(!supports(RgbMode::GradientRibbon, RgbScope::Inner));
        assert!(supports(RgbMode::Mixing, RgbScope::Outer));
        assert!(!supports(RgbMode::Mixing, RgbScope::All));
        assert!(supports(RgbMode::Rainbow, RgbScope::All));
        assert!(supports(RgbMode::Rainbow, RgbScope::Inner));
        assert!(!supports(RgbMode::Rainbow, RgbScope::Top));
    }
}
