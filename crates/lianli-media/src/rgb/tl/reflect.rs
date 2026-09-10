use anyhow::Result;
use lianli_shared::rgb::{RgbDirection, RgbEffect};

use super::engine::{gradient, scaled_colors, select_side, validate, Color, Matrix};

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<Color>>> {
    validate(effect, fans)?;
    let matrix = Matrix::new(fans);
    let colors = scaled_colors(effect);
    let counter_clockwise = matches!(effect.direction, RgbDirection::CounterClockwise);
    let width = 25 + 15 * fans as i32;
    let limit = matrix.max_x as f64 * 0.7;
    let initial_step = if counter_clockwise {
        limit as i32 as f32
    } else {
        0.0
    };
    let mut step = initial_step;
    let mut frames = Vec::new();

    for color in colors {
        let ribbon = gradient([0; 3], color, width as usize);
        loop {
            if counter_clockwise && step < 0.0 || !counter_clockwise && f64::from(step) > limit {
                break;
            }

            let mut frame = vec![[0; 3]; matrix.points.len()];
            for (led, point) in frame.iter_mut().zip(&matrix.points) {
                let distance = if point.x > matrix.center_x
                    && point.x as f32 > matrix.max_x as f32 - step
                    && point.x as f32 <= matrix.max_x as f32 - step + width as f32
                {
                    Some((point.x as f32 - (matrix.max_x as f32 - step)) as usize)
                } else if point.x <= matrix.center_x
                    && point.x as f32 >= step - width as f32
                    && (point.x as f32) < step
                {
                    Some((step - point.x as f32) as usize)
                } else {
                    None
                };
                if let Some(distance) = distance {
                    let index = if counter_clockwise {
                        distance
                    } else {
                        ribbon.len() - 1 - distance
                    }
                    .min(ribbon.len() - 1);
                    *led = ribbon[index];
                }
            }
            frames.push(select_side(&frame, fans, bottom));
            if counter_clockwise {
                step -= 10.0 * 0.6 + 6.3;
            } else {
                step += 10.0 * 0.6 + 6.3;
            }
        }
        step = initial_step;
    }

    Ok(frames)
}

#[cfg(test)]
mod tests {
    use lianli_shared::rgb::RgbMode;

    use super::*;

    fn effect(direction: RgbDirection) -> RgbEffect {
        RgbEffect {
            mode: RgbMode::Reflect,
            colors: vec![[255, 0, 0]],
            brightness: 4,
            direction,
            ..RgbEffect::default()
        }
    }

    #[test]
    fn clockwise_reflect_enters_from_both_outer_edges() {
        let frames = render(&effect(RgbDirection::Clockwise), 1, false).unwrap();
        assert_eq!(frames.len(), 11);
        assert!(frames[0].iter().all(|color| *color == [0; 3]));
        assert_eq!(frames[1][0], [177, 0, 0]);
        assert_eq!(frames[1][12], [177, 0, 0]);
        assert!(frames[1][13..].iter().all(|color| *color == [0; 3]));
    }

    #[test]
    fn counter_clockwise_reflect_reverses_the_ribbon() {
        let frames = render(&effect(RgbDirection::CounterClockwise), 1, false).unwrap();
        assert_eq!(frames.len(), 11);
        assert_eq!(frames[0][6], [234, 0, 0]);
        assert!(frames[0]
            .iter()
            .enumerate()
            .all(|(index, color)| index == 6 || *color == [0; 3]));
    }
}
