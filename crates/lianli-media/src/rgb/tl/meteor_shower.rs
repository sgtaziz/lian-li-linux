use super::engine::{select_side, validate, Color, Matrix};
use super::random::{DotNetRandom, COLORS};
use anyhow::Result;
use lianli_shared::rgb::{RgbDirection, RgbEffect};

struct Track {
    step: f32,
    speed: f32,
    width: i32,
    color: Color,
    count: u8,
}

pub(super) fn render(effect: &RgbEffect, fans: usize, bottom: bool) -> Result<Vec<Vec<Color>>> {
    validate(effect, fans)?;
    let matrix = Matrix::new(fans);
    let reverse = effect.direction == RgbDirection::CounterClockwise;
    let mut random = DotNetRandom::new(0);
    let spacing = (matrix.max_x - matrix.min_x) / 8;
    let mut tracks: Vec<_> = (0..8)
        .map(|index| Track {
            step: if reverse {
                (matrix.max_x + index % 4 * spacing) as f32
            } else {
                -(index % 4 * spacing) as f32
            },
            speed: random.next(10, 23) as f32 + 0.3,
            width: random.next(40, 80),
            color: COLORS[random.next(0, COLORS.len() as i32) as usize],
            count: 0,
        })
        .collect();
    let brightness = [0u16, 64, 128, 192, 255][effect.brightness as usize];
    let mut frames = Vec::new();
    loop {
        for track in &mut tracks {
            if (reverse && track.step < 0.0) || (!reverse && track.step > matrix.max_x as f32) {
                track.step = if reverse { matrix.max_x as f32 } else { 0.0 };
                track.speed = 10.0 + random.next_double() as f32 * 13.0 + 0.3;
                track.width = random.next(40, 80);
                track.color = COLORS[random.next(0, COLORS.len() as i32) as usize];
                track.count += 1;
            }
        }
        let frame: Vec<_> = matrix
            .points
            .iter()
            .map(|point| {
                let side = if point.y < matrix.center_y {
                    &tracks[..4]
                } else {
                    &tracks[4..]
                };
                side.iter()
                    .find(|track| {
                        track.count < 5
                            && point.x as f32 > track.step - track.width as f32
                            && point.x as f32 <= track.step
                    })
                    .map_or([0; 3], |track| track.color)
            })
            .collect();
        let finished = frame.iter().all(|color| *color == [0; 3]);
        let scaled: Vec<_> = frame
            .into_iter()
            .map(|color| color.map(|c| ((u16::from(c) * brightness) >> 8) as u8))
            .collect();
        frames.push(select_side(&scaled, fans, bottom));
        if finished {
            break;
        }
        for track in &mut tracks {
            track.step += if reverse { -track.speed } else { track.speed };
        }
    }
    Ok(frames)
}
