use lianli_shared::rgb::{RgbDirection, RgbEffectParameters, RgbMode};

pub fn for_fans(fans: u8) -> Vec<RgbEffectParameters> {
    super::MODES
        .iter()
        .map(|&mode| {
            let per_fan_colors = matches!(mode, RgbMode::Static | RgbMode::Breathing);
            let colors = match mode {
                RgbMode::Off
                | RgbMode::Rainbow
                | RgbMode::RainbowMorph
                | RgbMode::Voice
                | RgbMode::Kaleidoscope
                | RgbMode::Twinkle => 0,
                RgbMode::Static | RgbMode::Breathing => fans,
                RgbMode::Runway
                | RgbMode::Staggered
                | RgbMode::Tide
                | RgbMode::Mixing
                | RgbMode::TailChasing
                | RgbMode::Racing
                | RgbMode::Lottery
                | RgbMode::Intertwine => 2,
                RgbMode::ColorCycle | RgbMode::CoverCycle => 3,
                _ => 4,
            };
            RgbEffectParameters {
                mode,
                min_colors: if per_fan_colors {
                    colors
                } else if colors == 0 {
                    0
                } else if matches!(
                    mode,
                    RgbMode::Racing | RgbMode::Intertwine | RgbMode::TailChasing | RgbMode::Collide
                ) {
                    2
                } else {
                    1
                },
                max_colors: colors,
                per_fan_colors,
                directions: if matches!(
                    mode,
                    RgbMode::Rainbow
                        | RgbMode::Meteor
                        | RgbMode::ColorCycle
                        | RgbMode::Render
                        | RgbMode::TailChasing
                        | RgbMode::Stack
                        | RgbMode::CoverCycle
                        | RgbMode::Lottery
                        | RgbMode::MeteorShower
                        | RgbMode::Wave
                        | RgbMode::Paint
                        | RgbMode::Racing
                ) {
                    vec![RgbDirection::Clockwise, RgbDirection::CounterClockwise]
                } else {
                    vec![]
                },
                supports_speed: !matches!(mode, RgbMode::Off | RgbMode::Static),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_expose_supported_tl_parameters() {
        let effects = for_fans(3);
        let parameters = |mode| effects.iter().find(|p| p.mode == mode).unwrap();
        assert_eq!(parameters(RgbMode::Breathing).max_colors, 3);
        assert!(parameters(RgbMode::Breathing).per_fan_colors);
        assert_eq!(parameters(RgbMode::ColorCycle).max_colors, 3);
        assert_eq!(parameters(RgbMode::Mixing).max_colors, 2);
        assert_eq!(parameters(RgbMode::Twinkle).max_colors, 0);
        assert!(parameters(RgbMode::Reflect).directions.is_empty());
        assert_eq!(parameters(RgbMode::Paint).directions.len(), 2);
        assert_eq!(parameters(RgbMode::Wave).directions.len(), 2);
        assert_eq!(parameters(RgbMode::Racing).directions.len(), 2);
        assert_eq!(parameters(RgbMode::Lottery).directions.len(), 2);
    }
}
