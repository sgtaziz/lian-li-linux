use super::geometry::{frame, scale, Color, DotNetRandom};

const COLORS: [Color; 7] = [
    [153, 0, 204],
    [255, 51, 204],
    [255, 153, 0],
    [255, 255, 0],
    [0, 255, 102],
    [51, 255, 255],
    [66, 87, 248],
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
];
const SINGLE_SPARKS: &[(usize, usize)] = &[(9, 64), (48, 0)];

pub(super) fn render(led_count: usize, brightness: u8) -> Vec<Vec<Color>> {
    let mut random = DotNetRandom::new(0);
    let color_indices = std::array::from_fn::<_, 60, _>(|_| random.next(7) as usize);
    let mut frames = (0..200).map(|_| frame(led_count)).collect::<Vec<_>>();
    for &(led, start) in PULSE_STARTS {
        for (offset, intensity) in PULSE.into_iter().enumerate() {
            let frame_index = start + offset as i16;
            if (0..200).contains(&frame_index) {
                frames[frame_index as usize][led] =
                    scale(scale(COLORS[color_indices[led]], intensity), brightness);
            }
        }
    }
    for &(led, frame_index) in SINGLE_SPARKS {
        frames[frame_index][led] = scale(scale(COLORS[color_indices[led]], 5), brightness);
    }
    frames
}
