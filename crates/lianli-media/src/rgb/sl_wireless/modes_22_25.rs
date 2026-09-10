use super::basic::rainbow_color;
use super::engine::{brightness, palette_all, place, place_banks, scale, Color, Side};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

const COLOR_INDEX: [usize; 160] = [
    1, 0, 2, 3, 1, 2, 1, 0, 2, 3, 0, 3, 1, 0, 1, 2, 0, 2, 1, 0, 3, 1, 3, 0, 1, 0, 2, 0, 1, 3, 1, 2,
    3, 1, 2, 1, 0, 2, 1, 2, 0, 2, 0, 1, 3, 0, 2, 3, 1, 0, 1, 2, 3, 0, 1, 2, 1, 3, 0, 3, 2, 1, 0, 3,
    0, 2, 1, 0, 3, 0, 1, 2, 0, 3, 2, 1, 0, 1, 2, 3, 0, 1, 2, 1, 3, 2, 1, 0, 1, 3, 2, 0, 2, 1, 0, 1,
    3, 2, 1, 0, 2, 3, 0, 1, 3, 0, 2, 3, 1, 2, 0, 3, 1, 2, 3, 2, 0, 1, 2, 3, 1, 2, 1, 2, 0, 1, 2, 0,
    1, 3, 0, 2, 3, 1, 2, 0, 1, 2, 3, 2, 1, 2, 0, 1, 3, 0, 1, 2, 1, 2, 3, 2, 0, 2, 3, 0, 1, 3, 0, 2,
];

const PULSE: [u16; 19] = [
    5, 30, 55, 80, 105, 130, 160, 190, 220, 255, 220, 190, 160, 130, 105, 80, 55, 30, 5,
];

const PULSE_STARTS: &[(usize, i16)] = &[
    (0, 54),
    (0, 129),
    (1, 14),
    (1, 91),
    (1, 176),
    (2, 115),
    (3, 64),
    (3, 155),
    (4, -3),
    (4, 197),
    (6, 48),
    (7, 4),
    (7, 84),
    (8, 121),
    (10, 160),
    (11, 11),
    (11, 109),
    (12, 55),
    (13, -9),
    (13, 134),
    (13, 191),
    (14, 32),
    (17, 154),
    (19, 66),
    (20, 29),
    (20, 94),
    (21, 0),
    (22, 116),
    (23, 54),
    (23, 179),
    (24, 84),
    (25, 21),
    (25, 131),
    (26, -4),
    (26, 196),
    (27, 40),
    (29, 150),
    (30, 3),
    (31, 57),
    (31, 167),
    (32, 102),
    (33, 79),
    (36, -6),
    (36, 194),
    (37, 84),
    (38, 130),
    (39, 45),
    (40, 14),
    (42, 122),
    (43, -10),
    (43, 63),
    (43, 192),
    (44, 157),
    (45, 9),
    (46, 104),
    (47, 77),
    (49, 175),
    (50, 48),
    (50, 147),
    (51, 24),
    (53, 6),
    (54, 120),
    (55, 67),
    (56, 43),
    (57, -6),
    (57, 194),
    (59, 163),
    (60, 89),
    (62, 148),
    (63, 27),
    (63, 122),
    (64, 49),
    (65, 7),
    (67, 67),
    (68, 99),
    (68, 180),
    (69, 35),
    (70, 0),
    (71, 138),
    (72, 73),
    (73, 169),
    (76, 21),
    (76, 126),
    (77, 47),
    (78, 4),
    (78, 84),
    (81, 142),
    (82, -7),
    (82, 195),
    (83, 53),
    (84, 30),
    (84, 129),
    (84, 176),
    (86, 80),
    (90, 101),
    (91, 148),
    (93, 31),
    (94, 61),
    (95, 0),
    (96, 43),
    (96, 116),
    (97, 82),
    (99, 178),
    (100, 23),
    (100, 56),
    (101, 3),
    (101, 136),
    (104, 110),
    (104, 165),
    (106, 74),
    (107, 31),
    (107, 128),
    (108, 158),
    (109, -8),
    (109, 93),
    (109, 192),
    (112, 21),
    (113, 79),
    (114, -2),
    (114, 198),
    (115, 39),
    (116, 180),
    (117, 156),
    (118, 114),
    (119, 67),
    (120, 139),
    (124, 19),
    (124, 83),
    (126, 0),
    (127, 44),
    (127, 100),
    (127, 157),
    (129, 71),
    (131, 110),
    (132, -8),
    (132, 192),
    (133, 178),
    (134, 38),
    (134, 129),
    (135, 9),
    (136, 64),
    (136, 170),
    (138, 99),
    (138, 140),
    (141, 85),
    (142, 52),
    (142, 108),
    (142, 152),
    (145, -6),
    (145, 68),
    (145, 194),
    (146, 162),
    (148, 26),
    (150, 130),
    (150, 176),
    (151, 0),
    (151, 98),
    (152, 145),
    (153, 75),
    (154, 118),
    (155, 17),
    (155, 159),
    (158, 84),
    (159, 50),
];

const SINGLE_SPARKS: &[(usize, usize)] = &[(9, 64), (48, 0), (70, 107), (157, 35)];

pub(super) fn pioneer(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let outer_half = fans * Side::Outer.leds_per_track() / 2;
    let inner_half = fans * Side::Inner.leds_per_track() / 2;
    let color = scale(palette_all(effect)[0], brightness(effect));
    let mut frames = Vec::with_capacity(outer_half + inner_half + 4 * fans - 2);
    for (phase, half) in [outer_half, inner_half].into_iter().enumerate() {
        let span = half + 2 * fans - 1;
        for step in 0..span {
            if (phase == 0 && matches!(side, Side::Outer))
                || (phase == 1 && matches!(side, Side::Inner))
            {
                let mut track = vec![[0; 3]; half * 2];
                for position in 0..half {
                    let lit = (phase == 0 && position >= step) || (phase == 1 && position < step);
                    if lit {
                        let first = if phase == 0 {
                            position
                        } else {
                            half - position - 1
                        };
                        let second = if phase == 0 {
                            half * 2 - position - 1
                        } else {
                            half + position
                        };
                        track[first] = color;
                        track[second] = color;
                    }
                }
                frames.push(place(&track, side, fans));
            } else {
                frames.push(vec![[0; 3]; fans * 40]);
            }
        }
    }
    frames
}

pub(super) fn shuttle_run(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let per_fan = side.leds_per_track();
    let len = fans * per_fan;
    let width = per_fan / 2;
    let colors = palette_all(effect).map(|color| scale(color, brightness(effect)));
    let mut frames = Vec::with_capacity(2 * (len + per_fan - 1));
    for pass in 0..2 {
        for step in 0..len + per_fan - 1 {
            let mut first = vec![[0; 3]; len];
            let mut second = vec![[0; 3]; len];
            for position in 0..len {
                for (bank, track) in [&mut first, &mut second].into_iter().enumerate() {
                    let even = (position / per_fan).is_multiple_of(2);
                    let label = if (bank == 0) == even { 2 } else { 1 };
                    let delay = if label == 2 { width } else { 0 };
                    if position + delay <= step && position + delay + width > step {
                        let swap = pass == 1 && fans % 2 == 1;
                        let color = usize::from((label == 2) ^ swap);
                        let target = if pass == 0 {
                            position
                        } else {
                            len - position - 1
                        };
                        track[target] = colors[color];
                    }
                }
            }
            frames.push(place_banks(&first, &second, side, fans));
        }
    }
    frames
}

pub(super) fn gradient_ribbon(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let bright = brightness(effect);
    let clockwise = matches!(effect.direction, RgbDirection::Clockwise);
    let (repeat, first_offset, second_offset) = match side {
        Side::Inner => (3, 8, 16),
        Side::Outer => (2, 0, 24),
    };
    let len = fans * side.leds_per_track();
    (0..96)
        .map(|frame| {
            let make_track = |offset: usize| {
                let mut track = Vec::with_capacity(16 * repeat);
                for sample in 0..16 {
                    let color = scale(rainbow_color(frame + offset + sample, 96), bright);
                    track.extend(std::iter::repeat_n(color, repeat));
                }
                if clockwise {
                    track.reverse();
                }
                track
            };
            let first = make_track(first_offset);
            let second = make_track(second_offset);
            place_banks(&first[..len], &second[..len], side, fans)
        })
        .collect()
}

pub(super) fn twinkle(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let led_count = fans * 40;
    let colors = palette_all(effect);
    let bright = brightness(effect);
    let mut frames = vec![vec![[0; 3]; led_count]; 200];
    for &(led, start) in PULSE_STARTS {
        if led >= led_count || !selected(led, side) {
            continue;
        }
        for (offset, intensity) in PULSE.into_iter().enumerate() {
            let frame = start + offset as i16;
            if (0..200).contains(&frame) {
                frames[frame as usize][led] =
                    scale(scale(colors[COLOR_INDEX[led]], intensity), bright);
            }
        }
    }
    for &(led, frame) in SINGLE_SPARKS {
        if led < led_count && selected(led, side) {
            frames[frame][led] = scale(scale(colors[COLOR_INDEX[led]], 5), bright);
        }
    }
    frames
}

fn selected(led: usize, side: Side) -> bool {
    let led = led % 40;
    match side {
        Side::Inner => led < 12 || (20..32).contains(&led),
        Side::Outer => (12..20).contains(&led) || led >= 32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_shared::rgb::RgbMode;

    fn effect(mode: RgbMode) -> RgbEffect {
        RgbEffect {
            mode,
            colors: vec![[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]],
            brightness: 4,
            ..Default::default()
        }
    }

    #[test]
    fn pioneer_shrinks_outer_then_grows_inner() {
        let outer = pioneer(&effect(RgbMode::Pioneer), 1, Side::Outer);
        let inner = pioneer(&effect(RgbMode::Pioneer), 1, Side::Inner);
        assert_eq!(outer.len(), 12);
        assert_eq!(outer[0][12..20], [[254, 0, 0]; 8]);
        assert!(outer[5..].iter().all(|frame| frame[12..20] == [[0; 3]; 8]));
        assert!(inner[..5].iter().all(|frame| frame[..12] == [[0; 3]; 12]));
        assert_eq!(inner[6][5], [254, 0, 0]);
        assert_eq!(inner[6][6], [254, 0, 0]);
    }

    #[test]
    fn shuttle_run_preserves_the_two_source_bank_patterns() {
        let frames = shuttle_run(&effect(RgbMode::ShuttleRun), 1, Side::Outer);
        assert_eq!(frames.len(), 30);
        assert_eq!(frames[0][12], [0; 3]);
        assert_eq!(frames[0][32], [254, 0, 0]);
        assert_eq!(frames[4][12], [0, 254, 0]);
        assert_eq!(frames[4][33], [254, 0, 0]);
    }

    #[test]
    fn gradient_ribbon_uses_native_bank_offsets_and_direction() {
        let mut effect = effect(RgbMode::GradientRibbon);
        let clockwise = gradient_ribbon(&effect, 1, Side::Inner);
        effect.direction = RgbDirection::CounterClockwise;
        let counter = gradient_ribbon(&effect, 1, Side::Inner);
        assert_eq!(clockwise.len(), 96);
        assert_eq!(clockwise[0][0], [71, 183, 0]);
        assert_eq!(counter[0][0], [191, 63, 0]);
        assert_eq!(counter[0][20], [127, 127, 0]);
    }

    #[test]
    fn twinkle_matches_extracted_sparse_source_columns() {
        let frames = twinkle(&effect(RgbMode::Twinkle), 4, Side::Outer);
        assert_eq!(frames.len(), 200);
        assert_eq!(frames[35][157], [3, 3, 0]);
        assert_eq!(frames[129][104], [0, 0, 0]);
        assert!(frames.iter().any(|frame| frame[158] != [0; 3]));
    }
}
