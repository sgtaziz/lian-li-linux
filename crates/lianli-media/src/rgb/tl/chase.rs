use anyhow::{ensure, Result};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

use super::engine::{gradient, scaled_colors, select_side, validate, Color, Matrix};

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<Color>>> {
    validate(effect, fans)?;
    ensure!(
        effect.colors.len() >= 2,
        "TL tail chasing requires two colors"
    );
    let colors = scaled_colors(effect);
    let mut matrix = Matrix::new(fans);
    matrix.transform_cruise();
    let width = 40 * fans as i32;
    let rise = width as usize / 5 * 2;
    let mut ribbon1 = gradient([0; 3], colors[0], rise);
    ribbon1.extend(gradient(colors[0], colors[0], width as usize - rise));
    let mut ribbon2 = gradient([0; 3], colors[1], rise);
    ribbon2.extend(gradient(colors[1], colors[1], width as usize - rise));
    let counter_clockwise = matches!(effect.direction, RgbDirection::CounterClockwise);
    let mut step = if counter_clockwise {
        matrix.max_x as f32
    } else {
        width as f32
    };
    let mut frames = Vec::new();

    loop {
        if counter_clockwise && step < 0.0
            || !counter_clockwise && step > (matrix.max_x + width) as f32
        {
            break;
        }

        let mut frame = vec![[0; 3]; matrix.points.len()];
        for (led, point) in frame.iter_mut().zip(&matrix.points) {
            let x = point.x as f32;
            if x < step && x >= step - width as f32 {
                let distance = (step - x) as isize;
                *led = ribbon1[ribbon_index(distance, ribbon1.len(), counter_clockwise)];
            } else if x < matrix.center_x as f32 + step
                && x >= matrix.center_x as f32 + step - width as f32
            {
                let distance = (matrix.center_x as f32 + step - x) as isize;
                *led = ribbon2[ribbon_index(distance, ribbon2.len(), counter_clockwise)];
            } else if x < step - matrix.center_x as f32
                && x >= step - matrix.center_x as f32 - width as f32
            {
                let distance = (step - matrix.center_x as f32 - x) as isize;
                *led = ribbon2[ribbon_index(distance, ribbon2.len(), counter_clockwise)];
            } else if step > matrix.max_x as f32 {
                let wrapped_width = step - matrix.max_x as f32;
                if point.x >= 0 && x < wrapped_width {
                    let index =
                        clamp_index(ribbon1.len() as isize - 1 - point.x as isize, ribbon1.len());
                    *led = ribbon1[index];
                }
            } else if step < width as f32
                && counter_clockwise
                && x > matrix.max_x as f32 - (width as f32 - step)
                && point.x < matrix.max_x + width
            {
                *led = ribbon1[clamp_index(point.x as isize, ribbon1.len())];
            }
        }
        frames.push(select_side(&frame, fans, bottom));
        if counter_clockwise {
            if step <= 0.0 {
                step = matrix.max_x as f32;
            }
            step -= 10.0 * 0.8 + 8.3;
        } else {
            step += 10.0 * 0.8 + 8.3;
        }
    }

    Ok(frames)
}

fn ribbon_index(distance: isize, len: usize, counter_clockwise: bool) -> usize {
    let index = if counter_clockwise {
        distance
    } else {
        len as isize - 1 - distance
    };
    clamp_index(index, len)
}

fn clamp_index(index: isize, len: usize) -> usize {
    index.clamp(0, len as isize - 1) as usize
}

#[cfg(test)]
mod tests {
    use lianli_shared::rgb::RgbMode;

    use super::*;

    fn effect(direction: RgbDirection) -> RgbEffect {
        RgbEffect {
            mode: RgbMode::TailChasing,
            colors: vec![[255, 0, 0], [0, 255, 0]],
            brightness: 4,
            direction,
            ..RgbEffect::default()
        }
    }

    #[test]
    fn clockwise_tail_starts_with_the_primary_ribbon() {
        let frames = render(&effect(RgbDirection::Clockwise), 1, false).unwrap();
        assert_eq!(frames.len(), 24);
        assert_eq!(frames[0][0], [15, 0, 0]);
        assert_eq!(frames[0][1], [222, 0, 0]);
        assert_eq!(frames[0][2], [254, 0, 0]);
        assert!(frames[0][13..].iter().all(|color| *color == [0; 3]));
    }

    #[test]
    fn counter_clockwise_tail_enters_with_the_secondary_ribbon() {
        let frames = render(&effect(RgbDirection::CounterClockwise), 1, false).unwrap();
        assert_eq!(frames.len(), 24);
        assert_eq!(frames[0][10], [0, 254, 0]);
        assert_eq!(frames[0][12], [0, 79, 0]);
    }
}
