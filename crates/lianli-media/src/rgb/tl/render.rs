use super::engine::{scaled_colors, select_side, validate, Color, Matrix, LEDS_PER_FAN};
use anyhow::Result;
use lianli_shared::rgb::{RgbDirection, RgbEffect};

const STEP: f32 = 8.3;
const FAN_WIDTH: i32 = 203;
const FAN_RADIUS: f32 = 101.0;
const CENTER_Y: i32 = 83;

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<Color>>> {
    validate(effect, fans)?;
    let colors = scaled_colors(effect);
    let matrix = Matrix::new(fans);
    let counter_clockwise = matches!(effect.direction, RgbDirection::CounterClockwise);
    let mut fan_index = if counter_clockwise { fans - 1 } else { 0 };
    let mut is_back = false;
    let mut color_index = 1;
    let mut step = 0.0f32;
    let mut current = vec![colors[0]; fans * LEDS_PER_FAN];
    let mut frames = Vec::new();

    loop {
        let fan_center = fan_index as i32 * FAN_WIDTH + 93;
        if step > FAN_RADIUS {
            step = 0.0;
            if is_back {
                let finished = if counter_clockwise {
                    fan_index += 1;
                    fan_index >= fans
                } else if fan_index == 0 {
                    true
                } else {
                    fan_index -= 1;
                    false
                };
                if finished {
                    is_back = false;
                    fan_index = if counter_clockwise { fans - 1 } else { 0 };
                    color_index += 1;
                    if color_index >= colors.len() {
                        break;
                    }
                    continue;
                }
            } else {
                let finished = if counter_clockwise {
                    if fan_index == 0 {
                        true
                    } else {
                        fan_index -= 1;
                        false
                    }
                } else {
                    fan_index += 1;
                    fan_index >= fans
                };
                if finished {
                    is_back = true;
                    fan_index = if counter_clockwise { 0 } else { fans - 1 };
                    continue;
                }
            }
        }

        if color_index >= colors.len() {
            color_index = 0;
        }
        let color = colors[color_index];
        let fan_offset = fan_index * LEDS_PER_FAN;
        for (led, point) in matrix.points[fan_offset..fan_offset + LEDS_PER_FAN]
            .iter()
            .enumerate()
        {
            let selected_half = if is_back {
                point.y > CENTER_Y
            } else {
                point.y < CENTER_Y
            };
            let within_left =
                point.x < fan_center + 5 && (point.x as f32) >= fan_center as f32 - step - 5.0;
            let within_right =
                point.x > fan_center && (point.x as f32) < fan_center as f32 + step + 5.0;
            if selected_half && (within_left || within_right) {
                current[fan_offset + led] = color;
            }
        }
        frames.push(select_side(&current, fans, bottom));
        step += STEP;
    }

    Ok(frames)
}
