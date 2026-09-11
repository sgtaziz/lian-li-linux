use anyhow::Result;
use lianli_shared::rgb::RgbEffect;

use super::engine::{gradient, scaled_colors, select_side, validate, Color, Matrix};

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<Color>>> {
    validate(effect, fans)?;
    let mut matrix = Matrix::new(fans);
    matrix.transform_wave(fans, effect.direction);
    let colors = scaled_colors(effect);
    let width = 30 + 25 * fans as i32;
    let increment = 2.0 + 10.0 * 0.73 + 1.5 * fans as f32;
    let mut frames = Vec::new();
    let mut step = 0.0f32;

    for color in colors {
        let rise = width as usize / 5 * 2;
        let mut ribbon = gradient([0; 3], color, rise);
        ribbon.extend(gradient(color, color, width as usize - rise));

        while step <= (matrix.max_x + width) as f32 {
            let mut frame = vec![[0; 3]; matrix.points.len()];
            for (led, point) in frame.iter_mut().zip(&matrix.points) {
                if point.x as f32 > step - width as f32 && (point.x as f32) < step {
                    let distance = (step - point.x as f32) as usize;
                    let index = (ribbon.len() - 1 - distance).min(ribbon.len() - 1);
                    *led = ribbon[index];
                }
            }
            frames.push(select_side(&frame, fans, bottom));
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
            mode: RgbMode::Wave,
            colors: vec![[255, 64, 0]],
            brightness: 4,
            direction,
            ..RgbEffect::default()
        }
    }

    #[test]
    fn wave_uses_full_vendor_serpentine_path() {
        let clockwise = render(&effect(RgbDirection::Clockwise), 1, false).unwrap();
        let counter_clockwise = render(&effect(RgbDirection::CounterClockwise), 1, false).unwrap();

        assert_eq!(clockwise.len(), 41);
        assert_eq!(counter_clockwise.len(), 41);
        assert!(clockwise[0].iter().all(|color| *color == [0; 3]));
        assert_ne!(clockwise, counter_clockwise);
        assert!(clockwise
            .iter()
            .all(|frame| frame[13..].iter().all(|color| *color == [0; 3])));
    }
}
