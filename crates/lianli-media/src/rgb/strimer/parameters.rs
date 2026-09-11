use lianli_shared::rgb::{RgbDirection, RgbEffectParameters, RgbMode, RgbScope};

pub(crate) fn for_scope(led_count: usize, scope: RgbScope) -> Vec<RgbEffectParameters> {
    let modes = match scope {
        RgbScope::All => super::WHOLE_MODES,
        RgbScope::Segment1 | RgbScope::Segment2 | RgbScope::Segment3 | RgbScope::Segment4 => {
            super::SEGMENT_MODES
        }
        RgbScope::Segment5 | RgbScope::Segment6 if matches!(led_count, 132 | 174) => {
            super::SEGMENT_MODES
        }
        _ => return vec![],
    };
    modes
        .iter()
        .map(|&mode| for_mode(mode, scope, led_count))
        .collect()
}

fn for_mode(mode: RgbMode, scope: RgbScope, led_count: usize) -> RgbEffectParameters {
    let colors = if scope == RgbScope::All {
        match mode {
            RgbMode::Off | RgbMode::GradientRibbon | RgbMode::RainbowWave => 0,
            RgbMode::Pioneer => 1,
            RgbMode::Endless
            | RgbMode::ShuttleRun
            | RgbMode::River
            | RgbMode::Mixing
            | RgbMode::Runway => 2,
            RgbMode::Hourglass | RgbMode::ElectricCurrent | RgbMode::Transformation => 4,
            RgbMode::Voice => {
                if matches!(led_count, 132 | 174) {
                    6
                } else {
                    4
                }
            }
            _ => 6,
        }
    } else {
        match mode {
            RgbMode::Off | RgbMode::Rainbow | RgbMode::RainbowMorph => 0,
            RgbMode::Static | RgbMode::Breathing => 1,
            _ => 6,
        }
    };
    let direction = if scope == RgbScope::All {
        matches!(
            mode,
            RgbMode::ColorTransfer
                | RgbMode::Contest
                | RgbMode::CrossOver
                | RgbMode::BulletStack
                | RgbMode::Parallel
                | RgbMode::ShockWave
                | RgbMode::Drizzling
                | RgbMode::River
                | RgbMode::Transformation
                | RgbMode::GradientRibbon
                | RgbMode::RainbowWave
                | RgbMode::Meteor
                | RgbMode::Stack
        )
    } else {
        matches!(mode, RgbMode::Rainbow | RgbMode::Wave)
    };
    RgbEffectParameters {
        mode,
        min_colors: colors,
        max_colors: colors,
        per_fan_colors: false,
        directions: if direction {
            vec![RgbDirection::Clockwise, RgbDirection::CounterClockwise]
        } else {
            vec![]
        },
        supports_speed: !matches!(mode, RgbMode::Off | RgbMode::Static),
    }
}
