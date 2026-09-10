use anyhow::Result;
use lianli_shared::rgb::{RgbDirection, RgbEffect};

use super::engine::{scaled_colors, select_side, validate, Color, Matrix};

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<Color>>> {
    validate(effect, fans)?;
    let matrix = Matrix::new(fans);
    let colors = scaled_colors(effect);
    let mut current = vec![*colors.last().unwrap(); matrix.points.len()];
    let mut color_index = 0;
    let mut is_back = matches!(effect.direction, RgbDirection::CounterClockwise);
    let mut step = if is_back { matrix.max_x as f32 } else { 0.0 };
    let mut frames = Vec::new();

    loop {
        if step > matrix.max_x as f32 {
            is_back = true;
            color_index += 1;
            if color_index >= colors.len() {
                break;
            }
        }
        if step < 0.0 {
            is_back = false;
            color_index += 1;
            if color_index >= colors.len() {
                break;
            }
        }

        let color = colors[color_index];
        for (led, point) in current.iter_mut().zip(&matrix.points) {
            if is_back && point.x as f32 >= step || !is_back && (point.x as f32) < step {
                *led = color;
            }
        }
        frames.push(select_side(&current, fans, bottom));
        if is_back {
            step -= 10.0 + 14.3;
        } else {
            step += 10.0 + 14.3;
        }
    }

    Ok(frames)
}

#[cfg(test)]
mod tests {
    use lianli_shared::rgb::RgbMode;

    use super::*;

    fn effect(direction: RgbDirection) -> RgbEffect {
        RgbEffect {
            mode: RgbMode::Paint,
            colors: vec![[255, 0, 0], [0, 255, 0]],
            brightness: 4,
            direction,
            ..RgbEffect::default()
        }
    }

    #[test]
    fn clockwise_paint_starts_from_the_left_over_the_last_color() {
        let frames = render(&effect(RgbDirection::Clockwise), 1, false).unwrap();
        assert!(frames[0][..13].iter().all(|color| *color == [0, 254, 0]));
        assert_eq!(frames[1][0], [254, 0, 0]);
        assert_eq!(frames[1][1], [254, 0, 0]);
        assert_eq!(frames[1][2], [0, 254, 0]);
        assert!(frames[1][13..].iter().all(|color| *color == [0; 3]));
    }

    #[test]
    fn counter_clockwise_paint_starts_at_the_right_edge() {
        let frames = render(&effect(RgbDirection::CounterClockwise), 1, false).unwrap();
        assert!(frames[0][..12].iter().all(|color| *color == [0, 254, 0]));
        assert_eq!(frames[0][12], [254, 0, 0]);
    }
}
