use anyhow::Result;
use lianli_shared::rgb::{RgbDirection, RgbEffect};

use super::engine::{gradient, scaled_colors, select_side, validate, Color, Matrix};

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<Color>>> {
    validate(effect, fans)?;
    let matrix = Matrix::new(fans);
    let colors = scaled_colors(effect);
    let width = 50 + 20 * fans as i32;
    let mut step;
    let mut is_back;
    if matches!(effect.direction, RgbDirection::CounterClockwise) {
        step = matrix.max_x as f32;
        is_back = true;
    } else {
        step = 0.0;
        is_back = false;
    }

    let mut frames = Vec::new();
    for color in colors {
        let rise = width as usize / 5 * 2;
        let mut ribbon = gradient([0; 3], color, rise);
        ribbon.extend(gradient(color, color, width as usize - rise));

        loop {
            if !is_back && step > matrix.max_x as f32 {
                is_back = true;
                break;
            }
            if is_back && step < -(width as f32) {
                is_back = false;
                break;
            }

            let mut frame = vec![[0; 3]; matrix.points.len()];
            for (led, point) in frame.iter_mut().zip(&matrix.points) {
                if point.x as f32 > step && point.x as f32 <= step + width as f32 {
                    let distance = (point.x as f32 - step) as usize;
                    let index = if is_back {
                        ribbon.len() - 1 - distance
                    } else {
                        distance
                    }
                    .min(ribbon.len() - 1);
                    *led = ribbon[index];
                }
            }
            frames.push(select_side(&frame, fans, bottom));
            if is_back {
                step -= 10.0 + 14.3;
            } else {
                step += 10.0 + 14.3;
            }
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
            mode: RgbMode::PingPong,
            colors: vec![[255, 0, 0]],
            brightness: 4,
            direction,
            ..RgbEffect::default()
        }
    }

    #[test]
    fn clockwise_ping_pong_matches_vendor_first_pass() {
        let frames = render(&effect(RgbDirection::Clockwise), 1, false).unwrap();
        assert_eq!(frames.len(), 8);
        assert_eq!(frames[0][0], [0, 0, 0]);
        assert_eq!(frames[0][1], [117, 0, 0]);
        assert_eq!(frames[0][4], [254, 0, 0]);
        assert!(frames[0][13..].iter().all(|color| *color == [0; 3]));
    }

    #[test]
    fn counter_clockwise_ping_pong_starts_with_a_blank_boundary_frame() {
        let frames = render(&effect(RgbDirection::CounterClockwise), 1, true).unwrap();
        assert_eq!(frames.len(), 11);
        assert!(frames[0].iter().all(|color| *color == [0; 3]));
        assert!(frames[1][..13].iter().all(|color| *color == [0; 3]));
        assert_ne!(frames[1][25], [0; 3]);
    }
}
