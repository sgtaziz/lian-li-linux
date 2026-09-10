use super::engine::{scaled_colors, select_side, validate, Color, Matrix};
use anyhow::Result;
use lianli_shared::rgb::RgbEffect;

const STEP: f32 = 15.3;

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<Color>>> {
    validate(effect, fans)?;
    let colors = scaled_colors(effect);
    let matrix = Matrix::new(fans);
    let band_width = (8 * fans) as f32;
    let spacing = (50 * fans) as f32;
    let mut step = 0.0f32;
    let mut color_index = 0;
    let mut frames = Vec::new();

    loop {
        if f64::from(step) > f64::from(matrix.max_x) * 1.5 {
            step = 0.0;
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
                let x = point.x as f32;
                let lit = if point.x > matrix.center_x {
                    (0..4).any(|band| {
                        let start = matrix.center_x as f32 + step - spacing * band as f32;
                        x >= start && x < start + band_width
                    })
                } else {
                    (0..4).any(|band| {
                        let end = matrix.center_x as f32 - step + spacing * band as f32;
                        x < end && x >= end - band_width
                    })
                };
                if lit {
                    color
                } else {
                    [0; 3]
                }
            })
            .collect::<Vec<_>>();
        frames.push(select_side(&frame, fans, bottom));
        step += STEP;
    }

    Ok(frames)
}
