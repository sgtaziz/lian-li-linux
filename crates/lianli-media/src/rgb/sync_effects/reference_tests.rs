use super::*;

fn hash_bytes(bytes: impl IntoIterator<Item = u8>) -> u64 {
    bytes
        .into_iter()
        .fold(14_695_981_039_346_656_037, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(1_099_511_628_211)
        })
}

#[test]
fn rendered_frames_match_extracted_ui_generators() {
    let expected = [
        (RgbMode::Rainbow, 14142790490520549899u64),
        (RgbMode::Static, 3697557218306230553u64),
        (RgbMode::Breathing, 16119335978221231289u64),
        (RgbMode::Runway, 14174833362452329497u64),
        (RgbMode::Meteor, 5223044103114015039u64),
        (RgbMode::Stack, 1639843248206031682u64),
        (RgbMode::ColorCycle, 16608954137649408125u64),
        (RgbMode::CoverCycle, 17087459059188827528u64),
        (RgbMode::Wave, 15371054198406391468u64),
        (RgbMode::MeteorShower, 14018315772910898162u64),
        (RgbMode::Disco, 8145140152481647600u64),
        (RgbMode::BlowUp, 4604923023872545309u64),
        (RgbMode::HeartBeat, 14971572100679160273u64),
        (RgbMode::Warning, 18311343259735848761u64),
        (RgbMode::SeaFlow, 11804594125915943949u64),
        (RgbMode::Ripple, 10927930596213720869u64),
        (RgbMode::Echo, 336045806878524633u64),
    ];
    for (mode, expected) in expected {
        let mut summaries = Vec::new();
        for leds in [24, 25, 62, 147] {
            for direction in [RgbDirection::Clockwise, RgbDirection::CounterClockwise] {
                for brightness in [2, 4] {
                    let effect = RgbEffect {
                        mode,
                        direction,
                        brightness,
                        colors: vec![[120, 30, 50], [10, 140, 20], [30, 40, 160], [80, 60, 10]],
                        ..Default::default()
                    };
                    let animation = render(&effect, leds).unwrap();
                    let hash = hash_bytes(animation.frames.iter().flatten().flatten().copied());
                    summaries.extend_from_slice(&(animation.frames.len() as u64).to_le_bytes());
                    summaries.extend_from_slice(&hash.to_le_bytes());
                }
            }
        }
        assert_eq!(hash_bytes(summaries), expected, "{mode:?}");
    }
}
