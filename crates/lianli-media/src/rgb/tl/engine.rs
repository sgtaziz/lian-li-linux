use anyhow::{ensure, Result};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

pub(super) type Color = [u8; 3];

pub(super) const LEDS_PER_FAN: usize = 26;
const LED_POINTS: [(i32, i32); LEDS_PER_FAN] = [
    (0, 63),
    (13, 46),
    (27, 29),
    (44, 17),
    (61, 7),
    (77, 0),
    (94, 0),
    (111, 0),
    (127, 7),
    (144, 17),
    (161, 29),
    (173, 46),
    (187, 63),
    (0, 99),
    (13, 116),
    (27, 133),
    (44, 146),
    (61, 156),
    (77, 166),
    (94, 166),
    (111, 166),
    (127, 156),
    (144, 146),
    (161, 133),
    (173, 116),
    (187, 99),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Point {
    pub x: i32,
    pub y: i32,
}

pub(super) struct Matrix {
    pub points: Vec<Point>,
    pub min_x: i32,
    pub max_x: i32,
    pub max_y: i32,
    pub center_x: i32,
    pub center_y: i32,
}

impl Matrix {
    pub fn new(fans: usize) -> Self {
        let mut points = Vec::with_capacity(fans * LEDS_PER_FAN);
        for fan in 0..fans {
            let x_offset = fan as i32 * 203;
            points.extend(
                LED_POINTS
                    .iter()
                    .map(|&(x, y)| Point { x: x + x_offset, y }),
            );
        }
        let mut matrix = Self {
            points,
            min_x: 0,
            max_x: 0,
            max_y: 0,
            center_x: 0,
            center_y: 0,
        };
        matrix.update_bounds();
        matrix
    }

    pub fn transform_wave(&mut self, fans: usize, direction: RgbDirection) {
        let original_max_x = self.max_x;
        let original_max_y = self.max_y;
        let original_center_y = self.center_y;

        if matches!(direction, RgbDirection::CounterClockwise) {
            for point in &mut self.points {
                point.x = original_max_x - point.x;
            }
            let even_fan_count = fans.is_multiple_of(2);
            for (fan, fan_points) in self.points.chunks_exact_mut(LEDS_PER_FAN).enumerate() {
                if if even_fan_count {
                    fan % 2 == 0
                } else {
                    fan % 2 != 0
                } {
                    for point in fan_points {
                        point.y = original_max_y - point.y;
                    }
                }
            }
        } else {
            for (fan, fan_points) in self.points.chunks_exact_mut(LEDS_PER_FAN).enumerate() {
                if fan % 2 != 0 {
                    for point in fan_points {
                        point.y = original_max_y - point.y;
                    }
                }
            }
        }

        for point in &mut self.points {
            if point.y > original_center_y {
                point.x = original_max_x + (original_max_x - point.x) + 10;
            }
        }
        self.update_bounds();
    }

    pub fn transform_cruise(&mut self) {
        let original_max_x = self.max_x;
        let original_center_y = self.center_y;
        for point in &mut self.points {
            if point.y > original_center_y {
                point.x = original_max_x + (original_max_x - point.x) + 10;
            }
        }
        self.update_bounds();
    }

    pub fn transform_cover(&mut self, direction: RgbDirection) {
        let original_max_x = self.max_x;
        let original_center_y = self.center_y;
        if matches!(direction, RgbDirection::CounterClockwise) {
            for point in &mut self.points {
                point.x = original_max_x - point.x;
            }
        }
        for point in &mut self.points {
            if point.y > original_center_y {
                point.x = original_max_x + (original_max_x - point.x) + 10;
            }
        }
        self.update_bounds();
    }

    pub fn transform_racing(&mut self) {
        let original_max_y = self.max_y;
        for (fan, fan_points) in self.points.chunks_exact_mut(LEDS_PER_FAN).enumerate() {
            if fan % 2 != 0 {
                for point in fan_points {
                    point.y = original_max_y - point.y;
                }
            }
        }
        self.update_bounds();
    }

    fn update_bounds(&mut self) {
        let min_y = self.points.iter().map(|point| point.y).min().unwrap_or(0);
        self.min_x = self.points.iter().map(|point| point.x).min().unwrap_or(0);
        self.max_x = self.points.iter().map(|point| point.x).max().unwrap_or(0);
        self.max_y = self.points.iter().map(|point| point.y).max().unwrap_or(0);
        self.center_x = (self.max_x - self.min_x) / 2;
        self.center_y = (self.max_y - min_y) / 2;
    }
}

pub(super) fn validate(effect: &RgbEffect, fans: usize) -> Result<()> {
    ensure!((1..=4).contains(&fans), "TL fan count must be 1..=4");
    ensure!(
        !effect.colors.is_empty()
            || matches!(
                effect.mode,
                lianli_shared::rgb::RgbMode::Voice | lianli_shared::rgb::RgbMode::Kaleidoscope
            ),
        "TL effect requires at least one color"
    );
    ensure!(
        effect.colors.len() <= 4,
        "TL palette supports at most four colors"
    );
    ensure!(effect.brightness <= 4, "RGB brightness must be 0..=4");
    Ok(())
}

pub(super) fn scaled_colors(effect: &RgbEffect) -> Vec<Color> {
    let brightness = [0u16, 64, 128, 192, 255][effect.brightness as usize];
    effect
        .colors
        .iter()
        .map(|color| color.map(|channel| ((u16::from(channel) * brightness) >> 8) as u8))
        .collect()
}

pub(super) fn gradient(start: Color, end: Color, steps: usize) -> Vec<Color> {
    (0..=steps)
        .map(|step| {
            let t = step as f32 / steps as f32;
            std::array::from_fn(|channel| {
                let delta = (i32::from(end[channel]) - i32::from(start[channel])) as f32;
                (i32::from(start[channel]) + (delta * t) as i32) as u8
            })
        })
        .collect()
}

pub(super) fn select_side(frame: &[Color], fans: usize, bottom: bool) -> Vec<Color> {
    let mut selected = vec![[0; 3]; fans * LEDS_PER_FAN];
    let side_offset = usize::from(bottom) * 13;
    for fan in 0..fans {
        let start = fan * LEDS_PER_FAN + side_offset;
        selected[start..start + 13].copy_from_slice(&frame[start..start + 13]);
    }
    selected
}

pub(super) fn place_side_track(track: &[Color], fans: usize, bottom: bool) -> Vec<Color> {
    let mut frame = vec![[0; 3]; fans * LEDS_PER_FAN];
    let side_offset = usize::from(bottom) * 13;
    for (fan, colors) in track.chunks_exact(13).enumerate() {
        let start = fan * LEDS_PER_FAN + side_offset;
        frame[start..start + 13].copy_from_slice(colors);
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradient_matches_vendor_endpoint_and_negative_delta_rounding() {
        assert_eq!(
            gradient([255, 0, 10], [0, 255, 5], 2),
            vec![[255, 0, 10], [128, 127, 8], [0, 255, 5]]
        );
    }

    #[test]
    fn wave_matrix_follows_the_vendor_serpentine_geometry() {
        let mut clockwise = Matrix::new(1);
        clockwise.transform_wave(1, RgbDirection::Clockwise);
        assert_eq!((clockwise.min_x, clockwise.max_x), (0, 384));
        assert_eq!(clockwise.points[0], Point { x: 0, y: 63 });
        assert_eq!(clockwise.points[13], Point { x: 384, y: 99 });

        let mut counter_clockwise = Matrix::new(1);
        counter_clockwise.transform_wave(1, RgbDirection::CounterClockwise);
        assert_eq!(counter_clockwise.points[0], Point { x: 187, y: 63 });
        assert_eq!(counter_clockwise.points[13], Point { x: 197, y: 99 });
    }
}
