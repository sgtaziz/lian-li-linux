use super::engine::{scaled_colors, select_side, validate, Color, Matrix};
use anyhow::Result;
use lianli_shared::rgb::RgbEffect;

const STEP: f32 = 9.3;

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<Color>>> {
    validate(effect, fans)?;
    let colors = scaled_colors(effect);
    let matrix = Matrix::new(fans);
    let mut step = 0.0f32;
    let mut is_back = false;
    let mut color_index = 0;
    let mut frames = Vec::new();

    loop {
        if step > (matrix.center_x + 15) as f32 {
            is_back = true;
        }
        if step < 0.0 {
            step = 0.0;
            is_back = false;
            color_index += 1;
            if color_index >= colors.len() {
                break;
            }
        }

        let color = colors[color_index];
        let frame = matrix
            .points
            .iter()
            .map(|point| {
                let lit = if point.x > matrix.center_x {
                    point.x as f32 > matrix.max_x as f32 - step
                } else {
                    (point.x as f32) < step
                };
                if lit {
                    color
                } else {
                    [0; 3]
                }
            })
            .collect::<Vec<_>>();
        frames.push(select_side(&frame, fans, bottom));

        if is_back {
            step -= STEP;
        } else {
            step += STEP;
        }
    }

    Ok(frames)
}
