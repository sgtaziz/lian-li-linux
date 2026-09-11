use super::engine::{select_side, validate, Color, Matrix};
use super::random::{DotNetRandom, COLORS};
use anyhow::Result;
use lianli_shared::rgb::RgbEffect;

struct Track {
    step: i32,
    color: Color,
}

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<Color>>> {
    validate(effect, fans)?;
    let matrix = Matrix::new(fans);
    let width = (matrix.max_x - matrix.min_x) / 5;
    let mut random = DotNetRandom::new(0);
    let mut tracks: Vec<_> = (0..5)
        .map(|index| Track {
            step: width * index,
            color: COLORS[random.next(0, COLORS.len() as i32) as usize],
        })
        .collect();
    let brightness = [0u16, 64, 128, 192, 255][effect.brightness as usize];
    let mut frame = vec![[0; 3]; fans * 26];
    let mut frames = Vec::new();
    let mut completed = 0;
    while completed < COLORS.len() {
        for track in &mut tracks {
            if track.step < -width - matrix.center_x % width {
                track.step = matrix.center_x;
                track.color = COLORS[random.next(0, COLORS.len() as i32) as usize];
                completed += 1;
            }
        }
        for (point, color) in matrix.points.iter().zip(&mut frame) {
            for track in &tracks {
                if (point.x > track.step - 13
                    && point.x < track.step + width + 13
                    && point.x < matrix.center_x)
                    || (point.x > matrix.max_x - track.step - width - 13
                        && point.x < matrix.max_x - track.step + 13
                        && point.x > matrix.center_x)
                {
                    *color = track.color;
                }
            }
        }
        for track in &mut tracks {
            track.step -= 13;
        }
        let scaled: Vec<_> = frame
            .iter()
            .map(|color| color.map(|c| ((u16::from(c) * brightness) >> 8) as u8))
            .collect();
        frames.push(select_side(&scaled, fans, bottom));
    }
    Ok(frames)
}
