use super::geometry::{frame, scale, Color};

const COLORS: [Color; 7] = [
    [153, 0, 204],
    [255, 51, 204],
    [255, 153, 0],
    [255, 255, 0],
    [0, 255, 102],
    [51, 255, 255],
    [66, 87, 248],
];
const COLOR_INDEX: [usize; 35] = [
    1, 0, 6, 3, 4, 2, 5, 0, 1, 3, 6, 4, 1, 0, 5, 2, 4, 3, 1, 0, 2, 6, 3, 4, 1, 0, 2, 5, 1, 3, 4, 2,
    6, 1, 2,
];
const PULSE: [u8; 19] = [
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
];

pub(super) fn render(brightness: u8) -> Vec<Vec<Color>> {
    let mut frames = (0..200).map(|_| frame()).collect::<Vec<_>>();
    for &(led, start) in PULSE_STARTS {
        for (offset, intensity) in PULSE.into_iter().enumerate() {
            let frame_index = start + offset as i16;
            if (0..200).contains(&frame_index) {
                frames[frame_index as usize][led] =
                    scale(scale(COLORS[COLOR_INDEX[led]], intensity), brightness);
            }
        }
    }
    frames[64][9] = scale(scale(COLORS[COLOR_INDEX[9]], 5), brightness);
    frames
}
