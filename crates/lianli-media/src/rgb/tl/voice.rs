use super::engine::{select_side, validate, Color, Matrix};
use anyhow::Result;
use lianli_shared::rgb::RgbEffect;

const STEP: f32 = 25.3;
const SEED: i32 = 0;
use super::random::{DotNetRandom, COLORS};
fn scale(color: Color, brightness: u8) -> Color {
    let brightness = [0u16, 64, 128, 192, 255][brightness as usize];
    color.map(|channel| ((u16::from(channel) * brightness) >> 8) as u8)
}

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<Color>>> {
    // A fixed seed keeps separately rendered TL halves on the same native-valid rhythm sequence.
    render_seeded(effect, fans, bottom, SEED)
}

pub(super) fn render_seeded(
    effect: &RgbEffect,
    fans: usize,
    bottom: bool,
    seed: i32,
) -> Result<Vec<Vec<Color>>> {
    validate(effect, fans)?;
    let matrix = Matrix::new(fans);
    let mut random = DotNetRandom::new(seed);
    let mut max_step = random.next(matrix.min_x, matrix.center_x);
    let mut color_index = 0;
    let mut completed = 0;
    let mut step = 0.0f32;
    let mut is_back = false;
    let mut frames = Vec::new();

    loop {
        if step > max_step as f32 {
            is_back = true;
        }
        if step < 0.0 {
            step = 0.0;
            is_back = false;
            max_step = random.next(matrix.center_x / 5, matrix.center_x);
            color_index = random.next(0, COLORS.len() as i32) as usize;
            if completed == COLORS.len() {
                break;
            }
            completed += 1;
        }

        let color = scale(COLORS[color_index], effect.brightness);
        let frame = matrix
            .points
            .iter()
            .map(|point| {
                let lit = if point.x > matrix.center_x {
                    (point.x as f32) < matrix.center_x as f32 + step
                } else if is_back {
                    (point.x as f32) >= matrix.center_x as f32 - step && point.x < matrix.center_x
                } else {
                    point.x as f32 > matrix.center_x as f32 - step
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
