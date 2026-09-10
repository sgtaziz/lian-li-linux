use super::engine::{scaled_colors, select_side, validate, Color, Matrix};
use anyhow::{ensure, Result};
use lianli_shared::rgb::{RgbEffect, RgbMode};

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<Color>>> {
    validate(effect, fans)?;
    let electric = effect.mode == RgbMode::ElectricCurrent;
    ensure!(
        electric || effect.colors.len() >= 2,
        "Collide requires two colors"
    );
    let colors = scaled_colors(effect);
    let matrix = Matrix::new(fans);
    let mut width = 25 + 15 * fans;
    let mut ribbon = make_ribbon(colors[0], width, electric);
    let mut frames = Vec::new();
    let mut step = 0.0f32;
    let mut backwards = false;
    let mut color = 0;
    loop {
        if step > matrix.center_x as f32 + width as f32 {
            backwards = true;
            if electric {
                width = 30 + 50 * fans;
                step = matrix.center_x as f32 + width as f32;
            } else {
                color += 1;
                if color == colors.len() {
                    break;
                }
            }
            ribbon = make_ribbon(colors[color], width, electric);
        }
        if step < 0.0 && (!electric || backwards) {
            backwards = false;
            if electric {
                width = 25 + 15 * fans;
                color += 1;
                if color == colors.len() {
                    break;
                }
                ribbon = make_ribbon(colors[color], width, true);
            }
        }
        let frame = matrix
            .points
            .iter()
            .map(|point| sample(point.x, &matrix, step, width, backwards, &ribbon))
            .collect::<Vec<_>>();
        frames.push(select_side(&frame, fans, bottom));
        step += if electric {
            if backwards {
                -45.3
            } else {
                15.3
            }
        } else if backwards {
            -28.3
        } else {
            28.3
        };
    }
    Ok(frames)
}

fn make_ribbon(color: Color, width: usize, electric: bool) -> Vec<Color> {
    if !electric {
        return gradient([0; 3], color, width);
    }
    let rise = width / 5 * 2;
    let mut ribbon = gradient([0; 3], color, rise);
    ribbon.extend(gradient(color, color, width - rise));
    ribbon
}

fn gradient(start: Color, end: Color, steps: usize) -> Vec<Color> {
    (0..=steps)
        .map(|step| {
            let fraction = step as f32 / steps as f32;
            std::array::from_fn(|channel| {
                (i32::from(start[channel])
                    + ((i32::from(end[channel]) - i32::from(start[channel])) as f32 * fraction)
                        as i32) as u8
            })
        })
        .collect()
}

fn sample(
    x: i32,
    matrix: &Matrix,
    step: f32,
    width: usize,
    backwards: bool,
    ribbon: &[Color],
) -> Color {
    let width = width as f32;
    let left = if step > matrix.center_x as f32 {
        x < matrix.center_x && x as f32 > step - width
    } else {
        x as f32 > step - width && (x as f32) < step
    };
    let right = if step > matrix.center_x as f32 {
        x > matrix.center_x && (x as f32) < matrix.max_x as f32 - step + width
    } else {
        (x as f32) < matrix.max_x as f32 - step + width && x as f32 > matrix.max_x as f32 - step
    };
    let index = if left {
        let index = (x as f32 - (step - width)) as i32;
        if backwards {
            ribbon.len() as i32 - 1 - index
        } else {
            index
        }
    } else if right {
        let index = (x as f32 - (matrix.max_x as f32 - step)) as i32;
        if backwards {
            index
        } else {
            ribbon.len() as i32 - 1 - index
        }
    } else {
        return [0; 3];
    };
    ribbon[index.clamp(0, ribbon.len() as i32 - 1) as usize]
}
