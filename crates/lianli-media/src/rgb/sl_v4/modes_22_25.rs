use super::basic::rainbow_color;
use super::engine::{brightness, palette_all, place, place_banks, scale, Color, Side};
use super::shuttle_masks::{ARRAY3, ARRAY4};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

const COLOR_INDEX: [usize; 174] = [
    1, 0, 2, 3, 1, 2, 1, 0, 2, 3, 0, 3, 1, 0, 1, 2, 0, 2, 1, 0, 3, 1, 3, 0, 1, 0, 2, 0, 1, 3, 1, 2,
    3, 1, 2, 1, 0, 2, 1, 2, 0, 2, 0, 1, 3, 0, 2, 3, 1, 0, 1, 2, 3, 0, 1, 2, 1, 3, 0, 3, 2, 1, 0, 3,
    0, 2, 1, 0, 3, 0, 1, 2, 0, 3, 2, 1, 0, 1, 2, 3, 0, 1, 2, 1, 3, 2, 1, 0, 1, 3, 2, 0, 2, 1, 0, 1,
    3, 2, 1, 0, 2, 3, 0, 1, 3, 0, 2, 3, 1, 2, 0, 3, 1, 2, 3, 2, 0, 1, 2, 3, 1, 2, 1, 2, 0, 1, 2, 0,
    1, 3, 0, 2, 3, 1, 2, 0, 1, 2, 3, 2, 1, 2, 0, 1, 3, 0, 1, 2, 1, 2, 3, 2, 0, 2, 3, 0, 1, 3, 0, 2,
    0, 1, 3, 0, 1, 3, 2, 0, 1, 3, 0, 2, 1, 0,
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
    (161, 4),
    (161, 110),
    (162, 139),
    (163, 27),
    (163, 159),
    (164, -9),
    (164, 191),
    (167, 44),
    (167, 103),
    (168, 143),
    (169, 23),
    (169, 166),
    (170, 66),
    (171, 0),
    (171, 100),
    (172, 181),
    (173, 18),
    (173, 121),
];

const SINGLE_SPARKS: &[(usize, usize)] = &[
    (9, 64),
    (48, 0),
    (70, 107),
    (157, 35),
    (161, 131),
    (164, 93),
];

pub(super) fn pioneer(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let internal_len = [14, 26, 40, 52][fans - 1];
    let half = internal_len / 2;
    let output_len = fans * 13;
    let color = scale(palette_all(effect)[0], brightness(effect));
    let mut frames = Vec::with_capacity(2 * (half + 2 * fans - 1));
    for phase in 0..2 {
        let span = half + 2 * fans - 1;
        for step in 0..span {
            if (phase == 0 && matches!(side, Side::Outer))
                || (phase == 1 && matches!(side, Side::Inner))
            {
                let mut track = vec![[0; 3]; internal_len];
                for position in 0..half {
                    let lit = (phase == 0 && position >= step) || (phase == 1 && position < step);
                    if lit {
                        let first = if phase == 0 {
                            position
                        } else {
                            half - position - 1
                        };
                        let second = if phase == 0 {
                            internal_len - position - 1
                        } else {
                            half + position
                        };
                        track[first] = color;
                        track[second] = color;
                    }
                }
                frames.push(place(&track[..output_len], side, fans));
            } else {
                frames.push(vec![[0; 3]; fans * 52]);
            }
        }
    }
    frames
}

pub(super) fn shuttle_run(effect: &RgbEffect, fans: usize, side: Side) -> Vec<Vec<Color>> {
    let len = fans * 13;
    let colors = palette_all(effect).map(|color| scale(color, brightness(effect)));
    let mut frames = Vec::with_capacity(2 * (fans + 1) * 13);
    for pass in 0..2 {
        for step in 0..(fans + 1) * 13 {
            let mut first = vec![[0; 3]; len];
            let mut second = vec![[0; 3]; len];
            for position in 0..len {
                let target = if pass == 0 {
                    position
                } else {
                    len - position - 1
                };
                for (masks, track) in [(ARRAY3, &mut first), (ARRAY4, &mut second)] {
                    let (ones, twos) = masks[step];
                    let label = if ones & (1 << position) != 0 {
                        Some(0)
                    } else if twos & (1 << position) != 0 {
                        Some(1)
                    } else {
                        None
                    };
                    if let Some(mut color) = label {
                        if pass == 1 && fans % 2 == 1 {
                            color = 1 - color;
                        }
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
    let (first_offset, second_offset) = match side {
        Side::Inner => (8, 16),
        Side::Outer => (0, 24),
    };
    let len = fans * side.leds_per_track();
    (0..96)
        .map(|frame| {
            let make_track = |offset: usize| {
                let mut track = Vec::with_capacity(54);
                for sample in 0..18 {
                    let color = scale(rainbow_color(frame + offset + sample, 96), bright);
                    track.extend(std::iter::repeat_n(color, 3));
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

pub(super) fn twinkle(effect: &RgbEffect, fans: usize, _side: Side) -> Vec<Vec<Color>> {
    let led_count = fans * 52;
    let colors = palette_all(effect);
    let bright = brightness(effect);
    let mut frames = vec![vec![[0; 3]; led_count]; 200];
    for &(led, start) in PULSE_STARTS {
        for target in [led, led + 174] {
            if target >= led_count {
                continue;
            }
            for (offset, intensity) in PULSE.into_iter().enumerate() {
                let frame = start + offset as i16;
                if (0..200).contains(&frame) {
                    frames[frame as usize][target] =
                        scale(scale(colors[COLOR_INDEX[led]], intensity), bright);
                }
            }
        }
    }
    for &(led, frame) in SINGLE_SPARKS {
        for target in [led, led + 174] {
            if target < led_count {
                frames[frame][target] = scale(scale(colors[COLOR_INDEX[led]], 5), bright);
            }
        }
    }
    frames
}
