use anyhow::Result;
use lianli_shared::rgb::RgbEffect;

use super::engine::{scaled_colors, select_side, validate, Color, Matrix};

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<Color>>> {
    validate(effect, fans)?;
    let colors = scaled_colors(effect);
    let mut matrix = Matrix::new(fans);
    matrix.transform_cover(effect.direction);
    let mut current = vec![*colors.last().unwrap(); matrix.points.len()];
    let increment = 2.0 + 10.0 * 0.73 + 1.5 * fans as f32;
    let mut frames = Vec::new();
    let mut step = 0.0f32;

    for color in colors {
        while step <= (matrix.max_x + 20) as f32 {
            for (led, point) in current.iter_mut().zip(&matrix.points) {
                if (point.x as f32) < step {
                    *led = color;
                }
            }
            frames.push(select_side(&current, fans, bottom));
            step += increment;
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
            mode: RgbMode::CoverCycle,
            colors: vec![[255, 0, 0], [0, 255, 0]],
            brightness: 4,
            direction,
            ..RgbEffect::default()
        }
    }

    #[test]
    fn cover_cycle_preserves_the_vendor_background_and_frame_count() {
        let frames = render(&effect(RgbDirection::Clockwise), 1, false).unwrap();
        assert_eq!(frames.len(), 76);
        assert!(frames[0][..13].iter().all(|color| *color == [0, 254, 0]));
        assert_eq!(frames[1][0], [254, 0, 0]);
        assert_eq!(frames[1][1], [0, 254, 0]);
        assert!(frames[1][13..].iter().all(|color| *color == [0; 3]));
    }

    #[test]
    fn counter_clockwise_cover_enters_from_the_other_end() {
        let frames = render(&effect(RgbDirection::CounterClockwise), 1, false).unwrap();
        assert_eq!(frames[1][11], [0, 254, 0]);
        assert_eq!(frames[1][12], [254, 0, 0]);
    }
}
