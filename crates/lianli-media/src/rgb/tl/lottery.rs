use super::engine::{scaled_colors, select_side, validate, Color, Matrix};
use anyhow::Result;
use lianli_shared::rgb::{RgbDirection, RgbEffect};

const INTERNAL_SPEED: f32 = 10.0;

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<Color>>> {
    validate(effect, fans)?;
    let colors = scaled_colors(effect);
    let mut matrix = Matrix::new(fans);
    matrix.transform_cruise();
    let ribbon = init_ribbon(fans, &colors);
    let counter_clockwise = matches!(effect.direction, RgbDirection::CounterClockwise);
    let increment = INTERNAL_SPEED * 0.4 + 4.3;
    let mut step = 0.0f32;
    let mut frames = Vec::new();

    loop {
        if counter_clockwise && step >= ribbon.len() as f32 {
            break;
        }
        if !counter_clockwise && step < 0.0 {
            break;
        }

        let mut frame = Vec::with_capacity(matrix.points.len());
        for point in &matrix.points {
            let coordinate = (point.x / 8 * 6) as f32;
            let index = ((coordinate + step) as usize) % ribbon.len();
            frame.push(ribbon[index]);
        }
        frames.push(select_side(&frame, fans, bottom));

        if counter_clockwise {
            step += increment;
        } else {
            if step <= 0.0 {
                step = (ribbon.len() - 1) as f32;
            }
            step -= increment;
        }
    }
    Ok(frames)
}

fn init_ribbon(fans: usize, colors: &[Color]) -> Vec<Color> {
    let first = colors[0];
    let second = colors.get(1).copied().unwrap_or([0; 3]);
    let mut ribbon = Vec::with_capacity(fans * 240);
    for block in 0..fans * 4 {
        ribbon.extend(std::iter::repeat_n(
            if block % 2 == 0 { first } else { second },
            60,
        ));
    }
    ribbon
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_shared::rgb::RgbMode;

    fn effect(direction: RgbDirection) -> RgbEffect {
        RgbEffect {
            mode: RgbMode::Lottery,
            colors: vec![[255, 0, 0], [0, 255, 0]],
            brightness: 4,
            direction,
            ..RgbEffect::default()
        }
    }

    #[test]
    fn maps_the_unfolded_arcs_across_alternating_color_blocks() {
        let top = render(&effect(RgbDirection::CounterClockwise), 1, false).unwrap();
        let bottom = render(&effect(RgbDirection::CounterClockwise), 1, true).unwrap();

        assert_eq!(top[0][0], [254, 0, 0]);
        assert_eq!(top[0][7], [0, 254, 0]);
        assert_eq!(bottom[0][13], [254, 0, 0]);
        assert_eq!(bottom[0][20], [0, 254, 0]);
    }

    #[test]
    fn direction_reverses_the_ribbon_phase_sequence() {
        let clockwise = render(&effect(RgbDirection::Clockwise), 1, false).unwrap();
        let counter = render(&effect(RgbDirection::CounterClockwise), 1, false).unwrap();

        assert_eq!(clockwise[0], counter[0]);
        assert_ne!(clockwise[1], counter[1]);
    }
}
