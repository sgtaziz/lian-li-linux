use super::engine::{scaled_colors, select_side, validate, Color, Matrix, LEDS_PER_FAN};
use anyhow::Result;
use lianli_shared::rgb::RgbEffect;

const INTERNAL_SPEED: i32 = 10;

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<Color>>> {
    validate(effect, fans)?;
    let colors = scaled_colors(effect);
    let mut matrix = Matrix::new(fans);
    matrix.transform_cruise();
    let phase_frames = (12 + (20 - INTERNAL_SPEED * 2)) as usize;
    let mut frames = Vec::with_capacity(colors.len() * phase_frames * 2);

    for color in colors {
        for is_back in [false, true] {
            for counter in 0..phase_frames {
                let color = fade_in(color, counter);
                let mut frame = vec![[0; 3]; fans * LEDS_PER_FAN];
                for (fan, points) in matrix.points.chunks_exact(LEDS_PER_FAN).enumerate() {
                    let left_is_lit = (fan % 2 == 0) != is_back;
                    for (led, point) in points.iter().enumerate() {
                        if (point.x < matrix.center_x) == left_is_lit {
                            frame[fan * LEDS_PER_FAN + led] = color;
                        }
                    }
                }
                frames.push(select_side(&frame, fans, bottom));
            }
        }
    }
    Ok(frames)
}

fn fade_in(color: Color, counter: usize) -> Color {
    if counter >= 5 {
        return color;
    }
    let level = (counter as f32 * 40.0 + 1.0) / 255.0;
    color.map(|channel| (f32::from(channel) * level) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_shared::rgb::{RgbDirection, RgbMode};

    fn effect(direction: RgbDirection) -> RgbEffect {
        RgbEffect {
            mode: RgbMode::Staggered,
            colors: vec![[255, 128, 64]],
            brightness: 4,
            direction,
            ..RgbEffect::default()
        }
    }

    #[test]
    fn alternates_the_unfolded_top_and_bottom_arcs() {
        let top = render(&effect(RgbDirection::Clockwise), 1, false).unwrap();
        let bottom = render(&effect(RgbDirection::Clockwise), 1, true).unwrap();

        assert_eq!(top.len(), 24);
        assert_eq!(top[1][0], [40, 20, 10]);
        assert_eq!(top[5][..13], [[254, 127, 63]; 13]);
        assert_eq!(top[17], vec![[0; 3]; LEDS_PER_FAN]);
        assert_eq!(bottom[5], vec![[0; 3]; LEDS_PER_FAN]);
        assert_eq!(bottom[17][13..], [[254, 127, 63]; 13]);
    }

    #[test]
    fn alternates_adjacent_fans_and_ignores_direction_like_the_vendor_effect() {
        let clockwise = render(&effect(RgbDirection::Clockwise), 2, false).unwrap();
        let counter = render(&effect(RgbDirection::CounterClockwise), 2, false).unwrap();

        assert_eq!(clockwise, counter);
        assert_eq!(clockwise[5][0], [254, 127, 63]);
        assert_eq!(clockwise[5][26], [0; 3]);
        assert_eq!(clockwise[17][0], [0; 3]);
        assert_eq!(clockwise[17][26], [254, 127, 63]);
    }
}
