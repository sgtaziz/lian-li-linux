use super::engine::{gradient, scaled_colors, select_side, validate, Color, Matrix};
use anyhow::{ensure, Result};
use lianli_shared::rgb::{RgbDirection, RgbEffect};

const INTERNAL_SPEED: f32 = 10.0;

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<Color>>> {
    validate(effect, fans)?;
    ensure!(effect.colors.len() >= 2, "TL Racing requires two colors");
    let colors = scaled_colors(effect);
    let mut matrix = Matrix::new(fans);
    let width = 30 + 25 * fans as i32;
    let (first_ribbon, second_ribbon) = init_ribbons(width, &colors);
    matrix.transform_racing();

    let counter_clockwise = matches!(effect.direction, RgbDirection::CounterClockwise);
    let mut step = if counter_clockwise {
        (matrix.max_x + width + width / 2) as f32
    } else {
        0.0
    };
    let mut is_back = counter_clockwise;
    let increment = 2.0 + INTERNAL_SPEED * 0.73 + 1.5 * fans as f32;
    let mut frames = Vec::new();

    loop {
        if step > (matrix.max_x + width + width / 2) as f32 {
            is_back = true;
            if counter_clockwise {
                break;
            }
        }
        if step < 0.0 {
            is_back = false;
            if !counter_clockwise {
                break;
            }
        }

        let mut frame = Vec::with_capacity(matrix.points.len());
        for point in &matrix.points {
            let (offset, ribbon) = if point.y < matrix.max_y / 2 {
                (0, &first_ribbon)
            } else {
                (width / 2, &second_ribbon)
            };
            let shifted_step = step - offset as f32;
            let color = if point.x as f32 > shifted_step - width as f32
                && (point.x as f32) < shifted_step
            {
                let distance = (shifted_step - point.x as f32) as usize;
                let index = if is_back {
                    distance
                } else {
                    ribbon.len().saturating_sub(1 + distance)
                };
                ribbon[index.min(ribbon.len() - 1)]
            } else {
                [0; 3]
            };
            frame.push(color);
        }
        frames.push(select_side(&frame, fans, bottom));

        if is_back {
            step -= increment;
        } else {
            step += increment;
        }
    }
    Ok(frames)
}

fn init_ribbons(width: i32, colors: &[Color]) -> (Vec<Color>, Vec<Color>) {
    let first = colors[0];
    let second = colors.get(1).copied().unwrap_or([0; 3]);
    let rise = width / 5 * 2;
    let mut first_ribbon = gradient([0; 3], first, rise as usize);
    first_ribbon.extend(gradient(first, first, (width - rise) as usize));
    let mut second_ribbon = gradient([0; 3], second, rise as usize);
    second_ribbon.extend(gradient(second, second, (width - rise) as usize));
    (first_ribbon, second_ribbon)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_shared::rgb::RgbMode;

    fn effect(direction: RgbDirection) -> RgbEffect {
        RgbEffect {
            mode: RgbMode::Racing,
            colors: vec![[255, 0, 0], [0, 255, 0]],
            brightness: 4,
            direction,
            ..RgbEffect::default()
        }
    }

    #[test]
    fn offsets_the_two_tracks_and_serpentines_adjacent_fans() {
        let frames = render(&effect(RgbDirection::Clockwise), 2, false).unwrap();

        assert!(frames[0].iter().all(|color| *color == [0; 3]));
        assert!(frames.iter().any(|frame| frame[0][0] > 0));
        assert!(frames.iter().any(|frame| frame[26][1] > 0));
        assert!(frames.iter().all(|frame| frame[26][0] == 0));
    }

    #[test]
    fn opposite_direction_starts_at_the_far_turnaround() {
        let clockwise = render(&effect(RgbDirection::Clockwise), 1, false).unwrap();
        let counter = render(&effect(RgbDirection::CounterClockwise), 1, false).unwrap();

        assert_ne!(clockwise[1], counter[1]);
        assert_eq!(clockwise.len(), 50);
        assert_eq!(counter.len(), 51);
    }
}
