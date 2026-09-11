use anyhow::Result;
use lianli_shared::rgb::RgbEffect;

use super::engine::{scaled_colors, select_side, validate, Color, Matrix, LEDS_PER_FAN};

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<Color>>> {
    validate(effect, fans)?;
    let matrix = Matrix::new(fans);
    let colors = scaled_colors(effect);
    let mut current = vec![[0; 3]; fans * LEDS_PER_FAN];
    let mut frames = Vec::new();
    let mut step = 0.0f32;

    for color in colors {
        while step <= matrix.center_x as f32 {
            let span = matrix.max_x - matrix.min_x;
            for (led, point) in current.iter_mut().zip(&matrix.points) {
                if point.x as f32 <= step + 15.0 || point.x as f32 >= span as f32 - step - 15.0 {
                    *led = color;
                }
            }
            frames.push(select_side(&current, fans, bottom));
            step += 10.0 * 0.3 + 6.3;
        }
        step = 0.0;
    }

    Ok(frames)
}

#[cfg(test)]
mod tests {
    use lianli_shared::rgb::{RgbDirection, RgbMode};

    use super::*;

    fn effect(direction: RgbDirection) -> RgbEffect {
        RgbEffect {
            mode: RgbMode::Tide,
            colors: vec![[255, 128, 1]],
            brightness: 4,
            direction,
            ..RgbEffect::default()
        }
    }

    #[test]
    fn tide_gathers_from_both_ends_and_keeps_the_other_side_dark() {
        let frames = render(&effect(RgbDirection::Clockwise), 1, false).unwrap();
        assert_eq!(frames.len(), 10);
        assert_eq!(
            frames[0][..13],
            [
                [254, 127, 0],
                [254, 127, 0],
                [0, 0, 0],
                [0, 0, 0],
                [0, 0, 0],
                [0, 0, 0],
                [0, 0, 0],
                [0, 0, 0],
                [0, 0, 0],
                [0, 0, 0],
                [0, 0, 0],
                [254, 127, 0],
                [254, 127, 0],
            ]
        );
        assert!(frames[0][13..].iter().all(|color| *color == [0; 3]));
        assert!(frames.last().unwrap()[..13]
            .iter()
            .all(|color| *color == [254, 127, 0]));
    }

    #[test]
    fn tide_direction_does_not_change_vendor_output() {
        assert_eq!(
            render(&effect(RgbDirection::Clockwise), 2, true).unwrap(),
            render(&effect(RgbDirection::CounterClockwise), 2, true).unwrap()
        );
    }
}
