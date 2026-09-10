use super::engine::{frame, range, scale, Color, Frame};
use lianli_shared::rgb::RgbScope;

pub(super) fn runway(scope: RgbScope, colors: &[Color; 4], brightness: u8) -> Vec<Frame> {
    let active = range(scope);
    let width = active.len();
    let head = if scope == RgbScope::Front { 10 } else { 3 };
    let mut frames = Vec::with_capacity(2 * (width + head));
    for reverse in [false, true] {
        for step in 0..width + head {
            let mut output = frame();
            for position in 0..width {
                let color = if position <= step && position + head > step {
                    colors[1]
                } else {
                    colors[0]
                };
                let destination = if reverse {
                    width - position - 1
                } else {
                    position
                };
                output[active.start + destination] = scale(color, brightness);
            }
            frames.push(output);
        }
    }
    if scope == RgbScope::Rear {
        let mut source = Vec::with_capacity(frames.len() * 3);
        for _ in 0..3 {
            source.extend(frames.iter().cloned());
        }
        (0..180)
            .map(|index| source[(index as f32 / (180.0f32 / source.len() as f32)) as usize].clone())
            .collect()
    } else {
        frames
    }
}

pub(super) fn meteor(
    scope: RgbScope,
    colors: &[Color; 4],
    brightness: u8,
    counter_clockwise: bool,
) -> Vec<Frame> {
    const TAIL: [u8; 12] = [6, 8, 16, 24, 32, 48, 64, 96, 120, 150, 200, 255];
    let active = range(scope);
    let width = active.len();
    let head = if scope == RgbScope::Front { 10 } else { 8 };
    let mut frames = Vec::with_capacity(4 * (width + head));
    for &color in colors {
        for step in 0..width + head {
            let mut output = frame();
            let mut tail_index = 0;
            for position in 0..width {
                let pixel = if position <= step && position + head > step {
                    let color = scale(color, TAIL[tail_index]);
                    tail_index += 1;
                    color
                } else {
                    [0; 3]
                };
                let destination = if counter_clockwise {
                    width - position - 1
                } else {
                    position
                };
                output[active.start + destination] = scale(pixel, brightness);
            }
            frames.push(output);
        }
    }
    frames
}
