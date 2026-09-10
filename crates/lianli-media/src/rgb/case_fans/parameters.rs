use lianli_shared::rgb::{RgbDirection, RgbEffectParameters, RgbMode};

pub(crate) fn for_mode(mode: RgbMode) -> Option<RgbEffectParameters> {
    let colors = match mode {
        RgbMode::Off
        | RgbMode::Rainbow
        | RgbMode::RainbowMorph
        | RgbMode::Twinkle
        | RgbMode::CandyBox => 0,
        RgbMode::Static | RgbMode::Wave | RgbMode::HeartBeat | RgbMode::HeartBeatRunway => 1,
        RgbMode::Runway | RgbMode::TaiChi | RgbMode::Mixing | RgbMode::MeteorContest => 2,
        RgbMode::ColorCycle => 3,
        RgbMode::Breathing
        | RgbMode::Meteor
        | RgbMode::MeteorShower
        | RgbMode::Warning
        | RgbMode::Tide
        | RgbMode::DoubleMeteor
        | RgbMode::ReturnArc
        | RgbMode::Disco => 4,
        _ => return None,
    };
    let direction = matches!(
        mode,
        RgbMode::Rainbow
            | RgbMode::Meteor
            | RgbMode::ColorCycle
            | RgbMode::CoverCycle
            | RgbMode::Wave
            | RgbMode::MeteorShower
            | RgbMode::TaiChi
            | RgbMode::MeteorContest
            | RgbMode::ReturnArc
            | RgbMode::Disco
    );
    Some(RgbEffectParameters {
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
    })
}
