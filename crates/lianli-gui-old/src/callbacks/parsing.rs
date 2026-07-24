use lianli_shared::rgb::{RgbDirection, RgbMode, RgbScope};

pub(super) fn parse_rgb_mode(s: &str) -> RgbMode {
    match s {
        "Off" => RgbMode::Off,
        "Direct" => RgbMode::Direct,
        "Static" => RgbMode::Static,
        "Rainbow" => RgbMode::Rainbow,
        "RainbowMorph" => RgbMode::RainbowMorph,
        "Breathing" => RgbMode::Breathing,
        "Runway" => RgbMode::Runway,
        "Meteor" => RgbMode::Meteor,
        "ColorCycle" => RgbMode::ColorCycle,
        "Staggered" => RgbMode::Staggered,
        "Tide" => RgbMode::Tide,
        "Mixing" => RgbMode::Mixing,
        "Voice" => RgbMode::Voice,
        "Door" => RgbMode::Door,
        "Render" => RgbMode::Render,
        "Ripple" => RgbMode::Ripple,
        "Reflect" => RgbMode::Reflect,
        "TailChasing" => RgbMode::TailChasing,
        "Paint" => RgbMode::Paint,
        "PingPong" => RgbMode::PingPong,
        "Stack" => RgbMode::Stack,
        "CoverCycle" => RgbMode::CoverCycle,
        "Wave" => RgbMode::Wave,
        "Racing" => RgbMode::Racing,
        "Lottery" => RgbMode::Lottery,
        "Intertwine" => RgbMode::Intertwine,
        "MeteorShower" => RgbMode::MeteorShower,
        "Collide" => RgbMode::Collide,
        "ElectricCurrent" => RgbMode::ElectricCurrent,
        "Kaleidoscope" => RgbMode::Kaleidoscope,
        "BigBang" => RgbMode::BigBang,
        "Vortex" => RgbMode::Vortex,
        "Pump" => RgbMode::Pump,
        "ColorsMorph" => RgbMode::ColorsMorph,
        "TaiChi" => RgbMode::TaiChi,
        "CrossingOver" => RgbMode::CrossingOver,
        "ColorfulStarryNight" => RgbMode::ColorfulStarryNight,
        "StaticStarryNight" => RgbMode::StaticStarryNight,
        "Bounce" => RgbMode::Bounce,
        "TickerTape" => RgbMode::TickerTape,
        "Fluctuation" => RgbMode::Fluctuation,
        "Transmit" => RgbMode::Transmit,
        "Burst" => RgbMode::Burst,
        _ => RgbMode::Off,
    }
}

pub(super) fn parse_rgb_direction(s: &str) -> RgbDirection {
    match s {
        "Clockwise" => RgbDirection::Clockwise,
        "CounterClockwise" => RgbDirection::CounterClockwise,
        "Up" => RgbDirection::Up,
        "Down" => RgbDirection::Down,
        "Spread" => RgbDirection::Spread,
        "Gather" => RgbDirection::Gather,
        _ => RgbDirection::Clockwise,
    }
}

pub(super) fn parse_rgb_scope(s: &str) -> RgbScope {
    match s {
        "All" => RgbScope::All,
        "Top" => RgbScope::Top,
        "Bottom" => RgbScope::Bottom,
        "Inner" => RgbScope::Inner,
        "Outer" => RgbScope::Outer,
        _ => RgbScope::All,
    }
}
