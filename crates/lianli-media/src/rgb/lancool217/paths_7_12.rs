use super::engine::{frame, range, scale, Color, Frame};
use lianli_shared::rgb::RgbScope;

pub(super) fn color_cycle(
    scope: RgbScope,
    colors: &[Color; 4],
    brightness: u8,
    reverse: bool,
) -> Vec<Frame> {
    const INDEX: [u8; 40] = [
        1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 2, 2, 2, 2, 0, 0, 0, 0, 0, 0, 0, 0, 3, 3, 3,
        3, 3, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    let active = range(scope);
    (0..40)
        .map(|shift| {
            let mut ring = [[0; 3]; 40];
            for position in 0..40 {
                let destination = if reverse { position } else { 39 - position };
                let color = INDEX[(shift + position) % 40];
                if color > 0 {
                    ring[destination] = scale(colors[color as usize - 1], brightness);
                }
            }
            let mut output = frame();
            for led in active.clone() {
                output[led] = ring[led % 40];
            }
            output
        })
        .collect()
}

pub(super) fn cover_cycle(
    scope: RgbScope,
    colors: &[Color; 4],
    brightness: u8,
    reverse: bool,
) -> Vec<Frame> {
    let active = range(scope);
    let width = active.len();
    let mut frames = Vec::with_capacity(width * 2);
    for initial in 0..2 {
        for step in 0..width {
            let mut output = frame();
            for position in 0..width {
                let color = if position <= step {
                    1 - initial
                } else {
                    initial
                };
                let destination = if reverse {
                    width - position - 1
                } else {
                    position
                };
                output[active.start + destination] = scale(colors[color], brightness);
            }
            frames.push(output);
        }
    }
    if scope == RgbScope::Rear {
        stretch(&frames, 3, 160)
    } else {
        frames
    }
}

pub(super) fn wave(scope: RgbScope, color: Color, brightness: u8, reverse: bool) -> Vec<Frame> {
    const LEVELS: [u8; 16] = [
        0, 8, 16, 32, 64, 128, 168, 255, 168, 128, 64, 32, 16, 8, 0, 0,
    ];
    let active = range(scope);
    let mut frames = Vec::with_capacity(80);
    for _ in 0..5 {
        for shift in 0..16 {
            let mut ring = [[0; 3]; 16];
            for position in 0..16 {
                let destination = if reverse { position } else { 15 - position };
                ring[destination] =
                    scale(scale(color, LEVELS[(shift + position) % 16]), brightness);
            }
            let mut output = frame();
            for led in active.clone() {
                output[led] = ring[led % 16];
            }
            frames.push(output);
        }
    }
    frames
}

pub(super) fn meteor_shower(
    scope: RgbScope,
    colors: &[Color; 4],
    brightness: u8,
    reverse: bool,
) -> Vec<Frame> {
    const FRONT: [u8; 20] = [
        255, 192, 168, 128, 64, 32, 24, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    const REAR: [u8; 16] = [255, 168, 128, 64, 32, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let active = range(scope);
    let (cycle, levels) = if scope == RgbScope::Front {
        (80, FRONT.as_slice())
    } else {
        (64, REAR.as_slice())
    };
    (0..cycle)
        .map(|shift| {
            let mut ring = vec![[0; 3]; cycle];
            for position in 0..cycle {
                let destination = if reverse {
                    position
                } else {
                    cycle - position - 1
                };
                let source = (shift + position) % cycle;
                ring[destination] = scale(
                    scale(colors[source / levels.len()], levels[source % levels.len()]),
                    brightness,
                );
            }
            let mut output = frame();
            for position in 0..active.len() {
                output[active.start + position] = ring[position];
            }
            output
        })
        .collect()
}

pub(super) fn tai_chi(
    scope: RgbScope,
    palettes: &[[Color; 4]; 4],
    brightness: u8,
    reverse: bool,
) -> Vec<Frame> {
    let active = range(scope);
    let width = active.len();
    let palette = if scope == RgbScope::Front {
        palettes[0]
    } else {
        palettes[1]
    };
    (0..width)
        .map(|shift| {
            let mut output = frame();
            for position in 0..width {
                let color = usize::from((shift + position) % width >= width / 2);
                let destination = if reverse {
                    width - position - 1
                } else {
                    position
                };
                output[active.start + destination] = scale(palette[color], brightness);
            }
            output
        })
        .collect()
}

fn stretch(frames: &[Frame], repeats: usize, output_frames: usize) -> Vec<Frame> {
    let mut source = Vec::with_capacity(frames.len() * repeats);
    for _ in 0..repeats {
        source.extend(frames.iter().cloned());
    }
    let ratio = output_frames as f32 / source.len() as f32;
    (0..output_frames)
        .map(|index| source[(index as f32 / ratio) as usize].clone())
        .collect()
}
