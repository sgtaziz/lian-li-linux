use super::engine::{gradient, scaled_colors, select_side, validate, Color, Matrix};
use anyhow::{ensure, Result};
use lianli_shared::rgb::RgbEffect;

const INTERNAL_SPEED: f32 = 10.0;

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<Color>>> {
    validate(effect, fans)?;
    ensure!(
        effect.colors.len() >= 2,
        "TL Intertwine requires two colors"
    );
    let colors = scaled_colors(effect);
    let mut matrix = Matrix::new(fans);
    matrix.transform_cruise();
    let width = 30 + 20 * fans as i32;
    let (first_ribbon, second_ribbon) = init_ribbons(width, &colors);
    let blend = blend(
        *first_ribbon.last().unwrap(),
        *second_ribbon.last().unwrap(),
    );
    let mut step = 0.0f32;
    let mut step_back = matrix.center_x as f32;
    let mut is_initial = true;
    let increment = INTERNAL_SPEED + 10.3;
    let mut frames = Vec::new();

    loop {
        if step > (matrix.max_x + width) as f32 {
            break;
        }
        if step_back < -width as f32 {
            step_back = (matrix.max_x - width) as f32;
            is_initial = false;
        }

        let mut frame = Vec::with_capacity(matrix.points.len());
        for point in &matrix.points {
            let color = mixed_color(
                point.x,
                &matrix,
                step,
                step_back,
                width,
                is_initial,
                &first_ribbon,
                &second_ribbon,
                blend,
            );
            frame.push(post_scale(color, effect.brightness));
        }
        frames.push(select_side(&frame, fans, bottom));
        step += increment;
        step_back -= increment;
    }
    Ok(frames)
}

fn init_ribbons(width: i32, colors: &[Color]) -> (Vec<Color>, Vec<Color>) {
    let rise = width / 5 * 2;
    let mut first = gradient([0; 3], colors[0], rise as usize);
    first.extend(gradient(colors[0], colors[0], (width - rise) as usize));
    let mut second = gradient([0; 3], colors[1], rise as usize);
    second.extend(gradient(colors[1], colors[1], (width - rise) as usize));
    (first, second)
}

#[allow(clippy::too_many_arguments)]
fn mixed_color(
    x: i32,
    matrix: &Matrix,
    step: f32,
    step_back: f32,
    width: i32,
    is_initial: bool,
    first: &[Color],
    second: &[Color],
    blend: Color,
) -> Color {
    let x = x as f32;
    let forward = x > step - width as f32 && x <= step;
    let backward = x >= step_back && x < step_back + width as f32;
    if forward && backward {
        blend
    } else if forward {
        ribbon_color(first, (x - (step - width as f32)) as usize)
    } else if backward {
        if is_initial && x > matrix.center_x as f32 {
            [0; 3]
        } else {
            let index = first.len() - 1 - (x - step_back) as usize;
            ribbon_color(second, index)
        }
    } else if step > matrix.max_x as f32
        && x < step - matrix.max_x as f32
        && step_back < 0.0
        && x > matrix.max_x as f32 + step_back
    {
        blend
    } else if step > matrix.max_x as f32 && x < step - matrix.max_x as f32 {
        let distance = (step - matrix.max_x as f32 - x) as usize;
        ribbon_color(first, first.len().saturating_sub(1 + distance))
    } else if step_back < 0.0 && x > matrix.max_x as f32 + step_back {
        ribbon_color(second, (x - (matrix.max_x as f32 + step_back)) as usize)
    } else {
        [0; 3]
    }
}

fn ribbon_color(ribbon: &[Color], index: usize) -> Color {
    ribbon[index.min(ribbon.len() - 1)]
}

fn blend(first: Color, second: Color) -> Color {
    let average = std::array::from_fn::<_, 3, _>(|channel| {
        f64::from(first[channel]) * 0.5 + f64::from(second[channel]) * 0.5
    });
    let max = average.into_iter().fold(0.0f64, f64::max);
    if max == 0.0 {
        return [0; 3];
    }
    average.map(|channel| (channel * (255.0 / max)) as u8)
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
            mode: RgbMode::Intertwine,
            colors: vec![[255, 0, 0], [0, 0, 255]],
            brightness,
            direction,
            ..RgbEffect::default()
        }
    }

    #[test]
    fn collides_opposing_ribbons_on_the_unfolded_strip() {
        let frames = render(&effect(4, RgbDirection::Clockwise), 1, false).unwrap();

        assert_eq!(frames.len(), 22);
        assert!(frames.iter().flatten().any(|color| *color == [254, 0, 254]));
        assert!(frames
            .iter()
            .flatten()
            .any(|color| color[0] > 0 && color[2] == 0));
        assert!(frames
            .iter()
            .flatten()
            .any(|color| color[2] > 0 && color[0] == 0));
    }

    #[test]
    fn applies_post_blend_brightness_and_ignores_direction() {
        let clockwise = render(&effect(2, RgbDirection::Clockwise), 1, true).unwrap();
        let counter = render(&effect(2, RgbDirection::CounterClockwise), 1, true).unwrap();

        assert_eq!(clockwise, counter);
        assert!(clockwise
            .iter()
            .flatten()
            .any(|color| *color == [127, 0, 127]));
    }
}
