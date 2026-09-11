use super::engine::{gradient, scaled_colors, select_side, validate, Color, Matrix};
use anyhow::Result;
use lianli_shared::rgb::RgbEffect;

const INTERNAL_SPEED: f32 = 10.0;

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<Color>>> {
    validate(effect, fans)?;
    let colors = scaled_colors(effect);
    let matrix = Matrix::new(fans);
    let (left_ribbon, right_ribbon, blended_ribbon) = init_ribbons(&matrix, &colors);
    let width = (matrix.max_x - matrix.min_x) / 4;
    let mut step = -((matrix.max_x - matrix.min_x) / 6 / 2) as f32;
    let mut is_back = false;
    let mut frames = Vec::new();

    loop {
        if !is_back && step > (matrix.center_x - width / 2) as f32 {
            step = (matrix.center_x + width / 3 * 2) as f32;
            is_back = true;
        }
        if step < -(width * 4) as f32 {
            break;
        }

        let mut frame = Vec::with_capacity(matrix.points.len());
        for point in &matrix.points {
            let color = if is_back {
                blended_color(point.x, &matrix, step, width, &blended_ribbon)
            } else {
                incoming_color(point.x, &matrix, step, width, &left_ribbon, &right_ribbon)
            };
            frame.push(post_scale(color, effect.brightness));
        }
        frames.push(select_side(&frame, fans, bottom));

        if is_back {
            step -= INTERNAL_SPEED * 1.5 + 20.3;
        } else {
            step += INTERNAL_SPEED * 0.8 + 10.3;
        }
    }
    Ok(frames)
}

fn init_ribbons(matrix: &Matrix, colors: &[Color]) -> (Vec<Color>, Vec<Color>, Vec<Color>) {
    let width = (matrix.max_x - matrix.min_x) / 6;
    let left = colors[0];
    let right = colors.get(1).copied().unwrap_or([0; 3]);
    let blended = blend(left, right);
    let incoming_rise = width / 5 * 2;
    let blended_fall = width * 4 / 5 * 2;

    let mut left_ribbon = gradient([0; 3], left, incoming_rise as usize);
    left_ribbon.extend(gradient(left, left, (width - incoming_rise) as usize));
    let mut right_ribbon = gradient([0; 3], right, incoming_rise as usize);
    right_ribbon.extend(gradient(right, right, (width - incoming_rise) as usize));
    let mut blended_ribbon = gradient(blended, blended, (width * 4 - blended_fall) as usize);
    blended_ribbon.extend(gradient(blended, [0; 3], blended_fall as usize));
    (left_ribbon, right_ribbon, blended_ribbon)
}

fn blend(left: Color, right: Color) -> Color {
    let average = std::array::from_fn::<_, 3, _>(|channel| {
        f64::from(left[channel]) * 0.5 + f64::from(right[channel]) * 0.5
    });
    let max = average.into_iter().fold(0.0f64, f64::max);
    if max == 0.0 {
        return [0; 3];
    }
    let scale = 255.0 / max;
    average.map(|channel| (channel * scale) as u8)
}

fn incoming_color(
    x: i32,
    matrix: &Matrix,
    step: f32,
    width: i32,
    left: &[Color],
    right: &[Color],
) -> Color {
    if x < matrix.center_x {
        if x as f32 > step && (x as f32) < step + width as f32 {
            ribbon_color(left, (x as f32 - step) as usize)
        } else {
            [0; 3]
        }
    } else if (x as f32) < matrix.max_x as f32 - step
        && (x as f32) > matrix.max_x as f32 - step - width as f32
    {
        ribbon_color(right, (matrix.max_x as f32 - x as f32 - step) as usize)
    } else {
        [0; 3]
    }
}

fn blended_color(x: i32, matrix: &Matrix, step: f32, width: i32, ribbon: &[Color]) -> Color {
    if x < matrix.center_x {
        if x as f32 > step && (x as f32) < step + (width * 8) as f32 {
            ribbon_color(ribbon, (x as f32 - step) as usize)
        } else {
            [0; 3]
        }
    } else if (x as f32) < matrix.max_x as f32 - step
        && (x as f32) > matrix.max_x as f32 - step - (width * 8) as f32
    {
        ribbon_color(ribbon, (matrix.max_x as f32 - x as f32 - step) as usize)
    } else {
        [0; 3]
    }
}

fn ribbon_color(ribbon: &[Color], index: usize) -> Color {
    ribbon[index.min(ribbon.len() - 1)]
}

fn post_scale(color: Color, brightness: u8) -> Color {
    let scale = [0u16, 64, 128, 192, 255][brightness as usize];
    color.map(|channel| ((u16::from(channel) * scale) >> 8) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_shared::rgb::{RgbDirection, RgbMode};

    fn effect(brightness: u8, direction: RgbDirection) -> RgbEffect {
        RgbEffect {
            mode: RgbMode::Mixing,
            colors: vec![[255, 0, 0], [0, 0, 255]],
            brightness,
            direction,
            ..RgbEffect::default()
        }
    }

    #[test]
    fn sends_two_colors_inward_then_emits_the_normalized_blend() {
        let frames = render(&effect(4, RgbDirection::Clockwise), 1, false).unwrap();

        assert_eq!(frames.len(), 14);
        assert_eq!(frames[0][..3], [[253, 0, 0]; 3]);
        assert_eq!(frames[0][3], [0; 3]);
        assert_eq!(frames[0][10..13], [[0, 0, 253]; 3]);
        assert_eq!(frames[7][4..9], [[254, 0, 254]; 5]);
    }

    #[test]
    fn applies_the_vendor_post_blend_brightness_pass() {
        let frames = render(&effect(2, RgbDirection::Clockwise), 1, false).unwrap();

        assert_eq!(frames[0][0], [63, 0, 0]);
        assert_eq!(frames[7][4], [127, 0, 127]);
    }

    #[test]
    fn direction_does_not_change_the_vendor_animation() {
        let clockwise = render(&effect(4, RgbDirection::Clockwise), 2, true).unwrap();
        let counter = render(&effect(4, RgbDirection::CounterClockwise), 2, true).unwrap();
        assert_eq!(clockwise, counter);
    }
}
