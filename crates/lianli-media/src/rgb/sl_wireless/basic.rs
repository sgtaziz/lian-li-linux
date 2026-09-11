use super::engine::{brightness, palette, place, scale, Color, Side};
use lianli_shared::rgb::RgbEffect;

pub(super) fn rainbow(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let len = fans * side.leds_per_track();
    let step = match side {
        Side::Inner => [8, 4, 1, 2][fans - 1],
        Side::Outer => [12, 6, 4, 3][fans - 1],
    };
    let table_len = if len == 36 { 36 } else { 96 };
    let clockwise = !matches!(
        effect.direction,
        lianli_shared::rgb::RgbDirection::CounterClockwise
    );
    let bright = brightness(effect);
    (0..len)
        .map(|frame| {
            let mut source = frame * step;
            let mut track = vec![[0; 3]; len];
            for position in 0..len {
                let target = if clockwise {
                    len - position - 1
                } else {
                    position
                };
                track[target] = scale(rainbow_color(source, table_len), bright);
                source = (source + step) % table_len;
            }
            place(&track, side, fans)
        })
        .collect()
}

pub(super) fn rainbow_color(index: usize, table_len: usize) -> Color {
    let pulse = |position: usize| -> u8 {
        let position = position % table_len;
        if table_len == 96 {
            match position {
                0 => 255,
                1..=31 => (256 - position * 8) as u8,
                32..=64 => 0,
                _ => ((position - 64) * 8) as u8,
            }
        } else {
            match position {
                0 => 255,
                1..=11 => (255 - position * 22) as u8,
                12..=24 => 0,
                _ => (13 + (position - 25) * 22) as u8,
            }
        }
    };
    let third = table_len / 3;
    [pulse(index), pulse(index + third * 2), pulse(index + third)]
}

pub(super) fn rainbow_morph(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    crate::rgb::effects::basic::rainbow_morph(
        fans * side.leds_per_track(),
        brightness(effect),
        |track| place(track, side, fans),
    )
    .into_iter()
    .step_by(2)
    .take(127)
    .collect()
}

pub(super) fn static_color(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let frames = crate::rgb::effects::basic::static_color(
        &palette(effect, side)[..fans],
        side.leds_per_track(),
        brightness(effect),
        |track| place(track, side, fans),
    );
    vec![frames[0].clone(); 30]
}

pub(super) fn breathing(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    crate::rgb::effects::basic::breathing(
        &palette(effect, side)[..fans],
        side.leds_per_track(),
        brightness(effect),
        |track| place(track, side, fans),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_shared::rgb::{RgbMode, RgbScope};

    fn effect() -> RgbEffect {
        RgbEffect {
            mode: RgbMode::Static,
            colors: vec![[255, 128, 1], [1, 200, 99]],
            brightness: 4,
            scope: RgbScope::Inner,
            ..Default::default()
        }
    }

    #[test]
    fn static_assigns_one_palette_slot_per_fan() {
        let frames = static_color(&effect(), 2, Side::Outer);
        assert_eq!(frames.len(), 30);
        assert_eq!(frames[0][12], [254, 127, 0]);
        assert_eq!(frames[0][52], [0, 199, 98]);
    }

    #[test]
    fn rainbow_uses_the_vendor_96_sample_table() {
        let frames = rainbow(&effect(), 1, Side::Outer);
        assert_eq!(frames.len(), 8);
        assert_eq!(frames[0][19], [254, 0, 0]);
        assert_eq!(frames[0][18], [159, 95, 0]);
    }

    #[test]
    fn morph_keeps_the_vendor_even_frames_only() {
        let frames = rainbow_morph(&effect(), 1, Side::Inner);
        assert_eq!(frames.len(), 127);
        assert_eq!(frames[0][0], [254, 0, 0]);
        assert_eq!(frames[1][0], [248, 5, 0]);
    }

    #[test]
    fn breathing_preserves_two_stage_integer_brightness() {
        let frames = breathing(&effect(), 1, Side::Outer);
        assert_eq!(frames.len(), 170);
        assert_eq!(frames[1][12], [1, 0, 0]);
        assert_eq!(frames[85][12], [253, 126, 0]);
    }
}
